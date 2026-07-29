#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
# shellcheck source=common.sh
. "$SCRIPT_DIR/common.sh"

peer=${1:-}
config=${2:-${PROXY_INTEROP_PEER_CONFIG:-}}
require_peer "$peer"
if [[ -z "$config" || ! -s "$config" ]]; then
  echo "usage: $0 <kamailio|opensips> <rendered-config>" >&2
  exit 2
fi
if ! docker info >/dev/null 2>&1; then
  echo "Docker is not reachable" >&2
  exit 1
fi

export PROXY_INTEROP_PEER_CONFIG
PROXY_INTEROP_PEER_CONFIG=$(cd "$(dirname "$config")" && pwd -P)/"$(basename "$config")"
export PROXY_INTEROP_PEER_CONFIG

compose up --detach "$peer"

for _ in $(seq 1 60); do
  container_id=$(compose ps --quiet "$peer")
  if [[ -n "$container_id" ]]; then
    state=$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}' "$container_id")
    if [[ "$state" == "healthy" || "$state" == "running" ]]; then
      version_command=$(peer_version_command "$peer")
      compose exec --no-TTY "$peer" sh -c "$version_command"
      exit 0
    fi
    if [[ "$state" == "unhealthy" || "$state" == "exited" || "$state" == "dead" ]]; then
      break
    fi
  fi
  sleep 0.5
done

echo "$peer did not become ready" >&2
compose ps >&2 || true
compose logs --no-color --tail=200 "$peer" >&2 || true
exit 1
