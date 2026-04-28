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
run_stage "UDP blind transfer to 2003" \
  "$SCRIPT_DIR/udp_blind_transfer_remote/run.sh"

echo
echo "========================================================================"
echo "== Extended multi-endpoint Asterisk validation sequence complete"
echo "========================================================================"
