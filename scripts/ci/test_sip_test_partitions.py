from __future__ import annotations

import importlib.util
from pathlib import Path
import sys
import unittest


SCRIPT = Path(__file__).with_name("sip_test_partitions.py")
SPEC = importlib.util.spec_from_file_location("sip_test_partitions", SCRIPT)
assert SPEC and SPEC.loader
partitions = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = partitions
SPEC.loader.exec_module(partitions)


class SipTestPartitionTests(unittest.TestCase):
    def test_inventory_matches_cargo_tests_and_skips_feature_gated_targets(self) -> None:
        metadata = {
            "packages": [
                {
                    "name": "rvoip-sip",
                    "targets": [
                        {"name": "ordinary", "kind": ["test"], "required-features": []},
                        {
                            "name": "feature_only",
                            "kind": ["test"],
                            "required-features": ["perf-tests"],
                        },
                        {"name": "library", "kind": ["lib"], "required-features": []},
                    ],
                }
            ]
        }
        self.assertEqual(partitions.eligible_targets(metadata, "rvoip-sip"), ["ordinary"])

    def test_every_target_is_assigned_exactly_once_and_long_test_is_isolated(self) -> None:
        targets = ["audio_roundtrip_integration", *[f"test_{index}" for index in range(12)]]
        result = partitions.partition_targets(targets, 3)
        assigned = [name for item in result for name in item["targets"]]
        self.assertEqual(sorted(assigned), sorted(targets))
        self.assertEqual(len(assigned), len(set(assigned)))
        audio_partition = next(
            item for item in result if "audio_roundtrip_integration" in item["targets"]
        )
        self.assertEqual(audio_partition["targets"], ["audio_roundtrip_integration"])

    def test_partitioning_is_deterministic(self) -> None:
        targets = ["zeta", "alpha", "bridge_roundtrip_integration", "beta"]
        self.assertEqual(
            partitions.partition_targets(targets, 2),
            partitions.partition_targets(list(reversed(targets)), 2),
        )


if __name__ == "__main__":
    unittest.main()
