#!/usr/bin/env bash
set -Eeuo pipefail

ACTION="${1:?usage: interop-lifecycle.sh ACTION}"
ROOT="$(git rev-parse --show-toplevel)"
STATE="$ROOT/target/release-interop"
PBX_SNAPSHOT="$ROOT/infra/release-runners/pbx"
if [[ -n "${RVOIP_PBX_LOCAL_ENV_ROOT:-}" ]]; then
  LOCAL_ENV_ROOT="$RVOIP_PBX_LOCAL_ENV_ROOT"
elif [[ -n "${HOME:-}" ]]; then
  LOCAL_ENV_ROOT="$HOME/Developer"
else
  LOCAL_ENV_ROOT="$STATE/local-env"
fi
mkdir -p "$STATE" "$LOCAL_ENV_ROOT/asterisk" "$LOCAL_ENV_ROOT/freeswitch" "$LOCAL_ENV_ROOT/jambonz"

wait_udp_port() {
  local container="$1"
  local port="$2"
  local port_hex
  printf -v port_hex '%04X' "$port"
  for _ in $(seq 1 180); do
    if docker exec "$container" awk -v port=":$port_hex" '
      $2 ~ port "$" && $4 == "07" { found = 1 }
      END { exit !found }
    ' /proc/net/udp /proc/net/udp6 >/dev/null 2>&1; then
      return 0
    fi
    if ! docker inspect -f '{{.State.Running}}' "$container" 2>/dev/null | grep -qx true; then
      echo "$container exited before UDP port $port became ready" >&2
      docker logs "$container" >&2 || true
      return 1
    fi
    sleep 1
  done
  echo "UDP port $port did not become ready in $container" >&2
  docker logs "$container" >&2 || true
  return 1
}

wait_freeswitch_control_plane() {
  local container="$1"
  local password="$2"
  local attempts="${RVOIP_FREESWITCH_READY_ATTEMPTS:-180}"
  local consecutive_ready=0
  local sofia_status=""

  for _ in $(seq 1 "$attempts"); do
    if sofia_status="$(
      docker exec "$container" fs_cli -p "$password" \
        -x "sofia status" 2>&1
    )" && grep -Eq 'rvoip_udp[[:space:]].*RUNNING' <<<"$sofia_status" \
      && grep -Eq 'rvoip_tls_srtp[[:space:]].*RUNNING' <<<"$sofia_status"; then
      consecutive_ready="$((consecutive_ready + 1))"
      if [[ "$consecutive_ready" -ge 2 ]]; then
        return 0
      fi
    else
      consecutive_ready=0
    fi

    if ! docker inspect -f '{{.State.Running}}' "$container" 2>/dev/null \
      | grep -qx true; then
      echo "$container exited before the FreeSWITCH control plane became ready" >&2
      docker logs "$container" >&2 || true
      return 1
    fi
    sleep 1
  done

  echo "FreeSWITCH control plane did not become ready in $container" >&2
  printf 'sofia status:\n%s\n' "$sofia_status" >&2
  docker logs "$container" >&2 || true
  return 1
}

down() {
  docker rm -f "$1" >/dev/null 2>&1 || true
}

asterisk_up() {
  down rvoip-release-asterisk
  local build="$STATE/asterisk"
  rm -rf "$build"
  cp -R "$PBX_SNAPSHOT/asterisk" "$build"
  python3 - "$build/config/pjsip.conf" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
path.write_text(
    path.read_text()
    .replace("<redacted>", "password123")
    .replace("192.168.64.2", "127.0.0.1")
)
PY
  mkdir -p "$build/keys"
  openssl req -x509 -newkey rsa:2048 -nodes \
    -keyout "$build/keys/asterisk-key.pem" \
    -out "$build/keys/asterisk.pem" -days 2 \
    -subj '/CN=localhost' -addext 'subjectAltName=DNS:localhost,IP:127.0.0.1' \
    >/dev/null 2>&1
  cp "$build/keys/asterisk.pem" "$build/keys/ca.pem"
  docker build --build-arg MAKE_JOBS="$(nproc)" -t rvoip-release-asterisk "$build"
  docker run -d --name rvoip-release-asterisk --network host \
    -v "$build/config/pjsip.conf:/etc/asterisk/pjsip.conf:ro" \
    -v "$build/config/extensions.conf:/etc/asterisk/extensions.conf:ro" \
    -v "$build/config/modules.conf:/etc/asterisk/modules.conf:ro" \
    -v "$build/keys:/etc/asterisk/keys:ro" \
    rvoip-release-asterisk >/dev/null
  wait_udp_port rvoip-release-asterisk 5060
  cat > "$LOCAL_ENV_ROOT/asterisk/rvoip-local.env" <<EOF
SIP_SERVER=127.0.0.1
SIP_PORT=5060
SIP_TRANSPORT=TLS
SIP_PASSWORD=password123
SIP_USERNAME=1001
SIP_AUTH_USERNAME=1001
SIP_TLS_PORT=5061
TLS_CA_PATH=$build/keys/ca.pem
TLS_INSECURE=1
ASTERISK_TLS_SRTP_REQUIRED=1
ASTERISK_TLS_CONTACT_MODE=reachable-contact
LOCAL_IP=127.0.0.1
ADVERTISED_IP=127.0.0.1
MEDIA_ADVERTISED_IP=127.0.0.1
LOCAL_PORT=5070
POST_REGISTER_SETTLE_SECS=2
REGISTRATION_IDLE_SECS=1
PBX_REQUIRE_AMR=1
EOF
}

freeswitch_up() {
  down rvoip-freeswitch
  local build="$STATE/freeswitch"
  rm -rf "$build"
  cp -R "$PBX_SNAPSHOT/freeswitch" "$build"
  python3 - "$build/Dockerfile" "$build/docker-entrypoint.sh" <<'PY'
from pathlib import Path
import sys

dockerfile = Path(sys.argv[1])
dockerfile.write_text(
    dockerfile.read_text()
    .replace("ENV FS_DEFAULT_PASSWORD=<redacted>", "ENV FS_DEFAULT_PASSWORD=1234")
    .replace(
        "ENV FS_EVENT_SOCKET_PASSWORD=<redacted>",
        "ENV FS_EVENT_SOCKET_PASSWORD=ClueCon",
    )
)
path = Path(sys.argv[2])
lines = []
for line in path.read_text().splitlines():
    if line == "  password=<redacted>":
        line = "  password=${FS_DEFAULT_PASSWORD:-1234}"
    elif line.startswith("set_xml_var default_password"):
        line = 'set_xml_var default_password "${FS_DEFAULT_PASSWORD:-1234}"'
    elif line.startswith('  sed -i "s#<param name=\\"password\\"'):
        line = "  sed -i 's|<param name=\"password\" value=\"[^\"]*\"/>|<param name=\"password\" value=\"'\"${FS_EVENT_SOCKET_PASSWORD:-ClueCon}\"'\"/>|g' \"$event_socket\""
    lines.append(line)
path.write_text("\n".join(lines) + "\n")
PY
  docker build --build-arg MAKE_JOBS="$(nproc)" -t rvoip-release-freeswitch "$build"
  docker run -d --name rvoip-freeswitch --network host \
    -e FS_DEFAULT_PASSWORD=1234 \
    -e FS_EVENT_SOCKET_PASSWORD=ClueCon \
    -e FS_SIP_IP=127.0.0.1 \
    -e FS_LOCAL_RTP_IP=127.0.0.1 \
    -e FS_EXTERNAL_SIP_IP=127.0.0.1 \
    -e FS_EXTERNAL_RTP_IP=127.0.0.1 \
    rvoip-release-freeswitch >/dev/null
  wait_udp_port rvoip-freeswitch 5062
  wait_freeswitch_control_plane rvoip-freeswitch ClueCon
  cat > "$LOCAL_ENV_ROOT/freeswitch/freeswitch-local.env" <<'EOF'
FREESWITCH_ADDR=127.0.0.1:5060
FREESWITCH_IP=127.0.0.1
FREESWITCH_SIP_PORT=5060
FREESWITCH_UDP_ADDR=127.0.0.1:5062
FREESWITCH_TLS_ADDR=127.0.0.1:5063
FREESWITCH_UDP_SIP_PORT=5062
FREESWITCH_TLS_SIP_PORT=5063
# Transcoding-enabled profile twins. run.sh redirects amr_transcode_call here,
# because the profiles above pin disable-transcoding and would refuse a call
# whose two legs share no codec -- which is exactly what that row asks for.
FREESWITCH_XCODE_UDP_ADDR=127.0.0.1:5064
FREESWITCH_XCODE_TLS_ADDR=127.0.0.1:5065
FREESWITCH_RTP_START=16384
FREESWITCH_RTP_END=16484
FREESWITCH_PASSWORD=1234
FREESWITCH_UDP_USERS=2001,2002,2003
FREESWITCH_TLS_USERS=1001,1002,1003
RVOIP_LOCAL_IP=127.0.0.1
RVOIP_ADVERTISED_IP=127.0.0.1
RVOIP_MEDIA_ADVERTISED_IP=127.0.0.1
PBX_REQUIRE_AMR=1
EOF
}

sipp_start() {
  local binary="$ROOT/target/release/examples/perf_listener"
  test -x "$binary"
  if [[ -f "$STATE/sipp.pid" ]] && kill -0 "$(cat "$STATE/sipp.pid")" 2>/dev/null; then
    echo "managed SIPp target already running" >&2
    exit 1
  fi
  nohup "$binary" 35060 127.0.0.1 --perf-profile pbx-media-server \
    >"$STATE/sipp-listener.log" 2>&1 </dev/null &
  echo "$!" > "$STATE/sipp.pid"
  for _ in $(seq 1 100); do
    grep -q 'listening on' "$STATE/sipp-listener.log" 2>/dev/null && exit 0
    kill -0 "$(cat "$STATE/sipp.pid")" 2>/dev/null || exit 1
    sleep 0.1
  done
  exit 1
}

sipp_stop() {
  if [[ -f "$STATE/sipp.pid" ]]; then
    pid="$(cat "$STATE/sipp.pid")"
    kill -INT "$pid" 2>/dev/null || true
    for _ in $(seq 1 50); do
      kill -0 "$pid" 2>/dev/null || break
      sleep 0.1
    done
    kill -KILL "$pid" 2>/dev/null || true
    rm -f "$STATE/sipp.pid"
  fi
}

proxy_up() {
  # The compose-based proxy labs own their rendering, readiness gates
  # (registrar answering AND rtpengine enabled), and env-file writing.
  RVOIP_PBX_LOCAL_ENV_ROOT="$LOCAL_ENV_ROOT" sh "$PBX_SNAPSHOT/$1/up.sh"
}

proxy_down() {
  sh "$PBX_SNAPSHOT/$1/down.sh"
}

jambonz_up() {
  RVOIP_PBX_LOCAL_ENV_ROOT="$LOCAL_ENV_ROOT" \
    JAMBONZ_RECEIPT_DIR="${JAMBONZ_RECEIPT_DIR:-$STATE/jambonz/evidence}" \
    bash "$PBX_SNAPSHOT/jambonz/up.sh"
}

jambonz_down() {
  bash "$PBX_SNAPSHOT/jambonz/down.sh"
}

case "$ACTION" in
  asterisk-up) asterisk_up ;;
  asterisk-down|restore-asterisk-down) down rvoip-release-asterisk ;;
  freeswitch-up) freeswitch_up ;;
  freeswitch-down|restore-freeswitch-down) down rvoip-freeswitch ;;
  jambonz-up) jambonz_up ;;
  jambonz-down|restore-jambonz-down) jambonz_down ;;
  kamailio-up) proxy_up kamailio ;;
  kamailio-down|restore-kamailio-down) proxy_down kamailio ;;
  opensips-up) proxy_up opensips ;;
  opensips-down|restore-opensips-down) proxy_down opensips ;;
  sipp-start) sipp_start ;;
  sipp-stop) sipp_stop ;;
  *) echo "unknown interop lifecycle action: $ACTION" >&2; exit 2 ;;
esac
