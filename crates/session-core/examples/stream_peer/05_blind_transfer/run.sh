#!/usr/bin/env bash
# Blind transfer example: Alice calls Bob, Bob transfers to Charlie.
set -euo pipefail
cd "$(dirname "$0")/../../.."   # crate root

GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; RED='\033[0;31m'; NC='\033[0m'
cleanup() { pkill -P $$ 2>/dev/null || true; wait 2>/dev/null || true; }
trap cleanup EXIT

echo -e "${GREEN}Building...${NC}"
cargo build -p rvoip-session-core \
  --example streampeer_blind_transfer_alice \
  --example streampeer_blind_transfer_bob \
  --example streampeer_blind_transfer_charlie 2>&1 | grep -v '^warning:' | grep -v '^\s' | grep -v '^$' || true

echo -e "${YELLOW}[CHARLIE]${NC} Starting (transfer target) on port 5062"
cargo run -p rvoip-session-core --example streampeer_blind_transfer_charlie --quiet \
  2>&1 | sed "s/^/$(printf '\033[1;33m')[CHARLIE]$(printf '\033[0m') /" &
sleep 1

echo -e "${CYAN}[BOB]${NC} Starting (transferor) on port 5061"
cargo run -p rvoip-session-core --example streampeer_blind_transfer_bob --quiet \
  2>&1 | sed "s/^/$(printf '\033[0;36m')[BOB]$(printf '\033[0m') /" &
sleep 1

echo -e "${GREEN}[ALICE]${NC} Starting (caller) on port 5060"
cargo run -p rvoip-session-core --example streampeer_blind_transfer_alice --quiet \
  2>&1 | sed "s/^/$(printf '\033[0;32m')[ALICE]$(printf '\033[0m') /"
ALICE_EXIT=$?

sleep 1
if [ $ALICE_EXIT -eq 0 ]; then
  echo -e "\n${GREEN}=== Example complete ===${NC}"
else
  echo -e "\n${RED}=== Alice failed (exit $ALICE_EXIT) ===${NC}"
  exit 1
fi
