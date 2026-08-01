from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("compare_nextest_inventory.py")
SPEC = importlib.util.spec_from_file_location("compare_nextest_inventory", SCRIPT)
assert SPEC and SPEC.loader
inventory = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = inventory
SPEC.loader.exec_module(inventory)


class InventoryTests(unittest.TestCase):
    def test_counts_cargo_and_nested_nextest_suites(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            cargo = root / "cargo.txt"
            nextest = root / "nextest.json"
            output = root / "parity.json"
            cargo.write_text("one: test\ntwo: test\n0 tests, 0 benchmarks\n")
            nextest.write_text(
                json.dumps(
                    {
                        "rust-suites": {
                            "binary": {
                                "testcases": {"one": {}, "two": {}}
                            }
                        }
                    }
                )
            )
            self.assertEqual(
                inventory.main(
                    [
                        "--cargo",
                        str(cargo),
                        "--nextest",
                        str(nextest),
                        "--output",
                        str(output),
                    ]
                ),
                0,
            )
            self.assertEqual(json.loads(output.read_text())["status"], "PASS")


if __name__ == "__main__":
    unittest.main()
