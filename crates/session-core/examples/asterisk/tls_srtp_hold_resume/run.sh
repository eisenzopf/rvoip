#!/usr/bin/env sh
# Asterisk TLS/SRTP hold/resume example: register 1001 and 1002 over
# SIP TLS, require SDES-SRTP media, exercise hold/resume, and verify
# audio before and after resume.
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
WORKSPACE_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../../../../.." && pwd)
OUT_DIR="$SCRIPT_DIR/output"
LOG_1001="$OUT_DIR/1001.log"
LOG_1002="$OUT_DIR/1002.log"
LOG_ANALYZE="$OUT_DIR/analyze.log"

PID_1001=""
PID_1002=""
PID_ANALYZE=""
TAIL_1001=""
TAIL_1002=""
TAIL_ANALYZE=""
LAST_TAIL_PID=""

cleanup() {
  for pid in $PID_1001 $PID_1002 $PID_ANALYZE $TAIL_1001 $TAIL_1002 $TAIL_ANALYZE; do
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

assert_log_contains() {
  pattern=$1
  label=$2
  if ! grep -R -q "$pattern" "$OUT_DIR"; then
    echo "[VERIFY] missing expected log evidence: $label ($pattern)"
    return 1
  fi
}

cd "$WORKSPACE_ROOT"
rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"

export SIP_TRANSPORT="${SIP_TRANSPORT:-TLS}"
export SIP_TLS_PORT="${SIP_TLS_PORT:-5061}"
export ASTERISK_TLS_SRTP_REQUIRED="${ASTERISK_TLS_SRTP_REQUIRED:-1}"

echo "Building Asterisk TLS/SRTP hold/resume endpoint examples..."
cargo build -p rvoip-session-core --features dev-insecure-tls \
  --example asterisk_tls_srtp_hold_resume_1001 \
  --example asterisk_tls_srtp_hold_resume_1002 \
  --example asterisk_tls_srtp_hold_resume_analyze

echo "[1002] Starting"
AUDIO_OUTPUT_DIR="$OUT_DIR" cargo run -p rvoip-session-core --features dev-insecure-tls \
  --example asterisk_tls_srtp_hold_resume_1002 --quiet \
  >"$LOG_1002" 2>&1 &
PID_1002=$!
start_prefix_log "1002" "$LOG_1002"
TAIL_1002=$LAST_TAIL_PID

wait_for_log "$LOG_1002" "Registered; waiting for TLS/SRTP call" "$PID_1002" "1002" 30

echo "[1001] Starting"
AUDIO_OUTPUT_DIR="$OUT_DIR" cargo run -p rvoip-session-core --features dev-insecure-tls \
  --example asterisk_tls_srtp_hold_resume_1001 --quiet \
  >"$LOG_1001" 2>&1 &
PID_1001=$!
start_prefix_log "1001" "$LOG_1001"
TAIL_1001=$LAST_TAIL_PID

wait_for_child "$PID_1001" "1001"
wait_for_child "$PID_1002" "1002"

kill "$TAIL_1001" "$TAIL_1002" 2>/dev/null || true
TAIL_1001=""
TAIL_1002=""

echo "[ANALYZE] Starting"
AUDIO_OUTPUT_DIR="$OUT_DIR" cargo run -p rvoip-session-core --features dev-insecure-tls \
  --example asterisk_tls_srtp_hold_resume_analyze --quiet \
  >"$LOG_ANALYZE" 2>&1 &
PID_ANALYZE=$!
start_prefix_log "ANALYZE" "$LOG_ANALYZE"
TAIL_ANALYZE=$LAST_TAIL_PID
wait_for_child "$PID_ANALYZE" "ANALYZE"

assert_log_contains "sips:" "TLS/SIPS URI"
assert_log_contains "transport=tls" "TLS transport URI parameter"
assert_log_contains "RTP/SAVP" "SRTP media profile"
assert_log_contains "a=crypto" "SDES-SRTP crypto attribute"
if grep -R -q "proceeding plaintext" "$OUT_DIR"; then
  echo "[VERIFY] plaintext RTP fallback was logged despite mandatory SRTP"
  exit 1
fi

echo
echo "=== Asterisk TLS/SRTP hold/resume example complete ==="
echo "Output directory: $OUT_DIR"
