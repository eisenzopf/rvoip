#!/usr/bin/env bash
# RFC 3261 §14.1 glare example: Alice and Bob reach Active, then both invoke
# hold() at the same wall-clock instant. Each UAS sees an incoming re-INVITE
# while its own is pending → HasPendingReinvite guard fires 491 Request
# Pending. Both peers' ReinviteGlare transition schedules a retry with random
# backoff; after retries resolve, both settle on OnHold.
# See tests/glare_retry_integration.rs for the CI version.
#
# NOTE: this example deliberately produces ERROR-level log lines on the
# first re-INVITE attempt ("Transaction terminated after timeout" →
# "Failed to execute action SendReINVITE"). Those are the 491 Request
# Pending handshake surfacing through the executor's generic
# action-failure logger, NOT a bug. The ReinviteGlare transition schedules
# a backoff retry and both peers converge to OnHold, which is the success
# criterion. See docs/EXAMPLE_RUN_ERRORS_TRACKING.md (Cluster D).
set -euo pipefail
cd "$(dirname "$0")/../../.."   # crate root

GREEN='\033[0;32m'; CYAN='\033[0;36m'; RED='\033[0;31m'; NC='\033[0m'
cleanup() { pkill -P $$ 2>/dev/null || true; wait 2>/dev/null || true; }
trap cleanup EXIT

ALICE_PORT="${ALICE_PORT:-35087}"
BOB_PORT="${BOB_PORT:-35088}"
# Both peers sleep until this wall-clock ms, then invoke hold() simultaneously.
# 4 s head start covers cargo-run spawn, peer bind, INVITE → 200 OK → ACK so
# both sides are Active by the time glare fires.
RVOIP_TEST_GLARE_START_MS="${RVOIP_TEST_GLARE_START_MS:-$(( $(date +%s) * 1000 + 4000 ))}"
export ALICE_PORT BOB_PORT RVOIP_TEST_GLARE_START_MS

echo -e "${GREEN}Building...${NC}"
cargo build -p rvoip-session-core \
  --example streampeer_glare_retry_alice \
  --example streampeer_glare_retry_bob 2>&1 | grep -v '^warning:' | grep -v '^\s' | grep -v '^$' || true

echo -e "${CYAN}[BOB]${NC} Listening on port $BOB_PORT (glare fires at ms=$RVOIP_TEST_GLARE_START_MS)"
cargo run -p rvoip-session-core --example streampeer_glare_retry_bob --quiet \
  2>&1 | sed "s/^/$(printf '\033[0;36m')[BOB]$(printf '\033[0m') /" &
sleep 1

echo -e "${GREEN}[ALICE]${NC} Calling Bob from $ALICE_PORT"
cargo run -p rvoip-session-core --example streampeer_glare_retry_alice --quiet \
  2>&1 | sed "s/^/$(printf '\033[0;32m')[ALICE]$(printf '\033[0m') /"
ALICE_EXIT=$?

sleep 1
if [ $ALICE_EXIT -eq 0 ]; then
  echo -e "\n${GREEN}=== Glare retry converged to OnHold ===${NC}"
else
  echo -e "\n${RED}=== Alice failed (exit $ALICE_EXIT) ===${NC}"
  exit 1
fi
