#!/usr/bin/env sh
# Extended multi-endpoint PBX scenarios.
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

"$SCRIPT_DIR/run.sh" --scenario ring_cancel "$@"
"$SCRIPT_DIR/run.sh" --scenario dtmf "$@"
"$SCRIPT_DIR/run.sh" --scenario reject "$@"
"$SCRIPT_DIR/run.sh" --scenario blind_transfer "$@"
