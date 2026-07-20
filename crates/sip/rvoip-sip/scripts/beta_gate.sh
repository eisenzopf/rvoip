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
ARTIFACT_DIR="${BETA_GATE_ARTIFACT_DIR:-$WORKSPACE_ROOT/target/beta-gate/$TIMESTAMP}"
SUMMARY="$ARTIFACT_DIR/summary.md"
ENV_REPORT="$ARTIFACT_DIR/environment/environment.md"
BETA_SOURCE_AT_START="$ARTIFACT_DIR/environment/source-at-beta-start.json"
CANONICAL_2K_EVIDENCE_HELPER="$SCRIPT_DIR/canonical_2k_evidence.py"
FAILURES=0
SKIPS=0
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
  BETA_GATE_REQUIRE_EXTERNAL=1   Treat skipped external gates as failures.
  BETA_DENY_WARNINGS=0           Allow Rust warnings during beta gates. Defaults to 1.
  BETA_TEST_LOG_FILTER           Runtime tracing filter for cargo test/build gates.
                                  Defaults to off for clean release evidence.
  BETA_REQUIRE_CLEAN_SOURCE=0    Allow a dirty or changing source fingerprint for a full gate.
                                  Full gates require clean, unchanged source by default; other modes do not.
  BETA_REQUIRE_CANONICAL_2K_EVIDENCE=1
                                  Require exactly three pre-run canonical clean PASS artifacts.
                                  Defaults to 0 for development gates.
  BETA_CANONICAL_2K_RUN_DIRS     Three chronological run directories separated by `:`. Required
                                  when canonical evidence is enabled; paths are copied into the
                                  beta report after fingerprint and gate revalidation.
  BETA_RUN_PBX=1                 Run examples/pbx/run.sh when PBX configs are present.
  BETA_RUN_LOCAL_PBX=1           Manage ~/Developer/asterisk and ~/Developer/freeswitch sequentially.
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
  BETA_PERF_MONOLITHIC_SOAK_DURATION_SECS
                                  Legacy monolithic-soak duration. Defaults to 1800 seconds.
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
  BETA_RUN_LONG_SOAK=0           Disable the ignored soak test; fails with --require-external.
  BETA_PERF_REGRESSION_FAIL=1    Make a perf regression vs the previous run a hard gate failure. Default 0 (report-only + perf-audit.md).
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
  RVOIP_PERF_MIMALLOC_COLLECT_AT Optional diagnostic mi_collect(true): off|phase|drain|both.
                                  Defaults to off in the beta gate.
  RVOIP_PERF_SYSTEM_ALLOCATOR=1  Build perf soak with the system allocator instead of mimalloc.
  RVOIP_PERF_DHAT=1              Build split soak with DHAT heap profiling allocator.
  RVOIP_PERF_HEAP_SNAPSHOTS=1    Capture per-process vmmap snapshots during split soak.
  RVOIP_PERF_HEAP_SNAPSHOT_SECS  Optional comma list of label:seconds or seconds snapshot offsets.
  RVOIP_PERF_MALLOC_STACK_LOGGING=1
                                  Enable macOS MallocStackLogging for child soak processes.
  RVOIP_PERF_LEAKS_SNAPSHOTS=1   Also run macOS leaks at heap snapshot offsets.
  RVOIP_PERF_SKIP_AUDIO_FRAME_DELIVERY=1
                                  Decode RTP media but skip app-facing AudioFrame delivery.
  RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR
                                  Soak RSS growth threshold. Defaults to Config's 10 MB/hr.
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
    return 0
  else
    record "FAIL" "$name" "$log" "$duration"
    FAILURES=$((FAILURES + 1))
    echo "FAIL: $name (see $log)" >&2
    return 1
  fi
}

skip_gate() {
  local name="$1"
  local reason="$2"
  local log="$ARTIFACT_DIR/$(slugify "$name").log"
  {
    echo "SKIP: $name"
    echo "$reason"
  } > "$log"
  record "SKIP" "$name" "$log" "-"
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
    /^(BETA_|PBX_|RVOIP_|SIPP_|ASTERISK_|FREESWITCH_|SIP_|TLS_)/ {
      key=$1
      value=substr($0, length($1) + 2)
      redacted=key
      upper=toupper(key)
      if (upper ~ /(PASSWORD|PASS|SECRET|TOKEN|CREDENTIAL|PRIVATE|AUTHORIZATION)/) {
        print key"=<redacted>"
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
      capture_command "$dir/${container}-inspect.json" docker inspect "$container"
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
  if [ -d "$dir/.git" ]; then
    capture_command "$out/git-rev.txt" git -C "$dir" rev-parse --short HEAD
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
- beta_deny_warnings: \`${BETA_DENY_WARNINGS:-1}\`
- beta_test_log_filter: \`${BETA_TEST_LOG_FILTER:-off}\`
- source_at_beta_start: \`environment/source-at-beta-start.json\`
- beta_require_clean_source: \`${BETA_REQUIRE_CLEAN_SOURCE}\`
- beta_require_canonical_2k_evidence: \`${BETA_REQUIRE_CANONICAL_2K_EVIDENCE:-0}\`
- host: \`$(captured_first_line "$env_dir/host-uname.txt")\`
- colima: \`$(captured_first_line "$env_dir/colima-status.txt")\`
- docker: \`$(captured_first_line "$env_dir/docker-version.txt")\`
- beta_perf_features: \`$(perf_features)\`
- beta_perf_infra_memory_diagnostics: \`${BETA_PERF_INFRA_MEMORY_DIAGNOSTICS:-0}\`
- beta_perf_media_diagnostics: \`${BETA_PERF_MEDIA_DIAGNOSTICS:-0}\`
- beta_perf_media_memory_diagnostics: \`${BETA_PERF_MEDIA_MEMORY_DIAGNOSTICS:-0}\`
- beta_perf_rtp_memory_diagnostics: \`${BETA_PERF_RTP_MEMORY_DIAGNOSTICS:-0}\`

Docker snapshots captured during local PBX lifecycle events are stored under
\`environment/docker-<phase>/\`. Secrets in copied local env/config files are
redacted by key name before being written into this artifact tree.
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
- beta_deny_warnings: \`${BETA_DENY_WARNINGS:-1}\`
- beta_test_log_filter: \`${BETA_TEST_LOG_FILTER:-off}\`
- source_at_beta_start: \`environment/source-at-beta-start.json\`
- beta_require_clean_source: \`${BETA_REQUIRE_CLEAN_SOURCE}\`
- beta_require_canonical_2k_evidence: \`${BETA_REQUIRE_CANONICAL_2K_EVIDENCE:-0}\`
- beta_canonical_2k_run_dirs: \`${BETA_CANONICAL_2K_RUN_DIRS:-not supplied}\`
- host: \`$(captured_first_line "$env_dir/host-uname.txt")\`
- colima: \`$(captured_first_line "$env_dir/colima-status.txt")\`
- docker: \`$(captured_first_line "$env_dir/docker-version.txt")\`
- beta_profile_matrix: \`$(perf_profile_matrix)\`
- beta_run_perf_all: \`${BETA_RUN_PERF_ALL:-0}\`
- beta_perf_media_churn_duration_secs: \`${BETA_PERF_MEDIA_CHURN_DURATION_SECS:-120}\`
- beta_perf_monolithic_soak_duration_secs: \`${BETA_PERF_MONOLITHIC_SOAK_DURATION_SECS:-1800}\`
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
- beta_run_sipp: \`${BETA_RUN_SIPP:-1}\`
- beta_sipp_diagnostics: \`${BETA_SIPP_DIAGNOSTICS:-0}\`
- beta_run_strict_ua: \`${BETA_RUN_STRICT_UA:-1}\`
- beta_run_long_soak: \`${BETA_RUN_LONG_SOAK:-1}\`
- rvoip_perf_soak_duration_secs: \`${RVOIP_PERF_SOAK_DURATION_SECS:-3600}\`
- rvoip_perf_soak_active_calls: \`${RVOIP_PERF_SOAK_ACTIVE_CALLS:-500}\`
- rvoip_perf_soak_min_hold_secs: \`${RVOIP_PERF_SOAK_MIN_HOLD_SECS:-10}\`
- rvoip_perf_soak_max_hold_secs: \`${RVOIP_PERF_SOAK_MAX_HOLD_SECS:-360}\`
- rvoip_perf_soak_cps: \`${RVOIP_PERF_SOAK_CPS:-0}\`
- rvoip_perf_soak_drain_cps: \`${RVOIP_PERF_SOAK_DRAIN_CPS:-10}\`
- rvoip_perf_soak_error_sample_limit: \`${RVOIP_PERF_SOAK_ERROR_SAMPLE_LIMIT:-32}\`
- rvoip_perf_retention_drain_wait_secs: \`${RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS:-120}\`
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
- rvoip_perf_max_rss_growth_mb_per_hr: \`${RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR:-Config default (10)}\`
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
- generated_at_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)

## Primary Evidence

- \`summary.md\`
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
- \`perf-audit.md\` (current-vs-previous perf regression audit)
- \`canonical-2k/index.json\` and \`canonical-2k/run-{1,2,3}/\`
  (required release evidence when enabled)

The report directory is a packaged copy of the beta-gate artifact tree plus
the current raw perf result files. Logs, matrices, redacted
environment evidence, PBX lifecycle snapshots, scenario metadata, and perf
JSON/markdown outputs are kept with their original relative paths where
possible.

Perf results package status: ${perf_results_status}
EOF
}

copy_perf_results_into_report() {
  local report_dir="$1"
  local source="$WORKSPACE_ROOT/target/perf-results"

  if [ ! -d "$source" ]; then
    printf 'not packaged; no perf-results directory found under %s/target' "$WORKSPACE_ROOT"
    return 0
  fi

  mkdir -p "$report_dir/perf-results"
  (cd "$source" && tar cf - .) | (cd "$report_dir/perf-results" && tar xf -)
  printf 'packaged from: %s' "$source"
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
  printf '%s\n' "$TIMESTAMP" > "$root/latest.txt"
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
  mkdir -p "$pbx_output_root"
  rm -f "$pbx_output_root/matrix.tsv" "$pbx_output_root/summary.md"
  capture_docker_snapshot before-local-pbx

  restore_local_pbx() {
    if [ "$restore" != "1" ]; then
      return
    fi
    if [ "$initially_asterisk" = "1" ]; then
      run_gate "restore local FreeSWITCH down" "$freeswitch_dir/scripts/down.sh" || true
      run_gate "restore local Asterisk up" "$asterisk_dir/scripts/up.sh" || true
      capture_docker_snapshot after-restore
    elif [ "$initially_freeswitch" = "1" ]; then
      run_gate "restore local Asterisk down" "$asterisk_dir/scripts/down.sh" || true
      run_gate "restore local FreeSWITCH up" "$freeswitch_dir/scripts/up.sh" || true
      capture_docker_snapshot after-restore
    else
      run_gate "restore local Asterisk down" "$asterisk_dir/scripts/down.sh" || true
      run_gate "restore local FreeSWITCH down" "$freeswitch_dir/scripts/down.sh" || true
      capture_docker_snapshot after-restore
    fi
  }

  if pbx_provider_enabled asterisk; then
    run_gate "local FreeSWITCH down before Asterisk" "$freeswitch_dir/scripts/down.sh" || true
    if run_gate "local Asterisk up" "$asterisk_dir/scripts/up.sh"; then
      capture_docker_snapshot after-asterisk-up
      run_gate "local Asterisk PBX matrix" \
        env PBX_OUT_ROOT="$pbx_output_root" \
        PBX_REPORT_APPEND=1 \
        PBX_G729_PROFILES="$pbx_g729_profiles" \
        "$CRATE_DIR/examples/pbx/run.sh" \
        --pbx asterisk --api "$pbx_api" --scenario "$pbx_scenario" || true
      capture_docker_snapshot after-asterisk-matrix
    fi
    run_gate "local Asterisk down after matrix" "$asterisk_dir/scripts/down.sh" || true
    capture_docker_snapshot after-asterisk-down
  fi

  if pbx_provider_enabled freeswitch; then
    run_gate "local Asterisk down before FreeSWITCH" "$asterisk_dir/scripts/down.sh" || true
    if run_gate "local FreeSWITCH up" "$freeswitch_dir/scripts/up.sh"; then
      capture_docker_snapshot after-freeswitch-up
      run_gate "local FreeSWITCH PBX matrix" \
        env PBX_OUT_ROOT="$pbx_output_root" \
        PBX_REPORT_APPEND=1 \
        PBX_G729_PROFILES="$pbx_g729_profiles" \
        "$CRATE_DIR/examples/pbx/run.sh" \
        --pbx freeswitch --api "$pbx_api" --scenario "$pbx_scenario" || true
      capture_docker_snapshot after-freeswitch-matrix
    fi
    run_gate "local FreeSWITCH down after matrix" "$freeswitch_dir/scripts/down.sh" || true
    capture_docker_snapshot after-freeswitch-down
  fi

  restore_local_pbx
  PBX_RESTORE_ARMED=0
}

start_managed_sipp_target() {
  local host="${BETA_SIPP_TARGET_HOST:-127.0.0.1}"
  local port="${BETA_SIPP_TARGET_PORT:-35060}"
  local sipp_dir="$ARTIFACT_DIR/sipp"
  local log="$sipp_dir/rvoip_perf_listener.log"
  local started_at
  local start_epoch
  local duration
  mkdir -p "$sipp_dir"

  run_gate "SIPp standalone target build" cargo build -p rvoip-sip --release --example perf_listener

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
  } > "$log"
  "${listener_cmd[@]}" >> "$log" 2>&1 &
  SIPP_LISTENER_PID=$!
  for _ in $(seq 1 100); do
    if grep -q 'listening on' "$log" 2>/dev/null; then
      duration="$(($(date +%s) - start_epoch))s"
      record "PASS" "SIPp standalone target start" "$log" "$duration"
      BETA_SIPP_TARGET_HOST="$host"
      BETA_SIPP_TARGET_PORT="$port"
      export BETA_SIPP_TARGET_HOST BETA_SIPP_TARGET_PORT
      return 0
    fi
    if ! kill -0 "$SIPP_LISTENER_PID" >/dev/null 2>&1; then
      duration="$(($(date +%s) - start_epoch))s"
      record "FAIL" "SIPp standalone target start" "$log" "$duration"
      FAILURES=$((FAILURES + 1))
      echo "FAIL: SIPp standalone target exited before listening (see $log)" >&2
      return 1
    fi
    sleep 0.1
  done
  duration="$(($(date +%s) - start_epoch))s"
  record "FAIL" "SIPp standalone target start" "$log" "$duration"
  FAILURES=$((FAILURES + 1))
  echo "FAIL: SIPp standalone target did not become ready (see $log)" >&2
  return 1
}

stop_managed_sipp_target() {
  local log="$ARTIFACT_DIR/sipp/rvoip_perf_listener.log"
  local start_epoch
  local duration
  if [ -z "$SIPP_LISTENER_PID" ]; then
    return 0
  fi
  echo
  echo "==> SIPp standalone target stop"
  start_epoch="$(date +%s)"
  if kill -0 "$SIPP_LISTENER_PID" >/dev/null 2>&1; then
    kill -INT "$SIPP_LISTENER_PID" >/dev/null 2>&1 || true
    wait "$SIPP_LISTENER_PID" >/dev/null 2>&1 || true
  fi
  SIPP_LISTENER_PID=""
  duration="$(($(date +%s) - start_epoch))s"
  record "PASS" "SIPp standalone target stop" "$log" "$duration"
}

run_sipp_standalone_gate() {
  if [ "${BETA_RUN_SIPP:-1}" = "0" ]; then
    skip_gate "SIPp standalone matrix" "BETA_RUN_SIPP=0 disables required SIPp evidence."
    return
  fi
  if ! command -v "${SIPP_BIN:-sipp}" >/dev/null 2>&1; then
    run_gate "SIPp standalone matrix" bash -c "echo \"SIPp binary '${SIPP_BIN:-sipp}' not found on PATH\" >&2; exit 1"
    return
  fi

  local managed_target=0
  if [ -z "${BETA_SIPP_TARGET_HOST:-}" ] || [ -z "${BETA_SIPP_TARGET_PORT:-}" ]; then
    managed_target=1
    start_managed_sipp_target
  fi

  local cps="${BETA_SIPP_CPS:-30 100 300 1000 2000}"
  run_gate "SIPp standalone matrix" env \
    RVOIP_PERF_RESULTS="$ARTIFACT_DIR/sipp" \
    RVOIP_PERF_CPS="$cps" \
    RVOIP_PERF_MIN_SUCCESS_PCT="${RVOIP_PERF_MIN_SUCCESS_PCT:-99.9}" \
    "$CRATE_DIR/tests/perf/sipp_scenarios/run_comparison.sh" \
    "$BETA_SIPP_TARGET_HOST" "$BETA_SIPP_TARGET_PORT" rvoip

  if [ "$managed_target" = "1" ]; then
    stop_managed_sipp_target
  fi
}

run_proxy_descope_audit() {
  run_gate "Kamailio/OpenSIPS proxy de-scope audit" bash -c \
    "set -euo pipefail
     rg -q 'Kamailio/OpenSIPS.*planned validation targets, not release' crates/sip/rvoip-sip/README.md
     rg -q 'Kamailio/OpenSIPS plus RTPengine.*Investigation' crates/sip/rvoip-sip/docs/TOPOLOGY_PROFILES.md
     rg -q 'Kamailio/OpenSIPS.*Investigation only' crates/sip/rvoip-sip/docs/INTEROP_CI_PLAN.md"
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
  run_gate "dependency advisory audit" env SECURITY_DIR="$security_dir" bash -c '
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
  run_gate "parser fuzz smoke ($target)" env \
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

run_local_gates() {
  run_gate "format check" cargo fmt --all -- --check
  run_gate "beta evidence helper tests" python3 -m unittest \
    crates/sip/rvoip-sip/scripts/test_perf_audit.py \
    crates/sip/rvoip-sip/scripts/test_canonical_2k_evidence.py \
    crates/sip/rvoip-sip/scripts/test_perf_2k_acceptance.py \
    crates/sip/rvoip-sip/scripts/test_perf_cargo_artifact.py
  run_gate "rvoip-sip all-target check" cargo check -p rvoip-sip --all-targets --features generated-validation,dev-insecure-tls
  run_gate "claimed lower-crate check" cargo check \
    -p rvoip-sip-core \
    -p rvoip-sip-transport \
    -p rvoip-sip-dialog \
    -p rvoip-media-core \
    -p rvoip-rtp-core \
    -p rvoip-auth-core \
    -p rvoip-sip-registrar \
    -p rvoip-sip-proxy \
    --all-targets
  run_gate "supporting SIP crate tests" cargo test \
    -p rvoip-auth-core \
    -p rvoip-sip-registrar \
    -p rvoip-sip-proxy \
    --all-targets
  # rtp-core is compile-checked above but its tests (RTP/RTCP/SRTP parsers +
  # the malformed-input regression guards) were not run by the local gate.
  run_gate "rtp-core tests" cargo test -p rvoip-rtp-core --all-targets
  run_gate "rvoip-sip unit tests" cargo test -p rvoip-sip --lib
  run_gate "rvoip-sip integration tests" cargo test -p rvoip-sip --tests --features generated-validation,dev-insecure-tls
  run_gate "rvoip-sip doctests" cargo test -p rvoip-sip --doc
  run_gate "rvoip-sip examples compile" cargo build -p rvoip-sip --examples --features dev-insecure-tls
  run_gate "PBX analyzer unit tests" cargo test -p rvoip-sip --example pbx_analyze --features dev-insecure-tls
  run_gate "rvoip-sip rustdoc" env RUSTDOCFLAGS="-D warnings" cargo doc -p rvoip-sip --no-deps --features generated-validation,dev-insecure-tls
  run_gate "sip-core RFC 4475 torture tests" cargo test -p rvoip-sip-core --features lenient_parsing --test torture_tests
  run_gate "sip-core generated message validation" cargo test -p rvoip-sip-core --features generated-validation --test generated_message_compliance
  run_gate "sip dialog generated validation" cargo test -p rvoip-sip-dialog --features generated-validation --test generated_sip_compliance
}

run_interop_gates() {
  if [ "${BETA_RUN_LOCAL_PBX:-0}" = "1" ]; then
    run_local_pbx_gate
  elif [ "${BETA_RUN_PBX:-0}" = "1" ]; then
    run_gate "PBX interop matrix" \
      env PBX_G729_PROFILES="${BETA_PBX_G729_PROFILES:-g729a g729ab}" \
      "$CRATE_DIR/examples/pbx/run.sh" --pbx both --api all --scenario all
  else
    skip_gate "PBX interop matrix" "Set BETA_RUN_LOCAL_PBX=1 for ~/Developer PBX lifecycle management, or BETA_RUN_PBX=1 after starting PBX containers yourself."
  fi

  run_sipp_standalone_gate

  if [ "${BETA_RUN_STRICT_UA:-1}" = "0" ]; then
    skip_gate "baresip strict-UA matrix" "BETA_RUN_STRICT_UA=0 disables required strict-UA evidence."
  else
    run_gate "baresip strict-UA matrix" env \
      RVOIP_STRICT_UA_RESULTS="$ARTIFACT_DIR/strict-ua" \
      "$CRATE_DIR/tests/interop/baresip/run_strict_ua.sh"
  fi

  run_proxy_descope_audit
}

run_perf_regression_audit() {
  # Audit this run's perf JSON against the most recent prior beta-report run and
  # flag degradations beyond tolerance. Report-only by default (a WARN that still
  # passes the gate) so dev-box run-to-run variance does not block releases; set
  # BETA_PERF_REGRESSION_FAIL=1 to make a regression a hard failure (e.g. on a
  # dedicated perf host). Either way perf-audit.md is written into the report.
  local current="$WORKSPACE_ROOT/target/perf-results"
  if [ ! -d "$current" ] || [ -z "$(ls "$current"/*.json 2>/dev/null)" ]; then
    skip_gate "perf regression audit" "no current perf-results to compare."
    return
  fi
  # The current run's report package is written only at the end of the gate, so
  # every match here is a prior run; the newest one with perf JSON is the baseline.
  local baseline=""
  local d
  # Sort by directory name (ISO timestamp) descending, so the newest prior run
  # wins regardless of mtime changes from copying/restoring report packages.
  for d in $(ls -d "$(beta_report_root)"/*/perf-results 2>/dev/null | sort -r); do
    if [ -n "$(ls "$d"/*.json 2>/dev/null)" ]; then
      baseline="$d"
      break
    fi
  done
  if [ -z "$baseline" ]; then
    skip_gate "perf regression audit" \
      "no prior beta-report run with perf-results; this run establishes the baseline."
    return
  fi
  local tol="${BETA_PERF_REGRESSION_TOLERANCE_PCT:-15}"
  local lat_tol="${BETA_PERF_LATENCY_TOLERANCE_PCT:-25}"
  local out="$ARTIFACT_DIR/perf-audit.md"
  if [ "${BETA_PERF_REGRESSION_FAIL:-0}" = "1" ]; then
    run_gate "perf regression audit" python3 "$SCRIPT_DIR/perf_audit.py" \
      --baseline "$baseline" --current "$current" --out "$out" \
      --tolerance-pct "$tol" --latency-tolerance-pct "$lat_tol" \
      --fail-on-regression
  else
    run_gate "perf regression audit" python3 "$SCRIPT_DIR/perf_audit.py" \
      --baseline "$baseline" --current "$current" --out "$out" \
      --tolerance-pct "$tol" --latency-tolerance-pct "$lat_tol"
    # Report-only mode still passed the gate above; surface any regression on the
    # console so it is not lost among the PASS rows.
    if grep -q "^status: REGRESSION" "$out" 2>/dev/null; then
      echo "WARNING: perf regression audit flagged degradations vs $baseline (report-only; see perf-audit.md). Set BETA_PERF_REGRESSION_FAIL=1 to gate on it." >&2
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
  run_gate "canonical 2k three-pass evidence" \
    python3 "$CANONICAL_2K_EVIDENCE_HELPER" "${arguments[@]}"
}

run_perf_gates() {
  local profile_spec
  local features
  features="$(perf_features)"
  if [ "${BETA_RUN_PERF_ALL:-0}" = "1" ]; then
    run_gate "literal-all perf configuration" env \
      BETA_RUN_BURST_MATRIX="${BETA_RUN_BURST_MATRIX:-0}" \
      BETA_BURST_MATRIX="${BETA_BURST_MATRIX:-all}" \
      BETA_RUN_LONG_SOAK="${BETA_RUN_LONG_SOAK:-1}" \
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
    run_gate "perf call setup CPS ($profile)" env \
      "${perf_env[@]}" \
      cargo test -p rvoip-sip --release --features "$features" --test perf_call_setup_cps -- --nocapture
  done
  run_gate "perf registration throughput" cargo test -p rvoip-sip --release --features "$features" --test perf_registration_throughput -- --nocapture
  run_gate "perf concurrent active calls" cargo test -p rvoip-sip --release --features "$features" --test perf_concurrent_active_calls -- --nocapture
  run_gate "perf RTP steady state" cargo test -p rvoip-sip --release --features "$features" --test perf_rtp_steady_state -- --nocapture
  run_gate "perf backpressure step" cargo test -p rvoip-sip --release --features "$features" --test perf_backpressure_step -- --nocapture
  run_gate "perf transport recovery" cargo test -p rvoip-sip --release --features "$features" --test perf_transport_recovery -- --nocapture
  if [ "${BETA_RUN_PERF_ALL:-0}" = "1" ]; then
    local all_features
    all_features="$(append_feature "$features" "dev-insecure-tls")"
    # The standard gates above cover call setup, registration, active calls,
    # RTP, backpressure, and transport recovery. These are the remaining
    # registered non-paired perf targets plus the perf-only resiliency target.
    run_gate "all registered resiliency tests" cargo test -p rvoip-sip --release --features "$all_features" --test 'resilien*' -- --nocapture
    run_gate "perf mid-call signaling under media" cargo test -p rvoip-sip --release --features "$all_features" --test perf_mid_call_signal_under_media -- --nocapture
    run_gate "perf TLS overhead" cargo test -p rvoip-sip --release --features "$all_features" --test perf_tls_overhead -- --nocapture
    run_gate "perf SRTP overhead" cargo test -p rvoip-sip --release --features "$all_features" --test perf_srtp_overhead -- --nocapture
    run_gate "perf PDD with 180 first" cargo test -p rvoip-sip --release --features "$all_features" --test perf_pdd_with_180_first -- --nocapture
    run_gate "perf sustained long-duration calls" cargo test -p rvoip-sip --release --features "$all_features" --test perf_sustained_long_duration_calls -- --nocapture
    run_gate "perf registrar binding scale" cargo test -p rvoip-sip --release --features "$all_features" --test perf_registrar_binding_scale -- --nocapture
    run_gate "perf mixed workload" cargo test -p rvoip-sip --release --features "$all_features" --test perf_mixed_workload -- --nocapture
    run_gate "perf B2BUA forwarding" cargo test -p rvoip-sip --release --features "$all_features" --test perf_b2bua_forwarding -- --nocapture
    run_gate "perf AI-agent load" cargo test -p rvoip-sip --release --features "$all_features" --test perf_ai_agent_load -- --nocapture
    run_gate "perf contact-center transfers" cargo test -p rvoip-sip --release --features "$all_features" --test perf_contact_center_transfers -- --nocapture
    run_gate "perf SIPp parity" cargo test -p rvoip-sip --release --features "$all_features" --test perf_sipp_parity -- --nocapture
    # Exercise non-ignored invariant/unit checks in the paired soak targets.
    run_gate "perf soak target invariant tests" cargo test -p rvoip-sip --release --features "$all_features" --test perf_soak_caller --test perf_soak_30min -- --nocapture
    # The remaining ignored paired tests run through the burst and split-soak
    # scripts below. These two ignored standalone tests need explicit gates.
    # Do not let the split-soak duration leak into these standalone targets.
    # They intentionally have independent evidence windows: a short isolated
    # media churn diagnostic and the legacy 30-minute monolithic soak.
    run_gate "perf media churn" env \
      RVOIP_PERF_SOAK_DURATION_SECS="${BETA_PERF_MEDIA_CHURN_DURATION_SECS:-120}" \
      cargo test -p rvoip-sip --release --features "$all_features" --test perf_media_churn perf_media_churn -- --exact --ignored --nocapture
    run_gate "perf monolithic soak" env \
      RVOIP_PERF_SOAK_DURATION_SECS="${BETA_PERF_MONOLITHIC_SOAK_DURATION_SECS:-1800}" \
      RVOIP_PERF_SOAK_DRAIN_CPS="${RVOIP_PERF_SOAK_DRAIN_CPS:-10}" \
      RVOIP_PERF_SOAK_ERROR_SAMPLE_LIMIT="${RVOIP_PERF_SOAK_ERROR_SAMPLE_LIMIT:-32}" \
      RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS="${RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS:-120}" \
      RVOIP_PERF_ARCHIVE_DIR="$ARTIFACT_DIR/perf-results" \
      cargo test -p rvoip-sip --release --features "$all_features" --test perf_soak_30min perf_soak_30min -- --exact --ignored --nocapture
    run_gate "perf mass teardown stress" env \
      RVOIP_PERF_MASS_TEARDOWN_CALLS="${RVOIP_PERF_MASS_TEARDOWN_CALLS:-500}" \
      RVOIP_PERF_MASS_TEARDOWN_SETUP_CPS="${RVOIP_PERF_MASS_TEARDOWN_SETUP_CPS:-30}" \
      RVOIP_PERF_SOAK_ERROR_SAMPLE_LIMIT="${RVOIP_PERF_SOAK_ERROR_SAMPLE_LIMIT:-32}" \
      RVOIP_PERF_ARCHIVE_DIR="$ARTIFACT_DIR/perf-results" \
      cargo test -p rvoip-sip --release --features "$all_features" --test perf_soak_30min perf_mass_teardown_stress -- --exact --ignored --nocapture
  fi
  run_gate "perf session churn leak" cargo test -p rvoip-sip --release --features "$features" --test perf_soak_30min perf_session_churn_leak -- --ignored --nocapture
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
      record "COVERED" "perf media burst smoke" "$burst_smoke_log" "-"
      echo "COVERED: perf media burst smoke - coalesced into perf media burst matrix"
    else
      run_gate "perf media burst smoke" env \
        RVOIP_PERF_FEATURES="$features" \
        RVOIP_PERF_BURST_SCENARIO_FILE="${BETA_BURST_SCENARIO_FILE:-$CRATE_DIR/config/perf-burst-scenarios.yaml}" \
        RVOIP_PERF_BURST_SCENARIOS="${BETA_BURST_SMOKE_SCENARIOS:-carrier-smoke}" \
        RVOIP_PERF_MEMORY_DIAGNOSTICS="${RVOIP_PERF_MEMORY_DIAGNOSTICS:-0}" \
        RVOIP_PERF_ALLOCATOR_DIAGNOSTICS="${RVOIP_PERF_ALLOCATOR_DIAGNOSTICS:-0}" \
        RVOIP_PERF_MEMORY_DIAG_INTERVAL_SECS="${RVOIP_PERF_MEMORY_DIAG_INTERVAL_SECS:-5}" \
        RVOIP_PERF_MIMALLOC_COLLECT_AT="${RVOIP_PERF_MIMALLOC_COLLECT_AT:-off}" \
        "$SCRIPT_DIR/perf_burst_matrix.sh"
    fi
  else
    skip_gate "perf media burst smoke" "BETA_RUN_BURST_SMOKE=0 disables required media burst smoke evidence."
  fi
  if [ "${BETA_RUN_BURST_MATRIX:-0}" = "1" ]; then
    run_gate "perf media burst matrix" env \
      RVOIP_PERF_FEATURES="$features" \
      RVOIP_PERF_BURST_SCENARIO_FILE="${BETA_BURST_SCENARIO_FILE:-$CRATE_DIR/config/perf-burst-scenarios.yaml}" \
      RVOIP_PERF_BURST_SCENARIOS="${BETA_BURST_MATRIX:-all}" \
      RVOIP_PERF_MEMORY_DIAGNOSTICS="${RVOIP_PERF_MEMORY_DIAGNOSTICS:-0}" \
      RVOIP_PERF_ALLOCATOR_DIAGNOSTICS="${RVOIP_PERF_ALLOCATOR_DIAGNOSTICS:-0}" \
      RVOIP_PERF_MEMORY_DIAG_INTERVAL_SECS="${RVOIP_PERF_MEMORY_DIAG_INTERVAL_SECS:-5}" \
      RVOIP_PERF_MIMALLOC_COLLECT_AT="${RVOIP_PERF_MIMALLOC_COLLECT_AT:-off}" \
      "$SCRIPT_DIR/perf_burst_matrix.sh"
  fi
  if [ "${BETA_RUN_LONG_SOAK:-1}" = "1" ]; then
    run_gate "perf soak candidate" env \
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
      RVOIP_PERF_EXTERNAL_RESOURCE_SAMPLER="${RVOIP_PERF_EXTERNAL_RESOURCE_SAMPLER:-1}" \
      "$SCRIPT_DIR/perf_soak_split.sh"
  else
    skip_gate "perf soak" "BETA_RUN_LONG_SOAK=0 disables release-candidate soak evidence."
  fi

  # Compare this run's perf metrics against the previous run and flag regressions.
  run_perf_regression_audit
}

write_environment_report
write_summary_gate_table_header

if bool_env_enabled "$BETA_REQUIRE_CLEAN_SOURCE"; then
  if ! run_gate "clean beta source fingerprint" verify_clean_source_fingerprint; then
    echo "Release-candidate gates require a clean source fingerprint. Set BETA_REQUIRE_CLEAN_SOURCE=0 only for development diagnostics." >&2
    exit 1
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
    run_perf_gates
    ;;
  interop)
    run_interop_gates
    ;;
  perf)
    run_perf_gates
    ;;
  security)
    run_security_gates
    ;;
  *)
    echo "Unknown mode: $MODE" >&2
    exit 2
    ;;
esac

if canonical_2k_evidence_requested; then
  run_gate "canonical 2k beta source unchanged" \
    python3 "$CANONICAL_2K_EVIDENCE_HELPER" verify-source \
      --workspace-root "$WORKSPACE_ROOT" \
      --beta-start "$BETA_SOURCE_AT_START"
elif bool_env_enabled "$BETA_REQUIRE_CLEAN_SOURCE"; then
  run_gate "beta source unchanged" \
    python3 "$CANONICAL_2K_EVIDENCE_HELPER" verify-source \
      --workspace-root "$WORKSPACE_ROOT" \
      --beta-start "$BETA_SOURCE_AT_START"
fi

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
- latest_pointer: \`$(beta_report_root)/latest.txt\`

## Result

- failures: $FAILURES
- skips: $SKIPS
EOF

package_beta_report

echo
echo "Summary: $SUMMARY"
if [ "${BETA_REPORT_PACKAGE:-1}" != "0" ]; then
  echo "Beta report: $(beta_report_run_dir)"
fi
if [ "$FAILURES" -ne 0 ]; then
  exit 1
fi
