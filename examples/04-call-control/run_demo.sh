#!/usr/bin/env bash
# Call-control demo: connect, then hold / resume / DTMF. Exits 0 on success.
set -euo pipefail
cd "$(dirname "$0")"

GREEN='\033[0;32m'; RED='\033[0;31m'; CYAN='\033[0;36m'; NC='\033[0m'
PIDS=(); cleanup() { for p in "${PIDS[@]:-}"; do kill "$p" 2>/dev/null || true; done; }
trap cleanup EXIT

mkdir -p logs
echo -e "${GREEN}Building…${NC}"; cargo build --release --quiet

echo -e "${CYAN}[peer]${NC} starting on :5061"
./target/release/peer --port 5061 > logs/peer.log 2>&1 & PIDS+=($!)
for _ in {1..20}; do lsof -iUDP:5061 -n >/dev/null 2>&1 && break; sleep 0.25; done

echo -e "${CYAN}[controller]${NC} connecting and driving hold/resume/DTMF"
./target/release/controller --port 5060 --peer-port 5061 > logs/controller.log 2>&1 & DRIVER=$!; PIDS+=($DRIVER)
wait "$DRIVER"; RC=$?
sleep 1

echo ""; sed 's/^/  /' logs/controller.log; sed 's/^/  /' logs/peer.log
if [ "$RC" -eq 0 ] && grep -q "received 3 DTMF" logs/peer.log; then
  echo -e "\n${GREEN}✅ DEMO SUCCESSFUL — hold, resume, and DTMF exercised${NC}"; exit 0
else
  echo -e "\n${RED}❌ DEMO FAILED (controller exit $RC) — see logs/${NC}"; exit 1
fi
