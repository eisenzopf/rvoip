#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../../../.."

cargo run -p rvoip-session-core --example freeswitch_udp_call_callee &
callee_pid=$!

cleanup() {
  kill "$callee_pid" 2>/dev/null || true
}
trap cleanup EXIT

sleep "${FREESWITCH_CALLEE_STARTUP_SECS:-2}"
cargo run -p rvoip-session-core --example freeswitch_udp_call_caller
wait "$callee_pid"
