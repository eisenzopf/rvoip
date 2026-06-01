#!/usr/bin/env bash
# Softphone audio demo: callee answers, both sides exchange PCMU tones and
# verify what they received (no audio hardware needed). Exits 0 on success.
set -euo pipefail
cd "$(dirname "$0")"

GREEN='\033[0;32m'; RED='\033[0;31m'; CYAN='\033[0;36m'; NC='\033[0m'
PIDS=(); cleanup() { for p in "${PIDS[@]:-}"; do kill "$p" 2>/dev/null || true; done; }
trap cleanup EXIT

mkdir -p logs
echo -e "${GREEN}Building…${NC}"; cargo build --release --quiet

echo -e "${CYAN}[callee]${NC} starting on :5073"
./target/release/callee --port 5073 > logs/callee.log 2>&1 & PIDS+=($!)
for _ in {1..20}; do lsof -iUDP:5073 -n >/dev/null 2>&1 && break; sleep 0.25; done

echo -e "${CYAN}[caller]${NC} dialing :5073"
./target/release/caller --port 5072 --peer-port 5073 > logs/caller.log 2>&1 & DRIVER=$!; PIDS+=($DRIVER)
wait "$DRIVER"; RC=$?
sleep 1

echo ""; sed 's/^/  /' logs/caller.log; sed 's/^/  /' logs/callee.log
if [ "$RC" -eq 0 ] && grep -q "caller received" logs/caller.log && grep -q "callee received" logs/callee.log; then
  echo -e "\n${GREEN}✅ DEMO SUCCESSFUL — bidirectional PCMU media verified${NC}"; exit 0
else
  echo -e "\n${RED}❌ DEMO FAILED (caller exit $RC) — see logs/${NC}"; exit 1
fi
