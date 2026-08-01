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

    def test_example_smoke_preserves_the_hardware_free_matrix(self) -> None:
        commands = run_checks.specialty_commands("examples-smoke", Path("/workspace"))
        self.assertEqual(len(commands), 10)
        self.assertEqual(sum(command[0][0:2] == ["cargo", "build"] for command in commands), 5)
        self.assertEqual(
            sum(any("run_demo.sh" in value for value in command[0]) for command in commands),
            5,
        )

    def test_vcon_gate_contains_live_store_and_boundary_checks(self) -> None:
        commands = run_checks.specialty_commands("vcon-postgres", Path("/workspace"))
        argv = [item[0] for item in commands]
        self.assertTrue(any("rvoip-vcon-postgres" in command for command in argv))
        self.assertTrue(any("e2e_full_stack" in command for command in argv))
        self.assertTrue(any("voip-3" in command for command in argv))


if __name__ == "__main__":
    unittest.main()
