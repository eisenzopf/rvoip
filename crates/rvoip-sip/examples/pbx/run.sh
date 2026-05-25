#!/usr/bin/env sh
# Unified PBX interop matrix runner.
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
WORKSPACE_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../../../.." && pwd)
OUT_ROOT="${PBX_OUT_ROOT:-$SCRIPT_DIR/output}"
RUN_STARTED_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)
RUN_STARTED_EPOCH=$(date +%s)
RUN_SUMMARY="$OUT_ROOT/summary.md"
RUN_MATRIX="$OUT_ROOT/matrix.tsv"
RUN_ENV="$OUT_ROOT/environment.md"

PBX_ARG=${PBX_PROVIDER:-asterisk}
API_ARG=${PBX_API:-all}
SCENARIO_ARG=${PBX_SCENARIO:-all}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --pbx|--provider)
      PBX_ARG=$2
      shift 2
      ;;
    --api)
      API_ARG=$2
      shift 2
      ;;
    --scenario)
      SCENARIO_ARG=$2
      shift 2
      ;;
    --help|-h)
      echo "Usage: $0 [--pbx asterisk|freeswitch|both] [--api endpoint|stream_peer|callback|all] [--scenario registration|basic_call|hold_resume|ring_cancel|dtmf|reject|blind_transfer|all]"
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

# shellcheck disable=SC1091
. "$SCRIPT_DIR/tls_cert.sh"
RUN_ENV="$OUT_ROOT/environment-${PBX_ARG}.md"

PBX_CHILDREN=""
PBX_REPORT_READY=0

cleanup() {
  for pid in $PBX_CHILDREN; do
    kill "$pid" 2>/dev/null || true
  done
  wait 2>/dev/null || true
}

redacted_env() {
  env | LC_ALL=C sort | awk -F= '
    /^(PBX_|SIP_|TLS_|ASTERISK_|FREESWITCH_|RVOIP_|AUDIO_|IDLE_)/ {
      key=$1
      value=substr($0, length($1) + 2)
      upper=toupper(key)
      if (upper ~ /(PASSWORD|PASS|SECRET|TOKEN|CREDENTIAL|PRIVATE|AUTHORIZATION)/) {
        print key"=<redacted>"
      } else {
        print key"="value
      }
    }
  '
}

capture_command() {
  output=$1
  shift
  {
    echo "+ $*"
    "$@"
  } >"$output" 2>&1 || true
}

write_run_environment() {
  mkdir -p "$OUT_ROOT"
  {
    echo "# PBX Interop Environment"
    echo
    echo "- started_at_utc: $RUN_STARTED_UTC"
    echo "- workspace: $WORKSPACE_ROOT"
    echo "- output_root: $OUT_ROOT"
    echo "- pbx_arg: $PBX_ARG"
    echo "- api_arg: $API_ARG"
    echo "- scenario_arg: $SCENARIO_ARG"
    echo "- git_rev: $(git -C "$WORKSPACE_ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"
    echo "- rustc: $(rustc --version 2>/dev/null || echo unknown)"
    echo "- cargo: $(cargo --version 2>/dev/null || echo unknown)"
    echo "- host: $(uname -a 2>/dev/null || echo unknown)"
    if command -v sipp >/dev/null 2>&1; then
      echo "- sipp: $(sipp -v 2>&1 | head -1)"
    else
      echo "- sipp: not found"
    fi
    if command -v tshark >/dev/null 2>&1; then
      echo "- tshark: $(tshark -v 2>&1 | head -1)"
    else
      echo "- tshark: not found"
    fi
    echo
    echo "## Redacted Runtime Environment"
    echo
    echo '```text'
    redacted_env
    echo '```'
  } >"$RUN_ENV"

  capture_command "$OUT_ROOT/git-status.txt" git -C "$WORKSPACE_ROOT" status --short
}

init_report() {
  mkdir -p "$OUT_ROOT"
  if [ "${PBX_REPORT_APPEND:-0}" != "1" ] || [ ! -f "$RUN_MATRIX" ]; then
    printf 'status\tprovider\tapi\tscenario\ttransport\trole\tduration_s\texit_code\tstarted_at_utc\tended_at_utc\tlog\tout_dir\n' >"$RUN_MATRIX"
  fi
  write_run_environment
  PBX_REPORT_READY=1
}

record_matrix() {
  status=$1
  provider=$2
  api=$3
  scenario=$4
  transport=$5
  role=$6
  duration=$7
  exit_code=$8
  started_at=$9
  ended_at=${10}
  log=${11}
  out_dir=${12}
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$status" "$provider" "$api" "$scenario" "$transport" "$role" \
    "$duration" "$exit_code" "$started_at" "$ended_at" "$log" "$out_dir" >>"$RUN_MATRIX"
}

write_run_summary() {
  exit_status=$1
  if [ "$PBX_REPORT_READY" != "1" ]; then
    return
  fi
  ended_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  duration=$(( $(date +%s) - RUN_STARTED_EPOCH ))
  pass_count=$(awk -F '\t' 'NR > 1 && $1 == "PASS" { n++ } END { print n + 0 }' "$RUN_MATRIX" 2>/dev/null || echo 0)
  fail_count=$(awk -F '\t' 'NR > 1 && $1 == "FAIL" { n++ } END { print n + 0 }' "$RUN_MATRIX" 2>/dev/null || echo 0)
  total_count=$(awk 'NR > 1 { n++ } END { print n + 0 }' "$RUN_MATRIX" 2>/dev/null || echo 0)

  {
    echo "# PBX Interop Run Summary"
    echo
    echo "- started_at_utc: $RUN_STARTED_UTC"
    echo "- ended_at_utc: $ended_at"
    echo "- duration_seconds: $duration"
    echo "- exit_status: $exit_status"
    echo "- output_root: $OUT_ROOT"
    echo "- environments: \`environment-*.md\`"
    echo "- matrix: \`matrix.tsv\`"
    echo
    echo "## Result"
    echo
    echo "- total_cells: $total_count"
    echo "- passed_cells: $pass_count"
    echo "- failed_cells: $fail_count"
    echo
    echo "## Matrix"
    echo
    echo "| Status | Provider | API | Scenario | Transport | Role | Duration | Exit | Log |"
    echo "|--------|----------|-----|----------|-----------|------|----------|------|-----|"
    awk -F '\t' 'NR > 1 {
      printf "| %s | %s | %s | %s | %s | %s | %ss | %s | `%s` |\n", $1, $2, $3, $4, $5, $6, $7, $8, $11
    }' "$RUN_MATRIX"
  } >"$RUN_SUMMARY"
}

finish() {
  status=$?
  trap - EXIT INT TERM
  cleanup
  write_run_summary "$status"
  exit "$status"
}

trap finish EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

pbx_list() {
  case "$PBX_ARG" in
    both|all) printf '%s\n' asterisk freeswitch ;;
    asterisk|ast) printf '%s\n' asterisk ;;
    freeswitch|free-switch|fs) printf '%s\n' freeswitch ;;
    *) echo "Unknown PBX: $PBX_ARG" >&2; exit 2 ;;
  esac
}

api_examples() {
  case "$API_ARG" in
    all) printf '%s\n' pbx_endpoint pbx_stream_peer pbx_callback_builder ;;
    endpoint) printf '%s\n' pbx_endpoint ;;
    stream_peer|peer|streampeer) printf '%s\n' pbx_stream_peer ;;
    callback|callback_builder) printf '%s\n' pbx_callback_builder ;;
    *) echo "Unknown API: $API_ARG" >&2; exit 2 ;;
  esac
}

scenario_list() {
  case "$SCENARIO_ARG" in
    all) printf '%s\n' registration basic_call hold_resume ring_cancel dtmf reject blind_transfer ;;
    basic|basic_call|call) printf '%s\n' basic_call ;;
    hold|hold_resume) printf '%s\n' hold_resume ;;
    ring|ring_cancel) printf '%s\n' ring_cancel ;;
    blind_transfer|transfer) printf '%s\n' blind_transfer ;;
    registration|dtmf|reject) printf '%s\n' "$SCENARIO_ARG" ;;
    *) echo "Unknown scenario: $SCENARIO_ARG" >&2; exit 2 ;;
  esac
}

load_provider_env() {
  provider=$1
  unset TLS_CERT_PATH TLS_KEY_PATH
  case "$provider" in
    freeswitch)
      unset SIP_SERVER SIP_PORT SIP_TLS_PORT SIP_PASSWORD TLS_CA_PATH
      unset ASTERISK_TLS_CONTACT_MODE ASTERISK_TLS_FLOW_REUSE ASTERISK_TLS_SRTP_REQUIRED
      ;;
    *)
      unset FREESWITCH_UDP_ADDR FREESWITCH_TLS_ADDR FREESWITCH_PASSWORD FREESWITCH_TRANSPORT
      unset FREESWITCH_TLS_CONTACT_MODE FREESWITCH_TLS_FLOW_REUSE FREESWITCH_TLS_SRTP_REQUIRED
      ;;
  esac
  if [ "$provider" = "asterisk" ] && [ -f "$HOME/Developer/asterisk/rvoip-local.env" ]; then
    set -a
    # shellcheck disable=SC1091
    . "$HOME/Developer/asterisk/rvoip-local.env"
    set +a
  fi
  if [ "$provider" = "freeswitch" ] && [ -f "$HOME/Developer/freeswitch/freeswitch-local.env" ]; then
    set -a
    # shellcheck disable=SC1091
    . "$HOME/Developer/freeswitch/freeswitch-local.env"
    set +a
  fi
  if [ -f "$SCRIPT_DIR/env/${provider}.env" ]; then
    set -a
    # shellcheck disable=SC1091
    . "$SCRIPT_DIR/env/${provider}.env"
    set +a
  fi
  if [ -f "$SCRIPT_DIR/.env.local" ]; then
    set -a
    # shellcheck disable=SC1091
    . "$SCRIPT_DIR/.env.local"
    set +a
  fi
}

example_label() {
  case "$1" in
    pbx_endpoint) printf '%s\n' endpoint ;;
    pbx_stream_peer) printf '%s\n' stream_peer ;;
    pbx_callback_builder) printf '%s\n' callback ;;
  esac
}

run_one() {
  provider=$1
  example=$2
  scenario=$3
  transport=$4
  role=$5
  out_dir=$6
  log=$7
  api_label=$(example_label "$example")
  started_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  start_epoch=$(date +%s)
  status_label=PASS

  mkdir -p "$out_dir"
  {
    echo "# PBX Cell Metadata"
    echo
    echo "- provider: $provider"
    echo "- api: $api_label"
    echo "- scenario: $scenario"
    echo "- transport: $transport"
    echo "- role: $role"
    echo "- started_at_utc: $started_at"
    echo "- output_dir: $out_dir"
    echo "- log: $log"
    echo
    echo "## Command"
    echo
    echo '```sh'
    echo "PBX_PROVIDER=$provider PBX_SCENARIO=$scenario PBX_TRANSPORT=$transport SIP_TRANSPORT=$transport PBX_ROLE=$role AUDIO_OUTPUT_DIR=$out_dir cargo run -p rvoip-sip --features dev-insecure-tls --example $example --quiet"
    echo '```'
    echo
    echo "## Redacted Environment"
    echo
    echo '```text'
    redacted_env
    echo '```'
  } >"$out_dir/${role}_metadata.md"

  {
    echo "provider: $provider"
    echo "api: $api_label"
    echo "scenario: $scenario"
    echo "transport: $transport"
    echo "role: $role"
    echo "started_at_utc: $started_at"
    echo
    echo "+ PBX_PROVIDER=$provider PBX_SCENARIO=$scenario PBX_TRANSPORT=$transport SIP_TRANSPORT=$transport PBX_ROLE=$role AUDIO_OUTPUT_DIR=$out_dir cargo run -p rvoip-sip --features dev-insecure-tls --example $example --quiet"
  } >"$log"

  set +e
  (
    cd "$WORKSPACE_ROOT"
    PBX_PROVIDER="$provider" \
    PBX_SCENARIO="$scenario" \
    PBX_TRANSPORT="$transport" \
    SIP_TRANSPORT="$transport" \
    PBX_ROLE="$role" \
    AUDIO_OUTPUT_DIR="$out_dir" \
      cargo run -p rvoip-sip --features dev-insecure-tls --example "$example" --quiet
  ) >>"$log" 2>&1
  rc=$?
  set -e

  ended_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  duration=$(( $(date +%s) - start_epoch ))
  if [ "$rc" -ne 0 ]; then
    status_label=FAIL
  fi
  {
    echo
    echo "ended_at_utc: $ended_at"
    echo "duration_seconds: $duration"
    echo "exit_status: $rc"
  } >>"$log"
  record_matrix "$status_label" "$provider" "$api_label" "$scenario" "$transport" "$role" "$duration" "$rc" "$started_at" "$ended_at" "$log" "$out_dir"
  return "$rc"
}

start_one() {
  provider=$1
  example=$2
  scenario=$3
  transport=$4
  role=$5
  out_dir=$6
  log=$7
  echo "[$provider/$(example_label "$example")/$transport/$scenario/$role] starting"
  run_one "$provider" "$example" "$scenario" "$transport" "$role" "$out_dir" "$log" &
  LAST_PID=$!
  PBX_CHILDREN="$PBX_CHILDREN $LAST_PID"
}

wait_for_log() {
  file=$1
  pattern=$2
  pid=$3
  label=$4
  limit=${5:-45}
  elapsed=0
  while [ "$elapsed" -lt "$limit" ]; do
    if grep -q "$pattern" "$file" 2>/dev/null; then
      return 0
    fi
    if ! kill -0 "$pid" 2>/dev/null; then
      echo "[$label] process exited before '$pattern' appeared"
      sed -n '1,160p' "$file" 2>/dev/null || true
      return 1
    fi
    sleep 1
    elapsed=$((elapsed + 1))
  done
  echo "[$label] timed out waiting for '$pattern'"
  sed -n '1,160p' "$file" 2>/dev/null || true
  return 1
}

wait_child() {
  pid=$1
  label=$2
  log=$3
  set +e
  wait "$pid"
  status=$?
  set -e
  if [ "$status" -ne 0 ]; then
    echo "[$label] failed with exit $status"
    sed -n '1,220p' "$log" 2>/dev/null || true
    return "$status"
  fi
}

prepare_tls() {
  provider=$1
  out_dir=$2
  export PBX_PROVIDER="$provider"
  export PBX_TRANSPORT=TLS
  export SIP_TRANSPORT=TLS
  case "$provider" in
    freeswitch)
      export TLS_INSECURE="${TLS_INSECURE:-1}"
      export FREESWITCH_TLS_CONTACT_MODE="${FREESWITCH_TLS_CONTACT_MODE:-reachable-contact}"
      export FREESWITCH_TLS_SRTP_REQUIRED="${FREESWITCH_TLS_SRTP_REQUIRED:-1}"
      ;;
    *)
      export ASTERISK_TLS_CONTACT_MODE="${ASTERISK_TLS_CONTACT_MODE:-reachable-contact}"
      export ASTERISK_TLS_SRTP_REQUIRED="${ASTERISK_TLS_SRTP_REQUIRED:-1}"
      ;;
  esac
  ensure_pbx_tls_listener_cert "$out_dir/tls"
}

run_analyze() {
  provider=$1
  scenario=$2
  transport=$3
  out_dir=$4
  log="$out_dir/analyze.log"
  started_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  start_epoch=$(date +%s)
  status_label=PASS
  {
    echo "provider: $provider"
    echo "api: analyzer"
    echo "scenario: $scenario"
    echo "transport: $transport"
    echo "role: analyze"
    echo "started_at_utc: $started_at"
    echo
    echo "+ PBX_PROVIDER=$provider PBX_SCENARIO=$scenario PBX_TRANSPORT=$transport SIP_TRANSPORT=$transport AUDIO_OUTPUT_DIR=$out_dir cargo run -p rvoip-sip --features dev-insecure-tls --example pbx_analyze --quiet"
  } >"$log"
  set +e
  (
    cd "$WORKSPACE_ROOT"
    PBX_PROVIDER="$provider" \
    PBX_SCENARIO="$scenario" \
    PBX_TRANSPORT="$transport" \
    SIP_TRANSPORT="$transport" \
    AUDIO_OUTPUT_DIR="$out_dir" \
      cargo run -p rvoip-sip --features dev-insecure-tls --example pbx_analyze --quiet
  ) >>"$log" 2>&1
  rc=$?
  set -e
  ended_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  duration=$(( $(date +%s) - start_epoch ))
  if [ "$rc" -ne 0 ]; then
    status_label=FAIL
  fi
  {
    echo
    echo "ended_at_utc: $ended_at"
    echo "duration_seconds: $duration"
    echo "exit_status: $rc"
  } >>"$log"
  record_matrix "$status_label" "$provider" analyzer "$scenario" "$transport" analyze "$duration" "$rc" "$started_at" "$ended_at" "$log" "$out_dir"
  return "$rc"
}

run_registration() {
  provider=$1
  example=$2
  api_label=$(example_label "$example")
  old_idle=${IDLE_SECS-}
  export IDLE_SECS="${REGISTRATION_IDLE_SECS:-2}"
  for transport in TLS UDP; do
    out_dir="$OUT_ROOT/$provider/$api_label/registration/$transport"
    if [ "$transport" = "TLS" ]; then
      prepare_tls "$provider" "$out_dir"
    fi
    run_one "$provider" "$example" registration "$transport" registration "$out_dir" "$out_dir/registration.log"
  done
  if [ -n "$old_idle" ]; then
    export IDLE_SECS="$old_idle"
  else
    unset IDLE_SECS
  fi
}

run_two_party() {
  provider=$1
  example=$2
  scenario=$3
  transport=$4
  api_label=$(example_label "$example")
  out_dir="$OUT_ROOT/$provider/$api_label/$scenario/$transport"
  rm -rf "$out_dir"
  mkdir -p "$out_dir"
  if [ "$transport" = "TLS" ]; then
    prepare_tls "$provider" "$out_dir"
  fi

  case "$scenario" in
    basic_call|hold_resume|dtmf|reject)
      start_one "$provider" "$example" "$scenario" "$transport" callee "$out_dir" "$out_dir/callee.log"
      pid_a=$LAST_PID
      wait_for_log "$out_dir/callee.log" "Registered." "$pid_a" "$scenario-callee"
      run_one "$provider" "$example" "$scenario" "$transport" caller "$out_dir" "$out_dir/caller.log"
      wait_child "$pid_a" "$scenario-callee" "$out_dir/callee.log"
      ;;
    ring_cancel)
      start_one "$provider" "$example" "$scenario" "$transport" target "$out_dir" "$out_dir/target.log"
      pid_a=$LAST_PID
      wait_for_log "$out_dir/target.log" "Registered." "$pid_a" "$scenario-target"
      run_one "$provider" "$example" "$scenario" "$transport" caller "$out_dir" "$out_dir/caller.log"
      wait_child "$pid_a" "$scenario-target" "$out_dir/target.log"
      ;;
  esac

  case "$scenario" in
    hold_resume|dtmf)
      run_analyze "$provider" "$scenario" "$transport" "$out_dir"
      ;;
  esac
}

run_transfer() {
  provider=$1
  example=$2
  transport=$3
  api_label=$(example_label "$example")
  scenario=blind_transfer
  out_dir="$OUT_ROOT/$provider/$api_label/$scenario/$transport"
  rm -rf "$out_dir"
  mkdir -p "$out_dir"
  if [ "$transport" = "TLS" ]; then
    prepare_tls "$provider" "$out_dir"
  fi

  start_one "$provider" "$example" "$scenario" "$transport" transferee "$out_dir" "$out_dir/transferee.log"
  pid_a=$LAST_PID
  wait_for_log "$out_dir/transferee.log" "Registered." "$pid_a" transfer-transferee
  start_one "$provider" "$example" "$scenario" "$transport" target "$out_dir" "$out_dir/target.log"
  pid_b=$LAST_PID
  wait_for_log "$out_dir/target.log" "Registered." "$pid_b" transfer-target
  run_one "$provider" "$example" "$scenario" "$transport" transferor "$out_dir" "$out_dir/transferor.log"
  wait_child "$pid_a" transfer-transferee "$out_dir/transferee.log"
  wait_child "$pid_b" transfer-target "$out_dir/target.log"
  run_analyze "$provider" "$scenario" "$transport" "$out_dir"
}

run_matrix_cell() {
  provider=$1
  example=$2
  scenario=$3
  case "$scenario" in
    registration)
      run_registration "$provider" "$example"
      ;;
    basic_call)
      run_two_party "$provider" "$example" basic_call UDP
      ;;
    hold_resume|ring_cancel|dtmf|reject)
      run_two_party "$provider" "$example" "$scenario" UDP
      run_two_party "$provider" "$example" "$scenario" TLS
      ;;
    blind_transfer)
      run_transfer "$provider" "$example" UDP
      run_transfer "$provider" "$example" TLS
      ;;
  esac
}

cd "$WORKSPACE_ROOT"
init_report
echo "Building unified PBX examples..."
echo "PBX output root: $OUT_ROOT"
cargo build -p rvoip-sip --features dev-insecure-tls \
  --example pbx_endpoint \
  --example pbx_stream_peer \
  --example pbx_callback_builder \
  --example pbx_analyze

for provider in $(pbx_list); do
  load_provider_env "$provider"
  for example in $(api_examples); do
    for scenario in $(scenario_list); do
      echo
      echo "========================================================================"
      echo "== $provider / $(example_label "$example") / $scenario"
      echo "========================================================================"
      run_matrix_cell "$provider" "$example" "$scenario"
    done
  done
done

echo
echo "========================================================================"
echo "== Unified PBX interop sequence complete"
echo "========================================================================"
