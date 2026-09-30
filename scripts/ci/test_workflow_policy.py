from __future__ import annotations

import json
from pathlib import Path
import re
import tomllib
import unittest

import run_checks


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

    def test_lockfile_changes_compile_amazon_connects_optional_aws_client(self) -> None:
        policy = json.loads((ROOT / "scripts/ci/policy.json").read_text())
        rules = {
            rule["gate"]: set(rule["patterns"])
            for rule in policy["specialty_rules"]
        }
        self.assertIn("amazon-connect-aws-control", rules)
        self.assertIn("Cargo.lock", rules["amazon-connect-aws-control"])
        self.assertIn("examples/Cargo.lock", rules["amazon-connect-aws-control"])

    def test_lockfile_changes_compile_the_optional_otel_exporter(self) -> None:
        policy = json.loads((ROOT / "scripts/ci/policy.json").read_text())
        rules = {
            rule["gate"]: set(rule["patterns"])
            for rule in policy["specialty_rules"]
        }
        self.assertIn("infra-otel", rules)
        self.assertIn("Cargo.lock", rules["infra-otel"])

    def test_optional_codecs_are_owned_by_a_gate_and_forced_on_main(self) -> None:
        policy = json.loads((ROOT / "scripts/ci/policy.json").read_text())
        rules = {
            rule["gate"]: set(rule["patterns"])
            for rule in policy["specialty_rules"]
        }
        # One gate per package. They carry identical patterns on purpose: any
        # change that would have selected the old combined gate still verifies
        # all four packages, it just does so concurrently instead of against
        # one shared 45-minute budget it could not fit inside.
        gates = sorted(run_checks.CODEC_FEATURE_GATES)
        main = (ROOT / ".github/workflows/main-ci.yml").read_text()
        for gate in gates:
            with self.subTest(gate=gate):
                self.assertIn(gate, rules)
                self.assertIn("crates/media/codec-core/**", rules[gate])
                # The optional codec backends -- libopus today -- are only
                # compiled under their feature, so a lockfile bump moves code
                # no shard builds.
                self.assertIn("Cargo.lock", rules[gate])
                # Path matching only fires when the codec crates themselves
                # change. A change anywhere else that breaks a feature-gated
                # path would go unnoticed until main, so main runs the gates
                # unconditionally.
                self.assertIn(f"--specialty-gate {gate}", main)
        # Identical patterns across the four, so splitting the job did not
        # narrow what any one change verifies.
        self.assertEqual(len({frozenset(rules[gate]) for gate in gates}), 1)

    def test_facade_feature_bundles_are_owned_by_a_gate_and_forced_on_main(self) -> None:
        policy = json.loads((ROOT / "scripts/ci/policy.json").read_text())
        rules = {
            rule["gate"]: set(rule["patterns"])
            for rule in policy["specialty_rules"]
        }
        gate = "facade-feature-bundles"
        self.assertIn(gate, rules)
        self.assertIn("crates/rvoip/Cargo.toml", rules[gate])
        self.assertIn("docs/FEATURE_BUNDLES.md", rules[gate])
        main = (ROOT / ".github/workflows/main-ci.yml").read_text()
        self.assertIn(f"--specialty-gate {gate}", main)

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

    def test_nextest_defers_exactly_the_catalogued_sip_process_fixtures(self) -> None:
        policy = json.loads((ROOT / "scripts/ci/policy.json").read_text())
        mapping = policy["pr_sip_fixture_examples"]
        config_text = (ROOT / ".config/nextest.toml").read_text()
        config = tomllib.loads(config_text)
        default_filter = config["profile"]["ci"]["default-filter"]
        deferred = set(re.findall(r"binary\(=([a-z0-9_]+)\)", default_filter))

        self.assertEqual(deferred, set(mapping))
        self.assertIn(
            "test(=adapters::session_event_handler::tests::"
            "malformed_inbound_sdes_update_returns_one_cached_488_without_state_mutation)",
            config_text,
        )
        self.assertIn('threads-required = "num-test-threads"', config_text)

        workflow = (ROOT / ".github/workflows/nextest-parity.yml").read_text()
        self.assertIn("target/nextest/ci/junit.xml", workflow)
        self.assertIn("if-no-files-found: error", workflow)

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
        self.assertIn("max-parallel: 3", specialty)

    def test_main_preserves_capacity_for_pr_and_release_feedback(self) -> None:
        text = (ROOT / ".github/workflows/main-ci.yml").read_text()
        crate_tests = text.split("\n  crate-tests:\n", maxsplit=1)[1].split(
            "\n  doctests:\n", maxsplit=1
        )[0]
        sip_tests = text.split("\n  sip-tests:\n", maxsplit=1)[1].split(
            "\n  specialty:\n", maxsplit=1
        )[0]
        specialty = text.split("\n  specialty:\n", maxsplit=1)[1].split(
            "\n  main-gate:\n", maxsplit=1
        )[0]

        self.assertIn("max-parallel: 3", crate_tests)
        self.assertIn("max-parallel: 4", sip_tests)
        self.assertIn("max-parallel: 3", specialty)

    def test_release_prepare_installs_native_validation_dependencies(self) -> None:
        text = (ROOT / ".github/workflows/release-prepare.yml").read_text()
        dependency_step = text.index("Install release validation dependencies")
        validation_step = text.index("Prepare all workspace versions")

        self.assertLess(dependency_step, validation_step)
        for package in (
            "libasound2-dev",
            "libopus-dev",
            "libssl-dev",
            "libvpx-dev",
            "protobuf-compiler",
            "pkg-config",
            "cmake",
        ):
            with self.subTest(package=package):
                self.assertIn(package, text[dependency_step:validation_step])

    def test_release_prepare_runs_evidence_helpers_before_commit(self) -> None:
        text = (ROOT / ".github/workflows/release-prepare.yml").read_text()
        prepare_step = text.index("Prepare all workspace versions")
        helper_test = text.index(
            "crates/sip/rvoip-sip/scripts/test_beta_attestation.py", prepare_step
        )
        commit_step = text.index("Commit release preparation")

        self.assertLess(prepare_step, helper_test)
        self.assertLess(helper_test, commit_step)

        policy = json.loads((ROOT / "scripts/ci/policy.json").read_text())
        release_tooling = next(
            rule
            for rule in policy["specialty_rules"]
            if rule["gate"] == "release-tooling"
        )
        for path in (
            "crates/sip/rvoip-sip/docs/RELEASE_NOTES_NEXT.md",
            "crates/sip/rvoip-sip/scripts/full_beta_release.sh",
        ):
            with self.subTest(path=path):
                self.assertIn(path, release_tooling["patterns"])

    def test_release_workflows_raise_rustup_download_retries(self) -> None:
        for filename in (
            "release-prepare.yml",
            "release-qualify.yml",
            "release-publish.yml",
        ):
            with self.subTest(workflow=filename):
                text = (ROOT / ".github/workflows" / filename).read_text()
                self.assertIn('RUSTUP_MAX_RETRIES: "10"', text)

    def test_release_all_features_paths_install_libvpx(self) -> None:
        workflows = (
            ("release-qualify.yml", "Install hosted-runner native dependencies", "Run gate shard"),
            ("release-publish.yml", "Install package build dependencies", "Establish exact local main"),
        )
        for filename, dependency_name, next_step_name in workflows:
            with self.subTest(workflow=filename):
                text = (ROOT / ".github/workflows" / filename).read_text()
                dependency_step = text.index(dependency_name)
                next_step = text.index(next_step_name, dependency_step)
                self.assertIn("libvpx-dev", text[dependency_step:next_step])

    def test_release_workers_install_interop_tools_only_for_interop(self) -> None:
        workflow = (ROOT / ".github/workflows/release-qualify.yml").read_text()
        startup = (ROOT / "infra/release-runners/aws-release-startup.sh").read_text()
        fanout = (ROOT / "scripts/release/aws_fanout.py").read_text()

        self.assertIn('resource_class="$(jq -r .resource_class', workflow)
        self.assertIn("python3 scripts/release/aws_fanout.py user-data", workflow)
        self.assertIn("source /etc/rvoip-release.env", startup)
        self.assertIn('"$RESOURCE_CLASS" == "ec2-interop"', startup)
        self.assertIn('"$RESOURCE_CLASS" == "ec2-proxy-interop"', startup)
        self.assertIn('"ec2-interop": "m5.4xlarge"', fanout)
        self.assertIn('"ec2-proxy-interop": "m5.large"', fanout)
        self.assertIn('"ec2-performance": "m5.4xlarge"', fanout)
        self.assertIn('"ec2-performance-soak": "m5.xlarge"', fanout)
        self.assertIn('"ec2-performance-soak-long": "m5.4xlarge"', fanout)
        self.assertIn("sip-tester", startup)
        self.assertIn("tshark", startup)
        self.assertIn("docker-compose-v2", startup)

    def test_release_workflow_is_ephemeral_and_fail_closed_on_aws(self) -> None:
        workflow = (ROOT / ".github/workflows/release-qualify.yml").read_text()

        for job in ("preflight-aws", "gate-aws", "cleanup-aws"):
            self.assertIn(job, workflow)
        for step in (
            "Require all compute and storage quota before creating workers",
            "Create performance workers once the bundle exists",
            "Wait for every immutable worker result",
            "Download and verify every shard evidence bundle",
            "Terminate all workers and root volumes",
            "Sweep workers left by interrupted controllers",
        ):
            self.assertIn(step, workflow)
        self.assertIn(
            "aws-actions/configure-aws-credentials@e1253824e5c10ff9df46874f81ed3ec929e19cfd",
            workflow,
        )
        self.assertIn(
            "role-to-assume: ${{ vars.RVOIP_AWS_PROVISIONER_ROLE_ARN }}", workflow
        )
        self.assertIn("--instance-initiated-shutdown-behavior terminate", workflow)
        self.assertIn("HttpTokens=required", workflow)
        self.assertIn("InstanceMetadataTags=enabled", workflow)
        self.assertIn("L-1216C47A", workflow)
        self.assertIn("DeleteOnTermination=true", workflow)
        self.assertIn("VolumeType=gp3", workflow)
        for line in workflow.splitlines():
            for forbidden in ("gcloud", "gs://", "google-github-actions"):
                self.assertNotIn(forbidden, line, line)

    def test_release_workers_use_a_verified_fail_open_s3_compiler_cache(self) -> None:
        workflow = (ROOT / ".github/workflows/release-qualify.yml").read_text()
        startup = (ROOT / "infra/release-runners/aws-release-startup.sh").read_text()

        self.assertIn("RVOIP_AWS_CACHE_BUCKET", workflow)
        self.assertIn('--env "RVOIP_CACHE_BUCKET=${CACHE_BUCKET}"', workflow)
        self.assertIn('CACHE_BUCKET="$RVOIP_CACHE_BUCKET"', startup)
        self.assertIn("SCCACHE_VERSION=0.15.0", startup)
        self.assertIn(
            "SCCACHE_SHA256=782d2b5dd7ae0a55ebe368ab258114d0928d019ac2d949ab85d5d02f3926709e",
            startup,
        )
        self.assertIn("--show-error --location", startup)
        self.assertIn("sha256sum --check --status", startup)
        self.assertIn("SCCACHE_MULTILEVEL_CHAIN=disk,s3", startup)
        self.assertIn('SCCACHE_BUCKET="$CACHE_BUCKET"', startup)
        self.assertIn('SCCACHE_REGION="$AWS_REGION"', startup)
        self.assertIn("SCCACHE_S3_USE_SSL=true", startup)
        self.assertIn("RUSTC_WRAPPER=sccache", startup)
        self.assertIn("unset RUSTC_WRAPPER", startup)
        self.assertIn("continuing with direct rustc", startup)
        self.assertIn("_sccache-stats.txt", startup)

    def test_performance_workers_consume_one_exact_prebuilt_bundle(self) -> None:
        workflow = (ROOT / ".github/workflows/release-qualify.yml").read_text()
        startup = (ROOT / "infra/release-runners/aws-release-startup.sh").read_text()
        builder = (
            ROOT / "infra/release-runners/aws-performance-prebuild-startup.sh"
        ).read_text()
        helper = (ROOT / "scripts/release/prebuilt_performance.py").read_text()

        self.assertIn("Build selected performance executables once", workflow)
        self.assertIn("--instance-type m5.8xlarge", workflow)
        self.assertIn(
            "'DeviceName=/dev/sda1,Ebs={VolumeSize=200,VolumeType=gp3,DeleteOnTermination=true}'",
            workflow,
        )
        self.assertLess(
            workflow.index("Build selected performance executables once"),
            workflow.index("Create performance workers once the bundle exists"),
        )
        # Workers that read no bundle launch before it is built, and only they do.
        early = workflow.index("Create bundle-free workers before the prebuild")
        self.assertLess(early, workflow.index("Build selected performance executables once"))
        early_step = workflow[early : workflow.index("Build selected performance executables once")]
        self.assertIn('startswith("ec2-performance") | not', early_step)
        self.assertIn('PREBUILT_URI: ""', early_step)
        late = workflow.index("Create performance workers once the bundle exists")
        late_step = workflow[late : workflow.index("Wait for every immutable worker result")]
        self.assertIn('select(.resource_class | startswith("ec2-performance"))', late_step)
        self.assertIn('--env "RVOIP_PREBUILT_URI=${PREBUILT_URI}"', workflow)
        self.assertIn('--env "RVOIP_PREBUILT_SHA256=${PREBUILT_SHA256}"', workflow)
        self.assertIn("install-bundle", startup)
        self.assertIn("RVOIP_PERF_PREBUILT_MANIFEST", startup)
        self.assertIn(
            'run_bundle_prefix="s3://${BUCKET}/release/${RUN_ID}/prebuild/"',
            startup,
        )
        self.assertIn(
            'cache_bundle_prefix="s3://${BUCKET}/release-cache/performance-prebuilt-v1/"',
            startup,
        )
        self.assertIn(
            '--env "RVOIP_EXTERNAL_MEMORY_DIAGNOSTICS=${EXTERNAL_MEMORY_DIAGNOSTICS}"',
            workflow,
        )
        self.assertIn(
            "inputs.profile == 'remote-diagnostic' && '1' || '0'", workflow
        )
        self.assertIn("capture_external_memory", startup)
        self.assertIn("AnonHugePages", startup)
        self.assertIn("thp_collapse_alloc", startup)
        self.assertIn('MIMALLOC_ALLOW_THP_OVERRIDE="${RVOIP_MIMALLOC_ALLOW_THP:-}"', startup)
        self.assertIn('export MIMALLOC_ALLOW_THP="$MIMALLOC_ALLOW_THP_OVERRIDE"', startup)
        self.assertIn(
            '"/bundles/${PREBUILT_SHA256}.tar.gz"',
            startup,
        )
        self.assertIn("performance-prebuilt.tar.gz", builder)
        self.assertIn('"schema": "rvoip-ec2-performance-prebuild-result-v1"', builder)
        self.assertIn('download "$MANIFEST_OBJECT"', builder)
        self.assertIn("performance-manifest-readback.json", builder)
        self.assertIn("publishing_attempted", builder)
        self.assertIn("bundle digest mismatch", helper)
        self.assertIn("exact candidate", helper)

    def test_production_allocator_disables_transparent_huge_pages(self) -> None:
        manifest = tomllib.loads(
            (ROOT / "crates/foundation/infra-common/Cargo.toml").read_text()
        )
        mimalloc = manifest["dependencies"]["mimalloc"]
        self.assertFalse(mimalloc["default-features"])
        self.assertIn("no_thp", mimalloc["features"])

    def test_exact_candidate_performance_bundle_is_reused_fail_closed(self) -> None:
        workflow = (ROOT / ".github/workflows/release-qualify.yml").read_text()
        builder = (
            ROOT / "infra/release-runners/aws-performance-prebuild-startup.sh"
        ).read_text()
        helper = (ROOT / "scripts/release/prebuilt_performance.py").read_text()

        self.assertIn("release-cache/performance-prebuilt-v1", workflow)
        self.assertIn("cache-key", workflow)
        self.assertIn("--cache-key", workflow)
        self.assertLess(
            workflow.index("cached-result.json"),
            workflow.index("--instance-type m5.8xlarge"),
        )
        self.assertIn('--env "RVOIP_PREBUILD_CACHE_KEY=${cache_key}"', workflow)
        self.assertIn('CACHE_KEY="$RVOIP_PREBUILD_CACHE_KEY"', builder)
        self.assertIn("bundles/${BUNDLE_SHA}.tar.gz", builder)
        self.assertIn("manifests/${MANIFEST_SHA}.json", builder)
        self.assertIn("ensure_content_addressed", builder)
        self.assertIn("content-addressed object digest mismatch", builder)
        self.assertIn('if (( exit_code == 0 )); then', builder)
        self.assertIn('upload "$RESULT" "${CACHE_PREFIX}/prebuild-result.json"', builder)
        self.assertIn("cache_key_sha256", helper)
        self.assertIn("outside the expected cache namespace", helper)

    def test_ec2_release_builds_use_the_versioned_lld_environment(self) -> None:
        workflow = (ROOT / ".github/workflows/release-qualify.yml").read_text()
        worker = (ROOT / "infra/release-runners/aws-release-startup.sh").read_text()
        builder = (
            ROOT / "infra/release-runners/aws-performance-prebuild-startup.sh"
        ).read_text()

        self.assertIn("prebuilt-perf-v2-lld", workflow)
        for startup in (builder, worker):
            with self.subTest(startup=startup):
                self.assertIn("libssl-dev lld", startup)
                self.assertIn("command -v ld.lld >/dev/null", startup)
                self.assertIn("ld.lld --version", startup)
                self.assertIn('RUSTFLAGS="-C link-arg=-fuse-ld=lld"', startup)
                self.assertIn("rvoip-release-v2-lld", startup)

    def test_ec2_artifact_uploads_stream_regular_files(self) -> None:
        for relative in (
            "infra/release-runners/aws-performance-prebuild-startup.sh",
            "infra/release-runners/aws-release-startup.sh",
            "infra/release-runners/aws-release-shutdown.sh",
        ):
            startup = (ROOT / relative).read_text()
            with self.subTest(startup=relative):
                self.assertIn(
                    'aws s3 cp --only-show-errors "$source" "s3://${BUCKET}/${object}"',
                    startup,
                )
                self.assertNotIn("--data-binary", startup)
                self.assertNotIn("--upload-file", startup)

    def test_release_infrastructure_preflight_is_full_shape_and_non_publishing(self) -> None:
        workflow = (ROOT / ".github/workflows/release-qualify.yml").read_text()
        startup = (ROOT / "infra/release-runners/aws-release-startup.sh").read_text()
        probe = (
            ROOT / "infra/release-runners/release-infrastructure-preflight.sh"
        ).read_text()
        burst = (
            ROOT / "crates/sip/rvoip-sip/scripts/perf_burst_matrix.sh"
        ).read_text()

        self.assertIn("remote-preflight", workflow)
        self.assertIn('test "$PROFILE" = remote-preflight', workflow)
        self.assertIn(
            "Require protected main candidate or trusted diagnostic branch head",
            workflow,
        )
        self.assertIn("RVOIP_AWS_PROVISIONER_ROLE_ARN", workflow)
        self.assertNotIn("RVOIP_GCP_", workflow)
        self.assertNotIn("pilot", workflow)
        self.assertIn('export RVOIP_RELEASE_RESOURCE_CLASS="$RESOURCE_CLASS"', startup)
        self.assertIn('export RVOIP_RELEASE_CANDIDATE="$CANDIDATE"', startup)
        self.assertIn('export RVOIP_RELEASE_GATES="$GATES"', startup)
        self.assertIn("sysctl -w net.core.rmem_max=67108864", startup)
        self.assertIn("sysctl -w net.core.wmem_max=67108864", startup)
        # One launch for the prebuilder, one for bundle-free workers (interop and
        # proxy interop) before the prebuild, one for performance workers after.
        self.assertEqual(workflow.count("aws ec2 run-instances"), 3)
        self.assertIn(
            "RELEASE_ENVIRONMENT_ID: rvoip-release-v6-rust-1.91-nextest-0.9.140-prebuilt-perf-v2-lld-ec2-m5-perf16-soaklong16-interop16",
            workflow,
        )
        self.assertIn("expected 46 publishable workspace packages", probe)
        self.assertIn("ec2-performance|ec2-performance-soak-long", probe)
        self.assertIn("ec2-proxy-interop", probe)
        self.assertIn("for _ in range(4096)", probe)
        self.assertIn('test "$NOFILE_SOFT" -ge 262144', probe)
        self.assertIn('test "$RMEM_MAX" -ge 8388608', probe)
        self.assertIn('test "$WMEM_MAX" -ge 8388608', probe)
        self.assertIn("linux_performance_host.py snapshot", probe)
        self.assertIn("socket-buffer-probe.json", probe)
        self.assertIn("linux_performance_host.py", burst)
        self.assertIn("--require-zero-drops", burst)
        self.assertIn('test -z "${CARGO_REGISTRY_TOKEN:-}"', probe)
        self.assertIn('test -z "${CRATES_IO_TOKEN:-}"', probe)
        self.assertIn('"publishing_credentials_present": False', probe)

    def test_release_planner_outlives_exact_candidate_codeql_poll(self) -> None:
        workflow = (ROOT / ".github/workflows/release-qualify.yml").read_text()
        plan = workflow.split("  plan:\n", maxsplit=1)[1].split("\n  gate-hosted:\n", maxsplit=1)[0]

        self.assertIn("timeout-minutes: 75", plan)
        self.assertIn("--timeout-seconds 3600", plan)

    def test_remote_diagnostics_are_exact_gate_fresh_and_non_publishing(self) -> None:
        workflow = (ROOT / ".github/workflows/release-qualify.yml").read_text()
        publication = (ROOT / ".github/workflows/release-publish.yml").read_text()

        self.assertIn("remote-diagnostic", workflow)
        self.assertIn("diagnostic_gates:", workflow)
        self.assertIn('args+=(--only-gates "$DIAGNOSTIC_GATES")', workflow)
        self.assertIn('test "$PROFILE" = remote-diagnostic', workflow)
        self.assertIn("inputs.profile != 'remote-diagnostic'", workflow)
        self.assertIn("inputs.diagnostic_gates || 'all'", workflow)
        self.assertIn("diagnostic candidates must be the exact head", workflow)
        self.assertIn("git ls-remote --heads origin", workflow)
        self.assertIn("Upload non-reusable diagnostic evidence", workflow)
        self.assertIn("name: diagnostic-gate-evidence", workflow)
        self.assertIn("Reusable for release: no", workflow)
        self.assertIn("at most five prior evidence runs may be combined", workflow)
        self.assertIn('--dir "target/prior-evidence/$run_id"', workflow)
        self.assertIn(
            'test "$(jq -r .profile "$aggregate")" = remote-release',
            publication,
        )

    def test_release_resume_only_finishes_a_proven_post_gate_failure(self) -> None:
        resume = (ROOT / ".github/workflows/release-resume.yml").read_text()

        self.assertIn('"workflowName": "Release qualification"', resume)
        self.assertIn('"conclusion": "failure"', resume)
        self.assertIn('release_gate.get("name") != "Release Gate"', resume)
        self.assertIn(
            '["Render exact-candidate qualification reports"]', resume
        )
        self.assertIn('"status": "PASS"', resume)
        self.assertIn('"gate_count": 213', resume)
        self.assertIn('"fresh_count": 213', resume)
        self.assertIn('"reused_count": 0', resume)
        self.assertIn('len(aggregate.get("accepted_gates", [])) != 213', resume)
        self.assertIn("git merge-base --is-ancestor", resume)
        self.assertIn("render_qualification_reports.py generate", resume)
        self.assertIn('"test_execution_repeated": False', resume)
        self.assertIn("actions/attest-build-provenance@", resume)
        self.assertIn("name: release-gate-evidence", resume)
        self.assertNotIn("scripts/release/gates.py run", resume)
        self.assertNotIn("cargo test", resume)

    def test_release_publish_consumes_exact_attested_candidate(self) -> None:
        publication = (ROOT / ".github/workflows/release-publish.yml").read_text()
        preflight = publication.split(
            "      - name: Resolve, verify, and check out qualified candidate\n",
            maxsplit=1,
        )[1].split("      - name: Fresh package and source verification\n", maxsplit=1)[0]

        self.assertIn("GH_TOKEN: ${{ github.token }}", preflight)
        self.assertIn('gh attestation verify "$aggregate"', preflight)
        self.assertIn('candidate="$(jq -r .candidate_sha "$aggregate")"', preflight)
        self.assertIn('git merge-base --is-ancestor "$candidate" origin/main', preflight)
        self.assertIn("cp scripts/release.py target/release-tool.py", preflight)
        self.assertIn('git checkout -B main "$candidate"', preflight)
        self.assertIn('test "$(git rev-parse HEAD)" = "$candidate"', preflight)
        self.assertLess(
            preflight.index('gh attestation verify "$aggregate"'),
            preflight.index('git checkout -B main "$candidate"'),
        )
        self.assertIn('python3 "$RELEASE_TOOL" verify', publication)
        self.assertIn('--qualified-head "$QUALIFIED_CANDIDATE"', publication)
        self.assertIn('python3 "$RELEASE_TOOL" "${args[@]}"', publication)

    def test_release_publish_retains_qualification_reports_as_release_assets(self) -> None:
        publication = (ROOT / ".github/workflows/release-publish.yml").read_text()
        release = publication.split(
            "      - name: Create immutable tag and generated GitHub release\n",
            maxsplit=1,
        )[1]

        self.assertIn("QUALIFICATION_RUN_ID: ${{ inputs.qualification_run_id }}", release)
        self.assertIn('test -s "$asset"', release)
        self.assertIn('"$reports/BETA_RELEASE_REPORT.md"', release)
        self.assertIn('"$reports/BETA_GATE_REPORT.md"', release)
        self.assertIn('"$reports/BETA_PERFORMANCE_REPORT.md"', release)
        self.assertIn('"$reports/QUALIFICATION_SUMMARY.json"', release)
        self.assertIn('"$reports/QUALIFICATION_REPORT_ATTESTATION.json"', release)
        self.assertIn('"$performance/current-performance-evaluation.json"', release)
        self.assertIn('"$performance/current-performance-artifact-index.json"', release)
        self.assertIn('gh release create "$tag" "${assets[@]}"', release)
        self.assertIn("Protected qualification run and full evidence", release)
        self.assertIn('--notes "$release_notes"', release)

    def test_release_ec2_workers_do_not_consume_one_github_job_each(self) -> None:
        workflow = (ROOT / ".github/workflows/release-qualify.yml").read_text()
        fanout = (ROOT / "scripts/release/aws_fanout.py").read_text()
        startup = (ROOT / "infra/release-runners/aws-release-startup.sh").read_text()
        shutdown = (ROOT / "infra/release-runners/aws-release-shutdown.sh").read_text()
        controller = workflow.split("\n  gate-aws:\n", maxsplit=1)[1].split(
            "\n  cleanup-aws:\n", maxsplit=1
        )[0]
        self.assertIn("Early failure cutoff", controller)
        self.assertIn("early-failure-decision", controller)
        self.assertIn("aws ec2 stop-instances", controller)
        self.assertIn("aws-release-shutdown.sh", controller)
        self.assertIn("stopped|terminated|shutting-down|stopping)", controller)
        self.assertIn(
            'TERMINAL_STATES = frozenset({"stopped", "terminated", "shutting-down", "stopping"})',
            fanout,
        )

        self.assertNotIn("strategy:", controller)
        self.assertNotIn("matrix: ${{ fromJSON", controller)
        self.assertIn("Run all ephemeral EC2 release shards", controller)
        self.assertIn("- GitHub controller jobs: 1", controller)
        self.assertIn("aws_fanout.py prepare", controller)
        self.assertIn("Create performance workers once the bundle exists", controller)
        self.assertIn('pids+=("$!")', controller)
        self.assertIn("Wait for every immutable worker result", controller)
        self.assertIn("aws_fanout.py verify", controller)
        self.assertIn("Terminate all workers and root volumes", controller)
        self.assertIn("release-gate-shard-aws-controller", controller)
        self.assertIn('RESULT_SCHEMA = "rvoip-ec2-release-shard-v1"', fanout)
        self.assertIn('"schema": "rvoip-ec2-release-shard-v1"', startup)
        self.assertIn('"schema": "rvoip-ec2-release-shard-v1"', shutdown)

    def test_release_pbx_lifecycle_uses_tracked_templates(self) -> None:
        lifecycle = (ROOT / "infra/release-runners/interop-lifecycle.sh").read_text()
        self.assertIn("infra/release-runners/pbx", lifecycle)
        self.assertNotIn("beta-report/", lifecycle)
        self.assertIn("wait_udp_port", lifecycle)
        self.assertIn("/proc/net/udp", lifecycle)
        self.assertNotIn("nc -z 127.0.0.1", lifecycle)
        self.assertIn("--name rvoip-freeswitch", lifecycle)
        self.assertIn("wait_udp_port rvoip-freeswitch 5062", lifecycle)
        self.assertIn("wait_freeswitch_control_plane", lifecycle)
        self.assertIn(
            "wait_freeswitch_control_plane rvoip-freeswitch ClueCon", lifecycle
        )
        self.assertIn('fs_cli -p "$password"', lifecycle)
        self.assertIn('-x "sofia status"', lifecycle)
        self.assertIn('rvoip_udp[[:space:]].*RUNNING', lifecycle)
        self.assertIn('rvoip_tls_srtp[[:space:]].*RUNNING', lifecycle)
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

        pbx_runner = (
            ROOT / "crates/sip/rvoip-sip/examples/pbx/run.sh"
        ).read_text()
        self.assertIn('fs_cli_rc=0', pbx_runner)
        self.assertIn('FREESWITCH_EVENT_SOCKET_PASSWORD:-ClueCon', pbx_runner)
        self.assertIn('[ "$openssl_rc" -eq 0 ]', pbx_runner)
        self.assertIn('[ "$fs_cli_rc" -eq 0 ]', pbx_runner)
        self.assertIn(
            "nc not found; TLS readiness requires the TCP socket probe", pbx_runner
        )
        self.assertIn(
            "openssl not found; TLS readiness requires the handshake probe",
            pbx_runner,
        )
        self.assertIn('freeswitch-container.log', pbx_runner)

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

    def test_burst_rss_gate_precedes_final_structural_snapshot(self) -> None:
        for path in (
            "crates/sip/rvoip-sip/tests/perf/perf_burst_caller.rs",
            "crates/sip/rvoip-sip/tests/perf/perf_burst_receiver.rs",
        ):
            source = (ROOT / path).read_text()
            with self.subTest(path=path):
                periodic_stop = source.index("retention_sampler.stop_periodic()")
                settled_window = source.index("sample_settled_rss_window(", periodic_stop)
                rss_gate = source.index("rss_result_metrics(", settled_window)
                final_retention = source.index(
                    "capture_endpoint_retention_sample(", rss_gate
                )
                self.assertLess(periodic_stop, settled_window)
                self.assertLess(settled_window, rss_gate)
                self.assertLess(rss_gate, final_retention)
                self.assertIn("&settled_resources", source[rss_gate : rss_gate + 250])

    def test_burst_caller_signals_receiver_before_post_load_qualification(self) -> None:
        caller = (
            ROOT / "crates/sip/rvoip-sip/tests/perf/perf_burst_caller.rs"
        ).read_text()
        active_end = caller.index("let active_wall = started.elapsed();")
        stop_write = caller.index("std::fs::write(&path", active_end)
        periodic_stop = caller.index("retention_sampler.stop_periodic()", stop_write)
        self.assertLess(active_end, stop_write)
        self.assertLess(stop_write, periodic_stop)

        runner = (
            ROOT / "crates/sip/rvoip-sip/scripts/perf_burst_matrix.sh"
        ).read_text()
        self.assertEqual(runner.count("cargo test"), 1)
        self.assertIn("--test perf_burst_receiver", runner)
        self.assertIn("--test perf_burst_caller", runner)
        self.assertEqual(runner.count("--build-target perf_burst_"), 4)
        self.assertIn("RVOIP_PERF_PREBUILT_MANIFEST", runner)

    def test_release_memory_gates_isolate_structural_diagnostics(self) -> None:
        receiver = (
            ROOT / "crates/sip/rvoip-sip/tests/perf/perf_burst_receiver.rs"
        ).read_text()
        self.assertIn(
            "EndpointRetentionSampler::start_with_periodic_limit(", receiver
        )
        self.assertIn("burst_retention_periodic_limit(", receiver)

        for path in (
            "crates/sip/rvoip-sip/tests/perf/perf_soak_30min.rs",
            "crates/sip/rvoip-sip/tests/perf/perf_soak_caller.rs",
            "crates/sip/rvoip-sip/tests/perf/perf_soak_receiver.rs",
        ):
            with self.subTest(path=path):
                source = (ROOT / path).read_text()
                self.assertIn("long_soak_retention_periodic_limit(", source)


if __name__ == "__main__":
    unittest.main()
