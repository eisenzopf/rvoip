#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../../.."

cleanup() { pkill -P $$ 2>/dev/null || true; wait 2>/dev/null || true; }
trap cleanup EXIT

cargo build -p rvoip-session-core \
  --example stream_peer_call_control_server \
  --example stream_peer_call_control_client

cargo run -p rvoip-session-core --example stream_peer_call_control_server --quiet &
sleep 1
cargo run -p rvoip-session-core --example stream_peer_call_control_client --quiet
