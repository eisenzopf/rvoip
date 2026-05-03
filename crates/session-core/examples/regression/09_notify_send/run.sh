#!/usr/bin/env bash
# NOTIFY send example: Alice calls Bob, Bob accepts, Alice sends a NOTIFY
# with `event_package = "dialog"` + `Subscription-State: active;expires=3600`,
# Bob's event stream surfaces `Event::NotifyReceived` with matching fields.
#
# Exercises the RFC 6665 NOTIFY round-trip through the new public API:
# `SessionHandle::send_notify` → `DialogAdapter::send_notify` → dialog-core
# → `DialogToSessionEvent::NotifyReceived` → `Event::NotifyReceived`. See
# `tests/notify_send_integration.rs` for the automated version.
set -euo pipefail
cd "$(dirname "$0")/../../.."   # crate root

GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; RED='\033[0;31m'; NC='\033[0m'
cleanup() { pkill -P $$ 2>/dev/null || true; wait 2>/dev/null || true; }
trap cleanup EXIT

ALICE_PORT="${ALICE_PORT:-35091}"
BOB_PORT="${BOB_PORT:-35092}"
export ALICE_PORT BOB_PORT

echo -e "${GREEN}Building...${NC}"
cargo build -p rvoip-session-core \
  --example streampeer_notify_send_alice \
  --example streampeer_notify_send_bob 2>&1 | grep -v '^warning:' | grep -v '^\s' | grep -v '^$' || true

echo -e "${CYAN}[BOB]${NC} Listening on port $BOB_PORT (auto-accept; expects Event::NotifyReceived)"
cargo run -p rvoip-session-core --example streampeer_notify_send_bob --quiet \
  2>&1 | sed "s/^/$(printf '\033[0;36m')[BOB]$(printf '\033[0m') /" &
BOB_PID=$!
sleep 1

echo -e "${GREEN}[ALICE]${NC} Calling Bob on port $BOB_PORT from $ALICE_PORT (sends NOTIFY after 200 OK)"
cargo run -p rvoip-session-core --example streampeer_notify_send_alice --quiet \
  2>&1 | sed "s/^/$(printf '\033[0;32m')[ALICE]$(printf '\033[0m') /"
ALICE_EXIT=$?

wait $BOB_PID
BOB_EXIT=$?

if [ $ALICE_EXIT -eq 0 ] && [ $BOB_EXIT -eq 0 ]; then
  echo -e "\n${GREEN}=== Example complete: Alice sent NOTIFY, Bob observed Event::NotifyReceived ===${NC}"
else
  echo -e "\n${RED}=== Failed (Alice=$ALICE_EXIT, Bob=$BOB_EXIT) ===${NC}"
  exit 1
fi
