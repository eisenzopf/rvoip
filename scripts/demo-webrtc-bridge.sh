#!/usr/bin/env bash
# WHIP → WebRTC server → orchestrator → synthetic QUIC leg (bridge demo).
set -euo pipefail

cd "$(dirname "$0")/.."

export WHIP_BIND="${WHIP_BIND:-127.0.0.1:8080}"

exec cargo run -p rvoip-webrtc --example webrtc_bridge_demo --features signaling-whip
