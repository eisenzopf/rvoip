#!/usr/bin/env bash
# Run connect-probe against live Amazon Connect with the Chime wire-frame trace
# streaming to the console. Loads config from an env file (default:
# tools/connect-probe.env next to this script) or the environment.
#
# Usage:
#   tools/run-probe.sh                  # A→B→C handshake test
#   tools/run-probe.sh --audio-secs 10  # also count inbound audio (needs an answer)
#   tools/run-probe.sh --dump-frames    # also emit our JOIN/SUBSCRIBE base64 (stdout)
#
# Anything after the script name is passed through to connect-probe.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# tools -> rvoip-amazon-connect -> webrtc -> crates -> repo root
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"

ENV_FILE="${PROBE_ENV:-$SCRIPT_DIR/connect-probe.env}"
if [[ -f "$ENV_FILE" ]]; then
  echo "→ loading config from $ENV_FILE"
  set -a; # shellcheck disable=SC1090
  source "$ENV_FILE"; set +a
fi

missing=()
for v in AMAZON_CONNECT_INSTANCE_ID AMAZON_CONNECT_FLOW_ID AWS_REGION; do
  [[ -n "${!v:-}" ]] || missing+=("$v")
done
if [[ ${#missing[@]} -gt 0 ]]; then
  echo "✗ missing required config: ${missing[*]}" >&2
  echo "  set them in $ENV_FILE (see connect-probe.env.example) or export them." >&2
  exit 2
fi

if [[ -z "${AWS_ACCESS_KEY_ID:-}" && -z "${AWS_PROFILE:-}" && -z "${AWS_SESSION_TOKEN:-}" ]]; then
  echo "⚠ no AWS_ACCESS_KEY_ID / AWS_PROFILE set — relying on the default AWS" >&2
  echo "  credential chain (instance/ECS role, SSO, etc.). Stage A will fail if none resolves." >&2
fi

# Stream the Chime signaling frames (base64) to stderr alongside library logs.
export RUST_LOG="${RUST_LOG:-info,rvoip_amazon_connect=debug,rvoip_amazon_connect::chime_wire=trace}"

echo "→ instance=$AMAZON_CONNECT_INSTANCE_ID flow=$AMAZON_CONNECT_FLOW_ID region=$AWS_REGION"
echo "→ RUST_LOG=$RUST_LOG"
cd "$REPO_ROOT"
exec cargo run --bin connect-probe --features aws-control -- "$@"
