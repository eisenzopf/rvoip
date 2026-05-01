#!/usr/bin/env sh
# Extended FreeSWITCH validation sequence using rvoip-controlled 1003/2003 endpoints.
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

assert_no_freeswitch_channels() {
  label=$1

  if ! command -v docker >/dev/null 2>&1; then
    echo "[freeswitch] docker not available; skipping channel leak check for $label"
    return 0
  fi
  if ! docker ps --format '{{.Names}}' | grep -qx 'rvoip-freeswitch'; then
    echo "[freeswitch] rvoip-freeswitch is not running; skipping channel leak check for $label"
    return 0
  fi

  sleep 2
  count=$(docker exec rvoip-freeswitch fs_cli -x 'show channels count' 2>/dev/null \
    | tr -d '\r' \
    | awk '
        /^[[:space:]]*[0-9]+[[:space:]]*$/ { print $1; exit }
        /total/ {
          for (i = 1; i <= NF; i++) {
            if ($i ~ /^[0-9]+$/) { print $i; exit }
          }
        }
      ')
  if [ "${count:-0}" != "0" ]; then
    echo "[freeswitch] channel leak after $label:"
    docker exec rvoip-freeswitch fs_cli -x 'show channels'
    exit 1
  fi
  echo "[freeswitch] no remaining channels after $label"
}

run_stage "TLS/SRTP ring/cancel: 1001 -> 1003" \
  "$SCRIPT_DIR/tls_srtp_ring_remote/run.sh"
run_stage "UDP ring/cancel: 2001 -> 2003" \
  "$SCRIPT_DIR/udp_ring_remote/run.sh"
run_stage "TLS/SRTP DTMF through FreeSWITCH: 1001 -> 1002" \
  "$SCRIPT_DIR/tls_srtp_dtmf/run.sh"
run_stage "UDP DTMF through FreeSWITCH: 2001 -> 2002" \
  "$SCRIPT_DIR/udp_dtmf/run.sh"
run_stage "TLS/SRTP blind transfer to 1003" \
  "$SCRIPT_DIR/tls_srtp_blind_transfer_remote/run.sh"
assert_no_freeswitch_channels "TLS/SRTP blind transfer to 1003"
run_stage "UDP blind transfer to 2003" \
  "$SCRIPT_DIR/udp_blind_transfer_remote/run.sh"
assert_no_freeswitch_channels "UDP blind transfer to 2003"

echo
echo "========================================================================"
echo "== Extended multi-endpoint FreeSWITCH validation sequence complete"
echo "========================================================================"
