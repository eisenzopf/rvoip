#!/usr/bin/env bash
# Bridge example: Alice -> bridge_peer -> Carol, with RTP relay through
# UnifiedCoordinator::bridge(). Alice sends 440Hz, Carol sends 880Hz,
# and a working bridge means each side's WAV carries the *other* peer's tone.
set -euo pipefail
cd "$(dirname "$0")/../../.."   # crate root

GREEN='\033[0;32m'; CYAN='\033[0;36m'; MAGENTA='\033[0;35m'; RED='\033[0;31m'; NC='\033[0m'
cleanup() { pkill -P $$ 2>/dev/null || true; wait 2>/dev/null || true; }
trap cleanup EXIT

echo -e "${GREEN}Building...${NC}"
cargo build -p rvoip-session-core \
  --example streampeer_bridge_alice \
  --example streampeer_bridge_carol \
  --example streampeer_bridge_peer 2>&1 | grep -v '^warning:' | grep -v '^\s' | grep -v '^$' || true

echo -e "${MAGENTA}[CAROL]${NC} Starting (callee)"
cargo run -p rvoip-session-core --example streampeer_bridge_carol --quiet \
  2>&1 | sed "s/^/$(printf '\033[0;35m')[CAROL]$(printf '\033[0m') /" &
sleep 1

echo -e "${GREEN}[BRIDGE]${NC} Starting (b2bua)"
cargo run -p rvoip-session-core --example streampeer_bridge_peer --quiet \
  2>&1 | sed "s/^/$(printf '\033[0;32m')[BRIDGE]$(printf '\033[0m') /" &
sleep 1

echo -e "${CYAN}[ALICE]${NC} Starting (caller)"
cargo run -p rvoip-session-core --example streampeer_bridge_alice --quiet \
  2>&1 | sed "s/^/$(printf '\033[0;36m')[ALICE]$(printf '\033[0m') /"
CLIENT_EXIT=$?

sleep 1
if [ $CLIENT_EXIT -eq 0 ]; then
  echo -e "\n${GREEN}=== Bridge example complete ===${NC}"
  echo -e "Inspect ${CYAN}output/alice_received.wav${NC} (should carry Carol's 880 Hz)"
  echo -e "Inspect ${MAGENTA}output/carol_received.wav${NC} (should carry Alice's 440 Hz)"
else
  echo -e "\n${RED}=== Alice failed (exit $CLIENT_EXIT) ===${NC}"
  exit 1
fi
