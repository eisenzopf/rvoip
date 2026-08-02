#!/usr/bin/env python3
"""Source fences for release-gate compatibility coverage."""

from __future__ import annotations

import json
import pathlib
import re
import stat
import subprocess
import tomllib
import unittest


SCRIPT_DIR = pathlib.Path(__file__).resolve().parent
CRATE_DIR = SCRIPT_DIR.parent
WORKSPACE_ROOT = CRATE_DIR.parents[2]
BETA_GATE = (SCRIPT_DIR / "beta_gate.sh").read_text(encoding="utf-8")
FULL_BETA_RELEASE_PATH = SCRIPT_DIR / "full_beta_release.sh"
FULL_BETA_RELEASE = FULL_BETA_RELEASE_PATH.read_text(encoding="utf-8")
PERF_SOAK_SPLIT = (SCRIPT_DIR / "perf_soak_split.sh").read_text(encoding="utf-8")


def shell_function(name: str) -> str:
    match = re.search(
        rf"^{re.escape(name)}\(\) \{{\n(?P<body>.*?)^\}}$",
        BETA_GATE,
        flags=re.MULTILINE | re.DOTALL,
    )
    if match is None:
        raise AssertionError(f"missing shell function {name}")
    return match.group("body")


class BetaGateCompatibilitySourceTests(unittest.TestCase):
    def test_split_soak_builds_both_exact_binaries_in_one_cargo_invocation(self) -> None:
        build = re.search(
            r"^build_exact_test_bins\(\) \{\n(?P<body>.*?)^\}$",
            PERF_SOAK_SPLIT,
            flags=re.MULTILINE | re.DOTALL,
        )
        self.assertIsNotNone(build)
        body = build.group("body")
        self.assertEqual(body.count("cargo test"), 1)
        self.assertIn("--test perf_soak_receiver", body)
        self.assertIn("--test perf_soak_caller", body)
        self.assertEqual(body.count("--build-target perf_soak_receiver"), 1)
        self.assertEqual(body.count("--build-target perf_soak_caller"), 1)

    def test_full_beta_release_wrapper_is_fail_closed_and_literal_all(self) -> None:
        self.assertIn(
            'DOCKER_BIN="$HOMEBREW_PREFIX/opt/docker/bin/docker"',
            FULL_BETA_RELEASE,
        )
        self.assertIn(
            'DOCKER_COMPOSE_BIN="$HOMEBREW_PREFIX/opt/docker-compose/bin/docker-compose"',
            FULL_BETA_RELEASE,
        )
        self.assertNotIn("Docker.app", FULL_BETA_RELEASE)
        self.assertNotIn("require_clean_peer", FULL_BETA_RELEASE)
        self.assertNotIn('"/.git"', FULL_BETA_RELEASE)
        self.assertIn("--network-address", FULL_BETA_RELEASE)
        self.assertIn('"$DOCKER_BIN" context use colima', FULL_BETA_RELEASE)
        self.assertIn("--activate=false", FULL_BETA_RELEASE)
        self.assertLess(
            FULL_BETA_RELEASE.index(
                'ORIGINAL_DOCKER_CONTEXT="$("$DOCKER_BIN" context show'
            ),
            FULL_BETA_RELEASE.index('if ! colima_profile_is_release_ready; then'),
        )
        self.assertIn("env -i", FULL_BETA_RELEASE)
        self.assertIn("BETA_*|RVOIP_*|PBX_*", FULL_BETA_RELEASE)
        self.assertIn("COLIMA_*|LIMA_*|XDG_CONFIG_HOME", FULL_BETA_RELEASE)
        self.assertIn("for pass in 1 2 3; do", FULL_BETA_RELEASE)
        self.assertIn(
            'prefix = "[perf-2k] mode=clean status=0 artifacts="',
            FULL_BETA_RELEASE,
        )
        self.assertIn("helper.validate_run(path, fingerprint)", FULL_BETA_RELEASE)
        self.assertIn("require_clean_source", FULL_BETA_RELEASE)

        required_configuration = (
            "RVOIP_REQUIRE_API_TOOLS=1",
            "BETA_REPORT_PACKAGE=1",
            "BETA_REQUIRE_CLEAN_SOURCE=1",
            "BETA_REQUIRE_CANONICAL_2K_EVIDENCE=1",
            "BETA_RUN_LOCAL_PBX=1",
            "BETA_PBX_PROVIDER=both",
            "BETA_PBX_API=all",
            "BETA_PBX_SCENARIO=all",
            "BETA_RUN_SIPP=1",
            "BETA_RUN_STRICT_UA=1",
            "BETA_RUN_FUZZ_SMOKE=1",
            "BETA_RUN_PERF_ALL=1",
            "BETA_PERF_REGRESSION_FAIL=1",
            "BETA_RUN_BURST_MATRIX=1",
            "BETA_BURST_MATRIX=all",
            "BETA_RUN_LONG_SOAK=1",
            "RVOIP_PERF_SKIP_AUDIO_FRAME_DELIVERY=0",
        )
        for value in required_configuration:
            self.assertIn(value, FULL_BETA_RELEASE)
        self.assertIn('"$BETA_GATE" --full --require-external', FULL_BETA_RELEASE)

    def test_full_beta_release_wrapper_is_executable_and_documented(self) -> None:
        self.assertNotEqual(
            FULL_BETA_RELEASE_PATH.stat().st_mode & stat.S_IXUSR,
            0,
            "full beta wrapper must remain executable",
        )
        subprocess.run(
            ["bash", "-n", str(FULL_BETA_RELEASE_PATH)],
            check=True,
            cwd=WORKSPACE_ROOT,
        )
        help_result = subprocess.run(
            [str(FULL_BETA_RELEASE_PATH), "--help"],
            check=True,
            cwd=WORKSPACE_ROOT,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        self.assertIn("Usage: full_beta_release.sh", help_result.stdout)
        for document in (
            CRATE_DIR / "README.md",
            CRATE_DIR / "docs/BETA_RELEASE_CHECKLIST.md",
        ):
            self.assertIn(
                "crates/sip/rvoip-sip/scripts/full_beta_release.sh",
                document.read_text(encoding="utf-8"),
                f"{document.name} must expose the one-command beta release",
            )

    def test_selected_state_table_evidence_is_explicit_and_fail_closed(self) -> None:
        self.assertIn(
            'BETA_STATE_TABLE_SOURCE="${BETA_STATE_TABLE_SOURCE:-embedded-default}"',
            BETA_GATE,
        )
        self.assertIn(
            'configured-path requires BETA_STATE_TABLE_SELECTED_YAML',
            BETA_GATE,
        )
        self.assertIn(
            'configured state-table evidence requires configured-path and an explicit selected YAML file',
            BETA_GATE,
        )
        attestation = shell_function("write_beta_attestation")
        self.assertIn(
            '--input "state-machine-yaml=$selected_state_table_yaml"',
            attestation,
        )
        self.assertNotIn(
            '--input "state-machine-yaml=$CRATE_DIR/state_tables/default.yaml"',
            attestation,
        )

    def test_local_gate_requires_public_api_fence(self) -> None:
        local = shell_function("run_local_gates")
        self.assertIn(
            'run_gate_continue "public API compatibility" "$SCRIPT_DIR/check_public_api.sh"',
            local,
        )
        self.assertIn('if [ "$MODE" = "full" ]; then\n    RVOIP_REQUIRE_API_TOOLS=1', BETA_GATE)
        self.assertIn("export RVOIP_REQUIRE_API_TOOLS", BETA_GATE)

    def test_local_gate_executes_all_workspace_unit_tests_and_doctests(self) -> None:
        local = shell_function("run_local_gates")
        self.assertIn(
            'run_gate_continue "workspace unit tests" cargo test --workspace --exclude rvoip-sip --lib',
            local,
        )
        self.assertIn(
            'run_gate_continue "workspace target and integration tests" cargo test --workspace --exclude rvoip-sip --bins --examples --tests',
            local,
        )
        self.assertIn(
            'run_gate_continue "workspace doctests" cargo test --workspace --exclude rvoip-sip --doc',
            local,
        )
        self.assertIn(
            'run_gate_continue "rvoip-sip unit tests" cargo test -p rvoip-sip --lib',
            local,
        )
        self.assertIn(
            'run_gate_continue "rvoip-sip doctests" cargo test -p rvoip-sip --doc',
            local,
        )

    def test_downstream_invocations_are_package_isolated(self) -> None:
        downstream = shell_function("run_downstream_compatibility_gates")
        expected_packages = {
            "rvoip",
            "rvoip-client",
            "rvoip-core",
            "rvoip-amazon-connect",
            "rvoip-uctp",
            "rvoip-quic",
            "rvoip-webtransport",
            "rvoip-websocket",
            "rvoip-webrtc",
            "rvoip-audio-device",
        }
        seen: set[str] = set()
        blocks = re.split(
            r'(?=^\s*run_gate_continue "downstream )',
            downstream,
            flags=re.MULTILINE,
        )
        gate_blocks = [
            block
            for block in blocks
            if block.lstrip().startswith("run_gate_continue")
        ]
        self.assertGreaterEqual(len(gate_blocks), len(expected_packages))
        for block in gate_blocks:
            packages = re.findall(r"(?:^|\s)-p\s+([a-z0-9-]+)(?:\s|$)", block)
            self.assertEqual(
                len(packages),
                1,
                f"downstream gate must contain exactly one package: {block}",
            )
            seen.add(packages[0])
        self.assertEqual(seen, expected_packages)

    def test_independent_gate_failures_do_not_abort_final_evidence(self) -> None:
        direct_gate_lines = [
            line.strip()
            for line in BETA_GATE.splitlines()
            if re.match(r'^\s*run_gate\s+"', line)
        ]
        self.assertEqual(
            direct_gate_lines,
            ['run_gate "$@" || true'],
            "independent gates must use run_gate_continue; direct run_gate is reserved "
            "for success-dependent conditional control",
        )
        continuation = shell_function("run_gate_continue")
        self.assertIn('run_gate "$@" || true', continuation)
        self.assertIn('write_beta_attestation "$ARTIFACT_DIR"', BETA_GATE)
        self.assertIn("package_beta_report", BETA_GATE)

    def test_terminal_source_gates_remain_in_attested_gate_table(self) -> None:
        final_source_gate = BETA_GATE.index(
            'run_gate_continue "beta final source fingerprint capture"'
        )
        unchanged_source_gate = BETA_GATE.index(
            'run_gate_continue "canonical 2k beta source unchanged"'
        )
        performance_details = BETA_GATE.index(
            'cat "$ARTIFACT_DIR/performance-gate-metrics.md" >> "$SUMMARY"'
        )
        self.assertLess(final_source_gate, performance_details)
        self.assertLess(unchanged_source_gate, performance_details)

    def test_docker_peer_evidence_is_sanitized_before_disk_capture(self) -> None:
        capture = shell_function("capture_docker_snapshot")
        self.assertIn('python3 "$DOCKER_PEER_SNAPSHOT_HELPER"', capture)
        self.assertIn('-peer.json', capture)
        self.assertNotIn('-inspect.json', capture)
        self.assertNotIn('capture_command "$dir/${container}-inspect.json"', capture)

    def test_perf_results_have_a_recoverable_per_run_boundary(self) -> None:
        prepare = shell_function("prepare_perf_results_capture")
        capture = shell_function("capture_current_perf_results")
        package = shell_function("copy_perf_results_into_report")
        self.assertIn('mv "$PERF_RESULTS_DIR" "$archive_dir"', prepare)
        self.assertNotIn("rm -rf", prepare)
        self.assertIn('cd "$PERF_RESULTS_DIR"', capture)
        self.assertIn('$ARTIFACT_DIR/perf-results', capture)
        self.assertIn('$ARTIFACT_DIR/perf-results', package)
        self.assertNotIn('$WORKSPACE_ROOT/target/perf-results', package)
        self.assertIn("run_isolated_perf_gates", BETA_GATE)

    def test_pbx_g729_features_are_automatically_attested(self) -> None:
        features = shell_function("attestation_features")
        self.assertIn('PBX_CARGO_FEATURES:-dev-insecure-tls,g729', features)
        self.assertIn('BETA_RUN_LOCAL_PBX:-0', features)
        self.assertIn('BETA_RUN_PBX:-0', features)

    def test_externally_managed_pbx_evidence_is_captured_in_report(self) -> None:
        interop = shell_function("run_interop_gates")
        self.assertIn('PBX_OUT_ROOT="$ARTIFACT_DIR/pbx"', interop)
        self.assertIn('--pbx "${BETA_PBX_PROVIDER:-both}"', interop)
        self.assertIn('--api "${BETA_PBX_API:-all}"', interop)
        self.assertIn('--scenario "${BETA_PBX_SCENARIO:-all}"', interop)

    def test_release_qualification_inputs_are_in_machine_readable_summary(self) -> None:
        summary = shell_function("write_summary_gate_table_header")
        for field in (
            "beta_gate_require_external",
            "beta_profile_matrix",
            "beta_run_perf_all",
            "beta_perf_regression_fail",
            "rvoip_require_api_tools",
            "beta_perf_regression_baseline_id",
            "beta_perf_regression_baseline_manifest_sha256",
            "beta_run_burst_matrix",
            "beta_burst_matrix",
            "beta_run_long_soak",
            "beta_run_local_pbx",
            "beta_run_pbx",
            "beta_pbx_provider",
            "beta_pbx_api",
            "beta_pbx_scenario",
            "beta_pbx_g729_profiles",
        ):
            self.assertIn(f"- {field}:", summary)

    def test_perf_regression_uses_packaged_reviewed_baseline(self) -> None:
        audit = shell_function("run_perf_regression_audit")
        self.assertIn('"$PERF_REGRESSION_BASELINE_HELPER" package', audit)
        self.assertIn(
            'local baseline="$ARTIFACT_DIR/perf-regression-baseline/perf-results"',
            audit,
        )
        self.assertNotIn('"$(beta_report_root)"/*/perf-results', audit)
        attestation = shell_function("write_beta_attestation")
        self.assertIn(
            '--input "performance-regression-baseline=$PERF_REGRESSION_BASELINE_MANIFEST"',
            attestation,
        )
        self.assertIn(
            '--input "performance-regression-baseline-helper=$PERF_REGRESSION_BASELINE_HELPER"',
            attestation,
        )

    def test_one_retention_wait_is_exported_to_every_monolithic_target(self) -> None:
        self.assertIn(
            'RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS="${RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS:-130}"',
            BETA_GATE,
        )
        self.assertIn("export RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS", BETA_GATE)
        perf = shell_function("run_perf_gates")
        self.assertGreaterEqual(
            perf.count(
                'RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS="$RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS"'
            ),
            2,
        )

    def test_pbx_and_sipp_scripts_resolve_the_workspace_root(self) -> None:
        pbx = (CRATE_DIR / "examples/pbx/run.sh").read_text(encoding="utf-8")
        sipp = (
            CRATE_DIR / "tests/perf/sipp_scenarios/run_comparison.sh"
        ).read_text(encoding="utf-8")
        self.assertIn(
            'WORKSPACE_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../../../../.." && pwd)',
            pbx,
        )
        self.assertIn(
            'WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/../../../../../.." && pwd)"', sipp
        )

    def test_appended_pbx_provider_only_fails_for_new_rows(self) -> None:
        pbx = (CRATE_DIR / "examples/pbx/run.sh").read_text(encoding="utf-8")
        self.assertIn("RUN_INITIAL_FAILURES=0", pbx)
        self.assertIn("RUN_INITIAL_FAILURES=$(awk", pbx)
        self.assertIn('if [ "$failures" -gt "$RUN_INITIAL_FAILURES" ]; then', pbx)

    def test_pbx_tls_readiness_tolerates_expected_startup_probe_failures(self) -> None:
        pbx = (CRATE_DIR / "examples/pbx/run.sh").read_text(encoding="utf-8")
        self.assertIn('if nc -z -w 2 "$host" "$port"; then', pbx)
        self.assertIn("if printf '' \\\n", pbx)
        self.assertIn('openssl s_client -connect "$host:$port"', pbx)

    def test_all_thirteen_standalone_manifests_are_independent_gates(self) -> None:
        examples = shell_function("run_standalone_example_gates")
        inventory_match = re.search(
            r"local -a examples=\(\n(?P<items>.*?)^\s*\)",
            examples,
            flags=re.MULTILINE | re.DOTALL,
        )
        self.assertIsNotNone(inventory_match)
        inventory = inventory_match.group("items").split()
        expected = [
            "01-quickstart-p2p",
            "02-softphone-audio",
            "03-register-to-pbx",
            "04-call-control",
            "05-blind-transfer",
            "06-attended-transfer",
            "07-secure-call-srtp",
            "08-tls-transport",
            "09-ivr-server",
            "10-call-center-b2bua",
            "11-ai-harness-demo",
            "12-customer-escalation-sip-webrtc",
            "13-sip-to-amazon-connect",
        ]
        self.assertEqual(inventory, expected)
        for example in inventory:
            self.assertTrue(
                (WORKSPACE_ROOT / "examples" / example / "Cargo.toml").is_file(),
                f"missing standalone manifest for {example}",
            )
        self.assertIn(
            '--manifest-path "$WORKSPACE_ROOT/examples/$example/Cargo.toml"',
            examples,
        )

        catalog = json.loads(
            (WORKSPACE_ROOT / "scripts/release/gates.json").read_text(encoding="utf-8")
        )
        gates = {gate["id"]: gate for gate in catalog["gates"]}
        for example in expected:
            gate_id = f"test.example-{example.split('-', maxsplit=1)[0]}"
            self.assertIn(gate_id, gates, f"release catalog omits {example}")
            self.assertIn(
                f"{{workspace}}/examples/{example}/Cargo.toml",
                gates[gate_id]["command"],
                f"release catalog gate {gate_id} targets the wrong manifest",
            )

    def test_active_beta_metadata_matches_workspace_version(self) -> None:
        root_manifest = tomllib.loads(
            (WORKSPACE_ROOT / "Cargo.toml").read_text(encoding="utf-8")
        )
        version = root_manifest["workspace"]["package"]["version"]

        active_files = [
            WORKSPACE_ROOT / "README.md",
            WORKSPACE_ROOT / "crates/rvoip/README.md",
            WORKSPACE_ROOT / "crates/foundation/rvoip-core/README.md",
            WORKSPACE_ROOT / "crates/foundation/rvoip-core-traits/README.md",
            WORKSPACE_ROOT / "crates/media/codec-core/README.md",
            WORKSPACE_ROOT / "crates/identity/auth-core/README.md",
            WORKSPACE_ROOT / "crates/identity/auth-core/Cargo.toml",
            WORKSPACE_ROOT / "crates/sip/sip-core/README.md",
            WORKSPACE_ROOT / "crates/sip/sip-proxy/README.md",
            WORKSPACE_ROOT / "crates/sip/sip-proxy/Cargo.toml",
            WORKSPACE_ROOT / "examples/README.md",
            *sorted((WORKSPACE_ROOT / "examples").glob("*/Cargo.toml")),
        ]
        for path in active_files:
            body = path.read_text(encoding="utf-8")
            self.assertNotIn(
                "0.2.2",
                body,
                f"{path.relative_to(WORKSPACE_ROOT)} has stale beta metadata; "
                f"workspace version is {version}",
            )

    def test_release_procedures_pin_api_retention_and_regression_baseline(self) -> None:
        documents = [CRATE_DIR / "docs/BETA_RELEASE_CHECKLIST.md"]
        required = [
            "RVOIP_REQUIRE_API_TOOLS=1",
            "RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS=160",
            "BETA_PERF_REGRESSION_BASELINE_ROOT=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z",
            "BETA_PERF_REGRESSION_BASELINE_MANIFEST=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json",
        ]
        for document in documents:
            body = document.read_text(encoding="utf-8")
            for value in required:
                self.assertIn(value, body, f"{document.name} omits {value}")


if __name__ == "__main__":
    unittest.main()
