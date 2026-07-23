#!/usr/bin/env bash
# DTLS-SRTP example: Bob and Alice negotiate RFC 5763/5764 DTLS-SRTP.
set -euo pipefail
cd "$(dirname "$0")/../../.."   # crate root

GREEN='\033[0;32m'; CYAN='\033[0;36m'; RED='\033[0;31m'; NC='\033[0m'
cleanup() { pkill -P $$ 2>/dev/null || true; wait 2>/dev/null || true; }
trap cleanup EXIT

echo -e "${GREEN}Building...${NC}"
cargo build -p rvoip-sip --features dtls-srtp \
  --example stream_peer_dtls_srtp_bob \
  --example stream_peer_dtls_srtp_alice 2>&1 | grep -v '^warning:' | grep -v '^\s' | grep -v '^$' || true

echo -e "${GREEN}[BOB]${NC} Starting"
cargo run -p rvoip-sip --features dtls-srtp --example stream_peer_dtls_srtp_bob --quiet \
  2>&1 | sed "s/^/$(printf '\033[0;32m')[BOB]$(printf '\033[0m') /" &
sleep 2

echo -e "${CYAN}[ALICE]${NC} Starting"
cargo run -p rvoip-sip --features dtls-srtp --example stream_peer_dtls_srtp_alice --quiet \
  2>&1 | sed "s/^/$(printf '\033[0;36m')[ALICE]$(printf '\033[0m') /"
CLIENT_EXIT=$?

sleep 1
if [ $CLIENT_EXIT -eq 0 ]; then
  echo -e "\n${GREEN}=== Example complete ===${NC}"
else
  echo -e "\n${RED}=== Client failed (exit $CLIENT_EXIT) ===${NC}"
  exit 1
fi
