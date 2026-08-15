#!/usr/bin/env sh
set -eu
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
KAMAILIO_RENDERED_CFG="$SCRIPT_DIR/.rendered/kamailio.cfg" \
KAMAILIO_TLS_DIR="$SCRIPT_DIR/.rendered/tls" \
RTPENGINE_INTERFACE=unused \
docker compose -p rvoip-pbx-kamailio -f "$SCRIPT_DIR/docker-compose.yml" down
