#!/usr/bin/env bash
# PRACK example: runs both RFC 3262 scenarios back-to-back.
#   negative — Alice advertises no 100rel; Bob requires it → 420 Bad Extension.
#   positive — Bob sends reliable 183; Alice auto-PRACKs; call answers.
# See tests/prack_integration.rs for the CI version.
set -euo pipefail
cd "$(dirname "$0")/../../.."   # crate root

GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; RED='\033[0;31m'; NC='\033[0m'
cleanup() { pkill -P $$ 2>/dev/null || true; wait 2>/dev/null || true; }
trap cleanup EXIT

echo -e "${GREEN}Building...${NC}"
cargo build -p rvoip-session-core \
  --example streampeer_prack_alice \
  --example streampeer_prack_bob 2>&1 | grep -v '^warning:' | grep -v '^\s' | grep -v '^$' || true

run_scenario() {
  local mode="$1" alice_port="$2" bob_port="$3"

  echo -e "\n${YELLOW}── PRACK mode: ${mode} (Alice ${alice_port} ↔ Bob ${bob_port}) ──${NC}"

  PRACK_MODE="$mode" ALICE_PORT="$alice_port" BOB_PORT="$bob_port" \
    cargo run -p rvoip-session-core --example streampeer_prack_bob --quiet \
    2>&1 | sed "s/^/$(printf '\033[0;36m')[BOB.${mode}]$(printf '\033[0m') /" &
  sleep 1

  PRACK_MODE="$mode" ALICE_PORT="$alice_port" BOB_PORT="$bob_port" \
    cargo run -p rvoip-session-core --example streampeer_prack_alice --quiet \
    2>&1 | sed "s/^/$(printf '\033[0;32m')[ALICE.${mode}]$(printf '\033[0m') /"

  # Let Bob wind down on his own; different ports for each scenario mean any
  # stragglers don't collide with the next run.
  sleep 1
  echo -e "${GREEN}=== PRACK ${mode} complete ===${NC}"
}

run_scenario "negative" 35093 35094
run_scenario "positive" 35095 35096

echo -e "\n${GREEN}=== PRACK: both scenarios passed ===${NC}"
