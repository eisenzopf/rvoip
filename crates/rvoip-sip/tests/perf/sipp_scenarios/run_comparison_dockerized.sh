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
CPS_LEVELS="${RVOIP_PERF_CPS:-30 100 300 1000 2000 5000}"
SIPP_SHARD_CPS="${RVOIP_PERF_SIPP_SHARD_CPS:-1000}"
TRACE_SCREEN="${RVOIP_PERF_TRACE_SCREEN:-0}"
SIPP_EXTRA_ARGS="${RVOIP_PERF_SIPP_EXTRA_ARGS:--timer_resol 1 -max_recv_loops 10000 -max_sched_loops 10000}"
if [[ "$SIPP_SHARD_CPS" -lt 1 ]]; then SIPP_SHARD_CPS=1000; fi

if ! docker image inspect "$SIPP_IMAGE" >/dev/null 2>&1; then
  echo "sipp image '$SIPP_IMAGE' not built. Build it first." >&2
  exit 1
fi

echo "[run_comparison] target=$TARGET_HOST:$TARGET_PORT tag=$TAG image=$SIPP_IMAGE network=$NETWORK steady=${STEADY_SECS}s cps=$CPS_LEVELS shard_cps=$SIPP_SHARD_CPS"

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

ceil_div() {
  local n="$1"
  local d="$2"
  echo $(( (n + d - 1) / d ))
}

runner_count_for_cps() {
  local cps="$1"
  if [[ -n "${RVOIP_PERF_SIPP_RUNNERS:-}" ]]; then
    echo "$RVOIP_PERF_SIPP_RUNNERS"
    return
  fi
  ceil_div "$cps" "$SIPP_SHARD_CPS"
}

run_sipp_shard() {
  local stat_prefix="$1"
  local shard_cps="$2"
  local calls="$3"
  local local_port="$4"
  local run_timeout="$5"
  local cmd=(
    docker exec "$RUNNER" sipp
    -sf /scenarios/uac_perf.xml
    -r "$shard_cps"
    -m "$calls"
    -p "$local_port"
    -nostdin
  )

  for arg in $SIPP_EXTRA_ARGS; do
    cmd+=("$arg")
  done

  cmd+=(
    -trace_stat
    -stf "/results/$stat_prefix"
  )

  if [[ "$TRACE_SCREEN" == "1" || "$TRACE_SCREEN" == "true" ]]; then
    cmd+=(-trace_screen -screen_file "/results/${stat_prefix}_screen.log")
  fi

  cmd+=(
    -trace_err
    -error_file "/results/${stat_prefix}_errors.log"
    -timeout "${run_timeout}s"
    -timeout_error
    "${TARGET_HOST}:${TARGET_PORT}"
  )

  "${cmd[@]}" >/dev/null 2>&1
}

for CPS in $CPS_LEVELS; do
  RUNNERS="$(runner_count_for_cps "$CPS")"
  if [[ "$RUNNERS" -lt 1 ]]; then RUNNERS=1; fi
  if [[ "$RUNNERS" -gt "$CPS" ]]; then RUNNERS="$CPS"; fi

  RUN_TIMEOUT=$(( STEADY_SECS + 60 ))
  BASE_PORT=$(( 35000 + RANDOM % 2000 ))
  REMAINING_CPS="$CPS"
  PIDS=()

  echo "[run_comparison] === ${TAG} @ ${CPS} CPS (${RUNNERS} runner(s), steady ${STEADY_SECS}s) ==="

  for (( shard=0; shard<RUNNERS; shard++ )); do
    REMAINING_SHARDS=$(( RUNNERS - shard ))
    SHARD_RATE=$(( (REMAINING_CPS + REMAINING_SHARDS - 1) / REMAINING_SHARDS ))
    if [[ "$SHARD_RATE" -lt 1 ]]; then SHARD_RATE=1; fi
    REMAINING_CPS=$(( REMAINING_CPS - SHARD_RATE ))

    CALLS=$(( SHARD_RATE * STEADY_SECS ))
    if [[ "$CALLS" -lt 10 ]]; then CALLS=10; fi

    SIPP_LOCAL_PORT=$(( BASE_PORT + shard ))
    if [[ "$RUNNERS" -eq 1 ]]; then
      STAT_PREFIX="${TAG}_${CPS}cps"
    else
      STAT_PREFIX="${TAG}_${CPS}cps_s${shard}"
    fi

    echo "[run_comparison] shard=${shard} rate=${SHARD_RATE} calls=${CALLS} sipp:${SIPP_LOCAL_PORT} prefix=${STAT_PREFIX}"
    run_sipp_shard "$STAT_PREFIX" "$SHARD_RATE" "$CALLS" "$SIPP_LOCAL_PORT" "$RUN_TIMEOUT" &
    PIDS+=("$!")
  done

  RC=0
  for pid in "${PIDS[@]}"; do
    if ! wait "$pid"; then
      RC=1
    fi
  done

  echo "[run_comparison] rc=$RC"
  sleep 2
done

# Copy results out before the container is dropped.
echo "[run_comparison] copying results from container to $RESULTS_DIR"
docker cp "$RUNNER:/results/." "$RESULTS_DIR/"

echo "[run_comparison] done. results in $RESULTS_DIR"
