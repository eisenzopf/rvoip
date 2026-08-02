#!/usr/bin/env python3
"""Focused fixture tests for beta report attestation and verification."""

import argparse
import hashlib
import importlib.util
import json
import pathlib
import shutil
import subprocess
import tempfile
import unittest


SCRIPT_DIR = pathlib.Path(__file__).resolve().parent
WORKSPACE_ROOT = SCRIPT_DIR.parents[3]
SPEC = importlib.util.spec_from_file_location(
    "test_beta_attestation_impl", SCRIPT_DIR / "beta_attestation.py"
)
attestation = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(attestation)


def digest(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


class BetaAttestationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.base = pathlib.Path(self.temp.name)
        self.report = self.base / "report"
        self.environment = self.report / "environment"
        self.environment.mkdir(parents=True)
        self.fingerprint = digest(b"fixture source")
        self.commit = digest(b"fixture commit")[:40]
        self.tree = digest(b"fixture tree")[:40]
        workspace_manifest = (WORKSPACE_ROOT / "Cargo.toml").read_text(encoding="utf-8")
        package_section = workspace_manifest.split("[workspace.package]", 1)[1]
        version_line = next(
            line
            for line in package_section.splitlines()
            if line.startswith('version = "')
        )
        self.workspace_version = version_line.split('"', 2)[1]
        self.write_source("source-at-beta-start.json", self.fingerprint, False)
        self.write_source("source-at-beta-end.json", self.fingerprint, False)
        (self.environment / "rustc-version.txt").write_text(
            "+ rustc --version --verbose\n"
            "rustc 1.95.0 (fixture 2026-01-01)\n"
            "binary: rustc\ncommit-hash: fixture\n"
            "host: aarch64-apple-darwin\n",
            encoding="utf-8",
        )
        (self.environment / "cargo-version.txt").write_text(
            "+ cargo --version --verbose\ncargo 1.95.0 (fixture 2026-01-01)\n",
            encoding="utf-8",
        )
        (self.environment / "beta-env-redacted.txt").write_text(
            "BETA_RUN_PBX=1\nTOKEN=<redacted>\n", encoding="utf-8"
        )
        self.fixture_executable = self.base / "target/debug/deps/fixture_gate-deadbeef"
        self.fixture_executable.parent.mkdir(parents=True)
        self.fixture_executable.write_bytes(b"fixture executable")
        self.fixture_executable.chmod(0o755)
        (self.environment / "cargo-metadata.json").write_text(
            json.dumps(
                {
                    "target_directory": str(self.base / "target"),
                    "packages": [
                        {"name": "rvoip-sip", "version": self.workspace_version},
                        {"name": "other", "version": "0.1.0"},
                    ],
                }
            )
            + "\n",
            encoding="utf-8",
        )
        self.log = self.report / "perf_fixture_gate.log"
        self.log.write_text(
            f"gate: fixture gate\nRunning `{self.fixture_executable}`\n"
            "duration_seconds: 2\nexit_status: 0\n",
            encoding="utf-8",
        )
        result_dir = self.report / "perf-results"
        result_dir.mkdir()
        (result_dir / "result.json").write_text(
            json.dumps(
                {
                    "environment": {
                        "source_fingerprint_sha256": self.fingerprint,
                        "rvoip_sip_version": self.workspace_version,
                    },
                    "effective_config": {"media": True, "capacity": 256},
                    "status": "PASS",
                }
            )
            + "\n",
            encoding="utf-8",
        )
        for relative in sorted(
            attestation.STANDARD_PERFORMANCE_RESULT_PATHS
            | attestation.LITERAL_ALL_PERFORMANCE_RESULT_PATHS
        ):
            path = self.report / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(
                json.dumps(
                    {
                        "environment": {
                            "source_fingerprint_sha256": self.fingerprint,
                            "rvoip_sip_version": self.workspace_version,
                        },
                        "status": "PASS",
                    }
                )
                + "\n",
                encoding="utf-8",
            )
        self.write_reconciled_performance_results()
        burst_result = (
            result_dir
            / "perf_burst_matrix/burst_fixture/carrier-smoke/perf_burst_caller.json"
        )
        burst_result.parent.mkdir(parents=True)
        burst_result.write_text('{"status":"PASS"}\n', encoding="utf-8")
        for relative in sorted(
            attestation.PBX_EVIDENCE_PATHS | attestation.INTEROP_EVIDENCE_PATHS
        ):
            path = self.report / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(f"fixture evidence for {relative}\n", encoding="utf-8")
        (self.report / "proxy-interop/summary.json").write_text(
            json.dumps(
                {
                    "schema": "rvoip-sip-proxy-interop-v1",
                    "configuration": {
                        "kamailio_image": "kamailio@sha256:" + "a" * 64,
                        "kamailio_platform": "linux/amd64",
                        "opensips_image": "opensips@sha256:" + "b" * 64,
                        "opensips_platform": "linux/amd64",
                    },
                    "rows": [
                        {"peer": "kamailio", "status": "PASS"},
                        {"peer": "opensips", "status": "PASS"},
                    ],
                }
            )
            + "\n",
            encoding="utf-8",
        )
        for relative in sorted(attestation.PERFORMANCE_GATE_METRICS_PATHS):
            path = self.report / relative
            path.write_text("fixture performance metrics\n", encoding="utf-8")
        strict = self.report / "strict-ua"
        (strict / "baresip").mkdir(parents=True)
        (strict / "environment.md").write_text(
            "# Strict UA\n\n- baresip: baresip v4.8.0\n", encoding="utf-8"
        )
        (strict / "baresip/config").write_text("sip_tos 0\n", encoding="utf-8")
        for product in ("asterisk", "freeswitch"):
            local_peer = self.environment / "local-pbx" / product
            local_peer.mkdir(parents=True)
            (local_peer / "git-rev.txt").write_text(
                f"{product}-fixture-revision\n", encoding="utf-8"
            )
        sipp = self.report / "sipp"
        sipp.mkdir(exist_ok=True)
        (sipp / "environment.md").write_text(
            "# SIPp\n\n- sipp: SIPp v3.7.4\n", encoding="utf-8"
        )
        self.state_yaml = self.base / "default.yaml"
        self.state_yaml.write_text("version: 1\nstates: [Idle]\n", encoding="utf-8")
        self.recipe_yaml = self.base / "performance-recipes.yaml"
        self.recipe_yaml.write_text("profiles: {}\n", encoding="utf-8")
        self.burst_yaml = self.base / "perf-burst-scenarios.yaml"
        self.burst_yaml.write_text("scenarios: {}\n", encoding="utf-8")
        self.sipp_scenario = self.base / "uac_perf.xml"
        self.sipp_scenario.write_text("<scenario/>\n", encoding="utf-8")
        self.performance_baseline_manifest = (
            WORKSPACE_ROOT
            / "crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json"
        )
        self.assertEqual(
            digest(self.performance_baseline_manifest.read_bytes()),
            attestation.PERFORMANCE_REGRESSION_BASELINE_SHA256,
        )
        self.write_summary("PASS", failures=0, skips=0, mode="full")

    def tearDown(self):
        self.temp.cleanup()

    def write_source(self, name, fingerprint, dirty):
        (self.environment / name).write_text(
            json.dumps(
                {
                    "git_commit": self.commit,
                    "git_rev": self.commit[:8],
                    "git_tree": self.tree,
                    "git_dirty": dirty,
                    "source_fingerprint_sha256": fingerprint,
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )

    def write_reconciled_performance_results(self):
        common = {
            "environment": {
                "source_fingerprint_sha256": self.fingerprint,
                "rvoip_sip_version": self.workspace_version,
            },
            "status": "PASS",
        }
        payloads = {
            "perf_session_churn_leak.json": {
                "results": {"retention_drain_wait_secs": 120}
            },
            "perf_media_churn.json": {
                "results": {
                    "duration_secs": 120,
                    "active_calls_target": 30,
                    "active_call_min_hold_secs": 10,
                    "active_call_max_hold_secs": 360,
                    "retention_drain_wait_secs": 120,
                }
            },
            "perf_soak_30min.json": {
                "results": {
                    "duration_secs": 1800,
                    "active_calls_target": 30,
                    "active_call_min_hold_secs": 10,
                    "active_call_max_hold_secs": 360,
                    "soak_cps": 0,
                    "controlled_drain_cps": 10,
                    "retention_drain_wait_secs": 120,
                    "rss_gate": {"effective_mb_per_hr": 15},
                }
            },
            "perf_mass_teardown_stress.json": {
                "load": {"target_cps": 30, "cooldown_secs": 120},
                "results": {
                    "calls_requested": 500,
                    "mass_teardown_setup_cps": 30,
                    "retention_drain_wait_secs": 120,
                },
            },
            "perf_soak_caller.json": {
                "results": {
                    "duration_secs": 3600,
                    "active_calls_target": 500,
                    "active_call_min_hold_secs": 10,
                    "active_call_max_hold_secs": 360,
                    "soak_cps": 0,
                    "retention_drain_wait_secs": 120,
                    "rss_gate": {"effective_mb_per_hr": 15},
                }
            },
            "perf_soak_receiver.json": {
                "results": {
                    "configured_duration_secs": 3600,
                    "active_calls_target": 500,
                    "retention_drain_wait_secs": 120,
                    "rss_gate": {"effective_mb_per_hr": 15},
                }
            },
        }
        for name, payload in payloads.items():
            path = self.report / "perf-results" / name
            value = dict(common)
            value.update(payload)
            path.write_text(json.dumps(value) + "\n", encoding="utf-8")

    def write_summary(
        self,
        status,
        failures,
        skips,
        *,
        state_table_source="embedded-default",
        state_table_fallback_reason=None,
        require_external=False,
        mode="full",
        include_executable=True,
        config_overrides=None,
        omit_gate=None,
    ):
        config = self.fixture_gate_config(
            mode,
            require_external=require_external,
            state_table_source=state_table_source,
            state_table_fallback_reason=state_table_fallback_reason,
        )
        config.update(config_overrides or {})
        gate_names = attestation.required_gate_names(mode, config)
        if omit_gate is not None:
            gate_names = [name for name in gate_names if name != omit_gate]
        rows = []
        for index, name in enumerate(gate_names):
            gate_status = status if index == 0 else "PASS"
            duration = "0s" if gate_status == "SKIP" else "2s"
            relative_log = (
                "perf_fixture_gate.log" if index == 0 else f"gate_{index:03}.log"
            )
            running = (
                f"Running `{self.fixture_executable}`\n"
                if index == 0 and include_executable
                else ""
            )
            (self.report / relative_log).write_text(
                f"gate: {name}\n{running}duration_seconds: "
                f"{0 if gate_status == 'SKIP' else 2}\n"
                f"exit_status: {'not-run' if gate_status == 'SKIP' else (1 if gate_status == 'FAIL' else 0)}\n",
                encoding="utf-8",
            )
            if index == 0:
                self.log = self.report / relative_log
            rows.append(
                f"| {gate_status} | {name} | {duration} | `{relative_log}` |"
            )
        environment = "".join(
            f"- {key}: `{value}`\n" for key, value in sorted(config.items())
        )
        self.report.joinpath("summary.md").write_text(
            "# Beta Gate\n\n"
            "## Environment Snapshot\n\n"
            f"{environment}\n"
            "## Gates\n\n"
            "| Status | Gate | Duration | Log |\n"
            "|--------|------|----------|-----|\n"
            + "\n".join(rows)
            + "\n\n## Result\n\n"
            f"- failures: {failures}\n- skips: {skips}\n",
            encoding="utf-8",
        )

    def fixture_gate_config(
        self,
        mode,
        *,
        require_external=False,
        state_table_source="embedded-default",
        state_table_fallback_reason=None,
    ):
        return {
            "beta_attestation_features": "perf-tests,generated-validation,perf-tests",
            "beta_attestation_target": "rustc-host",
            "beta_burst_matrix": "all",
            "beta_gate_require_external": str(int(require_external)),
            "beta_pbx_api": "all",
            "beta_pbx_g729_profiles": "g729a g729ab",
            "beta_pbx_provider": "both",
            "beta_pbx_scenario": "all",
            "beta_perf_high_density_burst_cps": "160",
            "beta_perf_high_density_min_asr": "0.995",
            "beta_perf_high_density_rss_limit_mb_per_hr": "15",
            "beta_perf_media_churn_active_calls": "30",
            "beta_perf_media_churn_duration_secs": "120",
            "beta_perf_monolithic_soak_active_calls": "30",
            "beta_perf_monolithic_soak_duration_secs": "1800",
            "beta_perf_regression_baseline_id": "20260706T181609Z",
            "beta_perf_regression_baseline_manifest_sha256": digest(
                self.performance_baseline_manifest.read_bytes()
            ),
            "beta_perf_regression_fail": "1",
            "beta_profile_matrix": "endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000",
            "beta_require_canonical_2k_evidence": "1" if mode == "full" else "0",
            "beta_require_clean_source": "1" if mode == "full" else "0",
            "beta_run_burst_matrix": "1",
            "beta_run_burst_smoke": "1",
            "beta_run_fuzz_smoke": "1",
            "beta_run_local_pbx": "0",
            "beta_run_long_soak": "1",
            "beta_run_pbx": "1",
            "beta_run_perf_all": "1",
            "beta_run_sipp": "1",
            "beta_run_strict_ua": "1",
            "beta_proxy_interop_peers": "kamailio opensips",
            "beta_proxy_interop_orders": "rvoip-first peer-first",
            "beta_proxy_interop_transports": "udp tcp tls",
            "beta_proxy_interop_retention_drain_seconds": "130",
            "beta_proxy_interop_require_clean_source": "1",
            "beta_proxy_interop_require_unchanged_source": "1",
            "beta_state_table_fallback_reason": state_table_fallback_reason
            or "none",
            "beta_state_table_sha256": digest(self.state_yaml.read_bytes()),
            "beta_state_table_source": state_table_source,
            "rvoip_perf_mass_teardown_calls": "500",
            "rvoip_perf_mass_teardown_setup_cps": "30",
            "rvoip_perf_max_rss_growth_mb_per_hr": "15",
            "rvoip_perf_retention_drain_wait_secs": "120",
            "rvoip_perf_soak_active_calls": "500",
            "rvoip_perf_soak_cps": "0",
            "rvoip_perf_soak_drain_cps": "10",
            "rvoip_perf_soak_duration_secs": "3600",
            "rvoip_perf_soak_max_hold_secs": "360",
            "rvoip_perf_soak_min_hold_secs": "10",
            "rvoip_perf_skip_audio_frame_delivery": "0",
            "rvoip_require_api_tools": "1",
        }

    def install_performance_baseline_fixture(self):
        package = self.report / attestation.PERFORMANCE_REGRESSION_BASELINE_PACKAGE
        if package.exists():
            return
        manifest = json.loads(self.performance_baseline_manifest.read_text(encoding="utf-8"))
        for item in manifest["files"]:
            source_result = self.performance_baseline_manifest.parent / item["path"]
            result = package / "perf-results" / item["path"]
            result.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source_result, result)
        package.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(self.performance_baseline_manifest, package / "manifest.json")

    def create(
        self,
        *,
        dirty=False,
        changed=False,
        failures=0,
        skips=0,
        mode="full",
        state_table_source="embedded-default",
        state_table_fallback_reason=None,
        omit_input=None,
        include_executable=True,
        require_external=False,
        config_overrides=None,
        omit_gate=None,
        schema_version=1,
    ):
        if dirty:
            self.write_source("source-at-beta-start.json", self.fingerprint, True)
            self.write_source("source-at-beta-end.json", self.fingerprint, True)
        if changed:
            self.write_source(
                "source-at-beta-end.json", digest(b"changed source"), False
            )
        status = "SKIP" if skips else ("FAIL" if failures else "PASS")
        if mode == "full":
            self.install_performance_baseline_fixture()
        self.write_summary(
            status,
            failures,
            skips,
            state_table_source=state_table_source,
            state_table_fallback_reason=state_table_fallback_reason,
            require_external=require_external,
            mode=mode,
            include_executable=include_executable,
            config_overrides=config_overrides,
            omit_gate=omit_gate,
        )
        inputs = [
            f"state-machine-yaml={self.state_yaml}",
            f"performance-recipe={self.recipe_yaml}",
            f"burst-scenarios={self.burst_yaml}",
            f"sipp-scenario={self.sipp_scenario}",
            f"attestation-verifier={SCRIPT_DIR / 'beta_attestation.py'}",
            "performance-regression-baseline="
            f"{self.performance_baseline_manifest}",
        ]
        if schema_version == 2:
            policy = self.base / "beta-release-policy.yaml"
            generator = self.base / "beta_release_report.py"
            policy.write_text('{"schema":"fixture-policy"}\n', encoding="utf-8")
            generator.write_text("# fixture generator\n", encoding="utf-8")
            inputs.extend(
                [
                    f"beta-release-policy={policy}",
                    f"beta-release-report-generator={generator}",
                ]
            )
            config = {
                "schema": attestation.STRUCTURED_CONFIG_SCHEMA,
                "binding_mode": "native-v2-input",
                "mode": mode,
                "values": [
                    {
                        "key": "beta_gate_mode",
                        "type": "enum",
                        "value": mode,
                        "source": "derived-setting",
                    }
                ],
                "values_by_key": {"beta_gate_mode": mode},
            }
            (self.report / "effective-gate-config.json").write_text(
                json.dumps(config, indent=2, sort_keys=True) + "\n",
                encoding="utf-8",
            )
            summary_gates = attestation.parse_gates(self.report.resolve())
            records = [
                {
                    "sequence": sequence,
                    "id": f"fixture.gate-{sequence}",
                    "name": gate["name"],
                    "status": gate["status"],
                }
                for sequence, gate in enumerate(summary_gates, 1)
            ]
            structured = {
                "schema": attestation.STRUCTURED_RESULTS_SCHEMA,
                "binding_mode": "native-v2-input",
                "mode": mode,
                "required_count": len(records),
                "passed": sum(item["status"] == "PASS" for item in records),
                "failed": sum(item["status"] == "FAIL" for item in records),
                "skipped": sum(item["status"] == "SKIP" for item in records),
                "records": records,
            }
            (self.report / "gate-results.json").write_text(
                json.dumps(structured, indent=2, sort_keys=True) + "\n",
                encoding="utf-8",
            )
        if omit_input is not None:
            inputs = [
                value for value in inputs if not value.startswith(f"{omit_input}=")
            ]
        args = argparse.Namespace(
            report_root=str(self.report),
            source_start="environment/source-at-beta-start.json",
            source_end="environment/source-at-beta-end.json",
            cargo_metadata="environment/cargo-metadata.json",
            mode=mode,
            run_id="20260720T000000Z",
            started_at="2026-07-20T00:00:00Z",
            ended_at="2026-07-20T00:01:00Z",
            features="perf-tests,generated-validation,perf-tests",
            target=None,
            input=inputs,
            state_table_source=state_table_source,
            state_table_fallback_reason=state_table_fallback_reason,
            state_table_sha256=digest(self.state_yaml.read_bytes()),
            failures=failures,
            skips=skips,
            overall="FAIL" if failures else "PASS",
            schema_version=schema_version,
        )
        return attestation.create_attestation(args)

    def rewrite_attestation(self, update):
        path = self.report / attestation.ATTESTATION_NAME
        value = json.loads(path.read_text(encoding="utf-8"))
        update(value)
        path.write_text(
            json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        (self.report / attestation.CHECKSUM_NAME).write_text(
            f"{attestation.sha256_file(path)}  {attestation.ATTESTATION_NAME}\n",
            encoding="ascii",
        )

    def install_canonical_2k_fixture(self):
        canonical = self.report / "canonical-2k"
        executable = canonical / "executable/fixture_gate"
        executable.parent.mkdir(parents=True)
        shutil.copyfile(self.fixture_executable, executable)
        executable.chmod(0o755)
        executable_sha256 = digest(executable.read_bytes())
        reviewed_baseline = canonical / attestation.CANONICAL_2K_BASELINE_PACKAGED_PATH
        reviewed_baseline.parent.mkdir(parents=True)
        tracked_baseline = (
            SCRIPT_DIR.parent
            / "perf-baselines"
            / attestation.CANONICAL_2K_BASELINE_ID
            / attestation.CANONICAL_2K_BASELINE_RELATIVE_PATH
        )
        shutil.copyfile(tracked_baseline, reviewed_baseline)
        self.assertEqual(
            digest(reviewed_baseline.read_bytes()),
            attestation.CANONICAL_2K_BASELINE_SHA256,
        )
        runs = []
        for sequence in range(1, 4):
            run_dir = canonical / f"run-{sequence}"
            run_dir.mkdir()
            (run_dir / "evidence.txt").write_text(
                f"canonical fixture {sequence}\n", encoding="utf-8"
            )
            tree = attestation.canonical_2k_tree_sha256(run_dir)
            runs.append(
                {
                    "sequence": sequence,
                    "source_run_dir": f"/unavailable/source-run-{sequence}",
                    "packaged_run_dir": f"run-{sequence}",
                    "captured_at_utc": f"2026-07-19T00:0{sequence}:00Z",
                    "source_fingerprint_sha256": self.fingerprint,
                    "executable_sha256": executable_sha256,
                    "reviewed_baseline_id": attestation.CANONICAL_2K_BASELINE_ID,
                    "reviewed_baseline_relative_path": attestation.CANONICAL_2K_BASELINE_RELATIVE_PATH,
                    "reviewed_baseline_sha256": attestation.CANONICAL_2K_BASELINE_SHA256,
                    "reviewed_baseline_origin": "tracked-default",
                    "source_tree_sha256": tree,
                    "packaged_tree_sha256": tree,
                }
            )
        (canonical / "index.json").write_text(
            json.dumps(
                {
                    "schema": attestation.CANONICAL_2K_SCHEMA,
                    "status": "PASS",
                    "scenario": "perf_call_setup_cps_pbx-media-server",
                    "run_count": 3,
                    "common_source_fingerprint_sha256": self.fingerprint,
                    "common_executable_sha256": executable_sha256,
                    "packaged_executable": "executable/fixture_gate",
                    "reviewed_baseline": {
                        "id": attestation.CANONICAL_2K_BASELINE_ID,
                        "relative_path": attestation.CANONICAL_2K_BASELINE_RELATIVE_PATH,
                        "sha256": attestation.CANONICAL_2K_BASELINE_SHA256,
                        "packaged_path": attestation.CANONICAL_2K_BASELINE_PACKAGED_PATH,
                    },
                    "runs": runs,
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )

    def test_create_and_verify_copied_report_without_workspace(self):
        self.create(require_external=True)
        manifest = attestation.verify_report(
            self.report,
            require_clean=True,
            require_unchanged_source=True,
            require_no_skips=True,
            require_pass=True,
        )
        self.assertEqual(manifest["schema"], attestation.SCHEMA)
        self.assertEqual(manifest["package"]["version"], self.workspace_version)
        self.assertEqual(
            manifest["build"]["features"],
            ["generated-validation", "perf-tests"],
        )
        self.assertEqual(
            manifest["inputs"]["state-machine-yaml"]["source_name"], "default.yaml"
        )
        self.assertEqual(manifest["state_table"]["selected_source"], "embedded-default")
        self.assertEqual(
            manifest["state_table"]["selected_yaml_sha256"],
            digest(self.state_yaml.read_bytes()),
        )

        self.assertEqual(
            manifest["state_table"]["selected_yaml_path"],
            manifest["inputs"]["state-machine-yaml"]["path"],
        )
        self.assertEqual(
            {peer["product"] for peer in manifest["peers"]},
            {
                "asterisk",
                "baresip",
                "freeswitch",
                "kamailio",
                "opensips",
                "sipp",
            },
        )
        self.assertEqual(
            manifest["configuration"]["effective_gate_config_keys"],
            sorted(self.fixture_gate_config("full", require_external=True)),
        )
        self.assertEqual(
            manifest["configuration"]["effective_gate_config"][
                "rvoip_require_api_tools"
            ],
            "1",
        )
        self.assertEqual(len(manifest["build"]["executables"]), 1)
        self.assertEqual(
            manifest["build"]["executables"][0]["sha256"],
            digest(b"fixture executable"),
        )
        copied = self.base / "copied-report"
        shutil.copytree(self.report, copied)
        subprocess.run(
            [
                "python3",
                str(copied / "inputs/attestation-verifier.py"),
                "verify",
                "--report-root",
                str(copied),
                "--require-pass",
            ],
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        shutil.rmtree(self.base / "report")
        attestation.verify_report(copied, require_pass=True)

    def test_v2_directly_binds_structured_reporting_inputs(self):
        self.create(require_external=True, schema_version=2)
        manifest = attestation.verify_report(self.report)
        self.assertEqual(manifest["schema"], attestation.SCHEMA_V2)
        structured = manifest["structured_reporting"]
        self.assertEqual(structured["binding_mode"], "native-v2-input")
        self.assertEqual(
            structured["gate_results"]["required_count"],
            len(manifest["gates"]),
        )
        path = self.report / "gate-results.json"
        payload = json.loads(path.read_text(encoding="utf-8"))
        payload["records"][0]["name"] = "tampered"
        path.write_text(
            json.dumps(payload, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        with self.assertRaises(attestation.AttestationError):
            attestation.verify_report(self.report)

    def test_sanitized_docker_peer_snapshot_is_attested(self):
        docker_dir = self.environment / "docker-after-freeswitch"
        docker_dir.mkdir()
        configuration = {
            "network_mode": "freeswitch-local",
            "published_ports": {
                "5060/udp": [{"host_ip": "0.0.0.0", "host_port": "5060"}]
            },
            "exposed_ports": ["5060/udp"],
            "restart_policy": {"name": "unless-stopped"},
        }
        snapshot_path = docker_dir / "rvoip-freeswitch-peer.json"
        snapshot_path.write_text(
            json.dumps(
                {
                    "schema": attestation.DOCKER_PEER_SCHEMA,
                    "product": "freeswitch",
                    "container": {"name": "/rvoip-freeswitch"},
                    "image": {
                        "id": "sha256:" + digest(b"freeswitch fixture"),
                        "reference": "rvoip-freeswitch:fixture",
                    },
                    "configuration": configuration,
                    "state": {"status": "running", "running": True},
                    "network": {"networks": {}},
                },
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )

        self.create()
        manifest = attestation.verify_report(self.report, require_pass=True)
        peer = next(
            item
            for item in manifest["peers"]
            if item["product"] == "freeswitch" and item["image_digest"] is not None
        )
        self.assertEqual(peer["version"], "rvoip-freeswitch:fixture")
        self.assertEqual(
            peer["image_digest"], "sha256:" + digest(b"freeswitch fixture")
        )
        self.assertEqual(
            peer["config_sha256"], attestation.canonical_json_sha256(configuration)
        )
        self.assertEqual(
            peer["evidence_paths"],
            ["environment/docker-after-freeswitch/rvoip-freeswitch-peer.json"],
        )

    def test_raw_docker_inspect_evidence_is_rejected(self):
        docker_dir = self.environment / "docker-before"
        docker_dir.mkdir()
        (docker_dir / "rvoip-freeswitch-inspect.json").write_text(
            json.dumps(
                [
                    {
                        "Image": "sha256:" + digest(b"legacy image"),
                        "Config": {
                            "Image": "legacy:unsafe",
                            "Env": ["TOKEN=must-not-be-attested"],
                        },
                    }
                ]
            )
            + "\n",
            encoding="utf-8",
        )

        with self.assertRaisesRegex(
            attestation.AttestationError, "raw Docker inspect evidence is forbidden"
        ):
            attestation.peers_block(self.report, {})

    def test_missing_artifact_is_rejected(self):
        self.create()
        self.log.unlink()
        with self.assertRaisesRegex(attestation.AttestationError, "missing"):
            attestation.verify_report(self.report)

    def test_changed_source_evidence_is_rejected(self):
        self.create()
        path = self.environment / "source-at-beta-start.json"
        path.write_text("{}\n", encoding="utf-8")
        with self.assertRaisesRegex(attestation.AttestationError, "artifact changed"):
            attestation.verify_report(self.report)

    def test_changed_yaml_is_rejected(self):
        self.create()
        (self.report / "inputs/state-machine-yaml.yaml").write_text(
            "version: changed\n", encoding="utf-8"
        )
        with self.assertRaisesRegex(attestation.AttestationError, "artifact changed"):
            attestation.verify_report(self.report)

    def test_configured_selected_yaml_source_and_hash_are_attested(self):
        self.state_yaml = self.base / "tenant-state-table.yaml"
        selected = b'version: "2.0"\nstates: [Idle]\ntransitions: []\n'
        self.state_yaml.write_bytes(selected)
        self.create(state_table_source="configured-path")
        manifest = attestation.verify_report(self.report)

        self.assertEqual(manifest["state_table"]["selected_source"], "configured-path")
        self.assertIsNone(manifest["state_table"]["fallback_reason"])
        self.assertEqual(
            manifest["state_table"]["selected_yaml_source_name"],
            "tenant-state-table.yaml",
        )
        self.assertEqual(
            manifest["state_table"]["selected_yaml_sha256"], digest(selected)
        )

    def test_configured_fallback_requires_bounded_reason(self):
        with self.assertRaisesRegex(
            attestation.AttestationError, "bounded fallback reason"
        ):
            self.create(state_table_source="configured-path-fallback")

        self.create(
            state_table_source="configured-path-fallback",
            state_table_fallback_reason="validation-failed",
        )
        manifest = attestation.verify_report(self.report)
        self.assertEqual(
            manifest["state_table"]["fallback_reason"], "validation-failed"
        )

    def test_non_fallback_source_rejects_fallback_reason(self):
        with self.assertRaisesRegex(attestation.AttestationError, "only valid"):
            self.create(
                state_table_source="configured-path",
                state_table_fallback_reason="read-failed",
            )

    def test_selected_yaml_hash_must_match_copied_input(self):
        self.create()

        def replace_selected_hash(value):
            value["state_table"]["selected_yaml_sha256"] = digest(b"different YAML")

        self.rewrite_attestation(replace_selected_hash)
        with self.assertRaisesRegex(attestation.AttestationError, "disagrees"):
            attestation.verify_report(self.report)

    def test_selected_yaml_change_after_gate_hash_is_rejected(self):
        expected = digest(self.state_yaml.read_bytes())
        self.state_yaml.write_text(
            "version: changed-before-attestation\n", encoding="utf-8"
        )
        original = digest(self.state_yaml.read_bytes())
        self.assertNotEqual(expected, original)

        with self.assertRaisesRegex(attestation.AttestationError, "changed after"):
            args = argparse.Namespace(
                report_root=str(self.report),
                source_start="environment/source-at-beta-start.json",
                source_end="environment/source-at-beta-end.json",
                cargo_metadata="environment/cargo-metadata.json",
                mode="local",
                run_id="20260720T000000Z",
                started_at="2026-07-20T00:00:00Z",
                ended_at="2026-07-20T00:01:00Z",
                features="",
                input=[
                    f"state-machine-yaml={self.state_yaml}",
                    f"performance-recipe={self.recipe_yaml}",
                    f"burst-scenarios={self.burst_yaml}",
                    f"attestation-verifier={SCRIPT_DIR / 'beta_attestation.py'}",
                ],
                state_table_source="embedded-default",
                state_table_fallback_reason=None,
                state_table_sha256=expected,
                failures=0,
                skips=0,
                overall="PASS",
            )
            attestation.create_attestation(args)

    def test_replaced_result_json_is_rejected(self):
        self.create()
        (self.report / "perf-results/result.json").write_text("{}\n", encoding="utf-8")
        with self.assertRaisesRegex(attestation.AttestationError, "artifact changed"):
            attestation.verify_report(self.report)

    def test_performance_recipe_and_burst_inputs_are_required(self):
        for name in ("performance-recipe", "burst-scenarios"):
            with self.subTest(name=name):
                with self.assertRaisesRegex(
                    attestation.AttestationError, "required attestation inputs"
                ):
                    self.create(omit_input=name)

    def test_full_mode_requires_reviewed_performance_baseline_input(self):
        with self.assertRaisesRegex(
            attestation.AttestationError, "required attestation inputs"
        ):
            self.create(omit_input="performance-regression-baseline")

    def test_packaged_performance_baseline_files_are_hash_verified(self):
        self.install_performance_baseline_fixture()
        result = (
            self.report
            / "perf-regression-baseline/perf-results/"
            "perf_call_setup_cps_pbx-media-server/2000.json"
        )
        result.write_text('{"status":"mutated"}\n', encoding="utf-8")
        with self.assertRaisesRegex(
            attestation.AttestationError,
            "performance regression baseline (byte count|hash) differs",
        ):
            self.create()

    def test_full_records_and_requires_api_tool_configuration(self):
        self.install_canonical_2k_fixture()
        self.create(
            require_external=True,
            config_overrides={"rvoip_require_api_tools": "0"},
        )
        manifest = attestation.verify_report(self.report)
        self.assertEqual(
            manifest["configuration"]["effective_gate_config"][
                "rvoip_require_api_tools"
            ],
            "0",
        )
        self.assertIn(
            "full release evidence did not require pinned public API tools",
            manifest["pointers"]["ineligibility_reasons"],
        )

    def test_missing_required_mode_gate_is_recorded_and_ineligible(self):
        self.create(
            mode="security",
            omit_gate="parser fuzz smoke (sip_message)",
        )
        manifest = attestation.verify_report(self.report)
        self.assertEqual(
            manifest["gate_inventory"]["missing"],
            ["parser fuzz smoke (sip_message)"],
        )
        self.assertFalse(manifest["pointers"]["mode_specific_eligible"])

    def test_duplicate_gate_names_are_rejected(self):
        self.write_summary("PASS", failures=0, skips=0, mode="local")
        summary = self.report / "summary.md"
        text = summary.read_text(encoding="utf-8")
        first_row = next(
            line for line in text.splitlines() if line.startswith("| PASS |")
        )
        summary.write_text(
            text.replace("\n\n## Result", f"\n{first_row}\n\n## Result"),
            encoding="utf-8",
        )
        with self.assertRaisesRegex(attestation.AttestationError, "duplicate gate"):
            attestation.parse_gates(self.report.resolve())

    def test_full_minimum_soak_configuration_is_fail_closed(self):
        self.install_canonical_2k_fixture()
        self.create(
            require_external=True,
            config_overrides={"rvoip_perf_soak_duration_secs": "3599"},
        )
        manifest = attestation.verify_report(self.report)
        self.assertTrue(
            any(
                "rvoip_perf_soak_duration_secs >= 3600" in reason
                for reason in manifest["pointers"]["ineligibility_reasons"]
            )
        )

    def test_result_effective_settings_must_match_gate_configuration(self):
        path = self.report / "perf-results/perf_soak_caller.json"
        result = json.loads(path.read_text(encoding="utf-8"))
        result["results"]["duration_secs"] = 120
        path.write_text(json.dumps(result) + "\n", encoding="utf-8")
        self.install_canonical_2k_fixture()
        self.create(require_external=True)
        manifest = attestation.verify_report(self.report)
        reconciliation = next(
            item
            for item in manifest["configuration"][
                "performance_result_reconciliation"
            ]
            if item["evidence_path"] == "perf-results/perf_soak_caller.json"
        )
        self.assertFalse(reconciliation["matches"])
        self.assertTrue(
            any(
                reason.startswith(
                    "performance result effective settings disagree with the gate"
                )
                for reason in manifest["pointers"]["ineligibility_reasons"]
            )
        )

    def test_run_duration_is_bound_to_timestamps(self):
        self.create()
        self.rewrite_attestation(lambda value: value["run"].update(duration_seconds=59))
        with self.assertRaisesRegex(attestation.AttestationError, "run duration"):
            attestation.verify_report(self.report)

    def test_features_are_bound_to_effective_gate_configuration(self):
        self.create()
        self.rewrite_attestation(
            lambda value: value["build"].update(features=["different-feature"])
        )
        with self.assertRaisesRegex(attestation.AttestationError, "features disagree"):
            attestation.verify_report(self.report)

    def test_combined_effective_redacted_config_hash_is_required(self):
        self.create()
        self.rewrite_attestation(
            lambda value: value["configuration"].update(
                effective_redacted_configuration_sha256=digest(b"wrong")
            )
        )
        with self.assertRaisesRegex(attestation.AttestationError, "combined effective"):
            attestation.verify_report(self.report)

    def test_gate_rows_cannot_be_omitted_from_rewritten_attestation(self):
        self.create()

        def remove_gate(value):
            value["gates"] = []
            value["result"].update(
                failed_gates=0,
                skipped_gates=0,
                required_skips_counted_as_failures=0,
            )

        self.rewrite_attestation(remove_gate)
        with self.assertRaisesRegex(attestation.AttestationError, "gate evidence"):
            attestation.verify_report(self.report)

    def test_result_json_cannot_be_omitted_with_an_absence_claim(self):
        self.create()

        def remove_results(value):
            value["results"]["json"] = []
            value["results"]["json_evidence"] = {
                "status": "absent",
                "count": 0,
                "absent_reason": "claimed absent",
            }
            value["results"]["performance_json_evidence"] = {
                "status": "absent",
                "count": 0,
                "absent_reason": "claimed absent",
            }
            value["pointers"] = attestation.pointer_block(
                value["run"]["mode"],
                value["source"]["clean"],
                value["source"]["unchanged"],
                value["result"]["failures"],
                value["result"]["skips"],
                value["build"]["executables"],
                value["peers"],
                value["results"],
                attestation.effective_gate_config(self.report),
                value["artifacts"]["files"],
                value["gate_inventory"],
                value["performance_regression_baseline"],
                value["configuration"]["performance_result_reconciliation"],
            )
            value["qualification"] = attestation.qualification_block(
                value["run"]["mode"], value["pointers"]
            )

        self.rewrite_attestation(remove_results)
        with self.assertRaisesRegex(attestation.AttestationError, "result evidence"):
            attestation.verify_report(self.report)

    def test_peer_without_version_or_digest_is_rejected(self):
        self.create()

        def remove_identity(value):
            value["peers"][0]["version"] = None
            value["peers"][0]["image_digest"] = None

        self.rewrite_attestation(remove_identity)
        with self.assertRaisesRegex(
            attestation.AttestationError, "peer version/image digest"
        ):
            attestation.verify_report(self.report)

    def test_missing_real_proxy_peer_is_not_release_eligible(self):
        summary_path = self.report / "proxy-interop/summary.json"
        summary = json.loads(summary_path.read_text(encoding="utf-8"))
        summary["rows"] = [
            row for row in summary["rows"] if row["peer"] != "opensips"
        ]
        summary_path.write_text(
            json.dumps(summary, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        self.create(require_external=True)
        manifest = attestation.verify_report(self.report)
        self.assertFalse(manifest["qualification"]["release_candidate"])
        self.assertTrue(
            any(
                "missing opensips" in reason
                for reason in manifest["qualification"]["ineligibility_reasons"]
            )
        )

    def test_uncaptured_optional_mode_evidence_is_explicitly_absent(self):
        shutil.rmtree(self.report / "strict-ua")
        shutil.rmtree(self.report / "perf-results")
        shutil.rmtree(self.report / "sipp")
        shutil.rmtree(self.report / "proxy-interop")
        shutil.rmtree(self.environment / "local-pbx")
        self.create(mode="local", include_executable=False)
        manifest = attestation.verify_report(self.report)
        self.assertEqual(manifest["build"]["executable_evidence"]["status"], "absent")
        self.assertTrue(manifest["build"]["executable_evidence"]["absent_reason"])
        self.assertEqual(manifest["peer_evidence"]["status"], "absent")
        self.assertTrue(manifest["peer_evidence"]["absent_reason"])
        self.assertEqual(manifest["results"]["json_evidence"]["status"], "absent")
        self.assertTrue(manifest["results"]["json_evidence"]["absent_reason"])
        self.assertEqual(
            manifest["results"]["performance_json_evidence"]["status"], "absent"
        )
        self.assertEqual(manifest["results"]["canonical_2k"]["status"], "absent")

    def test_dirty_tree_is_integrity_valid_but_not_release_valid(self):
        self.create(dirty=True)
        manifest = attestation.verify_report(self.report)
        self.assertFalse(manifest["source"]["clean"])
        self.assertEqual(manifest["qualification"]["status"], "NON-RC")
        self.assertFalse(manifest["qualification"]["release_candidate"])
        with self.assertRaisesRegex(attestation.AttestationError, "clean source"):
            attestation.verify_report(self.report, require_clean=True)
        with self.assertRaisesRegex(attestation.AttestationError, "mode-complete"):
            attestation.verify_report(self.report, require_mode_eligible=True)

    def test_changed_source_is_not_unchanged_release_evidence(self):
        self.create(changed=True)
        manifest = attestation.verify_report(self.report)
        self.assertFalse(manifest["source"]["unchanged"])
        with self.assertRaisesRegex(attestation.AttestationError, "unchanged source"):
            attestation.verify_report(self.report, require_unchanged_source=True)

    def test_skips_are_recorded_and_can_be_rejected(self):
        self.create(skips=1)
        manifest = attestation.verify_report(self.report, require_pass=True)
        self.assertEqual(manifest["result"]["skips"], 1)
        with self.assertRaisesRegex(attestation.AttestationError, "zero skipped"):
            attestation.verify_report(self.report, require_no_skips=True)

    def test_required_external_skip_is_reconciled_as_a_failure(self):
        self.create(failures=1, skips=1, require_external=True)
        manifest = attestation.verify_report(self.report)
        self.assertEqual(manifest["result"]["failed_gates"], 0)
        self.assertEqual(manifest["result"]["skipped_gates"], 1)
        self.assertEqual(manifest["result"]["required_skips_counted_as_failures"], 1)
        self.assertEqual(manifest["result"]["overall"], "FAIL")

    def test_failed_gate_is_attested_but_not_pass_evidence(self):
        self.create(failures=1)
        manifest = attestation.verify_report(self.report)
        self.assertEqual(manifest["result"]["overall"], "FAIL")
        with self.assertRaisesRegex(attestation.AttestationError, "overall PASS"):
            attestation.verify_report(self.report, require_pass=True)

    def test_schema_change_is_rejected_even_with_new_checksum(self):
        self.create()
        self.rewrite_attestation(lambda value: value.update(schema="future-schema"))
        with self.assertRaisesRegex(attestation.AttestationError, "unsupported"):
            attestation.verify_report(self.report)

    def test_added_unattested_file_is_rejected(self):
        self.create()
        (self.report / "late.txt").write_text("late evidence\n", encoding="utf-8")
        with self.assertRaisesRegex(
            attestation.AttestationError, "unattested artifacts"
        ):
            attestation.verify_report(self.report)

    def move_to_indexed_report(self):
        index = self.base / "beta-report"
        indexed = index / "20260720T000000Z"
        indexed.parent.mkdir()
        self.report.rename(indexed)
        self.report = indexed
        self.environment = indexed / "environment"
        self.log = indexed / "perf_fixture_gate.log"
        return index

    def test_clean_full_without_canonical_evidence_is_not_release_pointer_eligible(
        self,
    ):
        self.create()
        index = self.move_to_indexed_report()
        updated = attestation.update_latest_pointers(self.report, index)
        self.assertEqual([path.name for path in updated], ["latest.txt"])
        self.assertFalse((index / "latest-full-clean.txt").exists())
        manifest = attestation.verify_report(self.report)
        self.assertIn(
            "full release evidence lacks three canonical 2K runs",
            manifest["pointers"]["ineligibility_reasons"],
        )

    def test_clean_complete_full_evidence_updates_clean_full_pointer(self):
        self.install_canonical_2k_fixture()
        self.create(require_external=True)
        manifest = attestation.verify_report(self.report)
        self.assertEqual(manifest["results"]["canonical_2k"]["status"], "captured")
        self.assertEqual(len(manifest["results"]["canonical_2k"]["runs"]), 3)
        self.assertEqual(
            manifest["results"]["canonical_2k"]["reviewed_baseline"]["sha256"],
            attestation.CANONICAL_2K_BASELINE_SHA256,
        )
        baseline_path = manifest["results"]["canonical_2k"]["reviewed_baseline"][
            "path"
        ]
        baseline_inventory = next(
            item
            for item in manifest["artifacts"]["files"]
            if item["path"] == baseline_path
        )
        self.assertEqual(baseline_inventory["kind"], "input")
        self.assertEqual(
            baseline_inventory["sha256"],
            attestation.CANONICAL_2K_BASELINE_SHA256,
        )
        self.assertEqual(
            manifest["performance_regression_baseline"]["status"], "captured"
        )
        packaged_baseline_paths = {
            item["path"]
            for item in manifest["performance_regression_baseline"]["files"]
        } | {manifest["performance_regression_baseline"]["manifest_path"]}
        self.assertTrue(
            all(
                item["kind"] == "input"
                for item in manifest["artifacts"]["files"]
                if item["path"] in packaged_baseline_paths
            )
        )
        self.assertTrue(manifest["gate_inventory"]["complete"])
        self.assertTrue(
            all(
                item["matches"]
                for item in manifest["configuration"][
                    "performance_result_reconciliation"
                ]
            )
        )
        self.assertTrue(manifest["pointers"]["mode_specific_eligible"])
        self.assertEqual(
            manifest["qualification"]["status"], "RELEASE-CANDIDATE"
        )
        self.assertTrue(manifest["qualification"]["release_candidate"])
        attestation.verify_report(self.report, require_mode_eligible=True)
        index = self.move_to_indexed_report()
        updated = attestation.update_latest_pointers(self.report, index)
        self.assertEqual(
            [path.name for path in updated], ["latest.txt", "latest-full-clean.txt"]
        )

    def test_full_pass_without_fail_closed_external_gates_is_non_rc(self):
        self.install_canonical_2k_fixture()
        self.create(require_external=False)
        manifest = attestation.verify_report(self.report, require_pass=True)
        self.assertFalse(manifest["pointers"]["mode_specific_eligible"])
        self.assertIn(
            "full release evidence must fail closed on unavailable external gates",
            manifest["pointers"]["ineligibility_reasons"],
        )

    def test_full_pass_without_pbx_report_artifacts_is_non_rc(self):
        self.install_canonical_2k_fixture()
        (self.report / "pbx/summary.md").unlink()
        self.create(require_external=True)
        manifest = attestation.verify_report(self.report, require_pass=True)
        self.assertFalse(manifest["pointers"]["mode_specific_eligible"])
        self.assertTrue(
            any(
                reason.startswith("PBX report evidence is incomplete")
                for reason in manifest["pointers"]["ineligibility_reasons"]
            )
        )

    def test_full_pass_without_proxy_runtime_state_evidence_is_non_rc(self):
        self.install_canonical_2k_fixture()
        (self.report / "proxy-interop/runtime-state-check.json").unlink()
        self.create(require_external=True)
        manifest = attestation.verify_report(self.report, require_pass=True)
        self.assertFalse(manifest["pointers"]["mode_specific_eligible"])
        self.assertTrue(
            any(
                reason.startswith("interop report evidence is incomplete")
                and "proxy-interop/runtime-state-check.json" in reason
                for reason in manifest["pointers"]["ineligibility_reasons"]
            )
        )

    def test_full_pass_without_split_soak_json_is_non_rc(self):
        self.install_canonical_2k_fixture()
        (self.report / "perf-results/perf_soak_receiver.json").unlink()
        self.create(require_external=True)
        manifest = attestation.verify_report(self.report, require_pass=True)
        self.assertFalse(manifest["pointers"]["mode_specific_eligible"])
        self.assertTrue(
            any(
                reason.startswith("standard performance/soak result evidence is incomplete")
                for reason in manifest["pointers"]["ineligibility_reasons"]
            )
        )

    def test_canonical_run_tree_must_match_index(self):
        self.install_canonical_2k_fixture()
        (self.report / "canonical-2k/run-2/evidence.txt").write_text(
            "changed after index\n", encoding="utf-8"
        )
        with self.assertRaisesRegex(attestation.AttestationError, "tree hash"):
            self.create()

    def test_canonical_reviewed_baseline_must_match_approved_hash(self):
        self.install_canonical_2k_fixture()
        baseline = self.report / "canonical-2k" / attestation.CANONICAL_2K_BASELINE_PACKAGED_PATH
        baseline.write_bytes(baseline.read_bytes() + b"mutated\n")
        with self.assertRaisesRegex(attestation.AttestationError, "baseline artifact hash"):
            self.create()

    def test_every_mode_has_a_distinct_pointer_and_perf_updates_its_pointer(self):
        self.assertEqual(
            attestation.MODE_POINTERS,
            {
                "local": "latest-local.txt",
                "interop": "latest-interop.txt",
                "security": "latest-security.txt",
                "perf": "latest-perf.txt",
                "full": "latest-full-clean.txt",
            },
        )
        self.create(mode="perf")
        index = self.move_to_indexed_report()
        updated = attestation.update_latest_pointers(self.report, index)
        self.assertEqual(
            [path.name for path in updated], ["latest.txt", "latest-perf.txt"]
        )

    def test_dirty_full_pass_updates_only_informational_pointer(self):
        self.create(dirty=True)
        index = self.move_to_indexed_report()
        updated = attestation.update_latest_pointers(self.report, index)
        self.assertEqual([path.name for path in updated], ["latest.txt"])
        self.assertFalse((index / "latest-full-clean.txt").exists())

    def test_failed_run_does_not_replace_mode_pointer(self):
        self.create(failures=1)
        index = self.move_to_indexed_report()
        (index / "latest-full-clean.txt").write_text(
            "20260719T000000Z\n", encoding="ascii"
        )
        updated = attestation.update_latest_pointers(self.report, index)
        self.assertEqual([path.name for path in updated], ["latest.txt"])
        self.assertEqual(
            (index / "latest-full-clean.txt").read_text(encoding="ascii"),
            "20260719T000000Z\n",
        )

    def test_skipped_run_does_not_replace_mode_pointer(self):
        self.create(skips=1)
        index = self.move_to_indexed_report()
        updated = attestation.update_latest_pointers(self.report, index)
        self.assertEqual([path.name for path in updated], ["latest.txt"])
        self.assertFalse((index / "latest-full-clean.txt").exists())

    def test_active_release_metadata_agrees_with_workspace_version(self):
        crate_manifest = (WORKSPACE_ROOT / "crates/sip/rvoip-sip/Cargo.toml").read_text(
            encoding="utf-8"
        )
        checklist = (
            WORKSPACE_ROOT / "crates/sip/rvoip-sip/docs/BETA_RELEASE_CHECKLIST.md"
        ).read_text(encoding="utf-8")
        release_notes = (
            WORKSPACE_ROOT / "crates/sip/rvoip-sip/docs/RELEASE_NOTES_NEXT.md"
        ).read_text(encoding="utf-8")
        self.assertRegex(
            crate_manifest,
            r"(?m)^version\.workspace\s*=\s*true(?:\s*(?:#.*)?)?$",
        )
        marker = f"Current candidate and runtime crate version: `{self.workspace_version}`"
        self.assertIn(marker, checklist)
        self.assertIn(
            f"{self.workspace_version} Release Candidate Notes", release_notes
        )
        self.assertIn(
            f"**Unified `{self.workspace_version}` release train.**",
            (WORKSPACE_ROOT / "README.md").read_text(encoding="utf-8"),
        )


if __name__ == "__main__":
    unittest.main()
