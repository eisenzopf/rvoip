#!/usr/bin/env bash
# RFC 4028 session-timer example: Alice and Bob negotiate a 10 s Session-Expires.
# Alice (refresher) sends an UPDATE at half-expiry and observes
# SessionRefreshed. See tests/session_timer_integration.rs for the CI version.
set -euo pipefail
cd "$(dirname "$0")/../../.."   # crate root

GREEN='\033[0;32m'; CYAN='\033[0;36m'; RED='\033[0;31m'; NC='\033[0m'
cleanup() { pkill -P $$ 2>/dev/null || true; wait 2>/dev/null || true; }
trap cleanup EXIT

ALICE_PORT="${ALICE_PORT:-35083}"
BOB_PORT="${BOB_PORT:-35084}"
export ALICE_PORT BOB_PORT

echo -e "${GREEN}Building...${NC}"
cargo build -p rvoip-session-core \
  --example streampeer_session_timer_alice \
  --example streampeer_session_timer_bob 2>&1 | grep -v '^warning:' | grep -v '^\s' | grep -v '^$' || true

echo -e "${CYAN}[BOB]${NC} Listening on port $BOB_PORT"
cargo run -p rvoip-session-core --example streampeer_session_timer_bob --quiet \
  2>&1 | sed "s/^/$(printf '\033[0;36m')[BOB]$(printf '\033[0m') /" &
sleep 1

echo -e "${GREEN}[ALICE]${NC} Calling Bob from $ALICE_PORT (expects SessionRefreshed ≤ 12s)"
cargo run -p rvoip-session-core --example streampeer_session_timer_alice --quiet \
  2>&1 | sed "s/^/$(printf '\033[0;32m')[ALICE]$(printf '\033[0m') /"
ALICE_EXIT=$?

sleep 1
if [ $ALICE_EXIT -eq 0 ]; then
  echo -e "\n${GREEN}=== Session timer refresh observed ===${NC}"
else
  echo -e "\n${RED}=== Alice failed (exit $ALICE_EXIT) ===${NC}"
  exit 1
fi
