#!/usr/bin/env bash
# IVR demo: a reactive CallbackPeer server answers an inbound call and reacts to
# DTMF from a scripted caller. Exits 0 on success.
set -euo pipefail
cd "$(dirname "$0")"

GREEN='\033[0;32m'; RED='\033[0;31m'; CYAN='\033[0;36m'; NC='\033[0m'
PIDS=(); cleanup() { for p in "${PIDS[@]:-}"; do kill "$p" 2>/dev/null || true; done; }
trap cleanup EXIT

mkdir -p logs
echo -e "${GREEN}Building…${NC}"; cargo build --release --quiet

echo -e "${CYAN}[ivr]${NC} starting on :5120"
./target/release/server > logs/ivr.log 2>&1 & PIDS+=($!)
for _ in {1..20}; do lsof -iUDP:5120 -n >/dev/null 2>&1 && break; sleep 0.25; done

echo -e "${CYAN}[caller]${NC} calling the IVR and pressing 1 2 #"
./target/release/caller > logs/caller.log 2>&1 & DRIVER=$!; PIDS+=($DRIVER)
wait "$DRIVER"; RC=$?
sleep 1

echo ""; sed 's/^/  /' logs/caller.log; sed 's/^/  /' logs/ivr.log
if [ "$RC" -eq 0 ] && grep -q "connected to IVR" logs/caller.log && grep -q "pressed" logs/ivr.log; then
  echo -e "\n${GREEN}✅ DEMO SUCCESSFUL — IVR reacted to inbound call + DTMF${NC}"; exit 0
else
  echo -e "\n${RED}❌ DEMO FAILED (caller exit $RC) — see logs/${NC}"; exit 1
fi
