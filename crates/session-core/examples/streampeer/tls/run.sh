#!/usr/bin/env bash
# TLS-call example: generates a one-off self-signed cert, points Alice
# and Bob at it, and places a sips: call. Exercises Sprint 1 A1: the
# MultiplexedTransport routes the INVITE through the TLS listener (PT
# 5061) instead of UDP.
#
# The cert is dev-only: `tls_insecure_skip_verify = true` on both sides
# accepts the otherwise-untrusted CN=localhost cert. Cloud-carrier
# deployments should use real certs and leave that knob false.
set -euo pipefail
cd "$(dirname "$0")/../../.."   # crate root

GREEN='\033[0;32m'; CYAN='\033[0;36m'; RED='\033[0;31m'; NC='\033[0m'

CERT_DIR="$(mktemp -d -t rvoip_tls_example.XXXXXX)"
export TLS_CERT_PATH="$CERT_DIR/cert.pem"
export TLS_KEY_PATH="$CERT_DIR/key.pem"
SERVER_LOG="$(mktemp -t tls_server.XXXXXX)"

cleanup() {
  pkill -P $$ 2>/dev/null || true
  wait 2>/dev/null || true
  rm -rf "$CERT_DIR"
  rm -f "$SERVER_LOG"
}
trap cleanup EXIT

if ! command -v openssl >/dev/null; then
  echo -e "${RED}openssl not found — required to generate the self-signed dev cert${NC}" >&2
  exit 1
fi

echo -e "${GREEN}Generating self-signed cert…${NC}"
openssl req -x509 -newkey rsa:2048 -nodes \
  -keyout "$TLS_KEY_PATH" -out "$TLS_CERT_PATH" \
  -days 1 -subj "/CN=localhost" \
  >/dev/null 2>&1

echo -e "${GREEN}Building…${NC}"
cargo build -p rvoip-session-core \
  --example streampeer_tls_server \
  --example streampeer_tls_client 2>&1 \
  | grep -v '^warning:' | grep -v '^\s' | grep -v '^$' || true

echo -e "${GREEN}[SERVER]${NC} Starting TLS server on 5060 (+ sips:5061)"
cargo run -p rvoip-session-core --example streampeer_tls_server --quiet > "$SERVER_LOG" 2>&1 &
SERVER_PID=$!
sleep 2

echo -e "${CYAN}[CLIENT]${NC} Starting TLS client"
cargo run -p rvoip-session-core --example streampeer_tls_client --quiet \
  2>&1 | sed "s/^/$(printf '\033[0;36m')[CLIENT]$(printf '\033[0m') /"
CLIENT_EXIT=$?

sleep 1
kill -INT $SERVER_PID 2>/dev/null || true
wait $SERVER_PID 2>/dev/null || true

echo ""
sed "s/^/$(printf '\033[0;32m')[SERVER]$(printf '\033[0m') /" "$SERVER_LOG"

# Proof the server actually observed the incoming call over TLS — if
# the multiplexer mis-routed to UDP the INVITE would never reach the
# TLS listener and the server log would be empty of call events.
if ! grep -q "Incoming TLS call" "$SERVER_LOG"; then
  echo -e "\n${RED}=== Server did not observe a TLS-transported INVITE ===${NC}"
  exit 1
fi

if [ $CLIENT_EXIT -eq 0 ]; then
  echo -e "\n${GREEN}=== Example complete — sips: call established over TLS ===${NC}"
else
  echo -e "\n${RED}=== Client failed (exit $CLIENT_EXIT) ===${NC}"
  exit 1
fi
