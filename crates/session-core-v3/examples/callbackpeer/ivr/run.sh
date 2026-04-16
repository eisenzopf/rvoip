#!/usr/bin/env bash
# IVR example: client navigates DTMF menu.
set -euo pipefail
cd "$(dirname "$0")/../../.."   # crate root

GREEN='\033[0;32m'; CYAN='\033[0;36m'; RED='\033[0;31m'; NC='\033[0m'
cleanup() { pkill -P $$ 2>/dev/null || true; wait 2>/dev/null || true; }
trap cleanup EXIT

echo -e "${GREEN}Building...${NC}"
cargo build -p rvoip-session-core-v3 \
  --example callbackpeer_ivr_server \
  --example callbackpeer_ivr_client 2>&1 | grep -v '^warning:' | grep -v '^\s' | grep -v '^$' || true

echo -e "${GREEN}[IVR]${NC} Starting"
cargo run -p rvoip-session-core-v3 --example callbackpeer_ivr_server --quiet \
  2>&1 | sed "s/^/$(printf '\033[0;32m')[IVR]$(printf '\033[0m') /" &
sleep 2

echo -e "${CYAN}[CLIENT]${NC} Starting"
cargo run -p rvoip-session-core-v3 --example callbackpeer_ivr_client --quiet \
  2>&1 | sed "s/^/$(printf '\033[0;36m')[CLIENT]$(printf '\033[0m') /"
CLIENT_EXIT=$?

sleep 1
if [ $CLIENT_EXIT -eq 0 ]; then
  echo -e "\n${GREEN}=== Example complete ===${NC}"
else
  echo -e "\n${RED}=== Client failed (exit $CLIENT_EXIT) ===${NC}"
  exit 1
fi
