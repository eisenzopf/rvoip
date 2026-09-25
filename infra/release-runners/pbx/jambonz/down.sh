#!/usr/bin/env bash
set -Eeuo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(git -C "$HERE" rev-parse --show-toplevel)"
STATE="$ROOT/target/release-interop/jambonz"
SOURCE_ROOT="$STATE/source"

# shellcheck disable=SC1091
source "$HERE/versions.env"
export JAMBONZ_OUTBOUND_APP_IMAGE="rvoip-jambonz-sbc-outbound:$JAMBONZ_OUTBOUND_COMMIT"
export JAMBONZ_MYSQL_FIXTURE_IMAGE="rvoip-jambonz-mysql-fixture:$JAMBONZ_OUTBOUND_COMMIT"
export JAMBONZ_DRACHTIO_IMAGE JAMBONZ_AUTH_IMAGE
export JAMBONZ_REGISTRAR_IMAGE JAMBONZ_REDIS_IMAGE JAMBONZ_RTPENGINE_IMAGE
export JAMBONZ_INFLUXDB_IMAGE JAMBONZ_MYSQL_IMAGE JAMBONZ_SOURCE_BUILDER_IMAGE
export JAMBONZ_RTP_ADVERTISED_IP="${JAMBONZ_RTP_ADVERTISED_IP:-172.39.0.12}"
export JAMBONZ_RTP_PORT_START="${JAMBONZ_RTP_PORT_START:-10000}"
export JAMBONZ_RTP_PORT_END="${JAMBONZ_RTP_PORT_END:-10199}"

compose_files=(--file "$HERE/docker-compose.yml")
if [[ "$(uname -s)" == "Darwin" ]]; then
  compose_files+=(--file "$HERE/docker-compose.colima.yml")
fi
docker compose --project-name rvoip-jambonz "${compose_files[@]}" \
  down --volumes --remove-orphans >/dev/null 2>&1 || true

if docker ps --all --filter 'name=^/rvoip-jambonz-' --format '{{.Names}}' | grep -q .; then
  echo "Jambonz cleanup left containers behind" >&2
  docker ps --all --filter 'name=^/rvoip-jambonz-' --format '{{.Names}}' >&2
  exit 1
fi
if docker network inspect rvoip-jambonz >/dev/null 2>&1; then
  echo "Jambonz cleanup left the rvoip-jambonz network behind" >&2
  exit 1
fi

if [[ -n "${JAMBONZ_RECEIPT_DIR:-}" ]]; then
  mkdir -p "$JAMBONZ_RECEIPT_DIR"
  {
    echo "status=PASS"
    echo "containers_remaining=0"
    echo "networks_remaining=0"
  } >"$JAMBONZ_RECEIPT_DIR/cleanup.txt"
fi
