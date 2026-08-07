#!/usr/bin/env bash
# Attended transfer demo: Alice calls Bob; Bob consults Charlie, then attended-
# transfers Alice to Charlie (REFER + Replaces). Exits 0 on success.
#
# Ports are deliberately outside the 5060/5061 range used by the combined
# examples-smoke suite (quickstart-p2p, secure-call-srtp) so a slow cleanup
# from a prior demo cannot leave Alice dialing a stale peer.
set -euo pipefail
cd "$(dirname "$0")"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-../target}"

GREEN='\033[0;32m'; RED='\033[0;31m'; CYAN='\033[0;36m'; NC='\033[0m'
export ALICE_PORT=5080 BOB_PORT=5081 CHARLIE_PORT=5082
PIDS=()

cleanup() {
  for p in "${PIDS[@]:-}"; do kill "$p" 2>/dev/null || true; done
  sleep 0.1
  for p in "${PIDS[@]:-}"; do kill -9 "$p" 2>/dev/null || true; done
  wait 2>/dev/null || true
}

dump_logs() {
  echo ""
  for f in alice bob charlie; do
    if [ -f "logs/$f.log" ]; then
      echo "--- $f.log ---"
      sed "s/^/  /" "logs/$f.log" || true
    fi
  done
}

finish() {
  local rc=$?
  dump_logs
  cleanup
  exit "$rc"
}
trap finish EXIT

mkdir -p logs
rm -f logs/alice.log logs/bob.log logs/charlie.log
echo -e "${GREEN}Building…${NC}"; cargo build --release --quiet

echo -e "${CYAN}[charlie]${NC} :$CHARLIE_PORT  ${CYAN}[bob]${NC} :$BOB_PORT"
"$CARGO_TARGET_DIR/release/charlie" > logs/charlie.log 2>&1 & PIDS+=($!)
"$CARGO_TARGET_DIR/release/bob"     > logs/bob.log     2>&1 & PIDS+=($!)

ready=0
for _ in {1..40}; do
  if lsof -iUDP:"$BOB_PORT" -n >/dev/null 2>&1 \
    && lsof -iUDP:"$CHARLIE_PORT" -n >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 0.25
done
if [ "$ready" -ne 1 ]; then
  echo -e "${RED}❌ Bob/Charlie failed to bind UDP :$BOB_PORT / :$CHARLIE_PORT${NC}"
  exit 1
fi

echo -e "${CYAN}[alice]${NC} :$ALICE_PORT calling Bob"
"$CARGO_TARGET_DIR/release/alice" > logs/alice.log 2>&1 & DRIVER=$!; PIDS+=($DRIVER)
wait "$DRIVER"; RC=$?

if [ "$RC" -eq 0 ] && grep -q "attended transfer complete" logs/alice.log; then
  echo -e "\n${GREEN}✅ DEMO SUCCESSFUL — Alice was attended-transferred to Charlie${NC}"
  exit 0
else
  echo -e "\n${RED}❌ DEMO FAILED (alice exit $RC) — see logs/${NC}"
  exit 1
fi
