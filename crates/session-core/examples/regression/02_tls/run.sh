#!/usr/bin/env bash
# TLS-call example: generates a one-off CA + server cert chain, points
# Alice and Bob at it, and places a sips: call. Exercises Sprint 1 A1:
# the MultiplexedTransport routes the INVITE through the TLS listener
# (PT 5061) instead of UDP.
#
# Two passes (both must succeed):
#
#  1. **Insecure mode** (`TLS_INSECURE=1`) — client sets
#     `tls_insecure_skip_verify=true`, server cert is accepted without
#     validation. Sprint 2.5 P6's dev-only escape hatch.
#
#  2. **Secure mode** (`TLS_INSECURE=0`) — client validates the
#     server cert against the locally-generated CA via
#     `tls_extra_ca_path`. The cert's SAN is `127.0.0.1` so hostname
#     verification passes. This is the production code path: real
#     carrier deployments do exactly this with a public CA instead of
#     our local one.
#
# Cloud-carrier deployments should use real certs and leave
# `tls_insecure_skip_verify` false (or, even better, build without the
# `dev-insecure-tls` Cargo feature so the field doesn't compile).
set -euo pipefail
cd "$(dirname "$0")/../../.."   # crate root

GREEN='\033[0;32m'; CYAN='\033[0;36m'; RED='\033[0;31m'; YELLOW='\033[1;33m'; NC='\033[0m'

CERT_DIR="$(mktemp -d -t rvoip_tls_example.XXXXXX)"
export TLS_CERT_PATH="$CERT_DIR/server.pem"
export TLS_KEY_PATH="$CERT_DIR/server-key.pem"
export TLS_CA_PATH="$CERT_DIR/ca.pem"
SERVER_LOG="$(mktemp -t tls_server.XXXXXX)"

cleanup() {
  pkill -P $$ 2>/dev/null || true
  wait 2>/dev/null || true
  rm -rf "$CERT_DIR"
  rm -f "$SERVER_LOG"
}
trap cleanup EXIT

if ! command -v openssl >/dev/null; then
  echo -e "${RED}openssl not found — required to generate the dev cert chain${NC}" >&2
  exit 1
fi

echo -e "${GREEN}Generating CA + server cert (SAN=127.0.0.1)…${NC}"
# 1. CA key + self-signed CA cert.
openssl genrsa -out "$CERT_DIR/ca-key.pem" 2048 >/dev/null 2>&1
openssl req -x509 -new -nodes -key "$CERT_DIR/ca-key.pem" \
  -days 1 -subj "/CN=rvoip-test-ca" \
  -out "$TLS_CA_PATH" >/dev/null 2>&1

# 2. Server key + CSR with SAN=127.0.0.1 (so hostname verification
#    against the IP literal succeeds).
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
openssl req -new -key "$TLS_KEY_PATH" \
  -out "$CERT_DIR/server.csr" \
  -config "$CERT_DIR/server.cnf" >/dev/null 2>&1

# 3. Sign the server cert with the CA.
openssl x509 -req -in "$CERT_DIR/server.csr" \
  -CA "$TLS_CA_PATH" -CAkey "$CERT_DIR/ca-key.pem" -CAcreateserial \
  -out "$TLS_CERT_PATH" -days 1 \
  -extfile "$CERT_DIR/server.cnf" -extensions v3_req >/dev/null 2>&1

echo -e "${GREEN}Building…${NC}"
cargo build -p rvoip-session-core --features dev-insecure-tls \
  --example streampeer_tls_server \
  --example streampeer_tls_client 2>&1 \
  | grep -v '^warning:' | grep -v '^\s' | grep -v '^$' || true

# Helper: run one pass (insecure or secure). Aborts the script if
# either the server doesn't observe the call or the client fails.
run_pass() {
  local mode_label="$1"
  local mode_color="$2"
  local insecure="$3"
  local server_log="$4"
  : > "$server_log"

  echo ""
  echo -e "${mode_color}══════════════════════════════════════════════════════════════${NC}"
  echo -e "${mode_color}▶ TLS pass: ${mode_label}${NC}"
  echo -e "${mode_color}══════════════════════════════════════════════════════════════${NC}"

  echo -e "${GREEN}[SERVER]${NC} Starting TLS server on 5060 (+ sips:5061)"
  cargo run -p rvoip-session-core --features dev-insecure-tls \
    --example streampeer_tls_server --quiet > "$server_log" 2>&1 &
  local server_pid=$!
  sleep 2

  echo -e "${CYAN}[CLIENT]${NC} Starting TLS client (TLS_INSECURE=$insecure)"
  TLS_INSECURE="$insecure" cargo run -p rvoip-session-core \
    --features dev-insecure-tls --example streampeer_tls_client --quiet \
    2>&1 | sed "s/^/$(printf '\033[0;36m')[CLIENT]$(printf '\033[0m') /"
  local client_exit=${PIPESTATUS[0]}

  sleep 1
  kill -INT "$server_pid" 2>/dev/null || true
  wait "$server_pid" 2>/dev/null || true

  echo ""
  sed "s/^/$(printf '\033[0;32m')[SERVER]$(printf '\033[0m') /" "$server_log"

  if ! grep -q "Incoming TLS call" "$server_log"; then
    echo -e "\n${RED}=== ${mode_label}: server did not observe a TLS-transported INVITE ===${NC}"
    return 1
  fi

  if [ $client_exit -ne 0 ]; then
    echo -e "\n${RED}=== ${mode_label}: client failed (exit $client_exit) ===${NC}"
    return 1
  fi

  echo -e "\n${GREEN}=== ${mode_label}: sips: call established over TLS ===${NC}"
  return 0
}

# Pass 1: insecure-skip-verify mode (Sprint 2.5 P6 dev escape hatch).
run_pass "insecure mode (tls_insecure_skip_verify=true)" "$YELLOW" 1 "$SERVER_LOG"

# Pass 2: secure mode — client validates server cert against the CA.
run_pass "secure mode (CA validation via tls_extra_ca_path)" "$GREEN" 0 "$SERVER_LOG"

echo ""
echo -e "${GREEN}══════════════════════════════════════════════════════════════${NC}"
echo -e "${GREEN}=== Both passes complete — TLS works with and without verify ===${NC}"
echo -e "${GREEN}══════════════════════════════════════════════════════════════${NC}"
