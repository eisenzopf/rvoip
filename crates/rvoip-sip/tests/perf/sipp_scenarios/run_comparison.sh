#!/usr/bin/env bash
# rvoip vs Asterisk performance comparison using sipp as the common driver.
#
# - sipp drives a custom UAC scenario (uac_perf.xml) against both
#   targets at 30 / 100 / 300 CPS.
# - Targets share the same wire-level test shape (INVITE → 200 → ACK
#   → 100 ms hold → BYE → 200), so the differences captured reflect
#   the SUT's signalling path, not the driver.
# - Each run writes a per-CPS stat CSV + the screen log to RESULTS_DIR;
#   the aggregator at the end emits a markdown comparison table.
#
# Usage:
#   ./run_comparison.sh [TARGET_HOST] [TARGET_PORT] [TAG]
#
# The script is meant to be invoked twice — once per target — and the
# resulting CSVs live side-by-side under RESULTS_DIR for the aggregator.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCENARIO="$SCRIPT_DIR/uac_perf.xml"
RESULTS_DIR="${RVOIP_PERF_RESULTS:-$SCRIPT_DIR/results}"
mkdir -p "$RESULTS_DIR"

TARGET_HOST="${1:-127.0.0.1}"
TARGET_PORT="${2:-5060}"
TAG="${3:-target}"
SIPP_BIN="${SIPP_BIN:-sipp}"
STEADY_SECS="${RVOIP_PERF_STEADY_SECS:-15}"
# CPS levels: doc says 30/100/300; can be overridden for quick smoke.
CPS_LEVELS="${RVOIP_PERF_CPS:-30 100 300}"

if ! command -v "$SIPP_BIN" >/dev/null 2>&1; then
  echo "sipp not found on PATH (looked for '$SIPP_BIN'). brew install sipp." >&2
  exit 1
fi
if ! [[ -f "$SCENARIO" ]]; then
  echo "scenario not found: $SCENARIO" >&2
  exit 1
fi

echo "[run_comparison] target=$TARGET_HOST:$TARGET_PORT tag=$TAG steady=${STEADY_SECS}s cps=$CPS_LEVELS"

for CPS in $CPS_LEVELS; do
  CALLS=$(( CPS * STEADY_SECS ))
  if [[ "$CALLS" -lt 10 ]]; then CALLS=10; fi

  # Pick a per-run local sipp port to avoid contention across re-runs.
  SIPP_LOCAL_PORT=$(( 35000 + RANDOM % 1000 ))
  STAT_PREFIX="$RESULTS_DIR/${TAG}_${CPS}cps"
  SCREEN="$RESULTS_DIR/${TAG}_${CPS}cps_screen.log"
  ERRLOG="$RESULTS_DIR/${TAG}_${CPS}cps_errors.log"

  echo "[run_comparison] === ${TAG} @ ${CPS} CPS (${CALLS} calls, sipp:${SIPP_LOCAL_PORT}) ==="

  # Drive the run. Timeout = STEADY_SECS + 30 s buffer so we don't get
  # stuck if the SUT silently drops calls (sipp would otherwise retry
  # forever).
  RUN_TIMEOUT=$(( STEADY_SECS + 30 ))
  set +e
  "$SIPP_BIN" \
    -sf "$SCENARIO" \
    -r "$CPS" \
    -m "$CALLS" \
    -p "$SIPP_LOCAL_PORT" \
    -nostdin \
    -trace_stat \
    -stf "$STAT_PREFIX" \
    -trace_screen \
    -screen_file "$SCREEN" \
    -trace_err \
    -error_file "$ERRLOG" \
    -timeout "${RUN_TIMEOUT}s" \
    -timeout_error \
    "${TARGET_HOST}:${TARGET_PORT}" >/dev/null 2>&1
  RC=$?
  set -e

  # sipp exits 0 = all good, 1 = at least one call failed, 2 = some
  # failure threshold hit, 97 = exited via -timeout. We tolerate
  # anything that left a stat CSV behind because the analysis step
  # cares about successful-call counts, not the exit code.
  STAT_FILE="$(ls -1 "${STAT_PREFIX}"_*.csv 2>/dev/null | head -1 || true)"
  if [[ -z "$STAT_FILE" ]]; then
    echo "[run_comparison] WARN: no stat CSV produced for ${TAG}@${CPS}cps (rc=$RC)" >&2
  else
    echo "[run_comparison] -> $STAT_FILE (rc=$RC)"
  fi

  # Small inter-run pause so the SUT settles between CPS bands.
  sleep 2
done

echo "[run_comparison] done. results in $RESULTS_DIR"
