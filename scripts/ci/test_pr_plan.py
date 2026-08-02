from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest


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
            "full_paths": ["Cargo.toml", "Cargo.lock"],
            "known_policy_paths": ["README.md", ".github/workflows/**", "scripts/ci/**"],
            "specialty_rules": [
                {"gate": "browser-smoke", "patterns": ["tests/browser/**"]},
                {"gate": "examples", "patterns": ["examples/**", "crates/delta/src/api/**"]},
                {"gate": "release-tooling", "patterns": ["infra/release/**"]},
            ],
            "example_projects": ["one", "two"],
            "package_weights": {"alpha": 3, "beta": 2},
        }

    def tearDown(self) -> None:
        self.temp.cleanup()

    def plan(self, *paths: str):
        return pr_plan.make_plan(
            root=self.root,
            metadata=self.metadata,
            policy=self.policy,
            paths=list(paths),
            base="base",
            head="head",
            candidate=None,
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

    def test_ci_only_change_runs_policy_without_workspace_crates(self) -> None:
        plan = self.plan("scripts/ci/pr_plan.py", ".github/workflows/pr-gate.yml")
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

    def test_public_api_change_uses_bounded_example_contract_lane(self) -> None:
        plan = self.plan("crates/delta/src/api/client.rs")
        self.assertEqual(plan["specialty_gates"], ["examples-contract"])

    def test_example_only_change_builds_only_touched_project(self) -> None:
        plan = self.plan("examples/two/src/main.rs")
        self.assertEqual(plan["specialty_gates"], ["example--two"])


if __name__ == "__main__":
    unittest.main()
