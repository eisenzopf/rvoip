#!/usr/bin/env bash
# demo-uctp-bridge.sh — start the UCTP v0 demo and stream logs from each
# process side-by-side with colored prefixes. Cleans up on Ctrl-C.
#
# Default: orchestrator + uctp_agent_quic + uctp_agent_wt
#   - orchestrator listens on 127.0.0.1:4433 (UCTP+WT) and 127.0.0.1:5072 (SIP)
#   - it auto-bridges the first two non-SIP inbound connections
#
# Usage:
#   scripts/demo-uctp-bridge.sh                 # orchestrator + both UCTP agents
#   scripts/demo-uctp-bridge.sh quic            # orchestrator + QUIC agent only
#   scripts/demo-uctp-bridge.sh wt              # orchestrator + WT agent only
#   scripts/demo-uctp-bridge.sh quic wt sip     # also start sip_caller
#
# Tail control: every line is prefixed `[role]` and colored so you can see
# which process emitted what. Press Ctrl-C to kill everything.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

# --- Args ---
if [[ $# -eq 0 ]]; then
  AGENTS=(quic wt)
else
  AGENTS=("$@")
fi

# --- Colors ---
RESET=$'\033[0m'
BOLD=$'\033[1m'
RED=$'\033[31m'
GREEN=$'\033[32m'
YELLOW=$'\033[33m'
BLUE=$'\033[34m'
MAGENTA=$'\033[35m'
CYAN=$'\033[36m'

# --- Process tracking ---
PIDS=()
LOG_DIR="$(mktemp -d -t uctp-demo-XXXXXX)"
echo "${BOLD}logs:${RESET} $LOG_DIR"

cleanup() {
  echo
  echo "${YELLOW}[demo] shutting down…${RESET}"
  for pid in "${PIDS[@]}"; do
    if kill -0 "$pid" 2>/dev/null; then
      kill -TERM "$pid" 2>/dev/null || true
    fi
  done
  # Give them a beat to flush
  sleep 0.5
  for pid in "${PIDS[@]}"; do
    if kill -0 "$pid" 2>/dev/null; then
      kill -KILL "$pid" 2>/dev/null || true
    fi
  done
  echo "${YELLOW}[demo] done — logs preserved at $LOG_DIR${RESET}"
}
trap cleanup EXIT INT TERM

# --- Build all binaries up front so we don't interleave cargo output with logs ---
echo "${BOLD}[demo] building example binaries…${RESET}"
cargo build -p rvoip-uctp \
  --example orchestrator_bridge \
  --example uctp_agent_quic \
  --example uctp_agent_wt \
  --example sip_caller \
  >/dev/null 2>&1

# --- Helpers ---
# Run a cargo example in the background, prefix every log line with a
# colored [role] tag, and track the PID for cleanup.
spawn() {
  local role="$1" color="$2" bin="$3"
  shift 3
  local log_file="$LOG_DIR/$role.log"
  (
    # Disable cargo's progress chrome so the prefix matches log content
    cargo run -p rvoip-uctp --example "$bin" -- "$@" 2>&1 \
      | while IFS= read -r line; do
          printf '%s[%s]%s %s\n' "$color" "$role" "$RESET" "$line"
          printf '%s\n' "$line" >>"$log_file"
        done
  ) &
  PIDS+=($!)
}

# --- 1. orchestrator first; wait until it's announced 'ready' ---
spawn orch "$CYAN" orchestrator_bridge

# Wait for the orchestrator's "ready" log line, with a 20s ceiling.
echo "${BOLD}[demo] waiting for orchestrator to be ready…${RESET}"
ready_file="$LOG_DIR/orch.log"
for _ in $(seq 1 40); do
  if [[ -s "$ready_file" ]] && grep -q "ready — waiting for events" "$ready_file"; then
    break
  fi
  sleep 0.5
done
if ! grep -q "ready — waiting for events" "$ready_file" 2>/dev/null; then
  echo "${RED}[demo] orchestrator did not become ready in 20s — check $ready_file${RESET}"
  exit 1
fi
echo "${GREEN}[demo] orchestrator is ready${RESET}"

# --- 2. agents in the order requested ---
for agent in "${AGENTS[@]}"; do
  case "$agent" in
    quic)
      spawn "quic-agent" "$GREEN"   uctp_agent_quic
      ;;
    wt)
      spawn "wt-agent"   "$MAGENTA" uctp_agent_wt
      ;;
    sip)
      # Small delay so the SIP transport is fully up
      sleep 0.3
      spawn "sip-caller" "$YELLOW"  sip_caller
      ;;
    *)
      echo "${RED}[demo] unknown agent: $agent (expected quic | wt | sip)${RESET}"
      exit 2
      ;;
  esac
done

echo "${BOLD}[demo] all processes started. Ctrl-C to stop.${RESET}"
echo "${BOLD}[demo] pids: ${PIDS[*]}${RESET}"

# --- 3. Block until any child exits or Ctrl-C ---
# `wait -n` isn't in bash 3.2 (default macOS), so poll the PIDs.
while true; do
  all_alive=true
  for pid in "${PIDS[@]}"; do
    if ! kill -0 "$pid" 2>/dev/null; then
      all_alive=false
      break
    fi
  done
  if ! $all_alive; then
    break
  fi
  sleep 0.5
done
echo "${YELLOW}[demo] a process exited; tearing down the rest${RESET}"
