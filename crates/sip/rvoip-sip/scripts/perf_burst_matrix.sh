#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/../../../.." && pwd)"
CRATE_DIR="${WORKSPACE_ROOT}/crates/sip/rvoip-sip"
CRATES_ROOT="${WORKSPACE_ROOT}/crates"
PERF_DIR="${CRATES_ROOT}/target/perf-results"

export CARGO_MANIFEST_DIR="${CRATE_DIR}"

: "${RVOIP_PERF_BURST_BOB_PORT:=26060}"
: "${RVOIP_PERF_BURST_ALICE_PORT:=26062}"
: "${RVOIP_PERF_BURST_SCENARIO_FILE:=${CRATE_DIR}/config/perf-burst-scenarios.yaml}"
: "${RVOIP_PERF_BURST_SCENARIOS:=carrier-smoke}"
: "${RVOIP_PERF_CALL_TIMEOUT_SECS:=30}"
: "${RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS:=20}"
: "${RVOIP_PERF_MEMORY_DIAGNOSTICS:=0}"
: "${RVOIP_PERF_ALLOCATOR_DIAGNOSTICS:=0}"
: "${RVOIP_PERF_MEMORY_DIAG_INTERVAL_SECS:=5}"
: "${RVOIP_PERF_MIMALLOC_COLLECT_AT:=off}"
: "${RVOIP_PERF_SYSTEM_ALLOCATOR:=0}"
: "${RVOIP_PERF_DHAT:=0}"

export RVOIP_PERF_BURST_SCENARIO_FILE
export RVOIP_PERF_CALL_TIMEOUT_SECS
export RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS
export RVOIP_PERF_MEMORY_DIAGNOSTICS
export RVOIP_PERF_ALLOCATOR_DIAGNOSTICS
export RVOIP_PERF_MEMORY_DIAG_INTERVAL_SECS
export RVOIP_PERF_MIMALLOC_COLLECT_AT
export RVOIP_PERF_DHAT

mkdir -p "${PERF_DIR}"
cd "${WORKSPACE_ROOT}"

PERF_FEATURES="perf-tests"
if [[ "${RVOIP_PERF_DHAT}" == "1" ]]; then
  if [[ "${RVOIP_PERF_SYSTEM_ALLOCATOR}" == "1" ]]; then
    echo "RVOIP_PERF_DHAT=1 uses DHAT's allocator; ignoring RVOIP_PERF_SYSTEM_ALLOCATOR=1" >&2
  fi
  PERF_FEATURES="perf-tests,dhat"
elif [[ "${RVOIP_PERF_SYSTEM_ALLOCATOR}" == "1" ]]; then
  PERF_FEATURES="perf-tests,perf-system-allocator"
fi

echo "Building burst test binaries (features: ${PERF_FEATURES})..."
cargo test \
  -p rvoip-sip \
  --release \
  --features "${PERF_FEATURES}" \
  --test perf_burst_receiver \
  --no-run
cargo test \
  -p rvoip-sip \
  --release \
  --features "${PERF_FEATURES}" \
  --test perf_burst_caller \
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

normalise_scenarios() {
  local raw="$1"
  if [[ "${raw}" == "all" ]]; then
    echo "carrier-smoke access-edge-microburst contact-center-flash shift-change-long-hold overload-recovery high-density-media-burst"
  else
    echo "${raw//,/ }"
  fi
}

capture_host_udp_stats() {
  local path="$1"
  {
    echo "timestamp_epoch=$(date +%s)"
    echo "command=netstat -s -p udp"
    echo
    echo "[parsed]"
    if command -v netstat >/dev/null 2>&1; then
      netstat -s -p udp 2>/dev/null | awk '
        {
          value = $1
          if (value !~ /^[0-9]+$/) {
            next
          }
          $1 = ""
          sub(/^[ \t]+/, "")
          if ($0 == "datagrams received") {
            print "udp_datagrams_received=" value
          } else if ($0 == "dropped due to no socket") {
            print "udp_dropped_no_socket=" value
          } else if ($0 == "dropped due to full socket buffers") {
            print "udp_dropped_full_socket_buffers=" value
          } else if ($0 == "delivered") {
            print "udp_delivered=" value
          } else if ($0 == "datagram output") {
            print "udp_datagram_output=" value
          } else if ($0 == "open UDP sockets") {
            print "udp_open_sockets=" value
          }
        }
      '
      echo
      echo "[raw]"
      netstat -s -p udp 2>&1 || true
    else
      echo "available=false"
      echo
      echo "[raw]"
      echo "netstat not found"
    fi
  } > "${path}"
}

host_udp_value() {
  local path="$1"
  local key="$2"
  awk -F= -v key="${key}" '$1 == key { print $2; found=1; exit } END { if (!found) print "" }' "${path}"
}

write_host_udp_delta() {
  local before="$1"
  local after="$2"
  local out="$3"
  {
    echo "before=${before}"
    echo "after=${after}"
    for key in \
      udp_datagrams_received \
      udp_dropped_no_socket \
      udp_dropped_full_socket_buffers \
      udp_delivered \
      udp_datagram_output \
      udp_open_sockets; do
      local before_value
      local after_value
      before_value="$(host_udp_value "${before}" "${key}")"
      after_value="$(host_udp_value "${after}" "${key}")"
      echo "${key}_before=${before_value:-n/a}"
      echo "${key}_after=${after_value:-n/a}"
      if [[ "${before_value}" =~ ^[0-9]+$ && "${after_value}" =~ ^[0-9]+$ ]]; then
        echo "${key}_delta=$((after_value - before_value))"
      else
        echo "${key}_delta=n/a"
      fi
    done
  } > "${out}"
}

RECEIVER_BIN="$(find_test_bin perf_burst_receiver)"
CALLER_BIN="$(find_test_bin perf_burst_caller)"
ROOT_RUN_DIR="${PERF_DIR}/perf_burst_matrix/burst_$(date +%Y%m%d_%H%M%S)_$$"
mkdir -p "${ROOT_RUN_DIR}"

receiver_pid=""
caller_pid=""

cleanup() {
  if [[ -n "${caller_pid}" ]] && kill -0 "${caller_pid}" 2>/dev/null; then
    kill -TERM "${caller_pid}" 2>/dev/null || true
  fi
  if [[ -n "${receiver_pid}" ]] && kill -0 "${receiver_pid}" 2>/dev/null; then
    kill -TERM "${receiver_pid}" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

failures=0
for scenario in $(normalise_scenarios "${RVOIP_PERF_BURST_SCENARIOS}"); do
  RUN_DIR="${ROOT_RUN_DIR}/${scenario}"
  READY_FILE="${RUN_DIR}/receiver.ready"
  STOP_FILE="${RUN_DIR}/receiver.stop"
  mkdir -p "${RUN_DIR}"
  rm -f "${READY_FILE}" "${STOP_FILE}"
  HOST_UDP_BEFORE="${RUN_DIR}/host_udp_before.txt"
  HOST_UDP_AFTER="${RUN_DIR}/host_udp_after.txt"
  HOST_UDP_DELTA="${RUN_DIR}/host_udp_delta.txt"
  capture_host_udp_stats "${HOST_UDP_BEFORE}"

  echo "Starting burst receiver for scenario ${scenario} on SIP port ${RVOIP_PERF_BURST_BOB_PORT}..."
  (
    export RVOIP_PERF_BURST_SCENARIO="${scenario}"
    export RVOIP_PERF_BURST_BOB_PORT
    export RVOIP_PERF_BURST_ALICE_PORT
    export RVOIP_PERF_BURST_READY_FILE="${READY_FILE}"
    export RVOIP_PERF_BURST_STOP_FILE="${STOP_FILE}"
    export RVOIP_PERF_BURST_RUN_DIR="${RUN_DIR}"
    export RVOIP_PERF_SOAK_RUN_DIR="${RUN_DIR}"
    exec "${RECEIVER_BIN}" perf_burst_receiver --ignored --nocapture
  ) &
  receiver_pid=$!

  ready_deadline=$((SECONDS + RVOIP_PERF_CALL_TIMEOUT_SECS))
  while [[ ! -f "${READY_FILE}" ]]; do
    if ! kill -0 "${receiver_pid}" 2>/dev/null; then
      echo "Burst receiver exited before becoming ready for ${scenario}" >&2
      wait "${receiver_pid}" || true
      failures=$((failures + 1))
      continue 2
    fi
    if (( SECONDS >= ready_deadline )); then
      echo "Timed out waiting for burst receiver readiness file: ${READY_FILE}" >&2
      failures=$((failures + 1))
      kill -TERM "${receiver_pid}" 2>/dev/null || true
      wait "${receiver_pid}" 2>/dev/null || true
      receiver_pid=""
      continue 2
    fi
    sleep 0.1
  done

  echo "Starting burst caller for scenario ${scenario} on SIP port ${RVOIP_PERF_BURST_ALICE_PORT}..."
  caller_status=0
  (
    export RVOIP_PERF_BURST_SCENARIO="${scenario}"
    export RVOIP_PERF_BURST_BOB_PORT
    export RVOIP_PERF_BURST_ALICE_PORT
    export RVOIP_PERF_BURST_READY_FILE="${READY_FILE}"
    export RVOIP_PERF_BURST_STOP_FILE="${STOP_FILE}"
    export RVOIP_PERF_BURST_RUN_DIR="${RUN_DIR}"
    export RVOIP_PERF_SOAK_RUN_DIR="${RUN_DIR}"
    exec "${CALLER_BIN}" perf_burst_caller --ignored --nocapture
  ) || caller_status=$?
  caller_pid=""

  touch "${STOP_FILE}"

  receiver_status=0
  wait "${receiver_pid}" || receiver_status=$?
  receiver_pid=""
  capture_host_udp_stats "${HOST_UDP_AFTER}"
  write_host_udp_delta "${HOST_UDP_BEFORE}" "${HOST_UDP_AFTER}" "${HOST_UDP_DELTA}"

  if (( caller_status != 0 || receiver_status != 0 )); then
    echo "Burst scenario ${scenario} failed: caller=${caller_status} receiver=${receiver_status}" >&2
    failures=$((failures + 1))
  fi
done

trap - EXIT INT TERM

AGG_MD="${ROOT_RUN_DIR}/_burst.md"
{
  echo "# rvoip-sip Media Burst Matrix"
  echo
  echo "- scenario_file: ${RVOIP_PERF_BURST_SCENARIO_FILE}"
  echo "- scenarios: ${RVOIP_PERF_BURST_SCENARIOS}"
  echo "- run_dir: ${ROOT_RUN_DIR}"
  echo
  echo "## Role Summary"
  echo
  python3 - "${ROOT_RUN_DIR}" <<'PY'
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
print("| Scenario | Caller ASR | Caller overload | Caller retained | Caller RSS MB/hr | Receiver calls | Receiver retained | Receiver audio after drain | Receiver RSS MB/hr |")
print("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |")
for scenario_dir in sorted(p for p in root.iterdir() if p.is_dir()):
    scenario = scenario_dir.name
    caller_file = scenario_dir / f"perf_burst_caller_{scenario}.json"
    receiver_file = scenario_dir / f"perf_burst_receiver_{scenario}.json"

    caller = {}
    receiver = {}
    if caller_file.exists():
        caller = json.loads(caller_file.read_text()).get("results", {})
    if receiver_file.exists():
        receiver = json.loads(receiver_file.read_text()).get("results", {})

    errors = caller.get("errors") or {}

    def fmt(value):
        if value is None:
            return "n/a"
        if isinstance(value, float):
            return f"{value:.4g}"
        return str(value)

    print(
        "| {scenario} | {caller_asr} | {caller_overload} | {caller_retained} | "
        "{caller_rss} | {receiver_calls} | {receiver_retained} | "
        "{receiver_audio} | {receiver_rss} |".format(
            scenario=scenario,
            caller_asr=fmt(caller.get("asr")),
            caller_overload=fmt(errors.get("overload_rejected")),
            caller_retained=fmt(caller.get("retained_objects_after_drain")),
            caller_rss=fmt(caller.get("rss_post_drain_growth_mb_per_hr")),
            receiver_calls=fmt(receiver.get("incoming_calls_observed")),
            receiver_retained=fmt(receiver.get("retained_objects_after_drain")),
            receiver_audio=fmt(receiver.get("bob_active_audio_receivers")),
            receiver_rss=fmt(receiver.get("rss_post_drain_growth_mb_per_hr")),
        )
    )
PY
  echo
  for file in "${ROOT_RUN_DIR}"/*/_burst.md; do
    [[ -f "${file}" ]] || continue
    echo
    cat "${file}"
  done
} > "${AGG_MD}"

echo "Burst matrix reports:"
echo "  run dir : ${ROOT_RUN_DIR}"
echo "  summary : ${AGG_MD}"

if (( failures != 0 )); then
  echo "Burst matrix failed with ${failures} scenario failure(s)" >&2
  exit 1
fi
