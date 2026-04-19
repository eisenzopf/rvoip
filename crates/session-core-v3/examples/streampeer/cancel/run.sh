#!/usr/bin/env bash
# CANCEL / 487 example: Alice calls Bob, Bob holds the incoming call
# without accepting, Alice hangs up while ringing, observes Event::CallCancelled.
#
# Exercises the full UAC-hangup → CANCEL → 200 OK → 487 → CallCancelled round
# trip — RFC 3261 §9 / §10.2.8. See `tests/cancel_integration.rs` for the
# automated version.
set -euo pipefail
cd "$(dirname "$0")/../../.."   # crate root

GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; RED='\033[0;31m'; NC='\033[0m'
cleanup() { pkill -P $$ 2>/dev/null || true; wait 2>/dev/null || true; }
trap cleanup EXIT

ALICE_PORT="${ALICE_PORT:-35071}"
BOB_PORT="${BOB_PORT:-35072}"
export ALICE_PORT BOB_PORT

echo -e "${GREEN}Building...${NC}"
cargo build -p rvoip-session-core-v3 \
  --example streampeer_cancel_alice \
  --example streampeer_cancel_bob 2>&1 | grep -v '^warning:' | grep -v '^\s' | grep -v '^$' || true

echo -e "${CYAN}[BOB]${NC} Listening on port $BOB_PORT (will hold incoming call without accepting)"
cargo run -p rvoip-session-core-v3 --example streampeer_cancel_bob --quiet \
  2>&1 | sed "s/^/$(printf '\033[0;36m')[BOB]$(printf '\033[0m') /" &
sleep 1

echo -e "${GREEN}[ALICE]${NC} Calling Bob on port $BOB_PORT from $ALICE_PORT (will hangup mid-ring)"
cargo run -p rvoip-session-core-v3 --example streampeer_cancel_alice --quiet \
  2>&1 | sed "s/^/$(printf '\033[0;32m')[ALICE]$(printf '\033[0m') /"
ALICE_EXIT=$?

sleep 1
if [ $ALICE_EXIT -eq 0 ]; then
  echo -e "\n${GREEN}=== Example complete: Alice saw CallCancelled as expected ===${NC}"
else
  echo -e "\n${RED}=== Alice failed (exit $ALICE_EXIT) ===${NC}"
  exit 1
fi
