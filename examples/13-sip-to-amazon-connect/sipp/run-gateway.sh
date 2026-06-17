#!/usr/bin/env bash
# Start the example-13 gateway for the SIPp test: live Amazon Connect path,
# bound to loopback, routing to the agent flow so a CCP rings and can talk back.
# Reuses the same AWS config as the connect-probe (connect-probe.env).
set -euo pipefail
cd "$(dirname "$0")/.."  # examples/13-sip-to-amazon-connect

ENV_FILE="../../crates/webrtc/rvoip-amazon-connect/tools/connect-probe.env"
if [ -f "$ENV_FILE" ]; then
  echo "→ loading AWS config from $ENV_FILE"
  set -a; # shellcheck disable=SC1090
  source "$ENV_FILE"; set +a
fi

# Route to the agent flow (the one the voice widget uses) so the CCP rings.
export AMAZON_CONNECT_FLOW_ID="${AMAZON_CONNECT_FLOW_ID:-2a3b3059-6542-4d7e-b270-66642fa7b005}"
# Loopback so SIPp on the same host reaches it and media advertises 127.0.0.1.
export SIP_BIND_IP="${SIP_BIND_IP:-127.0.0.1}"
export SIP_PORT="${SIP_PORT:-5060}"
export RUST_LOG="${RUST_LOG:-info,rvoip_amazon_connect=debug}"

echo "→ instance=$AMAZON_CONNECT_INSTANCE_ID flow=$AMAZON_CONNECT_FLOW_ID region=${AWS_REGION:-?}"
echo "→ SIP UAS on $SIP_BIND_IP:$SIP_PORT — point SIPp here."
exec cargo run --features aws-live
