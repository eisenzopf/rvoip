#!/usr/bin/env sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
FREESWITCH_DIR=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
WORKSPACE_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../../../../.." && pwd)
OUT_DIR="$SCRIPT_DIR/output"
LOG_1001="$OUT_DIR/1001.log"
LOG_1003="$OUT_DIR/1003.log"
PID_1003=""

cleanup() {
  if [ -n "$PID_1003" ]; then
    kill "$PID_1003" 2>/dev/null || true
  fi
  wait 2>/dev/null || true
}
trap cleanup EXIT INT TERM

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

assert_log_contains() {
  pattern=$1
  label=$2
  if ! grep -R -q "$pattern" "$OUT_DIR"; then
    echo "[VERIFY] missing expected log evidence: $label ($pattern)"
    return 1
  fi
}

if [ -f "$HOME/Developer/freeswitch/freeswitch-local.env" ]; then
  set -a
  # shellcheck disable=SC1091
  . "$HOME/Developer/freeswitch/freeswitch-local.env"
  set +a
fi

if [ -f "$FREESWITCH_DIR/.env" ]; then
  set -a
  # shellcheck disable=SC1091
  . "$FREESWITCH_DIR/.env"
  set +a
fi

# shellcheck disable=SC1091
. "$FREESWITCH_DIR/tls_cert.sh"

cd "$WORKSPACE_ROOT"
rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"
ensure_freeswitch_tls_listener_cert "$OUT_DIR/tls"

export SIP_TRANSPORT=TLS
export SIP_TLS_PORT="${SIP_TLS_PORT:-5063}"
export TLS_INSECURE="${TLS_INSECURE:-1}"
export FREESWITCH_TLS_SRTP_REQUIRED="${FREESWITCH_TLS_SRTP_REQUIRED:-1}"
export RVOIP_SIP_DIAGNOSTICS="${RVOIP_SIP_DIAGNOSTICS:-1}"
export RUST_LOG="${RUST_LOG:-info,rvoip_dialog_core=warn},rvoip_dialog_core::transaction::manager=info"

echo "Building TLS/SRTP ring/cancel example..."
cargo build -p rvoip-session-core --features dev-insecure-tls \
  --example freeswitch_tls_srtp_ring_remote_1001 \
  --example freeswitch_tls_srtp_ring_remote_1003

echo
echo "[1003] Starting rvoip ring/cancel target."
AUDIO_OUTPUT_DIR="$OUT_DIR" cargo run -p rvoip-session-core --features dev-insecure-tls \
  --example freeswitch_tls_srtp_ring_remote_1003 --quiet >"$LOG_1003" 2>&1 &
PID_1003=$!
wait_for_log "$LOG_1003" "Registered; waiting" "$PID_1003" "1003" 30

echo "[1001] Starting caller."
AUDIO_OUTPUT_DIR="$OUT_DIR" cargo run -p rvoip-session-core --features dev-insecure-tls \
  --example freeswitch_tls_srtp_ring_remote_1001 --quiet >"$LOG_1001" 2>&1

wait "$PID_1003"
PID_1003=""

assert_log_contains "Incoming call" "target observed incoming call before cancel"
assert_log_contains "Ring/cancel test passed." "caller completed cancellation"
assert_log_contains "sips:" "TLS/SIPS URI"
assert_log_contains "transport=tls" "TLS transport URI parameter"
assert_log_contains "SIP/2.0/TLS" "TLS Via transport"

echo
echo "=== TLS/SRTP ring/cancel example complete ==="
echo "Logs: $OUT_DIR"
