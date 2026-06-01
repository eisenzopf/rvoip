#!/usr/bin/env bash
# Run protocol and behavior regression example fixtures.
set -euo pipefail
cd "$(dirname "$0")"

SCRIPTS=(
  "./01_dtmf_round_trip/run.sh"
  "./02_tls/run.sh"
  "./03_srtp/run.sh"
  "./04_cancel/run.sh"
  "./05_prack/run.sh"
  "./06_session_timer/run.sh"
  "./07_session_timer_failure/run.sh"
  "./08_glare_retry/run.sh"
  "./09_notify_send/run.sh"
)

START=$SECONDS

for script in "${SCRIPTS[@]}"; do
  echo
  echo "==> $script"
  bash "$script"
  sleep 1
done

echo
echo "All ${#SCRIPTS[@]} regression example fixtures passed in $((SECONDS - START))s"
