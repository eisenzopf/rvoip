#!/usr/bin/env sh
# Compatibility wrapper. The unified PBX suite is maintained in ../pbx.
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec "$SCRIPT_DIR/../pbx/run.sh" --pbx freeswitch --api callback "$@"
