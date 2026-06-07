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
: "${RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS:=120}"
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

RECEIVER_BIN="$(find_test_bin perf_soak_receiver)"
CALLER_BIN="$(find_test_bin perf_soak_caller)"

RUN_DIR="${PERF_DIR}/perf_soak_split_$(date +%Y%m%d_%H%M%S)_$$"
READY_FILE="${RUN_DIR}/receiver.ready"
STOP_FILE="${RUN_DIR}/receiver.stop"
mkdir -p "${RUN_DIR}"

receiver_pid=""

cleanup() {
  touch "${STOP_FILE}" 2>/dev/null || true
  if [[ -n "${receiver_pid}" ]] && kill -0 "${receiver_pid}" 2>/dev/null; then
    kill -TERM "${receiver_pid}" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

echo "Starting receiver on SIP port ${RVOIP_PERF_SOAK_BOB_PORT}..."
(
  export RVOIP_PERF_SOAK_BOB_PORT
  export RVOIP_PERF_SOAK_ALICE_PORT
  export RVOIP_PERF_SOAK_READY_FILE="${READY_FILE}"
  export RVOIP_PERF_SOAK_STOP_FILE="${STOP_FILE}"
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

echo "Starting caller on SIP port ${RVOIP_PERF_SOAK_ALICE_PORT}..."
caller_status=0
(
  export RVOIP_PERF_SOAK_BOB_PORT
  export RVOIP_PERF_SOAK_ALICE_PORT
  export RVOIP_PERF_SOAK_READY_FILE="${READY_FILE}"
  export RVOIP_PERF_SOAK_STOP_FILE="${STOP_FILE}"
  exec "${CALLER_BIN}" perf_soak_caller --ignored --nocapture
) || caller_status=$?

touch "${STOP_FILE}"

receiver_status=0
wait "${receiver_pid}" || receiver_status=$?
receiver_pid=""
trap - EXIT INT TERM

echo "Split soak reports:"
echo "  caller  : ${PERF_DIR}/perf_soak_caller.json"
echo "  receiver: ${PERF_DIR}/perf_soak_receiver.json"
echo "  run dir : ${RUN_DIR}"

if (( caller_status != 0 || receiver_status != 0 )); then
  echo "Split soak failed: caller=${caller_status} receiver=${receiver_status}" >&2
  exit 1
fi
