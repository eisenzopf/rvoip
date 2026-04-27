#!/usr/bin/env sh
# Asterisk registration smoke tests: register secure user 1001 over SIP TLS,
# then register UDP user 2001. Each registration unregisters before the next
# stage starts.
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
ASTERISK_DIR=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
WORKSPACE_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../../../../.." && pwd)

if [ -f "$ASTERISK_DIR/.env" ]; then
  set -a
  # shellcheck disable=SC1091
  . "$ASTERISK_DIR/.env"
  set +a
fi

run_registration() {
  label=$1
  transport=$2
  username=$3
  local_port=$4

  echo
  echo "=== Registration: $label ==="
  SIP_TRANSPORT="$transport" \
  SIP_USERNAME="$username" \
  SIP_AUTH_USERNAME="$username" \
  LOCAL_PORT="$local_port" \
  IDLE_SECS="${REGISTRATION_IDLE_SECS:-2}" \
    cargo run -p rvoip-session-core --features dev-insecure-tls \
      --example asterisk_registration --quiet
}

cd "$WORKSPACE_ROOT"

echo "Building Asterisk registration example..."
cargo build -p rvoip-session-core --features dev-insecure-tls \
  --example asterisk_registration

run_registration "TLS user 1001" "TLS" "1001" "${ENDPOINT_1001_LOCAL_PORT:-5070}"
run_registration "UDP user 2001" "UDP" "2001" "${ENDPOINT_2001_LOCAL_PORT:-5080}"

echo
echo "=== Asterisk registration smoke tests complete ==="
