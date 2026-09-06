#!/usr/bin/env bash
# rvoip-sip beta-candidate release gate.
#
# This script is intentionally release-gate-first: it records deterministic
# commands and artifacts even when an external lab dependency is unavailable.
# Missing external prerequisites are reported as SKIP by default. Set
# BETA_GATE_REQUIRE_EXTERNAL=1 to make skipped external gates fail the run.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRATE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
# crates/sip/rvoip-sip -> repo root is three levels up (post directory reorg).
WORKSPACE_ROOT="$(cd "$CRATE_DIR/../../.." && pwd)"

# Local PBX interop runs use Docker through Colima on macOS. Homebrew installs
# those CLIs outside the minimal PATH that some CI/desktop shells provide.
export PATH="/opt/homebrew/opt/docker/bin:/opt/homebrew/opt/docker-compose/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"

if [ "${BETA_DENY_WARNINGS:-1}" != "0" ]; then
  export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-D warnings"
  export RUSTDOCFLAGS="${RUSTDOCFLAGS:+$RUSTDOCFLAGS }-D warnings"
fi
export RUST_LOG="${BETA_TEST_LOG_FILTER:-off}"

MODE="${BETA_GATE_MODE:-local}"
REQUIRE_EXTERNAL="${BETA_GATE_REQUIRE_EXTERNAL:-0}"
BETA_FUZZ_TOOLCHAIN="${BETA_FUZZ_TOOLCHAIN:-nightly}"
TIMESTAMP="$(date -u +%Y%m%dT%H%M%SZ)"
STARTED_AT_UTC="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
ARTIFACT_DIR="${BETA_GATE_ARTIFACT_DIR:-$WORKSPACE_ROOT/target/beta-gate/$TIMESTAMP}"
SUMMARY="$ARTIFACT_DIR/summary.md"
ENV_REPORT="$ARTIFACT_DIR/environment/environment.md"
BETA_SOURCE_AT_START="$ARTIFACT_DIR/environment/source-at-beta-start.json"
BETA_SOURCE_AT_END="$ARTIFACT_DIR/environment/source-at-beta-end.json"
CANONICAL_2K_EVIDENCE_HELPER="$SCRIPT_DIR/canonical_2k_evidence.py"
BETA_ATTESTATION_HELPER="$SCRIPT_DIR/beta_attestation.py"
BETA_RELEASE_REPORT_HELPER="$SCRIPT_DIR/beta_release_report.py"
BETA_RELEASE_POLICY="$CRATE_DIR/config/beta-release-policy.yaml"
DOCKER_PEER_SNAPSHOT_HELPER="$SCRIPT_DIR/docker_peer_snapshot.py"
PERF_REGRESSION_BASELINE_HELPER="$SCRIPT_DIR/perf_regression_baseline.py"
PROXY_INTEROP_BETA_GATE="$WORKSPACE_ROOT/crates/sip/sip-proxy/tests/interop/scripts/beta_gate.sh"
PERF_RESULTS_DIR="$WORKSPACE_ROOT/target/perf-results"
PERF_RESULTS_ARCHIVE_ROOT="$WORKSPACE_ROOT/target/perf-results-archive"
PERF_RESULTS_CAPTURE_MARKER="$ARTIFACT_DIR/environment/perf-results-capture.md"
EFFECTIVE_GATE_CONFIG="$ARTIFACT_DIR/effective-gate-config.json"
GATE_RESULTS_DIR="$ARTIFACT_DIR/gate-results.d"
GATE_RESULTS="$ARTIFACT_DIR/gate-results.json"
ENDED_AT_UTC=""
FAILURES=0
SKIPS=0
GATE_SEQUENCE=0
SIPP_LISTENER_PID=""
PBX_RESTORE_ARMED=0
PBX_RESTORE_ENABLED=0
PBX_RESTORE_INITIAL_ASTERISK=0
PBX_RESTORE_INITIAL_FREESWITCH=0
PBX_RESTORE_ASTERISK_DIR=""
PBX_RESTORE_FREESWITCH_DIR=""

cleanup_background() {
  if [ -n "$SIPP_LISTENER_PID" ] && kill -0 "$SIPP_LISTENER_PID" >/dev/null 2>&1; then
    kill -INT "$SIPP_LISTENER_PID" >/dev/null 2>&1 || true
    wait "$SIPP_LISTENER_PID" >/dev/null 2>&1 || true
  fi
}

cleanup_local_pbx_state() {
  if [ "$PBX_RESTORE_ARMED" != "1" ]; then
    return
  fi
  # Disarm first so a failure or signal during restoration cannot recurse.
  PBX_RESTORE_ARMED=0
  if [ "$PBX_RESTORE_ENABLED" != "1" ]; then
    return
  fi

  if [ "$PBX_RESTORE_INITIAL_ASTERISK" = "1" ]; then
    "$PBX_RESTORE_FREESWITCH_DIR/scripts/down.sh" >/dev/null 2>&1 || true
    "$PBX_RESTORE_ASTERISK_DIR/scripts/up.sh" >/dev/null 2>&1 || true
  elif [ "$PBX_RESTORE_INITIAL_FREESWITCH" = "1" ]; then
    "$PBX_RESTORE_ASTERISK_DIR/scripts/down.sh" >/dev/null 2>&1 || true
    "$PBX_RESTORE_FREESWITCH_DIR/scripts/up.sh" >/dev/null 2>&1 || true
  else
    "$PBX_RESTORE_ASTERISK_DIR/scripts/down.sh" >/dev/null 2>&1 || true
    "$PBX_RESTORE_FREESWITCH_DIR/scripts/down.sh" >/dev/null 2>&1 || true
  fi
}

cleanup_on_exit() {
  local status=$?
  cleanup_background
  cleanup_local_pbx_state
  return "$status"
}
trap cleanup_on_exit EXIT

usage() {
  cat <<'EOF'
Usage: beta_gate.sh [--local|--full|--interop|--perf|--security] [--require-external]

Modes:
  --local    Fast local gate: format/check/tests/docs/examples/compliance smoke.
  --full     Local gate plus interop and perf gates.
  --interop  External interop gates only.
  --perf     Performance gates only.
  --security Dependency audit and parser fuzz-smoke gates only.

Environment:
  BETA_GATE_ARTIFACT_DIR         Output directory. Defaults to target/beta-gate/<timestamp>.
  BETA_REPORT_DIR                Crate-local report directory. Defaults to crates/sip/rvoip-sip/beta-report.
  BETA_REPORT_PACKAGE=0          Disable copying completed artifacts into BETA_REPORT_DIR.
                                  The raw artifact directory still receives an attestation.
  BETA_GATE_REQUIRE_EXTERNAL=1   Treat skipped external gates as failures.
  BETA_DENY_WARNINGS=0           Allow Rust warnings during beta gates. Defaults to 1.
  BETA_TEST_LOG_FILTER           Runtime tracing filter for cargo test/build gates.
                                  Defaults to off for clean release evidence.
  RVOIP_REQUIRE_API_TOOLS=1      Require the pinned cargo-public-api and
                                  cargo-semver-checks tools. Full mode defaults
                                  to 1; development modes default to 0 while
                                  still running the compiler fixture.
  BETA_REQUIRE_CLEAN_SOURCE=0    Allow a dirty or changing source fingerprint for a full gate.
                                  Full gates require clean, unchanged source by default; other modes do not.
  BETA_REQUIRE_CANONICAL_2K_EVIDENCE=1
                                  Require exactly three pre-run canonical clean PASS artifacts.
                                  Defaults to 0 for development gates.
  BETA_CANONICAL_2K_RUN_DIRS     Three chronological run directories separated by `:`. Required
                                  when canonical evidence is enabled; paths are copied into the
                                  beta report after fingerprint and gate revalidation.
  BETA_ATTESTATION_FEATURES      Additional comma-separated Cargo features to record.
  BETA_ATTESTATION_TARGET        Explicit Cargo target triple to attest. Defaults to
                                  CARGO_BUILD_TARGET, then the captured rustc host.
  BETA_STATE_TABLE_SOURCE        Selected runtime YAML source: embedded-default,
                                  configured-path, or configured-path-fallback.
                                  Defaults to embedded-default.
  BETA_STATE_TABLE_SELECTED_YAML Exact YAML file selected by the tested runtime. Required for
                                  configured-path; defaults to state_tables/default.yaml only
                                  for embedded-default/configured-path-fallback.
  BETA_STATE_TABLE_FALLBACK_REASON
                                  Bounded fallback reason: read-failed, decode-failed,
                                  load-failed, or validation-failed. Required only for fallback.
  BETA_REQUIRE_CONFIGURED_STATE_TABLE_EVIDENCE=1
                                  Fail closed unless configured-path and an explicit selected
                                  YAML file are supplied for attestation.
  BETA_RUN_PBX=1                 Run examples/pbx/run.sh when PBX configs are present.
  BETA_RUN_LOCAL_PBX=1           Manage ~/Developer/asterisk and ~/Developer/freeswitch sequentially.
  BETA_RUN_PROXY_PBX=1           Run the Kamailio/OpenSIPS+rtpengine labs (infra/release-runners/pbx). Default: skip.
  BETA_RESTORE_LOCAL_PBX=0       Do not restore the PBX container that was running before the gate.
  BETA_PBX_API                   PBX API subset: endpoint|stream_peer|callback|all. Defaults to all.
  BETA_PBX_SCENARIO              PBX scenario subset. Defaults to all.
  BETA_PBX_PROVIDER              PBX provider subset: asterisk|freeswitch|both. Defaults to both.
  BETA_PBX_G729_PROFILES         G.729 PBX profiles. Defaults to "g729a g729ab".
  BETA_ASTERISK_DIR              Local Asterisk checkout. Defaults to ~/Developer/asterisk.
  BETA_FREESWITCH_DIR            Local FreeSWITCH checkout. Defaults to ~/Developer/freeswitch.
  BETA_PBX_LOG_TAIL              Docker log lines captured around PBX lifecycle events. Defaults to 1000.
  BETA_CAPTURE_DOCKER_LOGS=0     Disable local PBX Docker inspect/log snapshots.
  BETA_RUN_SIPP=1                Run SIPp. Defaults to a managed local rvoip target.
  BETA_SIPP_TARGET_HOST          SIPp target host. Defaults to managed local rvoip target.
  BETA_SIPP_TARGET_PORT          SIPp target port. Defaults to 35060 for managed target.
  BETA_SIPP_CPS                  CPS list for standalone SIPp gate.
  BETA_SIPP_PERF_PROFILE         Managed SIPp target recipe. Defaults to pbx-media-server.
  BETA_SIPP_DIAGNOSTICS=1        Enable managed SIPp target diagnostics. Defaults to 0 for
                                  release latency measurements.
  BETA_PERF_PROFILE_MATRIX       Perf profile:CPS matrix. Defaults to endpoint, pbx-media-server,
                                  and signaling-only-server-high-performance.
  BETA_RUN_PERF_ALL=1            Run every registered perf/resiliency test, including ignored
                                  media-churn and monolithic-soak tests. Requires the full burst
                                  matrix and split soak to be enabled so paired ignored tests run.
                                  The untuned endpoint profile remains a 30-CPS compatibility gate;
                                  high-CPS tiers are qualified by the server profiles.
  BETA_PERF_MEDIA_CHURN_DURATION_SECS
                                  Isolated media-churn duration. Defaults to 120 seconds.
  BETA_PERF_MEDIA_CHURN_ACTIVE_CALLS
                                  Isolated media-churn active-call target. Defaults to 30.
  BETA_PERF_MONOLITHIC_SOAK_DURATION_SECS
                                  Monolithic-soak duration. Defaults to 3600 seconds so the
                                  final ten-minute RSS gate follows allocator warm-up.
  BETA_PERF_MONOLITHIC_SOAK_ACTIVE_CALLS
                                  Monolithic-soak active-call target. Defaults to 30.
  BETA_PERFORMANCE_RECIPE_FILE   Optional YAML recipe book path.
  BETA_PERF_INFRA_MEMORY_DIAGNOSTICS=1
                                  Compile SIP/infra memory diagnostics for perf gates.
  BETA_PERF_MEDIA_DIAGNOSTICS=1  Compile media setup/audio-quality diagnostics for perf gates.
  BETA_PERF_MEDIA_MEMORY_DIAGNOSTICS=1
                                  Compile media-core memory diagnostics for perf gates.
  BETA_PERF_RTP_MEMORY_DIAGNOSTICS=1
                                  Compile RTP-core memory diagnostics for perf gates.
  BETA_RUN_BURST_SMOKE=0         Disable required short media burst smoke.
  BETA_RUN_BURST_MATRIX=1        Run full opt-in media burst scenario matrix.
  BETA_BURST_SCENARIO_FILE       Burst scenario YAML. Defaults to config/perf-burst-scenarios.yaml.
  BETA_BURST_MATRIX              Burst scenario list for full matrix, or "all".
  RVOIP_PERF_MIN_SUCCESS_PCT     SIPp pass threshold. Defaults to 99.9.
  BETA_RUN_STRICT_UA=0           Disable the baresip strict-UA gate; fails with --require-external.
  PROXY_INTEROP_HOST_ADDRESS     Host address reachable from the pinned proxy-peer containers.
                                  The full and interop gates always run both Kamailio and
                                  OpenSIPS, in both adjacency orders, over UDP, TCP, and
                                  verified TLS. Missing Docker, SIPp, packet capture, a peer,
                                  an order, or a required transport is a hard failure.
  BETA_RUN_LONG_SOAK=0           Disable the ignored soak test; fails with --require-external.
  BETA_PERF_REGRESSION_FAIL=1    Make a regression vs the reviewed immutable baseline a hard gate failure. Default 0 (report-only + perf-audit.md).
  BETA_PERF_REGRESSION_BASELINE_ROOT
                                  Reviewed immutable regression-baseline root. Defaults to
                                  perf-baselines/20260706T181609Z in this crate.
  BETA_PERF_REGRESSION_BASELINE_MANIFEST
                                  Manifest for the reviewed regression baseline. Defaults to
                                  <baseline-root>/manifest.json. The manifest and every listed
                                  result are verified and packaged before comparison.
  BETA_PERF_REGRESSION_TOLERANCE_PCT  Throughput/RSS regression tolerance (percent). Defaults to 15.
  BETA_PERF_LATENCY_TOLERANCE_PCT     Latency p50/p95/p99 regression tolerance (percent). Defaults to 25.
  BETA_RUN_FUZZ_SMOKE=0          Disable parser fuzz-smoke coverage; fails with --require-external.
  BETA_FUZZ_TOOLCHAIN            Rust toolchain used by cargo-fuzz. Defaults to nightly.
  BETA_FUZZ_SMOKE_RUNS           libFuzzer runs per parser target. Defaults to 1000.
  BETA_FUZZ_SMOKE_SECONDS        libFuzzer max_total_time per parser target. Defaults to 10.
  RVOIP_PERF_SOAK_DURATION_SECS  Soak duration. Defaults to 3600 in the beta gate.
  RVOIP_PERF_SOAK_ACTIVE_CALLS   Cycling active/media calls. Defaults to 500 in the beta gate.
  RVOIP_PERF_SOAK_MIN_HOLD_SECS  Minimum cycling active-call hold. Defaults to 10.
  RVOIP_PERF_SOAK_MAX_HOLD_SECS  Maximum cycling active-call hold. Defaults to 360.
  RVOIP_PERF_SOAK_CPS            Optional immediate hangup churn. Defaults to 0.
  RVOIP_PERF_SOAK_DRAIN_CPS      Paced monolithic-soak teardown rate. Defaults to 10.
  RVOIP_PERF_SOAK_ERROR_SAMPLE_LIMIT
                                  Bounded structured failure samples. Defaults to 32.
  RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS
                                  Monolithic post-drain retention/RSS window. Defaults to 120
                                  in the beta gate (the direct-test default is 40).
  RVOIP_PERF_MASS_TEARDOWN_CALLS Simultaneous teardown stress call count. Defaults to 500.
  RVOIP_PERF_MASS_TEARDOWN_SETUP_CPS
                                  Setup rate for mass teardown stress. Defaults to 30.
  RVOIP_PERF_MEMORY_DIAGNOSTICS  Write memory diagnostic JSONL during soak. Defaults to 0.
  RVOIP_PERF_ALLOCATOR_DIAGNOSTICS
                                  Include mimalloc snapshots in memory diagnostics. Defaults to 0.
  RVOIP_PERF_MEMORY_DIAG_INTERVAL_SECS
                                  Memory diagnostic interval. Defaults to 5.
  RVOIP_PERF_MIMALLOC_COLLECT_AT Optional diagnostic mi_collect(true): off|phase|drain|both|settled|all.
                                  Defaults to off in the beta gate.
  RVOIP_PERF_SYSTEM_ALLOCATOR=1  Build perf soak with the system allocator instead of mimalloc.
  RVOIP_PERF_DHAT=1              Build split soak with DHAT heap profiling allocator.
  RVOIP_PERF_HEAP_SNAPSHOTS=1    Capture per-process vmmap snapshots during split soak.
  RVOIP_PERF_HEAP_SNAPSHOT_SECS  Optional comma list of label:seconds or seconds snapshot offsets.
  RVOIP_PERF_MALLOC_STACK_LOGGING=1
                                  Enable macOS MallocStackLogging for child soak processes.
  RVOIP_PERF_LEAKS_SNAPSHOTS=1   Also run macOS leaks at heap snapshot offsets.
  RVOIP_PERF_SKIP_AUDIO_FRAME_DELIVERY=1
                                  Diagnostic-only: decode RTP media but skip app-facing
                                  AudioFrame delivery. Full release qualification requires 0.
  RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR
                                  Beta RSS growth threshold. Defaults to 15 MB/hr.
  RVOIP_PERF_APP_EVENT_CHANNEL_CAPACITY
                                  App-facing event buffer capacity for perf soaks.
                                  Defaults to Config's recipe value.
  RVOIP_PERF_RSS_TAIL_WINDOW_SECS
                                  Sustained RSS slope window. Defaults to 60.
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --local) MODE=local ;;
    --full) MODE=full ;;
    --interop) MODE=interop ;;
    --perf) MODE=perf ;;
    --security) MODE=security ;;
    --require-external) REQUIRE_EXTERNAL=1 ;;
    --help|-h) usage; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

if [ -z "${BETA_REQUIRE_CLEAN_SOURCE+x}" ]; then
  if [ "$MODE" = "full" ]; then
    BETA_REQUIRE_CLEAN_SOURCE=1
  else
    BETA_REQUIRE_CLEAN_SOURCE=0
  fi
fi
export BETA_REQUIRE_CLEAN_SOURCE

# A full release gate always executes the pinned structural and semantic API
# checks. Development modes retain the opt-in default so contributors can run
# compiler-only diagnostics without installing release tooling.
if [ -z "${RVOIP_REQUIRE_API_TOOLS+x}" ]; then
  if [ "$MODE" = "full" ]; then
    RVOIP_REQUIRE_API_TOOLS=1
  else
    RVOIP_REQUIRE_API_TOOLS=0
  fi
fi
case "$RVOIP_REQUIRE_API_TOOLS" in
  0|1) ;;
  *) echo "RVOIP_REQUIRE_API_TOOLS must be 0 or 1" >&2; exit 2 ;;
esac
export RVOIP_REQUIRE_API_TOOLS

# Use one effective retention fence for every beta performance target. Without
# this export, the monolithic soak received 120 seconds while mass teardown and
# session churn silently fell back to their direct-test default of 40 seconds.
RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS="${RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS:-130}"
export RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS

PERF_REGRESSION_BASELINE_ROOT="${BETA_PERF_REGRESSION_BASELINE_ROOT:-$CRATE_DIR/perf-baselines/20260706T181609Z}"
case "$PERF_REGRESSION_BASELINE_ROOT" in
  /*) ;;
  *) PERF_REGRESSION_BASELINE_ROOT="$WORKSPACE_ROOT/$PERF_REGRESSION_BASELINE_ROOT" ;;
esac
PERF_REGRESSION_BASELINE_MANIFEST="${BETA_PERF_REGRESSION_BASELINE_MANIFEST:-$PERF_REGRESSION_BASELINE_ROOT/manifest.json}"
case "$PERF_REGRESSION_BASELINE_MANIFEST" in
  /*) ;;
  *) PERF_REGRESSION_BASELINE_MANIFEST="$WORKSPACE_ROOT/$PERF_REGRESSION_BASELINE_MANIFEST" ;;
esac
if [ -f "$PERF_REGRESSION_BASELINE_MANIFEST" ] && [ ! -L "$PERF_REGRESSION_BASELINE_MANIFEST" ]; then
  perf_regression_baseline_manifest_sha256="$(python3 - "$PERF_REGRESSION_BASELINE_MANIFEST" <<'PY'
import hashlib
import pathlib
import sys

print(hashlib.sha256(pathlib.Path(sys.argv[1]).read_bytes()).hexdigest())
PY
)"
  perf_regression_baseline_id="$(python3 - "$PERF_REGRESSION_BASELINE_MANIFEST" <<'PY'
import json
import pathlib
import sys

try:
    value = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
except (OSError, ValueError):
    value = {}
print(value.get("baseline_id", "invalid"))
PY
)"
else
  perf_regression_baseline_manifest_sha256=unavailable
  perf_regression_baseline_id=unavailable
fi

BETA_STATE_TABLE_SOURCE="${BETA_STATE_TABLE_SOURCE:-embedded-default}"
BETA_STATE_TABLE_FALLBACK_REASON="${BETA_STATE_TABLE_FALLBACK_REASON:-}"
state_table_yaml_was_explicit=0
if [ -n "${BETA_STATE_TABLE_SELECTED_YAML:-}" ]; then
  state_table_yaml_was_explicit=1
  selected_state_table_yaml="$BETA_STATE_TABLE_SELECTED_YAML"
else
  selected_state_table_yaml="$CRATE_DIR/state_tables/default.yaml"
fi
case "$selected_state_table_yaml" in
  /*) ;;
  *) selected_state_table_yaml="$WORKSPACE_ROOT/$selected_state_table_yaml" ;;
esac

case "$BETA_STATE_TABLE_SOURCE" in
  embedded-default)
    if [ -n "$BETA_STATE_TABLE_FALLBACK_REASON" ]; then
      echo "BETA_STATE_TABLE_FALLBACK_REASON is only valid for configured-path-fallback" >&2
      exit 2
    fi
    ;;
  configured-path)
    if [ "$state_table_yaml_was_explicit" != "1" ]; then
      echo "configured-path requires BETA_STATE_TABLE_SELECTED_YAML" >&2
      exit 2
    fi
    if [ -n "$BETA_STATE_TABLE_FALLBACK_REASON" ]; then
      echo "BETA_STATE_TABLE_FALLBACK_REASON is only valid for configured-path-fallback" >&2
      exit 2
    fi
    ;;
  configured-path-fallback)
    case "$BETA_STATE_TABLE_FALLBACK_REASON" in
      read-failed|decode-failed|load-failed|validation-failed) ;;
      *)
        echo "configured-path-fallback requires a bounded BETA_STATE_TABLE_FALLBACK_REASON" >&2
        exit 2
        ;;
    esac
    ;;
  *)
    echo "Invalid BETA_STATE_TABLE_SOURCE: $BETA_STATE_TABLE_SOURCE" >&2
    exit 2
    ;;
esac

case "${BETA_REQUIRE_CONFIGURED_STATE_TABLE_EVIDENCE:-0}" in
  1|true|TRUE|yes|YES|on|ON)
    if [ "$BETA_STATE_TABLE_SOURCE" != "configured-path" ] \
      || [ "$state_table_yaml_was_explicit" != "1" ]; then
      echo "configured state-table evidence requires configured-path and an explicit selected YAML file" >&2
      exit 2
    fi
    ;;
esac

if [ ! -f "$selected_state_table_yaml" ] || [ -L "$selected_state_table_yaml" ]; then
  echo "Selected state-table evidence must be a regular non-symlink file" >&2
  exit 2
fi
if [ "$BETA_STATE_TABLE_SOURCE" != "configured-path" ] \
  && ! cmp -s "$selected_state_table_yaml" "$CRATE_DIR/state_tables/default.yaml"; then
  echo "embedded/fallback state-table evidence must match the embedded default YAML" >&2
  exit 2
fi
selected_state_table_sha256="$(python3 - "$selected_state_table_yaml" <<'PY'
import hashlib
import pathlib
import sys

print(hashlib.sha256(pathlib.Path(sys.argv[1]).read_bytes()).hexdigest())
PY
)"

# Capture the source before creating any beta artifact. This keeps a custom
# artifact directory inside the checkout from changing the identity it is
# supposed to record; the final source fence will reject that unsafe topology.
source_at_start_tmp="$(mktemp "${TMPDIR:-/tmp}/rvoip-beta-source.XXXXXX")"
if ! python3 "$CANONICAL_2K_EVIDENCE_HELPER" fingerprint \
  --workspace-root "$WORKSPACE_ROOT" \
  --out "$source_at_start_tmp"; then
  rm -f "$source_at_start_tmp"
  exit 1
fi
mkdir -p "$ARTIFACT_DIR/environment"
mv "$source_at_start_tmp" "$BETA_SOURCE_AT_START"
mkdir -p "$GATE_RESULTS_DIR"
cat > "$SUMMARY" <<EOF
# rvoip-sip Beta Gate Summary

- timestamp: $TIMESTAMP
- mode: $MODE
- workspace: $WORKSPACE_ROOT
- artifact_dir: $ARTIFACT_DIR
- environment: \`environment/environment.md\`
EOF

slugify() {
  printf '%s' "$1" | tr '[:upper:] /:' '[:lower:]___' | tr -cd 'a-z0-9_.-'
}

record() {
  local status="$1"
  local name="$2"
  local log="$3"
  local duration="${4:--}"
  printf '| %s | %s | %s | `%s` |\n' "$status" "$name" "$duration" "${log#$ARTIFACT_DIR/}" >> "$SUMMARY"
}

file_sha256() {
  python3 - "$1" <<'PY'
import hashlib
import pathlib
import sys

print(hashlib.sha256(pathlib.Path(sys.argv[1]).read_bytes()).hexdigest())
PY
}

record_structured_gate() {
  local status="$1"
  local name="$2"
  local log="$3"
  local duration_seconds="$4"
  local started_at="$5"
  local ended_at="$6"
  local exit_status="$7"
  shift 7
  GATE_SEQUENCE=$((GATE_SEQUENCE + 1))
  python3 "$BETA_RELEASE_REPORT_HELPER" record-gate \
    --policy "$BETA_RELEASE_POLICY" \
    --results-dir "$GATE_RESULTS_DIR" \
    --sequence "$GATE_SEQUENCE" \
    --name "$name" \
    --status "$status" \
    --started "$started_at" \
    --ended "$ended_at" \
    --duration "$duration_seconds" \
    --exit-status "$exit_status" \
    --log "${log#$ARTIFACT_DIR/}" \
    --log-sha256 "$(file_sha256 "$log")" \
    -- "$@"
}

run_gate() {
  local name="$1"
  shift
  local log="$ARTIFACT_DIR/$(slugify "$name").log"
  local started_at
  local ended_at
  local start_epoch
  local end_epoch
  local duration
  local status
  echo
  echo "==> $name"
  started_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  start_epoch="$(date +%s)"
  {
    echo "gate: $name"
    echo "started_at_utc: $started_at"
    echo "workspace: $WORKSPACE_ROOT"
    echo "command: $*"
    echo
    echo "+ $*"
  } > "$log"
  set +e
  (cd "$WORKSPACE_ROOT" && "$@" >> "$log" 2>&1)
  status=$?
  set -e
  ended_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  end_epoch="$(date +%s)"
  duration="$((end_epoch - start_epoch))s"
  {
    echo
    echo "ended_at_utc: $ended_at"
    echo "duration_seconds: $((end_epoch - start_epoch))"
    echo "exit_status: $status"
  } >> "$log"
  if [ "$status" -eq 0 ]; then
    record "PASS" "$name" "$log" "$duration"
    record_structured_gate "PASS" "$name" "$log" "$((end_epoch - start_epoch))" \
      "$started_at" "$ended_at" "$status" "$@"
    return 0
  else
    record "FAIL" "$name" "$log" "$duration"
    record_structured_gate "FAIL" "$name" "$log" "$((end_epoch - start_epoch))" \
      "$started_at" "$ended_at" "$status" "$@"
    FAILURES=$((FAILURES + 1))
    echo "FAIL: $name (see $log)" >&2
    return 1
  fi
}

# A failed gate is evidence, not a reason to lose the rest of the evidence.
# Callers that need success-dependent control flow use run_gate directly in an
# if statement. Every independent gate uses this wrapper so the terminal source
# fence, summary, attestation, and report package are still attempted.
run_gate_continue() {
  run_gate "$@" || true
}

skip_gate() {
  local name="$1"
  local reason="$2"
  local log="$ARTIFACT_DIR/$(slugify "$name").log"
  local recorded_at
  recorded_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  {
    echo "gate: $name"
    echo "started_at_utc: $recorded_at"
    echo "ended_at_utc: $recorded_at"
    echo "command: skip-gate $reason"
    echo "duration_seconds: 0"
    echo "exit_status: 0"
    echo "SKIP: $reason"
  } > "$log"
  record "SKIP" "$name" "$log" "0s"
  record_structured_gate "SKIP" "$name" "$log" 0 \
    "$recorded_at" "$recorded_at" 0 skip-gate "$reason"
  SKIPS=$((SKIPS + 1))
  echo "SKIP: $name - $reason"
  if [ "$REQUIRE_EXTERNAL" = "1" ]; then
    FAILURES=$((FAILURES + 1))
  fi
}

bool_env_enabled() {
  case "${1:-0}" in
    1|true|TRUE|yes|YES|on|ON) return 0 ;;
    *) return 1 ;;
  esac
}

verify_clean_source_fingerprint() {
  python3 - "$BETA_SOURCE_AT_START" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
source = json.loads(path.read_text(encoding="utf-8"))
if source.get("git_dirty") is not False:
    print(
        "beta release source must be a clean Git worktree; "
        f"captured git_dirty={source.get('git_dirty')!r}",
        file=sys.stderr,
    )
    raise SystemExit(1)
print(f"clean source fingerprint: {source['source_fingerprint_sha256']}")
PY
}

append_feature() {
  local current="$1"
  local feature="$2"
  if [ -z "$current" ]; then
    printf '%s' "$feature"
    return
  fi
  case ",$current," in
    *,"$feature",*) printf '%s' "$current" ;;
    *) printf '%s,%s' "$current" "$feature" ;;
  esac
}

perf_features() {
  local features="perf-tests"

  if bool_env_enabled "${BETA_PERF_INFRA_MEMORY_DIAGNOSTICS:-0}" \
    || bool_env_enabled "${RVOIP_PERF_MEMORY_DIAGNOSTICS:-0}" \
    || bool_env_enabled "${RVOIP_PERF_ALLOCATOR_DIAGNOSTICS:-0}"; then
    features="$(append_feature "$features" "perf-infra-memory-diagnostics")"
  fi
  if bool_env_enabled "${BETA_PERF_MEDIA_DIAGNOSTICS:-0}"; then
    features="$(append_feature "$features" "perf-media-diagnostics")"
  fi
  if bool_env_enabled "${BETA_PERF_MEDIA_MEMORY_DIAGNOSTICS:-0}"; then
    features="$(append_feature "$features" "perf-media-memory-diagnostics")"
  fi
  if bool_env_enabled "${BETA_PERF_RTP_MEMORY_DIAGNOSTICS:-0}"; then
    features="$(append_feature "$features" "perf-rtp-memory-diagnostics")"
  fi

  printf '%s' "$features"
}

perf_profile_matrix() {
  if [ -n "${BETA_PERF_PROFILE_MATRIX:-}" ]; then
    printf '%s' "$BETA_PERF_PROFILE_MATRIX"
  else
    printf '%s' "endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000"
  fi
}

capture_command() {
  local output="$1"
  shift
  {
    echo "+ $*"
    "$@"
  } > "$output" 2>&1 || true
}

redacted_env() {
  env | LC_ALL=C sort | awk -F= '
    /^(BETA_|PBX_|PROXY_|RVOIP_|SIPP_|ASTERISK_|FREESWITCH_|SIP_|TLS_)/ {
      key=$1
      value=substr($0, length($1) + 2)
      redacted=key
      upper=toupper(key)
      if (upper ~ /(PASSWORD|PASS|SECRET|TOKEN|CREDENTIAL|PRIVATE|AUTHORIZATION)/) {
        print key"=<redacted>"
      } else if (key == "BETA_STATE_TABLE_SELECTED_YAML") {
        print key"=<captured as hashed attestation input>"
      } else {
        print key"="value
      }
    }
  '
}

captured_payload() {
  local file="$1"
  if [ ! -f "$file" ]; then
    printf 'not captured\n'
    return
  fi
  awk 'NR == 1 && /^\+ / { next } { print }' "$file"
}

captured_first_line() {
  local value
  value="$(captured_payload "$1" | awk 'NF { print; exit }')"
  printf '%s' "${value:-none}"
}

captured_status_label() {
  local payload
  payload="$(captured_payload "$1")"
  if [ "$payload" = "not captured" ]; then
    printf 'not captured'
  elif [ -z "$payload" ]; then
    printf 'clean'
  else
    printf 'dirty'
  fi
}

markdown_payload_block() {
  local title="$1"
  local file="$2"
  echo "## $title"
  echo
  echo '```text'
  captured_payload "$file"
  echo '```'
  echo
}

markdown_file_block() {
  local title="$1"
  local file="$2"
  echo "## $title"
  echo
  if [ -f "$file" ]; then
    echo '```text'
    cat "$file"
    echo '```'
  else
    echo 'not captured'
  fi
  echo
}

markdown_local_pbx_config() {
  local name="$1"
  local source_dir="$2"
  local out_dir="$3"
  echo "## Local PBX Config: $name"
  echo
  echo "- source_dir: $source_dir"
  if [ -d "$out_dir" ]; then
    echo "- captured_files:"
    find "$out_dir" -maxdepth 3 -type f -print | sort | while IFS= read -r file; do
      echo "  - ${file#$ARTIFACT_DIR/}"
    done
  else
    echo "- captured_files: none"
  fi
  echo
  for file in README.md Dockerfile docker-compose.yml docker-entrypoint.sh freeswitch-modules.conf rvoip-local.env freeswitch-local.env config/pjsip.conf config/extensions.conf config/modules.conf git-rev.txt git-status.txt; do
    if [ -f "$out_dir/$file" ]; then
      markdown_file_block "$name $file" "$out_dir/$file"
    fi
  done
}

redact_file() {
  local input="$1"
  local output="$2"
  if [ ! -f "$input" ]; then
    return
  fi
  sed -E \
    -e 's/([Pp][Aa][Ss][Ss][Ww][Oo][Rr][Dd][[:space:]]*[:=][[:space:]]*).*/\1<redacted>/' \
    -e 's/([Ss][Ee][Cc][Rr][Ee][Tt][[:space:]]*[:=][[:space:]]*).*/\1<redacted>/' \
    -e 's/([Tt][Oo][Kk][Ee][Nn][[:space:]]*[:=][[:space:]]*).*/\1<redacted>/' \
    -e 's/password123/<redacted>/g' \
    "$input" > "$output" || true
}

capture_docker_snapshot() {
  local label="$1"
  local dir="$ARTIFACT_DIR/environment/docker-$label"
  local tail_lines="${BETA_PBX_LOG_TAIL:-1000}"
  if [ "${BETA_CAPTURE_DOCKER_LOGS:-1}" = "0" ]; then
    return
  fi
  mkdir -p "$dir"
  if ! command -v docker >/dev/null 2>&1; then
    echo "docker not found" > "$dir/README.txt"
    return
  fi
  capture_command "$dir/docker-ps.txt" docker ps --all
  for container in rvoip-asterisk rvoip-freeswitch; do
    if docker inspect "$container" >/dev/null 2>&1; then
      local snapshot_tmp="$dir/.${container}-peer.json.tmp"
      local snapshot_error="$dir/${container}-peer.stderr.txt"
      if docker inspect "$container" 2>"$snapshot_error" \
        | python3 "$DOCKER_PEER_SNAPSHOT_HELPER" \
            --product "${container#rvoip-}" >"$snapshot_tmp" 2>>"$snapshot_error"; then
        mv "$snapshot_tmp" "$dir/${container}-peer.json"
        if [ ! -s "$snapshot_error" ]; then
          rm -f "$snapshot_error"
        fi
      else
        rm -f "$snapshot_tmp"
        echo "sanitized Docker peer snapshot failed; see $(basename "$snapshot_error")" \
          > "$dir/${container}-peer-missing.txt"
      fi
      capture_command "$dir/${container}-logs-tail.txt" docker logs --tail "$tail_lines" "$container"
    else
      echo "$container not found" > "$dir/${container}-missing.txt"
    fi
  done
}

copy_local_pbx_config_evidence() {
  local name="$1"
  local dir="$2"
  local out="$ARTIFACT_DIR/environment/local-pbx/$name"
  mkdir -p "$out"
  for file in README.md Dockerfile docker-compose.yml docker-entrypoint.sh freeswitch-modules.conf rvoip-local.env freeswitch-local.env config/pjsip.conf config/extensions.conf config/modules.conf; do
    if [ -f "$dir/$file" ]; then
      mkdir -p "$out/$(dirname "$file")"
      redact_file "$dir/$file" "$out/$file"
    fi
  done
  if git -C "$dir" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    capture_command "$out/git-rev.txt" git -C "$dir" rev-parse HEAD
    capture_command "$out/git-status.txt" git -C "$dir" status --short
  fi
}

write_environment_report() {
  local env_dir="$ARTIFACT_DIR/environment"
  local asterisk_dir="${BETA_ASTERISK_DIR:-$HOME/Developer/asterisk}"
  local freeswitch_dir="${BETA_FREESWITCH_DIR:-$HOME/Developer/freeswitch}"
  mkdir -p "$env_dir"

  capture_command "$env_dir/git-rev.txt" git -C "$WORKSPACE_ROOT" rev-parse --short HEAD
  capture_command "$env_dir/git-status.txt" git -C "$WORKSPACE_ROOT" status --short
  capture_command "$env_dir/rustc-version.txt" rustc --version --verbose
  capture_command "$env_dir/cargo-version.txt" cargo --version --verbose
  local cargo_metadata_tmp="$env_dir/.cargo-metadata.json.tmp"
  if ! (cd "$WORKSPACE_ROOT" && cargo metadata --no-deps --format-version 1 \
    > "$cargo_metadata_tmp" 2> "$env_dir/cargo-metadata.stderr.txt"); then
    rm -f "$cargo_metadata_tmp"
    echo "failed to capture Cargo metadata for beta attestation" >&2
    return 1
  fi
  mv "$cargo_metadata_tmp" "$env_dir/cargo-metadata.json"
  capture_command "$env_dir/host-uname.txt" uname -a
  if command -v sw_vers >/dev/null 2>&1; then
    capture_command "$env_dir/macos-version.txt" sw_vers
  fi
  if command -v sysctl >/dev/null 2>&1; then
    capture_command "$env_dir/host-hardware.txt" sysctl -n machdep.cpu.brand_string hw.physicalcpu hw.logicalcpu hw.memsize
  fi
  if command -v colima >/dev/null 2>&1; then
    capture_command "$env_dir/colima-version.txt" colima version
    capture_command "$env_dir/colima-status.txt" colima status
  fi
  if command -v docker >/dev/null 2>&1; then
    capture_command "$env_dir/docker-version.txt" docker version
    capture_command "$env_dir/docker-ps-start.txt" docker ps --all
  else
    {
      echo "docker not found on PATH"
      echo "PATH=$PATH"
    } > "$env_dir/docker-version.txt"
  fi
  if command -v docker-compose >/dev/null 2>&1; then
    capture_command "$env_dir/docker-compose-version.txt" docker-compose version
  elif docker compose version >/dev/null 2>&1; then
    capture_command "$env_dir/docker-compose-version.txt" docker compose version
  fi
  redacted_env > "$env_dir/beta-env-redacted.txt"
  copy_local_pbx_config_evidence asterisk "$asterisk_dir"
  copy_local_pbx_config_evidence freeswitch "$freeswitch_dir"
  capture_docker_snapshot start

  {
    cat <<EOF
# Beta Gate Environment

- timestamp_utc: $TIMESTAMP
- mode: $MODE
- workspace: $WORKSPACE_ROOT
- artifact_dir: $ARTIFACT_DIR
- git_revision: \`$(captured_first_line "$env_dir/git-rev.txt")\`
- git_status: \`$(captured_status_label "$env_dir/git-status.txt")\`
- rustc: \`$(captured_first_line "$env_dir/rustc-version.txt")\`
- cargo: \`$(captured_first_line "$env_dir/cargo-version.txt")\`
- cargo_metadata: \`environment/cargo-metadata.json\`
- beta_deny_warnings: \`${BETA_DENY_WARNINGS:-1}\`
- beta_test_log_filter: \`${BETA_TEST_LOG_FILTER:-off}\`
- rvoip_require_api_tools: \`${RVOIP_REQUIRE_API_TOOLS}\`
- source_at_beta_start: \`environment/source-at-beta-start.json\`
- source_at_beta_end: \`environment/source-at-beta-end.json\`
- beta_require_clean_source: \`${BETA_REQUIRE_CLEAN_SOURCE}\`
- beta_gate_require_external: \`${REQUIRE_EXTERNAL}\`
- beta_attestation_features: \`$(attestation_features)\`
- beta_attestation_target: \`${BETA_ATTESTATION_TARGET:-${CARGO_BUILD_TARGET:-rustc-host}}\`
- beta_require_canonical_2k_evidence: \`${BETA_REQUIRE_CANONICAL_2K_EVIDENCE:-0}\`
- beta_state_table_source: \`${BETA_STATE_TABLE_SOURCE}\`
- beta_state_table_fallback_reason: \`${BETA_STATE_TABLE_FALLBACK_REASON:-none}\`
- beta_state_table_sha256: \`${selected_state_table_sha256}\`
- beta_require_configured_state_table_evidence: \`${BETA_REQUIRE_CONFIGURED_STATE_TABLE_EVIDENCE:-0}\`
- beta_run_pbx: \`${BETA_RUN_PBX:-0}\`
- beta_run_local_pbx: \`${BETA_RUN_LOCAL_PBX:-0}\`
- beta_proxy_interop_peers: \`kamailio opensips\`
- beta_proxy_interop_orders: \`rvoip-first peer-first\`
- beta_proxy_interop_transports: \`udp tcp tls\`
- beta_proxy_interop_retention_drain_seconds: \`130\`
- beta_proxy_interop_require_clean_source: \`1\`
- beta_proxy_interop_require_unchanged_source: \`1\`
- beta_perf_regression_fail: \`${BETA_PERF_REGRESSION_FAIL:-0}\`
- beta_perf_regression_baseline_id: \`${perf_regression_baseline_id}\`
- beta_perf_regression_baseline_manifest_sha256: \`${perf_regression_baseline_manifest_sha256}\`
- host: \`$(captured_first_line "$env_dir/host-uname.txt")\`
- colima: \`$(captured_first_line "$env_dir/colima-status.txt")\`
- docker: \`$(captured_first_line "$env_dir/docker-version.txt")\`
- beta_perf_features: \`$(perf_features)\`
- beta_perf_infra_memory_diagnostics: \`${BETA_PERF_INFRA_MEMORY_DIAGNOSTICS:-0}\`
- beta_perf_media_diagnostics: \`${BETA_PERF_MEDIA_DIAGNOSTICS:-0}\`
- beta_perf_media_memory_diagnostics: \`${BETA_PERF_MEDIA_MEMORY_DIAGNOSTICS:-0}\`
- beta_perf_rtp_memory_diagnostics: \`${BETA_PERF_RTP_MEMORY_DIAGNOSTICS:-0}\`

Docker snapshots captured during local PBX lifecycle events are stored under
\`environment/docker-<phase>/\`. Peer JSON uses a strict allowlist; raw Docker
inspect documents are never written and are rejected by attestation. Secrets in
copied local env/config files are redacted by key name before being written into
this artifact tree.
EOF

    echo
    markdown_payload_block "Git Status" "$env_dir/git-status.txt"
    markdown_payload_block "Rust Toolchain" "$env_dir/rustc-version.txt"
    markdown_payload_block "Cargo Toolchain" "$env_dir/cargo-version.txt"
    markdown_payload_block "Host Kernel" "$env_dir/host-uname.txt"
    if [ -f "$env_dir/macos-version.txt" ]; then
      markdown_payload_block "macOS Version" "$env_dir/macos-version.txt"
    fi
    if [ -f "$env_dir/host-hardware.txt" ]; then
      markdown_payload_block "Host Hardware" "$env_dir/host-hardware.txt"
    fi
    if [ -f "$env_dir/colima-status.txt" ]; then
      markdown_payload_block "Colima Status" "$env_dir/colima-status.txt"
    fi
    if [ -f "$env_dir/docker-version.txt" ]; then
      markdown_payload_block "Docker Version" "$env_dir/docker-version.txt"
    fi
    if [ -f "$env_dir/docker-compose-version.txt" ]; then
      markdown_payload_block "Docker Compose Version" "$env_dir/docker-compose-version.txt"
    fi
    if [ -f "$env_dir/docker-ps-start.txt" ]; then
      markdown_payload_block "Initial Docker State" "$env_dir/docker-ps-start.txt"
    fi
    markdown_file_block "Redacted Gate Environment" "$env_dir/beta-env-redacted.txt"
    markdown_local_pbx_config asterisk "$asterisk_dir" "$env_dir/local-pbx/asterisk"
    markdown_local_pbx_config freeswitch "$freeswitch_dir" "$env_dir/local-pbx/freeswitch"

    cat <<EOF
## Raw Evidence Files

The inlined values above are also retained as raw evidence files under
\`environment/\` so scripts can consume the same captured data without parsing
Markdown.
EOF
  } > "$ENV_REPORT"
}

write_summary_gate_table_header() {
  local env_dir="$ARTIFACT_DIR/environment"
  {
    cat <<EOF

## Environment Snapshot

- git_revision: \`$(captured_first_line "$env_dir/git-rev.txt")\`
- git_status: \`$(captured_status_label "$env_dir/git-status.txt")\`
- rustc: \`$(captured_first_line "$env_dir/rustc-version.txt")\`
- cargo: \`$(captured_first_line "$env_dir/cargo-version.txt")\`
- cargo_metadata: \`environment/cargo-metadata.json\`
- beta_deny_warnings: \`${BETA_DENY_WARNINGS:-1}\`
- beta_test_log_filter: \`${BETA_TEST_LOG_FILTER:-off}\`
- rvoip_require_api_tools: \`${RVOIP_REQUIRE_API_TOOLS}\`
- source_at_beta_start: \`environment/source-at-beta-start.json\`
- source_at_beta_end: \`environment/source-at-beta-end.json\`
- beta_require_clean_source: \`${BETA_REQUIRE_CLEAN_SOURCE}\`
- beta_gate_require_external: \`${REQUIRE_EXTERNAL}\`
- beta_attestation_features: \`$(attestation_features)\`
- beta_attestation_target: \`${BETA_ATTESTATION_TARGET:-${CARGO_BUILD_TARGET:-rustc-host}}\`
- beta_require_canonical_2k_evidence: \`${BETA_REQUIRE_CANONICAL_2K_EVIDENCE:-0}\`
- beta_canonical_2k_run_dirs: \`${BETA_CANONICAL_2K_RUN_DIRS:-not supplied}\`
- beta_state_table_source: \`${BETA_STATE_TABLE_SOURCE}\`
- beta_state_table_fallback_reason: \`${BETA_STATE_TABLE_FALLBACK_REASON:-none}\`
- beta_state_table_sha256: \`${selected_state_table_sha256}\`
- beta_require_configured_state_table_evidence: \`${BETA_REQUIRE_CONFIGURED_STATE_TABLE_EVIDENCE:-0}\`
- host: \`$(captured_first_line "$env_dir/host-uname.txt")\`
- colima: \`$(captured_first_line "$env_dir/colima-status.txt")\`
- docker: \`$(captured_first_line "$env_dir/docker-version.txt")\`
- beta_profile_matrix: \`$(perf_profile_matrix)\`
- beta_run_perf_all: \`${BETA_RUN_PERF_ALL:-0}\`
- beta_perf_regression_fail: \`${BETA_PERF_REGRESSION_FAIL:-0}\`
- beta_perf_regression_baseline_id: \`${perf_regression_baseline_id}\`
- beta_perf_regression_baseline_manifest_sha256: \`${perf_regression_baseline_manifest_sha256}\`
- beta_perf_high_density_burst_cps: \`160\`
- beta_perf_high_density_min_asr: \`0.995\`
- beta_perf_high_density_rss_limit_mb_per_hr: \`15\`
- beta_perf_media_churn_duration_secs: \`${BETA_PERF_MEDIA_CHURN_DURATION_SECS:-120}\`
- beta_perf_media_churn_active_calls: \`${BETA_PERF_MEDIA_CHURN_ACTIVE_CALLS:-30}\`
- beta_perf_monolithic_soak_duration_secs: \`${BETA_PERF_MONOLITHIC_SOAK_DURATION_SECS:-3600}\`
- beta_perf_monolithic_soak_active_calls: \`${BETA_PERF_MONOLITHIC_SOAK_ACTIVE_CALLS:-30}\`
- beta_performance_recipe_file: \`${BETA_PERFORMANCE_RECIPE_FILE:-bundled config/performance-recipes.yaml}\`
- beta_perf_features: \`$(perf_features)\`
- beta_perf_infra_memory_diagnostics: \`${BETA_PERF_INFRA_MEMORY_DIAGNOSTICS:-0}\`
- beta_perf_media_diagnostics: \`${BETA_PERF_MEDIA_DIAGNOSTICS:-0}\`
- beta_perf_media_memory_diagnostics: \`${BETA_PERF_MEDIA_MEMORY_DIAGNOSTICS:-0}\`
- beta_perf_rtp_memory_diagnostics: \`${BETA_PERF_RTP_MEMORY_DIAGNOSTICS:-0}\`
- beta_run_burst_smoke: \`${BETA_RUN_BURST_SMOKE:-1}\`
- beta_run_burst_matrix: \`${BETA_RUN_BURST_MATRIX:-0}\`
- beta_burst_scenario_file: \`${BETA_BURST_SCENARIO_FILE:-bundled config/perf-burst-scenarios.yaml}\`
- beta_burst_matrix: \`${BETA_BURST_MATRIX:-all}\`
- beta_pbx_provider: \`${BETA_PBX_PROVIDER:-both}\`
- beta_pbx_api: \`${BETA_PBX_API:-all}\`
- beta_pbx_scenario: \`${BETA_PBX_SCENARIO:-all}\`
- beta_pbx_g729_profiles: \`${BETA_PBX_G729_PROFILES:-g729a g729ab}\`
- beta_run_local_pbx: \`${BETA_RUN_LOCAL_PBX:-0}\`
- beta_run_pbx: \`${BETA_RUN_PBX:-0}\`
- beta_run_sipp: \`${BETA_RUN_SIPP:-1}\`
- beta_sipp_diagnostics: \`${BETA_SIPP_DIAGNOSTICS:-0}\`
- beta_run_strict_ua: \`${BETA_RUN_STRICT_UA:-1}\`
- beta_proxy_interop_peers: \`kamailio opensips\`
- beta_proxy_interop_orders: \`rvoip-first peer-first\`
- beta_proxy_interop_transports: \`udp tcp tls\`
- beta_proxy_interop_retention_drain_seconds: \`130\`
- beta_proxy_interop_require_clean_source: \`1\`
- beta_proxy_interop_require_unchanged_source: \`1\`
- beta_run_long_soak: \`${BETA_RUN_LONG_SOAK:-1}\`
- rvoip_perf_soak_duration_secs: \`${RVOIP_PERF_SOAK_DURATION_SECS:-3600}\`
- rvoip_perf_soak_active_calls: \`${RVOIP_PERF_SOAK_ACTIVE_CALLS:-500}\`
- rvoip_perf_soak_min_hold_secs: \`${RVOIP_PERF_SOAK_MIN_HOLD_SECS:-10}\`
- rvoip_perf_soak_max_hold_secs: \`${RVOIP_PERF_SOAK_MAX_HOLD_SECS:-360}\`
- rvoip_perf_soak_cps: \`${RVOIP_PERF_SOAK_CPS:-0}\`
- rvoip_perf_soak_drain_cps: \`${RVOIP_PERF_SOAK_DRAIN_CPS:-10}\`
- rvoip_perf_soak_error_sample_limit: \`${RVOIP_PERF_SOAK_ERROR_SAMPLE_LIMIT:-32}\`
- rvoip_perf_retention_drain_wait_secs: \`${RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS:-130}\`
- rvoip_perf_mass_teardown_calls: \`${RVOIP_PERF_MASS_TEARDOWN_CALLS:-500}\`
- rvoip_perf_mass_teardown_setup_cps: \`${RVOIP_PERF_MASS_TEARDOWN_SETUP_CPS:-30}\`
- rvoip_perf_memory_diagnostics: \`${RVOIP_PERF_MEMORY_DIAGNOSTICS:-0}\`
- rvoip_perf_allocator_diagnostics: \`${RVOIP_PERF_ALLOCATOR_DIAGNOSTICS:-0}\`
- rvoip_perf_memory_diag_interval_secs: \`${RVOIP_PERF_MEMORY_DIAG_INTERVAL_SECS:-5}\`
- rvoip_perf_mimalloc_collect_at: \`${RVOIP_PERF_MIMALLOC_COLLECT_AT:-off}\`
- rvoip_perf_system_allocator: \`${RVOIP_PERF_SYSTEM_ALLOCATOR:-0}\`
- rvoip_perf_dhat: \`${RVOIP_PERF_DHAT:-0}\`
- rvoip_perf_heap_snapshots: \`${RVOIP_PERF_HEAP_SNAPSHOTS:-0}\`
- rvoip_perf_heap_snapshot_secs: \`${RVOIP_PERF_HEAP_SNAPSHOT_SECS:-auto}\`
- rvoip_perf_malloc_stack_logging: \`${RVOIP_PERF_MALLOC_STACK_LOGGING:-0}\`
- rvoip_perf_leaks_snapshots: \`${RVOIP_PERF_LEAKS_SNAPSHOTS:-0}\`
- rvoip_perf_skip_audio_frame_delivery: \`${RVOIP_PERF_SKIP_AUDIO_FRAME_DELIVERY:-0}\`
- rvoip_perf_max_rss_growth_mb_per_hr: \`${RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR:-15}\`
- rvoip_perf_app_event_channel_capacity: \`${RVOIP_PERF_APP_EVENT_CHANNEL_CAPACITY:-Config default}\`
- rvoip_perf_rss_tail_window_secs: \`${RVOIP_PERF_RSS_TAIL_WINDOW_SECS:-60}\`

Full environment evidence, Docker state, redacted runtime variables, and local
PBX config snapshots are in \`environment/environment.md\`.

## Gates

| Status | Gate | Duration | Log |
|--------|------|----------|-----|
EOF
  } >> "$SUMMARY"
}

beta_report_root() {
  printf '%s' "${BETA_REPORT_DIR:-$CRATE_DIR/beta-report}"
}

beta_report_run_dir() {
  printf '%s/%s' "$(beta_report_root)" "$TIMESTAMP"
}

beta_report_mode_pointer() {
  case "$MODE" in
    local) printf '%s/latest-local.txt' "$(beta_report_root)" ;;
    interop) printf '%s/latest-interop.txt' "$(beta_report_root)" ;;
    security) printf '%s/latest-security.txt' "$(beta_report_root)" ;;
    perf) printf '%s/latest-perf.txt' "$(beta_report_root)" ;;
    full) printf '%s/latest-full-clean.txt' "$(beta_report_root)" ;;
  esac
}

attestation_features() {
  local features=""
  local automatic=""
  local requested
  local feature
  case "$MODE" in
    local)
      automatic="generated-validation,dev-insecure-tls"
      ;;
    full)
      automatic="generated-validation,dev-insecure-tls,$(perf_features)"
      ;;
    interop)
      automatic="dev-insecure-tls"
      ;;
    perf)
      automatic="$(perf_features)"
      ;;
    security)
      automatic=""
      ;;
  esac

  # PBX examples are built separately from the local feature matrix. Record
  # their effective Cargo features automatically whenever that interop surface
  # is enabled; the default includes the claim-bearing G.729A/G.729AB codec.
  local pbx_features=""
  if [ "$MODE" = "full" ] || [ "$MODE" = "interop" ]; then
    if [ "${BETA_RUN_LOCAL_PBX:-0}" = "1" ] || [ "${BETA_RUN_PBX:-0}" = "1" ]; then
      pbx_features="${PBX_CARGO_FEATURES:-dev-insecure-tls,g729}"
    fi
  fi

  for requested in "${BETA_ATTESTATION_FEATURES:-}" "$automatic" "$pbx_features"; do
    requested="${requested//,/ }"
    for feature in $requested; do
      features="$(append_feature "$features" "$feature")"
    done
  done
  printf '%s' "$features"
}

attestation_input_path() {
  local value="$1"
  case "$value" in
    /*) printf '%s' "$value" ;;
    *) printf '%s/%s' "$WORKSPACE_ROOT" "$value" ;;
  esac
}

write_beta_attestation() {
  local report_root="$1"
  local overall="PASS"
  local recipe_file
  local burst_file
  local -a state_table_args=(
    --state-table-source "$BETA_STATE_TABLE_SOURCE"
    --state-table-sha256 "$selected_state_table_sha256"
  )
  if [ "$FAILURES" -ne 0 ]; then
    overall="FAIL"
  fi
  if [ -n "$BETA_STATE_TABLE_FALLBACK_REASON" ]; then
    state_table_args+=(--state-table-fallback-reason "$BETA_STATE_TABLE_FALLBACK_REASON")
  fi
  recipe_file="$(attestation_input_path "${BETA_PERFORMANCE_RECIPE_FILE:-$CRATE_DIR/config/performance-recipes.yaml}")"
  burst_file="$(attestation_input_path "${BETA_BURST_SCENARIO_FILE:-$CRATE_DIR/config/perf-burst-scenarios.yaml}")"
  python3 "$BETA_ATTESTATION_HELPER" create \
    --report-root "$report_root" \
    --schema-version 2 \
    --mode "$MODE" \
    --run-id "$TIMESTAMP" \
    --started-at "$STARTED_AT_UTC" \
    --ended-at "$ENDED_AT_UTC" \
    --features "$(attestation_features)" \
    --target "${BETA_ATTESTATION_TARGET:-${CARGO_BUILD_TARGET:-}}" \
    "${state_table_args[@]}" \
    --input "state-machine-yaml=$selected_state_table_yaml" \
    --input "performance-recipe=$recipe_file" \
    --input "burst-scenarios=$burst_file" \
    --input "performance-regression-baseline=$PERF_REGRESSION_BASELINE_MANIFEST" \
    --input "performance-regression-baseline-helper=$PERF_REGRESSION_BASELINE_HELPER" \
    --input "sipp-scenario=$CRATE_DIR/tests/perf/sipp_scenarios/uac_perf.xml" \
    --input "attestation-verifier=$BETA_ATTESTATION_HELPER" \
    --input "beta-release-policy=$BETA_RELEASE_POLICY" \
    --input "beta-release-report-generator=$BETA_RELEASE_REPORT_HELPER" \
    --failures "$FAILURES" \
    --skips "$SKIPS" \
    --overall "$overall"
}

write_report_manifest() {
  local report_dir="$1"
  local perf_results_status="${2:-not packaged}"
  local manifest="$report_dir/report-manifest.md"
  cat > "$manifest" <<EOF
# rvoip-sip Beta Report Manifest

- timestamp: $TIMESTAMP
- mode: $MODE
- workspace: $WORKSPACE_ROOT
- source_artifact_dir: $ARTIFACT_DIR
- report_dir: $report_dir
- summary: \`summary.md\`
- environment: \`environment/environment.md\`
- machine_attestation: \`attestation.json\`
- attestation_checksum: \`attestation.json.sha256\`
- generated_at_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)

## Primary Evidence

- \`summary.md\`
- \`attestation.json\` and \`attestation.json.sha256\`
- \`effective-gate-config.json\` and \`gate-results.json\` (native typed
  reporting inputs; Markdown is display-only)
- \`inputs/attestation-verifier.py\` (copied standalone verifier)
- \`environment/environment.md\`
- \`pbx/summary.md\`
- \`pbx/matrix.tsv\`
- \`sipp/environment.md\`
- \`sipp/run_summary.md\`
- \`sipp/analysis.md\`
- \`strict-ua/summary.md\`
- \`security/cargo-audit.txt\`
- \`security/fuzz/\`
- \`perf-results/\`
- \`perf-regression-baseline/manifest.json\` and \`perf-regression-baseline/perf-results/\`
  (reviewed immutable inputs copied before comparison)
- \`perf-audit.md\` (current-vs-reviewed-baseline regression audit)
- \`canonical-2k/index.json\` and \`canonical-2k/run-{1,2,3}/\`
  (required release evidence when enabled)
- \`release-reports/\` (generated and independently verified before a
  qualifying full-run release pointer can update)

The report directory is a packaged copy of the beta-gate artifact tree,
including the isolated current-run perf result capture when performance gates
are enabled. Logs, matrices, redacted
environment evidence, PBX lifecycle snapshots, scenario metadata, and perf
JSON/markdown outputs are kept with their original relative paths where
possible.

Perf results package status: ${perf_results_status}

Verify this copied report without reading workspace paths:

\`python3 inputs/attestation-verifier.py verify --report-root .\`

Require formal mode-complete release evidence (the clean/full report must also
meet every release-pointer prerequisite):

\`python3 inputs/attestation-verifier.py verify --report-root . --require-clean --require-unchanged-source --require-no-skips --require-pass --require-mode-eligible\`
EOF
}

prepare_perf_results_capture() {
  local archive_dir="$PERF_RESULTS_ARCHIVE_ROOT/$TIMESTAMP-before-$MODE"
  local prior_files=0
  local prior_kib=0
  mkdir -p "$PERF_RESULTS_ARCHIVE_ROOT" "$(dirname "$PERF_RESULTS_CAPTURE_MARKER")"

  if [ -L "$PERF_RESULTS_DIR" ]; then
    echo "refusing symlinked perf-results directory: $PERF_RESULTS_DIR" >&2
    return 1
  fi
  if [ -e "$archive_dir" ] || [ -L "$archive_dir" ]; then
    echo "perf-results archive destination already exists: $archive_dir" >&2
    return 1
  fi
  if [ -e "$PERF_RESULTS_DIR" ]; then
    if [ ! -d "$PERF_RESULTS_DIR" ]; then
      echo "perf-results path is not a directory: $PERF_RESULTS_DIR" >&2
      return 1
    fi
    prior_files="$(find "$PERF_RESULTS_DIR" -type f | wc -l | tr -d ' ')"
    prior_kib="$(du -sk "$PERF_RESULTS_DIR" | awk '{print $1}')"
    mv "$PERF_RESULTS_DIR" "$archive_dir"
  else
    archive_dir="none (no prior perf-results directory)"
  fi
  mkdir -p "$PERF_RESULTS_DIR"
  cat > "$PERF_RESULTS_CAPTURE_MARKER" <<EOF
# Performance Results Capture Boundary

- created_at_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)
- current_run_directory: $PERF_RESULTS_DIR
- prior_results_archive: $archive_dir
- prior_file_count: $prior_files
- prior_size_kib: $prior_kib
- policy: only files written after this boundary may be analyzed or packaged
EOF
}

perf_results_capture_ready() {
  [ -f "$PERF_RESULTS_CAPTURE_MARKER" ] \
    && [ -d "$PERF_RESULTS_DIR" ] \
    && [ ! -L "$PERF_RESULTS_DIR" ]
}

capture_current_perf_results() {
  if ! perf_results_capture_ready; then
    echo "performance results capture boundary is absent" >&2
    return 1
  fi
  local destination="$ARTIFACT_DIR/perf-results"
  mkdir -p "$destination"
  (cd "$PERF_RESULTS_DIR" && tar cf - .) | (cd "$destination" && tar xf -)
  cat >> "$PERF_RESULTS_CAPTURE_MARKER" <<EOF
- captured_at_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)
- captured_artifact_directory: $destination
- captured_file_count: $(find "$PERF_RESULTS_DIR" -type f | wc -l | tr -d ' ')
EOF
}

write_performance_gate_metrics() {
  local -a arguments=(
    --perf-root "$ARTIFACT_DIR/perf-results"
    --output-json "$ARTIFACT_DIR/performance-gate-metrics.json"
    --output-markdown "$ARTIFACT_DIR/performance-gate-metrics.md"
    --high-density-cps 160
    --high-density-min-asr 0.995
    --rss-limit-mb-per-hr "${RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR:-15}"
    --monolithic-duration-secs "${BETA_PERF_MONOLITHIC_SOAK_DURATION_SECS:-3600}"
    --monolithic-active-calls "${BETA_PERF_MONOLITHIC_SOAK_ACTIVE_CALLS:-30}"
  )
  if [ "${BETA_RUN_BURST_MATRIX:-0}" = "1" ]; then
    case " ${BETA_BURST_MATRIX:-all} " in
      *" all "*|*" high-density-media-burst "*) arguments+=(--require-high-density) ;;
    esac
  fi
  if [ "${BETA_RUN_PERF_ALL:-0}" = "1" ]; then
    arguments+=(--require-monolithic)
  fi
  if [ "${BETA_REQUIRE_CANONICAL_2K_EVIDENCE:-0}" = "1" ]; then
    arguments+=(
      --canonical-index "$ARTIFACT_DIR/canonical-2k/index.json"
      --candidate-sha "$(git -C "$WORKSPACE_ROOT" rev-parse HEAD)"
      --require-canonical
    )
  fi
  python3 "$SCRIPT_DIR/beta_performance_gate_metrics.py" "${arguments[@]}"
}

copy_perf_results_into_report() {
  local report_dir="$1"
  if [ ! -d "$ARTIFACT_DIR/perf-results" ]; then
    printf 'not packaged; this mode produced no isolated perf-results capture'
    return 0
  fi
  if [ ! -d "$report_dir/perf-results" ]; then
    echo "packaged report is missing the isolated perf-results capture" >&2
    return 1
  fi
  printf 'packaged from isolated raw artifact capture: %s/perf-results' "$ARTIFACT_DIR"
}

package_beta_report() {
  if [ "${BETA_REPORT_PACKAGE:-1}" = "0" ]; then
    return 0
  fi

  local root
  local report_dir
  local artifact_abs
  local report_abs
  root="$(beta_report_root)"
  report_dir="$(beta_report_run_dir)"
  mkdir -p "$report_dir"
  artifact_abs="$(cd "$ARTIFACT_DIR" && pwd -P)"
  report_abs="$(cd "$report_dir" && pwd -P)"

  if [ "$artifact_abs" != "$report_abs" ]; then
    (cd "$ARTIFACT_DIR" && tar cf - .) | (cd "$report_dir" && tar xf -)
  fi

  local perf_results_status
  perf_results_status="$(copy_perf_results_into_report "$report_dir")"

  write_report_manifest "$report_dir" "$perf_results_status"
  write_beta_attestation "$report_dir"
  if [ "$MODE" = "full" ] && [ "$FAILURES" -eq 0 ] && [ "$SKIPS" -eq 0 ]; then
    python3 "$BETA_RELEASE_REPORT_HELPER" generate \
      --report-root "$report_dir" \
      --policy "$BETA_RELEASE_POLICY" \
      --output-dir "$report_dir/release-reports"
    python3 "$BETA_RELEASE_REPORT_HELPER" verify \
      --generated-dir "$report_dir/release-reports" \
      --policy "$BETA_RELEASE_POLICY"
  fi
  python3 "$BETA_ATTESTATION_HELPER" update-pointers \
    --report-root "$report_dir" \
    --index-root "$root"
}

container_running() {
  local name="$1"
  docker ps --format '{{.Names}}' 2>/dev/null | grep -Fxq "$name"
}

pbx_provider_enabled() {
  local provider="$1"
  local selected="${BETA_PBX_PROVIDER:-both}"
  case "$selected" in
    both|all) return 0 ;;
    ast|asterisk) [ "$provider" = "asterisk" ] ;;
    fs|free-switch|freeswitch) [ "$provider" = "freeswitch" ] ;;
    *) return 1 ;;
  esac
}

run_local_pbx_gate() {
  local asterisk_dir="${BETA_ASTERISK_DIR:-$HOME/Developer/asterisk}"
  local freeswitch_dir="${BETA_FREESWITCH_DIR:-$HOME/Developer/freeswitch}"
  local pbx_api="${BETA_PBX_API:-all}"
  local pbx_scenario="${BETA_PBX_SCENARIO:-all}"
  local pbx_g729_profiles="${BETA_PBX_G729_PROFILES:-g729a g729ab}"
  local pbx_output_root="$ARTIFACT_DIR/pbx"
  local restore="${BETA_RESTORE_LOCAL_PBX:-1}"
  local initially_asterisk=0
  local initially_freeswitch=0

  if [ ! -x "$asterisk_dir/scripts/up.sh" ] || [ ! -x "$asterisk_dir/scripts/down.sh" ]; then
    skip_gate "local Asterisk PBX matrix" "Asterisk scripts not found under $asterisk_dir."
    return
  fi
  if [ ! -x "$freeswitch_dir/scripts/up.sh" ] || [ ! -x "$freeswitch_dir/scripts/down.sh" ]; then
    skip_gate "local FreeSWITCH PBX matrix" "FreeSWITCH scripts not found under $freeswitch_dir."
    return
  fi

  if container_running rvoip-asterisk; then initially_asterisk=1; fi
  if container_running rvoip-freeswitch; then initially_freeswitch=1; fi
  PBX_RESTORE_ENABLED="$restore"
  PBX_RESTORE_INITIAL_ASTERISK="$initially_asterisk"
  PBX_RESTORE_INITIAL_FREESWITCH="$initially_freeswitch"
  PBX_RESTORE_ASTERISK_DIR="$asterisk_dir"
  PBX_RESTORE_FREESWITCH_DIR="$freeswitch_dir"
  PBX_RESTORE_ARMED=1
  python3 "$BETA_RELEASE_REPORT_HELPER" update-config \
    --policy "$BETA_RELEASE_POLICY" \
    --config "$EFFECTIVE_GATE_CONFIG" \
    --derived "beta_restore_local_pbx=$restore" \
    --derived "beta_restore_asterisk_up=$initially_asterisk" \
    --derived "beta_restore_freeswitch_up=$initially_freeswitch"
  mkdir -p "$pbx_output_root"
  rm -f "$pbx_output_root/matrix.tsv" "$pbx_output_root/summary.md"
  capture_docker_snapshot before-local-pbx

  restore_local_pbx() {
    if [ "$restore" != "1" ]; then
      return
    fi
    if [ "$initially_asterisk" = "1" ]; then
      run_gate_continue "restore local FreeSWITCH down" "$freeswitch_dir/scripts/down.sh"
      run_gate_continue "restore local Asterisk up" "$asterisk_dir/scripts/up.sh"
      capture_docker_snapshot after-restore
    elif [ "$initially_freeswitch" = "1" ]; then
      run_gate_continue "restore local Asterisk down" "$asterisk_dir/scripts/down.sh"
      run_gate_continue "restore local FreeSWITCH up" "$freeswitch_dir/scripts/up.sh"
      capture_docker_snapshot after-restore
    else
      run_gate_continue "restore local Asterisk down" "$asterisk_dir/scripts/down.sh"
      run_gate_continue "restore local FreeSWITCH down" "$freeswitch_dir/scripts/down.sh"
      capture_docker_snapshot after-restore
    fi
  }

  if pbx_provider_enabled asterisk; then
    run_gate_continue "local FreeSWITCH down before Asterisk" "$freeswitch_dir/scripts/down.sh"
    if run_gate "local Asterisk up" "$asterisk_dir/scripts/up.sh"; then
      capture_docker_snapshot after-asterisk-up
      run_gate_continue "local Asterisk PBX matrix" \
        env PBX_OUT_ROOT="$pbx_output_root" \
        PBX_REPORT_APPEND=1 \
        PBX_G729_PROFILES="$pbx_g729_profiles" \
        "$CRATE_DIR/examples/pbx/run.sh" \
        --pbx asterisk --api "$pbx_api" --scenario "$pbx_scenario"
      capture_docker_snapshot after-asterisk-matrix
    fi
    run_gate_continue "local Asterisk down after matrix" "$asterisk_dir/scripts/down.sh"
    capture_docker_snapshot after-asterisk-down
  fi

  if pbx_provider_enabled freeswitch; then
    run_gate_continue "local Asterisk down before FreeSWITCH" "$asterisk_dir/scripts/down.sh"
    if run_gate "local FreeSWITCH up" "$freeswitch_dir/scripts/up.sh"; then
      capture_docker_snapshot after-freeswitch-up
      run_gate_continue "local FreeSWITCH PBX matrix" \
        env PBX_OUT_ROOT="$pbx_output_root" \
        PBX_REPORT_APPEND=1 \
        PBX_G729_PROFILES="$pbx_g729_profiles" \
        "$CRATE_DIR/examples/pbx/run.sh" \
        --pbx freeswitch --api "$pbx_api" --scenario "$pbx_scenario"
      capture_docker_snapshot after-freeswitch-matrix
    fi
    run_gate_continue "local FreeSWITCH down after matrix" "$freeswitch_dir/scripts/down.sh"
    capture_docker_snapshot after-freeswitch-down
  fi

  restore_local_pbx
  PBX_RESTORE_ARMED=0
}

run_local_proxy_pbx_gate() {
  # Kamailio and OpenSIPS registrar-proxy labs with rtpengine media relay
  # (infra/release-runners/pbx/{kamailio,opensips}). Opt-in: the labs pull
  # proxy + rtpengine images and bind host-network SIP/RTP ports, which not
  # every beta environment can do. The lab up.sh scripts gate readiness on
  # the registrar answering AND rtpengine being enabled, and write the env
  # files the harness sources.
  local lifecycle="$WORKSPACE_ROOT/infra/release-runners/interop-lifecycle.sh"
  local pbx_output_root="$ARTIFACT_DIR/pbx"
  local proxy_scenario="${BETA_PROXY_PBX_SCENARIO:-all}"
  local proxy_api="${BETA_PROXY_PBX_API:-endpoint}"

  if [ ! -f "$lifecycle" ]; then
    skip_gate "proxy PBX matrix" "interop-lifecycle.sh not found at $lifecycle."
    return
  fi

  mkdir -p "$pbx_output_root"
  local provider
  local label
  for provider in kamailio opensips; do
    case "$provider" in
      kamailio) label="Kamailio" ;;
      opensips) label="OpenSIPS" ;;
      *) label="$provider" ;;
    esac
    if run_gate "local ${label} lab up" bash "$lifecycle" "${provider}-up"; then
      capture_docker_snapshot "after-${provider}-up"
      run_gate_continue "local ${label} PBX matrix" \
        env PBX_OUT_ROOT="$pbx_output_root" \
        PBX_REPORT_APPEND=1 \
        "$CRATE_DIR/examples/pbx/run.sh" \
        --pbx "$provider" --api "$proxy_api" --scenario "$proxy_scenario"
      capture_docker_snapshot "after-${provider}-matrix"
    fi
    run_gate_continue "local ${label} lab down" bash "$lifecycle" "${provider}-down"
  done
}

start_managed_sipp_target() {
  local host="${BETA_SIPP_TARGET_HOST:-127.0.0.1}"
  local port="${BETA_SIPP_TARGET_PORT:-35060}"
  local sipp_dir="$ARTIFACT_DIR/sipp"
  local listener_log="$sipp_dir/rvoip_perf_listener.log"
  local gate_log="$ARTIFACT_DIR/$(slugify "SIPp standalone target start").log"
  local started_at
  local ended_at
  local start_epoch
  local end_epoch
  local duration
  mkdir -p "$sipp_dir"

  if ! run_gate "SIPp standalone target build" \
    cargo build -p rvoip-sip --release --example perf_listener; then
    return 1
  fi

  echo
  echo "==> SIPp standalone target start"
  started_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  start_epoch="$(date +%s)"
  local perf_profile="${BETA_SIPP_PERF_PROFILE:-pbx-media-server}"
  local recipe_file="${BETA_PERFORMANCE_RECIPE_FILE:-}"
  local listener_cmd=("$WORKSPACE_ROOT/target/release/examples/perf_listener" "$port" "$host" --perf-profile "$perf_profile")
  case "${BETA_SIPP_DIAGNOSTICS:-0}" in
    1|true|TRUE|yes|YES|on|ON)
      listener_cmd+=(--diagnostics)
      ;;
  esac
  if [ -n "$recipe_file" ]; then
    listener_cmd+=(--recipe-file "$recipe_file")
  fi
  {
    echo "gate: SIPp standalone target start"
    echo "started_at_utc: $started_at"
    echo "workspace: $WORKSPACE_ROOT"
    echo "command: ${listener_cmd[*]}"
    echo
  } > "$gate_log"
  {
    echo "managed_by_gate: SIPp standalone target start"
    echo "command: ${listener_cmd[*]}"
  } > "$listener_log"
  "${listener_cmd[@]}" >> "$listener_log" 2>&1 &
  SIPP_LISTENER_PID=$!
  for _ in $(seq 1 100); do
    if grep -q 'listening on' "$listener_log" 2>/dev/null; then
      ended_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
      end_epoch="$(date +%s)"
      duration="$((end_epoch - start_epoch))s"
      {
        echo "listener_evidence: sipp/rvoip_perf_listener.log"
        echo "ended_at_utc: $ended_at"
        echo "duration_seconds: $((end_epoch - start_epoch))"
        echo "exit_status: 0"
      } >> "$gate_log"
      record "PASS" "SIPp standalone target start" "$gate_log" "$duration"
      record_structured_gate "PASS" "SIPp standalone target start" "$gate_log" \
        "$((end_epoch - start_epoch))" "$started_at" "$ended_at" 0 "${listener_cmd[@]}"
      BETA_SIPP_TARGET_HOST="$host"
      BETA_SIPP_TARGET_PORT="$port"
      export BETA_SIPP_TARGET_HOST BETA_SIPP_TARGET_PORT
      return 0
    fi
    if ! kill -0 "$SIPP_LISTENER_PID" >/dev/null 2>&1; then
      ended_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
      end_epoch="$(date +%s)"
      duration="$((end_epoch - start_epoch))s"
      {
        echo "listener_evidence: sipp/rvoip_perf_listener.log"
        echo "ended_at_utc: $ended_at"
        echo "duration_seconds: $((end_epoch - start_epoch))"
        echo "exit_status: 1"
      } >> "$gate_log"
      record "FAIL" "SIPp standalone target start" "$gate_log" "$duration"
      record_structured_gate "FAIL" "SIPp standalone target start" "$gate_log" \
        "$((end_epoch - start_epoch))" "$started_at" "$ended_at" 1 "${listener_cmd[@]}"
      FAILURES=$((FAILURES + 1))
      echo "FAIL: SIPp standalone target exited before listening (see $listener_log)" >&2
      return 1
    fi
    sleep 0.1
  done
  ended_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  end_epoch="$(date +%s)"
  duration="$((end_epoch - start_epoch))s"
  {
    echo "listener_evidence: sipp/rvoip_perf_listener.log"
    echo "ended_at_utc: $ended_at"
    echo "duration_seconds: $((end_epoch - start_epoch))"
    echo "exit_status: 1"
  } >> "$gate_log"
  record "FAIL" "SIPp standalone target start" "$gate_log" "$duration"
  record_structured_gate "FAIL" "SIPp standalone target start" "$gate_log" \
    "$((end_epoch - start_epoch))" "$started_at" "$ended_at" 1 "${listener_cmd[@]}"
  FAILURES=$((FAILURES + 1))
  echo "FAIL: SIPp standalone target did not become ready (see $listener_log)" >&2
  return 1
}

stop_managed_sipp_target() {
  local log="$ARTIFACT_DIR/$(slugify "SIPp standalone target stop").log"
  local listener_log="$ARTIFACT_DIR/sipp/rvoip_perf_listener.log"
  local started_at
  local ended_at
  local start_epoch
  local end_epoch
  local duration
  if [ -z "$SIPP_LISTENER_PID" ]; then
    return 0
  fi
  echo
  echo "==> SIPp standalone target stop"
  started_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  start_epoch="$(date +%s)"
  {
    echo "gate: SIPp standalone target stop"
    echo "started_at_utc: $started_at"
    echo "workspace: $WORKSPACE_ROOT"
    echo "command: managed-sipp-listener-stop"
    echo "listener_evidence: ${listener_log#$ARTIFACT_DIR/}"
    echo
  } > "$log"
  if kill -0 "$SIPP_LISTENER_PID" >/dev/null 2>&1; then
    kill -INT "$SIPP_LISTENER_PID" >/dev/null 2>&1 || true
    wait "$SIPP_LISTENER_PID" >/dev/null 2>&1 || true
  fi
  SIPP_LISTENER_PID=""
  ended_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  end_epoch="$(date +%s)"
  duration="$((end_epoch - start_epoch))s"
  {
    echo "ended_at_utc: $ended_at"
    echo "duration_seconds: $((end_epoch - start_epoch))"
    echo "exit_status: 0"
  } >> "$log"
  record "PASS" "SIPp standalone target stop" "$log" "$duration"
  record_structured_gate "PASS" "SIPp standalone target stop" "$log" \
    "$((end_epoch - start_epoch))" "$started_at" "$ended_at" 0 managed-sipp-listener-stop
}

run_sipp_standalone_gate() {
  if [ "${BETA_RUN_SIPP:-1}" = "0" ]; then
    skip_gate "SIPp standalone matrix" "BETA_RUN_SIPP=0 disables required SIPp evidence."
    return
  fi
  if ! command -v "${SIPP_BIN:-sipp}" >/dev/null 2>&1; then
    run_gate_continue "SIPp standalone matrix" bash -c "echo \"SIPp binary '${SIPP_BIN:-sipp}' not found on PATH\" >&2; exit 1"
    return
  fi

  local managed_target=0
  if [ -z "${BETA_SIPP_TARGET_HOST:-}" ] || [ -z "${BETA_SIPP_TARGET_PORT:-}" ]; then
    managed_target=1
    if ! start_managed_sipp_target; then
      return 0
    fi
  fi

  local cps="${BETA_SIPP_CPS:-30 100 300 1000 2000}"
  run_gate_continue "SIPp standalone matrix" env \
    RVOIP_PERF_RESULTS="$ARTIFACT_DIR/sipp" \
    RVOIP_PERF_CPS="$cps" \
    RVOIP_PERF_MIN_SUCCESS_PCT="${RVOIP_PERF_MIN_SUCCESS_PCT:-99.9}" \
    "$CRATE_DIR/tests/perf/sipp_scenarios/run_comparison.sh" \
    "$BETA_SIPP_TARGET_HOST" "$BETA_SIPP_TARGET_PORT" rvoip

  if [ "$managed_target" = "1" ]; then
    stop_managed_sipp_target
  fi
}

run_proxy_interop_gate() {
  # This release gate is deliberately not optional and never becomes SKIP.
  # Its stable entry point owns prerequisite checks and must fail closed until
  # the complete real-peer matrix (including verified TLS) is implemented.
  run_gate_continue "Kamailio/OpenSIPS stateful-proxy interoperability matrix" \
    env \
      PROXY_INTEROP_ARTIFACT_DIR="$ARTIFACT_DIR/proxy-interop" \
      PROXY_INTEROP_PEERS="kamailio opensips" \
      PROXY_INTEROP_ORDERS="rvoip-first peer-first" \
      PROXY_INTEROP_TRANSPORTS="udp tcp tls" \
      PROXY_INTEROP_RETENTION_DRAIN_SECONDS=130 \
      PROXY_INTEROP_FAIL_FAST=1 \
      PROXY_INTEROP_REQUIRE_CLEAN_SOURCE=1 \
      PROXY_INTEROP_REQUIRE_UNCHANGED_SOURCE=1 \
      PROXY_INTEROP_REQUIRE_PREEXISTING_STATE=1 \
      bash -c '
        set -euo pipefail
        "$1"
        python3 "$2" validate-proxy-interop --report-root "$3"
      ' _ \
        "$PROXY_INTEROP_BETA_GATE" \
        "$BETA_RELEASE_REPORT_HELPER" \
        "$ARTIFACT_DIR"
}

run_dependency_audit() {
  local security_dir="$ARTIFACT_DIR/security"
  mkdir -p "$security_dir"
  cat > "$security_dir/accepted-advisories.md" <<'EOF'
# Accepted Dependency Advisories

- advisory: `RUSTSEC-2023-0071`
- package: `rsa`
- status: accepted beta risk
- reason: RustSec reports no fixed upgrade is available.
- affected paths:
  - `users-core` RS256/JWK support from configured signing keys.
  - `webauthn-rs` transitive crypto via `crypto-glue`.
- beta stance: keep this advisory visible in release evidence and revisit before stable release or when upstream publishes a fixed upgrade path.

- advisories: `RUSTSEC-2026-0185` (`quinn-proto`), `RUSTSEC-2026-0104` / `RUSTSEC-2026-0098` / `RUSTSEC-2026-0099` (`rustls-webpki`)
- status: accepted beta risk
- reason: transitive via the `quinn` (QUIC) and `rustls` stacks; no fixed upgrade adopted in the currently pinned versions.
- beta stance: revisit when the pinned stacks bump `quinn-proto` >= 0.11.15 and `rustls-webpki` to the fixed line.
EOF
  run_gate_continue "dependency advisory audit" env SECURITY_DIR="$security_dir" bash -c '
    set -euo pipefail
    mkdir -p "$SECURITY_DIR"
    if ! cargo audit --version > "$SECURITY_DIR/cargo-audit-version.txt" 2>&1; then
      echo "cargo-audit is not available. Install it with: cargo install cargo-audit" >&2
      exit 127
    fi
    set +e
    cargo audit --ignore RUSTSEC-2023-0071 --ignore RUSTSEC-2026-0185 --ignore RUSTSEC-2026-0104 --ignore RUSTSEC-2026-0098 --ignore RUSTSEC-2026-0099 > "$SECURITY_DIR/cargo-audit.txt" 2>&1
    audit_status=$?
    cargo audit --ignore RUSTSEC-2023-0071 --ignore RUSTSEC-2026-0185 --ignore RUSTSEC-2026-0104 --ignore RUSTSEC-2026-0098 --ignore RUSTSEC-2026-0099 --json > "$SECURITY_DIR/cargo-audit.json" 2> "$SECURITY_DIR/cargo-audit-json.stderr"
    json_status=$?
    set -e
    {
      echo
      echo "Accepted dependency advisory retained for beta evidence:"
      cat "$SECURITY_DIR/accepted-advisories.md"
    } >> "$SECURITY_DIR/cargo-audit.txt"
    cat "$SECURITY_DIR/cargo-audit.txt"
    if [ "$audit_status" -ne 0 ] || [ "$json_status" -ne 0 ]; then
      exit 1
    fi
  '
}

run_fuzz_smoke_target() {
  local target="$1"
  # Optional 2nd arg overrides the fuzz crate dir for this target, so one gate
  # can cover multiple fuzz crates (SIP + media). Defaults to the SIP crate.
  local fuzz_dir="$ARTIFACT_DIR/security/fuzz"
  local fuzz_crate_dir="${2:-${BETA_FUZZ_CRATE_DIR:-$CRATE_DIR/../fuzz}}"
  mkdir -p "$fuzz_dir"
  run_gate_continue "parser fuzz smoke ($target)" env \
    FUZZ_CRATE_DIR="$fuzz_crate_dir" \
    WORKSPACE_ROOT="$WORKSPACE_ROOT" \
    FUZZ_TARGET="$target" \
    FUZZ_LOG="$fuzz_dir/$target.log" \
    BETA_FUZZ_SMOKE_RUNS="${BETA_FUZZ_SMOKE_RUNS:-1000}" \
    BETA_FUZZ_SMOKE_SECONDS="${BETA_FUZZ_SMOKE_SECONDS:-10}" \
    BETA_FUZZ_TOOLCHAIN="${BETA_FUZZ_TOOLCHAIN:-nightly}" \
    bash -c '
      set -euo pipefail
      mkdir -p "$(dirname "$FUZZ_LOG")"
      if ! cargo +"$BETA_FUZZ_TOOLCHAIN" fuzz --version > "${FUZZ_LOG%.log}.version.txt" 2>&1; then
        echo "cargo-fuzz or Rust toolchain '$BETA_FUZZ_TOOLCHAIN' is not available." >&2
        echo "Install with: rustup toolchain install $BETA_FUZZ_TOOLCHAIN && cargo install cargo-fuzz" >&2
        exit 127
      fi
      cd "$WORKSPACE_ROOT"
      set +e
      CARGO_TARGET_DIR="$WORKSPACE_ROOT/target/fuzz" \
        cargo +"$BETA_FUZZ_TOOLCHAIN" fuzz run --fuzz-dir "$FUZZ_CRATE_DIR" "$FUZZ_TARGET" -- \
          -runs="$BETA_FUZZ_SMOKE_RUNS" \
          -max_total_time="$BETA_FUZZ_SMOKE_SECONDS" \
          > "$FUZZ_LOG" 2>&1
      fuzz_status=$?
      set -e
      cat "$FUZZ_LOG"
      exit "$fuzz_status"
    '
}

run_fuzz_smoke_gates() {
  if [ "${BETA_RUN_FUZZ_SMOKE:-1}" = "0" ]; then
    skip_gate "parser fuzz smoke" "BETA_RUN_FUZZ_SMOKE=0 disables required parser fuzz-smoke evidence."
    return
  fi
  # SIP parser fuzz targets (crates/sip/fuzz).
  run_fuzz_smoke_target sip_message
  run_fuzz_smoke_target uri
  run_fuzz_smoke_target header
  run_fuzz_smoke_target sdp
  # RTP / RTCP / SRTP / DTLS / STUN / payload media parser fuzz targets
  # (crates/media/fuzz). The 2nd arg points the gate at that fuzz crate.
  local media_fuzz_dir="$WORKSPACE_ROOT/crates/media/fuzz"
  run_fuzz_smoke_target rtp_packet "$media_fuzz_dir"
  run_fuzz_smoke_target rtcp_packet "$media_fuzz_dir"
  run_fuzz_smoke_target srtp_unprotect "$media_fuzz_dir"
  run_fuzz_smoke_target dtls_record "$media_fuzz_dir"
  run_fuzz_smoke_target stun_response "$media_fuzz_dir"
  run_fuzz_smoke_target g711_unpack "$media_fuzz_dir"
}

run_security_gates() {
  run_dependency_audit || true
  run_fuzz_smoke_gates || true
}

run_downstream_compatibility_gates() {
  # Keep every package/profile in its own Cargo invocation. A combined
  # workspace check can unify optional dependency features across consumers
  # and produce a false compatibility pass that no standalone consumer gets.
  run_gate_continue "downstream rvoip default check" \
    cargo check -p rvoip --lib
  run_gate_continue "downstream rvoip app check" \
    cargo check -p rvoip --lib --no-default-features --features app

  run_gate_continue "downstream rvoip-client default check" \
    cargo check -p rvoip-client --lib
  run_gate_continue "downstream rvoip-client full check" \
    cargo check -p rvoip-client --lib --no-default-features --features full

  # rvoip-core consumes the SIP adapter through its examples, so all targets
  # are required here rather than a library-only check.
  run_gate_continue "downstream rvoip-core check" \
    cargo check -p rvoip-core --all-targets
  run_gate_continue "downstream rvoip-amazon-connect server check" \
    cargo check -p rvoip-amazon-connect --all-targets --features server

  run_gate_continue "downstream rvoip-uctp check" \
    cargo check -p rvoip-uctp --all-targets
  run_gate_continue "downstream rvoip-quic check" \
    cargo check -p rvoip-quic --all-targets
  run_gate_continue "downstream rvoip-webtransport check" \
    cargo check -p rvoip-webtransport --all-targets
  run_gate_continue "downstream rvoip-websocket media and TLS check" \
    cargo check -p rvoip-websocket --all-targets --features media-webrtc,wss

  # These profiles cover the headless shipping surfaces without opting into
  # camera/audio/browser or live-cloud features that need external hardware,
  # credentials, or native SDKs.
  run_gate_continue "downstream rvoip-webrtc interop check" \
    cargo check -p rvoip-webrtc --all-targets \
      --features comprehensive,tls-rustls,bridge-quic
  run_gate_continue "downstream rvoip-audio-device check" \
    cargo check -p rvoip-audio-device --all-targets
}

run_standalone_example_gates() {
  # Each directory is an intentionally detached Cargo workspace. Keep the
  # explicit inventory so adding, dropping, or truncating the release matrix
  # is reviewable; examples 12 and 13 exercise the cross-product gateways.
  local -a examples=(
    01-quickstart-p2p
    02-softphone-audio
    03-register-to-pbx
    04-call-control
    05-blind-transfer
    06-attended-transfer
    07-secure-call-srtp
    08-tls-transport
    09-ivr-server
    10-call-center-b2bua
    11-ai-harness-demo
    12-customer-escalation-sip-webrtc
    13-sip-to-amazon-connect
  )
  local example
  for example in "${examples[@]}"; do
    run_gate_continue "standalone example $example tests" \
      cargo test \
        --manifest-path "$WORKSPACE_ROOT/examples/$example/Cargo.toml" \
        --all-targets
  done
}

run_local_gates() {
  run_gate_continue "format check" cargo fmt --all -- --check
  run_gate_continue "beta evidence helper tests" python3 -m unittest \
    crates/sip/rvoip-sip/scripts/test_beta_attestation.py \
    crates/sip/rvoip-sip/scripts/test_beta_performance_gate_metrics.py \
    crates/sip/rvoip-sip/scripts/test_beta_gate_source.py \
    crates/sip/rvoip-sip/scripts/test_perf_audit.py \
    crates/sip/rvoip-sip/scripts/test_canonical_2k_evidence.py \
    crates/sip/rvoip-sip/scripts/test_perf_2k_acceptance.py \
    crates/sip/rvoip-sip/scripts/test_perf_2k_baseline.py \
    crates/sip/rvoip-sip/scripts/test_perf_regression_baseline.py \
    crates/sip/rvoip-sip/scripts/test_perf_cargo_artifact.py \
    crates/sip/rvoip-sip/scripts/test_docker_peer_snapshot.py \
    crates/sip/sip-proxy/tests/interop/scripts/test_state_snapshot.py
  run_gate_continue "public API compatibility" "$SCRIPT_DIR/check_public_api.sh"
  run_gate_continue "rvoip-sip all-target check" cargo check -p rvoip-sip --all-targets --features generated-validation,dev-insecure-tls
  run_gate_continue "claimed lower-crate check" cargo check \
    -p rvoip-sip-core \
    -p rvoip-sip-transport \
    -p rvoip-sip-dialog \
    -p rvoip-media-core \
    -p rvoip-rtp-core \
    -p rvoip-auth-core \
    -p rvoip-sip-registrar \
    -p rvoip-sip-proxy \
    --all-targets
  run_gate_continue "supporting SIP crate tests" cargo test \
    -p rvoip-auth-core \
    -p rvoip-sip-registrar \
    -p rvoip-sip-proxy \
    --all-targets
  # rtp-core is compile-checked above but its tests (RTP/RTCP/SRTP parsers +
  # the malformed-input regression guards) were not run by the local gate.
  run_gate_continue "rtp-core tests" cargo test -p rvoip-rtp-core --all-targets
  # The release gate must execute every workspace library unit test and
  # doctest, not merely compile downstream crates. Keep rvoip-sip excluded
  # here because its dedicated stages below preserve feature-specific logs and
  # avoid silently replacing the SIP integration contract with feature-unified
  # workspace coverage.
  run_gate_continue "workspace unit tests" cargo test --workspace --exclude rvoip-sip --lib
  run_gate_continue "workspace target and integration tests" cargo test --workspace --exclude rvoip-sip --bins --examples --tests
  run_gate_continue "workspace doctests" cargo test --workspace --exclude rvoip-sip --doc
  run_gate_continue "rvoip-sip unit tests" cargo test -p rvoip-sip --lib
  run_gate_continue "rvoip-sip integration tests" cargo test -p rvoip-sip --tests --features generated-validation,dev-insecure-tls
  run_gate_continue "rvoip-sip doctests" cargo test -p rvoip-sip --doc
  run_gate_continue "rvoip-sip examples compile" cargo build -p rvoip-sip --examples --features dev-insecure-tls
  run_downstream_compatibility_gates
  run_standalone_example_gates
  run_gate_continue "PBX analyzer unit tests" cargo test -p rvoip-sip --example pbx_analyze --features dev-insecure-tls
  run_gate_continue "rvoip-sip rustdoc" env RUSTDOCFLAGS="-D warnings" cargo doc -p rvoip-sip --no-deps --features generated-validation,dev-insecure-tls
  run_gate_continue "sip-core RFC 4475 torture tests" cargo test -p rvoip-sip-core --features lenient_parsing --test torture_tests
  run_gate_continue "sip-core generated message validation" cargo test -p rvoip-sip-core --features generated-validation --test generated_message_compliance
  run_gate_continue "sip dialog generated validation" cargo test -p rvoip-sip-dialog --features generated-validation --test generated_sip_compliance
}

run_interop_gates() {
  if [ "${BETA_RUN_LOCAL_PBX:-0}" = "1" ]; then
    run_local_pbx_gate
  elif [ "${BETA_RUN_PBX:-0}" = "1" ]; then
    run_gate_continue "PBX interop matrix" \
      env PBX_OUT_ROOT="$ARTIFACT_DIR/pbx" \
      PBX_G729_PROFILES="${BETA_PBX_G729_PROFILES:-g729a g729ab}" \
      "$CRATE_DIR/examples/pbx/run.sh" \
      --pbx "${BETA_PBX_PROVIDER:-both}" \
      --api "${BETA_PBX_API:-all}" \
      --scenario "${BETA_PBX_SCENARIO:-all}"
  else
    skip_gate "PBX interop matrix" "Set BETA_RUN_LOCAL_PBX=1 for ~/Developer PBX lifecycle management, or BETA_RUN_PBX=1 after starting PBX containers yourself."
  fi

  if [ "${BETA_RUN_PROXY_PBX:-0}" = "1" ]; then
    run_local_proxy_pbx_gate
  else
    skip_gate "proxy PBX matrix" "Set BETA_RUN_PROXY_PBX=1 to run the Kamailio/OpenSIPS+rtpengine labs (registration, basic_call, AMR passthrough)."
  fi

  run_sipp_standalone_gate

  if [ "${BETA_RUN_STRICT_UA:-1}" = "0" ]; then
    skip_gate "baresip strict-UA matrix" "BETA_RUN_STRICT_UA=0 disables required strict-UA evidence."
  else
    run_gate_continue "baresip strict-UA matrix" env \
      RVOIP_STRICT_UA_RESULTS="$ARTIFACT_DIR/strict-ua" \
      "$CRATE_DIR/tests/interop/baresip/run_strict_ua.sh"
  fi

  run_proxy_interop_gate
}

run_perf_regression_audit() {
  # Verify and package the reviewed immutable baseline before comparing it.
  # Never select a mutable "latest" report directory: a copied attestation must
  # contain the exact bytes that drove the hard regression decision.
  local current="$PERF_RESULTS_DIR"
  if [ ! -d "$current" ] || [ -z "$(ls "$current"/*.json 2>/dev/null)" ]; then
    skip_gate "perf regression audit" "no current perf-results to compare."
    return
  fi
  run_gate_continue "perf regression baseline evidence" \
    python3 "$PERF_REGRESSION_BASELINE_HELPER" package \
      --manifest "$PERF_REGRESSION_BASELINE_MANIFEST" \
      --source-root "$PERF_REGRESSION_BASELINE_ROOT" \
      --artifact-dir "$ARTIFACT_DIR"
  local baseline="$ARTIFACT_DIR/perf-regression-baseline/perf-results"
  if [ ! -f "$ARTIFACT_DIR/perf-regression-baseline/manifest.json" ] \
    || [ ! -d "$baseline" ]; then
    skip_gate "perf regression audit" \
      "the reviewed performance-regression baseline could not be verified and packaged."
    return
  fi
  local tol="${BETA_PERF_REGRESSION_TOLERANCE_PCT:-15}"
  local lat_tol="${BETA_PERF_LATENCY_TOLERANCE_PCT:-25}"
  local out="$ARTIFACT_DIR/perf-audit.md"
  if [ "${BETA_PERF_REGRESSION_FAIL:-0}" = "1" ]; then
    run_gate_continue "perf regression audit" python3 "$SCRIPT_DIR/perf_audit.py" \
      --baseline "$baseline" \
      --baseline-manifest "$ARTIFACT_DIR/perf-regression-baseline/manifest.json" \
      --current "$current" --out "$out" \
      --tolerance-pct "$tol" --latency-tolerance-pct "$lat_tol" \
      --fail-on-regression
  else
    run_gate_continue "perf regression audit" python3 "$SCRIPT_DIR/perf_audit.py" \
      --baseline "$baseline" \
      --baseline-manifest "$ARTIFACT_DIR/perf-regression-baseline/manifest.json" \
      --current "$current" --out "$out" \
      --tolerance-pct "$tol" --latency-tolerance-pct "$lat_tol"
    # Report-only mode still passed the gate above; surface any regression on the
    # console so it is not lost among the PASS rows.
    if grep -q "^status: REGRESSION" "$out" 2>/dev/null; then
      echo "WARNING: perf regression audit flagged degradations vs reviewed baseline $perf_regression_baseline_id (report-only; see perf-audit.md). Set BETA_PERF_REGRESSION_FAIL=1 to gate on it." >&2
    fi
  fi
}

canonical_2k_evidence_requested() {
  bool_env_enabled "${BETA_REQUIRE_CANONICAL_2K_EVIDENCE:-0}" \
    || [ -n "${BETA_CANONICAL_2K_RUN_DIRS:-}" ]
}

run_canonical_2k_evidence_gate() {
  local encoded="${BETA_CANONICAL_2K_RUN_DIRS:-}"
  local -a run_dirs=()
  local -a arguments=(
    import
    --workspace-root "$WORKSPACE_ROOT"
    --beta-start "$BETA_SOURCE_AT_START"
    --artifact-dir "$ARTIFACT_DIR"
  )
  if [ -n "$encoded" ]; then
    IFS=':' read -r -a run_dirs <<< "$encoded"
  fi
  local run_dir
  for run_dir in "${run_dirs[@]}"; do
    if [ -n "$run_dir" ]; then
      arguments+=(--run-dir "$run_dir")
    fi
  done
  run_gate_continue "canonical 2k three-pass evidence" \
    python3 "$CANONICAL_2K_EVIDENCE_HELPER" "${arguments[@]}"
}

run_perf_gates() {
  local profile_spec
  local features
  features="$(perf_features)"
  if [ "${BETA_RUN_PERF_ALL:-0}" = "1" ]; then
    run_gate_continue "literal-all perf configuration" env \
      BETA_RUN_BURST_MATRIX="${BETA_RUN_BURST_MATRIX:-0}" \
      BETA_BURST_MATRIX="${BETA_BURST_MATRIX:-all}" \
      BETA_RUN_LONG_SOAK="${BETA_RUN_LONG_SOAK:-1}" \
      RVOIP_PERF_SKIP_AUDIO_FRAME_DELIVERY="${RVOIP_PERF_SKIP_AUDIO_FRAME_DELIVERY:-0}" \
      bash -c '
        set -euo pipefail
        [ "$BETA_RUN_BURST_MATRIX" = "1" ] || {
          echo "BETA_RUN_PERF_ALL=1 requires BETA_RUN_BURST_MATRIX=1" >&2
          exit 1
        }
        [ "$BETA_BURST_MATRIX" = "all" ] || {
          echo "BETA_RUN_PERF_ALL=1 requires BETA_BURST_MATRIX=all" >&2
          exit 1
        }
        [ "$BETA_RUN_LONG_SOAK" = "1" ] || {
          echo "BETA_RUN_PERF_ALL=1 requires BETA_RUN_LONG_SOAK=1" >&2
          exit 1
        }
        [ "$RVOIP_PERF_SKIP_AUDIO_FRAME_DELIVERY" = "0" ] || {
          echo "BETA_RUN_PERF_ALL=1 requires full app-facing AudioFrame delivery" >&2
          exit 1
        }
      '
  fi
  for profile_spec in $(perf_profile_matrix); do
    local profile="${profile_spec%%:*}"
    local cps="${profile_spec#*:}"
    local perf_env=(
      RVOIP_PERF_PROFILE="$profile"
      RVOIP_PERF_REPORT_SCENARIO="perf_call_setup_cps_${profile}"
      RVOIP_PERF_SWEEP_CPS="$cps"
    )
    if [ -n "${BETA_PERFORMANCE_RECIPE_FILE:-}" ]; then
      perf_env+=(RVOIP_PERF_RECIPE_FILE="$BETA_PERFORMANCE_RECIPE_FILE")
    fi
    run_gate_continue "perf call setup CPS ($profile)" env \
      "${perf_env[@]}" \
      cargo test -p rvoip-sip --release --features "$features" --test perf_call_setup_cps -- --nocapture
  done
  run_gate_continue "perf registration throughput" cargo test -p rvoip-sip --release --features "$features" --test perf_registration_throughput -- --nocapture
  run_gate_continue "perf concurrent active calls" cargo test -p rvoip-sip --release --features "$features" --test perf_concurrent_active_calls -- --nocapture
  run_gate_continue "perf RTP steady state" cargo test -p rvoip-sip --release --features "$features" --test perf_rtp_steady_state -- --nocapture
  run_gate_continue "perf backpressure step" cargo test -p rvoip-sip --release --features "$features" --test perf_backpressure_step -- --nocapture
  run_gate_continue "perf transport recovery" cargo test -p rvoip-sip --release --features "$features" --test perf_transport_recovery -- --nocapture
  if [ "${BETA_RUN_PERF_ALL:-0}" = "1" ]; then
    local all_features
    all_features="$(append_feature "$features" "dev-insecure-tls")"
    # The standard gates above cover call setup, registration, active calls,
    # RTP, backpressure, and transport recovery. These are the remaining
    # registered non-paired perf targets plus the perf-only resiliency target.
    run_gate_continue "all registered resiliency tests" cargo test -p rvoip-sip --release --features "$all_features" --test 'resilien*' -- --nocapture
    run_gate_continue "perf mid-call signaling under media" cargo test -p rvoip-sip --release --features "$all_features" --test perf_mid_call_signal_under_media -- --nocapture
    run_gate_continue "perf TLS overhead" cargo test -p rvoip-sip --release --features "$all_features" --test perf_tls_overhead -- --nocapture
    run_gate_continue "perf SRTP overhead" cargo test -p rvoip-sip --release --features "$all_features" --test perf_srtp_overhead -- --nocapture
    run_gate_continue "perf PDD with 180 first" cargo test -p rvoip-sip --release --features "$all_features" --test perf_pdd_with_180_first -- --nocapture
    run_gate_continue "perf sustained long-duration calls" cargo test -p rvoip-sip --release --features "$all_features" --test perf_sustained_long_duration_calls -- --nocapture
    run_gate_continue "perf registrar binding scale" cargo test -p rvoip-sip --release --features "$all_features" --test perf_registrar_binding_scale -- --nocapture
    run_gate_continue "perf mixed workload" cargo test -p rvoip-sip --release --features "$all_features" --test perf_mixed_workload -- --nocapture
    run_gate_continue "perf B2BUA forwarding" cargo test -p rvoip-sip --release --features "$all_features" --test perf_b2bua_forwarding -- --nocapture
    run_gate_continue "perf AI-agent load" cargo test -p rvoip-sip --release --features "$all_features" --test perf_ai_agent_load -- --nocapture
    run_gate_continue "perf contact-center transfers" cargo test -p rvoip-sip --release --features "$all_features" --test perf_contact_center_transfers -- --nocapture
    run_gate_continue "perf SIPp parity" cargo test -p rvoip-sip --release --features "$all_features" --test perf_sipp_parity -- --nocapture
    # Exercise non-ignored invariant/unit checks in the paired soak targets.
    run_gate_continue "perf soak target invariant tests" cargo test -p rvoip-sip --release --features "$all_features" --test perf_soak_caller --test perf_soak_30min -- --nocapture
    # The remaining ignored paired tests run through the burst and split-soak
    # scripts below. These two ignored standalone tests need explicit gates.
    # Do not let the split-soak duration leak into these standalone targets.
    # They intentionally have independent evidence windows: a short isolated
    # media churn diagnostic and the one-hour monolithic soak.
    run_gate_continue "perf media churn" env \
      RVOIP_PERF_SOAK_DURATION_SECS="${BETA_PERF_MEDIA_CHURN_DURATION_SECS:-120}" \
      RVOIP_PERF_SOAK_ACTIVE_CALLS="${BETA_PERF_MEDIA_CHURN_ACTIVE_CALLS:-30}" \
      cargo test -p rvoip-sip --release --features "$all_features" --test perf_media_churn perf_media_churn -- --exact --ignored --nocapture
    run_gate_continue "perf monolithic soak" env \
      RVOIP_PERF_SOAK_DURATION_SECS="${BETA_PERF_MONOLITHIC_SOAK_DURATION_SECS:-3600}" \
      RVOIP_PERF_SOAK_ACTIVE_CALLS="${BETA_PERF_MONOLITHIC_SOAK_ACTIVE_CALLS:-30}" \
      RVOIP_PERF_SOAK_DRAIN_CPS="${RVOIP_PERF_SOAK_DRAIN_CPS:-10}" \
      RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR="${RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR:-15}" \
      RVOIP_PERF_SOAK_ERROR_SAMPLE_LIMIT="${RVOIP_PERF_SOAK_ERROR_SAMPLE_LIMIT:-32}" \
      RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS="${RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS:-130}" \
      RVOIP_PERF_ARCHIVE_DIR="$ARTIFACT_DIR/perf-results" \
      cargo test -p rvoip-sip --release --features "$all_features" --test perf_soak_30min perf_soak_30min -- --exact --ignored --nocapture
    run_gate_continue "perf mass teardown stress" env \
      RVOIP_PERF_MASS_TEARDOWN_CALLS="${RVOIP_PERF_MASS_TEARDOWN_CALLS:-500}" \
      RVOIP_PERF_MASS_TEARDOWN_SETUP_CPS="${RVOIP_PERF_MASS_TEARDOWN_SETUP_CPS:-30}" \
      RVOIP_PERF_SOAK_ERROR_SAMPLE_LIMIT="${RVOIP_PERF_SOAK_ERROR_SAMPLE_LIMIT:-32}" \
      RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS="$RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS" \
      RVOIP_PERF_ARCHIVE_DIR="$ARTIFACT_DIR/perf-results" \
      cargo test -p rvoip-sip --release --features "$all_features" --test perf_soak_30min perf_mass_teardown_stress -- --exact --ignored --nocapture
  fi
  run_gate_continue "perf session churn leak" env \
    RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS="$RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS" \
    cargo test -p rvoip-sip --release --features "$features" --test perf_soak_30min perf_session_churn_leak -- --ignored --nocapture
  local burst_smoke_covered_by_matrix=0
  if [ "${BETA_RUN_BURST_MATRIX:-0}" = "1" ] &&
     [ "${BETA_BURST_SMOKE_SCENARIOS:-carrier-smoke}" = "carrier-smoke" ]; then
    local burst_matrix_selection
    local burst_scenario
    burst_matrix_selection="${BETA_BURST_MATRIX:-all}"
    burst_matrix_selection="${burst_matrix_selection//,/ }"
    for burst_scenario in $burst_matrix_selection; do
      if [ "$burst_scenario" = "all" ] || [ "$burst_scenario" = "carrier-smoke" ]; then
        burst_smoke_covered_by_matrix=1
        break
      fi
    done
  fi
  if [ "${BETA_RUN_BURST_SMOKE:-1}" = "1" ]; then
    if [ "$burst_smoke_covered_by_matrix" = "1" ]; then
      local burst_smoke_log="$ARTIFACT_DIR/perf-media-burst-smoke.log"
      {
        echo "COVERED: perf media burst smoke"
        echo "The selected full burst matrix includes carrier-smoke; the standalone invocation is coalesced into that gate."
      } > "$burst_smoke_log"
      echo "COVERED: perf media burst smoke - coalesced into perf media burst matrix"
    else
      run_gate_continue "perf media burst smoke" env \
        RVOIP_PERF_FEATURES="$features" \
        RVOIP_PERF_BURST_SCENARIO_FILE="${BETA_BURST_SCENARIO_FILE:-$CRATE_DIR/config/perf-burst-scenarios.yaml}" \
        RVOIP_PERF_BURST_SCENARIOS="${BETA_BURST_SMOKE_SCENARIOS:-carrier-smoke}" \
        RVOIP_PERF_MEMORY_DIAGNOSTICS="${RVOIP_PERF_MEMORY_DIAGNOSTICS:-0}" \
        RVOIP_PERF_ALLOCATOR_DIAGNOSTICS="${RVOIP_PERF_ALLOCATOR_DIAGNOSTICS:-0}" \
        RVOIP_PERF_MEMORY_DIAG_INTERVAL_SECS="${RVOIP_PERF_MEMORY_DIAG_INTERVAL_SECS:-5}" \
        RVOIP_PERF_MIMALLOC_COLLECT_AT="${RVOIP_PERF_MIMALLOC_COLLECT_AT:-off}" \
        RVOIP_PERF_SKIP_AUDIO_FRAME_DELIVERY=0 \
        RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR="${RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR:-15}" \
        "$SCRIPT_DIR/perf_burst_matrix.sh"
    fi
  else
    skip_gate "perf media burst smoke" "BETA_RUN_BURST_SMOKE=0 disables required media burst smoke evidence."
  fi
  if [ "${BETA_RUN_BURST_MATRIX:-0}" = "1" ]; then
    run_gate_continue "perf media burst matrix" env \
      RVOIP_PERF_FEATURES="$features" \
      RVOIP_PERF_BURST_SCENARIO_FILE="${BETA_BURST_SCENARIO_FILE:-$CRATE_DIR/config/perf-burst-scenarios.yaml}" \
      RVOIP_PERF_BURST_SCENARIOS="${BETA_BURST_MATRIX:-all}" \
      RVOIP_PERF_MEMORY_DIAGNOSTICS="${RVOIP_PERF_MEMORY_DIAGNOSTICS:-0}" \
      RVOIP_PERF_ALLOCATOR_DIAGNOSTICS="${RVOIP_PERF_ALLOCATOR_DIAGNOSTICS:-0}" \
      RVOIP_PERF_MEMORY_DIAG_INTERVAL_SECS="${RVOIP_PERF_MEMORY_DIAG_INTERVAL_SECS:-5}" \
      RVOIP_PERF_MIMALLOC_COLLECT_AT="${RVOIP_PERF_MIMALLOC_COLLECT_AT:-off}" \
      RVOIP_PERF_SKIP_AUDIO_FRAME_DELIVERY=0 \
      RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR="${RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR:-15}" \
      "$SCRIPT_DIR/perf_burst_matrix.sh"
  fi
  if [ "${BETA_RUN_LONG_SOAK:-1}" = "1" ]; then
    run_gate_continue "perf soak candidate" env \
      RVOIP_PERF_FEATURES="$features" \
      RVOIP_PERF_SOAK_DURATION_SECS="${RVOIP_PERF_SOAK_DURATION_SECS:-3600}" \
      RVOIP_PERF_SOAK_ACTIVE_CALLS="${RVOIP_PERF_SOAK_ACTIVE_CALLS:-500}" \
      RVOIP_PERF_SOAK_MIN_HOLD_SECS="${RVOIP_PERF_SOAK_MIN_HOLD_SECS:-10}" \
      RVOIP_PERF_SOAK_MAX_HOLD_SECS="${RVOIP_PERF_SOAK_MAX_HOLD_SECS:-360}" \
      RVOIP_PERF_SOAK_CPS="${RVOIP_PERF_SOAK_CPS:-0}" \
      RVOIP_PERF_MEMORY_DIAGNOSTICS="${RVOIP_PERF_MEMORY_DIAGNOSTICS:-0}" \
      RVOIP_PERF_ALLOCATOR_DIAGNOSTICS="${RVOIP_PERF_ALLOCATOR_DIAGNOSTICS:-0}" \
      RVOIP_PERF_MEMORY_DIAG_INTERVAL_SECS="${RVOIP_PERF_MEMORY_DIAG_INTERVAL_SECS:-5}" \
      RVOIP_PERF_MIMALLOC_COLLECT_AT="${RVOIP_PERF_MIMALLOC_COLLECT_AT:-off}" \
      RVOIP_PERF_SYSTEM_ALLOCATOR="${RVOIP_PERF_SYSTEM_ALLOCATOR:-0}" \
      RVOIP_PERF_DHAT="${RVOIP_PERF_DHAT:-0}" \
      RVOIP_PERF_HEAP_SNAPSHOTS="${RVOIP_PERF_HEAP_SNAPSHOTS:-0}" \
      RVOIP_PERF_HEAP_SNAPSHOT_SECS="${RVOIP_PERF_HEAP_SNAPSHOT_SECS:-}" \
      RVOIP_PERF_MALLOC_STACK_LOGGING="${RVOIP_PERF_MALLOC_STACK_LOGGING:-0}" \
      RVOIP_PERF_LEAKS_SNAPSHOTS="${RVOIP_PERF_LEAKS_SNAPSHOTS:-0}" \
      RVOIP_PERF_SKIP_AUDIO_FRAME_DELIVERY="${RVOIP_PERF_SKIP_AUDIO_FRAME_DELIVERY:-0}" \
      RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR="${RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR:-15}" \
      RVOIP_PERF_EXTERNAL_RESOURCE_SAMPLER="${RVOIP_PERF_EXTERNAL_RESOURCE_SAMPLER:-1}" \
      "$SCRIPT_DIR/perf_soak_split.sh"
  else
    skip_gate "perf soak" "BETA_RUN_LONG_SOAK=0 disables release-candidate soak evidence."
  fi

  # Compare this run's perf metrics against the packaged reviewed baseline.
  run_perf_regression_audit
}

run_isolated_perf_gates() {
  run_gate_continue "perf results capture boundary" prepare_perf_results_capture
  if perf_results_capture_ready; then
    run_perf_gates
  else
    skip_gate "performance gates" \
      "The stale-result isolation boundary failed; refusing to analyze or package an unbounded target/perf-results tree."
  fi
}

write_environment_report
env \
  BETA_GATE_REQUIRE_EXTERNAL="$REQUIRE_EXTERNAL" \
  BETA_REQUIRE_CLEAN_SOURCE="$BETA_REQUIRE_CLEAN_SOURCE" \
  BETA_REQUIRE_CANONICAL_2K_EVIDENCE="${BETA_REQUIRE_CANONICAL_2K_EVIDENCE:-0}" \
  python3 "$BETA_RELEASE_REPORT_HELPER" capture-config \
    --policy "$BETA_RELEASE_POLICY" \
    --output "$EFFECTIVE_GATE_CONFIG" \
    --mode "$MODE" \
    --environment-dir "$ARTIFACT_DIR/environment" \
    --derived "beta_attestation_features=$(attestation_features)" \
    --derived "beta_attestation_target=${BETA_ATTESTATION_TARGET:-${CARGO_BUILD_TARGET:-rustc-host}}" \
    --derived "beta_state_table_source=$BETA_STATE_TABLE_SOURCE" \
    --derived "beta_state_table_fallback_reason=${BETA_STATE_TABLE_FALLBACK_REASON:-none}" \
    --derived "beta_state_table_sha256=$selected_state_table_sha256" \
    --derived "beta_profile_matrix=$(perf_profile_matrix)" \
    --derived "beta_perf_features=$(perf_features)" \
    --derived "beta_proxy_interop_peers=kamailio opensips" \
    --derived "beta_proxy_interop_orders=rvoip-first peer-first" \
    --derived "beta_proxy_interop_transports=udp tcp tls" \
    --derived "beta_proxy_interop_retention_drain_seconds=130" \
    --derived "beta_proxy_interop_require_clean_source=1" \
    --derived "beta_proxy_interop_require_unchanged_source=1" \
    --derived "beta_perf_regression_baseline_id=$perf_regression_baseline_id" \
    --derived "beta_perf_regression_baseline_manifest_sha256=$perf_regression_baseline_manifest_sha256"
write_summary_gate_table_header

if bool_env_enabled "$BETA_REQUIRE_CLEAN_SOURCE"; then
  if ! run_gate "clean beta source fingerprint" verify_clean_source_fingerprint; then
    echo "Release-candidate gates require a clean source fingerprint. Set BETA_REQUIRE_CLEAN_SOURCE=0 only for development diagnostics." >&2
  fi
fi

if canonical_2k_evidence_requested; then
  run_canonical_2k_evidence_gate
fi

case "$MODE" in
  local)
    run_local_gates
    ;;
  full)
    run_local_gates
    run_security_gates
    run_interop_gates
    run_isolated_perf_gates
    ;;
  interop)
    run_interop_gates
    ;;
  perf)
    run_isolated_perf_gates
    ;;
  security)
    run_security_gates
    ;;
  *)
    echo "Unknown mode: $MODE" >&2
    exit 2
    ;;
esac

if [ "$MODE" = "full" ] || [ "$MODE" = "perf" ]; then
  if perf_results_capture_ready; then
    run_gate_continue "perf results evidence capture" capture_current_perf_results
    run_gate_continue "performance gate metrics report" \
      write_performance_gate_metrics
  else
    skip_gate "performance gate metrics report" \
      "The isolated performance result capture is unavailable."
  fi
fi

# Capture the terminal source identity for every mode. Clean/full runs also
# execute the fail-closed equality gate below; development modes retain the
# comparison as diagnostic evidence in attestation.json.
run_gate_continue "beta final source fingerprint capture" \
  python3 "$CANONICAL_2K_EVIDENCE_HELPER" fingerprint \
    --workspace-root "$WORKSPACE_ROOT" \
    --out "$BETA_SOURCE_AT_END"

if canonical_2k_evidence_requested; then
  run_gate_continue "canonical 2k beta source unchanged" \
    python3 "$CANONICAL_2K_EVIDENCE_HELPER" verify-source \
      --workspace-root "$WORKSPACE_ROOT" \
      --beta-start "$BETA_SOURCE_AT_START"
elif bool_env_enabled "$BETA_REQUIRE_CLEAN_SOURCE"; then
  run_gate_continue "beta source unchanged" \
    python3 "$CANONICAL_2K_EVIDENCE_HELPER" verify-source \
      --workspace-root "$WORKSPACE_ROOT" \
      --beta-start "$BETA_SOURCE_AT_START"
fi

if ! python3 "$BETA_RELEASE_REPORT_HELPER" finalize-gates \
  --results-dir "$GATE_RESULTS_DIR" \
  --output "$GATE_RESULTS" \
  --mode "$MODE"; then
  FAILURES=$((FAILURES + 1))
  echo "FAIL: structured gate result finalization" >&2
fi

# Keep every gate row contiguous under the summary's `## Gates` heading.
# Attestation parsing intentionally stops at the next level-two section, so
# detailed performance Markdown must follow the terminal source-integrity gates.
if { [ "$MODE" = "full" ] || [ "$MODE" = "perf" ]; } \
  && [ -f "$ARTIFACT_DIR/performance-gate-metrics.md" ]; then
  printf '\n' >> "$SUMMARY"
  cat "$ARTIFACT_DIR/performance-gate-metrics.md" >> "$SUMMARY"
fi

ENDED_AT_UTC="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

if [ -f "$ARTIFACT_DIR/security/accepted-advisories.md" ]; then
  cat >> "$SUMMARY" <<'EOF'

## Accepted Dependency Advisories

- `RUSTSEC-2023-0071` (`rsa`): accepted beta risk because RustSec reports no fixed upgrade.
- `RUSTSEC-2026-0185` (`quinn-proto`), `RUSTSEC-2026-0104`/`-0098`/`-0099` (`rustls-webpki`): accepted; transitive via the `quinn`/`rustls` stacks.
- Affected paths: `users-core` RS256/JWK support and `webauthn-rs` transitive crypto.
- Evidence: `security/accepted-advisories.md`.
EOF
fi

cat >> "$SUMMARY" <<EOF

## Report Package

- enabled: \`${BETA_REPORT_PACKAGE:-1}\`
- report_dir: \`$(beta_report_run_dir)\`
- raw_attestation: \`attestation.json\`
- generic_latest_pointer_informational_only: \`$(beta_report_root)/latest.txt\`
- successful_mode_pointer: \`$(beta_report_mode_pointer)\`
- pointer_policy: mode-specific pointers update only after an independently
  verified PASS with zero skips and the mode's required evidence. Interop
  requires an identified peer; performance requires an executable and result
  JSON; full additionally requires unchanged clean source, an identified peer,
  performance result JSON, and three canonical 2K runs.

## Result

- failures: $FAILURES
- skips: $SKIPS
EOF

write_beta_attestation "$ARTIFACT_DIR"
package_beta_report

echo
echo "Summary: $SUMMARY"
if [ "${BETA_REPORT_PACKAGE:-1}" != "0" ]; then
  echo "Beta report: $(beta_report_run_dir)"
fi
if [ "$FAILURES" -ne 0 ]; then
  exit 1
fi
