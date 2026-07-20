#!/usr/bin/env bash
# Reproduce the in-process 2,000-CPS PBX/media-server workload with one exact
# Cargo-produced test executable. Diagnostic profiles use the same workload and
# runtime recipe, but are deliberately not beta acceptance evidence.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/../../../.." && pwd)"
CRATE_DIR="${WORKSPACE_ROOT}/crates/sip/rvoip-sip"
PERF_DIR="${WORKSPACE_ROOT}/target/perf-results"
TEST_NAME="perf_call_setup_cps"
FEATURES="perf-tests"
REVIEWED_BASELINE="${CRATE_DIR}/beta-report/20260706T181609Z/perf-results"

usage() {
  cat <<'EOF'
Usage: perf_call_setup_2k_profile.sh <clean|cpu|timing|memory|boundary>

  clean   Unprofiled beta-comparable control (strict ASR gate).
  cpu     samply CPU profile; diagnostic, not acceptance evidence.
  timing  macOS Instruments Time Profiler trace; diagnostic only.
  memory  macOS Instruments Allocations trace plus four-phase retention timeline.
  boundary  Full shared-peer conditioning sweep with one retained-state snapshot
            at each calls-drained boundary; diagnostic only.

The clean workload conditions shared peers at 30, 100, 300, and 1,000 CPS,
then measures 2,000 CPS with a 5s ramp, 30s steady, 5s cooldown, and a
95-second post-drain cleanup sample. It uses eight Tokio workers, four Alice
endpoint shards, and the pbx-media-server recipe. Only clean is acceptance.

Useful profiler-only overrides:
  RVOIP_PERF_PROFILE_SAMPLY_RATE       samply Hz (default 1000)
  RVOIP_PERF_PROFILE_CSWITCH_MARKERS  1 to add samply context switches
  RVOIP_PERF_PROFILE_BUILD_ONLY       1 to build/resolve/manifest, then stop
EOF
}

MODE="${1:-}"
if [[ $# -ne 1 || ! "${MODE}" =~ ^(clean|cpu|timing|memory|boundary)$ ]]; then
  usage >&2
  exit 2
fi

command -v cargo >/dev/null 2>&1 || {
  echo "cargo is required" >&2
  exit 1
}
command -v python3 >/dev/null 2>&1 || {
  echo "python3 is required for exact Cargo artifact resolution" >&2
  exit 1
}

# A clean control must not inherit compiler/profile/allocator experiments from
# the caller. Reject them instead of silently clearing them so the invocation
# explains why it is not canonical.
noncanonical_build_env=()
while IFS= read -r name; do
  case "${name}" in
    RUSTFLAGS|CARGO_ENCODED_RUSTFLAGS|CARGO_BUILD_RUSTFLAGS|CARGO_INCREMENTAL|CARGO_PROFILE_RELEASE_*|CARGO_TARGET_*_RUSTFLAGS|MIMALLOC_*)
      noncanonical_build_env+=("${name}")
      ;;
  esac
done < <(compgen -e)
if (( ${#noncanonical_build_env[@]} > 0 )); then
  printf '[perf-2k] noncanonical build/allocator override is set: %s\n' \
    "${noncanonical_build_env[@]}" >&2
  echo "[perf-2k] clear these variables before producing beta evidence" >&2
  exit 2
fi

case "${MODE}" in
  clean)
    SCENARIO="perf_call_setup_cps_pbx-media-server"
    MIN_ASR="0.999"
    REQUIRE_ALL_POINTS="1"
    REQUIRE_ZERO_ERRORS="1"
    RETENTION_SNAPSHOT="0"
    BOUNDARY_SNAPSHOT="0"
    SWEEP_CPS="30,100,300,1000,2000"
    POST_DRAIN_SAMPLE_SECS="95"
    ;;
  cpu|timing)
    SCENARIO="perf_call_setup_cps_pbx-media-server_profile_${MODE}"
    MIN_ASR="0"
    REQUIRE_ALL_POINTS="0"
    REQUIRE_ZERO_ERRORS="0"
    RETENTION_SNAPSHOT="0"
    BOUNDARY_SNAPSHOT="0"
    SWEEP_CPS="2000"
    POST_DRAIN_SAMPLE_SECS="0"
    ;;
  memory)
    SCENARIO="perf_call_setup_cps_pbx-media-server_profile_memory"
    MIN_ASR="0"
    REQUIRE_ALL_POINTS="0"
    REQUIRE_ZERO_ERRORS="0"
    RETENTION_SNAPSHOT="1"
    BOUNDARY_SNAPSHOT="0"
    SWEEP_CPS="2000"
    POST_DRAIN_SAMPLE_SECS="0"
    ;;
  boundary)
    SCENARIO="perf_call_setup_cps_pbx-media-server_profile_boundary"
    MIN_ASR="0"
    REQUIRE_ALL_POINTS="1"
    REQUIRE_ZERO_ERRORS="1"
    RETENTION_SNAPSHOT="0"
    BOUNDARY_SNAPSHOT="1"
    SWEEP_CPS="30,100,300,1000,2000"
    POST_DRAIN_SAMPLE_SECS="0"
    FEATURES="perf-tests,perf-infra-memory-diagnostics"
    ;;
esac

if [[ "${MODE}" == "clean" && ! -f "${REVIEWED_BASELINE}/${SCENARIO}/2000.json" ]]; then
  echo "[perf-2k] reviewed baseline is missing ${SCENARIO}/2000.json: ${REVIEWED_BASELINE}" >&2
  exit 2
fi

RUN_STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
RUN_STARTED_EPOCH="$(date +%s)"
mkdir -p "${PERF_DIR}/profiles"
RUN_DIR="$(mktemp -d "${PERF_DIR}/profiles/${RUN_STAMP}_${MODE}_XXXXXX")"
OUTPUT_ROOT="${RUN_DIR}/output-target"
BUILD_MESSAGES="${RUN_DIR}/cargo-build.jsonl"
RUN_LOG="${RUN_DIR}/run.log"
SOURCE_AT_BUILD="${RUN_DIR}/source-at-build.json"
BUILD_ENVIRONMENT="${RUN_DIR}/build-environment.json"

# Canonical beta workload. Assign rather than default so a caller's shell does
# not silently turn this reproduction into a different experiment.
export CARGO_MANIFEST_DIR="${CRATE_DIR}"
export RVOIP_PERF_BUILD_FEATURES="${FEATURES}"
export RVOIP_PERF_RUN_MODE="${MODE}"
export RVOIP_PERF_OUTPUT_ROOT="${OUTPUT_ROOT}"
export RVOIP_PERF_SWEEP_CPS="${SWEEP_CPS}"
export RVOIP_PERF_RAMP_SECS="5"
export RVOIP_PERF_STEADY_SECS="30"
export RVOIP_PERF_COOLDOWN_SECS="5"
export RVOIP_PERF_CALL_TIMEOUT_SECS="15"
export RVOIP_PERF_WORKER_THREADS="8"
export RVOIP_PERF_PROFILE="pbx-media-server"
export RVOIP_PERF_CLIENT_PROFILE="endpoint"
export RVOIP_PERF_ALICE_SHARDS="4"
export RVOIP_PERF_SCHED_TICK_MS="1"
export RVOIP_PERF_REPORT_SCENARIO="${SCENARIO}"
export RVOIP_PERF_MIN_ASR="${MIN_ASR}"
export RVOIP_PERF_REQUIRE_ALL_POINTS="${REQUIRE_ALL_POINTS}"
export RVOIP_PERF_REQUIRE_ZERO_ERRORS="${REQUIRE_ZERO_ERRORS}"
export RVOIP_PERF_RETENTION_SNAPSHOT="${RETENTION_SNAPSHOT}"
export RVOIP_PERF_BOUNDARY_SNAPSHOT="${BOUNDARY_SNAPSHOT}"
export RVOIP_PERF_EMBED_RESOURCE_SAMPLES="0"
export RVOIP_PERF_RSS_TAIL_WINDOW_SECS="60"
export RVOIP_PERF_POST_DRAIN_SAMPLE_SECS="${POST_DRAIN_SAMPLE_SECS}"

# Rejected/diagnostic runtime switches stay off. The retention timeline is
# enabled only for memory diagnostics and is never part of a clean control.
export RVOIP_PERF_CALL_SETUP_DIAGNOSTICS="0"
export RVOIP_PERF_MEMORY_DIAGNOSTICS="0"
export RVOIP_PERF_ALLOCATOR_DIAGNOSTICS="0"
export RVOIP_PERF_SYSTEM_ALLOCATOR="0"
export RVOIP_PERF_DHAT="0"
export RVOIP_PERF_SKIP_AUDIO_FRAME_DELIVERY="0"
export RVOIP_MEDIA_AUDIO_TX_PACING="0"
export RVOIP_MEDIA_AUDIO_TX_SHARED_SCHEDULER="0"
export RVOIP_MEDIA_AUDIO_QUALITY_DIAGNOSTICS="0"
export RVOIP_MEDIA_DIAGNOSTICS="0"
export RVOIP_RTP_DIAGNOSTICS="0"
export RVOIP_SIP_DIAGNOSTICS="0"
export RVOIP_SRTP_DIAGNOSTICS="0"
unset RVOIP_PERF_TARGET_CPS
unset RVOIP_PERF_RECIPE_FILE
unset RVOIP_PERF_MAX_IN_FLIGHT
unset RVOIP_MEDIA_AUDIO_TX_PACING_TARGET_ACTIVE
unset RVOIP_MEDIA_AUDIO_TX_SHARED_BATCH_SIZE
unset RVOIP_TEST
unset RVOIP_TEST_TRANSACTION_TIMEOUT_MS

cd "${WORKSPACE_ROOT}"

# Record and validate the Cargo profile that produced the executable. The
# source fingerprint also covers Cargo.toml, while this structured block makes
# the effective optimization recipe directly inspectable in each artifact.
WORKSPACE_ROOT="${WORKSPACE_ROOT}" BUILD_ENVIRONMENT="${BUILD_ENVIRONMENT}" python3 <<'PY'
import json
import os
import pathlib
import tomllib

root = pathlib.Path(os.environ["WORKSPACE_ROOT"])
release = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["profile"]["release"]
expected = {
    "opt-level": 3,
    "lto": True,
    "codegen-units": 1,
    "panic": "abort",
    "debug": True,
    "strip": False,
}
if release != expected:
    raise SystemExit(
        f"canonical [profile.release] mismatch: actual={release!r} expected={expected!r}"
    )
features = os.environ["RVOIP_PERF_BUILD_FEATURES"].split(",")
snapshot = {
    "cargo_profile": "release",
    "cargo_release_profile": release,
    "cargo_features": features,
    "default_features": False,
    "allocator": "mimalloc",
    "rejected_override_patterns": [
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "CARGO_BUILD_RUSTFLAGS",
        "CARGO_INCREMENTAL",
        "CARGO_PROFILE_RELEASE_*",
        "CARGO_TARGET_*_RUSTFLAGS",
        "MIMALLOC_*",
    ],
}
pathlib.Path(os.environ["BUILD_ENVIRONMENT"]).write_text(
    json.dumps(snapshot, indent=2) + "\n", encoding="utf-8"
)
PY

# Capture source identity immediately before Cargo starts. The runtime report
# captures it again; a mismatch proves that the tree changed during the build
# or run and prevents an apparently precise comparison of different sources.
WORKSPACE_ROOT="${WORKSPACE_ROOT}" SOURCE_AT_BUILD="${SOURCE_AT_BUILD}" python3 <<'PY'
import hashlib
import json
import os
import pathlib
import subprocess

root = pathlib.Path(os.environ["WORKSPACE_ROOT"])

def git_bytes(*args):
    result = subprocess.run(
        ["git", *args], cwd=root, check=True, stdout=subprocess.PIPE
    )
    return result.stdout

def frame(digest, value):
    digest.update(len(value).to_bytes(8, "little"))
    digest.update(value)

try:
    commit = git_bytes("rev-parse", "HEAD").decode().strip()
    short = git_bytes("rev-parse", "--short", "HEAD").decode().strip()
    status = git_bytes(
        "status", "--porcelain=v1", "-z", "--untracked-files=all"
    )
    tracked_diff = git_bytes("diff", "--binary", "HEAD", "--", ".")
    untracked = sorted(
        part
        for part in git_bytes(
            "ls-files", "--others", "--exclude-standard", "-z"
        ).split(b"\0")
        if part
    )
    digest = hashlib.sha256(b"rvoip-source-fingerprint-v1\0")
    frame(digest, commit.encode())
    frame(digest, status)
    frame(digest, tracked_diff)
    for raw_path in untracked:
        frame(digest, raw_path)
        try:
            content = (root / os.fsdecode(raw_path)).read_bytes()
        except OSError as error:
            content = f"unreadable:{error.__class__.__name__}".encode()
        frame(digest, content)
    source = {
        "git_commit": commit,
        "git_rev": short,
        "git_dirty": bool(status),
        "source_fingerprint_sha256": digest.hexdigest(),
    }
except (OSError, subprocess.CalledProcessError, UnicodeError) as error:
    source = {
        "git_commit": "unknown",
        "git_rev": "unknown",
        "git_dirty": None,
        "source_fingerprint_sha256": "unknown",
        "error": str(error),
    }

pathlib.Path(os.environ["SOURCE_AT_BUILD"]).write_text(
    json.dumps(source, indent=2) + "\n", encoding="utf-8"
)
PY

echo "[perf-2k] building ${TEST_NAME} (${FEATURES}) and capturing Cargo JSON"
cargo test \
  -p rvoip-sip \
  --release \
  --no-default-features \
  --features "${FEATURES}" \
  --test "${TEST_NAME}" \
  --no-run \
  --message-format=json-render-diagnostics \
  >"${BUILD_MESSAGES}"

# Cargo's compiler-artifact message is the authority. Never select a hashed
# executable by mtime: stale feature variants can coexist in target/deps.
TEST_BIN="$(python3 - "${BUILD_MESSAGES}" "${TEST_NAME}" <<'PY'
import json
import os
import sys

messages_path, expected_name = sys.argv[1:]
candidates = []
with open(messages_path, "r", encoding="utf-8") as stream:
    for line in stream:
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        target = message.get("target") or {}
        executable = message.get("executable")
        if (
            message.get("reason") == "compiler-artifact"
            and target.get("name") == expected_name
            and "test" in (target.get("kind") or [])
            and executable
        ):
            candidates.append(executable)

unique = list(dict.fromkeys(candidates))
if len(unique) != 1:
    raise SystemExit(
        f"expected exactly one executable for {expected_name}, found {unique!r}"
    )
binary = os.path.realpath(unique[0])
if not (os.path.isfile(binary) and os.access(binary, os.X_OK)):
    raise SystemExit(f"Cargo artifact is not executable: {binary}")
print(binary)
PY
)"
printf '%s\n' "${TEST_BIN}" >"${RUN_DIR}/executable.txt"
echo "[perf-2k] exact executable: ${TEST_BIN}"

write_manifest() {
  local run_executed="$1"
  local test_exit_code="$2"
  local acceptance_status="$3"
  local audit_status="$4"
  local audit_exit_code="$5"
  local report_path="${RUN_DIR}/report.json"
  local audit_results_dir=""
  if [[ "${MODE}" == "clean" ]]; then
    audit_results_dir="${RUN_DIR}/perf-results"
  fi
  # Capture source identity again after the test, acceptance check, and audit.
  # The report's EnvironmentBlock intentionally caches provenance for all
  # points in one process, so it cannot detect a tree mutation that occurs
  # after the first conditioning point. This fresh process-boundary capture is
  # the authoritative end-of-run fence.
  TEST_BIN="${TEST_BIN}" \
  RUN_EXECUTED="${run_executed}" \
  TEST_EXIT_CODE="${test_exit_code}" \
  ACCEPTANCE_STATUS="${acceptance_status}" \
  AUDIT_STATUS="${audit_status}" \
  AUDIT_EXIT_CODE="${audit_exit_code}" \
  REPORT_PATH="${report_path}" \
  AUDIT_RESULTS_DIR="${audit_results_dir}" \
  RUN_DIR="${RUN_DIR}" \
  WORKSPACE_ROOT="${WORKSPACE_ROOT}" \
  MODE="${MODE}" \
  SCENARIO="${SCENARIO}" \
  FEATURES="${FEATURES}" \
  RUN_STARTED_EPOCH="${RUN_STARTED_EPOCH}" \
  SOURCE_AT_BUILD="${SOURCE_AT_BUILD}" \
  BUILD_ENVIRONMENT="${BUILD_ENVIRONMENT}" \
  REVIEWED_BASELINE="${REVIEWED_BASELINE}" \
    python3 <<'PY'
import datetime
import hashlib
import json
import os
import pathlib
import subprocess

binary = pathlib.Path(os.environ["TEST_BIN"])
report_path = pathlib.Path(os.environ["REPORT_PATH"])
run_executed = os.environ["RUN_EXECUTED"] == "true"
test_exit_code = (
    int(os.environ["TEST_EXIT_CODE"])
    if os.environ["TEST_EXIT_CODE"]
    else None
)
report = None
report_error = None
if report_path.is_file():
    try:
        with report_path.open("r", encoding="utf-8") as stream:
            report = json.load(stream)
    except (OSError, ValueError) as error:
        report_error = str(error)
source_at_build = json.loads(pathlib.Path(os.environ["SOURCE_AT_BUILD"]).read_text())
build_environment = json.loads(pathlib.Path(os.environ["BUILD_ENVIRONMENT"]).read_text())
runtime_environment = (report or {}).get("environment") or {}
effective_config = ((report or {}).get("diagnostics") or {}).get("effective_config")
phase_markers = ((report or {}).get("diagnostics") or {}).get("phase_markers")

def capture_source_provenance(root):
    def git_bytes(*args):
        result = subprocess.run(
            ["git", *args], cwd=root, check=True, stdout=subprocess.PIPE
        )
        return result.stdout

    def frame(digest, value):
        digest.update(len(value).to_bytes(8, "little"))
        digest.update(value)

    try:
        commit = git_bytes("rev-parse", "HEAD").decode().strip()
        short = git_bytes("rev-parse", "--short", "HEAD").decode().strip()
        status = git_bytes(
            "status", "--porcelain=v1", "-z", "--untracked-files=all"
        )
        tracked_diff = git_bytes("diff", "--binary", "HEAD", "--", ".")
        untracked = sorted(
            part
            for part in git_bytes(
                "ls-files", "--others", "--exclude-standard", "-z"
            ).split(b"\0")
            if part
        )
        digest = hashlib.sha256(b"rvoip-source-fingerprint-v1\0")
        frame(digest, commit.encode())
        frame(digest, status)
        frame(digest, tracked_diff)
        for raw_path in untracked:
            frame(digest, raw_path)
            try:
                content = (root / os.fsdecode(raw_path)).read_bytes()
            except OSError as error:
                content = f"unreadable:{error.__class__.__name__}".encode()
            frame(digest, content)
        return {
            "git_commit": commit,
            "git_rev": short,
            "git_dirty": bool(status),
            "source_fingerprint_sha256": digest.hexdigest(),
        }
    except (OSError, subprocess.CalledProcessError, UnicodeError) as error:
        return {
            "git_commit": "unknown",
            "git_rev": "unknown",
            "git_dirty": None,
            "source_fingerprint_sha256": "unknown",
            "error": str(error),
        }

source_at_finalize = capture_source_provenance(pathlib.Path(os.environ["WORKSPACE_ROOT"]))
finalize_path = pathlib.Path(os.environ["RUN_DIR"]) / "source-at-finalize.json"
finalize_path.write_text(
    json.dumps(source_at_finalize, indent=2) + "\n", encoding="utf-8"
)

def valid_source_fingerprint(value):
    return (
        isinstance(value, str)
        and len(value) == 64
        and all(character in "0123456789abcdef" for character in value)
    )

build_fingerprint = source_at_build.get("source_fingerprint_sha256")
runtime_fingerprint = runtime_environment.get("source_fingerprint_sha256")
finalize_fingerprint = source_at_finalize.get("source_fingerprint_sha256")
source_matches_runtime = (
    valid_source_fingerprint(build_fingerprint)
    and valid_source_fingerprint(runtime_fingerprint)
    and build_fingerprint == runtime_fingerprint
    if runtime_environment
    else None
)
source_matches_finalize = (
    valid_source_fingerprint(build_fingerprint)
    and valid_source_fingerprint(finalize_fingerprint)
    and build_fingerprint == finalize_fingerprint
)
source_matches = source_matches_runtime is True and source_matches_finalize is True

if not run_executed:
    report_status = "NOT_EXPECTED"
    overall_status = "BUILD_ONLY"
elif report_error:
    report_status = "INVALID"
    overall_status = "FAIL"
elif report is None:
    report_status = "MISSING"
    overall_status = "FAIL"
else:
    report_status = "CAPTURED"
    report_complete = (
        report.get("scenario") == os.environ["SCENARIO"]
        and bool(runtime_environment)
        and effective_config is not None
        and bool(phase_markers)
    )
    acceptance_ok = (
        os.environ["MODE"] != "clean"
        or os.environ["ACCEPTANCE_STATUS"] == "PASS"
    )
    audit_ok = (
        os.environ["MODE"] != "clean"
        or os.environ["AUDIT_STATUS"] == "PASS"
    )
    overall_status = (
        "PASS"
        if test_exit_code == 0
        and report_complete
        and source_matches
        and acceptance_ok
        and audit_ok
        else "FAIL"
    )

digest = hashlib.sha256()
with binary.open("rb") as stream:
    for block in iter(lambda: stream.read(1024 * 1024), b""):
        digest.update(block)

manifest = {
    "schema": "rvoip-perf-profile-manifest-v2",
    "captured_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    "mode": os.environ["MODE"],
    "scenario": os.environ["SCENARIO"],
    "status": 0 if overall_status in {"PASS", "BUILD_ONLY"} else 1,
    "overall_status": overall_status,
    "run_executed": run_executed,
    "test_exit_code": test_exit_code,
    "report_status": report_status,
    "report_error": report_error,
    "acceptance_status": os.environ["ACCEPTANCE_STATUS"],
    "perf_audit_status": os.environ["AUDIT_STATUS"],
    "perf_audit_exit_code": (
        int(os.environ["AUDIT_EXIT_CODE"])
        if os.environ["AUDIT_EXIT_CODE"]
        else None
    ),
    "workspace_root": os.environ["WORKSPACE_ROOT"],
    "cargo_features_requested": os.environ["FEATURES"].split(","),
    "allocator_expected": "mimalloc",
    "build_environment": build_environment,
    "source_at_build": source_at_build,
    "source_at_finalize": source_at_finalize,
    "executable": str(binary),
    "executable_sha256": digest.hexdigest(),
    "report_path": str(report_path) if report is not None else None,
    "acceptance_path": (
        str(pathlib.Path(os.environ["RUN_DIR"]) / "acceptance.json")
        if (pathlib.Path(os.environ["RUN_DIR"]) / "acceptance.json").is_file()
        else None
    ),
    "audit_results_dir": (
        os.environ["AUDIT_RESULTS_DIR"]
        if report is not None and os.environ["AUDIT_RESULTS_DIR"]
        else None
    ),
    "reviewed_baseline_path": (
        os.environ["REVIEWED_BASELINE"] if os.environ["MODE"] == "clean" else None
    ),
    "perf_audit_path": (
        str(pathlib.Path(os.environ["RUN_DIR"]) / "perf-audit.md")
        if (pathlib.Path(os.environ["RUN_DIR"]) / "perf-audit.md").is_file()
        else None
    ),
    "environment": runtime_environment or None,
    "source_fingerprint_matches_runtime": source_matches_runtime,
    "source_fingerprint_matches_finalize": source_matches_finalize,
    "source_fingerprint_unchanged_for_full_run": source_matches,
    "effective_config": effective_config,
    "phase_markers": phase_markers,
}
out = pathlib.Path(os.environ["RUN_DIR"]) / "manifest.json"
out.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
raise SystemExit(1 if overall_status == "FAIL" else 0)
PY
}

if [[ "${RVOIP_PERF_PROFILE_BUILD_ONLY:-0}" == "1" ]]; then
  write_manifest false "" NOT_APPLICABLE NOT_APPLICABLE ""
  echo "[perf-2k] build-only complete: ${RUN_DIR}"
  exit 0
fi

test_args=("${TEST_NAME}" --exact --nocapture)
status=0
set +e
case "${MODE}" in
  clean|boundary)
    "${TEST_BIN}" "${test_args[@]}" 2>&1 | tee "${RUN_LOG}"
    status=${PIPESTATUS[0]}
    ;;
  cpu)
    command -v samply >/dev/null 2>&1 || {
      echo "samply is required for cpu mode" >&2
      exit 1
    }
    samply_args=(
      record
      --save-only
      --unstable-presymbolicate
      --rate "${RVOIP_PERF_PROFILE_SAMPLY_RATE:-1000}"
      --profile-name "rvoip-sip-2k-cps"
      --output "${RUN_DIR}/cpu-profile.json.gz"
    )
    if [[ "${RVOIP_PERF_PROFILE_CSWITCH_MARKERS:-0}" == "1" ]]; then
      samply_args+=(--cswitch-markers)
    fi
    samply "${samply_args[@]}" "${TEST_BIN}" "${test_args[@]}" \
      2>&1 | tee "${RUN_LOG}"
    status=${PIPESTATUS[0]}
    ;;
  timing|memory)
    command -v xctrace >/dev/null 2>&1 || {
      echo "xctrace is required for ${MODE} mode (macOS/Xcode)" >&2
      exit 1
    }
    if [[ "${MODE}" == "timing" ]]; then
      template="Time Profiler"
      trace_path="${RUN_DIR}/time-profiler.trace"
    else
      template="Allocations"
      trace_path="${RUN_DIR}/allocations.trace"
    fi
    xctrace record \
      --quiet \
      --no-prompt \
      --template "${template}" \
      --output "${trace_path}" \
      --target-stdout - \
      --launch -- "${TEST_BIN}" "${test_args[@]}" \
      2>&1 | tee "${RUN_LOG}"
    status=${PIPESTATUS[0]}
    ;;
esac
set -e

test_status="${status}"
acceptance_status="NOT_APPLICABLE"
audit_status="NOT_APPLICABLE"
audit_exit_code=""
if [[ "${MODE}" == "clean" || "${MODE}" == "boundary" ]]; then
  raw_report_path="${OUTPUT_ROOT}/perf-results/${SCENARIO}/2000.json"
else
  raw_report_path="${OUTPUT_ROOT}/perf-results/${SCENARIO}.json"
fi
captured_report_path="${RUN_DIR}/report.json"
if [[ -f "${raw_report_path}" ]]; then
  cp "${raw_report_path}" "${captured_report_path}"
  if [[ "${MODE}" == "clean" ]]; then
    # The reviewed baseline is a multi-point sweep whose 2,000-CPS result lives
    # at <scenario>/2000.json. Stage this single-point result under that exact
    # relative path so perf_audit never reads a stale workspace-wide result.
    audit_results_dir="${RUN_DIR}/perf-results"
    audit_report_dir="${audit_results_dir}/${SCENARIO}"
    mkdir -p "${audit_report_dir}"
    cp "${captured_report_path}" "${audit_report_dir}/2000.json"
    printf '%s\n' "${audit_results_dir}" >"${RUN_DIR}/audit-results-dir.txt"

    # Enforce the reviewed beta thresholds directly. The relative baseline
    # audit remains useful, but cannot substitute for these absolute release
    # limits or the zero-error requirement.
    if python3 "${SCRIPT_DIR}/perf_2k_acceptance.py" \
      --report "${captured_report_path}" \
      --out "${RUN_DIR}/acceptance.json" \
      --scenario "${SCENARIO}"
    then
      acceptance_status="PASS"
    else
      acceptance_status="FAIL"
      status=1
    fi

    if python3 "${SCRIPT_DIR}/perf_audit.py" \
      --baseline "${REVIEWED_BASELINE}" \
      --current "${audit_results_dir}" \
      --out "${RUN_DIR}/perf-audit.md" \
      --fail-on-regression
    then
      audit_status="PASS"
      audit_exit_code="0"
    else
      audit_exit_code="$?"
      audit_status="FAIL"
      status=1
    fi
  fi
else
  echo "[perf-2k] run did not produce a report at ${raw_report_path}" >&2
  status=1
fi
if ! write_manifest true "${test_status}" "${acceptance_status}" "${audit_status}" "${audit_exit_code}"; then
  status=1
fi

echo "[perf-2k] mode=${MODE} status=${status} artifacts=${RUN_DIR}"
exit "${status}"
