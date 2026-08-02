#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

# This is the release-policy entry point. With no arguments it retains the
# compatibility behavior and runs the complete matrix. The remote release
# catalog passes one reviewed peer/order/transport tuple per gate so the same
# complete matrix can run in parallel and produce independently reusable
# evidence.
if (( $# == 0 )); then
  peers="kamailio opensips"
  orders="rvoip-first peer-first"
  transports="udp tcp tls"
elif (( $# == 3 )); then
  case "$1" in
    kamailio|opensips) peers="$1" ;;
    *) echo "unsupported release peer: $1" >&2; exit 2 ;;
  esac
  case "$2" in
    rvoip-first|peer-first) orders="$2" ;;
    *) echo "unsupported release adjacency order: $2" >&2; exit 2 ;;
  esac
  case "$3" in
    udp|tcp|tls) transports="$3" ;;
    *) echo "unsupported release transport: $3" >&2; exit 2 ;;
  esac
else
  echo "usage: $0 [kamailio|opensips rvoip-first|peer-first udp|tcp|tls]" >&2
  exit 2
fi

export PROXY_INTEROP_PEERS="$peers"
export PROXY_INTEROP_ORDERS="$orders"
export PROXY_INTEROP_TRANSPORTS="$transports"
export PROXY_INTEROP_RETENTION_DRAIN_SECONDS=130
export PROXY_INTEROP_FAIL_FAST=1
export PROXY_INTEROP_REQUIRE_CLEAN_SOURCE=1
export PROXY_INTEROP_REQUIRE_UNCHANGED_SOURCE=1
export PROXY_INTEROP_REQUIRE_PREEXISTING_STATE=1

python3 -B "$SCRIPT_DIR/test_verify_tls_evidence.py"
python3 -B "$SCRIPT_DIR/test_opensips_tls_provenance.py"
python3 -B "$SCRIPT_DIR/test_tls_boundary.py"

exec "$SCRIPT_DIR/run.sh"
