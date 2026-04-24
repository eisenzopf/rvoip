#!/usr/bin/env bash
# SRTP-call example: both sides set offer_srtp = srtp_required = true,
# so the negotiation must complete with `a=crypto:` present on the
# answer and paired SrtpContexts installed on both UDP transports.
# Exercises the Sprint 1 B2 path end-to-end.
#
# The RTP bytes on the wire are AES-CM-128 encrypted after setup; this
# script doesn't capture wire bytes (that's locked in at the transport
# layer by `srtp_round_trip_through_real_udp_sockets` in rtp-core), it
# only asserts the call establishes.
set -euo pipefail
cd "$(dirname "$0")/../../.."   # crate root

GREEN='\033[0;32m'; CYAN='\033[0;36m'; RED='\033[0;31m'; NC='\033[0m'
SERVER_LOG="$(mktemp -t srtp_server.XXXXXX)"
cleanup() {
  pkill -P $$ 2>/dev/null || true
  wait 2>/dev/null || true
  rm -f "$SERVER_LOG"
}
trap cleanup EXIT

echo -e "${GREEN}Building…${NC}"
cargo build -p rvoip-session-core \
  --example streampeer_srtp_server \
  --example streampeer_srtp_client 2>&1 \
  | grep -v '^warning:' | grep -v '^\s' | grep -v '^$' || true

echo -e "${GREEN}[SERVER]${NC} Starting SRTP-mandatory server on 5060"
cargo run -p rvoip-session-core --example streampeer_srtp_server --quiet > "$SERVER_LOG" 2>&1 &
SERVER_PID=$!
sleep 2

echo -e "${CYAN}[CLIENT]${NC} Starting SRTP-mandatory client"
cargo run -p rvoip-session-core --example streampeer_srtp_client --quiet \
  2>&1 | sed "s/^/$(printf '\033[0;36m')[CLIENT]$(printf '\033[0m') /"
CLIENT_EXIT=$?

sleep 1
kill -INT $SERVER_PID 2>/dev/null || true
wait $SERVER_PID 2>/dev/null || true

echo ""
sed "s/^/$(printf '\033[0;32m')[SERVER]$(printf '\033[0m') /" "$SERVER_LOG"

# Regression check: the server must log the incoming SRTP call. If the
# negotiation failed (peer rejected `a=crypto:` / one side dropped
# SDES), the call setup would never reach the server.
if ! grep -q "Incoming SRTP-required call" "$SERVER_LOG"; then
  echo -e "\n${RED}=== Server did not observe a SRTP-negotiated call ===${NC}"
  exit 1
fi

if [ $CLIENT_EXIT -eq 0 ]; then
  echo -e "\n${GREEN}=== Example complete — RFC 4568 SDES-SRTP call established ===${NC}"
else
  echo -e "\n${RED}=== Client failed (exit $CLIENT_EXIT) ===${NC}"
  exit 1
fi
