#!/usr/bin/env sh
# CallbackPeer Asterisk validation sequence.
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
ASTERISK_DIR=$(CDPATH= cd -- "$SCRIPT_DIR/../asterisk" && pwd)
WORKSPACE_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../../../.." && pwd)
OUT_BASE="$SCRIPT_DIR/output"

PID_A=""
PID_B=""
PID_C=""

cleanup() {
  for pid in $PID_A $PID_B $PID_C; do
    if [ -n "$pid" ]; then
      kill "$pid" 2>/dev/null || true
    fi
  done
  wait 2>/dev/null || true
}
trap cleanup EXIT INT TERM

if [ -f "$ASTERISK_DIR/.env" ]; then
  set -a
  # shellcheck disable=SC1091
  . "$ASTERISK_DIR/.env"
  set +a
fi

# shellcheck disable=SC1091
. "$ASTERISK_DIR/tls_cert.sh"

wait_for_log() {
  file=$1
  pattern=$2
  pid=$3
  label=$4
  limit=${5:-30}
  elapsed=0
  while [ "$elapsed" -lt "$limit" ]; do
    if grep -q "$pattern" "$file" 2>/dev/null; then
      return 0
    fi
    if ! kill -0 "$pid" 2>/dev/null; then
      echo "[$label] process exited before '$pattern' appeared"
      return 1
    fi
    sleep 1
    elapsed=$((elapsed + 1))
  done
  echo "[$label] timed out waiting for '$pattern'"
  return 1
}

wait_child() {
  pid=$1
  label=$2
  set +e
  wait "$pid"
  status=$?
  set -e
  if [ "$status" -ne 0 ]; then
    echo "[$label] failed with exit $status"
    return "$status"
  fi
}

run_endpoint() {
  scenario=$1
  out_dir=$2
  log=$3
  AUDIO_OUTPUT_DIR="$out_dir" CALLBACK_SCENARIO="$scenario" \
    cargo run -p rvoip-session-core --features dev-insecure-tls \
      --example asterisk_callback_endpoint --quiet >"$log" 2>&1
}

start_endpoint() {
  scenario=$1
  out_dir=$2
  log=$3
  label=$4
  echo "[$label] Starting $scenario"
  AUDIO_OUTPUT_DIR="$out_dir" CALLBACK_SCENARIO="$scenario" \
    cargo run -p rvoip-session-core --features dev-insecure-tls \
      --example asterisk_callback_endpoint --quiet >"$log" 2>&1 &
  LAST_PID=$!
}

run_analyze() {
  scenario=$1
  out_dir=$2
  log=$3
  AUDIO_OUTPUT_DIR="$out_dir" CALLBACK_ANALYZE="$scenario" \
    cargo run -p rvoip-session-core --features dev-insecure-tls \
      --example asterisk_callback_analyze --quiet >"$log" 2>&1
}

prepare_tls() {
  out_dir=$1
  export SIP_TRANSPORT=TLS
  export SIP_TLS_PORT="${SIP_TLS_PORT:-5061}"
  export ASTERISK_TLS_CONTACT_MODE="${ASTERISK_TLS_CONTACT_MODE:-reachable-contact}"
  export ASTERISK_TLS_SRTP_REQUIRED="${ASTERISK_TLS_SRTP_REQUIRED:-1}"
  ensure_asterisk_tls_listener_cert "$out_dir/tls"
}

prepare_udp() {
  export SIP_TRANSPORT=UDP
  export SIP_PORT="${SIP_PORT:-5060}"
}

run_registration() {
  out_dir="$OUT_BASE/registration"
  rm -rf "$out_dir"
  mkdir -p "$out_dir"
  prepare_tls "$out_dir"
  run_endpoint registration_tls "$out_dir" "$out_dir/tls_1001.log"
  prepare_udp
  run_endpoint registration_udp "$out_dir" "$out_dir/udp_2001.log"
}

run_tls_hold() {
  out_dir="$OUT_BASE/tls_srtp_hold_resume"
  rm -rf "$out_dir"
  mkdir -p "$out_dir"
  prepare_tls "$out_dir"
  start_endpoint tls_hold_callee "$out_dir" "$out_dir/1002.log" 1002
  PID_A=$LAST_PID
  wait_for_log "$out_dir/1002.log" "Registered" "$PID_A" 1002 30
  run_endpoint tls_hold_caller "$out_dir" "$out_dir/1001.log"
  wait_child "$PID_A" 1002
  PID_A=""
  run_analyze tls_hold "$out_dir" "$out_dir/analyze.log"
}

run_udp_hold() {
  out_dir="$OUT_BASE/udp_hold_resume"
  rm -rf "$out_dir"
  mkdir -p "$out_dir"
  prepare_udp
  start_endpoint udp_hold_callee "$out_dir" "$out_dir/2002.log" 2002
  PID_A=$LAST_PID
  wait_for_log "$out_dir/2002.log" "Registered" "$PID_A" 2002 30
  run_endpoint udp_hold_caller "$out_dir" "$out_dir/2001.log"
  wait_child "$PID_A" 2002
  PID_A=""
  run_analyze udp_hold "$out_dir" "$out_dir/analyze.log"
}

cd "$WORKSPACE_ROOT"
rm -rf "$OUT_BASE"
mkdir -p "$OUT_BASE"

echo "Building CallbackPeer Asterisk examples..."
cargo build -p rvoip-session-core --features dev-insecure-tls \
  --example asterisk_callback_endpoint \
  --example asterisk_callback_analyze

echo
echo "========================================================================"
echo "== Callback registration smoke tests"
echo "========================================================================"
run_registration

echo
echo "========================================================================"
echo "== Callback TLS/SRTP hold/resume"
echo "========================================================================"
run_tls_hold

echo
echo "========================================================================"
echo "== Callback UDP hold/resume"
echo "========================================================================"
run_udp_hold

RUN_EXTENDED="${ASTERISK_RUN_EXTENDED_TESTS:-${ASTERISK_RUN_REMOTE_TESTS:-0}}"
case "$RUN_EXTENDED" in
  1|true|TRUE|yes|YES|on|ON)
    "$SCRIPT_DIR/run_extended.sh"
    ;;
esac

echo
echo "========================================================================"
echo "== CallbackPeer Asterisk validation sequence complete"
echo "========================================================================"
