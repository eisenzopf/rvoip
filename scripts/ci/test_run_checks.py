from __future__ import annotations

import importlib.util
from pathlib import Path
import sys
import unittest


SCRIPT = Path(__file__).with_name("run_checks.py")
SPEC = importlib.util.spec_from_file_location("run_checks", SCRIPT)
assert SPEC and SPEC.loader
run_checks = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = run_checks
SPEC.loader.exec_module(run_checks)


class RunChecksTests(unittest.TestCase):
    def test_package_arguments_reject_shell_metacharacters(self) -> None:
        with self.assertRaises(run_checks.CheckError):
            run_checks.package_args("safe; touch compromised")

    def test_shard_test_and_clippy_are_independent_commands(self) -> None:
        tests = run_checks.shard_test_commands("alpha,beta")
        clippy = run_checks.shard_clippy_commands("alpha,beta")
        self.assertEqual(len(tests), 1)
        self.assertEqual(tests[0][0][0:2], ["cargo", "test"])
        self.assertEqual(len(clippy), 1)
        self.assertEqual(clippy[0][0][0:2], ["cargo", "clippy"])

    def test_sip_core_and_integration_commands_are_independent(self) -> None:
        core = run_checks.sip_core_commands()
        integration = run_checks.sip_integration_commands("beta,alpha")
        self.assertIn("--lib", core[0][0])
        self.assertNotIn("--tests", core[0][0])
        self.assertEqual(
            integration[0][0],
            [
                "cargo",
                "test",
                "--locked",
                "-p",
                "rvoip-sip",
                "--test",
                "alpha",
                "--test",
                "beta",
            ],
        )

    def test_pr_sip_core_and_clippy_are_independent_lanes(self) -> None:
        core = run_checks.sip_core_commands()
        clippy = run_checks.sip_clippy_commands()
        self.assertEqual(len(core), 1)
        self.assertEqual(core[0][0][0:2], ["cargo", "test"])
        self.assertEqual(len(clippy), 1)
        self.assertEqual(clippy[0][0][0:2], ["cargo", "clippy"])
        self.assertIn("--all-targets", clippy[0][0])

    def test_sip_fixture_lane_prebuilds_examples_in_the_shared_target(self) -> None:
        root = Path("/workspace")
        commands = run_checks.sip_fixture_commands(
            "cancel_integration,blind_transfer_integration",
            "cancel_bob,cancel_alice",
            root,
        )
        self.assertEqual(commands[0][0][0:2], ["cargo", "build"])
        self.assertEqual(
            commands[0][0][-4:],
            ["--example", "cancel_alice", "--example", "cancel_bob"],
        )
        self.assertEqual(commands[1][0][0:2], ["cargo", "test"])
        self.assertEqual(
            commands[1][2]["RVOIP_SIP_PREBUILT_EXAMPLE_DIR"],
            "/workspace/target/debug/examples",
        )

    def test_sip_target_arguments_reject_shell_metacharacters(self) -> None:
        with self.assertRaises(run_checks.CheckError):
            run_checks.sip_integration_commands("safe; touch compromised")
        with self.assertRaises(run_checks.CheckError):
            run_checks.sip_fixture_commands("safe", "unsafe;example", Path("/workspace"))

    def test_example_smoke_preserves_the_hardware_free_matrix(self) -> None:
        commands = run_checks.specialty_commands("examples-smoke", Path("/workspace"))
        self.assertEqual(len(commands), 10)
        self.assertEqual(sum(command[0][0:2] == ["cargo", "build"] for command in commands), 5)
        self.assertEqual(
            sum(any("run_demo.sh" in value for value in command[0]) for command in commands),
            5,
        )

    def test_example_smoke_partition_runs_one_release_demo(self) -> None:
        commands = run_checks.specialty_commands(
            "example-smoke--07-secure-call-srtp", Path("/workspace")
        )
        self.assertEqual(len(commands), 2)
        self.assertEqual(commands[0][0], ["cargo", "build", "--release", "--locked"])
        self.assertEqual(commands[1][0], ["timeout", "120", "./run_demo.sh"])
        self.assertEqual(
            commands[0][1], Path("/workspace/examples/07-secure-call-srtp")
        )

    def test_vcon_gate_contains_live_store_and_boundary_checks(self) -> None:
        commands = run_checks.specialty_commands("vcon-postgres", Path("/workspace"))
        argv = [item[0] for item in commands]
        self.assertTrue(any("rvoip-vcon-postgres" in command for command in argv))
        self.assertTrue(any("e2e_full_stack" in command for command in argv))
        self.assertTrue(any("voip-3" in command for command in argv))

    def test_amazon_connect_gate_compiles_the_optional_control_plane(self) -> None:
        commands = run_checks.specialty_commands(
            "amazon-connect-aws-control", Path("/workspace")
        )
        self.assertEqual(len(commands), 2)
        self.assertIn("aws-control", commands[0][0])
        self.assertIn("rvoip-amazon-connect", commands[0][0])
        self.assertIn("--all-features", commands[1][0])
        self.assertIn(
            "/workspace/examples/13-sip-to-amazon-connect/Cargo.toml",
            commands[1][0],
        )

    def test_otel_gate_compiles_the_optional_exporter(self) -> None:
        commands = run_checks.specialty_commands("infra-otel", Path("/workspace"))
        self.assertEqual(len(commands), 1)
        self.assertIn("rvoip-infra-common", commands[0][0])
        self.assertIn("otel", commands[0][0])
        self.assertIn("--all-targets", commands[0][0])

    def test_codec_gate_runs_what_the_default_feature_shards_compile_out(self) -> None:
        commands = run_checks.specialty_commands("codec-features", Path("/workspace"))
        argv = [item[0] for item in commands]
        self.assertEqual(len(argv), 4)
        # Every command asks for all features. A command here without it is a
        # command that duplicates the shard and proves nothing.
        for command in argv:
            self.assertIn("--all-features", command)
        for package in ("rvoip-codec-core", "rvoip-media-core"):
            with self.subTest(package=package):
                owned = [command for command in argv if package in command]
                self.assertEqual(len(owned), 2)
                self.assertEqual(
                    sorted(command[1] for command in owned), ["clippy", "test"]
                )

    def test_shards_alone_never_reach_the_optional_codecs(self) -> None:
        # The reason the gate above exists, asserted rather than assumed: the
        # shard that owns rvoip-codec-core builds it with its default features,
        # which is g711 only. Should the shards ever gain --all-features, this
        # fails and the specialty gate becomes removable.
        for commands in (
            run_checks.shard_commands("rvoip-codec-core,rvoip-media-core"),
            run_checks.shard_test_commands("rvoip-codec-core"),
        ):
            for command, _, _ in commands:
                self.assertNotIn("--all-features", command)

    def test_release_tooling_gate_owns_proxy_harness_tests(self) -> None:
        commands = run_checks.specialty_commands("release-tooling", Path("/workspace"))
        argv = [item[0] for item in commands]
        self.assertIn(
            [
                "python3",
                "-m",
                "unittest",
                "discover",
                "-s",
                "crates/sip/sip-proxy/tests/interop/scripts",
                "-p",
                "test_*.py",
            ],
            argv,
        )


if __name__ == "__main__":
    unittest.main()
