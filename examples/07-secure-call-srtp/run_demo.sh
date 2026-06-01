#!/usr/bin/env bash
# SRTP demo: client places an SRTP-mandatory call to the server; media is
# AES-CM-128/HMAC-SHA1-80 encrypted. Exits 0 on success.
set -euo pipefail
cd "$(dirname "$0")"

GREEN='\033[0;32m'; RED='\033[0;31m'; CYAN='\033[0;36m'; NC='\033[0m'
PIDS=(); cleanup() { for p in "${PIDS[@]:-}"; do kill "$p" 2>/dev/null || true; done; }
trap cleanup EXIT

mkdir -p logs
echo -e "${GREEN}Building…${NC}"; cargo build --release --quiet

echo -e "${CYAN}[server]${NC} starting on :5060 (SRTP mandatory)"
./target/release/server > logs/server.log 2>&1 & PIDS+=($!)
for _ in {1..20}; do lsof -iUDP:5060 -n >/dev/null 2>&1 && break; sleep 0.25; done

echo -e "${CYAN}[client]${NC} placing SRTP call"
./target/release/client > logs/client.log 2>&1 & DRIVER=$!; PIDS+=($DRIVER)
wait "$DRIVER"; RC=$?
sleep 1

echo ""; sed 's/^/  /' logs/client.log; sed 's/^/  /' logs/server.log
if [ "$RC" -eq 0 ] && grep -q "media is encrypted" logs/client.log && grep -q "established with SRTP" logs/server.log; then
  echo -e "\n${GREEN}✅ DEMO SUCCESSFUL — encrypted SRTP call established${NC}"; exit 0
else
  echo -e "\n${RED}❌ DEMO FAILED (client exit $RC) — see logs/${NC}"; exit 1
fi
