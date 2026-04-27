#!/usr/bin/env bash
# Asterisk softphone example: registers with a remote Asterisk SIP server
# using settings from ./.env, idles for IDLE_SECS, then unregisters.
set -euo pipefail
cd "$(dirname "$0")/../../.."   # crate root

GREEN='\033[0;32m'; CYAN='\033[0;36m'; RED='\033[0;31m'; NC='\033[0m'

echo -e "${GREEN}Building...${NC}"
cargo build -p rvoip-session-core \
  --example asterisk_softphone 2>&1 | grep -v '^warning:' | grep -v '^\s' | grep -v '^$' || true

echo -e "${CYAN}[SOFTPHONE]${NC} Starting"
cargo run -p rvoip-session-core --example asterisk_softphone --quiet \
  2>&1 | sed "s/^/$(printf '\033[0;36m')[SOFTPHONE]$(printf '\033[0m') /"
EXIT=$?

if [ $EXIT -eq 0 ]; then
  echo -e "\n${GREEN}=== Example complete ===${NC}"
else
  echo -e "\n${RED}=== Softphone failed (exit $EXIT) ===${NC}"
  exit 1
fi
