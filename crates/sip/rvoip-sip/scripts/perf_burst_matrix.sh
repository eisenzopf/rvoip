#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/../../../.." && pwd)"
CRATE_DIR="${WORKSPACE_ROOT}/crates/sip/rvoip-sip"
PERF_DIR="${RVOIP_PERF_RESULTS:-${WORKSPACE_ROOT}/target/perf-results}"
CARGO_ARTIFACT_HELPER="${SCRIPT_DIR}/perf_cargo_artifact.py"

export CARGO_MANIFEST_DIR="${CRATE_DIR}"

: "${RVOIP_PERF_BURST_BOB_PORT:=26060}"
: "${RVOIP_PERF_BURST_ALICE_PORT:=26062}"
: "${RVOIP_PERF_BURST_SCENARIO_FILE:=${CRATE_DIR}/config/perf-burst-scenarios.yaml}"
: "${RVOIP_PERF_BURST_SCENARIOS:=carrier-smoke}"
# RFC 3261 Timer B/F can run for 32 seconds. Keep the harness observation
# horizon beyond that protocol deadline so it never manufactures a timeout
# while the transaction layer is still behaving correctly.
: "${RVOIP_PERF_CALL_TIMEOUT_SECS:=40}"
# Burst acceptance first waits through the 90-second SIP-retention horizon and
# proves logical retention is zero. It then leaves a fixed allocator-quiescence
# interval before measuring a separate quiet RSS window. The Rust harness
# clamps shorter drain overrides to the same 160-second retention minimum.
: "${RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS:=160}"
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

ROOT_RUN_DIR="${PERF_DIR}/perf_burst_matrix/burst_$(date +%Y%m%d_%H%M%S)_$$"
BUILD_DIR="${ROOT_RUN_DIR}/build"
SOURCE_AT_BUILD="${BUILD_DIR}/source-at-build.json"
SOURCE_AFTER_BUILD="${BUILD_DIR}/source-after-build.json"
SOURCE_AT_FINALIZE="${BUILD_DIR}/source-at-finalize.json"
mkdir -p "${BUILD_DIR}"

python3 "${CARGO_ARTIFACT_HELPER}" capture-source \
  --workspace-root "${WORKSPACE_ROOT}" \
  --output "${SOURCE_AT_BUILD}" >/dev/null

resolve_exact_test_bin() {
  local name="$1"
  local messages="$2"
  local manifest="${BUILD_DIR}/${name}-artifact.json"
  local target_source="${CRATE_DIR}/tests/perf/${name}.rs"

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
    --build-target perf_burst_receiver \
    --build-target perf_burst_caller \
    --default-features enabled
}

BURST_CARGO_MESSAGES="${BUILD_DIR}/burst-cargo-messages.jsonl"
if [[ -n "${RVOIP_PERF_PREBUILT_MANIFEST:-}" ]]; then
  : "${RVOIP_RELEASE_CANDIDATE:?prebuilt performance bundle requires exact candidate}"
  : "${RVOIP_RELEASE_ENVIRONMENT_ID:?prebuilt performance bundle requires environment ID}"
  echo "Resolving exact burst artifacts from the verified candidate bundle..." >&2
  resolve_prebuilt_test_bin() {
    local name="$1"
    python3 "${WORKSPACE_ROOT}/scripts/release/prebuilt_performance.py" resolve \
      --manifest "${RVOIP_PERF_PREBUILT_MANIFEST}" \
      --workspace "${WORKSPACE_ROOT}" \
      --candidate "${RVOIP_RELEASE_CANDIDATE}" \
      --environment-id "${RVOIP_RELEASE_ENVIRONMENT_ID}" \
      --features "${PERF_FEATURES}" \
      --target "${name}" \
      --artifact-manifest "${BUILD_DIR}/${name}-artifact.json" \
      --source-at-build "${SOURCE_AT_BUILD}" \
      --build-target perf_burst_receiver \
      --build-target perf_burst_caller
  }
  RECEIVER_BIN="$(resolve_prebuilt_test_bin perf_burst_receiver)"
  CALLER_BIN="$(resolve_prebuilt_test_bin perf_burst_caller)"
else
  echo "Building exact burst artifacts together (features: ${PERF_FEATURES})..." >&2
  if ! cargo test \
      -p rvoip-sip \
      --release \
      --features "${PERF_FEATURES}" \
      --test perf_burst_receiver \
      --test perf_burst_caller \
      --no-run \
      --message-format=json-render-diagnostics \
      >"${BURST_CARGO_MESSAGES}"; then
    echo "Cargo failed while building burst artifacts; refusing existing binaries" >&2
    exit 1
  fi
  RECEIVER_BIN="$(resolve_exact_test_bin perf_burst_receiver "${BURST_CARGO_MESSAGES}")"
  CALLER_BIN="$(resolve_exact_test_bin perf_burst_caller "${BURST_CARGO_MESSAGES}")"
fi
python3 "${CARGO_ARTIFACT_HELPER}" capture-source \
  --workspace-root "${WORKSPACE_ROOT}" \
  --output "${SOURCE_AFTER_BUILD}" >/dev/null
python3 "${CARGO_ARTIFACT_HELPER}" assert-source \
  --expected "${SOURCE_AT_BUILD}" \
  --actual "${SOURCE_AFTER_BUILD}" \
  --label "while building burst executables" >/dev/null
printf 'receiver=%s\ncaller=%s\n' "${RECEIVER_BIN}" "${CALLER_BIN}" \
  >"${BUILD_DIR}/executables.txt"

normalise_scenarios() {
  local raw="$1"
  if [[ "${raw}" == "all" ]]; then
    echo "carrier-smoke access-edge-microburst contact-center-flash shift-change-long-hold overload-recovery high-density-media-burst buffer-ab-legacy"
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

  # The caller signals immediately after offered load completes so caller and
  # receiver retention/RSS qualification run concurrently. This is a harmless
  # fallback for older caller binaries and interrupted runs.
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
for scenario_dir in sorted(
    p for p in root.iterdir() if p.is_dir() and p.name != "build"
):
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
echo "  build evidence: ${BUILD_DIR}"

python3 "${CARGO_ARTIFACT_HELPER}" capture-source \
  --workspace-root "${WORKSPACE_ROOT}" \
  --output "${SOURCE_AT_FINALIZE}" >/dev/null
python3 "${CARGO_ARTIFACT_HELPER}" assert-source \
  --expected "${SOURCE_AT_BUILD}" \
  --actual "${SOURCE_AT_FINALIZE}" \
  --label "during the burst-matrix run" >/dev/null

if (( failures != 0 )); then
  echo "Burst matrix failed with ${failures} scenario failure(s)" >&2
  exit 1
fi
