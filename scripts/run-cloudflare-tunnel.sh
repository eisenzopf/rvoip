#!/usr/bin/env bash
# Front a local Parley (HTTP 8080, UCTP 7443) with Cloudflare Tunnel.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CONFIG="${CLOUDFLARE_TUNNEL_CONFIG:-$ROOT/deploy/cloudflare/config.yml}"
HTTP_PROBE="${PARLEY_BIND_HTTP:-127.0.0.1:8080}"
UCTP_PROBE="${PARLEY_BIND_UCTP_WS:-127.0.0.1:7443}"

die() {
  echo "error: $*" >&2
  exit 1
}

if ! command -v cloudflared >/dev/null 2>&1; then
  die "cloudflared is not installed. See https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/"
fi

echo "Cloudflare Tunnel for Parley"
echo "  HTTPS  https://parley.rudeless.ai        -> http://$HTTP_PROBE"
echo "  WSS    wss://parley-uctp.rudeless.ai     -> http://$UCTP_PROBE"
echo "  SIP is not tunneled."
echo

if [[ -n "${CLOUDFLARE_TUNNEL_TOKEN:-}" ]]; then
  echo "Using CLOUDFLARE_TUNNEL_TOKEN (Zero Trust install token)."
  echo "Configure public hostnames parley.rudeless.ai and parley-uctp.rudeless.ai on that tunnel."
  exec cloudflared tunnel run --token "$CLOUDFLARE_TUNNEL_TOKEN"
fi

if [[ ! -f "$CONFIG" ]]; then
  die "missing $CONFIG (or set CLOUDFLARE_TUNNEL_TOKEN)"
fi

if grep -q 'PARLEY_TUNNEL_UUID' "$CONFIG"; then
  die "replace PARLEY_TUNNEL_UUID in $CONFIG after: cloudflared tunnel create parley"
fi

echo "Using config $CONFIG"
exec cloudflared tunnel --config "$CONFIG" run
