#!/usr/bin/env bash
# Attended transfer demo: Alice calls Bob; Bob consults Charlie, then attended-
# transfers Alice to Charlie (REFER + Replaces). Exits 0 on success.
set -euo pipefail
cd "$(dirname "$0")"

GREEN='\033[0;32m'; RED='\033[0;31m'; CYAN='\033[0;36m'; NC='\033[0m'
export ALICE_PORT=5060 BOB_PORT=5061 CHARLIE_PORT=5062
PIDS=(); cleanup() { for p in "${PIDS[@]:-}"; do kill "$p" 2>/dev/null || true; done; }
trap cleanup EXIT

mkdir -p logs
echo -e "${GREEN}Building…${NC}"; cargo build --release --quiet

echo -e "${CYAN}[charlie]${NC} :$CHARLIE_PORT  ${CYAN}[bob]${NC} :$BOB_PORT"
./target/release/charlie > logs/charlie.log 2>&1 & PIDS+=($!)
./target/release/bob     > logs/bob.log     2>&1 & PIDS+=($!)
for _ in {1..20}; do lsof -iUDP:$BOB_PORT -n >/dev/null 2>&1 && lsof -iUDP:$CHARLIE_PORT -n >/dev/null 2>&1 && break; sleep 0.25; done

echo -e "${CYAN}[alice]${NC} :$ALICE_PORT calling Bob"
./target/release/alice > logs/alice.log 2>&1 & DRIVER=$!; PIDS+=($DRIVER)
wait "$DRIVER"; RC=$?
sleep 1

echo ""; for f in alice bob charlie; do sed "s/^/  /" logs/$f.log; done
if [ "$RC" -eq 0 ] && grep -q "attended transfer complete" logs/alice.log; then
  echo -e "\n${GREEN}✅ DEMO SUCCESSFUL — Alice was attended-transferred to Charlie${NC}"; exit 0
else
  echo -e "\n${RED}❌ DEMO FAILED (alice exit $RC) — see logs/${NC}"; exit 1
fi
