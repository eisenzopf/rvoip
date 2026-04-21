#!/usr/bin/env bash
# RFC 4028 §10 session-timer FAILURE example: Alice calls Bob with a 4 s
# Session-Expires; Bob exits mid-call, Alice's UPDATE times out, dialog-core
# falls back to re-INVITE, that also times out, and the session tears down
# with a 408 BYE → SessionRefreshFailed event. Alice exits 0 iff the failure
# event arrives within 15 s. See tests/session_timer_failure_integration.rs.
set -euo pipefail
cd "$(dirname "$0")/../../.."   # crate root

GREEN='\033[0;32m'; CYAN='\033[0;36m'; RED='\033[0;31m'; NC='\033[0m'
cleanup() { pkill -P $$ 2>/dev/null || true; wait 2>/dev/null || true; }
trap cleanup EXIT

ALICE_PORT="${ALICE_PORT:-35085}"
BOB_PORT="${BOB_PORT:-35086}"
# Shorten Timer F so each UPDATE / re-INVITE to the dead peer gives up in
# ~2.5 s instead of the default 32 s. macOS UDP send-to-dead-port is silent,
# so the transaction layer has to drive the timeout itself.
RVOIP_TEST_TRANSACTION_TIMEOUT_MS="${RVOIP_TEST_TRANSACTION_TIMEOUT_MS:-2500}"
export ALICE_PORT BOB_PORT RVOIP_TEST_TRANSACTION_TIMEOUT_MS

echo -e "${GREEN}Building...${NC}"
cargo build -p rvoip-session-core \
  --example streampeer_session_timer_failure_alice \
  --example streampeer_session_timer_failure_bob 2>&1 | grep -v '^warning:' | grep -v '^\s' | grep -v '^$' || true

echo -e "${CYAN}[BOB]${NC} Listening on port $BOB_PORT (will exit mid-call at ~1.5s)"
cargo run -p rvoip-session-core --example streampeer_session_timer_failure_bob --quiet \
  2>&1 | sed "s/^/$(printf '\033[0;36m')[BOB]$(printf '\033[0m') /" &
sleep 1

echo -e "${GREEN}[ALICE]${NC} Calling Bob from $ALICE_PORT (expects SessionRefreshFailed ≤ 15s)"
cargo run -p rvoip-session-core --example streampeer_session_timer_failure_alice --quiet \
  2>&1 | sed "s/^/$(printf '\033[0;32m')[ALICE]$(printf '\033[0m') /"
ALICE_EXIT=$?

sleep 1
if [ $ALICE_EXIT -eq 0 ]; then
  echo -e "\n${GREEN}=== Session-timer failure handled correctly ===${NC}"
else
  echo -e "\n${RED}=== Alice failed (exit $ALICE_EXIT) ===${NC}"
  exit 1
fi
