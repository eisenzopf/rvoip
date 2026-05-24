#!/usr/bin/env bash
# Signaling-only SIPp matrix for high-CPS sharding experiments.
#
# The listener knobs used here are intentionally thin CLI wrappers around
# public rvoip_sip::Config fields. This script should not grow server
# behavior that developers cannot set from Config.
#
# Usage:
#   ./run_signaling_sharding_matrix.sh [TARGET_HOST] [ADVERTISED_ADDR] [BASE_PORT]
#
# Common overrides:
#   RVOIP_SHARDING_CPS_LEVELS="18000 20000"
#   RVOIP_SHARDING_UDP_WORKERS="4"
#   RVOIP_SHARDING_TRANSPORT_WORKERS="1 2 4"
#   RVOIP_SHARDING_TRANSACTION_WORKERS="1 2 4 8"
#   RVOIP_SHARDING_DIALOG_WORKERS="1 2 4 8"
#   RVOIP_SHARDING_CAPACITIES="20000 30000"
#   RVOIP_SHARDING_SIP_UDP_RECV_BUFFER_SIZE=8388608
#   RVOIP_SHARDING_SAMPLE=1
#   RVOIP_SHARDING_SAMPLY=1
#   RVOIP_SHARDING_SAMPLY_CSWITCH_MARKERS=1
#
# Notes:
#   macOS may require `samply setup` once before PID attach works.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../../.." && pwd)"
RUN_COMPARISON="$SCRIPT_DIR/run_comparison_dockerized.sh"
ANALYZE="$SCRIPT_DIR/analyze.py"

TARGET_HOST="${1:-host.docker.internal}"
SIPP_IMAGE="${SIPP_IMAGE:-local-sipp}"
NETWORK="${SIPP_NETWORK:-asterisk_asterisk-local}"

resolve_advertised_addr() {
  local host="$1"
  if ! command -v docker >/dev/null 2>&1; then
    return 1
  fi
  docker run --rm \
    --network "$NETWORK" \
    --entrypoint /bin/sh \
    "$SIPP_IMAGE" \
    -c "getent hosts '$host' 2>/dev/null | awk 'NR == 1 { print \\$1 }'" \
    2>/dev/null
}

if [[ $# -ge 2 ]]; then
  ADVERTISED_ADDR="$2"
elif [[ -n "${RVOIP_SHARDING_ADVERTISED_ADDR:-}" ]]; then
  ADVERTISED_ADDR="$RVOIP_SHARDING_ADVERTISED_ADDR"
else
  ADVERTISED_ADDR="$(resolve_advertised_addr "$TARGET_HOST" | head -n 1 || true)"
  if [[ -z "$ADVERTISED_ADDR" ]]; then
    ADVERTISED_ADDR="$TARGET_HOST"
  fi
fi
BASE_PORT="${3:-${RVOIP_SHARDING_BASE_PORT:-39460}}"

LISTENER_BIN="${RVOIP_PERF_LISTENER_BIN:-$REPO_ROOT/target/release/examples/perf_listener}"
RESULTS_DIR="${RVOIP_SHARDING_RESULTS:-$SCRIPT_DIR/results/signaling_sharding_matrix_$(date +%Y%m%d_%H%M%S)}"

CPS_LEVELS="${RVOIP_SHARDING_CPS_LEVELS:-18000}"
UDP_WORKERS="${RVOIP_SHARDING_UDP_WORKERS:-4}"
TRANSPORT_WORKERS="${RVOIP_SHARDING_TRANSPORT_WORKERS:-1}"
TRANSACTION_WORKERS="${RVOIP_SHARDING_TRANSACTION_WORKERS:-1 2 4 8}"
DIALOG_WORKERS="${RVOIP_SHARDING_DIALOG_WORKERS:-1}"
CAPACITIES="${RVOIP_SHARDING_CAPACITIES:-20000}"
STEADY_SECS="${RVOIP_SHARDING_STEADY_SECS:-15}"
SIPP_SHARD_CPS="${RVOIP_SHARDING_SIPP_SHARD_CPS:-1000}"
LISTENER_WARMUP_SECS="${RVOIP_SHARDING_LISTENER_WARMUP_SECS:-2}"
TRANSACTION_TIMING="${RVOIP_SHARDING_TRANSACTION_TIMING:-0}"
DIALOG_TIMING="${RVOIP_SHARDING_DIALOG_TIMING:-0}"
BUILD_LISTENER="${RVOIP_SHARDING_BUILD:-auto}"
SAMPLE_ENABLED="${RVOIP_SHARDING_SAMPLE:-0}"
SAMPLE_SECONDS="${RVOIP_SHARDING_SAMPLE_SECONDS:-$STEADY_SECS}"
SAMPLE_INTERVAL_MS="${RVOIP_SHARDING_SAMPLE_INTERVAL_MS:-}"
SAMPLY_ENABLED="${RVOIP_SHARDING_SAMPLY:-0}"
SAMPLY_SECONDS="${RVOIP_SHARDING_SAMPLY_SECONDS:-$STEADY_SECS}"
SAMPLY_RATE="${RVOIP_SHARDING_SAMPLY_RATE:-1000}"
SAMPLY_CSWITCH_MARKERS="${RVOIP_SHARDING_SAMPLY_CSWITCH_MARKERS:-0}"
SAMPLY_EXTRA_ARGS="${RVOIP_SHARDING_SAMPLY_EXTRA_ARGS:-}"

mkdir -p "$RESULTS_DIR"

if [[ "$BUILD_LISTENER" == "1" || "$BUILD_LISTENER" == "true" || ! -x "$LISTENER_BIN" ]]; then
  echo "[signaling_sharding_matrix] building release perf_listener"
  cargo build -p rvoip-sip --release --example perf_listener
fi

if [[ ! -x "$LISTENER_BIN" ]]; then
  echo "[signaling_sharding_matrix] listener binary not executable: $LISTENER_BIN" >&2
  exit 1
fi

if [[ ! -x "$RUN_COMPARISON" ]]; then
  echo "[signaling_sharding_matrix] missing SIPp runner: $RUN_COMPARISON" >&2
  exit 1
fi

append_optional_capacity_arg() {
  local env_name="$1"
  local flag="$2"
  local value="${!env_name:-}"
  if [[ -n "$value" ]]; then
    listener_args+=("$flag" "$value")
  fi
}

CURRENT_LISTENER_PID=""
CURRENT_SAMPLE_PID=""
CURRENT_SAMPLY_PID=""
CURRENT_SAMPLY_STOPPER_PID=""

stop_profiler_pid() {
  local profiler_pid="$1"
  local profiler_name="$2"
  if [[ -z "$profiler_pid" ]]; then
    return
  fi

  if kill -0 "$profiler_pid" >/dev/null 2>&1; then
    echo "[signaling_sharding_matrix] stopping $profiler_name profiler" | tee -a "$RESULTS_DIR/matrix_metadata.txt"
    kill -INT "$profiler_pid" >/dev/null 2>&1 || true
    (
      sleep 4
      kill -TERM "$profiler_pid" >/dev/null 2>&1 || true
    ) &
    local terminator_pid="$!"
    wait "$profiler_pid" >/dev/null 2>&1 || true
    kill "$terminator_pid" >/dev/null 2>&1 || true
    wait "$terminator_pid" >/dev/null 2>&1 || true
    return
  fi

  wait "$profiler_pid" >/dev/null 2>&1 || true
}

stop_listener() {
  if [[ -n "$CURRENT_SAMPLE_PID" ]]; then
    stop_profiler_pid "$CURRENT_SAMPLE_PID" "sample"
    CURRENT_SAMPLE_PID=""
  fi
  if [[ -n "$CURRENT_SAMPLY_PID" ]]; then
    stop_profiler_pid "$CURRENT_SAMPLY_PID" "samply"
    CURRENT_SAMPLY_PID=""
  fi
  if [[ -n "$CURRENT_SAMPLY_STOPPER_PID" ]]; then
    kill "$CURRENT_SAMPLY_STOPPER_PID" >/dev/null 2>&1 || true
    wait "$CURRENT_SAMPLY_STOPPER_PID" >/dev/null 2>&1 || true
    CURRENT_SAMPLY_STOPPER_PID=""
  fi
  if [[ -n "$CURRENT_LISTENER_PID" ]] && kill -0 "$CURRENT_LISTENER_PID" >/dev/null 2>&1; then
    kill -INT "$CURRENT_LISTENER_PID" >/dev/null 2>&1 || true
    for _ in {1..30}; do
      if ! kill -0 "$CURRENT_LISTENER_PID" >/dev/null 2>&1; then
        CURRENT_LISTENER_PID=""
        return
      fi
      sleep 0.2
    done
    kill -TERM "$CURRENT_LISTENER_PID" >/dev/null 2>&1 || true
    wait "$CURRENT_LISTENER_PID" >/dev/null 2>&1 || true
    CURRENT_LISTENER_PID=""
  fi
}

start_sample() {
  local sample_log="$1"
  if [[ "$SAMPLE_ENABLED" != "1" && "$SAMPLE_ENABLED" != "true" ]]; then
    return
  fi
  if ! command -v sample >/dev/null 2>&1; then
    echo "[signaling_sharding_matrix] sample requested but macOS sample is unavailable" | tee -a "$RESULTS_DIR/matrix_metadata.txt"
    return
  fi

  local sample_args=("$CURRENT_LISTENER_PID" "$SAMPLE_SECONDS" -file "$sample_log")
  if [[ -n "$SAMPLE_INTERVAL_MS" ]]; then
    sample_args+=(-i "$SAMPLE_INTERVAL_MS")
  fi
  echo "[signaling_sharding_matrix] sample output: $sample_log" | tee -a "$RESULTS_DIR/matrix_metadata.txt"
  sample "${sample_args[@]}" >/dev/null 2>&1 &
  CURRENT_SAMPLE_PID="$!"
}

start_samply() {
  local profile_path="$1"
  local samply_log="$2"
  local profile_name="$3"
  if [[ "$SAMPLY_ENABLED" != "1" && "$SAMPLY_ENABLED" != "true" ]]; then
    return
  fi
  if ! command -v samply >/dev/null 2>&1; then
    echo "[signaling_sharding_matrix] samply requested but samply is unavailable" | tee -a "$RESULTS_DIR/matrix_metadata.txt"
    return
  fi

  local samply_args=(
    record
    --save-only
    --pid "$CURRENT_LISTENER_PID"
    --rate "$SAMPLY_RATE"
    --output "$profile_path"
    --profile-name "$profile_name"
  )
  if [[ "$SAMPLY_CSWITCH_MARKERS" == "1" || "$SAMPLY_CSWITCH_MARKERS" == "true" ]]; then
    samply_args+=(--cswitch-markers)
  fi
  if [[ -n "$SAMPLY_EXTRA_ARGS" ]]; then
    # shellcheck disable=SC2206
    local extra_samply_args=($SAMPLY_EXTRA_ARGS)
    samply_args+=("${extra_samply_args[@]}")
  fi

  echo "[signaling_sharding_matrix] samply profile: $profile_path (log: $samply_log)" | tee -a "$RESULTS_DIR/matrix_metadata.txt"
  samply "${samply_args[@]}" > "$samply_log" 2>&1 &
  CURRENT_SAMPLY_PID="$!"
  local samply_pid="$CURRENT_SAMPLY_PID"
  if [[ -n "$SAMPLY_SECONDS" && "$SAMPLY_SECONDS" != "0" ]]; then
    (
      sleep "$SAMPLY_SECONDS"
      kill -INT "$samply_pid" >/dev/null 2>&1 || true
    ) &
    CURRENT_SAMPLY_STOPPER_PID="$!"
  fi
}

cleanup() {
  stop_listener
}
trap cleanup EXIT
trap 'cleanup; exit 130' INT
trap 'cleanup; exit 143' TERM

{
  echo "target_host=$TARGET_HOST"
  echo "advertised_addr=$ADVERTISED_ADDR"
  echo "base_port=$BASE_PORT"
  echo "cps_levels=$CPS_LEVELS"
  echo "udp_workers=$UDP_WORKERS"
  echo "transport_workers=$TRANSPORT_WORKERS"
  echo "transaction_workers=$TRANSACTION_WORKERS"
  echo "dialog_workers=$DIALOG_WORKERS"
  echo "capacities=$CAPACITIES"
  echo "steady_secs=$STEADY_SECS"
  echo "sipp_shard_cps=$SIPP_SHARD_CPS"
  echo "transaction_timing=$TRANSACTION_TIMING"
  echo "dialog_timing=$DIALOG_TIMING"
  echo "sample_enabled=$SAMPLE_ENABLED"
  echo "sample_seconds=$SAMPLE_SECONDS"
  echo "sample_interval_ms=$SAMPLE_INTERVAL_MS"
  echo "samply_enabled=$SAMPLY_ENABLED"
  echo "samply_seconds=$SAMPLY_SECONDS"
  echo "samply_rate=$SAMPLY_RATE"
  echo "samply_cswitch_markers=$SAMPLY_CSWITCH_MARKERS"
  echo "samply_extra_args=$SAMPLY_EXTRA_ARGS"
  echo "listener_bin=$LISTENER_BIN"
} > "$RESULTS_DIR/matrix_metadata.txt"

echo "[signaling_sharding_matrix] results=$RESULTS_DIR"
echo "[signaling_sharding_matrix] target=$TARGET_HOST advertised=$ADVERTISED_ADDR base_port=$BASE_PORT"
echo "[signaling_sharding_matrix] cps=[$CPS_LEVELS] udp=[$UDP_WORKERS] transport=[$TRANSPORT_WORKERS] tx=[$TRANSACTION_WORKERS] dialog=[$DIALOG_WORKERS] capacities=[$CAPACITIES]"

run_index=0
for capacity in $CAPACITIES; do
  for cps in $CPS_LEVELS; do
    for udp_workers in $UDP_WORKERS; do
      for transport_workers in $TRANSPORT_WORKERS; do
        for tx_workers in $TRANSACTION_WORKERS; do
          for dialog_workers in $DIALOG_WORKERS; do
          port=$(( BASE_PORT + run_index ))
          tag="sig_cap${capacity}_udp${udp_workers}_tp${transport_workers}_tx${tx_workers}_dlg${dialog_workers}"
          listener_log="$RESULTS_DIR/${tag}_${cps}cps_listener.log"
          listener_args=(
            "$port"
            "$ADVERTISED_ADDR"
            --fast-auto-accept
            --diagnostics
            --signaling-only-media
            --high-cps-capacity "$capacity"
            --udp-parse-workers "$udp_workers"
            --udp-parse-round-robin
          )

          if [[ "$transport_workers" -gt 1 ]]; then
            listener_args+=(--sip-transport-dispatch-workers "$transport_workers")
          fi
          if [[ "$tx_workers" -gt 1 ]]; then
            listener_args+=(--transaction-dispatch-workers "$tx_workers")
          fi
          if [[ "$dialog_workers" -gt 1 ]]; then
            listener_args+=(--sip-dialog-dispatch-workers "$dialog_workers")
          fi
          if [[ "$TRANSACTION_TIMING" == "1" || "$TRANSACTION_TIMING" == "true" ]]; then
            listener_args+=(--transaction-timing-diagnostics)
          fi
          if [[ "$DIALOG_TIMING" == "1" || "$DIALOG_TIMING" == "true" ]]; then
            listener_args+=(--dialog-timing-diagnostics)
          fi

          append_optional_capacity_arg RVOIP_SHARDING_UDP_QUEUE_CAPACITY --udp-parse-queue-capacity
          append_optional_capacity_arg RVOIP_SHARDING_SIP_TRANSPORT_CHANNEL_CAPACITY --sip-transport-channel-capacity
          append_optional_capacity_arg RVOIP_SHARDING_SIP_TRANSPORT_DISPATCH_QUEUE_CAPACITY --sip-transport-dispatch-queue-capacity
          append_optional_capacity_arg RVOIP_SHARDING_SIP_UDP_RECV_BUFFER_SIZE --sip-udp-recv-buffer-size
          append_optional_capacity_arg RVOIP_SHARDING_SIP_UDP_SEND_BUFFER_SIZE --sip-udp-send-buffer-size
          append_optional_capacity_arg RVOIP_SHARDING_TRANSACTION_EVENT_CHANNEL_CAPACITY --transaction-event-channel-capacity
          append_optional_capacity_arg RVOIP_SHARDING_TRANSACTION_DISPATCH_QUEUE_CAPACITY --transaction-dispatch-queue-capacity
          append_optional_capacity_arg RVOIP_SHARDING_DIALOG_DISPATCH_QUEUE_CAPACITY --sip-dialog-dispatch-queue-capacity
          append_optional_capacity_arg RVOIP_SHARDING_SESSION_EVENT_WORKERS --session-event-dispatcher-workers
          append_optional_capacity_arg RVOIP_SHARDING_SESSION_EVENT_QUEUE_CAPACITY --session-event-dispatcher-queue-capacity

          if [[ -n "${RVOIP_SHARDING_EXTRA_LISTENER_ARGS:-}" ]]; then
            # shellcheck disable=SC2206
            extra_listener_args=($RVOIP_SHARDING_EXTRA_LISTENER_ARGS)
            listener_args+=("${extra_listener_args[@]}")
          fi

          echo "[signaling_sharding_matrix] === $tag @ ${cps} CPS on port $port ==="
          echo "[signaling_sharding_matrix] listener args: ${listener_args[*]}" | tee -a "$RESULTS_DIR/matrix_metadata.txt"
          "$LISTENER_BIN" "${listener_args[@]}" > "$listener_log" 2>&1 &
          CURRENT_LISTENER_PID="$!"
          sleep "$LISTENER_WARMUP_SECS"
          if ! kill -0 "$CURRENT_LISTENER_PID" >/dev/null 2>&1; then
            echo "[signaling_sharding_matrix] listener exited early; see $listener_log" >&2
            wait "$CURRENT_LISTENER_PID" || true
            CURRENT_LISTENER_PID=""
            exit 1
          fi

          sample_log="$RESULTS_DIR/${tag}_${cps}cps_sample.txt"
          start_sample "$sample_log"
          samply_profile="$RESULTS_DIR/${tag}_${cps}cps_samply_profile.json.gz"
          samply_log="$RESULTS_DIR/${tag}_${cps}cps_samply.log"
          start_samply "$samply_profile" "$samply_log" "${tag}_${cps}cps"

          RVOIP_PERF_RESULTS="$RESULTS_DIR" \
          RVOIP_PERF_CPS="$cps" \
          RVOIP_PERF_STEADY_SECS="$STEADY_SECS" \
          RVOIP_PERF_SIPP_SHARD_CPS="$SIPP_SHARD_CPS" \
            "$RUN_COMPARISON" "$TARGET_HOST" "$port" "$tag"

          stop_listener
          run_index=$(( run_index + 1 ))
          sleep 1
          done
        done
      done
    done
  done
done

if [[ -x "$ANALYZE" ]]; then
  "$ANALYZE" "$RESULTS_DIR" "$RESULTS_DIR/summary.md"
  echo "[signaling_sharding_matrix] summary=$RESULTS_DIR/summary.md"
fi

echo "[signaling_sharding_matrix] done"
