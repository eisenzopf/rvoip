#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../../.."

cleanup() { pkill -P $$ 2>/dev/null || true; wait 2>/dev/null || true; }
trap cleanup EXIT

cargo build -p rvoip-session-core \
  --example callback_peer_builder_ivr_server \
  --example callback_peer_builder_ivr_client

cargo run -p rvoip-session-core --example callback_peer_builder_ivr_server --quiet &
sleep 1
cargo run -p rvoip-session-core --example callback_peer_builder_ivr_client --quiet
