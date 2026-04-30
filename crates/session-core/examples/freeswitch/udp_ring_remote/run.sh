#!/usr/bin/env sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
FREESWITCH_DIR=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
WORKSPACE_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../../../../.." && pwd)
OUT_DIR="$SCRIPT_DIR/output"
LOG_2001="$OUT_DIR/2001.log"
LOG_2003="$OUT_DIR/2003.log"
PID_2003=""

cleanup() {
  if [ -n "$PID_2003" ]; then
    kill "$PID_2003" 2>/dev/null || true
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

cd "$WORKSPACE_ROOT"
rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"

export SIP_TRANSPORT=UDP
export SIP_PORT="${SIP_PORT:-5062}"

echo "Building UDP ring/cancel example..."
cargo build -p rvoip-session-core \
  --example freeswitch_udp_ring_remote_2001 \
  --example freeswitch_udp_ring_remote_2003

echo
echo "[2003] Starting rvoip ring/cancel target."
AUDIO_OUTPUT_DIR="$OUT_DIR" cargo run -p rvoip-session-core \
  --example freeswitch_udp_ring_remote_2003 --quiet >"$LOG_2003" 2>&1 &
PID_2003=$!
wait_for_log "$LOG_2003" "Registered; waiting" "$PID_2003" "2003" 30

echo "[2001] Starting caller."
AUDIO_OUTPUT_DIR="$OUT_DIR" cargo run -p rvoip-session-core \
  --example freeswitch_udp_ring_remote_2001 --quiet >"$LOG_2001" 2>&1

wait "$PID_2003"
PID_2003=""

assert_log_contains "Incoming call" "target observed incoming call before cancel"
assert_log_contains "Ring/cancel test passed." "caller completed cancellation"

echo
echo "=== UDP ring/cancel example complete ==="
echo "Logs: $OUT_DIR"
