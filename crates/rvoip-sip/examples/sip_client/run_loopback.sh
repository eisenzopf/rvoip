#!/usr/bin/env sh
# Print the two commands needed for a local loopback softphone call.
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
WORKSPACE_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../../../.." && pwd)

cat <<EOF
Run a local RVoIP softphone call with two terminals.

Terminal 1:
cd "$WORKSPACE_ROOT"
cargo run -p rvoip-sip --example sip_client -- --preset bob-loopback

Terminal 2:
cd "$WORKSPACE_ROOT"
cargo run -p rvoip-sip --example sip_client -- --preset alice-loopback --dial sip:bob@127.0.0.1:5081

In Bob's terminal, press "a" to answer. Press "h" to hang up and "q" to quit.
EOF
