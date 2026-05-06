#!/usr/bin/env bash
# Run the local developer-facing session-core examples.
set -euo pipefail
cd "$(dirname "$0")"

SCRIPTS=(
  "cargo run -p rvoip-session-core --example endpoint_local_call --quiet"
  "cargo run -p rvoip-session-core --example endpoint_audio_roundtrip --quiet"
  "cargo run -p rvoip-session-core --example endpoint_incoming_redirect --quiet"
  "cargo run -p rvoip-session-core --example stream_peer_basic_call --quiet"
  "./stream_peer/02_call_control/run.sh"
  "./stream_peer/03_audio/run.sh"
  "./stream_peer/04_registration/run.sh"
  "./stream_peer/05_blind_transfer/run.sh"
  "./stream_peer/06_concurrent_calls/run.sh"
  "./callback_peer/01_auto_answer/run.sh"
  "./callback_peer/02_closure_gatekeeper/run.sh"
  "./callback_peer/03_builder_ivr/run.sh"
  "./callback_peer/04_routing_handler/run.sh"
  "./callback_peer/05_queue_handler/run.sh"
  "./callback_peer/06_trait_handler/run.sh"
  "cargo run -p rvoip-session-core --example unified_basic_call --quiet"
  "cargo run -p rvoip-session-core --example unified_event_filters --quiet"
  "./unified/04_b2bua_bridge/run.sh"
)

START=$SECONDS

for command in "${SCRIPTS[@]}"; do
  echo
  echo "==> $command"
  bash -lc "$command"
  sleep 1
done

echo
echo "All ${#SCRIPTS[@]} developer examples passed in $((SECONDS - START))s"
