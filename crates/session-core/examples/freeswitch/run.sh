#!/usr/bin/env sh
# Full FreeSWITCH validation sequence:
# 1) registration smoke tests for UDP user 2001 and TLS/SRTP user 1001
# 2) basic UDP/RTP call for 2001/2002
# 3) UDP/RTP hold/resume call for 2001/2002
# 4) TLS/SRTP hold/resume call for 1001/1002
# 5) optional extended 1003/2003 scenarios when enabled
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

if [ -f "$HOME/Developer/freeswitch/freeswitch-local.env" ]; then
  set -a
  # shellcheck disable=SC1091
  . "$HOME/Developer/freeswitch/freeswitch-local.env"
  set +a
fi

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

run_stage "Registration smoke tests: UDP 2001, then TLS/SRTP 1001" \
  "$SCRIPT_DIR/registration/run.sh"

run_stage "Basic UDP/RTP call: 2001 -> 2002" \
  "$SCRIPT_DIR/udp_call/run.sh"

run_stage "UDP hold/resume call: 2001 -> 2002" \
  "$SCRIPT_DIR/udp_hold_resume/run.sh"

run_stage "TLS/SRTP hold/resume call: 1001 -> 1002" \
  "$SCRIPT_DIR/tls_srtp_hold_resume/run.sh"

RUN_EXTENDED="${FREESWITCH_RUN_EXTENDED_TESTS:-${FREESWITCH_RUN_REMOTE_TESTS:-0}}"
case "$RUN_EXTENDED" in
  1|true|TRUE|yes|YES|on|ON)
    run_stage "Extended rvoip-controlled endpoint tests: 1003/2003" \
      "$SCRIPT_DIR/run_remote.sh"
    ;;
esac

echo
echo "========================================================================"
echo "== Full FreeSWITCH validation sequence complete"
echo "========================================================================"
