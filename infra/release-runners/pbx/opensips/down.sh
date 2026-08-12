#!/usr/bin/env sh
set -eu
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
OPENSIPS_RENDERED_CFG="$SCRIPT_DIR/.rendered/opensips.cfg" \
RTPENGINE_INTERFACE=unused \
docker compose -p rvoip-pbx-opensips -f "$SCRIPT_DIR/docker-compose.yml" down
