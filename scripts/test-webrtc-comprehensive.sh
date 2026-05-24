#!/usr/bin/env bash
# Comprehensive WebRTC server + client validation (audio, video, data channel, DTMF).
set -euo pipefail

cd "$(dirname "$0")/.."

export WS_BIND="${WS_BIND:-127.0.0.1:0}"
export READY_FILE="${READY_FILE:-$(mktemp)}"
export MEDIUM="${MEDIUM:-audiovideo}"
trap 'rm -f "$READY_FILE"; kill "$SERVER_PID" 2>/dev/null || true' EXIT

echo "Starting comprehensive WebRTC server (WS_BIND=$WS_BIND)..."
cargo run -p rvoip-webrtc --example webrtc_comprehensive_server \
  --features comprehensive &
SERVER_PID=$!

for _ in $(seq 1 50); do
  if [[ -s "$READY_FILE" ]]; then
    break
  fi
  sleep 0.2
done

if [[ ! -s "$READY_FILE" ]]; then
  echo "server did not write READY_FILE within timeout" >&2
  exit 1
fi

WS_URL="ws://$(cat "$READY_FILE")"
export WS_URL
echo "Server ready at $WS_URL — running client (medium=$MEDIUM)..."

cargo run -p rvoip-webrtc --example webrtc_comprehensive_client \
  --features comprehensive -- "$MEDIUM"

echo "Comprehensive WebRTC client/server test passed."
