#!/usr/bin/env bash
# DTMF round-trip example: server auto-answers and logs each received
# digit; client sends 1,2,3,4,# and hangs up. The SERVER stdout is
# captured to a temp file and tailed after the client exits so the
# pkill teardown doesn't truncate its output. We then assert the
# server actually observed all 5 digits — this locks in the full
# RFC 4733 receive path (PT 101 decode → media-core callback →
# session-core `on_dtmf`).
set -euo pipefail
cd "$(dirname "$0")/../../.."   # crate root

GREEN='\033[0;32m'; CYAN='\033[0;36m'; RED='\033[0;31m'; NC='\033[0m'
SERVER_LOG="$(mktemp -t dtmf_server.XXXXXX)"
cleanup() {
  pkill -P $$ 2>/dev/null || true
  wait 2>/dev/null || true
  rm -f "$SERVER_LOG"
}
trap cleanup EXIT

echo -e "${GREEN}Building...${NC}"
cargo build -p rvoip-session-core \
  --example streampeer_dtmf_server \
  --example streampeer_dtmf_client 2>&1 | grep -v '^warning:' | grep -v '^\s' | grep -v '^$' || true

echo -e "${GREEN}[SERVER]${NC} Starting DTMF-logging server on port 5060"
cargo run -p rvoip-session-core --example streampeer_dtmf_server --quiet > "$SERVER_LOG" 2>&1 &
SERVER_PID=$!
sleep 2

echo -e "${CYAN}[CLIENT]${NC} Starting DTMF sender"
cargo run -p rvoip-session-core --example streampeer_dtmf_client --quiet \
  2>&1 | sed "s/^/$(printf '\033[0;36m')[CLIENT]$(printf '\033[0m') /"
CLIENT_EXIT=$?

# Let the server process the final DTMF, then terminate cleanly and
# replay its stdout so the reader sees on_dtmf hits.
sleep 1
kill -INT $SERVER_PID 2>/dev/null || true
wait $SERVER_PID 2>/dev/null || true

echo ""
sed "s/^/$(printf '\033[0;32m')[SERVER]$(printf '\033[0m') /" "$SERVER_LOG"

# Regression check: the client sends 5 digits; the server must have
# observed all of them. "DTMF digits seen: 5" is the exact string the
# server prints in on_call_ended.
if ! grep -q "DTMF digits seen: 5" "$SERVER_LOG"; then
  echo -e "\n${RED}=== Server did not observe all 5 DTMF digits ===${NC}"
  exit 1
fi

if [ $CLIENT_EXIT -eq 0 ]; then
  echo -e "\n${GREEN}=== Example complete — 5/5 DTMF digits round-tripped ===${NC}"
else
  echo -e "\n${RED}=== Client failed (exit $CLIENT_EXIT) ===${NC}"
  exit 1
fi
