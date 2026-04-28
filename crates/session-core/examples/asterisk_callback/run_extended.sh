#!/usr/bin/env sh
# Extended CallbackPeer Asterisk validation sequence.
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

run_ring() {
  transport=$1
  out_dir="$OUT_BASE/${transport}_ring_cancel"
  rm -rf "$out_dir"
  mkdir -p "$out_dir"
  if [ "$transport" = tls ]; then
    prepare_tls "$out_dir"
    start_endpoint tls_ring_target "$out_dir" "$out_dir/1003.log" 1003
    PID_A=$LAST_PID
    wait_for_log "$out_dir/1003.log" "Registered" "$PID_A" 1003 30
    run_endpoint tls_ring_caller "$out_dir" "$out_dir/1001.log"
  else
    prepare_udp
    start_endpoint udp_ring_target "$out_dir" "$out_dir/2003.log" 2003
    PID_A=$LAST_PID
    wait_for_log "$out_dir/2003.log" "Registered" "$PID_A" 2003 30
    run_endpoint udp_ring_caller "$out_dir" "$out_dir/2001.log"
  fi
  wait_child "$PID_A" ring-target
  PID_A=""
}

run_dtmf() {
  transport=$1
  out_dir="$OUT_BASE/${transport}_dtmf"
  rm -rf "$out_dir"
  mkdir -p "$out_dir"
  if [ "$transport" = tls ]; then
    prepare_tls "$out_dir"
    start_endpoint tls_dtmf_callee "$out_dir" "$out_dir/1002.log" 1002
    PID_A=$LAST_PID
    wait_for_log "$out_dir/1002.log" "Registered" "$PID_A" 1002 30
    run_endpoint tls_dtmf_caller "$out_dir" "$out_dir/1001.log"
    wait_child "$PID_A" 1002
    PID_A=""
    run_analyze tls_dtmf "$out_dir" "$out_dir/analyze.log"
  else
    prepare_udp
    start_endpoint udp_dtmf_callee "$out_dir" "$out_dir/2002.log" 2002
    PID_A=$LAST_PID
    wait_for_log "$out_dir/2002.log" "Registered" "$PID_A" 2002 30
    run_endpoint udp_dtmf_caller "$out_dir" "$out_dir/2001.log"
    wait_child "$PID_A" 2002
    PID_A=""
  fi
}

run_transfer() {
  transport=$1
  out_dir="$OUT_BASE/${transport}_blind_transfer"
  rm -rf "$out_dir"
  mkdir -p "$out_dir"
  if [ "$transport" = tls ]; then
    prepare_tls "$out_dir"
    start_endpoint tls_transfer_target "$out_dir" "$out_dir/1003.log" 1003
    PID_A=$LAST_PID
    wait_for_log "$out_dir/1003.log" "Registered" "$PID_A" 1003 30
    start_endpoint tls_transferee "$out_dir" "$out_dir/1002.log" 1002
    PID_B=$LAST_PID
    wait_for_log "$out_dir/1002.log" "Registered" "$PID_B" 1002 30
    run_endpoint tls_transferor "$out_dir" "$out_dir/1001.log"
    wait_child "$PID_B" 1002
    PID_B=""
    wait_child "$PID_A" 1003
    PID_A=""
    run_analyze tls_transfer "$out_dir" "$out_dir/analyze.log"
  else
    prepare_udp
    start_endpoint udp_transfer_target "$out_dir" "$out_dir/2003.log" 2003
    PID_A=$LAST_PID
    wait_for_log "$out_dir/2003.log" "Registered" "$PID_A" 2003 30
    start_endpoint udp_transferee "$out_dir" "$out_dir/2002.log" 2002
    PID_B=$LAST_PID
    wait_for_log "$out_dir/2002.log" "Registered" "$PID_B" 2002 30
    run_endpoint udp_transferor "$out_dir" "$out_dir/2001.log"
    wait_child "$PID_B" 2002
    PID_B=""
    wait_child "$PID_A" 2003
    PID_A=""
  fi
}

run_reject() {
  transport=$1
  out_dir="$OUT_BASE/${transport}_reject"
  rm -rf "$out_dir"
  mkdir -p "$out_dir"
  if [ "$transport" = tls ]; then
    prepare_tls "$out_dir"
    start_endpoint tls_reject_callee "$out_dir" "$out_dir/1002.log" 1002
    PID_A=$LAST_PID
    wait_for_log "$out_dir/1002.log" "Registered" "$PID_A" 1002 30
    run_endpoint tls_reject_caller "$out_dir" "$out_dir/1001.log"
  else
    prepare_udp
    start_endpoint udp_reject_callee "$out_dir" "$out_dir/2002.log" 2002
    PID_A=$LAST_PID
    wait_for_log "$out_dir/2002.log" "Registered" "$PID_A" 2002 30
    run_endpoint udp_reject_caller "$out_dir" "$out_dir/2001.log"
  fi
  wait_child "$PID_A" reject-callee
  PID_A=""
}

cd "$WORKSPACE_ROOT"
mkdir -p "$OUT_BASE"

echo "Building CallbackPeer extended Asterisk examples..."
cargo build -p rvoip-session-core --features dev-insecure-tls \
  --example asterisk_callback_endpoint \
  --example asterisk_callback_analyze

echo
echo "========================================================================"
echo "== Callback TLS/SRTP ring/cancel"
echo "========================================================================"
run_ring tls

echo
echo "========================================================================"
echo "== Callback UDP ring/cancel"
echo "========================================================================"
run_ring udp

echo
echo "========================================================================"
echo "== Callback TLS/SRTP DTMF"
echo "========================================================================"
run_dtmf tls

echo
echo "========================================================================"
echo "== Callback UDP DTMF"
echo "========================================================================"
run_dtmf udp

echo
echo "========================================================================"
echo "== Callback TLS/SRTP blind transfer"
echo "========================================================================"
run_transfer tls

echo
echo "========================================================================"
echo "== Callback UDP blind transfer"
echo "========================================================================"
run_transfer udp

echo
echo "========================================================================"
echo "== Callback TLS/SRTP reject"
echo "========================================================================"
run_reject tls

echo
echo "========================================================================"
echo "== Callback UDP reject"
echo "========================================================================"
run_reject udp

echo
echo "========================================================================"
echo "== CallbackPeer extended Asterisk sequence complete"
echo "========================================================================"
