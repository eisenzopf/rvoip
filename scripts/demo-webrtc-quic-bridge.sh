#!/usr/bin/env bash
# WHIP → WebRTC server → orchestrator → real rvoip-quic UCTP leg.
set -euo pipefail

cd "$(dirname "$0")/.."

export WHIP_BIND="${WHIP_BIND:-127.0.0.1:8080}"
export QUIC_BIND="${QUIC_BIND:-127.0.0.1:4433}"

exec cargo run -p rvoip-webrtc --example webrtc_quic_bridge_demo --features bridge-quic
