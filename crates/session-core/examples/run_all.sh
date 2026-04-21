#!/usr/bin/env bash
# Run every session-core example run.sh serially. First failure aborts
# the suite and the failing script's exit code propagates out.
#
# Excludes `advanced/registrar_server.rs` — no driving client, would block.
set -euo pipefail
cd "$(dirname "$0")"

BOLD='\033[1m'; BLUE='\033[1;34m'; GREEN='\033[1;32m'; RED='\033[1;31m'; NC='\033[0m'

SCRIPTS=(
  getting_started/run.sh
  streampeer/registration/run.sh
  streampeer/audio/run.sh
  streampeer/dtmf/run.sh
  streampeer/hold_resume/run.sh
  streampeer/cancel/run.sh
  streampeer/prack/run.sh
  streampeer/session_timer/run.sh
  streampeer/session_timer_failure/run.sh
  streampeer/glare_retry/run.sh
  streampeer/blind_transfer/run.sh
  streampeer/bridge/run.sh
  streampeer/notify_send/run.sh
  callbackpeer/auto_answer/run.sh
  callbackpeer/closure/run.sh
  callbackpeer/custom/run.sh
  callbackpeer/ivr/run.sh
  callbackpeer/queue/run.sh
  callbackpeer/routing/run.sh
  advanced/concurrent_calls/run.sh
)

START=$SECONDS
PASSED=()

for s in "${SCRIPTS[@]}"; do
  echo -e "\n${BLUE}══════════════════════════════════════════════════════════════${NC}"
  echo -e "${BOLD}▶ $s${NC}"
  echo -e "${BLUE}══════════════════════════════════════════════════════════════${NC}"
  if ! bash "$s"; then
    echo -e "\n${RED}✘ $s failed — aborting suite${NC}"
    echo -e "${BOLD}Passed before failure:${NC} ${PASSED[*]:-<none>}"
    exit 1
  fi
  PASSED+=("$s")
  # Brief pause so any lingering cargo/rvoip-child processes release their
  # SIP ports before the next script binds them. Without this the default
  # SIP ports (5060/5061) can race between back-to-back scripts.
  sleep 2
done

echo -e "\n${GREEN}══════════════════════════════════════════════════════════════${NC}"
echo -e "${GREEN}✓ All ${#SCRIPTS[@]} example scripts passed in $((SECONDS - START))s${NC}"
echo -e "${GREEN}══════════════════════════════════════════════════════════════${NC}"
