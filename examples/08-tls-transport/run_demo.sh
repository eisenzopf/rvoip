#!/usr/bin/env bash
# TLS-call demo: generate a one-off CA + server cert (SAN=127.0.0.1), point the
# server and client at it, and place a sips: call. The INVITE routes through the
# TLS listener (port 5061) instead of UDP. Runs two passes, both must succeed:
#
#   1. insecure mode (TLS_INSECURE=1) — client skips server-cert validation.
#   2. secure mode   (TLS_INSECURE=0) — client validates the cert against the CA.
#
# Production deployments use real certs and omit the `dev-insecure-tls` feature.
set -euo pipefail
cd "$(dirname "$0")"

GREEN='\033[0;32m'; CYAN='\033[0;36m'; RED='\033[0;31m'; YELLOW='\033[1;33m'; NC='\033[0m'

if ! command -v openssl >/dev/null; then
  echo -e "${RED}openssl not found — required to generate the dev cert chain${NC}" >&2
  exit 1
fi

CERT_DIR="$(mktemp -d -t rvoip_tls_example.XXXXXX)"
export TLS_CERT_PATH="$CERT_DIR/server.pem"
export TLS_KEY_PATH="$CERT_DIR/server-key.pem"
export TLS_CA_PATH="$CERT_DIR/ca.pem"
SERVER_LOG="$(mktemp -t tls_server.XXXXXX)"

cleanup() { pkill -P $$ 2>/dev/null || true; wait 2>/dev/null || true; rm -rf "$CERT_DIR"; rm -f "$SERVER_LOG"; }
trap cleanup EXIT

echo -e "${GREEN}Generating CA + server cert (SAN=127.0.0.1)…${NC}"
openssl genrsa -out "$CERT_DIR/ca-key.pem" 2048 >/dev/null 2>&1
openssl req -x509 -new -nodes -key "$CERT_DIR/ca-key.pem" -days 1 \
  -subj "/CN=rvoip-test-ca" -out "$TLS_CA_PATH" >/dev/null 2>&1
openssl genrsa -out "$TLS_KEY_PATH" 2048 >/dev/null 2>&1
cat > "$CERT_DIR/server.cnf" <<EOF
[ req ]
distinguished_name = req_distinguished_name
req_extensions     = v3_req
prompt             = no
[ req_distinguished_name ]
CN = 127.0.0.1
[ v3_req ]
subjectAltName = @alt_names
[ alt_names ]
IP.1 = 127.0.0.1
DNS.1 = localhost
EOF
openssl req -new -key "$TLS_KEY_PATH" -out "$CERT_DIR/server.csr" -config "$CERT_DIR/server.cnf" >/dev/null 2>&1
openssl x509 -req -in "$CERT_DIR/server.csr" -CA "$TLS_CA_PATH" -CAkey "$CERT_DIR/ca-key.pem" \
  -CAcreateserial -out "$TLS_CERT_PATH" -days 1 \
  -extfile "$CERT_DIR/server.cnf" -extensions v3_req >/dev/null 2>&1

echo -e "${GREEN}Building…${NC}"
cargo build --release --quiet

run_pass() {
  local label="$1" color="$2" insecure="$3"
  : > "$SERVER_LOG"
  echo ""
  echo -e "${color}▶ TLS pass: ${label}${NC}"
  ./target/release/server > "$SERVER_LOG" 2>&1 &
  local server_pid=$!
  sleep 2
  TLS_INSECURE="$insecure" ./target/release/client 2>&1 | sed "s/^/  [client] /"
  local client_exit=${PIPESTATUS[0]}
  sleep 1
  kill -INT "$server_pid" 2>/dev/null || true
  wait "$server_pid" 2>/dev/null || true
  sed "s/^/  [server] /" "$SERVER_LOG"
  if ! grep -q "Incoming TLS call" "$SERVER_LOG"; then
    echo -e "${RED}=== ${label}: server did not observe a TLS-transported INVITE ===${NC}"; return 1
  fi
  if [ "$client_exit" -ne 0 ]; then
    echo -e "${RED}=== ${label}: client failed (exit $client_exit) ===${NC}"; return 1
  fi
  echo -e "${GREEN}=== ${label}: sips: call established over TLS ===${NC}"
}

run_pass "insecure mode (skip cert validation)" "$YELLOW" 1
run_pass "secure mode (CA validation)" "$GREEN" 0

echo ""
echo -e "${GREEN}✅ Both passes complete — TLS works with and without verify${NC}"
