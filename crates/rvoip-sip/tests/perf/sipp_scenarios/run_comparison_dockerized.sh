#!/usr/bin/env bash
# rvoip vs Asterisk sipp-driven comparison using a containerized sipp.
#
# Why: Docker Desktop on macOS doesn't share host-loopback UDP into
# bridge-networked containers in both directions — the request gets
# in, but the reply's source-IP rewrite (container bridge IP) makes
# the host-side sipp drop it. Running sipp inside a sidecar on the
# same docker network as the SUT sidesteps this, and using
# `host.docker.internal` lets the same sipp container reach a
# rvoip-sip listener bound to the macOS host.
#
# A second macOS-specific quirk: bind-mounted host directories don't
# always pick up files the container wrote during a short-lived run.
# To avoid that, we keep the container alive, run sipp inside it
# (writing to a container-local /results), and `docker cp` the
# results out before tearing it down.
#
# Usage:
#   ./run_comparison_dockerized.sh [TARGET_HOST] [TARGET_PORT] [TAG]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCENARIO="$SCRIPT_DIR/uac_perf.xml"
RESULTS_DIR="${RVOIP_PERF_RESULTS:-$SCRIPT_DIR/results}"
mkdir -p "$RESULTS_DIR"

TARGET_HOST="${1:-rvoip-asterisk}"
TARGET_PORT="${2:-5060}"
TAG="${3:-asterisk}"
SIPP_IMAGE="${SIPP_IMAGE:-local-sipp}"
NETWORK="${SIPP_NETWORK:-asterisk_asterisk-local}"
STEADY_SECS="${RVOIP_PERF_STEADY_SECS:-15}"
CPS_LEVELS="${RVOIP_PERF_CPS:-30 100 300}"

if ! docker image inspect "$SIPP_IMAGE" >/dev/null 2>&1; then
  echo "sipp image '$SIPP_IMAGE' not built. Build it first." >&2
  exit 1
fi

echo "[run_comparison] target=$TARGET_HOST:$TARGET_PORT tag=$TAG image=$SIPP_IMAGE network=$NETWORK steady=${STEADY_SECS}s cps=$CPS_LEVELS"

# Spin up one long-running sipp container per sweep and reuse it for
# every CPS band; copy results out at the end.
RUNNER="sipp-runner-$$"
docker rm -f "$RUNNER" >/dev/null 2>&1 || true
docker run -d \
  --name "$RUNNER" \
  --network "$NETWORK" \
  -v "$SCRIPT_DIR:/scenarios:ro" \
  --entrypoint /bin/sh \
  "$SIPP_IMAGE" \
  -c 'mkdir -p /results && tail -f /dev/null' >/dev/null

cleanup() {
  docker rm -f "$RUNNER" >/dev/null 2>&1 || true
}
trap cleanup EXIT

for CPS in $CPS_LEVELS; do
  CALLS=$(( CPS * STEADY_SECS ))
  if [[ "$CALLS" -lt 10 ]]; then CALLS=10; fi

  SIPP_LOCAL_PORT=$(( 35000 + RANDOM % 1000 ))
  STAT_PREFIX="${TAG}_${CPS}cps"
  RUN_TIMEOUT=$(( STEADY_SECS + 30 ))

  echo "[run_comparison] === ${TAG} @ ${CPS} CPS (${CALLS} calls, sipp:${SIPP_LOCAL_PORT}) ==="

  set +e
  docker exec "$RUNNER" sipp \
    -sf /scenarios/uac_perf.xml \
    -r "$CPS" \
    -m "$CALLS" \
    -p "$SIPP_LOCAL_PORT" \
    -nostdin \
    -trace_stat \
    -stf "/results/$STAT_PREFIX" \
    -trace_screen \
    -screen_file "/results/${STAT_PREFIX}_screen.log" \
    -trace_err \
    -error_file "/results/${STAT_PREFIX}_errors.log" \
    -timeout "${RUN_TIMEOUT}s" \
    -timeout_error \
    "${TARGET_HOST}:${TARGET_PORT}" >/dev/null 2>&1
  RC=$?
  set -e

  echo "[run_comparison] rc=$RC"
  sleep 2
done

# Copy results out before the container is dropped.
echo "[run_comparison] copying results from container to $RESULTS_DIR"
docker cp "$RUNNER:/results/." "$RESULTS_DIR/"

echo "[run_comparison] done. results in $RESULTS_DIR"
