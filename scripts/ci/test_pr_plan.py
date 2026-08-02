from __future__ import annotations

import importlib.util
import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock


SCRIPT = Path(__file__).with_name("pr_plan.py")
SPEC = importlib.util.spec_from_file_location("pr_plan", SCRIPT)
assert SPEC and SPEC.loader
pr_plan = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = pr_plan
SPEC.loader.exec_module(pr_plan)


class PlannerTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        package_rows = []
        definitions = {
            "alpha": [],
            "beta": ["alpha"],
            "gamma": ["beta"],
            "delta": [],
        }
        for name, dependencies in definitions.items():
            package_root = self.root / "crates" / name
            package_root.mkdir(parents=True)
            manifest = package_root / "Cargo.toml"
            manifest.write_text(f"[package]\nname = {name!r}\n")
            package_rows.append(
                {
                    "id": f"path+file://{package_root}#{name}@1.0.0",
                    "name": name,
                    "version": "1.0.0",
                    "source": None,
                    "manifest_path": str(manifest),
                    "dependencies": [
                        {"name": dependency, "kind": "dev" if name == "gamma" else None}
                        for dependency in dependencies
                    ],
                }
            )
        self.metadata = {
            "workspace_members": [package["id"] for package in package_rows],
            "packages": package_rows,
        }
        self.policy = {
            "max_shards": 3,
            "target_shard_weight": 3,
            "full_paths": ["Cargo.toml"],
            "scoped_full_paths": ["Cargo.lock"],
            "known_policy_paths": [
                "README.md",
                ".config/nextest.toml",
                ".github/workflows/**",
                "scripts/ci/**",
            ],
            "specialty_only_paths": ["crates/delta/tests/harness/**"],
            "specialty_rules": [
                {"gate": "browser-smoke", "patterns": ["tests/browser/**"]},
                {"gate": "examples", "patterns": ["examples/**", "crates/delta/src/api/**"]},
                {
                    "gate": "release-tooling",
                    "patterns": ["infra/release/**", "crates/delta/tests/harness/**"],
                },
            ],
            "example_projects": ["one", "two"],
            "pr_example_projects": ["one"],
            "package_weights": {"alpha": 3, "beta": 2},
        }

    def tearDown(self) -> None:
        self.temp.cleanup()

    def plan(self, *paths: str, job_mode: str = "combined"):
        return pr_plan.make_plan(
            root=self.root,
            metadata=self.metadata,
            policy=self.policy,
            paths=list(paths),
            base="base",
            head="head",
            candidate=None,
            job_mode=job_mode,
        )

    def test_direct_change_selects_all_reverse_dependency_kinds(self) -> None:
        plan = self.plan("crates/alpha/src/lib.rs")
        self.assertEqual(plan["mode"], "targeted")
        self.assertEqual(plan["direct_crates"], ["alpha"])
        self.assertEqual(plan["selected_crates"], ["alpha", "beta", "gamma"])

    def test_unrelated_crate_is_not_selected(self) -> None:
        plan = self.plan("crates/delta/tests/new.rs")
        self.assertEqual(plan["selected_crates"], ["delta"])

    def test_docs_only_has_no_rust_shards(self) -> None:
        plan = self.plan("docs/guide.md", "crates/alpha/README.md")
        self.assertEqual(plan["mode"], "docs")
        self.assertEqual(plan["shards"], [])
        self.assertEqual(plan["shard_jobs"], [])

    def test_root_manifest_and_lockfile_select_full_workspace(self) -> None:
        for path in ("Cargo.toml", "Cargo.lock"):
            with self.subTest(path=path):
                plan = self.plan(path)
                self.assertEqual(plan["mode"], "full")
                self.assertEqual(set(plan["selected_crates"]), {"alpha", "beta", "gamma", "delta"})

    def test_validated_lockfile_delta_uses_changed_manifest_closure(self) -> None:
        plan = pr_plan.make_plan(
            root=self.root,
            metadata=self.metadata,
            policy=self.policy,
            paths=["Cargo.lock", "crates/alpha/Cargo.toml"],
            base="base",
            head="head",
            candidate=None,
            job_mode="combined",
            validated_scoped_paths={"Cargo.lock"},
            lockfile_changed_packages=["alpha", "external"],
        )
        self.assertEqual(plan["mode"], "targeted")
        self.assertEqual(plan["direct_crates"], ["alpha"])
        self.assertEqual(plan["selected_crates"], ["alpha", "beta", "gamma"])
        self.assertEqual(plan["validated_scoped_paths"], ["Cargo.lock"])
        self.assertEqual(plan["lockfile_changed_packages"], ["alpha", "external"])

    def test_lockfile_scope_requires_policy_declaration(self) -> None:
        policy = copy.deepcopy(self.policy)
        policy["scoped_full_paths"] = []
        with self.assertRaisesRegex(pr_plan.PlanError, "not declared"):
            pr_plan.make_plan(
                root=self.root,
                metadata=self.metadata,
                policy=policy,
                paths=["Cargo.lock", "crates/alpha/Cargo.toml"],
                base="base",
                head="head",
                candidate=None,
                validated_scoped_paths={"Cargo.lock"},
            )

    def test_lockfile_delta_canonicalizes_dependency_references(self) -> None:
        explicit = """\
version = 4
[[package]]
name = "alpha"
version = "1.0.0"
dependencies = ["external 2.0.0"]
[[package]]
name = "external"
version = "2.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
"""
        implicit = explicit.replace("external 2.0.0", "external", 1)
        self.assertEqual(pr_plan.lockfile_delta(explicit, implicit), (set(), set()))

    def test_lockfile_delta_tolerates_unreachable_stale_record(self) -> None:
        payload = """\
version = 4
[[package]]
name = "alpha"
version = "1.0.0"
[[package]]
name = "stale"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
dependencies = ["missing 9.9.9"]
"""
        self.assertEqual(pr_plan.lockfile_delta(payload, payload), (set(), set()))
        self.assertEqual(
            pr_plan.lockfile_dependency_closure(payload, {"alpha"}),
            {pr_plan.LockPackage("alpha", "1.0.0", "")},
        )

    def test_lockfile_closure_tolerates_unmaterialized_target_dependency(self) -> None:
        payload = """\
version = 4
[[package]]
name = "alpha"
version = "1.0.0"
dependencies = ["target-only 9.9.9"]
"""
        self.assertEqual(
            pr_plan.lockfile_dependency_closure(payload, {"alpha"}),
            {pr_plan.LockPackage("alpha", "1.0.0", "")},
        )

    def test_lockfile_validator_accepts_dependency_that_becomes_reachable(self) -> None:
        base_lock = """\
version = 4
[[package]]
name = "alpha"
version = "1.0.0"
[[package]]
name = "external"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "old"
"""
        head_lock = """\
version = 4
[[package]]
name = "alpha"
version = "1.0.0"
dependencies = ["external"]
[[package]]
name = "external"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "new"
"""
        with mock.patch.object(pr_plan, "run", side_effect=[base_lock, head_lock]):
            changed = pr_plan.validate_scoped_lockfile(
                root=self.root,
                packages=pr_plan.workspace_packages(self.root, self.metadata),
                paths=["Cargo.lock", "crates/alpha/Cargo.toml"],
                base="base",
                head="head",
            )
        self.assertEqual(changed, ["alpha", "external"])

    def test_lockfile_validator_accepts_only_reachable_dependency_delta(self) -> None:
        metadata = copy.deepcopy(self.metadata)
        base_lock = """\
version = 4
[[package]]
name = "alpha"
version = "1.0.0"
dependencies = ["external 1.0.0"]
[[package]]
name = "external"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
"""
        head_lock = """\
version = 4
[[package]]
name = "alpha"
version = "1.0.0"
dependencies = ["external 2.0.0"]
[[package]]
name = "external"
version = "2.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
"""
        with mock.patch.object(pr_plan, "run", side_effect=[base_lock, head_lock]):
            changed = pr_plan.validate_scoped_lockfile(
                root=self.root,
                packages=pr_plan.workspace_packages(self.root, metadata),
                paths=["Cargo.lock", "crates/alpha/Cargo.toml"],
                base="base",
                head="head",
            )
        self.assertEqual(changed, ["alpha", "external"])

    def test_lockfile_validator_rejects_unrelated_package_delta(self) -> None:
        base_lock = """\
version = 4
[[package]]
name = "alpha"
version = "1.0.0"
[[package]]
name = "unrelated"
version = "1.0.0"
"""
        head_lock = """\
version = 4
[[package]]
name = "alpha"
version = "1.0.0"
[[package]]
name = "unrelated"
version = "2.0.0"
"""
        with mock.patch.object(pr_plan, "run", side_effect=[base_lock, head_lock]):
            with self.assertRaisesRegex(pr_plan.PlanError, "escapes"):
                pr_plan.validate_scoped_lockfile(
                    root=self.root,
                    packages=pr_plan.workspace_packages(self.root, self.metadata),
                    paths=["Cargo.lock", "crates/alpha/Cargo.toml"],
                    base="base",
                    head="head",
                )

    def test_ci_only_change_runs_policy_without_workspace_crates(self) -> None:
        plan = self.plan("scripts/ci/pr_plan.py", ".github/workflows/pr-gate.yml")
        self.assertEqual(plan["mode"], "policy")
        self.assertEqual(plan["selected_crates"], [])
        self.assertEqual(plan["shard_jobs"], [])

    def test_nextest_config_change_runs_policy_without_workspace_crates(self) -> None:
        plan = self.plan(".config/nextest.toml")
        self.assertEqual(plan["mode"], "policy")
        self.assertEqual(plan["selected_crates"], [])
        self.assertEqual(plan["shard_jobs"], [])

    def test_ci_and_crate_change_remains_targeted(self) -> None:
        plan = self.plan("scripts/ci/pr_plan.py", "crates/delta/src/lib.rs")
        self.assertEqual(plan["mode"], "targeted")
        self.assertEqual(plan["selected_crates"], ["delta"])

    def test_unmapped_source_path_fails_safe_to_full(self) -> None:
        plan = self.plan("crates/removed-crate/src/lib.rs")
        self.assertEqual(plan["mode"], "full")
        self.assertIn("removed-crate", plan["reason"])

    def test_specialty_only_path_does_not_force_workspace(self) -> None:
        plan = self.plan("tests/browser/spec.ts")
        self.assertEqual(plan["mode"], "targeted")
        self.assertEqual(plan["selected_crates"], [])
        self.assertEqual(plan["specialty_gates"], ["browser-smoke"])

    def test_release_runner_path_uses_specialty_instead_of_full_workspace(self) -> None:
        plan = self.plan("infra/release/startup.sh")
        self.assertEqual(plan["mode"], "targeted")
        self.assertEqual(plan["selected_crates"], [])
        self.assertEqual(plan["specialty_gates"], ["release-tooling"])

    def test_owned_test_harness_uses_specialty_without_crate_closure(self) -> None:
        plan = self.plan("crates/delta/tests/harness/run.sh")
        self.assertEqual(plan["mode"], "targeted")
        self.assertEqual(plan["selected_crates"], [])
        self.assertEqual(plan["specialty_gates"], ["release-tooling"])

    def test_unowned_specialty_only_path_fails_closed(self) -> None:
        policy = copy.deepcopy(self.policy)
        policy["specialty_rules"] = [
            rule for rule in policy["specialty_rules"] if rule["gate"] != "release-tooling"
        ]
        plan = pr_plan.make_plan(
            root=self.root,
            metadata=self.metadata,
            policy=policy,
            paths=["crates/delta/tests/harness/run.sh"],
            base="base",
            head="head",
            candidate=None,
            job_mode="combined",
        )
        self.assertEqual(plan["mode"], "full")
        self.assertIn("harness/run.sh", plan["reason"])

    def test_rename_records_old_and_new_paths(self) -> None:
        payload = b"R100\0crates/alpha/old.rs\0crates/beta/new.rs\0D\0crates/delta/gone.rs\0"
        self.assertEqual(
            pr_plan.parse_name_status_z(payload),
            ["crates/alpha/old.rs", "crates/beta/new.rs", "crates/delta/gone.rs"],
        )

    def test_sharding_is_deterministic_and_complete(self) -> None:
        first = self.plan("Cargo.lock")["shards"]
        second = self.plan("Cargo.lock")["shards"]
        self.assertEqual(first, second)
        flattened = [name for shard in first for name in shard["packages"]]
        self.assertEqual(sorted(flattened), ["alpha", "beta", "delta", "gamma"])
        self.assertLessEqual(len(first), 3)

    def test_each_shard_has_one_combined_test_and_clippy_job(self) -> None:
        plan = self.plan("Cargo.lock")
        expected = {
            (shard["id"], "all", shard["packages_csv"])
            for shard in plan["shards"]
        }
        actual = {
            (job["shard_id"], job["check"], job["packages_csv"])
            for job in plan["shard_jobs"]
        }
        self.assertEqual(actual, expected)

    def test_split_jobs_remain_available_for_gcp_workspace_workers(self) -> None:
        plan = self.plan("Cargo.lock", job_mode="split")
        self.assertEqual(
            {job["check"] for job in plan["shard_jobs"]},
            {"test", "clippy"},
        )

    def test_github_outputs_are_single_line_json(self) -> None:
        path = self.root / "github-output"
        plan = self.plan("crates/delta/src/lib.rs")
        pr_plan.write_github_outputs(path, plan)
        values = dict(line.split("=", 1) for line in path.read_text().splitlines())
        self.assertEqual(values["mode"], "targeted")
        jobs = json.loads(values["shard_jobs"])["include"]
        self.assertEqual({job["check"] for job in jobs}, {"all"})
        self.assertTrue(all(job["packages"] == ["delta"] for job in jobs))
        shards = json.loads(values["shards"])["include"]
        self.assertEqual(shards, plan["shards"])
        self.assertEqual(values["sip_job_count"], "0")
        self.assertEqual(json.loads(values["sip_jobs"])["include"], [])

    def test_public_api_change_uses_bounded_example_contract_set(self) -> None:
        plan = self.plan("crates/delta/src/api/client.rs")
        self.assertEqual(plan["specialty_gates"], ["example--one"])

    def test_example_only_change_builds_only_touched_project(self) -> None:
        plan = self.plan("examples/two/src/main.rs")
        self.assertEqual(plan["specialty_gates"], ["example--two"])

    def test_example_smoke_matrix_is_split_into_independent_gates(self) -> None:
        policy = copy.deepcopy(self.policy)
        policy["specialty_rules"].append(
            {"gate": "examples-smoke", "patterns": ["examples/**"]}
        )
        policy["pr_example_smoke_projects"] = ["one", "two"]
        plan = pr_plan.make_plan(
            root=self.root,
            metadata=self.metadata,
            policy=policy,
            paths=["examples/two/src/main.rs"],
            base="base",
            head="head",
            candidate=None,
            job_mode="combined",
        )
        self.assertEqual(
            plan["specialty_gates"],
            ["example--two", "example-smoke--one", "example-smoke--two"],
        )

    def test_invalid_example_smoke_matrix_fails_closed(self) -> None:
        policy = copy.deepcopy(self.policy)
        policy["specialty_rules"].append(
            {"gate": "examples-smoke", "patterns": ["examples/**"]}
        )
        policy["pr_example_smoke_projects"] = ["missing"]
        with self.assertRaisesRegex(pr_plan.PlanError, "smoke set"):
            pr_plan.make_plan(
                root=self.root,
                metadata=self.metadata,
                policy=policy,
                paths=["examples/two/src/main.rs"],
                base="base",
                head="head",
                candidate=None,
            )

    def test_invalid_pr_example_contract_set_fails_closed(self) -> None:
        self.policy["pr_example_projects"] = ["missing"]
        with self.assertRaises(pr_plan.PlanError):
            self.plan("crates/delta/src/api/client.rs")

    def test_sip_uses_partitioned_pr_lanes_and_defers_declared_long_target(self) -> None:
        metadata = copy.deepcopy(self.metadata)
        package_root = self.root / "crates" / "rvoip-sip"
        package_root.mkdir(parents=True)
        manifest = package_root / "Cargo.toml"
        manifest.write_text("[package]\nname = 'rvoip-sip'\n")
        package_id = f"path+file://{package_root}#rvoip-sip@1.0.0"
        metadata["workspace_members"].append(package_id)
        metadata["packages"].append(
            {
                "id": package_id,
                "name": "rvoip-sip",
                "manifest_path": str(manifest),
                "dependencies": [],
                "targets": [
                    {"name": "rvoip_sip", "kind": ["lib"]},
                    {"name": "audio_roundtrip_integration", "kind": ["test"]},
                    {"name": "cancel_integration", "kind": ["test"]},
                    {"name": "event_tests", "kind": ["test"]},
                    {"name": "srtp_call_integration", "kind": ["test"]},
                ],
            }
        )
        policy = copy.deepcopy(self.policy)
        policy["pr_deferred_sip_targets"] = ["audio_roundtrip_integration"]
        policy["pr_sip_fixture_examples"] = {
            "cancel_integration": ["cancel_alice", "cancel_bob"]
        }
        policy["pr_sip_partitions"] = 2
        plan = pr_plan.make_plan(
            root=self.root,
            metadata=metadata,
            policy=policy,
            paths=["crates/rvoip-sip/src/lib.rs"],
            base="base",
            head="head",
            candidate=None,
            job_mode="combined",
        )
        self.assertEqual(plan["deferred_sip_targets"], ["audio_roundtrip_integration"])
        self.assertEqual(
            [job["id"] for job in plan["sip_jobs"]],
            ["core-test", "clippy", "fixtures", "integration-1", "integration-2"],
        )
        fixture_job = next(job for job in plan["sip_jobs"] if job["id"] == "fixtures")
        self.assertEqual(fixture_job["targets_csv"], "cancel_integration")
        self.assertEqual(fixture_job["examples_csv"], "cancel_alice,cancel_bob")
        assigned = {
            target
            for job in plan["sip_jobs"]
            if job["kind"] == "integration"
            for target in job["targets_csv"].split(",")
        }
        self.assertEqual(assigned, {"event_tests", "srtp_call_integration"})
        self.assertNotIn(
            "rvoip-sip",
            [package for shard in plan["shards"] for package in shard["packages"]],
        )

    def test_unknown_sip_fixture_target_fails_closed(self) -> None:
        metadata = copy.deepcopy(self.metadata)
        package_root = self.root / "crates" / "rvoip-sip"
        package_root.mkdir(parents=True)
        manifest = package_root / "Cargo.toml"
        manifest.write_text("[package]\nname = 'rvoip-sip'\n")
        package_id = f"path+file://{package_root}#rvoip-sip@1.0.0"
        metadata["workspace_members"].append(package_id)
        metadata["packages"].append(
            {
                "id": package_id,
                "name": "rvoip-sip",
                "manifest_path": str(manifest),
                "dependencies": [],
                "targets": [{"name": "event_tests", "kind": ["test"]}],
            }
        )
        policy = copy.deepcopy(self.policy)
        policy["pr_sip_fixture_examples"] = {"missing": ["fixture"]}
        with self.assertRaises(pr_plan.PlanError):
            pr_plan.make_plan(
                root=self.root,
                metadata=metadata,
                policy=policy,
                paths=["crates/rvoip-sip/src/lib.rs"],
                base="base",
                head="head",
                candidate=None,
                job_mode="combined",
            )

    def test_main_runs_pr_deferred_sip_target_in_a_separate_lane(self) -> None:
        metadata = copy.deepcopy(self.metadata)
        package_root = self.root / "crates" / "rvoip-sip"
        package_root.mkdir(parents=True)
        manifest = package_root / "Cargo.toml"
        manifest.write_text("[package]\nname = 'rvoip-sip'\n")
        package_id = f"path+file://{package_root}#rvoip-sip@1.0.0"
        metadata["workspace_members"].append(package_id)
        metadata["packages"].append(
            {
                "id": package_id,
                "name": "rvoip-sip",
                "manifest_path": str(manifest),
                "dependencies": [],
                "targets": [
                    {"name": "audio_roundtrip_integration", "kind": ["test"]},
                    {"name": "event_tests", "kind": ["test"]},
                ],
            }
        )
        policy = copy.deepcopy(self.policy)
        policy["pr_deferred_sip_targets"] = ["audio_roundtrip_integration"]
        policy["pr_sip_fixture_examples"] = {
            "audio_roundtrip_integration": ["audio_alice", "audio_bob"]
        }
        plan = pr_plan.make_plan(
            root=self.root,
            metadata=metadata,
            policy=policy,
            paths=["Cargo.lock"],
            base="base",
            head="head",
            candidate=None,
            job_mode="combined",
            deferred_sip_mode="separate",
        )
        self.assertEqual(plan["deferred_sip_targets"], [])
        self.assertEqual(
            plan["separate_sip_targets"], ["audio_roundtrip_integration"]
        )
        long_job = next(
            job
            for job in plan["sip_jobs"]
            if job["id"] == "long-audio_roundtrip_integration"
        )
        self.assertEqual(long_job["kind"], "fixtures")
        self.assertEqual(long_job["targets_csv"], "audio_roundtrip_integration")
        self.assertEqual(long_job["examples_csv"], "audio_alice,audio_bob")


if __name__ == "__main__":
    unittest.main()
