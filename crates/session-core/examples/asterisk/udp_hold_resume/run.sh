#!/usr/bin/env sh
# Asterisk UDP hold/resume example: register 2001 and 2002, call through
# Asterisk, exercise a mid-call hold/resume, and verify audio before and
# after resume.
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
WORKSPACE_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../../../../.." && pwd)
OUT_DIR="$SCRIPT_DIR/output"
LOG_2001="$OUT_DIR/2001.log"
LOG_2002="$OUT_DIR/2002.log"
LOG_ANALYZE="$OUT_DIR/analyze.log"

PID_2001=""
PID_2002=""
PID_ANALYZE=""
TAIL_2001=""
TAIL_2002=""
TAIL_ANALYZE=""
LAST_TAIL_PID=""

cleanup() {
  for pid in $PID_2001 $PID_2002 $PID_ANALYZE $TAIL_2001 $TAIL_2002 $TAIL_ANALYZE; do
    if [ -n "$pid" ]; then
      kill "$pid" 2>/dev/null || true
    fi
  done
  wait 2>/dev/null || true
}
trap cleanup EXIT INT TERM

start_prefix_log() {
  label=$1
  file=$2
  (
    tail -n +1 -f "$file" 2>/dev/null | sed "s/^/[$label] /"
  ) &
  LAST_TAIL_PID=$!
}

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

wait_for_child() {
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
  return 0
}

cd "$WORKSPACE_ROOT"
rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"

export SIP_TRANSPORT=UDP
export SIP_PORT="${SIP_PORT:-5060}"

echo "Building Asterisk UDP hold/resume endpoint examples..."
cargo build -p rvoip-session-core \
  --example asterisk_udp_hold_resume_2001 \
  --example asterisk_udp_hold_resume_2002 \
  --example asterisk_udp_hold_resume_analyze

echo "[2002] Starting"
AUDIO_OUTPUT_DIR="$OUT_DIR" cargo run -p rvoip-session-core --example asterisk_udp_hold_resume_2002 --quiet \
  >"$LOG_2002" 2>&1 &
PID_2002=$!
start_prefix_log "2002" "$LOG_2002"
TAIL_2002=$LAST_TAIL_PID

wait_for_log "$LOG_2002" "Registered; waiting for call" "$PID_2002" "2002" 30

echo "[2001] Starting"
AUDIO_OUTPUT_DIR="$OUT_DIR" cargo run -p rvoip-session-core --example asterisk_udp_hold_resume_2001 --quiet \
  >"$LOG_2001" 2>&1 &
PID_2001=$!
start_prefix_log "2001" "$LOG_2001"
TAIL_2001=$LAST_TAIL_PID

wait_for_child "$PID_2001" "2001"
wait_for_child "$PID_2002" "2002"

kill "$TAIL_2001" "$TAIL_2002" 2>/dev/null || true
TAIL_2001=""
TAIL_2002=""

echo "[ANALYZE] Starting"
AUDIO_OUTPUT_DIR="$OUT_DIR" cargo run -p rvoip-session-core --example asterisk_udp_hold_resume_analyze --quiet \
  >"$LOG_ANALYZE" 2>&1 &
PID_ANALYZE=$!
start_prefix_log "ANALYZE" "$LOG_ANALYZE"
TAIL_ANALYZE=$LAST_TAIL_PID
wait_for_child "$PID_ANALYZE" "ANALYZE"

echo
echo "=== Asterisk UDP hold/resume example complete ==="
echo "Output directory: $OUT_DIR"
