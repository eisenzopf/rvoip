from __future__ import annotations

import importlib.util
import fnmatch
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
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
fanout = load_module("release_aws_fanout", ROOT / "scripts/release/aws_fanout.py")
run_checks = load_module("ci_run_checks", ROOT / "scripts/ci/run_checks.py")


SIP_MANIFEST = ROOT / "crates/sip/rvoip-sip/Cargo.toml"
TEST_HOOKS_GUARD = re.compile(r'cfg\((?:all\()?feature\s*=\s*"test-hooks"')
BROAD_TARGET_FLAGS = {"--lib", "--tests", "--doc", "--examples", "--bins", "--all-targets"}


def rvoip_sip_targets() -> dict[tuple[str, str], Path]:
    """Every rvoip-sip test and example target name mapped to its source file."""

    manifest = tomllib.loads(SIP_MANIFEST.read_text())
    crate = SIP_MANIFEST.parent
    targets: dict[tuple[str, str], Path] = {}
    for kind, directory in (("test", "tests"), ("example", "examples")):
        for path in (crate / directory).glob("*.rs"):
            targets[(kind, path.stem)] = path
        for entry in manifest.get(kind, []):
            default = crate / directory / f"{entry['name']}.rs"
            targets[(kind, entry["name"])] = crate / entry.get("path", default)
    return targets


def cargo_rvoip_sip_selection(command: list[str]) -> tuple[set[str], list[tuple[str, str]], bool] | None:
    """Return (broad flags, explicit targets, has test-hooks) for an rvoip-sip cargo gate."""

    if "cargo" not in command:
        return None
    argv = command[command.index("cargo") :]
    if len(argv) < 2 or argv[1] not in {"test", "build", "check", "clippy"}:
        return None
    packages = {argv[index + 1] for index, value in enumerate(argv[:-1]) if value in {"-p", "--package"}}
    if "rvoip-sip" not in packages:
        return None
    features: set[str] = set()
    for index, value in enumerate(argv[:-1]):
        if value == "--features":
            features.update(part.split("/")[-1] for part in argv[index + 1].split(","))
    explicit = [
        (argv[index].removeprefix("--"), argv[index + 1])
        for index in range(len(argv) - 1)
        if argv[index] in {"--test", "--example"}
    ]
    broad = BROAD_TARGET_FLAGS.intersection(argv)
    if not broad and not explicit:
        broad = {"<default targets>"}
    return broad, explicit, "test-hooks" in features or "--all-features" in argv


class GateFrameworkTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.catalog = json.loads((ROOT / "scripts/release/gates.json").read_text())

    def test_catalog_maps_108_legacy_and_all_workspace_core_gates(self) -> None:
        gates.validate_catalog(ROOT, self.catalog)
        self.assertEqual(
            sum(bool(gate.get("legacy")) for gate in self.catalog["gates"]), 108
        )
        self.assertEqual(
            sum(gate["id"].startswith("core.") for gate in self.catalog["gates"]),
            self.catalog["workspace_package_count"],
        )
        roots, _ = gates.workspace_graph(ROOT)
        self.assertEqual(
            {gate["id"].removeprefix("core.") for gate in self.catalog["gates"] if gate["id"].startswith("core.")},
            set(roots),
        )
        self.assertEqual(
            self.catalog["remote_release_legacy_coverage"]["required_legacy_count"],
            108,
        )
        core_gates = [
            gate for gate in self.catalog["gates"] if gate["id"].startswith("core.")
        ]
        self.assertTrue(
            all("scripts/ci/run_checks.py" in gate["affected_paths"] for gate in core_gates)
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

    def test_release_qualification_compiles_every_facade_feature_bundle(self) -> None:
        gate = next(
            gate
            for gate in self.catalog["gates"]
            if gate["id"] == "build.facade-feature-bundles"
        )
        self.assertIn(gate["id"], self.catalog["profiles"]["remote-release"])
        self.assertEqual(gate["dependencies"], ["source.remote-clean"])
        self.assertIn("nested-ci-receipt.json", gate["expected_outputs"])
        self.assertEqual(
            gate["command"],
            [
                "python3",
                "scripts/ci/run_checks.py",
                "specialty",
                "--name",
                "release-facade-feature-bundles",
                "--gate",
                "facade-feature-bundles",
                "--output",
                "{artifact_dir}/nested-ci-receipt.json",
            ],
        )

        bundle_commands = run_checks.specialty_commands("facade-feature-bundles", ROOT)
        with (ROOT / "crates/rvoip/Cargo.toml").open("rb") as handle:
            manifest = tomllib.load(handle)
        expected = [
            row["feature"]
            for row in manifest["package"]["metadata"]["rvoip"]["feature-bundles"]
        ]
        cargo_commands = [command[0] for command in bundle_commands[1:]]
        self.assertEqual([command[-1] for command in cargo_commands], expected)
        self.assertTrue(
            all("--no-default-features" in command for command in cargo_commands)
        )

    def test_evidence_helpers_are_bound_to_preparation_inputs(self) -> None:
        gate = next(
            gate
            for gate in self.catalog["gates"]
            if gate["id"] == "build.evidence-helper-tests"
        )
        for path in (
            ".github/workflows/release-prepare.yml",
            "crates/sip/rvoip-sip/docs/BETA_RELEASE_CHECKLIST.md",
            "docs/release/RELEASE_NOTES_NEXT.md",
            "crates/sip/rvoip-sip/scripts/full_beta_release.sh",
            "scripts/release.py",
            "scripts/test_release.py",
        ):
            with self.subTest(path=path):
                self.assertIn(path, gate["affected_paths"])

    def test_release_srtp_interop_gate_exercises_dtls_srtp_end_to_end(self) -> None:
        gate = next(
            gate
            for gate in self.catalog["gates"]
            if gate["id"] == "interop.remote-libsrtp"
        )
        self.assertIn(gate["id"], self.catalog["profiles"]["remote-release"])
        self.assertIn("rtp-interop", gate["command"])
        commands = [
            command[0]
            for command in run_checks.specialty_commands("rtp-interop", ROOT)
        ]
        rendered = [" ".join(command) for command in commands]
        self.assertTrue(any("test_libsrtp_interop.sh" in command for command in rendered))
        self.assertTrue(any("srtp_interop_webrtc_srtp" in command for command in rendered))
        self.assertTrue(
            any(
                "dtls_fingerprint_mismatch_fails_before_context_installation" in command
                for command in rendered
            )
        )
        self.assertTrue(any("dtls_srtp_call_integration" in command for command in rendered))
        self.assertTrue(any(command[0:2] == ["cargo", "clippy"] for command in commands))

    def test_fuzz_gates_select_their_fuzz_crate_explicitly(self) -> None:
        fuzz_gates = [
            gate
            for gate in self.catalog["gates"]
            if gate["id"].startswith("security.fuzz-")
        ]
        # 10 legacy targets (SIP message/uri/header/sdp, RTP, RTCP, SRTP,
        # DTLS, STUN, G.711) plus the three AMR ones, which are synthesized
        # because they post-date the canonical run.
        self.assertEqual(len(fuzz_gates), 13)
        self.assertEqual(
            {gate["id"] for gate in fuzz_gates if "amr" in gate["id"]},
            {
                "security.fuzz-amr-unpack",
                "security.fuzz-amr-decode",
                "security.fuzz-amr-encode",
            },
        )
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
            {
                gate_id: by_id[gate_id]["resource_class"]
                for gate_id in soak_ids
            },
            {
                "perf.monolithic-soak": "ec2-performance-soak-long",
                "perf.media-burst-matrix": "ec2-performance-soak",
                "perf.soak-candidate": "ec2-performance-soak-long",
            },
        )
        matrix = gates.matrix_for(
            [
                {"id": gate_id, "decision": "RUN"}
                for gate_id in sorted(soak_ids | {"perf.resiliency-all"})
            ],
            by_id,
        )
        soak_shards = [
            shard
            for shard in matrix
            if shard["resource_class"] == "ec2-performance-soak-long"
        ]
        self.assertEqual(len(soak_shards), 2)
        self.assertTrue(all(len(shard["gates"]) == 1 for shard in soak_shards))
        self.assertTrue(
            all(shard["machine_type"] == "m5.4xlarge" for shard in soak_shards)
        )
        self.assertNotIn(
            "perf.media-burst-matrix",
            {gate for shard in soak_shards for gate in shard["gates"]},
        )

    def test_release_requires_a_current_candidate_performance_evaluation(
        self,
    ) -> None:
        by_id = {gate["id"]: gate for gate in self.catalog["gates"]}
        gate = by_id["perf.canonical-2k-current"]
        self.assertIn(gate["id"], self.catalog["profiles"]["remote-release"])
        # Reusable only by exact version-normalized inputs, so a bump commit
        # carries its candidate's evaluation (bound to that ancestor by the
        # report.performance-metrics reconciliation) instead of rerunning it.
        self.assertFalse(gate["always_fresh"])
        self.assertEqual(gate["resource_class"], "ec2-performance-soak-long")
        self.assertEqual(gate["timeout_minutes"], 120)
        self.assertEqual(gate["estimated_seconds"], 4_500)
        self.assertIn("canonical_2k_release_eval.sh", " ".join(gate["command"]))
        self.assertEqual(
            by_id["source.canonical-2k"]["dependencies"],
            ["source.clean-start", "perf.canonical-2k-current"],
        )
        metrics = by_id["report.performance-metrics"]
        self.assertIn("perf.canonical-2k-current", metrics["dependencies"])
        self.assertIn(
            "current-performance-evaluation.md", metrics["expected_outputs"]
        )

        script = (
            ROOT / "crates/sip/rvoip-sip/scripts/canonical_2k_release_eval.sh"
        ).read_text()
        self.assertIn("for pass in 1 2 3", script)
        self.assertIn("env -i", script)
        self.assertIn('${EVIDENCE_TOOL}" import', script)
        self.assertIn("CARGO_HOME", script)
        self.assertIn("RUSTUP_HOME", script)
        self.assertIn("RUSTC_WRAPPER", script)
        self.assertIn("RVOIP_PERF_PREBUILT_MANIFEST", script)
        self.assertIn("RVOIP_RELEASE_CANDIDATE", script)
        self.assertIn("RVOIP_RELEASE_ENVIRONMENT_ID", script)

    def test_release_runs_the_invite_planner_retention_qualification(self) -> None:
        # The test is `perf-tests`-gated and `#[ignore]`d, so only a release
        # build that asks for ignored tests by exact name runs it.
        by_id = {gate["id"]: gate for gate in self.catalog["gates"]}
        gate = by_id["perf.invite-planner-retention"]
        self.assertIn(gate["id"], self.catalog["profiles"]["remote-release"])
        self.assertIn(gate["id"], by_id["report.remote-aggregate"]["dependencies"])
        self.assertEqual(gate["resource_class"], "ec2-performance")
        self.assertEqual(gate["dependencies"], ["source.remote-clean"])
        self.assertFalse(gate["always_fresh"])
        command = gate["command"]
        self.assertEqual(command[:3], ["cargo", "test", "--locked"])
        self.assertIn("--release", command)
        self.assertEqual(command[command.index("--features") + 1], "perf-tests")
        self.assertEqual(
            command[command.index("--test") + 1],
            "invite_planner_retention_qualification",
        )
        self.assertEqual(
            command[command.index("--") :],
            ["--", "--exact", "--ignored", "--nocapture"],
        )
        source = (
            ROOT
            / "crates/sip/rvoip-sip/tests/invite_planner_retention_qualification.rs"
        ).read_text()
        name = command[command.index("--test") + 2]
        self.assertIn(f"async fn {name}()", source)
        self.assertIn('#![cfg(feature = "perf-tests")]', source)

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
        self.assertTrue(all(shard["machine_type"] == "m5.xlarge" for shard in matrix))
        aggregate = by_id["perf.media-burst-matrix"]
        self.assertEqual(aggregate["executor"], "aggregate")
        self.assertEqual(set(aggregate["dependencies"]), scenario_ids)

    def test_remote_preflight_recreates_the_complete_ec2_worker_shape(self) -> None:
        selected = self.catalog["profiles"]["remote-preflight"]
        self.assertEqual(len(selected), 24)
        by_id = {gate["id"]: gate for gate in self.catalog["gates"]}
        matrix = gates.matrix_for(
            [{"id": gate_id, "decision": "RUN"} for gate_id in selected],
            by_id,
        )
        counts = {
            resource: sum(shard["resource_class"] == resource for shard in matrix)
            for resource in (
                "ec2-performance",
                "ec2-performance-soak",
                "ec2-performance-soak-long",
                "ec2-interop",
                "ec2-proxy-interop",
            )
        }
        self.assertEqual(
            counts,
            {
                "ec2-performance": 6,
                "ec2-performance-soak": 7,
                "ec2-performance-soak-long": 3,
                # One worker per interop lane, as in the release profile.
                "ec2-interop": 6,
                "ec2-proxy-interop": 2,
            },
        )
        self.assertEqual(len(matrix), 24)
        self.assertEqual(
            sum(fanout.MACHINE_VCPUS[shard["machine_type"]] for shard in matrix),
            272,
        )
        self.assertEqual(sum(int(shard["disk_size_gb"]) for shard in matrix), 4600)
        self.assertTrue(all(not shard["hosted"] for shard in matrix))
        self.assertTrue(all(len(shard["gates"]) == 1 for shard in matrix))
        for gate_id in selected:
            with self.subTest(gate=gate_id):
                gate = by_id[gate_id]
                self.assertTrue(gate["always_fresh"])
                self.assertIn(
                    "infra/release-runners/release-infrastructure-preflight.sh",
                    gate["command"],
                )

    def test_a_lab_chain_stays_on_one_worker(self) -> None:
        # Diagnostic 43951452044 split Kamailio's up/matrix/down across the
        # two proxy workers: the matrix found no lab env file, and the
        # teardown ran before the lab came up.
        by_id = {gate["id"]: gate for gate in self.catalog["gates"]}
        profiles = {
            "remote-release": self.catalog["profiles"]["remote-release"],
            "kamailio only": [
                "interop.proxy-pbx.kamailio.up",
                "interop.proxy-pbx.kamailio.matrix",
                "interop.proxy-pbx.kamailio.down",
                "interop.remote-proxies.kamailio.rvoip-first.udp",
            ],
        }
        for name, selected in profiles.items():
            with self.subTest(profile=name):
                matrix = gates.matrix_for(
                    [{"id": gate_id, "decision": "RUN"} for gate_id in selected],
                    by_id,
                )
                shard_of = {gate_id: shard["id"] for shard in matrix for gate_id in shard["gates"]}
                for gate_id in shard_of:
                    for dependency in by_id[gate_id].get("dependencies", []):
                        if dependency in shard_of and by_id[dependency]["resource_class"] == by_id[gate_id]["resource_class"]:
                            self.assertEqual(shard_of[dependency], shard_of[gate_id], (gate_id, dependency))
        # Units still balance across the class's workers.
        independent = [
            {"id": f"g{index}", "resource_class": "ec2-proxy-interop", "estimated_seconds": 60, "dependencies": []}
            for index in range(4)
        ]
        self.assertEqual([len(shard) for shard in gates.balance_gates(independent, 2)], [2, 2])

    def test_remote_release_uses_the_same_272_vcpu_ec2_shape(self) -> None:
        selected = self.catalog["profiles"]["remote-release"]
        by_id = {gate["id"]: gate for gate in self.catalog["gates"]}
        matrix = gates.matrix_for(
            [{"id": gate_id, "decision": "RUN"} for gate_id in selected],
            by_id,
        )
        ec2 = [shard for shard in matrix if not shard["hosted"]]
        # Each long gate owns a long-soak worker, so neither hour-long soak
        # waits behind the other.
        long_soak = [shard for shard in ec2 if shard["resource_class"] == "ec2-performance-soak-long"]
        self.assertEqual(
            sorted(tuple(shard["gates"]) for shard in long_soak),
            [("perf.canonical-2k-current",), ("perf.monolithic-soak",), ("perf.soak-candidate",)],
        )
        counts = {
            resource: sum(shard["resource_class"] == resource for shard in ec2)
            for resource in (
                "ec2-performance",
                "ec2-performance-soak",
                "ec2-performance-soak-long",
                "ec2-interop",
                "ec2-proxy-interop",
            )
        }
        self.assertEqual(
            counts,
            {
                "ec2-performance": 6,
                "ec2-performance-soak": 7,
                "ec2-performance-soak-long": 3,
                "ec2-interop": 6,
                "ec2-proxy-interop": 2,
            },
        )
        self.assertEqual(len(ec2), 24)
        # Six concurrent interop lanes instead of one serial lab chain add
        # 80 vCPUs at peak; the account quota is 1,152.
        self.assertEqual(
            sum(fanout.MACHINE_VCPUS[shard["machine_type"]] for shard in ec2),
            272,
        )
        self.assertEqual(sum(int(shard["disk_size_gb"]) for shard in ec2), 4600)
        interop = {shard["id"]: shard for shard in ec2 if shard["resource_class"] == "ec2-interop"}
        self.assertEqual(
            sorted(interop),
            [
                "ec2-interop-amr-sweep",
                "ec2-interop-asterisk",
                "ec2-interop-freeswitch",
                "ec2-interop-jambonz",
                "ec2-interop-libsrtp",
                "ec2-interop-sipp",
            ],
        )
        # The interop critical path is the longest lane, not the chain's sum.
        self.assertLessEqual(max(shard["estimated_seconds"] for shard in interop.values()), 2200)
        lane_of = {gate_id: shard_id for shard_id, shard in interop.items() for gate_id in shard["gates"]}
        for shard in interop.values():
            for gate_id in shard["gates"]:
                for dependency in by_id[gate_id].get("dependencies", []):
                    if dependency in lane_of:
                        with self.subTest(gate=gate_id, dependency=dependency):
                            self.assertEqual(lane_of[dependency], lane_of[gate_id])
        hosted = [shard for shard in matrix if shard["hosted"]]
        standard = [
            shard for shard in hosted if shard["resource_class"] == "github-standard"
        ]
        self.assertEqual(len(standard), 12)
        # Twelve standard, five nightly, one evidence, and one EC2 controller
        # stay below the twenty-job repository concurrency ceiling.
        self.assertLessEqual(len(hosted) + 1, 19)
        # Adding the 47th crate (rvoip-amazon-a2a) adds a two-minute core gate.
        # Packing the updated catalog raises the longest planned shard from
        # 1345 to 1347 seconds while preserving the twelve-shard concurrency cap.
        # The 48th crate (rvoip-webex-byova) adds one more bounded two-minute
        # core gate, raising the longest planned shard from 1347 to 1359 seconds.
        # The unpublished benchmark DUT (rvoip-bench-uas) adds another, raising
        # it from 1359 to 1365 seconds.
        self.assertLessEqual(max(shard["estimated_seconds"] for shard in standard), 1365)

    def test_remote_diagnostic_runs_only_exact_ec2_gates_and_dependencies(self) -> None:
        requested = ["interop.remote-proxies", "perf.monolithic-soak"]
        selected = gates.profile_selection(
            self.catalog, "remote-diagnostic", requested
        )
        self.assertEqual(
            set(selected),
            {
                "interop.remote-proxies",
                "perf.monolithic-soak",
                "source.clean-start",
                "source.remote-clean",
            }
            | set(builder.PROXY_INTEROP_GATE_IDS),
        )
        by_id = {gate["id"]: gate for gate in self.catalog["gates"]}
        matrix = gates.matrix_for(
            [{"id": gate_id, "decision": "RUN"} for gate_id in selected], by_id
        )
        self.assertEqual(
            {shard["resource_class"] for shard in matrix},
            {
                "ec2-proxy-interop",
                "ec2-performance-soak-long",
                "github-evidence",
            },
        )
        self.assertEqual(
            {
                gate_id
                for shard in matrix
                for gate_id in shard["gates"]
                if shard["resource_class"].startswith("ec2-")
            },
            set(builder.PROXY_INTEROP_GATE_IDS) | {"perf.monolithic-soak"},
        )

        preflight = gates.profile_selection(
            self.catalog, "remote-diagnostic", ["preflight.performance-01"]
        )
        self.assertEqual(
            set(preflight),
            {"preflight.performance-01"},
        )

        invalid_cases = (
            ([], "requires at least one"),
            (["core.rvoip"], "executable EC2 gates"),
            (["perf.media-burst-matrix"], "executable EC2 gates"),
            (["not.a.gate"], "must belong to remote-release or remote-preflight"),
            ([f"gate-{index}" for index in range(21)], "at most 20"),
        )
        for gate_ids, message in invalid_cases:
            with self.subTest(gates=gate_ids):
                with self.assertRaisesRegex(gates.GateError, message):
                    gates.profile_selection(
                        self.catalog, "remote-diagnostic", gate_ids
                    )
        with self.assertRaisesRegex(gates.GateError, "allowed only"):
            gates.profile_selection(
                self.catalog, "remote-release", ["perf.monolithic-soak"]
            )

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
                self.assertIn('rust-version = "1.95"', manifest.read_text())

    def test_rvoip_sip_gates_build_the_test_hooks_their_targets_need(self) -> None:
        # Since 0.4.0 the RFC 4028 session-timer tests and their process
        # fixtures carry `#![cfg(feature = "test-hooks")]`. A gate that builds
        # rvoip-sip without the feature compiles them to empty binaries and
        # still passes: the 0.4.0 `test.sip-integration` gate ran 0 tests in
        # five session-timer targets that way. A gate selecting rvoip-sip
        # targets broadly must enable the feature, as the PR and main lanes
        # do (`SIP_LANE_FEATURES`), and a gate naming a target explicitly must
        # enable it when that target's source is gated on it.
        targets = rvoip_sip_targets()
        gated = {key for key, path in targets.items() if TEST_HOOKS_GUARD.search(path.read_text())}
        # The fence must not pass vacuously: the session-timer suites exist.
        self.assertIn(("test", "session_timer_integration"), gated)
        self.assertGreaterEqual(sum(kind == "test" for kind, _ in gated), 6)
        self.assertEqual(run_checks.SIP_LANE_FEATURES, ["--features", "test-hooks"])

        checked = 0
        for gate in self.catalog["gates"]:
            selection = cargo_rvoip_sip_selection(gate.get("command") or [])
            if selection is None:
                continue
            checked += 1
            broad, explicit, has_test_hooks = selection
            with self.subTest(gate=gate["id"]):
                if broad:
                    self.assertTrue(
                        has_test_hooks,
                        f"{gate['id']} selects rvoip-sip {sorted(broad)} without test-hooks",
                    )
                for kind, pattern in explicit:
                    matched = [
                        key for key in targets if key[0] == kind and fnmatch.fnmatchcase(key[1], pattern)
                    ]
                    self.assertTrue(matched, f"{gate['id']} names unknown {kind} {pattern}")
                    needs = sorted(name for key in matched if key in gated for name in [key[1]])
                    self.assertFalse(
                        needs and not has_test_hooks,
                        f"{gate['id']} builds test-hooks-gated {kind} {needs} without the feature",
                    )
        # Unit, integration, doctest, all-target, examples, analyzer, SIPp
        # build, and the performance gates.
        self.assertGreaterEqual(checked, 30)

    def test_rvoip_sip_test_hooks_fence_rejects_a_gate_without_the_feature(self) -> None:
        targets = rvoip_sip_targets()
        self.assertEqual(
            cargo_rvoip_sip_selection(
                ["cargo", "test", "--locked", "-p", "rvoip-sip", "--tests"]
            ),
            ({"--tests"}, [], False),
        )
        broad, explicit, has_test_hooks = cargo_rvoip_sip_selection(
            ["cargo", "test", "-p", "rvoip-sip", "--test", "session_timer_*"]
        )
        matched = [key for key in targets if key[0] == "test" and fnmatch.fnmatchcase(key[1], explicit[0][1])]
        self.assertFalse(broad)
        self.assertFalse(has_test_hooks)
        self.assertTrue(
            any(TEST_HOOKS_GUARD.search(targets[key].read_text()) for key in matched)
        )
        self.assertIsNone(
            cargo_rvoip_sip_selection(["cargo", "test", "--workspace", "--exclude", "rvoip-sip"])
        )

    def test_rustdoc_warning_policy_is_one_environment_argument(self) -> None:
        gate = next(
            gate for gate in self.catalog["gates"] if gate["id"] == "build.rustdoc"
        )
        self.assertEqual(
            gate["command"][:3], ["env", "RUSTDOCFLAGS=-D warnings", "cargo"]
        )
        self.assertNotIn("warnings", gate["command"])
        self.assertIn("--workspace", gate["command"])
        self.assertIn("--all-features", gate["command"])
        self.assertIn("--no-deps", gate["command"])
        self.assertIn("--locked", gate["command"])
        self.assertNotIn("-p", gate["command"])

    def test_format_gate_does_not_receive_unsupported_locked_flag(self) -> None:
        gate = next(
            gate for gate in self.catalog["gates"] if gate["id"] == "build.format"
        )
        self.assertEqual(gate["command"][:2], ["cargo", "fmt"])
        self.assertNotIn("--locked", gate["command"])
        self.assertEqual(
            builder.insert_locked(["cargo", "+1.95.0", "fmt", "--all"]),
            ["cargo", "+1.95.0", "fmt", "--all"],
        )

    def test_ec2_gates_allow_for_cold_release_builds(self) -> None:
        for gate in self.catalog["gates"]:
            if gate["resource_class"].startswith("ec2-"):
                with self.subTest(gate=gate["id"]):
                    self.assertGreaterEqual(gate["timeout_minutes"], 20)

    def test_cargo_gates_allow_for_cold_release_builds(self) -> None:
        for gate in self.catalog["gates"]:
            if gate["kind"] == "cargo":
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

    def test_remote_proxy_gate_persists_full_failure_evidence(self) -> None:
        by_id = {gate["id"]: gate for gate in self.catalog["gates"]}
        row_ids = set(builder.PROXY_INTEROP_GATE_IDS)
        aggregate = by_id["interop.remote-proxies"]
        self.assertEqual(aggregate["executor"], "aggregate")
        self.assertEqual(set(aggregate["dependencies"]), row_ids)

        for gate_id in row_ids:
            gate = by_id[gate_id]
            with self.subTest(gate=gate_id):
                self.assertEqual(gate["resource_class"], "ec2-proxy-interop")
                self.assertIn(
                    "PROXY_INTEROP_ARTIFACT_DIR={artifact_dir}/proxy-interop",
                    gate["command"],
                )
                self.assertEqual(gate["command"][-3:], gate_id.split(".")[-3:])

        matrix = gates.matrix_for(
            [{"id": gate_id, "decision": "RUN"} for gate_id in row_ids],
            by_id,
        )
        self.assertEqual(len(matrix), 2)
        self.assertTrue(all(len(shard["gates"]) == 6 for shard in matrix))
        self.assertTrue(
            all(shard["machine_type"] == "m5.large" for shard in matrix)
        )
        self.assertTrue(all(shard["disk_size_gb"] == 100 for shard in matrix))

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

    def test_ec2_gate_digest_includes_ephemeral_worker_definition(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            runner = root / "scripts/release/gates.py"
            startup = root / "infra/release-runners/aws-release-startup.sh"
            runner.parent.mkdir(parents=True)
            startup.parent.mkdir(parents=True)
            runner.write_text("runner-v1\n")
            startup.write_text("startup-v1\n")
            gate = {
                "id": "gate-a",
                "resource_class": "ec2-performance",
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
                        "infra/release-runners/aws-release-startup.sh",
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

    def test_prior_receipts_combine_failed_full_and_passing_diagnostic_runs(self) -> None:
        inputs = {
            "gate_definition_sha256": "d" * 64,
            "input_sha256": "i" * 64,
            "environment_sha256": "e" * 64,
        }
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for run_id, status in (("100", "FAIL"), ("101", "PASS")):
                gate_dir = root / run_id / "release-evidence" / "perf.example"
                gate_dir.mkdir(parents=True)
                receipt = {
                    "schema": gates.RECEIPT_SCHEMA,
                    "gate_id": "perf.example",
                    "candidate_sha": ("a" if status == "FAIL" else "b") * 40,
                    "status": status,
                    **inputs,
                }
                (gate_dir / "receipt.json").write_text(json.dumps(receipt))

            combined = gates.load_prior_receipts(root)["perf.example"]
            self.assertEqual([item["status"] for item in combined], ["FAIL", "PASS"])
            self.assertEqual(
                gates.exact_reusable(
                    combined, {"always_fresh": False}, inputs, "environment"
                )["status"],
                "PASS",
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

    def test_always_fresh_bookkeeping_does_not_invalidate_runtime_gates(self) -> None:
        selected = [
            {
                "id": "source.clean",
                "always_fresh": True,
                "affected_paths": ["**"],
            },
            {
                "id": "interop.freeswitch",
                "always_fresh": False,
                "affected_paths": ["infra/release-runners/interop-lifecycle.sh"],
            },
            {
                "id": "core.unrelated",
                "always_fresh": False,
                "affected_paths": ["crates/core/**"],
            },
        ]
        self.assertEqual(
            gates.directly_changed_gate_ids(
                ["infra/release-runners/interop-lifecycle.sh"], selected
            ),
            {"interop.freeswitch"},
        )

    def test_changed_path_parser_keeps_rename_copy_and_delete_sources(self) -> None:
        self.assertEqual(
            gates.parse_name_status_z(
                "R100\0crates/old.rs\0docs/new.md\0"
                "C090\0crates/source.rs\0crates/copy.rs\0"
                "D\0crates/deleted.rs\0M\0crates/modified.rs\0"
            ),
            [
                "crates/copy.rs",
                "crates/deleted.rs",
                "crates/modified.rs",
                "crates/old.rs",
                "crates/source.rs",
                "docs/new.md",
            ],
        )
        with self.assertRaises(gates.GateError):
            gates.parse_name_status_z("R100\0only-old-path\0")

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
        self.assertEqual(
            gates.unknown_change(
                [
                    "docs/release-notes.md",
                    "scripts/ci/test_workflow_policy.py",
                    "scripts/test_release_gates.py",
                ],
                selected,
            ),
            [],
        )

    def test_ci_policy_tests_do_not_change_unrelated_gate_inputs(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            runner = root / "scripts/release/gates.py"
            policy_test = root / "scripts/ci/test_workflow_policy.py"
            runner.parent.mkdir(parents=True)
            policy_test.parent.mkdir(parents=True)
            runner.write_text("runner-v1\n")
            policy_test.write_text("test-v1\n")
            gate = {
                "id": "gate-a",
                "resource_class": "github-standard",
                "affected_paths": ["crates/a/**"],
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
                        "scripts/ci/test_workflow_policy.py",
                    ],
                    package_roots={},
                    package_dependencies={},
                    gate_definitions=definitions,
                )

            first = record()
            policy_test.write_text("test-v2\n")
            self.assertEqual(first, record())

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

    def test_github_outputs_split_hosted_and_ephemeral_ec2_matrices(self) -> None:
        plan = {
            "matrix": [
                {"id": "hosted", "hosted": True},
                {"id": "ec2", "hosted": False},
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
        self.assertEqual(json.loads(values["aws_matrix"])["include"], [plan["matrix"][1]])
        self.assertEqual(values["hosted_shard_count"], "1")
        self.assertEqual(values["aws_shard_count"], "1")

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
            self.assertFalse(result["publishing_attempted"])
            self.assertTrue((root / "collect-gate.aggregate/receipt.json").is_file())

            (dependency_dir / "receipt.json").unlink()
            self.assertEqual(
                gates.collect(plan=plan, evidence_root=root, output=output), 1
            )

    def test_performance_reconciliation_runs_real_baseline_audit(self) -> None:
        baseline_root = (
            ROOT / "crates/sip/rvoip-sip/perf-baselines/20260929T224805Z"
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

    def test_performance_reconciliation_ignores_nested_provenance_copies(
        self,
    ) -> None:
        baseline_root = (
            ROOT / "crates/sip/rvoip-sip/perf-baselines/20260929T224805Z"
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
                current_target = evidence / "_perf-results/matrix-shard" / relative
                provenance_current = (
                    evidence
                    / "_perf-results/canonical-shard/profiles/run-1/output-target/perf-results"
                    / relative
                )
                provenance_baseline = (
                    evidence
                    / "_perf-results/canonical-shard/profiles/run-1/reviewed-baseline"
                    / relative
                )
                for target in (
                    baseline_target,
                    current_target,
                    provenance_current,
                    provenance_baseline,
                ):
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(source, target)
            artifact = evidence / "collect-report.regression-audit"
            artifact.mkdir()
            result = gates.reconcile_performance_regression(
                ROOT, evidence, artifact
            )
            self.assertEqual(
                set(result["selected_results"]), set(manifest["comparison_paths"])
            )

    def test_performance_reconciliation_rejects_duplicate_authoritative_results(
        self,
    ) -> None:
        baseline_root = (
            ROOT / "crates/sip/rvoip-sip/perf-baselines/20260929T224805Z"
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
                current_target = evidence / "_perf-results/matrix-shard" / relative
                baseline_target.parent.mkdir(parents=True, exist_ok=True)
                current_target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(source, baseline_target)
                shutil.copy2(source, current_target)
            duplicate = (
                evidence
                / "_perf-results/duplicate-shard"
                / manifest["comparison_paths"][0]
            )
            duplicate.parent.mkdir(parents=True)
            shutil.copy2(
                baseline_root / manifest["comparison_paths"][0], duplicate
            )
            artifact = evidence / "collect-report.regression-audit"
            artifact.mkdir()
            with self.assertRaisesRegex(
                gates.GateError, "has 2 exact-candidate results"
            ):
                gates.reconcile_performance_regression(ROOT, evidence, artifact)

    def test_performance_reconciliation_rejects_missing_results_directory(
        self,
    ) -> None:
        baseline_root = (
            ROOT / "crates/sip/rvoip-sip/perf-baselines/20260929T224805Z"
        )
        with tempfile.TemporaryDirectory() as directory:
            evidence = Path(directory) / "evidence"
            packaged = evidence / "baseline-gate/perf-regression-baseline"
            packaged.mkdir(parents=True)
            shutil.copy2(baseline_root / "manifest.json", packaged / "manifest.json")
            artifact = evidence / "collect-report.regression-audit"

            with self.assertRaisesRegex(
                gates.GateError,
                "exact-candidate performance results directory is missing",
            ):
                gates.reconcile_performance_regression(ROOT, evidence, artifact)

    def test_current_performance_reconciliation_fails_without_candidate_artifacts(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with self.assertRaisesRegex(
                gates.GateError, "exact-candidate performance artifacts are missing"
            ):
                gates.reconcile_performance_metrics(
                    ROOT, root, root / "artifact", "c" * 40
                )

    def test_current_performance_reconciliation_requires_one_high_density_run(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            shard = root / "_perf-results/shard"
            shard.mkdir(parents=True)
            (shard / "result.json").write_text("{}\n")
            with self.assertRaisesRegex(
                gates.GateError, "exactly one high-density media-burst result"
            ):
                gates.reconcile_performance_metrics(
                    ROOT, root, root / "artifact", "c" * 40
                )

    def test_current_performance_reconciliation_requires_canonical_package(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            shard = root / "_perf-results/shard"
            shard.mkdir(parents=True)
            (shard / "result.json").write_text("{}\n")
            burst = (
                root
                / "perf.media-burst.high-density-media-burst"
                / "perf_burst_matrix/burst_fixture/high-density-media-burst"
            )
            burst.mkdir(parents=True)
            for role in ("caller", "receiver"):
                (burst / f"perf_burst_{role}_high-density-media-burst.json").write_text(
                    "{}\n"
                )
            with self.assertRaisesRegex(
                gates.GateError, "exact-candidate canonical 2,000-CPS evidence index"
            ):
                gates.reconcile_performance_metrics(
                    ROOT, root, root / "artifact", "c" * 40
                )

    # ---- Version-normalized reuse (docs/RELEASE_PROCESS_PLAN.md, P2) ----

    BUMP_ROOT_MANIFEST = """[workspace]
members = ["crates/a"]

[workspace.package]
version = "{version}"
edition = "2021"

[workspace.dependencies]
rvoip-a = {{ path = "crates/a", version = "{version}" }}
renamed = {{ package = "rvoip-b", path = "crates/b", version = "{version}" }}
serde = {{ version = "1.0.210", features = ["derive"] }}
tokio = "1.40.0"
"""
    BUMP_MEMBER_MANIFEST = """[package]
name = "rvoip-a"
version.workspace = true

[dependencies]
rvoip-b = {{ path = "../b", version = "{version}" }}
rvoip-c = "{version}"
anyhow = "1.0.89"

[dependencies.rvoip-d]
path = "../d"
version = "{version}"

[target.'cfg(unix)'.dev-dependencies]
rvoip-e = {{ path = "../e", version = "{version}" }}
"""
    BUMP_LOCKFILE = """version = 4

[[package]]
name = "anyhow"
version = "1.0.89"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "{checksum}"

[[package]]
name = "rvoip-a"
version = "{version}"
dependencies = [
 "anyhow",
 "rvoip-b {version}",
]

[[package]]
name = "rvoip-b"
version = "{version}"
"""

    def write_bump_tree(self, root: Path, version: str, *, checksum: str = "a" * 64) -> list[str]:
        files = {
            "Cargo.toml": self.BUMP_ROOT_MANIFEST.format(version=version),
            "crates/a/Cargo.toml": self.BUMP_MEMBER_MANIFEST.format(version=version),
            "Cargo.lock": self.BUMP_LOCKFILE.format(version=version, checksum=checksum),
            "README.md": f"Add `rvoip = \"{version}\"` (current release {version}).\n",
            "CHANGELOG.md": f"## {version} \u2014 2026-11-16\n\n- notes for {version}\n",
            "docs/release/RELEASE_NOTES_NEXT.md": f"# rvoip {version}\n",
            "crates/a/src/lib.rs": "pub fn answer() -> u32 { 42 }\n",
            "scripts/release/gates.py": "runner-v1\n",
        }
        for relative, text in files.items():
            path = root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        return sorted(files)

    def bump_record(self, root: Path, files: list[str], paths: list[str]) -> dict:
        gate = {
            "id": "gate-a",
            "resource_class": "ec2-performance",
            "affected_paths": paths,
            "affected_crates": [],
            "dependencies": [],
            "command": ["true"],
        }
        return gates.input_record(
            root=root,
            gate=gate,
            environment_id="environment-v1",
            files=files,
            package_roots={},
            package_dependencies={},
            gate_definitions={"gate-a": gate},
        )

    def test_version_bump_only_commit_keeps_every_gate_input_digest(self) -> None:
        paths = ["**"]
        with tempfile.TemporaryDirectory() as first, tempfile.TemporaryDirectory() as second:
            parent = self.bump_record(Path(first), self.write_bump_tree(Path(first), "0.4.0"), paths)
            bump = self.bump_record(Path(second), self.write_bump_tree(Path(second), "0.4.1"), paths)
            self.assertEqual(parent["input_sha256"], bump["input_sha256"])
            self.assertEqual(parent["input_normalization"], gates.INPUT_NORMALIZATION)
            # Bookkeeping never reaches the digest; normalized files are marked.
            self.assertNotIn("CHANGELOG.md", parent["files"])
            self.assertNotIn("docs/release/RELEASE_NOTES_NEXT.md", parent["files"])
            self.assertTrue(parent["files"]["Cargo.lock"].startswith("lockfile:"))
            self.assertTrue(parent["files"]["Cargo.toml"].startswith("manifest:"))
            self.assertTrue(parent["files"]["README.md"].startswith("document:"))

    def test_version_normalization_still_sees_real_changes(self) -> None:
        with tempfile.TemporaryDirectory() as first, tempfile.TemporaryDirectory() as second:
            base = self.bump_record(Path(first), self.write_bump_tree(Path(first), "0.4.0"), ["**"])
            root = Path(second)
            files = self.write_bump_tree(root, "0.4.1", checksum="b" * 64)
            # A registry dependency that moved in the same commit as the bump.
            self.assertNotEqual(base["input_sha256"], self.bump_record(root, files, ["**"])["input_sha256"])
            files = self.write_bump_tree(root, "0.4.1")
            (root / "Cargo.toml").write_text(
                (root / "Cargo.toml").read_text().replace('tokio = "1.40.0"', 'tokio = "1.41.0"')
            )
            self.assertNotEqual(base["input_sha256"], self.bump_record(root, files, ["**"])["input_sha256"])
            files = self.write_bump_tree(root, "0.4.1")
            (root / "crates/a/src/lib.rs").write_text("pub fn answer() -> u32 { 43 }\n")
            self.assertNotEqual(base["input_sha256"], self.bump_record(root, files, ["**"])["input_sha256"])

    def test_manifest_normalization_rewrites_only_internal_versions(self) -> None:
        normalized = gates.normalize_manifest(
            self.BUMP_ROOT_MANIFEST.format(version="9.8.7")
            + self.BUMP_MEMBER_MANIFEST.format(version="9.8.7")
            + '# Copying this example? Keep: rvoip-sip = "9.8.7"\n'
        )
        self.assertNotIn("9.8.7", normalized)
        self.assertIn('serde = { version = "1.0.210"', normalized)
        self.assertIn('tokio = "1.40.0"', normalized)
        self.assertIn('anyhow = "1.0.89"', normalized)
        lock = gates.normalize_lockfile(
            self.BUMP_LOCKFILE.format(version="9.8.7", checksum="c" * 64)
        )
        self.assertNotIn("9.8.7", lock)
        self.assertIn('version = "1.0.89"', lock)
        self.assertIn('"rvoip-b <rvoip-version>"', lock)

    def test_version_bearing_documents_follow_release_preparation(self) -> None:
        release = load_module("release_tool_for_gates", ROOT / "scripts/release.py")
        expected = {
            path.as_posix()
            for path in release.ACTIVE_RELEASE_METADATA_FILES
            if path.name != "Cargo.toml"
        }
        self.assertEqual(gates.version_bearing_documents(), expected)
        self.assertEqual(gates.normalization_kind("examples/01-quickstart-p2p/Cargo.toml"), "manifest")
        self.assertEqual(gates.normalization_kind("examples/Cargo.lock"), "lockfile")
        self.assertIsNone(gates.normalization_kind("crates/sip/rvoip-sip/src/lib.rs"))

    def test_release_bookkeeping_is_neutral_for_changed_path_validation(self) -> None:
        selected = [{"kind": "cargo", "always_fresh": False, "affected_paths": ["crates/a/**"]}]
        self.assertEqual(
            gates.unknown_change(
                [
                    "CHANGELOG.md",
                    "README.md",
                    "changes/414.md",
                    "docs/release/RELEASE_NOTES_NEXT.md",
                ],
                selected,
            ),
            [],
        )

    def test_reuse_requires_a_receipt_from_the_candidates_chain(self) -> None:
        inputs = {
            "gate_definition_sha256": "d" * 64,
            "input_sha256": "i" * 64,
            "environment_sha256": "e" * 64,
        }
        receipt = {"status": "PASS", "candidate_sha": "a" * 40, **inputs}
        gate = {"always_fresh": False}
        self.assertIs(
            gates.exact_reusable([receipt], gate, inputs, "env", lambda sha: sha == "a" * 40),
            receipt,
        )
        self.assertIsNone(
            gates.exact_reusable([receipt], gate, inputs, "env", lambda sha: False)
        )

    def test_chain_checker_accepts_only_ancestors(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            def git(*args: str) -> str:
                return subprocess.run(
                    ["git", "-c", "user.name=t", "-c", "user.email=t@t", *args],
                    cwd=root, check=True, capture_output=True, text=True,
                ).stdout.strip()
            git("init", "-q", "-b", "main")
            (root / "a").write_text("1\n")
            git("add", "a"); git("commit", "-qm", "parent")
            parent = git("rev-parse", "HEAD")
            (root / "a").write_text("2\n")
            git("commit", "-qam", "bump")
            candidate = git("rev-parse", "HEAD")
            git("checkout", "-q", "-b", "side", parent)
            (root / "a").write_text("3\n")
            git("commit", "-qam", "side")
            side = git("rev-parse", "HEAD")
            accepted = gates.chain_checker(root, candidate)
            self.assertTrue(accepted(parent))
            self.assertTrue(accepted(candidate))
            self.assertFalse(accepted(side))
            self.assertFalse(accepted("not-a-sha"))

    def test_performance_gates_are_reused_whole_from_one_run(self) -> None:
        by_id = {
            "perf.a": {"resource_class": "ec2-performance"},
            "perf.b": {"resource_class": "ec2-performance-soak-long"},
            "core.x": {"resource_class": "github-standard"},
        }
        prior = Path("/prior")
        def receipt(run: str) -> dict:
            return {"_path": f"/prior/{run}/release-evidence/gate/receipt.json"}
        reusable = {"perf.a": receipt("1"), "perf.b": receipt("1"), "core.x": receipt("1")}
        gates.require_whole_performance_group(list(by_id), by_id, reusable, prior)
        self.assertIsNotNone(reusable["perf.a"])
        reusable = {"perf.a": receipt("1"), "perf.b": None, "core.x": receipt("1")}
        gates.require_whole_performance_group(list(by_id), by_id, reusable, prior)
        self.assertIsNone(reusable["perf.a"])
        self.assertIsNotNone(reusable["core.x"])
        reusable = {"perf.a": receipt("1"), "perf.b": receipt("2"), "core.x": receipt("1")}
        gates.require_whole_performance_group(list(by_id), by_id, reusable, prior)
        self.assertIsNone(reusable["perf.a"])

    def reused_plan_fixture(self, root: Path) -> tuple[dict, Path, Path]:
        prior = root / "prior"
        gate_dir = prior / "4100" / "release-evidence" / "perf.example"
        gate_dir.mkdir(parents=True)
        (gate_dir / "command.log").write_text("pass\n")
        (gate_dir / "artifact.json").write_text("{}\n")
        perf = prior / "4100" / "release-evidence" / "_perf-results" / "ec2-performance-1"
        perf.mkdir(parents=True)
        (perf / "result.json").write_text("{}\n")
        receipt = {
            "schema": gates.RECEIPT_SCHEMA,
            "gate_id": "perf.example",
            "candidate_sha": "a" * 40,
            "status": "PASS",
            "gate_definition_sha256": "d" * 64,
            "input_sha256": "i" * 64,
            "environment_sha256": "e" * 64,
            "log": {"path": "perf.example/command.log", "sha256": "0" * 64, "bytes": 5},
        }
        (gate_dir / "receipt.json").write_bytes(gates.canonical_bytes(receipt))
        plan = {
            "schema": gates.PLAN_SCHEMA,
            "candidate_sha": "b" * 40,
            "profile": "test",
            "catalog_sha256": "x" * 64,
            "environment_id": "test",
            "gates": [
                {
                    "id": "perf.example",
                    "decision": "REUSE",
                    "executor": "argv",
                    "resource_class": "ec2-performance",
                    "dependencies": [],
                    "gate_definition_sha256": "d" * 64,
                    "input_sha256": "i" * 64,
                    "environment_sha256": "e" * 64,
                    "reuse_receipt": receipt,
                    "reuse_receipt_sha256": gates.file_sha256(gate_dir / "receipt.json"),
                    "reuse_receipt_path": "4100/release-evidence/perf.example/receipt.json",
                }
            ],
        }
        return plan, prior, root / "evidence"

    def test_collector_stages_reused_evidence_for_reports_and_the_next_run(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            plan, prior, evidence = self.reused_plan_fixture(Path(directory))
            output = Path(directory) / "aggregate.json"
            self.assertEqual(
                gates.collect(plan=plan, evidence_root=evidence, output=output, prior_evidence=prior),
                0,
            )
            result = json.loads(output.read_text())
            self.assertEqual((result["fresh_count"], result["reused_count"]), (0, 1))
            self.assertEqual(result["accepted_gates"][0]["receipt_candidate_sha"], "a" * 40)
            self.assertTrue((evidence / "perf.example/artifact.json").is_file())
            self.assertTrue((evidence / "_perf-results/ec2-performance-1/result.json").is_file())
            # The staged copy is itself loadable as prior evidence for the next
            # requalification in the chain.
            self.assertEqual(
                [item["candidate_sha"] for item in gates.load_prior_receipts(evidence)["perf.example"]],
                ["a" * 40],
            )

    def test_collector_rejects_reuse_without_matching_prior_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            plan, prior, evidence = self.reused_plan_fixture(Path(directory))
            output = Path(directory) / "aggregate.json"
            self.assertEqual(gates.collect(plan=plan, evidence_root=evidence, output=output), 1)
            receipt = prior / "4100/release-evidence/perf.example/receipt.json"
            receipt.write_bytes(receipt.read_bytes().replace(b"PASS", b"FAIL"))
            shutil.rmtree(evidence, ignore_errors=True)
            self.assertEqual(
                gates.collect(plan=plan, evidence_root=evidence, output=output, prior_evidence=prior),
                1,
            )
            self.assertIn(
                "reused evidence receipt digest changed",
                " ".join(json.loads(output.read_text())["failures"]),
            )

    def test_collector_binds_bench_receipts_to_gate_profile_and_commit(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            gate_dir = root / "perf.rfc7502-release"
            gate_dir.mkdir()
            log = gate_dir / "command.log"
            log.write_text("pass\n")
            bench = {
                "schema": gates.BENCH_RECEIPT_SCHEMA,
                "profile": "release",
                "candidate_sha": "c" * 40,
                "status": "PASS",
                "reasons": [],
                # Additive lane fields (AZ pin, capacity reservations) are
                # recorded in the evidence as written, never rejected.
                "environment": {
                    "env_id": "c8i-test",
                    "availability_zone": "us-east-1a",
                    "capacity_reservations": [
                        {
                            "id": "cr-0123456789abcdef0",
                            "instance_type": "c8i.4xlarge",
                            "count": 1,
                            "az": "us-east-1a",
                            "created_at": "2026-11-17T00:00:00Z",
                            "cancelled_at": None,
                            "held_for_resume": True,
                        }
                    ],
                },
            }
            (gate_dir / "bench-receipt.json").write_text(json.dumps(bench))
            receipt = {
                "schema": gates.RECEIPT_SCHEMA,
                "gate_id": "perf.rfc7502-release",
                "candidate_sha": "c" * 40,
                "status": "PASS",
                "gate_definition_sha256": "d" * 64,
                "input_sha256": "i" * 64,
                "environment_sha256": "e" * 64,
                "log": {
                    "path": "perf.rfc7502-release/command.log",
                    "sha256": gates.file_sha256(log),
                    "bytes": log.stat().st_size,
                },
            }
            (gate_dir / "receipt.json").write_bytes(gates.canonical_bytes(receipt))
            plan = {
                "schema": gates.PLAN_SCHEMA,
                "candidate_sha": "c" * 40,
                "profile": "bench-release",
                "catalog_sha256": "x" * 64,
                "environment_id": "test",
                "gates": [
                    {
                        "id": "perf.rfc7502-release",
                        "decision": "RUN",
                        "resource_class": "bench-lane",
                        "gate_definition_sha256": "d" * 64,
                        "input_sha256": "i" * 64,
                        "environment_sha256": "e" * 64,
                    }
                ],
            }
            output = root / "aggregate.json"
            self.assertEqual(gates.collect(plan=plan, evidence_root=root, output=output), 0)
            for key, value in (("profile", "minor"), ("candidate_sha", "f" * 40), ("status", "FAIL")):
                with self.subTest(key=key):
                    (gate_dir / "bench-receipt.json").write_text(json.dumps({**bench, key: value}))
                    self.assertEqual(gates.collect(plan=plan, evidence_root=root, output=output), 1)

    def test_performance_evaluation_expects_the_catalog_soak_settings(self) -> None:
        # The evaluation's expected soak duration and concurrency come from the
        # gate commands, so a catalog change cannot leave it expecting the old
        # value (the 0.5.0 candidate 424c6189d failed on exactly that).
        settings = gates.soak_evaluation_settings(ROOT)
        self.assertEqual(
            settings,
            {
                "perf.monolithic-soak": {"duration_secs": "3600", "active_calls": "30"},
                "perf.soak-candidate": {"duration_secs": "7200", "active_calls": "500"},
            },
        )
        source = (ROOT / "scripts/release/gates.py").read_text()
        call = source.split('"crates/sip/rvoip-sip/scripts/beta_performance_gate_metrics.py",', 1)[1].split("root=root", 1)[0]
        for flag in ("--monolithic-duration-secs", "--monolithic-active-calls", "--split-duration-secs", "--split-active-calls"):
            following = call.split(f'"{flag}",', 1)[1].lstrip().split("\n", 1)[0]
            self.assertTrue(following.startswith("soak["), f"{flag} must come from the catalog, not a literal: {following}")

    def test_soak_candidate_runs_two_hours_with_the_same_limit(self) -> None:
        catalog = json.loads((ROOT / "scripts/release/gates.json").read_text())
        gates_by_id = {gate["id"]: gate for gate in catalog["gates"]}
        soak = gates_by_id["perf.soak-candidate"]
        self.assertIn("RVOIP_PERF_SOAK_DURATION_SECS=7200", soak["command"])
        self.assertIn("RVOIP_PERF_SOAK_DURATION_SECS=7200", soak["display_command"])
        # The threshold and the window are unchanged: 15 MB/h over the
        # harness's own last-20-minute active window (no override in argv).
        self.assertIn("RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR=15", soak["command"])
        self.assertFalse([arg for arg in soak["command"] if "GATE_WINDOW" in arg or "ACTIVE_TAIL" in arg])
        # The measured two-hour gate takes 7,449 s; keep real headroom.
        self.assertGreaterEqual(soak["timeout_minutes"] * 60, 7449 + 20 * 60)
        self.assertGreaterEqual(soak["estimated_seconds"], 7200)
        # The monolithic soak stays at one hour.
        monolithic = " ".join(gates_by_id["perf.monolithic-soak"]["command"] or [])
        self.assertNotIn("DURATION_SECS=7200", monolithic)

    def test_bench_gate_wrapper_resumes_one_lane_across_legs(self) -> None:
        bench = load_module("release_bench_gate", ROOT / "scripts/release/bench_gate.py")
        candidate = "c" * 40
        self.assertEqual(
            bench.lane_run_id("minor", candidate, "4242"), "bench-minor-cccccccccccc-4242"
        )
        args = bench.argparse.Namespace(
            profile="minor", candidate=candidate, receipt=Path("r.json"), max_wall=330
        )
        command = bench.lane_command(args, Path("/t/release/rvoip-bench-uas"), "4242")
        self.assertIn("--resume", command)
        self.assertEqual(command[command.index("--max-wall") + 1], "330")
        self.assertEqual(command[command.index("--capacity-wait") + 1], "90")
        args.capacity_wait = 30
        command = bench.lane_command(args, Path("/t/release/rvoip-bench-uas"), "4242")
        self.assertEqual(command[command.index("--capacity-wait") + 1], "30")
        self.assertEqual(command[command.index("--run-id") + 1], "bench-minor-cccccccccccc-4242")
        self.assertNotIn("--caller-bin", command)
        # The minor lane records R into the gate's evidence, never the checkout.
        self.assertEqual(command[command.index("--record-out") + 1], "rates.json")
        self.assertIn("--record", command)
        args.profile = "release"
        args.receipt = Path("/e/perf.rfc7502-release/bench-receipt.json")
        # A release lane records the rows it measured (those without a recorded R).
        command = bench.lane_command(args, Path("/t/dut"), "4242")
        self.assertIn("--record", command)
        self.assertEqual(
            command[command.index("--record-out") + 1], "/e/perf.rfc7502-release/rates.json"
        )

    def test_bench_gate_fails_closed_until_the_bench_environment_is_pinned(self) -> None:
        bench = load_module("release_bench_gate", ROOT / "scripts/release/bench_gate.py")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            subprocess.run(["git", "init", "-q", str(root)], check=True)
            self.assertIn("run_lane.py is not installed", bench.preflight(root))
            (root / "scripts/bench").mkdir(parents=True)
            (root / "scripts/bench/run_lane.py").write_text("")
            (root / "Cargo.toml").write_text('[workspace]\nmembers = ["dut"]\nresolver = "2"\n')
            (root / "dut/src").mkdir(parents=True)
            (root / "dut/src/main.rs").write_text("fn main() {}\n")
            (root / "dut/Cargo.toml").write_text('[package]\nname = "other"\nversion = "0.1.0"\nedition = "2021"\n')
            subprocess.run(["cargo", "generate-lockfile", "-q"], cwd=root, check=True)
            self.assertIn("rvoip-bench-uas, is not in the workspace", bench.preflight(root))
            (root / "dut/Cargo.toml").write_text(
                '[package]\nname = "rvoip-bench-uas"\nversion = "0.1.0"\nedition = "2021"\n'
            )
            subprocess.run(["cargo", "generate-lockfile", "-q"], cwd=root, check=True)
            baselines = root / bench.BASELINES
            # The state main is in between #425 and the bench environment PR.
            self.assertIn("ENVIRONMENT is missing", bench.preflight(root))
            baselines.mkdir(parents=True)
            (baselines / "ENVIRONMENT").write_text("env-a\n")
            self.assertIn("env-a/env.json is missing", bench.preflight(root))
            (baselines / "env-a").mkdir()
            (baselines / "env-a/env.json").write_text("{}")
            self.assertIn("env-a/rates.json is missing", bench.preflight(root))
            (baselines / "env-a/rates.json").write_text("{}")
            self.assertIsNone(bench.preflight(root))
        # In this checkout, until the environment lands, the gate stops before building.
        if bench.preflight(ROOT):
            self.assertEqual(
                bench.main(["--profile", "minor", "--candidate", "c" * 40, "--receipt", "x.json"]),
                bench.EXIT_USAGE,
            )

    def test_lab_teardowns_are_reused_with_their_chain_not_always_fresh(self) -> None:
        by_id = {gate["id"]: gate for gate in self.catalog["gates"]}
        for gate_id in (
            "interop.proxy-pbx.kamailio.down",
            "interop.proxy-pbx.opensips.down",
            "interop.amr-rate-sweep.down",
            "interop.jambonz-down",
        ):
            with self.subTest(gate=gate_id):
                self.assertFalse(by_id[gate_id]["always_fresh"])
                self.assertTrue(by_id[gate_id]["dependencies"])
        latest = by_id["interop.jambonz-latest"]
        self.assertTrue(latest["always_fresh"])
        self.assertEqual(latest["resource_class"], "github-evidence")
        # Jambonz has its own interop lane and worker now.
        self.assertNotIn("interop.freeswitch-down-after", by_id["interop.jambonz-up"]["dependencies"])
        self.assertEqual(by_id["interop.jambonz-up"]["interop_lane"], "jambonz")

    def test_bump_requalification_needs_no_ec2_worker(self) -> None:
        """Everything still always-fresh in a release profile runs hosted."""
        by_id = {gate["id"]: gate for gate in self.catalog["gates"]}
        for profile in ("remote-release", "release-patch", "release-minor"):
            fresh = [
                gate_id
                for gate_id in self.catalog["profiles"][profile]
                if by_id[gate_id].get("always_fresh")
            ]
            with self.subTest(profile=profile):
                self.assertTrue(fresh)
                self.assertEqual(
                    {by_id[gate_id]["resource_class"] for gate_id in fresh},
                    {"github-evidence"},
                )

    def test_bench_gates_are_release_only(self) -> None:
        by_id = {gate["id"]: gate for gate in self.catalog["gates"]}
        profiles = self.catalog["profiles"]
        bench = {"perf.rfc7502-release", "perf.rfc7502-minor"}
        self.assertFalse(bench & set(profiles["remote-release"]))
        self.assertFalse(bench & set(by_id["report.remote-aggregate"]["dependencies"]))
        self.assertEqual(
            set(profiles["release-patch"]) - set(profiles["remote-release"]),
            {"perf.rfc7502-release"},
        )
        # Every release confirms the recorded R; the full search is on demand (bench-minor).
        self.assertEqual(
            set(profiles["release-minor"]) - set(profiles["remote-release"]),
            {"perf.rfc7502-release"},
        )
        self.assertEqual(profiles["bench-minor"], ["perf.rfc7502-minor", "source.remote-clean"])
        for gate_id, profile in (("perf.rfc7502-release", "release"), ("perf.rfc7502-minor", "minor")):
            gate = by_id[gate_id]
            with self.subTest(gate=gate_id):
                self.assertEqual(gate["resource_class"], "bench-lane")
                self.assertFalse(gate["always_fresh"])
                self.assertEqual(
                    gate["command"],
                    [
                        "python3",
                        "{workspace}/scripts/release/bench_gate.py",
                        "--profile",
                        profile,
                        "--candidate",
                        "{candidate}",
                        "--receipt",
                        "{artifact_dir}/bench-receipt.json",
                    ],
                )
                self.assertIn("scripts/bench/**", gate["affected_paths"])
                self.assertIn(3, gate["retry_on_exit_codes"])
                # The recorded R values and the pinned tester environment
                # decide the verdict, so a change to them reruns the lane.
                for path in (
                    "crates/sip/rvoip-sip/perf-baselines/rfc7502/ENVIRONMENT",
                    "crates/sip/rvoip-sip/perf-baselines/rfc7502/env-a/env.json",
                    "crates/tools/rvoip-bench-uas/src/main.rs",
                    "scripts/bench/run_lane.py",
                ):
                    self.assertTrue(gates.matches(path, gate["affected_paths"]), path)
        matrix = gates.matrix_for(
            [{"id": "perf.rfc7502-release", "decision": "RUN"}], by_id
        )
        self.assertEqual(len(matrix), 1)
        self.assertTrue(matrix[0]["bench"])
        self.assertFalse(matrix[0]["hosted"])


if __name__ == "__main__":
    unittest.main()
