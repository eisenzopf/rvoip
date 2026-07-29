#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
# shellcheck source=common.sh
. "$SCRIPT_DIR/common.sh"

# Compose interpolates the volume even while tearing down. A caller normally
# retains the real rendered path; this fallback is only used by manual cleanup.
export PROXY_INTEROP_PEER_CONFIG=${PROXY_INTEROP_PEER_CONFIG:-$INTEROP_DIR/config/kamailio.cfg.in}
compose down --remove-orphans --volumes
