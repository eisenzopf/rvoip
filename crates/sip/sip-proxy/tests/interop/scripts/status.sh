#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
# shellcheck source=common.sh
. "$SCRIPT_DIR/common.sh"

export PROXY_INTEROP_PEER_CONFIG=${PROXY_INTEROP_PEER_CONFIG:-$INTEROP_DIR/config/kamailio.cfg.in}
compose ps
for peer in kamailio opensips; do
  if [[ -n "$(compose ps --quiet "$peer")" ]]; then
    compose exec --no-TTY "$peer" sh -c "$(peer_version_command "$peer")"
  fi
done
