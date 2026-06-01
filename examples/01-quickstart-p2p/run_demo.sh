#!/usr/bin/env bash
# Quickstart P2P demo: boot the callee, then have the caller dial it over
# loopback, exchange ~1s of media, and hang up. Exits 0 on success.
set -euo pipefail
cd "$(dirname "$0")"

GREEN='\033[0;32m'; RED='\033[0;31m'; CYAN='\033[0;36m'; NC='\033[0m'
CALLER_PORT=5060
CALLEE_PORT=5061

PIDS=()
cleanup() { for pid in "${PIDS[@]:-}"; do kill "$pid" 2>/dev/null || true; done; }
trap cleanup EXIT

mkdir -p logs
echo -e "${GREEN}Building…${NC}"
cargo build --release --quiet

echo -e "${CYAN}[callee]${NC} starting on :$CALLEE_PORT"
./target/release/callee --port "$CALLEE_PORT" > logs/callee.log 2>&1 &
PIDS+=($!)

# Wait until the callee's SIP/UDP port is listening.
for _ in {1..20}; do
  lsof -iUDP:"$CALLEE_PORT" -n >/dev/null 2>&1 && break
  sleep 0.25
done

echo -e "${CYAN}[caller]${NC} dialing callee on :$CALLEE_PORT"
./target/release/caller --port "$CALLER_PORT" --peer-port "$CALLEE_PORT" > logs/caller.log 2>&1 &
CALLER_PID=$!; PIDS+=($CALLER_PID)
wait "$CALLER_PID"; RC=$?

echo ""
sed 's/^/  /' logs/caller.log
sed 's/^/  /' logs/callee.log

if [ "$RC" -eq 0 ] && grep -q "call completed" logs/caller.log; then
  echo -e "\n${GREEN}✅ DEMO SUCCESSFUL — P2P call established and torn down cleanly${NC}"
  exit 0
else
  echo -e "\n${RED}❌ DEMO FAILED (caller exit $RC) — see logs/${NC}"
  exit 1
fi
