from __future__ import annotations

import fnmatch
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
                # A local reusable workflow is pinned by the commit it runs from.
                self.assertTrue(
                    all(
                        SHA_ACTION.search(line) or "uses: ./.github/workflows/" in line
                        for line in uses
                    )
                )

    def test_every_runner_job_declares_a_timeout(self) -> None:
        # GitHub's default job timeout is six hours; every hosted job sets
        # its own bound so a hung step cannot hold a runner that long.
        job_header = re.compile(r"^  ([A-Za-z0-9_-]+):\s*$")
        for workflow in sorted((ROOT / ".github/workflows").glob("*.yml")):
            lines = workflow.read_text().splitlines()
            start = lines.index("jobs:") + 1
            jobs: dict[str, list[str]] = {}
            current = None
            for line in lines[start:]:
                if line and not line.startswith(" "):
                    break
                match = job_header.match(line)
                if match:
                    current = match.group(1)
                    jobs[current] = []
                elif current:
                    jobs[current].append(line)
            self.assertTrue(jobs, workflow.name)
            for job, body in jobs.items():
                if not any(line.strip().startswith("runs-on:") for line in body):
                    continue  # reusable-workflow call
                with self.subTest(workflow=workflow.name, job=job):
                    self.assertTrue(
                        any(line.strip().startswith("timeout-minutes:") for line in body)
                    )

    def test_pr_gate_has_no_secret_or_write_permission_surface(self) -> None:
        text = (ROOT / ".github/workflows/pr-gate.yml").read_text()
        self.assertNotIn("secrets.", text)
        # GITHUB_TOKEN itself stays read-only; only the App token writes.
        self.assertIsNone(re.search(r"(?m)^  contents: write", text))
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
        gates = sorted(run_checks.ALL_CODEC_FEATURE_GATES)
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
        # Identical shared patterns across every codec gate, so splitting the
        # job did not narrow what any one change verifies.
        sip_shards = set(run_checks.CODEC_FEATURE_SIP_SHARDS)
        sip_only = {"crates/sip/rvoip-sip/tests/**"}
        self.assertEqual(
            len({frozenset(rules[gate] - sip_only) for gate in gates}), 1
        )
        # The rvoip-sip shards also run every rvoip-sip test target with the
        # codec features on, so a PR touching only a SIP test selects them;
        # the other packages' gates never build those targets.
        for gate in gates:
            with self.subTest(gate=gate):
                if gate in sip_shards:
                    self.assertLessEqual(sip_only, rules[gate])
                else:
                    self.assertFalse(sip_only & rules[gate])

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

    def test_rustdoc_gate_runs_on_prs_and_is_forced_on_main(self) -> None:
        # #315: `build.rustdoc` was a release-only check. PRs select it from
        # policy patterns; main runs it unconditionally, like the other
        # all-features gates, so a miss in the patterns still surfaces there
        # rather than at release qualification.
        policy = json.loads((ROOT / "scripts/ci/policy.json").read_text())
        gates = {rule["gate"] for rule in policy["specialty_rules"]}
        self.assertIn("rustdoc", gates)
        main = (ROOT / ".github/workflows/main-ci.yml").read_text()
        self.assertIn("--specialty-gate rustdoc", main)
        # `--all-features` builds rvoip-webrtc's libvpx bindings, as does
        # feature-targets-webrtc's `client-video-vp8`, so both specialty jobs
        # must install the headers for every gate in LIBVPX_GATES.
        self.assertIn("rustdoc", run_checks.LIBVPX_GATES)
        condition = " || ".join(
            f"matrix.gate == '{gate}'" for gate in run_checks.LIBVPX_GATES
        )
        for workflow in ("pr-gate.yml", "main-ci.yml"):
            text = (ROOT / ".github/workflows" / workflow).read_text()
            specialty = text.split("\n  specialty:\n", maxsplit=1)[1]
            step = specialty.split("Install libvpx for VP8 bindings", 1)
            with self.subTest(workflow=workflow):
                self.assertEqual(len(step), 2)
                self.assertIn(f"if: {condition}", step[1][:200])
                self.assertIn("libvpx-dev", step[1][:200])
                # env-libvpx-sys's bindgen 0.68 breaks under libclang 19+
                # (rust-bindgen#2824), so these gates stay on 24.04's clang 18.
                self.assertIn(
                    f"runs-on: ${{{{ ({condition}) && 'ubuntu-24.04' "
                    "|| 'ubuntu-latest' }}",
                    re.split(r"\n  (?=\S)", specialty, maxsplit=1)[0],
                )
        qualify = (ROOT / ".github/workflows/release-qualify.yml").read_text()
        hosted = qualify.split("\n  gate-hosted:\n", maxsplit=1)[1]
        hosted = re.split(r"\n  (?=\S)", hosted, maxsplit=1)[0]
        self.assertIn("runs-on: ubuntu-24.04", hosted)

    def test_every_rustdoc_included_document_selects_the_rustdoc_gate(self) -> None:
        # A file pulled into rustdoc with `doc = include_str!(...)` is rendered
        # (and link-checked) by `cargo doc`, yet is Markdown the planner
        # otherwise treats as docs-only. Each one must be named by the gate.
        policy = json.loads((ROOT / "scripts/ci/policy.json").read_text())
        patterns = next(
            rule["patterns"]
            for rule in policy["specialty_rules"]
            if rule["gate"] == "rustdoc"
        )
        include = re.compile(r'\bdoc\s*=\s*include_str!\(\s*"([^"]+)"\s*\)')
        included = set()
        for source in (ROOT / "crates").rglob("*.rs"):
            if "target" in source.relative_to(ROOT).parts:
                continue
            for relative in include.findall(source.read_text(errors="replace")):
                document = (source.parent / relative).resolve()
                included.add(document.relative_to(ROOT.resolve()).as_posix())
        self.assertTrue(included)
        for document in sorted(included):
            with self.subTest(document=document):
                self.assertTrue(
                    any(fnmatch.fnmatchcase(document, pattern) for pattern in patterns)
                )

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
        commit_step = text.index("Commit and push the version bump")

        self.assertLess(prepare_step, helper_test)
        self.assertLess(helper_test, commit_step)

        policy = json.loads((ROOT / "scripts/ci/policy.json").read_text())
        release_tooling = next(
            rule
            for rule in policy["specialty_rules"]
            if rule["gate"] == "release-tooling"
        )
        for path in (
            "docs/release/RELEASE_NOTES_NEXT.md",
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
            "RELEASE_ENVIRONMENT_ID: rvoip-release-v6-rust-1.95-nextest-0.9.140-prebuilt-perf-v2-lld-ec2-m5-perf16-soaklong16-interop16",
            workflow,
        )
        self.assertIn("expected 48 publishable workspace packages", probe)
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

    def test_exact_candidate_codeql_is_a_parallel_gate_not_a_plan_step(self) -> None:
        workflow = (ROOT / ".github/workflows/release-qualify.yml").read_text()
        plan = workflow.split("  plan:\n", maxsplit=1)[1].split("\n  gate-hosted:\n", maxsplit=1)[0]
        catalog = json.loads((ROOT / "scripts/release/gates.json").read_text())
        gate = next(g for g in catalog["gates"] if g["id"] == "security.codeql-exact")

        # The 23-29 minute CodeQL wait no longer sits in front of every worker.
        self.assertNotIn("check_codeql_release_policy.py", plan)
        self.assertIn("timeout-minutes: 20", plan)
        self.assertEqual(gate["resource_class"], "github-evidence")
        self.assertIn("--timeout-seconds", gate["command"])
        self.assertIn("3600", gate["command"])
        self.assertGreaterEqual(gate["timeout_minutes"], 61)
        self.assertIn(gate["id"], catalog["profiles"]["remote-release"])
        self.assertIn("GH_TOKEN: ${{ matrix.needs_github_token && github.token || '' }}", workflow)

    def test_no_schedule_starts_an_ec2_qualification(self) -> None:
        # The full fanout is 24 workers and 272 vCPUs; it runs only when
        # someone dispatches it.
        for workflow in sorted((ROOT / ".github/workflows").glob("*.yml")):
            text = workflow.read_text()
            if "schedule:" not in text:
                continue
            with self.subTest(workflow=workflow.name):
                self.assertNotIn("release-qualify.yml", text)
                self.assertNotIn("aws_fanout.py", text)
        interop = (ROOT / ".github/workflows/nightly-interop.yml").read_text()
        self.assertIn("schedule:", interop)
        self.assertNotIn("self-hosted", interop)
        catalog = json.loads((ROOT / "scripts/release/gates.json").read_text())
        self.assertFalse(
            {"perf.rfc7502-release", "perf.rfc7502-minor"} & set(catalog["profiles"]["remote-release"])
        )

    def test_bench_jobs_keep_setup_and_teardown_headroom(self) -> None:
        # A bench job spends its time outside run_lane's --max-wall on checkout,
        # the toolchain, packages, a cold DUT build and up to 5 min of teardown.
        # If GitHub's job timeout lands first, it kills the lane mid-teardown.
        # A capacity wait counts inside --max-wall, so it is not part of that
        # headroom; it must instead leave at least half the budget for the lane.
        workflow = (ROOT / ".github/workflows/release-qualify.yml").read_text()
        jobs = dict(
            (block.split(":\n", 1)[0].strip(), block)
            for block in re.split(r"\n(?=  [a-z0-9-]+:\n)", workflow.split("\njobs:\n", 1)[1])
        )
        bench_gate = (ROOT / "scripts/release/bench_gate.py").read_text()
        default = int(re.search(r"^DEFAULT_MAX_WALL_MINUTES = (\d+)$", bench_gate, re.M).group(1))
        default_wait = int(re.search(r"^CAPACITY_WAIT_MINUTES = (\d+)$", bench_gate, re.M).group(1))
        self.assertIn("against --max-wall", bench_gate)
        for name in ("bench-leg-1", "bench-leg-2", "gate-bench"):
            with self.subTest(job=name):
                block = jobs[name]
                timeout = int(re.search(r"\n    timeout-minutes: (\d+)\n", block).group(1))
                walls = [int(value) for value in re.findall(r"--max-wall (\d+)", block)]
                walls += [int(value) for value in re.findall(r'RVOIP_BENCH_MAX_WALL: "(\d+)"', block)]
                if "bench_gate.py" in block and not walls:
                    walls = [default]
                self.assertTrue(walls, f"{name} sets no bench wall budget")
                waits = [int(value) for value in re.findall(r"--capacity-wait (\d+)", block)]
                waits += [int(value) for value in re.findall(r'RVOIP_BENCH_CAPACITY_WAIT: "(\d+)"', block)]
                waits = waits or [default_wait]
                for wall in walls:
                    self.assertGreaterEqual(timeout - wall, 45, f"{name}: {timeout} - {wall}")
                    for wait in waits:
                        self.assertLessEqual(2 * wait, wall, f"{name}: wait {wait} in wall {wall}")

    def test_bench_reservations_are_released_after_the_last_bench_job(self) -> None:
        workflow = (ROOT / ".github/workflows/release-qualify.yml").read_text()
        jobs = dict(
            (block.split(":\n", 1)[0], block)
            for block in re.split(r"\n(?=  [a-z0-9-]+:\n)", workflow.split("\njobs:\n", 1)[1])
        )
        cleanup = jobs["  cleanup-bench"]
        self.assertIn("if: always() && needs.plan.outputs.bench_shard_count != '0'", cleanup)
        self.assertIn("needs: [plan, bench-leg-1, bench-leg-2, gate-bench]", cleanup)
        self.assertIn("role-to-assume: ${{ vars.RVOIP_AWS_PROVISIONER_ROLE_ARN }}", cleanup)
        self.assertIn("LANE_RUN_IDS: bench-*-${{ github.run_id }}", cleanup)
        self.assertIn("Name=tag:rvoip-bench,Values=true", cleanup)
        self.assertIn("aws ec2 cancel-capacity-reservation", cleanup)
        self.assertIn("aws ec2 terminate-instances", cleanup)
        self.assertIn('test -z "$remaining"', cleanup)
        # A reservation held for the next leg must survive the earlier legs.
        for name, block in jobs.items():
            if name.strip() != "cleanup-bench":
                with self.subTest(job=name.strip()):
                    self.assertNotIn("cancel-capacity-reservation", block)
        self.assertIn("cleanup-bench", jobs["  collect"].split("\n    needs:", 1)[1].split("\n", 1)[0])

    def test_hosted_release_shards_restore_a_compiler_cache(self) -> None:
        workflow = (ROOT / ".github/workflows/release-qualify.yml").read_text()
        hosted = workflow.split("\n  gate-hosted:\n", maxsplit=1)[1].split("\n  gate-bench:\n", maxsplit=1)[0]
        self.assertIn("Swatinem/rust-cache@", hosted)
        self.assertIn("shared-key: release-hosted-${{ matrix.id }}", hosted)

    def test_early_failure_cutoff_is_scoped_by_the_catalog(self) -> None:
        workflow = (ROOT / ".github/workflows/release-qualify.yml").read_text()
        self.assertIn("--catalog scripts/release/gates.json)", workflow)

    def test_release_candidate_is_any_main_commit_with_a_receipt_chain(self) -> None:
        workflow = (ROOT / ".github/workflows/release-qualify.yml").read_text()

        self.assertNotIn("must qualify the current protected main head", workflow)
        self.assertIn("git merge-base --is-ancestor HEAD origin/main", workflow)
        self.assertIn("### Receipt chain", workflow)
        self.assertIn("--prior-evidence target/prior-evidence", workflow)
        self.assertIn("fetch-depth: 0", workflow.split("  collect:\n", maxsplit=1)[1])

    def test_branch_diagnostics_run_only_on_their_own_ref(self) -> None:
        # A run's caches are scoped to github.ref. Code that is not on main
        # must therefore run from its own branch ref, never from main's, or
        # the caches it saves would reach main's runs (CodeQL #913).
        workflow = (ROOT / ".github/workflows/release-qualify.yml").read_text()
        name = "Require protected main candidate or trusted diagnostic branch head"
        step = workflow.split(f"- name: {name}\n", 1)[1]
        lines = step.split("        run: |\n", 1)[1].splitlines()
        script = []
        for line in lines:
            if line.strip() and not line.startswith(" " * 10):
                break
            script.append(line[10:])
        script = "\n".join(script) + "\n"

        import subprocess
        import tempfile

        def git(cwd, *args):
            return subprocess.run(
                ["git", "-c", "user.name=t", "-c", "user.email=t@t", "-c", "init.defaultBranch=main", *args],
                cwd=cwd, check=True, capture_output=True, text=True,
            ).stdout.strip()

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            origin, work = root / "origin.git", root / "work"
            git(root, "init", "--bare", str(origin))
            git(root, "init", str(work))
            git(work, "commit", "--allow-empty", "-m", "main")
            main = git(work, "rev-parse", "HEAD")
            git(work, "remote", "add", "origin", str(origin))
            git(work, "push", "origin", "HEAD:refs/heads/main")
            git(work, "commit", "--allow-empty", "-m", "branch 1")
            stale = git(work, "rev-parse", "HEAD")
            git(work, "commit", "--allow-empty", "-m", "branch 2")
            head = git(work, "rev-parse", "HEAD")
            git(work, "push", "origin", "HEAD:refs/heads/feature/x")

            def allowed(candidate, profile, ref):
                git(work, "checkout", "--quiet", "--detach", candidate)
                return subprocess.run(
                    ["bash", "-c", script], cwd=work, capture_output=True, text=True,
                    env={"PATH": __import__("os").environ["PATH"], "PROFILE": profile,
                         "GITHUB_REF": ref, "HOME": directory},
                ).returncode == 0

            self.assertTrue(allowed(main, "remote-release", "refs/heads/main"))
            self.assertTrue(allowed(main, "remote-diagnostic", "refs/heads/main"))
            self.assertTrue(allowed(head, "remote-diagnostic", "refs/heads/feature/x"))
            self.assertFalse(allowed(head, "remote-diagnostic", "refs/heads/main"))
            self.assertFalse(allowed(head, "remote-release", "refs/heads/feature/x"))
            self.assertFalse(allowed(stale, "remote-diagnostic", "refs/heads/feature/x"))
            self.assertFalse(allowed(head, "remote-diagnostic", "refs/tags/feature/x"))

    def test_remote_diagnostics_are_exact_gate_fresh_and_non_publishing(self) -> None:
        workflow = (ROOT / ".github/workflows/release-qualify.yml").read_text()
        publication = (ROOT / ".github/workflows/release-publish.yml").read_text()

        self.assertIn("remote-diagnostic", workflow)
        self.assertIn("diagnostic_gates:", workflow)
        self.assertIn('args+=(--only-gates "$DIAGNOSTIC_GATES")', workflow)
        self.assertIn('test "$PROFILE" = remote-diagnostic', workflow)
        self.assertIn("inputs.profile != 'remote-diagnostic'", workflow)
        self.assertIn("inputs.diagnostic_gates || 'all'", workflow)
        self.assertIn("must be the head of the branch the run is dispatched on", workflow)
        self.assertNotIn("git ls-remote --heads origin", workflow)
        self.assertIn("Upload non-reusable diagnostic evidence", workflow)
        self.assertIn("name: diagnostic-gate-evidence", workflow)
        self.assertIn("Reusable for release: no", workflow)
        download = (ROOT / "scripts/release/download_prior_evidence.sh").read_text()
        self.assertIn("bash scripts/release/download_prior_evidence.sh", workflow)
        self.assertIn("at most five prior evidence runs may be combined", download)
        self.assertIn('--dir "target/prior-evidence/$run_id"', download)
        # Release profiles reuse only attested release evidence; only a
        # diagnostic may read diagnostic evidence.
        self.assertIn('if [[ "${PROFILE:?}" == remote-diagnostic ]]; then', download)
        self.assertIn("--name diagnostic-gate-evidence", download)
        self.assertIn(
            "remote-release|release-patch|release-minor) ;;",
            (ROOT / "scripts/release/resolve_qualified_candidate.sh").read_text(),
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
        # The gate count is read from the candidate's catalog, never pinned.
        self.assertNotIn("213", resume)
        self.assertIn('git show "$candidate:scripts/release/gates.json"', resume)
        self.assertIn('expected_ids = catalog["profiles"][profile]', resume)
        self.assertIn('"gate_count": gate_count', resume)
        self.assertNotIn("--version 0.3.10", resume)
        self.assertIn("git merge-base --is-ancestor", resume)
        self.assertIn("render_qualification_reports.py generate", resume)
        self.assertIn('"test_execution_repeated": False', resume)
        self.assertIn("actions/attest-build-provenance@", resume)
        self.assertIn("name: release-gate-evidence", resume)
        self.assertNotIn("scripts/release/gates.py run", resume)
        self.assertNotIn("cargo test", resume)

    def test_release_prepare_opens_the_pr_as_the_release_app(self) -> None:
        text = (ROOT / ".github/workflows/release-prepare.yml").read_text()

        # The App path is the default whenever both App settings exist: its
        # token checks out, pushes and opens the PR, and the PR's own PR Gate
        # starts by itself.
        self.assertIn(
            "APP_CONFIGURED: ${{ vars.RVOIP_RELEASE_APP_CLIENT_ID != '' && "
            "secrets.RVOIP_RELEASE_APP_PRIVATE_KEY != '' }}",
            text,
        )
        mint = text.split("- name: Mint the release App token", 1)[1].split("\n      - name:", 1)[0]
        self.assertIn("if: env.APP_CONFIGURED == 'true'", mint)
        self.assertIn("actions/create-github-app-token@", mint)
        self.assertIn("client-id: ${{ vars.RVOIP_RELEASE_APP_CLIENT_ID }}", mint)
        self.assertIn("private-key: ${{ secrets.RVOIP_RELEASE_APP_PRIVATE_KEY }}", mint)
        self.assertIn("token: ${{ steps.app-token.outputs.token || github.token }}", text)
        self.assertIn(
            "GH_TOKEN: ${{ steps.app-token.outputs.token || github.token }}",
            text.split("- name: Open the release pull request", 1)[1],
        )
        # The workflow-wide default stays read-only; only this job may write.
        self.assertIsNone(re.search(r"(?m)^  contents: write", text))
        self.assertIn("git merge-base --is-ancestor HEAD origin/main", text)
        self.assertLess(
            text.index("python3 scripts/release/notes.py"),
            text.index("python3 scripts/release.py prepare"),
        )

    def test_release_prepare_fallback_needs_a_maintainer_approval(self) -> None:
        text = (ROOT / ".github/workflows/release-prepare.yml").read_text()

        # Without the App, GITHUB_TOKEN opens the PR and GitHub holds the PR's
        # own PR Gate run for a maintainer; a dispatched copy does not unblock
        # the merge (0.4.0). The workflow warns, dispatches the gate as an early
        # signal, and links the held run. It never approves a run itself.
        warn = text.split("- name: Warn when the release App is not configured", 1)[1].split("\n      - name:", 1)[0]
        self.assertIn("if: env.APP_CONFIGURED != 'true'", warn)
        self.assertIn("::warning title=Prepare release PR fallback::", warn)
        fallback = text.split('- name: "Fallback: dispatch the PR Gate and link the held run to approve"', 1)[1]
        self.assertIn("if: env.APP_CONFIGURED != 'true'", fallback)
        self.assertIn('gh workflow run pr-gate.yml --repo "$REPOSITORY" --ref "$branch"', fallback)
        self.assertIn("event=pull_request", fallback)
        self.assertIn("Approve and run", fallback)
        self.assertIn('gh pr comment "$branch"', fallback)
        self.assertEqual(text.count("gh workflow run pr-gate.yml"), 1)
        for forbidden in ("/approve", "pending_deployments", "--admin"):
            with self.subTest(forbidden=forbidden):
                self.assertNotIn(forbidden, text)
        pr_gate = (ROOT / ".github/workflows/pr-gate.yml").read_text()
        self.assertIn("workflow_dispatch:", pr_gate)

    def test_pr_gate_checks_fragments_and_bump_notes_finality(self) -> None:
        text = (ROOT / ".github/workflows/pr-gate.yml").read_text()
        policy = text.split("\n  policy:\n", maxsplit=1)[1].split("\n  crate-tests:\n", maxsplit=1)[0]

        self.assertIn("python3 scripts/release/notes.py check", policy)
        self.assertIn('notes-final --if-version-changed-since "$base"', policy)

    def test_release_publish_needs_one_approval_and_packages_once(self) -> None:
        text = (ROOT / ".github/workflows/release-publish.yml").read_text()
        package = text.split("\n  package:\n", maxsplit=1)[1].split("\n  publish:\n", maxsplit=1)[0]
        publish = text.split("\n  publish:\n", maxsplit=1)[1]

        self.assertNotIn("environment:", package)
        self.assertNotIn("CARGO_REGISTRY_TOKEN", package)
        self.assertIn('python3 "$RELEASE_TOOL" package', package)
        self.assertIn("name: release-packages", package)
        self.assertIn("environment: release-publish", publish)
        self.assertIn("if: inputs.execute", publish)
        self.assertIn("needs: package", publish)
        self.assertIn("--packages target/release-packages", publish)
        self.assertIn("--execute", publish)
        # The protected job neither verifies nor builds again.
        self.assertNotIn('"$RELEASE_TOOL" verify', publish)
        self.assertNotIn("Swatinem/rust-cache", publish)
        self.assertNotIn("apt-get", publish)
        self.assertIn('test "$PACKAGED_CANDIDATE" = "$QUALIFIED_CANDIDATE"', publish)

    def test_release_publish_consumes_exact_attested_candidate(self) -> None:
        publication = (ROOT / ".github/workflows/release-publish.yml").read_text()
        preflight = (ROOT / "scripts/release/resolve_qualified_candidate.sh").read_text()
        self.assertEqual(
            publication.count("run: bash scripts/release/resolve_qualified_candidate.sh"), 2
        )
        self.assertIn("GH_TOKEN: ${{ github.token }}", publication)
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
