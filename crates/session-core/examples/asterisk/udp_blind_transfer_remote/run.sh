#!/usr/bin/env sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
ASTERISK_DIR=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
WORKSPACE_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../../../../.." && pwd)
OUT_DIR="$SCRIPT_DIR/output"
LOG_TRANSFEREE="$OUT_DIR/2002.log"
LOG_TRANSFEROR="$OUT_DIR/2001.log"
LOG_TARGET="$OUT_DIR/2003.log"
PID_TRANSFEREE=""
PID_TARGET=""

cleanup() {
  if [ -n "$PID_TARGET" ]; then
    kill "$PID_TARGET" 2>/dev/null || true
  fi
  if [ -n "$PID_TRANSFEREE" ]; then
    kill "$PID_TRANSFEREE" 2>/dev/null || true
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

echo "Building UDP blind transfer examples..."
cargo build -p rvoip-session-core \
  --example asterisk_udp_blind_transfer_remote_transferor \
  --example asterisk_udp_blind_transfer_remote_transferee \
  --example asterisk_udp_blind_transfer_remote_target

echo "[2003] Starting transfer target"
AUDIO_OUTPUT_DIR="$OUT_DIR" cargo run -p rvoip-session-core \
  --example asterisk_udp_blind_transfer_remote_target --quiet \
  >"$LOG_TARGET" 2>&1 &
PID_TARGET=$!

wait_for_log "$LOG_TARGET" "Registered; waiting" "$PID_TARGET" "2003" 30

echo "[2002] Starting transferee"
AUDIO_OUTPUT_DIR="$OUT_DIR" cargo run -p rvoip-session-core \
  --example asterisk_udp_blind_transfer_remote_transferee --quiet \
  >"$LOG_TRANSFEREE" 2>&1 &
PID_TRANSFEREE=$!

wait_for_log "$LOG_TRANSFEREE" "Waiting for transferor call" "$PID_TRANSFEREE" "2002" 30

echo
echo "[2001] Starting transferor"
AUDIO_OUTPUT_DIR="$OUT_DIR" cargo run -p rvoip-session-core \
  --example asterisk_udp_blind_transfer_remote_transferor --quiet \
  >"$LOG_TRANSFEROR" 2>&1

wait "$PID_TRANSFEREE"
PID_TRANSFEREE=""
wait "$PID_TARGET"
PID_TARGET=""

echo
echo "=== UDP blind transfer test complete ==="
echo "Logs: $OUT_DIR"
