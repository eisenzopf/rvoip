#!/usr/bin/env sh
# FreeSWITCH registration smoke tests: register UDP user 2001 against the
# rvoip_udp profile, then TLS/SRTP user 1001 against rvoip_tls_srtp.
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
FREESWITCH_DIR=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
WORKSPACE_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../../../../.." && pwd)

if [ -f "$HOME/Developer/freeswitch/freeswitch-local.env" ]; then
  set -a
  # shellcheck disable=SC1091
  . "$HOME/Developer/freeswitch/freeswitch-local.env"
  set +a
fi

if [ -f "$FREESWITCH_DIR/.env" ]; then
  set -a
  # shellcheck disable=SC1091
  . "$FREESWITCH_DIR/.env"
  set +a
fi

# shellcheck disable=SC1091
. "$FREESWITCH_DIR/tls_cert.sh"

run_registration() {
  label=$1
  transport=$2
  username=$3
  local_port=$4

  echo
  echo "=== Registration: $label ==="
  env \
  SIP_TRANSPORT="$transport" \
  SIP_USERNAME="$username" \
  "ENDPOINT_${username}_LOCAL_PORT=$local_port" \
  IDLE_SECS="${REGISTRATION_IDLE_SECS:-2}" \
  TLS_INSECURE="${TLS_INSECURE:-1}" \
    cargo run -p rvoip-session-core --features dev-insecure-tls \
      --example freeswitch_registration --quiet
}

cd "$WORKSPACE_ROOT"

ensure_freeswitch_tls_listener_cert "$SCRIPT_DIR/output/tls"

echo "Building FreeSWITCH registration example..."
cargo build -p rvoip-session-core --features dev-insecure-tls \
  --example freeswitch_registration

run_registration "UDP user 2001" "UDP" "2001" "${ENDPOINT_2001_LOCAL_PORT:-15080}"
run_registration "TLS/SRTP user 1001" "TLS" "1001" "${ENDPOINT_1001_LOCAL_PORT:-15070}"

echo
echo "=== FreeSWITCH registration smoke tests complete ==="
