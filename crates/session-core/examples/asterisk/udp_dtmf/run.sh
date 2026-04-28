#!/usr/bin/env sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
ASTERISK_DIR=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
WORKSPACE_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../../../../.." && pwd)
OUT_DIR="$SCRIPT_DIR/output"
LOG_2001="$OUT_DIR/2001.log"
LOG_2002="$OUT_DIR/2002.log"
PID_2002=""

cleanup() {
  if [ -n "$PID_2002" ]; then
    kill "$PID_2002" 2>/dev/null || true
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

if [ -f "$ASTERISK_DIR/.env" ]; then
  set -a
  # shellcheck disable=SC1091
  . "$ASTERISK_DIR/.env"
  set +a
fi

cd "$WORKSPACE_ROOT"
rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"

export SIP_TRANSPORT=UDP
export SIP_PORT="${SIP_PORT:-5060}"

echo "Building UDP DTMF examples..."
cargo build -p rvoip-session-core \
  --example asterisk_udp_dtmf_2001 \
  --example asterisk_udp_dtmf_2002

AUDIO_OUTPUT_DIR="$OUT_DIR" cargo run -p rvoip-session-core \
  --example asterisk_udp_dtmf_2002 --quiet >"$LOG_2002" 2>&1 &
PID_2002=$!
wait_for_log "$LOG_2002" "Registered; waiting" "$PID_2002" "2002" 30

AUDIO_OUTPUT_DIR="$OUT_DIR" cargo run -p rvoip-session-core \
  --example asterisk_udp_dtmf_2001 --quiet >"$LOG_2001" 2>&1

wait "$PID_2002"
PID_2002=""

echo
echo "=== UDP DTMF example complete ==="
echo "Logs: $OUT_DIR"
