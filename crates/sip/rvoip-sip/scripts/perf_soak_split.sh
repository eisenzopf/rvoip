#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/../../../.." && pwd)"
CRATE_DIR="${WORKSPACE_ROOT}/crates/sip/rvoip-sip"
PERF_DIR="${WORKSPACE_ROOT}/target/perf-results"
CARGO_ARTIFACT_HELPER="${SCRIPT_DIR}/perf_cargo_artifact.py"

export CARGO_MANIFEST_DIR="${CRATE_DIR}"

: "${RVOIP_PERF_SOAK_BOB_PORT:=25060}"
: "${RVOIP_PERF_SOAK_ALICE_PORT:=25062}"
: "${RVOIP_PERF_CALL_TIMEOUT_SECS:=30}"
: "${RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS:=120}"
: "${RVOIP_PERF_EXTERNAL_RESOURCE_SAMPLER:=0}"
: "${RVOIP_PERF_PROFILE_RESOURCE_INTERVAL_SECS:=5}"
: "${RVOIP_PERF_MEMORY_DIAGNOSTICS:=0}"
: "${RVOIP_PERF_ALLOCATOR_DIAGNOSTICS:=0}"
: "${RVOIP_PERF_MEMORY_DIAG_INTERVAL_SECS:=5}"
: "${RVOIP_PERF_MIMALLOC_COLLECT_AT:=off}"
: "${RVOIP_PERF_SYSTEM_ALLOCATOR:=0}"
: "${RVOIP_PERF_DHAT:=0}"
: "${RVOIP_PERF_HEAP_SNAPSHOTS:=0}"
: "${RVOIP_PERF_HEAP_SNAPSHOT_SECS:=}"
: "${RVOIP_PERF_LEAKS_SNAPSHOTS:=0}"
: "${RVOIP_PERF_MALLOC_STACK_LOGGING:=0}"
export RVOIP_PERF_CALL_TIMEOUT_SECS
export RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS
export RVOIP_PERF_MEMORY_DIAGNOSTICS
export RVOIP_PERF_ALLOCATOR_DIAGNOSTICS
export RVOIP_PERF_MEMORY_DIAG_INTERVAL_SECS
export RVOIP_PERF_MIMALLOC_COLLECT_AT
export RVOIP_PERF_DHAT

mkdir -p "${PERF_DIR}"
cd "${WORKSPACE_ROOT}"

append_perf_feature() {
  local feature="$1"
  case ",${PERF_FEATURES}," in
    *,"${feature}",*) ;;
    *) PERF_FEATURES="${PERF_FEATURES},${feature}" ;;
  esac
}

PERF_FEATURES="${RVOIP_PERF_FEATURES:-perf-tests}"
if [[ "${RVOIP_PERF_MEMORY_DIAGNOSTICS}" == "1" || "${RVOIP_PERF_ALLOCATOR_DIAGNOSTICS}" == "1" ]]; then
  append_perf_feature "perf-infra-memory-diagnostics"
fi
if [[ "${RVOIP_PERF_MEDIA_DIAGNOSTICS:-0}" == "1" ]]; then
  append_perf_feature "perf-media-diagnostics"
fi
if [[ "${RVOIP_PERF_MEDIA_MEMORY_DIAGNOSTICS:-0}" == "1" ]]; then
  append_perf_feature "perf-media-memory-diagnostics"
fi
if [[ "${RVOIP_PERF_RTP_MEMORY_DIAGNOSTICS:-0}" == "1" ]]; then
  append_perf_feature "perf-rtp-memory-diagnostics"
fi
if [[ "${RVOIP_PERF_DHAT}" == "1" ]]; then
  if [[ "${RVOIP_PERF_SYSTEM_ALLOCATOR}" == "1" ]]; then
    echo "RVOIP_PERF_DHAT=1 uses DHAT's allocator; ignoring RVOIP_PERF_SYSTEM_ALLOCATOR=1" >&2
  fi
  append_perf_feature "dhat"
elif [[ "${RVOIP_PERF_SYSTEM_ALLOCATOR}" == "1" ]]; then
  append_perf_feature "perf-system-allocator"
fi

RUN_DIR="${PERF_DIR}/perf_soak_split_$(date +%Y%m%d_%H%M%S)_$$"
BUILD_DIR="${RUN_DIR}/build"
SOURCE_AT_BUILD="${BUILD_DIR}/source-at-build.json"
SOURCE_AFTER_BUILD="${BUILD_DIR}/source-after-build.json"
SOURCE_AT_FINALIZE="${BUILD_DIR}/source-at-finalize.json"
mkdir -p "${BUILD_DIR}"

python3 "${CARGO_ARTIFACT_HELPER}" capture-source \
  --workspace-root "${WORKSPACE_ROOT}" \
  --output "${SOURCE_AT_BUILD}" >/dev/null

build_exact_test_bin() {
  local name="$1"
  local messages="${BUILD_DIR}/${name}-cargo-messages.jsonl"
  local manifest="${BUILD_DIR}/${name}-artifact.json"
  local target_source="${CRATE_DIR}/tests/perf/${name}.rs"

  echo "Building exact ${name} artifact (features: ${PERF_FEATURES})..." >&2
  if ! cargo test \
      -p rvoip-sip \
      --release \
      --features "${PERF_FEATURES}" \
      --test "${name}" \
      --no-run \
      --message-format=json-render-diagnostics \
      >"${messages}"; then
    echo "Cargo failed while building ${name}; refusing any existing binary" >&2
    return 1
  fi

  python3 "${CARGO_ARTIFACT_HELPER}" resolve \
    --messages "${messages}" \
    --manifest "${manifest}" \
    --workspace-root "${WORKSPACE_ROOT}" \
    --source-at-build "${SOURCE_AT_BUILD}" \
    --target "${name}" \
    --target-source "${target_source}" \
    --package rvoip-sip \
    --profile release \
    --features "${PERF_FEATURES}" \
    --default-features enabled
}

RECEIVER_BIN="$(build_exact_test_bin perf_soak_receiver)"
CALLER_BIN="$(build_exact_test_bin perf_soak_caller)"
python3 "${CARGO_ARTIFACT_HELPER}" capture-source \
  --workspace-root "${WORKSPACE_ROOT}" \
  --output "${SOURCE_AFTER_BUILD}" >/dev/null
python3 "${CARGO_ARTIFACT_HELPER}" assert-source \
  --expected "${SOURCE_AT_BUILD}" \
  --actual "${SOURCE_AFTER_BUILD}" \
  --label "while building split-soak executables" >/dev/null
printf 'receiver=%s\ncaller=%s\n' "${RECEIVER_BIN}" "${CALLER_BIN}" \
  >"${BUILD_DIR}/executables.txt"

start_ps_sampler() {
  local role="$1"
  local pid="$2"
  local out_dir="$3"
  local out_file="${out_dir}/${role}_external_resource_samples_${pid}.jsonl"

  (
    mkdir -p "${out_dir}"
    local started
    started="$(date +%s)"
    while kill -0 "${pid}" 2>/dev/null; do
      local rss_kb cpu_pct timestamp elapsed rss_mb
      timestamp="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
      elapsed=$(($(date +%s) - started))
      read -r rss_kb cpu_pct < <(ps -o rss= -o %cpu= -p "${pid}" | awk 'NR == 1 { print $1, $2 }')
      if [[ -n "${rss_kb:-}" ]]; then
        rss_mb="$(awk -v rss="${rss_kb}" 'BEGIN { printf "%.3f", rss / 1024.0 }')"
        printf '{"timestamp":"%s","t_secs":%s,"pid":%s,"rss_mb":%s,"cpu_pct":%s}\n' \
          "${timestamp}" "${elapsed}" "${pid}" "${rss_mb}" "${cpu_pct:-0}" >> "${out_file}"
      fi
      sleep "${RVOIP_PERF_PROFILE_RESOURCE_INTERVAL_SECS}"
    done
  ) >/dev/null 2>&1 &
  echo $!
}

total_soak_duration_secs() {
  if [[ -n "${RVOIP_PERF_SOAK_DURATION_SECS:-}" ]]; then
    echo "${RVOIP_PERF_SOAK_DURATION_SECS}"
    return
  fi

  if [[ -n "${RVOIP_PERF_SOAK_ACTIVE_CALL_PHASES:-}" ]]; then
    awk -F'[:,]' '
      {
        total = 0
        for (i = 2; i <= NF; i += 2) {
          total += $i
        }
        print total
      }
    ' <<< "${RVOIP_PERF_SOAK_ACTIVE_CALL_PHASES}"
    return
  fi

  echo 1800
}

heap_snapshot_schedule() {
  if [[ -n "${RVOIP_PERF_HEAP_SNAPSHOT_SECS}" ]]; then
    local index=1
    local entry
    IFS=',' read -r -a entries <<< "${RVOIP_PERF_HEAP_SNAPSHOT_SECS}"
    for entry in "${entries[@]}"; do
      entry="${entry//[[:space:]]/}"
      if [[ -z "${entry}" ]]; then
        continue
      fi
      if [[ "${entry}" == *:* ]]; then
        echo "${entry}"
      else
        echo "sample${index}:${entry}"
      fi
      index=$((index + 1))
    done
    return
  fi

  local total drain high low post
  total="$(total_soak_duration_secs)"
  drain="${RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS}"
  if [[ -n "${RVOIP_PERF_SOAK_ACTIVE_CALL_PHASES:-}" ]]; then
    local first_duration second_duration
    first_duration="$(awk -F'[:,]' '{ print $2 }' <<< "${RVOIP_PERF_SOAK_ACTIVE_CALL_PHASES}")"
    second_duration="$(awk -F'[:,]' '{ print (NF >= 4 ? $4 : 0) }' <<< "${RVOIP_PERF_SOAK_ACTIVE_CALL_PHASES}")"
    if (( first_duration > 120 )); then
      high=$((first_duration - 60))
    else
      high=$((first_duration / 2))
    fi
    if (( second_duration > 120 )); then
      low=$((first_duration + 60))
    elif (( second_duration > 0 )); then
      low=$((first_duration + second_duration / 2))
    else
      low=$((total / 2))
    fi
  else
    high=$((total / 2))
    low=$((total > 120 ? total - 60 : total / 2))
  fi
  post=$((total + drain / 2))
  echo "high:${high}"
  echo "low:${low}"
  echo "drain:${post}"
}

start_heap_snapshot_sampler() {
  local role="$1"
  local pid="$2"
  local out_dir="$3"
  local manifest="${out_dir}/${role}_heap_snapshots_${pid}.jsonl"

  (
    mkdir -p "${out_dir}"
    local started now wait_for label offset vmmap_file summary_file leaks_file timestamp
    started="$(date +%s)"
    while IFS=: read -r label offset; do
      if [[ -z "${label:-}" || -z "${offset:-}" ]]; then
        continue
      fi
      now="$(date +%s)"
      wait_for=$((started + offset - now))
      if (( wait_for > 0 )); then
        sleep "${wait_for}"
      fi
      if ! kill -0 "${pid}" 2>/dev/null; then
        break
      fi
      timestamp="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
      vmmap_file="${out_dir}/${role}_${label}_vmmap_${pid}.txt"
      summary_file="${out_dir}/${role}_${label}_vmmap_summary_${pid}.txt"
      leaks_file="${out_dir}/${role}_${label}_leaks_${pid}.txt"
      if command -v vmmap >/dev/null 2>&1; then
        vmmap "${pid}" > "${vmmap_file}" 2>&1 || true
        vmmap -summary "${pid}" > "${summary_file}" 2>&1 || true
      fi
      if [[ "${RVOIP_PERF_LEAKS_SNAPSHOTS}" == "1" ]] && command -v leaks >/dev/null 2>&1; then
        leaks "${pid}" > "${leaks_file}" 2>&1 || true
      fi
      printf '{"timestamp":"%s","role":"%s","pid":%s,"label":"%s","offset_secs":%s,"vmmap":"%s","vmmap_summary":"%s","leaks":"%s"}\n' \
        "${timestamp}" "${role}" "${pid}" "${label}" "${offset}" "${vmmap_file}" "${summary_file}" "${leaks_file}" >> "${manifest}"
    done < <(heap_snapshot_schedule)
  ) >/dev/null 2>&1 &
  echo $!
}

enable_malloc_stack_logging_if_requested() {
  if [[ "${RVOIP_PERF_MALLOC_STACK_LOGGING}" == "1" ]]; then
    export MallocStackLogging=1
    export MallocStackLoggingNoCompact=1
  fi
}

READY_FILE="${RUN_DIR}/receiver.ready"
STOP_FILE="${RUN_DIR}/receiver.stop"
if [[ "${RVOIP_PERF_EXTERNAL_RESOURCE_SAMPLER}" == "1" ]]; then
  export RVOIP_PERF_PROFILE_EXTERNAL_RESOURCE_DIR="${RUN_DIR}"
fi

receiver_pid=""
caller_pid=""
receiver_resource_pid=""
caller_resource_pid=""
receiver_heap_pid=""
caller_heap_pid=""

cleanup() {
  touch "${STOP_FILE}" 2>/dev/null || true
  if [[ -n "${caller_pid}" ]] && kill -0 "${caller_pid}" 2>/dev/null; then
    kill -TERM "${caller_pid}" 2>/dev/null || true
  fi
  if [[ -n "${receiver_pid}" ]] && kill -0 "${receiver_pid}" 2>/dev/null; then
    kill -TERM "${receiver_pid}" 2>/dev/null || true
  fi
  if [[ -n "${receiver_resource_pid}" ]] && kill -0 "${receiver_resource_pid}" 2>/dev/null; then
    kill -TERM "${receiver_resource_pid}" 2>/dev/null || true
  fi
  if [[ -n "${caller_resource_pid}" ]] && kill -0 "${caller_resource_pid}" 2>/dev/null; then
    kill -TERM "${caller_resource_pid}" 2>/dev/null || true
  fi
  if [[ -n "${receiver_heap_pid}" ]] && kill -0 "${receiver_heap_pid}" 2>/dev/null; then
    kill -TERM "${receiver_heap_pid}" 2>/dev/null || true
  fi
  if [[ -n "${caller_heap_pid}" ]] && kill -0 "${caller_heap_pid}" 2>/dev/null; then
    kill -TERM "${caller_heap_pid}" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

echo "Starting receiver on SIP port ${RVOIP_PERF_SOAK_BOB_PORT}..."
(
  export RVOIP_PERF_SOAK_BOB_PORT
  export RVOIP_PERF_SOAK_ALICE_PORT
  export RVOIP_PERF_SOAK_READY_FILE="${READY_FILE}"
  export RVOIP_PERF_SOAK_STOP_FILE="${STOP_FILE}"
  export RVOIP_PERF_SOAK_RUN_DIR="${RUN_DIR}"
  enable_malloc_stack_logging_if_requested
  exec "${RECEIVER_BIN}" perf_soak_receiver --ignored --nocapture
) &
receiver_pid=$!

ready_deadline=$((SECONDS + RVOIP_PERF_CALL_TIMEOUT_SECS))
while [[ ! -f "${READY_FILE}" ]]; do
  if ! kill -0 "${receiver_pid}" 2>/dev/null; then
    echo "Receiver exited before becoming ready" >&2
    wait "${receiver_pid}" || true
    exit 1
  fi
  if (( SECONDS >= ready_deadline )); then
    echo "Timed out waiting for receiver readiness file: ${READY_FILE}" >&2
    exit 1
  fi
  sleep 0.1
done
if [[ "${RVOIP_PERF_EXTERNAL_RESOURCE_SAMPLER}" == "1" ]]; then
  receiver_resource_pid="$(start_ps_sampler receiver "${receiver_pid}" "${RUN_DIR}/diagnostics")"
fi
if [[ "${RVOIP_PERF_HEAP_SNAPSHOTS}" == "1" ]]; then
  receiver_heap_pid="$(start_heap_snapshot_sampler receiver "${receiver_pid}" "${RUN_DIR}/diagnostics")"
fi

echo "Starting caller on SIP port ${RVOIP_PERF_SOAK_ALICE_PORT}..."
caller_status=0
(
  export RVOIP_PERF_SOAK_BOB_PORT
  export RVOIP_PERF_SOAK_ALICE_PORT
  export RVOIP_PERF_SOAK_READY_FILE="${READY_FILE}"
  export RVOIP_PERF_SOAK_STOP_FILE="${STOP_FILE}"
  export RVOIP_PERF_SOAK_RUN_DIR="${RUN_DIR}"
  enable_malloc_stack_logging_if_requested
  exec "${CALLER_BIN}" perf_soak_caller --ignored --nocapture
) &
caller_pid=$!
if [[ "${RVOIP_PERF_EXTERNAL_RESOURCE_SAMPLER}" == "1" ]]; then
  caller_resource_pid="$(start_ps_sampler caller "${caller_pid}" "${RUN_DIR}/diagnostics")"
fi
if [[ "${RVOIP_PERF_HEAP_SNAPSHOTS}" == "1" ]]; then
  caller_heap_pid="$(start_heap_snapshot_sampler caller "${caller_pid}" "${RUN_DIR}/diagnostics")"
fi
wait "${caller_pid}" || caller_status=$?
caller_pid=""

touch "${STOP_FILE}"

receiver_status=0
wait "${receiver_pid}" || receiver_status=$?
receiver_pid=""
if [[ -n "${receiver_resource_pid}" ]] && kill -0 "${receiver_resource_pid}" 2>/dev/null; then
  kill -TERM "${receiver_resource_pid}" 2>/dev/null || true
  wait "${receiver_resource_pid}" 2>/dev/null || true
fi
receiver_resource_pid=""
if [[ -n "${caller_resource_pid}" ]] && kill -0 "${caller_resource_pid}" 2>/dev/null; then
  kill -TERM "${caller_resource_pid}" 2>/dev/null || true
  wait "${caller_resource_pid}" 2>/dev/null || true
fi
caller_resource_pid=""
if [[ -n "${receiver_heap_pid}" ]] && kill -0 "${receiver_heap_pid}" 2>/dev/null; then
  kill -TERM "${receiver_heap_pid}" 2>/dev/null || true
  wait "${receiver_heap_pid}" 2>/dev/null || true
fi
receiver_heap_pid=""
if [[ -n "${caller_heap_pid}" ]] && kill -0 "${caller_heap_pid}" 2>/dev/null; then
  kill -TERM "${caller_heap_pid}" 2>/dev/null || true
  wait "${caller_heap_pid}" 2>/dev/null || true
fi
caller_heap_pid=""
trap - EXIT INT TERM

python3 "${CARGO_ARTIFACT_HELPER}" capture-source \
  --workspace-root "${WORKSPACE_ROOT}" \
  --output "${SOURCE_AT_FINALIZE}" >/dev/null
python3 "${CARGO_ARTIFACT_HELPER}" assert-source \
  --expected "${SOURCE_AT_BUILD}" \
  --actual "${SOURCE_AT_FINALIZE}" \
  --label "during the split-soak run" >/dev/null

echo "Split soak reports:"
if [[ "${RVOIP_PERF_DHAT}" == "1" ]]; then
  echo "  allocator: dhat"
elif [[ "${RVOIP_PERF_SYSTEM_ALLOCATOR}" == "1" ]]; then
  echo "  allocator: system"
else
  echo "  allocator: mimalloc"
fi
echo "  caller  : ${PERF_DIR}/perf_soak_caller.json"
echo "  receiver: ${PERF_DIR}/perf_soak_receiver.json"
if [[ "${RVOIP_PERF_EXTERNAL_RESOURCE_SAMPLER}" == "1" ]]; then
  echo "  resource JSONL: ${RUN_DIR}/diagnostics"
fi
if [[ "${RVOIP_PERF_MEMORY_DIAGNOSTICS}" == "1" ]]; then
  echo "  memory JSONL  : ${RUN_DIR}/diagnostics"
fi
if [[ "${RVOIP_PERF_HEAP_SNAPSHOTS}" == "1" ]]; then
  echo "  heap snapshots: ${RUN_DIR}/diagnostics"
fi
if [[ "${RVOIP_PERF_DHAT}" == "1" ]]; then
  echo "  dhat profiles : ${RUN_DIR}/diagnostics"
fi
echo "  run dir : ${RUN_DIR}"
echo "  build evidence: ${BUILD_DIR}"

if (( caller_status != 0 || receiver_status != 0 )); then
  echo "Split soak failed: caller=${caller_status} receiver=${receiver_status}" >&2
  exit 1
fi
