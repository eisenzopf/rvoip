from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent.parent


def load_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


gates = load_module("release_gates", ROOT / "scripts/release/gates.py")
builder = load_module(
    "build_gate_catalog", ROOT / "scripts/release/build_gate_catalog.py"
)


class GateFrameworkTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.catalog = json.loads((ROOT / "scripts/release/gates.json").read_text())

    def test_catalog_maps_108_legacy_and_44_core_gates(self) -> None:
        gates.validate_catalog(ROOT, self.catalog)
        self.assertEqual(
            sum(bool(gate.get("legacy")) for gate in self.catalog["gates"]), 108
        )
        self.assertEqual(
            sum(gate["id"].startswith("core.") for gate in self.catalog["gates"]),
            44,
        )
        self.assertEqual(
            self.catalog["remote_release_legacy_coverage"]["required_legacy_count"],
            108,
        )

    def test_checked_in_catalog_is_reproducible(self) -> None:
        generated = builder.build_catalog(
            ROOT,
            ROOT
            / "crates/sip/rvoip-sip/docs/releases/beta/20260724T231400Z/gate-results.json",
        )
        self.assertEqual(generated, self.catalog)

    def test_fuzz_gates_run_from_their_fuzz_crate(self) -> None:
        fuzz_gates = [
            gate
            for gate in self.catalog["gates"]
            if gate["id"].startswith("security.fuzz-")
        ]
        self.assertEqual(len(fuzz_gates), 10)
        for gate in fuzz_gates:
            with self.subTest(gate=gate["id"]):
                self.assertIn(
                    gate["working_directory"],
                    {"crates/sip/fuzz", "crates/media/fuzz"},
                )
                self.assertNotIn("--manifest-path", gate["command"])

    def test_multi_hour_performance_gates_are_isolated_from_perf_batch(self) -> None:
        by_id = {gate["id"]: gate for gate in self.catalog["gates"]}
        soak_ids = {
            "perf.monolithic-soak",
            "perf.media-burst-matrix",
            "perf.soak-candidate",
        }
        self.assertEqual(
            {by_id[gate_id]["resource_class"] for gate_id in soak_ids},
            {"gcp-performance-soak"},
        )
        matrix = gates.matrix_for(
            [
                {"id": gate_id, "decision": "RUN"}
                for gate_id in sorted(soak_ids | {"perf.resiliency-all"})
            ],
            by_id,
        )
        soak_shards = [
            shard for shard in matrix if shard["resource_class"] == "gcp-performance-soak"
        ]
        self.assertEqual(len(soak_shards), 3)
        self.assertTrue(all(len(shard["gates"]) == 1 for shard in soak_shards))

    def test_definition_digest_is_key_order_independent(self) -> None:
        first = {"id": "a", "command": ["true"]}
        second = {"command": ["true"], "id": "a"}
        self.assertEqual(gates.definition_digest(first), gates.definition_digest(second))

    def test_exact_reuse_rejects_failure_drift_and_always_fresh(self) -> None:
        gate = {"always_fresh": False}
        inputs = {
            "gate_definition_sha256": "d" * 64,
            "input_sha256": "i" * 64,
            "environment_sha256": "e" * 64,
        }
        receipt = {
            "status": "PASS",
            **inputs,
        }
        self.assertIs(
            gates.exact_reusable([receipt], gate, inputs, "environment"), receipt
        )
        for key in ("status", "gate_definition_sha256", "input_sha256", "environment_sha256"):
            changed = dict(receipt)
            changed[key] = "FAIL" if key == "status" else "x" * 64
            with self.subTest(key=key):
                self.assertIsNone(
                    gates.exact_reusable([changed], gate, inputs, "environment")
                )
        self.assertIsNone(
            gates.exact_reusable(
                [receipt], {"always_fresh": True}, inputs, "environment"
            )
        )

    def test_failed_gate_invalidates_reverse_dependency_closure(self) -> None:
        by_id = {
            "a": {"dependencies": []},
            "b": {"dependencies": ["a"]},
            "c": {"dependencies": ["b"]},
            "d": {"dependencies": []},
        }
        reverse = gates.reverse_gate_dependencies(list(by_id), by_id)
        self.assertEqual(gates.dependent_closure({"a"}, reverse), {"a", "b", "c"})

    def test_shard_runs_gate_dependencies_before_dependents(self) -> None:
        by_id = {
            "c": {"dependencies": ["b"]},
            "b": {"dependencies": ["a"]},
            "a": {"dependencies": []},
            "independent": {"dependencies": []},
        }
        self.assertEqual(
            gates.dependency_order(list(by_id), by_id),
            ["a", "b", "c", "independent"],
        )

    def test_unmapped_changes_fail_closed(self) -> None:
        selected = [
            {
                "kind": "cargo",
                "always_fresh": False,
                "affected_paths": ["crates/known/**"],
            },
            {
                "kind": "remote",
                "always_fresh": True,
                "affected_paths": ["**"],
            },
        ]
        self.assertEqual(
            gates.unknown_change(["crates/unknown/lib.rs"], selected),
            ["crates/unknown/lib.rs"],
        )

    def test_collector_rejects_tampered_log_and_accepts_exact_receipt(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            gate_dir = root / "gate-a"
            gate_dir.mkdir()
            log = gate_dir / "command.log"
            log.write_text("pass\n")
            receipt = {
                "schema": gates.RECEIPT_SCHEMA,
                "gate_id": "gate.a",
                "candidate_sha": "c" * 40,
                "status": "PASS",
                "gate_definition_sha256": "d" * 64,
                "input_sha256": "i" * 64,
                "environment_sha256": "e" * 64,
                "log": {
                    "path": "gate-a/command.log",
                    "sha256": gates.file_sha256(log),
                    "bytes": log.stat().st_size,
                },
            }
            (gate_dir / "receipt.json").write_bytes(gates.canonical_bytes(receipt))
            plan = {
                "schema": gates.PLAN_SCHEMA,
                "candidate_sha": "c" * 40,
                "profile": "test",
                "catalog_sha256": "x" * 64,
                "environment_id": "test",
                "gates": [
                    {
                        "id": "gate.a",
                        "decision": "RUN",
                        "gate_definition_sha256": "d" * 64,
                        "input_sha256": "i" * 64,
                        "environment_sha256": "e" * 64,
                    }
                ],
            }
            output = root / "aggregate.json"
            self.assertEqual(
                gates.collect(plan=plan, evidence_root=root, output=output), 0
            )
            log.write_text("tampered\n")
            self.assertEqual(
                gates.collect(plan=plan, evidence_root=root, output=output), 1
            )

    def test_gate_command_timeout_writes_a_failure_log(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "command.log"
            status = gates.run_to_log(
                [sys.executable, "-c", "import time; time.sleep(2)"],
                cwd=Path(directory),
                log_path=log,
                timeout_seconds=1,
            )
            self.assertEqual(status, 124)
            self.assertIn("exceeded catalogued timeout", log.read_text())


if __name__ == "__main__":
    unittest.main()
