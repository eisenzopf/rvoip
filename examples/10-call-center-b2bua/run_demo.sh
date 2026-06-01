#!/usr/bin/env bash
# Mini call-center demo: two agents register, the B2BUA bridges a customer to an
# agent. Exits 0 on success.
set -euo pipefail
cd "$(dirname "$0")"

GREEN='\033[0;32m'; RED='\033[0;31m'; CYAN='\033[0;36m'; NC='\033[0m'
PIDS=(); cleanup() { for p in "${PIDS[@]:-}"; do kill "$p" 2>/dev/null || true; done; }
trap cleanup EXIT

mkdir -p logs
echo -e "${GREEN}Building…${NC}"; cargo build --release --quiet

echo -e "${CYAN}[agents]${NC} alice:5071  bob:5072"
./target/release/agent --port 5071 --name alice > logs/agent-alice.log 2>&1 & PIDS+=($!)
./target/release/agent --port 5072 --name bob   > logs/agent-bob.log   2>&1 & PIDS+=($!)
for _ in {1..20}; do lsof -iUDP:5071 -n >/dev/null 2>&1 && lsof -iUDP:5072 -n >/dev/null 2>&1 && break; sleep 0.25; done

echo -e "${CYAN}[call-center]${NC} support line on :5070"
./target/release/server --bind 127.0.0.1:5070 > logs/server.log 2>&1 & PIDS+=($!)
for _ in {1..20}; do lsof -iUDP:5070 -n >/dev/null 2>&1 && break; sleep 0.25; done

echo -e "${CYAN}[customer]${NC} calling support"
./target/release/customer --port 5080 --talk-secs 2 > logs/customer.log 2>&1 & DRIVER=$!; PIDS+=($DRIVER)
wait "$DRIVER"; RC=$?
sleep 1

echo ""; for f in customer server agent-alice agent-bob; do sed "s/^/  /" logs/$f.log; done
if [ "$RC" -eq 0 ] && grep -q "connected to an agent" logs/customer.log && grep -q "bridged" logs/server.log; then
  echo -e "\n${GREEN}✅ DEMO SUCCESSFUL — customer bridged to an agent via the B2BUA${NC}"; exit 0
else
  echo -e "\n${RED}❌ DEMO FAILED (customer exit $RC) — see logs/${NC}"; exit 1
fi
