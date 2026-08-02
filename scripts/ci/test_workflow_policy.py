from __future__ import annotations

import json
from pathlib import Path
import re
import tomllib
import unittest


ROOT = Path(__file__).resolve().parents[2]
SHA_ACTION = re.compile(r"uses:\s*[^\s#]+@([0-9a-f]{40})(?:\s|#|$)")


class WorkflowPolicyTests(unittest.TestCase):
    def test_actions_are_immutable_and_prs_never_use_privileged_events(self) -> None:
        workflows = sorted((ROOT / ".github/workflows").glob("*.yml"))
        self.assertTrue(workflows)
        for workflow in workflows:
            text = workflow.read_text()
            with self.subTest(workflow=workflow.name):
                self.assertNotIn("pull_request_target", text)
                uses = [line for line in text.splitlines() if "uses:" in line]
                self.assertTrue(all(SHA_ACTION.search(line) for line in uses))

    def test_pr_gate_has_no_secret_or_write_permission_surface(self) -> None:
        text = (ROOT / ".github/workflows/pr-gate.yml").read_text()
        self.assertNotIn("secrets.", text)
        self.assertNotIn("contents: write", text)
        self.assertNotIn("pull_request_target", text)

    def test_pr_gate_reuses_each_shard_build_for_tests_and_lint(self) -> None:
        text = (ROOT / ".github/workflows/pr-gate.yml").read_text()
        self.assertIn("run_checks.py shard", text)
        self.assertIn("shard-${{ matrix.shard_id }}-all", text)
        self.assertNotIn("run_checks.py shard-${{ matrix.check }}", text)
        self.assertIn("run_checks.py sip-core", text)
        self.assertIn("run_checks.py sip-clippy", text)
        self.assertIn("run_checks.py sip-fixtures", text)
        self.assertIn("sip-integration", text)

    def test_every_sip_process_fixture_is_prebuilt_by_the_dedicated_lane(self) -> None:
        policy = json.loads((ROOT / "scripts/ci/policy.json").read_text())
        mapping = policy["pr_sip_fixture_examples"]
        test_root = ROOT / "crates/sip/rvoip-sip/tests"
        fixture_targets = {
            path.stem
            for path in test_root.glob("*.rs")
            if "build_examples(" in path.read_text()
        }
        self.assertEqual(set(mapping), fixture_targets)

        manifest = tomllib.loads(
            (ROOT / "crates/sip/rvoip-sip/Cargo.toml").read_text()
        )
        declared_examples = {item["name"] for item in manifest.get("example", [])}
        mapped_examples = {
            example for examples in mapping.values() for example in examples
        }
        self.assertEqual(mapped_examples - declared_examples, set())

    def test_main_aggregates_one_receipt_per_combined_shard(self) -> None:
        text = (ROOT / ".github/workflows/main-ci.yml").read_text()
        self.assertIn("--shard-layout shards", text)
        self.assertIn("--job-mode combined", text)
        self.assertIn("--deferred-sip-mode separate", text)
        self.assertIn("Full SIP ${{ matrix.id }}", text)
        self.assertIn("run_checks.py sip-core", text)
        self.assertIn("run_checks.py sip-clippy", text)
        self.assertIn("run_checks.py sip-fixtures", text)
        self.assertIn("run_checks.py sip-integration", text)
        self.assertIn("--job 'sip-tests=${{ needs.sip-tests.result }}'", text)

    def test_main_release_tooling_installs_its_fuzz_toolchain(self) -> None:
        text = (ROOT / ".github/workflows/main-ci.yml").read_text()
        specialty = text.split("\n  specialty:\n", maxsplit=1)[1].split(
            "\n  main-gate:\n", maxsplit=1
        )[0]
        self.assertIn("Install nightly for release fuzz validation", specialty)
        self.assertIn("cargo-fuzz@0.13.2", specialty)
        self.assertGreaterEqual(specialty.count("matrix.gate == 'release-tooling'"), 2)
        self.assertIn("max-parallel: 6", specialty)

    def test_parallel_gcp_workspace_is_ephemeral_and_fail_closed(self) -> None:
        workflow = (ROOT / ".github/workflows/gcp-qualification-pilot.yml").read_text()
        startup = (ROOT / "infra/release-runners/gcp-pilot-startup.sh").read_text()

        self.assertIn("workspace-parallel", workflow)
        self.assertIn("Verify parallel GCP capacity", workflow)
        self.assertIn("Require every quota before creating workers", workflow)
        self.assertIn("CPUS_ALL_REGIONS", workflow)
        self.assertIn("SSD_TOTAL_GB", workflow)
        self.assertIn("DISKS_TOTAL_GB", workflow)
        self.assertIn("--boot-disk-auto-delete", workflow)
        self.assertIn("Delete shard worker and attached disk", workflow)
        self.assertIn('gcloud compute instances describe "$WORKER"', workflow)
        self.assertIn("Sweep workers left by interrupted shard controllers", workflow)
        self.assertIn("parallel GCP workers remain after cleanup", workflow)
        self.assertIn("if: always() && steps.worker.outputs.name != ''", workflow)
        self.assertIn("expected_shards", workflow)
        self.assertIn("expected_packages", workflow)
        self.assertIn("expected_sip_targets", workflow)
        self.assertIn("a SIP integration target appears in more than one shard", workflow)
        self.assertIn("sip_test_partitions.py", workflow)
        self.assertIn('"id": "sip-core"', workflow)
        self.assertIn('f"sip-integration-{index}"', workflow)
        self.assertIn('"disk_size_gb": 200', workflow)
        self.assertIn("publishing_attempted == false", workflow)
        self.assertIn('"publishing_attempted": False', startup)
        for profile in (
            "workspace-policy",
            "workspace-shard-test",
            "workspace-shard-clippy",
            "workspace-doctest",
            "workspace-security-timing",
            "workspace-sip-core",
            "workspace-sip-integration",
        ):
            self.assertIn(profile, startup)

    def test_release_workers_install_interop_tools_only_for_interop(self) -> None:
        workflow = (ROOT / ".github/workflows/release-qualify.yml").read_text()
        startup = (ROOT / "infra/release-runners/gcp-release-startup.sh").read_text()
        fanout = (ROOT / "scripts/release/gcp_fanout.py").read_text()

        self.assertIn('resource_class="$(jq -r .resource_class', workflow)
        self.assertIn("rvoip-resource-class=${resource_class}", workflow)
        self.assertIn('RESOURCE_CLASS="$(metadata rvoip-resource-class)"', startup)
        self.assertIn('if [[ "$RESOURCE_CLASS" == "gcp-interop" ]]', startup)
        self.assertIn('"gcp-interop": "n2-standard-4"', fanout)
        self.assertIn('"gcp-performance": "n2-standard-8"', fanout)
        self.assertIn('"gcp-performance-soak": "n2-standard-4"', fanout)
        self.assertIn("sip-tester", startup)
        self.assertIn("tshark", startup)
        self.assertIn("docker-compose-v2", startup)
        self.assertIn('",$GATES," == *",perf.sipp-parity,"*', startup)
        self.assertIn('",$GATES," == *",preflight.performance-01,"*', startup)
        self.assertGreaterEqual(startup.count("command -v sipp >/dev/null"), 2)
        self.assertIn("command -v tshark >/dev/null", startup)
        self.assertIn("docker compose version >/dev/null", startup)
        self.assertIn("ulimit -n 262144", startup)
        self.assertIn('test "$(ulimit -n)" -ge 262144', startup)
        self.assertIn("-name '*.jsonl'", startup)
        self.assertNotRegex(startup, r"apt-get install[^\n]*\bsipp\b")

    def test_release_infrastructure_preflight_is_full_shape_and_non_publishing(self) -> None:
        workflow = (ROOT / ".github/workflows/release-qualify.yml").read_text()
        startup = (ROOT / "infra/release-runners/gcp-release-startup.sh").read_text()
        probe = (
            ROOT / "infra/release-runners/release-infrastructure-preflight.sh"
        ).read_text()

        self.assertIn("remote-preflight", workflow)
        self.assertIn(
            'test "$FIRST_CANDIDATE" = true || test "$PROFILE" = remote-preflight',
            workflow,
        )
        self.assertIn("Require candidate to belong to protected origin/main", workflow)
        self.assertIn("RVOIP_GCP_WORKLOAD_IDENTITY_PROVIDER", workflow)
        self.assertNotIn("RVOIP_GCP_PILOT_PROVIDER", workflow)
        self.assertIn('export RVOIP_RELEASE_RESOURCE_CLASS="$RESOURCE_CLASS"', startup)
        self.assertIn('export RVOIP_RELEASE_CANDIDATE="$CANDIDATE"', startup)
        self.assertIn('export RVOIP_RELEASE_GATES="$GATES"', startup)
        self.assertIn("expected 44 publishable workspace packages", probe)
        self.assertIn("for _ in range(4096)", probe)
        self.assertIn('test "$NOFILE_LIMIT" -ge 262144', probe)
        self.assertIn('test -z "${CARGO_REGISTRY_TOKEN:-}"', probe)
        self.assertIn('test -z "${CRATES_IO_TOKEN:-}"', probe)
        self.assertIn('"publishing_credentials_present": False', probe)

    def test_release_gcp_workers_do_not_consume_one_github_job_each(self) -> None:
        workflow = (ROOT / ".github/workflows/release-qualify.yml").read_text()
        controller = workflow.split("\n  gate-gcp:\n", maxsplit=1)[1].split(
            "\n  cleanup-gcp:\n", maxsplit=1
        )[0]

        self.assertNotIn("strategy:", controller)
        self.assertNotIn("matrix: ${{ fromJSON", controller)
        self.assertIn("Run all ephemeral GCP release shards", controller)
        self.assertIn("gcp_fanout.py prepare", controller)
        self.assertIn("Create every ephemeral release worker concurrently", controller)
        self.assertIn('pids+=("$!")', controller)
        self.assertIn("Wait for every immutable worker result", controller)
        self.assertIn("gcp_fanout.py verify", controller)
        self.assertIn("Delete all workers and attached disks", controller)
        self.assertIn("release-gate-shard-gcp-controller", controller)

    def test_release_pbx_lifecycle_uses_tracked_templates(self) -> None:
        lifecycle = (ROOT / "infra/release-runners/interop-lifecycle.sh").read_text()
        self.assertIn("infra/release-runners/pbx", lifecycle)
        self.assertNotIn("beta-report/", lifecycle)
        self.assertIn("wait_udp_port", lifecycle)
        self.assertIn("/proc/net/udp", lifecycle)
        self.assertNotIn("nc -z 127.0.0.1", lifecycle)
        self.assertIn("--name rvoip-freeswitch", lifecycle)
        self.assertIn("wait_udp_port rvoip-freeswitch 5062", lifecycle)
        self.assertNotIn("--name rvoip-release-freeswitch", lifecycle)
        self.assertNotIn("down rvoip-release-freeswitch", lifecycle)
        for path in (
            "infra/release-runners/pbx/asterisk/Dockerfile",
            "infra/release-runners/pbx/asterisk/config/pjsip.conf",
            "infra/release-runners/pbx/freeswitch/Dockerfile",
            "infra/release-runners/pbx/freeswitch/docker-entrypoint.sh",
        ):
            with self.subTest(path=path):
                self.assertTrue((ROOT / path).is_file())

    def test_linux_proxy_helpers_share_the_reviewed_host_network(self) -> None:
        common = (
            ROOT / "crates/sip/sip-proxy/tests/interop/scripts/common.sh"
        ).read_text()
        self.assertIn('elif [[ "$(uname -s)" == "Linux" ]]', common)
        self.assertIn('COMPOSE_FILE_ARGS+=(--file "$COLIMA_HOST_COMPOSE_FILE")', common)
        self.assertIn(
            "PROXY_INTEROP_NETWORK_TOPOLOGY=linux-native-host-network", common
        )

    def test_burst_runner_honors_remote_artifact_directory(self) -> None:
        runner = (
            ROOT / "crates/sip/rvoip-sip/scripts/perf_burst_matrix.sh"
        ).read_text()
        self.assertIn(
            'PERF_DIR="${RVOIP_PERF_RESULTS:-${WORKSPACE_ROOT}/target/perf-results}"',
            runner,
        )

    def test_burst_rss_gate_uses_post_retention_settled_window(self) -> None:
        for path in (
            "crates/sip/rvoip-sip/tests/perf/perf_burst_caller.rs",
            "crates/sip/rvoip-sip/tests/perf/perf_burst_receiver.rs",
        ):
            source = (ROOT / path).read_text()
            with self.subTest(path=path):
                final_retention = source.index("capture_endpoint_retention_sample(")
                settled_window = source.index("sample_settled_rss_window(", final_retention)
                rss_gate = source.index("rss_result_metrics(", settled_window)
                self.assertLess(final_retention, settled_window)
                self.assertLess(settled_window, rss_gate)
                self.assertIn("&settled_resources", source[rss_gate : rss_gate + 250])


if __name__ == "__main__":
    unittest.main()
