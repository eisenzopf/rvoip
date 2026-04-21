#!/usr/bin/env bash
# Getting started: minimal self-contained hello-world binary.
set -euo pipefail
cd "$(dirname "$0")/../.."   # crate root

GREEN='\033[0;32m'; RED='\033[0;31m'; NC='\033[0m'
cleanup() { pkill -P $$ 2>/dev/null || true; wait 2>/dev/null || true; }
trap cleanup EXIT

echo -e "${GREEN}Building...${NC}"
cargo build -p rvoip-session-core --example hello 2>&1 | grep -v '^warning:' | grep -v '^\s' | grep -v '^$' || true

echo -e "${GREEN}[HELLO]${NC} Starting"
cargo run -p rvoip-session-core --example hello --quiet \
  2>&1 | sed "s/^/$(printf '\033[0;32m')[HELLO]$(printf '\033[0m') /"

echo -e "\n${GREEN}=== Example complete ===${NC}"
