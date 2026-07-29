#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
INTEROP_DIR=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
COMPOSE_FILE="$INTEROP_DIR/docker-compose.yml"
COLIMA_HOST_COMPOSE_FILE="$INTEROP_DIR/docker-compose.colima-host.yml"

export KAMAILIO_IMAGE=${KAMAILIO_IMAGE:-ghcr.io/kamailio/kamailio:6.1.3-bookworm@sha256:26b26c61801d679ffbe54ea3597c38964a46c4bfe60fb6537c7eeacc576b0c92}
export OPENSIPS_IMAGE=${OPENSIPS_IMAGE:-opensips/opensips:3.6@sha256:eba1396b438a7f8a9d33c17017aae4670cb43361eb7130359240cf85fc3e6979}
export KAMAILIO_PLATFORM=${KAMAILIO_PLATFORM:-linux/amd64}
export OPENSIPS_PLATFORM=${OPENSIPS_PLATFORM:-linux/amd64}
export PROXY_INTEROP_PEER_PORT=${PROXY_INTEROP_PEER_PORT:-25070}
export PROXY_INTEROP_PROJECT=${PROXY_INTEROP_PROJECT:-rvoip-proxy-interop}

COMPOSE_FILE_ARGS=(--file "$COMPOSE_FILE")
if command -v colima >/dev/null 2>&1 \
  && [[ "$(docker context show 2>/dev/null)" == "colima" ]]; then
  COMPOSE_FILE_ARGS+=(--file "$COLIMA_HOST_COMPOSE_FILE")
  export PROXY_INTEROP_NETWORK_TOPOLOGY=colima-vm-host
else
  export PROXY_INTEROP_NETWORK_TOPOLOGY=${PROXY_INTEROP_NETWORK_TOPOLOGY:-portable-published-port}
fi
if [[ "${PROXY_INTEROP_TLS_ENABLED:-0}" == 1 ]]; then
  COMPOSE_FILE_ARGS+=(--file "$INTEROP_DIR/docker-compose.tls.yml")
fi

compose() {
  docker compose \
    --project-name "$PROXY_INTEROP_PROJECT" \
    "${COMPOSE_FILE_ARGS[@]}" \
    "$@"
}

require_peer() {
  case "${1:-}" in
    kamailio|opensips) ;;
    *)
      echo "peer must be 'kamailio' or 'opensips'" >&2
      return 2
      ;;
  esac
}

peer_version_command() {
  case "$1" in
    kamailio) printf '%s\n' "/usr/sbin/kamailio -V" ;;
    opensips) printf '%s\n' "/usr/sbin/opensips -V" ;;
  esac
}
