#!/usr/bin/env bash
set -Eeuo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(git -C "$HERE" rev-parse --show-toplevel)"
STATE="$ROOT/target/release-interop/jambonz"
SOURCE_ROOT="$STATE/source"
RECEIPT_DIR="${JAMBONZ_RECEIPT_DIR:-$STATE/evidence}"
LOCAL_ENV_ROOT="${RVOIP_PBX_LOCAL_ENV_ROOT:-$STATE/local-env}"

# shellcheck disable=SC1091
source "$HERE/versions.env"

compose_files=(--file "$HERE/docker-compose.yml")
lab_access=linux-bridge
if [[ "$(uname -s)" == "Darwin" ]]; then
  compose_files+=(--file "$HERE/docker-compose.colima.yml")
  lab_access=colima-host-forward
fi

compose_lab() {
  docker compose --project-name rvoip-jambonz "${compose_files[@]}" "$@"
}

verify_colima_udp_forwarding() {
  local probe_container=rvoip-jambonz-colima-udp-probe
  local probe_port="${JAMBONZ_COLIMA_UDP_PROBE_PORT:-55061}"
  docker rm --force "$probe_container" >/dev/null 2>&1 || true
  docker run --detach --rm --name "$probe_container" \
    --publish "127.0.0.1:$probe_port:$probe_port/udp" \
    --entrypoint node "$JAMBONZ_SOURCE_BUILDER_IMAGE" -e \
    "const d=require('node:dgram').createSocket('udp4');d.on('message',(m,r)=>d.send(m,r.port,r.address));d.bind($probe_port,'0.0.0.0')" \
    >/dev/null
  if ! python3 - "$probe_port" <<'PY'
import socket
import sys
import time

port = int(sys.argv[1])
payload = b"rvoip-colima-udp-probe"
sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
sock.settimeout(0.5)
deadline = time.monotonic() + 20
while time.monotonic() < deadline:
    sock.sendto(payload, ("127.0.0.1", port))
    try:
        response, _ = sock.recvfrom(1024)
    except TimeoutError:
        continue
    if response == payload:
        break
else:
    raise SystemExit(1)
PY
  then
    docker logs "$probe_container" >&2 || true
    docker rm --force "$probe_container" >/dev/null 2>&1 || true
    cat >&2 <<EOF
Colima did not forward the UDP readiness probe. SIP/RTP interop requires a
dedicated x86_64 profile created with '--port-forwarder grpc'; the default SSH
forwarder carries TCP only. Recreate the disposable profile with that option.
EOF
    exit 1
  fi
  docker rm --force "$probe_container" >/dev/null
}

engine_arch="$(docker info --format '{{.Architecture}}')"
case "$engine_arch" in
  amd64|x86_64) ;;
  *)
    cat >&2 <<EOF
Jambonz release interop requires an amd64 Docker engine because its pinned
MySQL 5.7 and Jambonz images are amd64-only. This engine reports '$engine_arch'.
With Colima on Apple Silicon, use a dedicated x86_64 Colima profile, or run the
mandatory gate on the repository's x86 GCP interop worker.
EOF
    exit 1
    ;;
esac

fetch_component() {
  local name="$1"
  local repository="$2"
  local revision="$3"
  local expected_sha="$4"
  local destination="$SOURCE_ROOT/$name"
  local archive="$STATE/$name.tar.gz"
  rm -rf "$destination"
  mkdir -p "$destination"
  curl --fail --silent --show-error --location \
    "https://codeload.github.com/$repository/tar.gz/$revision" \
    --output "$archive"
  python3 - "$archive" "$expected_sha" <<'PY'
from pathlib import Path
import hashlib
import sys

path = Path(sys.argv[1])
expected = sys.argv[2]
actual = hashlib.sha256(path.read_bytes()).hexdigest()
if actual != expected:
    raise SystemExit(f"sha256 mismatch for {path}: expected {expected}, got {actual}")
PY
  tar --extract --gzip --file "$archive" --directory "$destination" --strip-components=1
  python3 - "$destination/package.json" "$JAMBONZ_RELEASE_LINE" <<'PY'
from pathlib import Path
import json
import sys

actual = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))["version"]
expected = sys.argv[2]
if actual != expected:
    raise SystemExit(f"component version mismatch: expected {expected}, got {actual}")
PY
}

"$HERE/down.sh"
mkdir -p "$SOURCE_ROOT" "$RECEIPT_DIR" "$LOCAL_ENV_ROOT/jambonz"

python3 "$HERE/verify-latest.py" --receipt "$RECEIPT_DIR/latest.json"
fetch_component inbound jambonz/sbc-inbound \
  "$JAMBONZ_INBOUND_COMMIT" "$JAMBONZ_INBOUND_TARBALL_SHA256"
fetch_component outbound jambonz/sbc-outbound \
  "$JAMBONZ_OUTBOUND_COMMIT" "$JAMBONZ_OUTBOUND_TARBALL_SHA256"

docker build --platform linux/amd64 \
  --file "$HERE/component.Dockerfile" \
  --build-arg "SOURCE_URL=https://codeload.github.com/jambonz/sbc-outbound/tar.gz/$JAMBONZ_OUTBOUND_COMMIT" \
  --build-arg "SOURCE_SHA256=$JAMBONZ_OUTBOUND_TARBALL_SHA256" \
  --build-arg "SOURCE_VERSION=$JAMBONZ_RELEASE_LINE" \
  --tag "rvoip-jambonz-sbc-outbound:$JAMBONZ_OUTBOUND_COMMIT" \
  "$HERE"

docker build --platform linux/amd64 \
  --file "$HERE/fixture.Dockerfile" \
  --target mysql \
  --build-arg "SOURCE_BUILDER_IMAGE=$JAMBONZ_SOURCE_BUILDER_IMAGE" \
  --build-arg "SOURCE_URL=https://codeload.github.com/jambonz/sbc-outbound/tar.gz/$JAMBONZ_OUTBOUND_COMMIT" \
  --build-arg "SOURCE_SHA256=$JAMBONZ_OUTBOUND_TARBALL_SHA256" \
  --build-arg "SOURCE_VERSION=$JAMBONZ_RELEASE_LINE" \
  --build-arg "MYSQL_IMAGE=$JAMBONZ_MYSQL_IMAGE" \
  --tag "rvoip-jambonz-mysql-fixture:$JAMBONZ_OUTBOUND_COMMIT" \
  "$HERE"

export JAMBONZ_OUTBOUND_APP_IMAGE="rvoip-jambonz-sbc-outbound:$JAMBONZ_OUTBOUND_COMMIT"
export JAMBONZ_MYSQL_FIXTURE_IMAGE="rvoip-jambonz-mysql-fixture:$JAMBONZ_OUTBOUND_COMMIT"
export JAMBONZ_DRACHTIO_IMAGE JAMBONZ_AUTH_IMAGE
export JAMBONZ_REGISTRAR_IMAGE JAMBONZ_REDIS_IMAGE JAMBONZ_RTPENGINE_IMAGE
export JAMBONZ_INFLUXDB_IMAGE JAMBONZ_MYSQL_IMAGE JAMBONZ_SOURCE_BUILDER_IMAGE

if [[ "$lab_access" == "colima-host-forward" ]]; then
  verify_colima_udp_forwarding
  host_gateway="$(docker run --rm --platform linux/amd64 \
    --entrypoint node "$JAMBONZ_SOURCE_BUILDER_IMAGE" -e \
    "require('node:dns').lookup('host.docker.internal',{family:4},(e,a)=>{if(e){console.error(e.message);process.exit(1)}process.stdout.write(a)})")"
  if [[ ! "$host_gateway" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "Unable to resolve Colima's host.docker.internal IPv4 address" >&2
    exit 1
  fi
  jambonz_addr="127.0.0.1:${JAMBONZ_HOST_SIP_PORT:-55060}"
  advertised_ip="$host_gateway"
  export JAMBONZ_RTP_ADVERTISED_IP=127.0.0.1
else
  jambonz_addr="172.39.0.10:5060"
  advertised_ip="172.39.0.1"
  export JAMBONZ_RTP_ADVERTISED_IP=172.39.0.12
fi
export JAMBONZ_RTP_PORT_START="${JAMBONZ_RTP_PORT_START:-10000}"
export JAMBONZ_RTP_PORT_END="${JAMBONZ_RTP_PORT_END:-10199}"

compose_lab up --detach

for _ in $(seq 1 120); do
  if docker exec rvoip-jambonz-sbc-outbound node -e \
    "fetch('http://127.0.0.1:3050/system-health').then(r => {if (!r.ok) process.exit(1)})" \
    >/dev/null 2>&1; then
    break
  fi
  if ! docker inspect --format '{{.State.Running}}' rvoip-jambonz-sbc-outbound 2>/dev/null \
    | grep -qx true; then
    compose_lab logs >&2
    exit 1
  fi
  sleep 1
done
docker exec rvoip-jambonz-sbc-outbound node -e \
  "fetch('http://127.0.0.1:3050/system-health').then(r => {if (!r.ok) process.exit(1)})"

for required_container in \
  rvoip-jambonz-mysql \
  rvoip-jambonz-drachtio \
  rvoip-jambonz-redis \
  rvoip-jambonz-rtpengine \
  rvoip-jambonz-registrar \
  rvoip-jambonz-auth \
  rvoip-jambonz-sbc-outbound \
  rvoip-jambonz-influxdb
do
  if [[ "$(docker inspect --format '{{.State.Running}}' "$required_container" 2>/dev/null || true)" != "true" ]]; then
    echo "Required Jambonz container is not running: $required_container" >&2
    compose_lab logs >&2
    exit 1
  fi
done

cat >"$LOCAL_ENV_ROOT/jambonz/jambonz-local.env" <<EOF
JAMBONZ_UDP_ADDR=$jambonz_addr
JAMBONZ_SIP_DOMAIN=sip.rvoip.test
JAMBONZ_PASSWORD=1234
RVOIP_LOCAL_IP=0.0.0.0
RVOIP_ADVERTISED_IP=$advertised_ip
RVOIP_MEDIA_ADVERTISED_IP=$advertised_ip
JAMBONZ_RTP_START=$JAMBONZ_RTP_PORT_START
JAMBONZ_RTP_END=$JAMBONZ_RTP_PORT_END
JAMBONZ_POST_REGISTER_SETTLE_SECS=2
JAMBONZ_TEST_TIMEOUT_SECS=90
EOF

compose_lab config >"$RECEIPT_DIR/compose-rendered.yaml"
docker inspect \
  rvoip-jambonz-drachtio rvoip-jambonz-registrar rvoip-jambonz-sbc-outbound \
  rvoip-jambonz-rtpengine >"$RECEIPT_DIR/container-inspect.json"
compose_lab ps >"$RECEIPT_DIR/compose-ps.txt"
{
  echo "access_mode=$lab_access"
  echo "jambonz_addr=$jambonz_addr"
  echo "rvoip_advertised_ip=$advertised_ip"
  echo "rtpengine_advertised_ip=$JAMBONZ_RTP_ADVERTISED_IP"
  echo "rtpengine_port_range=$JAMBONZ_RTP_PORT_START-$JAMBONZ_RTP_PORT_END"
} >"$RECEIPT_DIR/network-topology.txt"

printf 'Jambonz OSS %s is ready at %s (%s)\n' \
  "$JAMBONZ_RELEASE_LINE" "$jambonz_addr" "$lab_access"
