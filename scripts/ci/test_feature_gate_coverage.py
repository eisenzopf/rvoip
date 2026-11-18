from __future__ import annotations

import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import feature_gate_coverage as coverage  # noqa: E402
import run_checks  # noqa: E402


ROOT = Path(__file__).resolve().parents[2]


class FeatureGateCoverageTests(unittest.TestCase):
    """Every feature-gated target is built, and every such test run, by a gate.

    The crate shards build default features. A target with
    `required-features` is silently skipped there, and a test file behind
    `#![cfg(feature = ...)]` compiles to nothing and passes. Either runs only
    if some gate enables its features; `sip_webrtc_acceptance` went unrun
    for two months and rotted because none did.
    """

    @classmethod
    def setUpClass(cls) -> None:
        cls.packages, cls.targets = coverage.inventory()
        cls.rows = coverage.coverage()
        cls.by_target = {(row["package"], row["target"]): row for row in cls.rows}

    def test_every_feature_gated_target_has_a_gate(self) -> None:
        missing = coverage.ungated(self.rows)
        detail = "\n".join(
            f"  {row['package']} {row['kind']} {row['target']} ({row['source']}) "
            f"needs {', '.join(row['required_features']) or row['cfg']}"
            for row in missing
        )
        if missing:
            self.fail(
                f"{len(missing)} feature-gated targets no CI gate builds (examples, "
                "benches, bins) or runs (tests). Add the package and features to "
                "run_checks.FEATURE_TARGET_GATES, a matching specialty rule in "
                "policy.json, and a --specialty-gate flag in main-ci.yml:\n" + detail
            )

    def test_the_inventory_sees_both_hiding_patterns(self) -> None:
        # Guards the guard: if metadata parsing or the cfg scan broke, the
        # coverage test above would pass over an empty list.
        gating = {(row["package"], row["target"]): row["gating"] for row in self.rows}
        self.assertEqual(
            gating[("rvoip-webrtc", "sip_webrtc_acceptance")], "required-features"
        )
        self.assertEqual(gating[("rvoip-rtp-core", "dtls_srtp_handshake_test")], "cfg")
        self.assertGreater(
            sum(value == "required-features" for value in gating.values()), 50
        )

    def test_sip_webrtc_acceptance_runs_in_its_feature_gate(self) -> None:
        # The target whose rot motivated this policy: no shard, SIP lane or
        # other specialty gate runs it, and release only type-checks it.
        row = self.by_target[("rvoip-webrtc", "sip_webrtc_acceptance")]
        self.assertEqual(row["ci_run"], ["feature-targets-webrtc"])
        self.assertEqual(row["status"], "ci")

    def test_release_only_coverage_is_reserved_for_the_perf_harness(self) -> None:
        for row in self.rows:
            if row["status"] != "release-only":
                continue
            with self.subTest(target=row["target"]):
                self.assertEqual(row["package"], "rvoip-sip")
                self.assertIn(
                    "perf-tests",
                    row["required_features"] + coverage.cfg_features(row["cfg"] or ""),
                )

    def test_invite_planner_retention_runs_in_its_release_gate(self) -> None:
        # Was the last KNOWN_CFG_GAPS entry: `perf-tests`-gated and ignored,
        # so the PR SIP lane ran it as an empty binary and nothing ran it.
        row = self.by_target[("rvoip-sip", "invite_planner_retention_qualification")]
        self.assertEqual(row["gating"], "cfg")
        self.assertEqual(row["status"], "release-only")
        self.assertEqual(row["release_run"], ["release perf.invite-planner-retention"])

    def test_known_cfg_gaps_are_cfg_only_and_still_gaps(self) -> None:
        # An entry that gains a gate must leave the list; a target with
        # `required-features` may never be on it.
        for key, reason in coverage.KNOWN_CFG_GAPS.items():
            with self.subTest(target=key):
                self.assertTrue(reason)
                row = self.by_target.get(key)
                self.assertIsNotNone(row, f"{key} is no longer a cfg-gated test target")
                self.assertEqual(row["gating"], "cfg")
                self.assertEqual(row["status"], "UNGATED")

    def test_feature_target_gates_are_registered_everywhere(self) -> None:
        # The codec gates' sharp edge: a gate in run_checks.py but not in
        # policy.json never runs on a PR, and one missing from main-ci.yml's
        # forced list depends on a PR touching the right path.
        policy = json.loads((ROOT / "scripts/ci/policy.json").read_text())
        rules = {rule["gate"]: rule["patterns"] for rule in policy["specialty_rules"]}
        main = (ROOT / ".github/workflows/main-ci.yml").read_text()
        for gate, entries in run_checks.FEATURE_TARGET_GATES.items():
            with self.subTest(gate=gate):
                self.assertIn(gate, rules)
                self.assertIn(f"--specialty-gate {gate}\n", main)
                # Optional dependencies compile only under their feature, so a
                # lockfile change moves code no shard builds.
                self.assertIn("Cargo.lock", rules[gate])
                for run in entries:
                    self.assertIn(f"{self.packages[run.package].root}/**", rules[gate])
                self.assertIn(gate, coverage.main_specialty_gates())

    def test_feature_target_gates_run_each_package_with_its_features(self) -> None:
        for gate, entries in run_checks.FEATURE_TARGET_GATES.items():
            commands = run_checks.specialty_commands(gate, ROOT)
            with self.subTest(gate=gate):
                self.assertEqual(len(commands), len(entries))
                for (argv, cwd, env), run in zip(commands, entries):
                    self.assertEqual(argv[:3], ["cargo", "test", "--locked"])
                    self.assertEqual(argv[argv.index("-p") + 1], run.package)
                    self.assertNotIn("--all-features", argv)
                    self.assertIsNone(cwd)
                    self.assertIsNone(env)
                    features = [item for item in run.features.split(",") if item]
                    if features:
                        self.assertEqual(argv[argv.index("--features") + 1], run.features)
                    self.assertEqual("--no-default-features" in argv, run.no_default_features)
                    if run.tests:
                        named = [argv[i + 1] for i, value in enumerate(argv) if value == "--test"]
                        self.assertEqual(named, list(run.tests))
                        self.assertNotIn("--tests", argv)
                    else:
                        for flag in ("--lib", "--tests", "--bins", "--examples"):
                            self.assertIn(flag, argv)
                    declared = self.packages[run.package].features
                    for feature in features:
                        self.assertIn(feature, declared, f"{run.package} has no feature {feature}")
                    # Each entry must be needed: it must reach a feature-gated
                    # target of its package that the default shard does not.
                    invocation = coverage.parse_invocation(argv, gate=gate, tier="ci")
                    reached = [
                        target
                        for target in self.targets
                        if target.package == run.package
                        and coverage.covers(invocation, target, self.packages) == "run"
                    ]
                    self.assertTrue(reached, f"{gate} entry for {run.package} runs nothing new")

    def test_libvpx_gates_are_the_ones_building_vp8(self) -> None:
        for gate, entries in run_checks.FEATURE_TARGET_GATES.items():
            features = {feature for run in entries for feature in run.features.split(",")}
            with self.subTest(gate=gate):
                self.assertEqual(
                    "client-video-vp8" in features, gate in run_checks.LIBVPX_GATES
                )


class WorkspaceAllFeaturesGateTests(unittest.TestCase):
    """Cross-crate feature unification is compiled on every PR and main push.

    Per-package gates never unify features across crates, so code that only
    exists under workspace-wide `--all-features` (an optional dependency's
    trait impls meeting another crate's tests) went unbuilt until #424 broke
    it on main.
    """

    gate = run_checks.WORKSPACE_ALL_FEATURES_GATE

    def test_gate_is_workspace_wide_all_features_lint_of_every_target(self) -> None:
        commands = run_checks.specialty_commands(self.gate, ROOT)
        self.assertEqual(len(commands), 1)
        argv, cwd, env = commands[0]
        self.assertIsNone(cwd)
        self.assertIsNone(env)
        self.assertEqual(argv[:2], ["cargo", "clippy"])
        cargo_args = argv[: argv.index("--")] if "--" in argv else argv
        for flag in ("--locked", "--workspace", "--all-features", "--all-targets"):
            self.assertIn(flag, cargo_args)
        # Any package selection or exclusion breaks the unification the gate
        # exists to build, and so does a manifest narrower than the root.
        for flag in ("-p", "--package", "--exclude", "--manifest-path", "--features", "-F"):
            self.assertFalse(
                any(value == flag or value.startswith(flag + "=") for value in cargo_args),
                f"{flag} narrows the workspace build",
            )
        self.assertEqual(argv[argv.index("--") :], ["--", "-D", "warnings"])

    def test_gate_selects_on_every_change_and_runs_on_main(self) -> None:
        policy = json.loads((ROOT / "scripts/ci/policy.json").read_text())
        rules = {rule["gate"]: rule["patterns"] for rule in policy["specialty_rules"]}
        self.assertIn(self.gate, rules)
        for path in (
            "README.md",
            "docs/notes.txt",
            ".github/workflows/pr-gate.yml",
            "scripts/ci/run_checks.py",
            "Cargo.lock",
            "crates/sip/rvoip-sip/src/lib.rs",
            "examples/01-quickstart-p2p/src/main.rs",
        ):
            with self.subTest(path=path):
                self.assertTrue(coverage.pr_plan.matches_any(path, rules[self.gate]))
        main = (ROOT / ".github/workflows/main-ci.yml").read_text()
        self.assertIn(f"--specialty-gate {self.gate}\n", main)

    def test_gate_fetches_the_byova_protos_and_installs_libvpx(self) -> None:
        # `rvoip-webex-byova/grpc` and rvoip-webrtc's `client-video-vp8` are
        # both in `--all-features`; the job must provision them.
        argv = run_checks.specialty_commands(self.gate, ROOT)[0][0]
        self.assertTrue(run_checks.webex_byova_protos.needed(argv))
        self.assertIn(self.gate, run_checks.LIBVPX_GATES)


class FeatureGateModelTests(unittest.TestCase):
    """The command and cfg model the coverage test relies on."""

    @staticmethod
    def package(name: str, features: dict[str, list[str]]) -> coverage.Package:
        return coverage.Package(
            name, ROOT / "crates/x" / name / "Cargo.toml", features, "Cargo.toml"
        )

    def target(self, kind: str = "test", required: tuple[str, ...] = (), cfg=None):
        return coverage.Target("alpha", kind, "t", required, cfg, "crates/x/alpha/tests/t.rs")

    def setUp(self) -> None:
        self.packages = {
            "alpha": self.package(
                "alpha", {"default": ["a"], "a": [], "b": ["c", "dep:z"], "c": []}
            ),
            "beta": self.package("beta", {"default": [], "b": []}),
        }

    def invocation(self, argv: list[str]) -> coverage.Invocation:
        invocation = coverage.parse_invocation(argv, gate="g", tier="ci")
        assert invocation is not None
        return invocation

    def test_default_shard_does_not_reach_a_required_feature(self) -> None:
        shard = self.invocation(
            ["cargo", "test", "--locked", "--lib", "--tests", "-p", "alpha", "-p", "beta"]
        )
        self.assertEqual(coverage.covers(shard, self.target(required=("a",)), self.packages), "run")
        self.assertIsNone(coverage.covers(shard, self.target(required=("b",)), self.packages))

    def test_features_close_over_implications(self) -> None:
        gate = self.invocation(["cargo", "test", "-p", "alpha", "--features", "b", "--tests"])
        self.assertEqual(coverage.covers(gate, self.target(required=("c",)), self.packages), "run")
        prefixed = self.invocation(
            ["cargo", "test", "-p", "alpha", "-p", "beta", "--features", "alpha/b"]
        )
        self.assertEqual(
            coverage.covers(prefixed, self.target(required=("c",)), self.packages), "run"
        )
        no_default = self.invocation(
            ["cargo", "test", "-p", "alpha", "--no-default-features", "--tests"]
        )
        self.assertIsNone(coverage.covers(no_default, self.target(required=("a",)), self.packages))

    def test_build_only_commands_do_not_count_as_running_a_test(self) -> None:
        clippy = self.invocation(["cargo", "clippy", "-p", "alpha", "--all-features", "--all-targets"])
        self.assertEqual(coverage.covers(clippy, self.target(required=("b",)), self.packages), "build")
        no_run = self.invocation(["cargo", "test", "-p", "alpha", "--all-features", "--no-run"])
        self.assertEqual(coverage.covers(no_run, self.target(required=("b",)), self.packages), "build")
        example = self.target(kind="example", required=("b",))
        examples = self.invocation(["cargo", "test", "-p", "alpha", "--features", "b", "--examples"])
        self.assertEqual(coverage.covers(examples, example, self.packages), "build")

    def test_named_targets_and_globs(self) -> None:
        named = self.invocation(["cargo", "test", "-p", "alpha", "--all-features", "--test", "t*"])
        self.assertEqual(coverage.covers(named, self.target(required=("b",)), self.packages), "run")
        other = self.invocation(["cargo", "test", "-p", "alpha", "--all-features", "--test", "u"])
        self.assertIsNone(coverage.covers(other, self.target(required=("b",)), self.packages))

    def test_cfg_predicates(self) -> None:
        self.assertTrue(coverage.evaluate_cfg('feature = "a"', {"a"}))
        self.assertFalse(coverage.evaluate_cfg('feature = "a"', set()))
        self.assertTrue(coverage.evaluate_cfg('any(feature = "a", feature = "b")', {"b"}))
        self.assertFalse(coverage.evaluate_cfg('all(feature = "a", feature = "b")', {"b"}))
        self.assertTrue(coverage.evaluate_cfg('all(unix, feature = "a",)', {"a"}))
        self.assertFalse(coverage.evaluate_cfg('not(feature = "a")', {"a"}))
        self.assertEqual(
            coverage.cfg_features('all(feature = "b", any(feature = "a"))'), ["a", "b"]
        )
        shard = self.invocation(["cargo", "test", "-p", "alpha", "--tests"])
        self.assertIsNone(
            coverage.covers(shard, self.target(cfg='feature = "b"'), self.packages)
        )

    def test_crate_cfg_reads_multiline_inner_attributes(self) -> None:
        source = ROOT / "crates/webrtc/rvoip-webrtc/tests/sip_webrtc_acceptance.rs"
        predicate = coverage.crate_cfg(source)
        self.assertIsNotNone(predicate)
        self.assertIn("tls-rustls", coverage.cfg_features(predicate))

    def test_workflow_runs_join_continuations(self) -> None:
        commands = coverage.workflow_commands(ROOT / ".github/workflows/nightly-interop.yml")
        interop = [command for command in commands if "browser_interop" in command]
        self.assertTrue(interop)
        self.assertTrue(all("interop-browser" in command for command in interop))


if __name__ == "__main__":
    unittest.main()
