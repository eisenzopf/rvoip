#!/usr/bin/env sh
# Extended Asterisk validation sequence using rvoip-controlled 1003/2003 endpoints.
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

run_stage() {
  label=$1
  script=$2

  echo
  echo "========================================================================"
  echo "== $label"
  echo "========================================================================"
  "$script"
}

assert_no_asterisk_channels() {
  label=$1

  if ! command -v docker >/dev/null 2>&1; then
    echo "[asterisk] docker not available; skipping channel leak check for $label"
    return 0
  fi
  if ! docker ps --format '{{.Names}}' | grep -qx 'rvoip-asterisk'; then
    echo "[asterisk] rvoip-asterisk is not running; skipping channel leak check for $label"
    return 0
  fi

  sleep 2
  channels=$(docker exec rvoip-asterisk asterisk -rx 'core show channels concise' 2>/dev/null | sed '/^[[:space:]]*$/d')
  if [ -n "$channels" ]; then
    echo "[asterisk] channel leak after $label:"
    docker exec rvoip-asterisk asterisk -rx 'core show channels'
    exit 1
  fi
  echo "[asterisk] no remaining channels after $label"
}

run_stage "TLS/SRTP ring/cancel: 1001 -> 1003" \
  "$SCRIPT_DIR/tls_srtp_ring_remote/run.sh"
run_stage "UDP ring/cancel: 2001 -> 2003" \
  "$SCRIPT_DIR/udp_ring_remote/run.sh"
run_stage "TLS/SRTP DTMF through Asterisk: 1001 -> 1002" \
  "$SCRIPT_DIR/tls_srtp_dtmf/run.sh"
run_stage "UDP DTMF through Asterisk: 2001 -> 2002" \
  "$SCRIPT_DIR/udp_dtmf/run.sh"
run_stage "TLS/SRTP blind transfer to 1003" \
  "$SCRIPT_DIR/tls_srtp_blind_transfer_remote/run.sh"
assert_no_asterisk_channels "TLS/SRTP blind transfer to 1003"
run_stage "UDP blind transfer to 2003" \
  "$SCRIPT_DIR/udp_blind_transfer_remote/run.sh"
assert_no_asterisk_channels "UDP blind transfer to 2003"

echo
echo "========================================================================"
echo "== Extended multi-endpoint Asterisk validation sequence complete"
echo "========================================================================"
