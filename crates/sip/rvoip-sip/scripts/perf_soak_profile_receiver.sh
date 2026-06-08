#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/../../../.." && pwd)"
CRATE_DIR="${WORKSPACE_ROOT}/crates/sip/rvoip-sip"
CRATES_ROOT="${WORKSPACE_ROOT}/crates"
PERF_DIR="${CRATES_ROOT}/target/perf-results"

export CARGO_MANIFEST_DIR="${CRATE_DIR}"

: "${RVOIP_PERF_SOAK_BOB_PORT:=25060}"
: "${RVOIP_PERF_SOAK_ALICE_PORT:=25062}"
: "${RVOIP_PERF_CALL_TIMEOUT_SECS:=30}"
: "${RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS:=180}"
: "${RVOIP_PERF_PROFILE_VMMAP_INTERVAL_SECS:=60}"
: "${RVOIP_PERF_PROFILE_TRACE_TEMPLATE:=Allocations}"
: "${RVOIP_PERF_PROFILE_TRACE_TIME_LIMIT:=}"
export RVOIP_PERF_CALL_TIMEOUT_SECS
export RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS

mkdir -p "${PERF_DIR}"
cd "${WORKSPACE_ROOT}"

echo "Building split soak test binaries..."
cargo test \
  -p rvoip-sip \
  --release \
  --features perf-tests \
  --test perf_soak_receiver \
  --no-run
cargo test \
  -p rvoip-sip \
  --release \
  --features perf-tests \
  --test perf_soak_caller \
  --no-run

find_test_bin() {
  local name="$1"
  local bins=()
  shopt -s nullglob
  for candidate in "${WORKSPACE_ROOT}"/target/release/deps/"${name}"-*; do
    if [[ -f "${candidate}" && -x "${candidate}" ]]; then
      bins+=("${candidate}")
    fi
  done
  shopt -u nullglob

  if (( ${#bins[@]} == 0 )); then
    echo "Could not locate compiled ${name} test binary" >&2
    return 1
  fi

  ls -t "${bins[@]}" | head -n 1
}

find_target_pid() {
  local name="$1"
  pgrep -f "/target/release/deps/${name}-.* ${name} --ignored --nocapture" | head -n 1
}

start_vmmap_sampler() {
  local role="$1"
  local pid="$2"
  local out_dir="$3"

  (
    mkdir -p "${out_dir}"
    while kill -0 "${pid}" 2>/dev/null; do
      local ts
      ts="$(date -u +%Y%m%dT%H%M%SZ)"
      vmmap -summary "${pid}" > "${out_dir}/${role}_${ts}.vmmap.txt" 2>&1 || true
      sleep "${RVOIP_PERF_PROFILE_VMMAP_INTERVAL_SECS}"
    done
  ) >/dev/null 2>&1 &
  echo $!
}

RECEIVER_BIN="$(find_test_bin perf_soak_receiver)"
CALLER_BIN="$(find_test_bin perf_soak_caller)"

RUN_DIR="${PERF_DIR}/perf_soak_profile_receiver_$(date +%Y%m%d_%H%M%S)_$$"
READY_FILE="${RUN_DIR}/receiver.ready"
STOP_FILE="${RUN_DIR}/receiver.stop"
TRACE_PATH="${RUN_DIR}/receiver_allocations.trace"
VMMAP_DIR="${RUN_DIR}/vmmap"
mkdir -p "${RUN_DIR}" "${VMMAP_DIR}"

receiver_pid=""
xctrace_pid=""
caller_pid=""
receiver_vmmap_pid=""
caller_vmmap_pid=""

cleanup() {
  touch "${STOP_FILE}" 2>/dev/null || true
  if [[ -n "${caller_pid}" ]] && kill -0 "${caller_pid}" 2>/dev/null; then
    kill -TERM "${caller_pid}" 2>/dev/null || true
  fi
  if [[ -n "${receiver_pid}" ]] && kill -0 "${receiver_pid}" 2>/dev/null; then
    kill -TERM "${receiver_pid}" 2>/dev/null || true
  fi
  if [[ -n "${xctrace_pid}" ]] && kill -0 "${xctrace_pid}" 2>/dev/null; then
    kill -TERM "${xctrace_pid}" 2>/dev/null || true
  fi
  if [[ -n "${receiver_vmmap_pid}" ]] && kill -0 "${receiver_vmmap_pid}" 2>/dev/null; then
    kill -TERM "${receiver_vmmap_pid}" 2>/dev/null || true
  fi
  if [[ -n "${caller_vmmap_pid}" ]] && kill -0 "${caller_vmmap_pid}" 2>/dev/null; then
    kill -TERM "${caller_vmmap_pid}" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

echo "Starting receiver on SIP port ${RVOIP_PERF_SOAK_BOB_PORT}..."
(
  export CARGO_MANIFEST_DIR="${CRATE_DIR}"
  export RVOIP_PERF_SOAK_BOB_PORT
  export RVOIP_PERF_SOAK_ALICE_PORT
  export RVOIP_PERF_SOAK_READY_FILE="${READY_FILE}"
  export RVOIP_PERF_SOAK_STOP_FILE="${STOP_FILE}"
  export RVOIP_PERF_SOAK_RUN_DIR="${RUN_DIR}"
  exec "${RECEIVER_BIN}" perf_soak_receiver --ignored --nocapture
) &
receiver_pid=$!

ready_deadline=$((SECONDS + RVOIP_PERF_CALL_TIMEOUT_SECS + 60))
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
  sleep 0.2
done

receiver_target_pid="$(find_target_pid perf_soak_receiver || true)"
if [[ -z "${receiver_target_pid}" ]]; then
  receiver_target_pid="${receiver_pid}"
fi
echo "Receiver target pid: ${receiver_target_pid}"
receiver_vmmap_pid="$(start_vmmap_sampler receiver "${receiver_target_pid}" "${VMMAP_DIR}")"

echo "Attaching xctrace ${RVOIP_PERF_PROFILE_TRACE_TEMPLATE} to receiver pid ${receiver_target_pid}..."
xctrace_cmd=(
  xctrace record
  --quiet
  --no-prompt
  --template "${RVOIP_PERF_PROFILE_TRACE_TEMPLATE}"
  --output "${TRACE_PATH}"
  --attach "${receiver_target_pid}"
)
if [[ -n "${RVOIP_PERF_PROFILE_TRACE_TIME_LIMIT}" ]]; then
  xctrace_cmd+=(--time-limit "${RVOIP_PERF_PROFILE_TRACE_TIME_LIMIT}")
fi
"${xctrace_cmd[@]}" &
xctrace_pid=$!
sleep 3

echo "Starting caller on SIP port ${RVOIP_PERF_SOAK_ALICE_PORT}..."
(
  export CARGO_MANIFEST_DIR="${CRATE_DIR}"
  export RVOIP_PERF_SOAK_BOB_PORT
  export RVOIP_PERF_SOAK_ALICE_PORT
  export RVOIP_PERF_SOAK_READY_FILE="${READY_FILE}"
  export RVOIP_PERF_SOAK_STOP_FILE="${STOP_FILE}"
  export RVOIP_PERF_SOAK_RUN_DIR="${RUN_DIR}"
  exec "${CALLER_BIN}" perf_soak_caller --ignored --nocapture
) &
caller_pid=$!

sleep 1
caller_target_pid="$(find_target_pid perf_soak_caller || true)"
if [[ -n "${caller_target_pid}" ]]; then
  echo "Caller target pid: ${caller_target_pid}"
  caller_vmmap_pid="$(start_vmmap_sampler caller "${caller_target_pid}" "${VMMAP_DIR}")"
else
  echo "Could not identify caller target pid for vmmap sampling" >&2
fi

caller_status=0
wait "${caller_pid}" || caller_status=$?
caller_pid=""

touch "${STOP_FILE}"

receiver_status=0
wait "${receiver_pid}" || receiver_status=$?
receiver_pid=""

xctrace_status=0
wait "${xctrace_pid}" || xctrace_status=$?
xctrace_pid=""

if [[ -n "${receiver_vmmap_pid}" ]] && kill -0 "${receiver_vmmap_pid}" 2>/dev/null; then
  kill -TERM "${receiver_vmmap_pid}" 2>/dev/null || true
  wait "${receiver_vmmap_pid}" 2>/dev/null || true
fi
receiver_vmmap_pid=""
if [[ -n "${caller_vmmap_pid}" ]] && kill -0 "${caller_vmmap_pid}" 2>/dev/null; then
  kill -TERM "${caller_vmmap_pid}" 2>/dev/null || true
  wait "${caller_vmmap_pid}" 2>/dev/null || true
fi
caller_vmmap_pid=""
trap - EXIT INT TERM

echo "Receiver profile artifacts:"
echo "  caller report  : ${PERF_DIR}/perf_soak_caller.json"
echo "  receiver report: ${PERF_DIR}/perf_soak_receiver.json"
echo "  trace          : ${TRACE_PATH}"
echo "  vmmap dir      : ${VMMAP_DIR}"
echo "  run dir        : ${RUN_DIR}"

if (( caller_status != 0 || receiver_status != 0 || xctrace_status != 0 )); then
  echo "Receiver profile run failed: caller=${caller_status} receiver=${receiver_status} xctrace=${xctrace_status}" >&2
  exit 1
fi
