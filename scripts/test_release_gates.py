from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import shutil
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

    def test_every_profile_is_dependency_closed(self) -> None:
        by_id = {gate["id"]: gate for gate in self.catalog["gates"]}
        for profile, selected in self.catalog["profiles"].items():
            selected_ids = set(selected)
            with self.subTest(profile=profile):
                self.assertFalse(
                    {
                        gate_id: set(by_id[gate_id]["dependencies"]) - selected_ids
                        for gate_id in selected
                        if set(by_id[gate_id]["dependencies"]) - selected_ids
                    }
                )

    def test_fuzz_gates_select_their_fuzz_crate_explicitly(self) -> None:
        fuzz_gates = [
            gate
            for gate in self.catalog["gates"]
            if gate["id"].startswith("security.fuzz-")
        ]
        self.assertEqual(len(fuzz_gates), 10)
        for gate in fuzz_gates:
            with self.subTest(gate=gate["id"]):
                self.assertNotIn("--manifest-path", gate["command"])
                index = gate["command"].index("--fuzz-dir")
                self.assertIn(
                    gate["command"][index + 1],
                    {
                        "{workspace}/crates/sip/fuzz",
                        "{workspace}/crates/media/fuzz",
                    },
                )
                target_index = gate["command"].index("--target")
                self.assertEqual(
                    gate["command"][target_index + 1],
                    "x86_64-unknown-linux-gnu",
                )

        remote_fuzz_gates = [
            gate
            for gate in self.catalog["gates"]
            if gate["id"].startswith("security.remote-fuzz-")
        ]
        self.assertEqual(len(remote_fuzz_gates), 10)
        for gate in remote_fuzz_gates:
            with self.subTest(gate=gate["id"]):
                self.assertNotIn("--manifest-path", gate["command"])
                self.assertIn("--fuzz-dir", gate["command"])
                target_index = gate["command"].index("--target")
                self.assertEqual(
                    gate["command"][target_index + 1],
                    "x86_64-unknown-linux-gnu",
                )

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
        self.assertEqual(len(soak_shards), 2)
        self.assertTrue(all(len(shard["gates"]) == 1 for shard in soak_shards))
        self.assertNotIn(
            "perf.media-burst-matrix",
            {gate for shard in soak_shards for gate in shard["gates"]},
        )

    def test_media_burst_scenarios_run_on_independent_workers(self) -> None:
        scenario_ids = {
            gate["id"]
            for gate in self.catalog["gates"]
            if gate["id"].startswith("perf.media-burst.")
        }
        self.assertEqual(len(scenario_ids), 7)
        by_id = {gate["id"]: gate for gate in self.catalog["gates"]}
        matrix = gates.matrix_for(
            [{"id": gate_id, "decision": "RUN"} for gate_id in scenario_ids],
            by_id,
        )
        self.assertEqual(len(matrix), 7)
        self.assertTrue(all(len(shard["gates"]) == 1 for shard in matrix))
        self.assertTrue(all(shard["machine_type"] == "n2-standard-4" for shard in matrix))
        aggregate = by_id["perf.media-burst-matrix"]
        self.assertEqual(aggregate["executor"], "aggregate")
        self.assertEqual(set(aggregate["dependencies"]), scenario_ids)

    def test_remote_release_requires_bridgefu_chromium_dtmf_regression(self) -> None:
        gate_id = "interop.browser-dtmf"
        self.assertIn(gate_id, self.catalog["profiles"]["remote-release"])
        gate = next(gate for gate in self.catalog["gates"] if gate["id"] == gate_id)
        self.assertIn("browser_interop", gate["command"])
        self.assertIn("--include-ignored", gate["command"])
        matrix = gates.matrix_for(
            [{"id": gate_id, "decision": "RUN"}],
            {item["id"]: item for item in self.catalog["gates"]},
        )
        self.assertEqual(len(matrix), 1)
        self.assertTrue(matrix[0]["hosted"])
        self.assertTrue(matrix[0]["needs_chromium"])

    def test_locked_standalone_example_gates_have_reproducible_inputs(self) -> None:
        example_gates = [
            gate
            for gate in self.catalog["gates"]
            if gate["id"].startswith("test.example-")
        ]
        self.assertEqual(len(example_gates), 13)
        for gate in example_gates:
            command = gate["command"]
            with self.subTest(gate=gate["id"]):
                self.assertIn("--locked", command)
                manifest_index = command.index("--manifest-path") + 1
                manifest = ROOT / command[manifest_index].replace("{workspace}/", "")
                lockfile = ROOT / "examples/Cargo.lock"
                self.assertTrue(lockfile.is_file(), f"missing {lockfile}")
                self.assertIn('rust-version = "1.91"', manifest.read_text())

    def test_rustdoc_warning_policy_is_one_environment_argument(self) -> None:
        gate = next(
            gate for gate in self.catalog["gates"] if gate["id"] == "build.rustdoc"
        )
        self.assertEqual(
            gate["command"][:3], ["env", "RUSTDOCFLAGS=-D warnings", "cargo"]
        )
        self.assertNotIn("warnings", gate["command"])

    def test_format_gate_does_not_receive_unsupported_locked_flag(self) -> None:
        gate = next(
            gate for gate in self.catalog["gates"] if gate["id"] == "build.format"
        )
        self.assertEqual(gate["command"][:2], ["cargo", "fmt"])
        self.assertNotIn("--locked", gate["command"])
        self.assertEqual(
            builder.insert_locked(["cargo", "+1.91.0", "fmt", "--all"]),
            ["cargo", "+1.91.0", "fmt", "--all"],
        )

    def test_gcp_gates_allow_for_cold_release_builds(self) -> None:
        for gate in self.catalog["gates"]:
            if gate["resource_class"].startswith("gcp-"):
                with self.subTest(gate=gate["id"]):
                    self.assertGreaterEqual(gate["timeout_minutes"], 20)

    def test_interop_gate_digests_include_tracked_pbx_lifecycle(self) -> None:
        interop = [
            gate
            for gate in self.catalog["gates"]
            if gate["id"].startswith("interop.") and gate.get("legacy")
        ]
        self.assertTrue(interop)
        for gate in interop:
            with self.subTest(gate=gate["id"]):
                self.assertIn(
                    "infra/release-runners/interop-lifecycle.sh",
                    gate["affected_paths"],
                )
                self.assertIn("infra/release-runners/pbx/**", gate["affected_paths"])

    def test_definition_digest_is_key_order_independent(self) -> None:
        first = {"id": "a", "command": ["true"]}
        second = {"command": ["true"], "id": "a"}
        self.assertEqual(gates.definition_digest(first), gates.definition_digest(second))

    def test_unrelated_catalog_changes_do_not_invalidate_a_gate(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            runner = root / "scripts/release/gates.py"
            runner.parent.mkdir(parents=True)
            runner.write_text("runner-v1\n")
            gate = {
                "id": "gate-a",
                "resource_class": "github-standard",
                "affected_paths": [],
                "affected_crates": [],
                "dependencies": [],
                "command": ["true"],
            }
            definitions = {"gate-a": gate}
            first = gates.input_record(
                root=root,
                gate=gate,
                environment_id="environment-v1",
                files=["scripts/release/gates.py"],
                package_roots={},
                package_dependencies={},
                gate_definitions=definitions,
            )
            unrelated = {**definitions, "gate-b": {"id": "gate-b", "command": ["false"]}}
            second = gates.input_record(
                root=root,
                gate=gate,
                environment_id="environment-v1",
                files=["scripts/release/gates.py"],
                package_roots={},
                package_dependencies={},
                gate_definitions=unrelated,
            )
            self.assertEqual(first, second)

    def test_gcp_gate_digest_includes_ephemeral_worker_definition(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            runner = root / "scripts/release/gates.py"
            startup = root / "infra/release-runners/gcp-release-startup.sh"
            runner.parent.mkdir(parents=True)
            startup.parent.mkdir(parents=True)
            runner.write_text("runner-v1\n")
            startup.write_text("startup-v1\n")
            gate = {
                "id": "gate-a",
                "resource_class": "gcp-performance",
                "affected_paths": [],
                "affected_crates": [],
                "dependencies": [],
                "command": ["true"],
            }
            definitions = {"gate-a": gate}

            def record() -> dict:
                return gates.input_record(
                    root=root,
                    gate=gate,
                    environment_id="environment-v1",
                    files=[
                        "scripts/release/gates.py",
                        "infra/release-runners/gcp-release-startup.sh",
                    ],
                    package_roots={},
                    package_dependencies={},
                    gate_definitions=definitions,
                )

            first = record()
            startup.write_text("startup-v2\n")
            self.assertNotEqual(first["input_sha256"], record()["input_sha256"])

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

    def test_input_miss_invalidates_only_downstream_gate_closure(self) -> None:
        by_id = {
            "changed": {"dependencies": []},
            "consumer": {"dependencies": ["changed"]},
            "consumer-of-consumer": {"dependencies": ["consumer"]},
            "unrelated": {"dependencies": []},
        }
        reverse = gates.reverse_gate_dependencies(list(by_id), by_id)
        self.assertEqual(
            gates.dependent_closure({"changed"}, reverse),
            {"changed", "consumer", "consumer-of-consumer"},
        )

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

    def test_github_outputs_split_hosted_and_ephemeral_gcp_matrices(self) -> None:
        plan = {
            "matrix": [
                {"id": "hosted", "hosted": True},
                {"id": "gcp", "hosted": False},
            ],
            "gates": [{"decision": "RUN"}, {"decision": "REUSE"}],
            "candidate_sha": "c" * 40,
            "environment_id": "test",
        }
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "github-output"
            gates.write_github_output(output, plan)
            values = dict(line.split("=", 1) for line in output.read_text().splitlines())
        self.assertEqual(json.loads(values["hosted_matrix"])["include"], [plan["matrix"][0]])
        self.assertEqual(json.loads(values["gcp_matrix"])["include"], [plan["matrix"][1]])
        self.assertEqual(values["hosted_shard_count"], "1")
        self.assertEqual(values["gcp_shard_count"], "1")

    def test_collector_materializes_aggregate_only_after_dependencies_pass(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            dependency_dir = root / "dependency"
            dependency_dir.mkdir()
            log = dependency_dir / "command.log"
            log.write_text("pass\n")
            dependency = {
                "schema": gates.RECEIPT_SCHEMA,
                "gate_id": "gate.dependency",
                "candidate_sha": "c" * 40,
                "status": "PASS",
                "gate_definition_sha256": "d" * 64,
                "input_sha256": "i" * 64,
                "environment_sha256": "e" * 64,
                "log": {
                    "path": "dependency/command.log",
                    "sha256": gates.file_sha256(log),
                    "bytes": log.stat().st_size,
                },
            }
            (dependency_dir / "receipt.json").write_bytes(
                gates.canonical_bytes(dependency)
            )
            plan = {
                "schema": gates.PLAN_SCHEMA,
                "candidate_sha": "c" * 40,
                "profile": "test",
                "catalog_sha256": "x" * 64,
                "environment_id": "test",
                "gates": [
                    {
                        "id": "gate.dependency",
                        "decision": "RUN",
                        "executor": "argv",
                        "dependencies": [],
                        "gate_definition_sha256": "d" * 64,
                        "input_sha256": "i" * 64,
                        "environment_sha256": "e" * 64,
                    },
                    {
                        "id": "gate.aggregate",
                        "decision": "RUN",
                        "executor": "aggregate",
                        "dependencies": ["gate.dependency"],
                        "gate_definition_sha256": "a" * 64,
                        "input_sha256": "b" * 64,
                        "environment_sha256": "e" * 64,
                    },
                ],
            }
            output = root / "aggregate.json"
            self.assertEqual(
                gates.collect(plan=plan, evidence_root=root, output=output), 0
            )
            result = json.loads(output.read_text())
            self.assertEqual(result["status"], "PASS")
            self.assertEqual(result["gate_count"], 2)
            self.assertTrue((root / "collect-gate.aggregate/receipt.json").is_file())

            (dependency_dir / "receipt.json").unlink()
            self.assertEqual(
                gates.collect(plan=plan, evidence_root=root, output=output), 1
            )

    def test_performance_reconciliation_runs_real_baseline_audit(self) -> None:
        baseline_root = (
            ROOT / "crates/sip/rvoip-sip/perf-baselines/20260706T181609Z"
        )
        manifest = json.loads((baseline_root / "manifest.json").read_text())
        with tempfile.TemporaryDirectory() as directory:
            evidence = Path(directory) / "evidence"
            packaged = evidence / "baseline-gate/perf-regression-baseline"
            packaged.mkdir(parents=True)
            shutil.copy2(baseline_root / "manifest.json", packaged / "manifest.json")
            for relative in manifest["comparison_paths"]:
                source = baseline_root / relative
                baseline_target = packaged / "perf-results" / relative
                current_target = evidence / "_perf-results/shard" / relative
                baseline_target.parent.mkdir(parents=True, exist_ok=True)
                current_target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(source, baseline_target)
                shutil.copy2(source, current_target)
            artifact = evidence / "collect-report.regression-audit"
            artifact.mkdir()
            result = gates.reconcile_performance_regression(
                ROOT, evidence, artifact
            )
            self.assertTrue((artifact / "perf-audit.md").is_file())
            self.assertEqual(
                set(result["selected_results"]), set(manifest["comparison_paths"])
            )


if __name__ == "__main__":
    unittest.main()
