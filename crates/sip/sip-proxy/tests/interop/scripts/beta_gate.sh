#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

# This is the one release-policy entry point. Development smoke runs may call
# run.sh with a smaller matrix, but a beta candidate must request every peer,
# adjacency order, and release transport and must bind the result to one clean,
# unchanged source tree.
export PROXY_INTEROP_PEERS="kamailio opensips"
export PROXY_INTEROP_ORDERS="rvoip-first peer-first"
export PROXY_INTEROP_TRANSPORTS="udp tcp tls"
export PROXY_INTEROP_RETENTION_DRAIN_SECONDS=130
export PROXY_INTEROP_FAIL_FAST=1
export PROXY_INTEROP_REQUIRE_CLEAN_SOURCE=1
export PROXY_INTEROP_REQUIRE_UNCHANGED_SOURCE=1
export PROXY_INTEROP_REQUIRE_PREEXISTING_STATE=1

python3 -B "$SCRIPT_DIR/test_verify_tls_evidence.py"
python3 -B "$SCRIPT_DIR/test_opensips_tls_provenance.py"
python3 -B "$SCRIPT_DIR/test_tls_boundary.py"

exec "$SCRIPT_DIR/run.sh"
