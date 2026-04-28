#!/usr/bin/env sh
# Full Asterisk validation sequence:
# 1) registration smoke tests for TLS user 1001 and UDP user 2001
# 2) TLS/SRTP hold/resume call for 1001/1002
# 3) UDP hold/resume call for 2001/2002
# 4) optional TLS registered-flow and extended 1003/2003 scenarios when enabled
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

if [ -f "$SCRIPT_DIR/.env" ]; then
  set -a
  # shellcheck disable=SC1091
  . "$SCRIPT_DIR/.env"
  set +a
fi

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

RUN_FLOW_REUSE="${ASTERISK_RUN_FLOW_REUSE_TESTS:-0}"
case "$RUN_FLOW_REUSE" in
  1|true|TRUE|yes|YES|on|ON)
    run_stage "TLS/SRTP registered-flow call: 1001 -> 1002" \
      "$SCRIPT_DIR/tls_srtp_registered_flow/run.sh"
    ;;
esac

RUN_EXTENDED="${ASTERISK_RUN_EXTENDED_TESTS:-${ASTERISK_RUN_REMOTE_TESTS:-0}}"
case "$RUN_EXTENDED" in
  1|true|TRUE|yes|YES|on|ON)
    run_stage "Extended rvoip-controlled endpoint tests: 1003/2003" \
      "$SCRIPT_DIR/run_remote.sh"
    ;;
esac

echo
echo "========================================================================"
echo "== Full Asterisk validation sequence complete"
echo "========================================================================"
