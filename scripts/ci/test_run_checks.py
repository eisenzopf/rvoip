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


if __name__ == "__main__":
    unittest.main()
