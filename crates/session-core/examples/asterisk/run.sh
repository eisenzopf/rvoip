#!/usr/bin/env sh
# Full Asterisk validation sequence:
# 1) registration smoke tests for TLS user 1001 and UDP user 2001
# 2) TLS/SRTP hold/resume call for 1001/1002
# 3) UDP hold/resume call for 2001/2002
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

run_stage "Registration smoke tests: TLS 1001, then UDP 2001" \
  "$SCRIPT_DIR/registration/run.sh"

run_stage "TLS/SRTP hold/resume call: 1001 -> 1002" \
  "$SCRIPT_DIR/tls_srtp_hold_resume/run.sh"

run_stage "UDP hold/resume call: 2001 -> 2002" \
  "$SCRIPT_DIR/udp_hold_resume/run.sh"

echo
echo "========================================================================"
echo "== Full Asterisk validation sequence complete"
echo "========================================================================"
