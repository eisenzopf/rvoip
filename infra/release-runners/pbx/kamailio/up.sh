#!/usr/bin/env sh
# Bring up the Kamailio+rtpengine interop lab.
#
# Renders the .in configs with the public host, starts the compose project,
# waits for both services, and writes the env file the rvoip pbx harness
# sources ($LOCAL_ENV_ROOT/kamailio/kamailio-local.env — ~/Developer/kamailio
# locally). On colima the public host is the VM's routable address, which
# requires `colima start --network-address`; without it this fails loudly
# rather than starting a lab whose SDP advertises an unroutable IP — the
# exact failure mode the FreeSWITCH lab's history documents.
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
SIP_PORT=${KAMAILIO_SIP_PORT:-5072}
TLS_PORT=${KAMAILIO_TLS_PORT:-5073}
NG_SOCK="udp:127.0.0.1:2223"
RTP_START=23000
RTP_END=23200

if [ -n "${RVOIP_PBX_LOCAL_ENV_ROOT:-}" ]; then
  LOCAL_ENV_ROOT=$RVOIP_PBX_LOCAL_ENV_ROOT
elif [ -n "${HOME:-}" ]; then
  LOCAL_ENV_ROOT=$HOME/Developer
else
  LOCAL_ENV_ROOT=/tmp/rvoip-local-env
fi

resolve_public_host() {
  if [ -n "${PBX_KAMAILIO_PUBLIC_HOST:-}" ]; then
    printf '%s\n' "$PBX_KAMAILIO_PUBLIC_HOST"
    return
  fi
  context=$(docker context show 2>/dev/null || echo default)
  if [ "$context" = "colima" ] || colima status >/dev/null 2>&1; then
    vm_ip=$(colima list 2>/dev/null | awk 'NR > 1 && $1 == "default" { print $8 }')
    if [ -z "$vm_ip" ] || [ "$vm_ip" = "-" ]; then
      echo "colima has no routable address; start it with: colima start --network-address" >&2
      exit 2
    fi
    printf '%s\n' "$vm_ip"
    return
  fi
  printf '127.0.0.1\n'
}

resolve_client_host() {
  # The address rvoip endpoints advertise back to the VM. On colima the VM
  # reaches the mac at the bridge's .1; on a native host it is loopback.
  case "$1" in
    127.0.0.1) printf '127.0.0.1\n' ;;
    *) printf '%s\n' "$(printf '%s' "$1" | awk -F. '{ printf "%s.%s.%s.1", $1, $2, $3 }')" ;;
  esac
}

# Transcoding is opt-in and changes what this lab proves: with flags the relay
# re-encodes, which exercises rtpengine's own AMR decoder against our stream;
# without them it forwards verbatim, which is what the AMR exit criterion
# needed. Never both in one run.
if [ -n "${PBX_PROXY_TRANSCODE:-}" ]; then
  # PCMU only, and deliberately so.
  #
  # codec-transcode-X means "offer X to the far side as well". Adding
  # codec-transcode-AMR-WB here therefore told rtpengine the PCMU callee also
  # spoke AMR-WB, so it picked a *passthrough* handler for the inbound
  # AMR-WB ("Sink supports codec AMR-WB/16000"), never built an AMR-WB ->
  # PCMU decoder, and the PCMU leg received nothing. Offering PCMU to the AMR
  # side is the whole requirement; the AMR side's own codec is already in its
  # offer.
  TRANSCODE_FLAGS=" codec-transcode-PCMU"
else
  TRANSCODE_FLAGS=""
fi

PUBLIC_HOST=$(resolve_public_host)
CLIENT_HOST=$(resolve_client_host "$PUBLIC_HOST")

RENDER_DIR="$SCRIPT_DIR/.rendered"
mkdir -p "$RENDER_DIR"
# A fresh self-signed certificate per run. Short-lived on purpose: it exists
# to exercise the TLS transport and the SRTP keys its SDP carries, and a lab
# key that outlives the lab is a key someone will eventually reuse.
mkdir -p "$RENDER_DIR/tls"
if [ ! -f "$RENDER_DIR/tls/cert.pem" ] || [ -n "${PBX_TLS_REGENERATE:-}" ]; then
  openssl req -x509 -newkey rsa:2048 -nodes \
    -keyout "$RENDER_DIR/tls/key.pem" \
    -out "$RENDER_DIR/tls/cert.pem" -days 2 \
    -subj "/CN=$PUBLIC_HOST" \
    -addext "subjectAltName=IP:$PUBLIC_HOST,DNS:localhost" >/dev/null 2>&1
fi

sed -e "s/__PUBLIC_HOST__/$PUBLIC_HOST/g" \
    -e "s/__SIP_PORT__/$SIP_PORT/g" \
    -e "s/__TLS_PORT__/$TLS_PORT/g" \
    -e "s#__TRANSCODE_FLAGS__#$TRANSCODE_FLAGS#g" \
    -e "s#__RTPENGINE_SOCK__#$NG_SOCK#g" \
    "$SCRIPT_DIR/kamailio.cfg.in" >"$RENDER_DIR/kamailio.cfg"
KAMAILIO_RENDERED_CFG="$RENDER_DIR/kamailio.cfg" \
KAMAILIO_TLS_DIR="$RENDER_DIR/tls" \
RTPENGINE_INTERFACE="$PUBLIC_HOST" \
docker compose -p rvoip-pbx-kamailio -f "$SCRIPT_DIR/docker-compose.yml" up -d

# Wait for kamailio to answer on its SIP port (the container is host-net, so
# probe through the VM/host address).
tries=0
while [ "$tries" -lt 30 ]; do
  if docker exec rvoip-kamailio kamctl ul show >/dev/null 2>&1 ||
     docker exec rvoip-kamailio kamcmd ul.dump >/dev/null 2>&1; then
    break
  fi
  tries=$((tries + 1))
  sleep 1
done
if [ "$tries" -ge 30 ]; then
  echo "kamailio did not become ready" >&2
  docker logs rvoip-kamailio 2>&1 | tail -20 >&2
  exit 1
fi

# Same relay-availability gate as the opensips lab: a disabled rtpengine
# node makes the cfg 503 every call, so the FIFO/RPC answering is not enough.
tries=0
while [ "$tries" -lt 30 ]; do
  if docker exec rvoip-kamailio kamcmd rtpengine.show all 2>/dev/null |
     grep -q 'disabled: 0'; then
    break
  fi
  tries=$((tries + 1))
  sleep 1
done
if [ "$tries" -ge 30 ]; then
  echo "kamailio never saw rtpengine become available" >&2
  docker logs rvoip-rtpengine-kamailio 2>&1 | tail -10 >&2
  exit 1
fi

mkdir -p "$LOCAL_ENV_ROOT/kamailio"
cat >"$LOCAL_ENV_ROOT/kamailio/kamailio-local.env" <<EOF
# Generated by $SCRIPT_DIR/up.sh
KAMAILIO_UDP_ADDR=$PUBLIC_HOST:$SIP_PORT
KAMAILIO_TLS_ADDR=$PUBLIC_HOST:$TLS_PORT
TLS_CA_PATH=$RENDER_DIR/tls/cert.pem
TLS_INSECURE=1
KAMAILIO_PASSWORD=password123
KAMAILIO_RTP_START=$RTP_START
KAMAILIO_RTP_END=$RTP_END
KAMAILIO_POST_REGISTER_SETTLE_SECS=1
RVOIP_LOCAL_IP=0.0.0.0
RVOIP_ADVERTISED_IP=$CLIENT_HOST
RVOIP_MEDIA_ADVERTISED_IP=$CLIENT_HOST
EOF

echo "Kamailio transcoding: ${PBX_PROXY_TRANSCODE:-off}"
echo "Kamailio lab up: sip $PUBLIC_HOST:$SIP_PORT, tls $PUBLIC_HOST:$TLS_PORT, rtpengine ng $NG_SOCK, rtp $RTP_START-$RTP_END"
echo "Env: $LOCAL_ENV_ROOT/kamailio/kamailio-local.env"
