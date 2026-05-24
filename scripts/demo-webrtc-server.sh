#!/usr/bin/env bash
# Start the rvoip-webrtc dual-role server (WHIP + WS + orchestrator).
set -euo pipefail

cd "$(dirname "$0")/.."

export WHIP_BIND="${WHIP_BIND:-127.0.0.1:8080}"
export WS_BIND="${WS_BIND:-127.0.0.1:8081}"

exec cargo run -p rvoip-webrtc --example webrtc_server \
  --features signaling-whip,signaling-ws
