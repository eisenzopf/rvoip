#!/usr/bin/env bash
set -Eeuo pipefail

LOG=/var/log/rvoip-release-qualification.log
exec > >(tee -a "$LOG") 2>&1

metadata() {
  curl --fail --silent --show-error \
    -H 'Metadata-Flavor: Google' \
    "http://metadata.google.internal/computeMetadata/v1/instance/attributes/$1"
}

CANDIDATE="$(metadata rvoip-candidate)"
RUN_ID="$(metadata rvoip-run-id)"
SHARD_ID="$(metadata rvoip-shard-id)"
RESOURCE_CLASS="$(metadata rvoip-resource-class)"
BUCKET="$(metadata rvoip-evidence-bucket)"
CACHE_BUCKET="$(metadata rvoip-cache-bucket)"
PREFIX="$(metadata rvoip-prefix)"
GATES="$(metadata rvoip-gates-b64 | base64 --decode)"
ENVIRONMENT_ID="$(metadata rvoip-environment-b64 | base64 --decode)"
PREBUILT_URI="$(metadata rvoip-prebuilt-uri)"
PREBUILT_SHA256="$(metadata rvoip-prebuilt-sha256)"
EXTERNAL_MEMORY_DIAGNOSTICS="$(
  metadata rvoip-external-memory-diagnostics 2>/dev/null || printf '0'
)"
MIMALLOC_ALLOW_THP_OVERRIDE="$(
  metadata rvoip-mimalloc-allow-thp 2>/dev/null || true
)"
WORKSPACE=/opt/rvoip
EVIDENCE=/tmp/release-shard
ARCHIVE=/tmp/release-shard.tar.gz
RESULT=/tmp/result.json
STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
START_SECONDS="$(date +%s)"
EXTERNAL_MEMORY_SAMPLER_PID=""

upload() {
  local source="$1"
  local object="$2"
  local token encoded
  token="$(curl --fail --silent --show-error \
    -H 'Metadata-Flavor: Google' \
    http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["access_token"])')"
  encoded="$(python3 -c 'import sys,urllib.parse; print(urllib.parse.quote(sys.argv[1], safe=""))' "$object")"
  curl --fail --silent --show-error --retry 3 --retry-all-errors \
    -X POST \
    -H "Authorization: Bearer ${token}" \
    -H 'Content-Type: application/octet-stream' \
    --upload-file "${source}" \
    "https://storage.googleapis.com/upload/storage/v1/b/${BUCKET}/o?uploadType=media&name=${encoded}"
}

download() {
  local object="$1"
  local destination="$2"
  local token encoded
  token="$(curl --fail --silent --show-error \
    -H 'Metadata-Flavor: Google' \
    http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["access_token"])')"
  encoded="$(python3 -c 'import sys,urllib.parse; print(urllib.parse.quote(sys.argv[1], safe=""))' "$object")"
  curl --fail --silent --show-error --retry 3 --retry-all-errors \
    -H "Authorization: Bearer ${token}" \
    "https://storage.googleapis.com/download/storage/v1/b/${BUCKET}/o/${encoded}?alt=media" \
    -o "$destination"
}

finish() {
  local exit_code=$?
  local ended_at duration archive_sha
  trap - EXIT
  if [[ -n "$EXTERNAL_MEMORY_SAMPLER_PID" ]]; then
    kill "$EXTERNAL_MEMORY_SAMPLER_PID" >/dev/null 2>&1 || true
    wait "$EXTERNAL_MEMORY_SAMPLER_PID" 2>/dev/null || true
    EXTERNAL_MEMORY_SAMPLER_PID=""
  fi
  ended_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  duration="$(( $(date +%s) - START_SECONDS ))"
  archive_sha=""
  if [[ "${RVOIP_SCCACHE_ACTIVE:-0}" == "1" ]]; then
    mkdir -p "$EVIDENCE"
    {
      sccache --show-stats
      sccache --stop-server
    } > "$EVIDENCE/_sccache-stats.txt" 2>&1 || true
  fi
  if [[ -d "$EVIDENCE" ]]; then
    if [[ -d "$WORKSPACE/target/perf-results" ]]; then
      mkdir -p "$EVIDENCE/_perf-results/$SHARD_ID"
      (
        cd "$WORKSPACE/target/perf-results"
        find . -type f \
          \( -name '*.json' -o -name '*.md' -o -name '*.tsv' \
             -o -name '*.csv' -o -name '*.jsonl' -o -name '*.log' \
             -o -name '*.txt' \) \
          -exec cp --parents -t "$EVIDENCE/_perf-results/$SHARD_ID" {} +
      )
    fi
    tar -C /tmp -czf "$ARCHIVE" release-shard
    archive_sha="$(sha256sum "$ARCHIVE" | awk '{print $1}')"
    upload "$ARCHIVE" "${PREFIX}/release-shard.tar.gz" || exit_code=75
  fi
  python3 - "$RESULT" "$CANDIDATE" "$RUN_ID" "$SHARD_ID" "$GATES" \
    "$STARTED_AT" "$ended_at" "$duration" "$exit_code" "$archive_sha" <<'PY'
import json
import sys

path, candidate, run_id, shard, gates, started, ended, duration, code, archive_sha = sys.argv[1:]
payload = {
    "schema": "rvoip-gcp-release-shard-v1",
    "candidate_sha": candidate,
    "github_run_id": run_id,
    "shard_id": shard,
    "gates": sorted(value for value in gates.split(",") if value),
    "started_at": started,
    "ended_at": ended,
    "duration_seconds": int(duration),
    "exit_code": int(code),
    "status": "PASS" if int(code) == 0 else "FAIL",
    "evidence_archive_sha256": archive_sha or None,
    "publishing_attempted": False,
}
with open(path, "w", encoding="utf-8") as handle:
    json.dump(payload, handle, indent=2, sort_keys=True)
    handle.write("\n")
PY
  upload "$LOG" "${PREFIX}/qualification.log" || true
  upload "$RESULT" "${PREFIX}/result.json" || true
  sync
  shutdown -h now || true
  exit "$exit_code"
}
trap finish EXIT

capture_host_memory_policy() {
  local path
  {
    echo "captured_at_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "mimalloc_allow_thp_override=${MIMALLOC_ALLOW_THP_OVERRIDE:-unset}"
    uname -a
    for path in \
      /sys/kernel/mm/transparent_hugepage/enabled \
      /sys/kernel/mm/transparent_hugepage/defrag \
      /sys/kernel/mm/transparent_hugepage/shmem_enabled \
      /sys/kernel/mm/transparent_hugepage/khugepaged/scan_sleep_millisecs \
      /sys/kernel/mm/transparent_hugepage/khugepaged/alloc_sleep_millisecs \
      /sys/kernel/mm/transparent_hugepage/khugepaged/pages_to_scan; do
      if [[ -r "$path" ]]; then
        printf '%s=' "$path"
        cat "$path"
      fi
    done
  } > "$EVIDENCE/_host-memory-policy.txt"
}

capture_external_memory() {
  local output="$EVIDENCE/_external-process-memory.tsv"
  local process pid cmdline role process_memory thp_memory now uptime
  printf '%s\n' \
    $'captured_at_utc\tuptime_secs\tpid\trole\trss_kb\tpss_kb\tpss_anon_kb\tpss_file_kb\tprivate_clean_kb\tprivate_dirty_kb\tanonymous_kb\tlazy_free_kb\tanon_huge_pages_kb\tswap_kb\tthp_collapse_alloc\tthp_collapse_alloc_failed\tthp_fault_alloc\tthp_fault_fallback\tthp_split_page\tthp_deferred_split_page\tthp_split_pmd' \
    > "$output"
  while true; do
    now="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    uptime="$(awk '{print $1}' /proc/uptime)"
    thp_memory="$(awk '
      BEGIN { names["thp_collapse_alloc"]; names["thp_collapse_alloc_failed"];
              names["thp_fault_alloc"]; names["thp_fault_fallback"];
              names["thp_split_page"]; names["thp_deferred_split_page"];
              names["thp_split_pmd"] }
      $1 in names { value[$1] = $2 }
      END { printf "%s\t%s\t%s\t%s\t%s\t%s\t%s",
        value["thp_collapse_alloc"] + 0,
        value["thp_collapse_alloc_failed"] + 0,
        value["thp_fault_alloc"] + 0,
        value["thp_fault_fallback"] + 0,
        value["thp_split_page"] + 0,
        value["thp_deferred_split_page"] + 0,
        value["thp_split_pmd"] + 0 }
    ' /proc/vmstat)"
    for process in /proc/[0-9]*; do
      pid="${process##*/}"
      [[ -r "$process/cmdline" && -r "$process/smaps_rollup" ]] || continue
      cmdline="$(tr '\0' ' ' < "$process/cmdline" 2>/dev/null || true)"
      case "$cmdline" in
        *perf_burst_receiver*) role=burst_receiver ;;
        *perf_burst_caller*) role=burst_caller ;;
        *perf_soak_receiver*) role=soak_receiver ;;
        *perf_soak_caller*) role=soak_caller ;;
        *) continue ;;
      esac
      process_memory="$(awk '
        /^(Rss|Pss|Pss_Anon|Pss_File|Private_Clean|Private_Dirty|Anonymous|LazyFree|AnonHugePages|Swap):/ {
          name=$1; sub(/:$/, "", name); value[name]=$2
        }
        END { printf "%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s",
          value["Rss"] + 0, value["Pss"] + 0, value["Pss_Anon"] + 0,
          value["Pss_File"] + 0, value["Private_Clean"] + 0,
          value["Private_Dirty"] + 0, value["Anonymous"] + 0,
          value["LazyFree"] + 0, value["AnonHugePages"] + 0,
          value["Swap"] + 0 }
      ' "$process/smaps_rollup" 2>/dev/null || true)"
      [[ -n "$process_memory" ]] || continue
      printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
        "$now" "$uptime" "$pid" "$role" "$process_memory" "$thp_memory" \
        >> "$output"
    done
    sleep 5
  done
}

export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y --no-install-recommends \
  build-essential ca-certificates cmake curl git jq libasound2-dev libopus-dev \
  libssl-dev lld pkg-config protobuf-compiler

if [[ "$RESOURCE_CLASS" == "gcp-interop" \
  || "$RESOURCE_CLASS" == "gcp-proxy-interop" ]]; then
  # Ubuntu packages the SIPp binary as `sip-tester`; `sipp` is not a package.
  # Keep these heavyweight, network-facing tools off performance-only workers.
  apt-get install -y --no-install-recommends \
    baresip docker-compose-v2 docker.io netcat-openbsd openssl sip-tester tshark
  command -v sipp >/dev/null
  command -v tshark >/dev/null
  docker compose version >/dev/null
elif [[ ",$GATES," == *",perf.sipp-parity,"* \
  || ",$GATES," == *",preflight.performance-01,"* ]]; then
  # This performance test otherwise treats a missing external SIPp binary as a
  # local-development skip. Release qualification must execute it, not skip it.
  apt-get install -y --no-install-recommends sip-tester
  command -v sipp >/dev/null
fi

curl --proto '=https' --tlsv1.2 --fail --silent --show-error \
  https://sh.rustup.rs -o /tmp/rustup-init.sh
sh /tmp/rustup-init.sh -y --profile minimal --default-toolchain 1.91.0 \
  --component clippy,rustfmt
export PATH=/root/.cargo/bin:$PATH
command -v ld.lld >/dev/null
ld.lld --version
# Match the exact-candidate prebuilder. Some non-performance GCP gates compile
# directly on their worker, so both paths must use the same versioned linker
# contract.
export RUSTFLAGS="-C link-arg=-fuse-ld=lld"

git clone --filter=blob:none https://github.com/eisenzopf/rvoip.git "$WORKSPACE"
cd "$WORKSPACE"
git fetch --depth=1 origin "$CANDIDATE"
git checkout --detach "$CANDIDATE"
test "$(git rev-parse HEAD)" = "$CANDIDATE"

prebuilt_gate_ids="$(python3 scripts/release/prebuilt_performance.py select-gates \
  --catalog scripts/release/gates.json --gates "$GATES")"
if [[ -n "$prebuilt_gate_ids" ]]; then
  run_bundle_prefix="gs://${BUCKET}/release/${RUN_ID}/prebuild/"
  cache_bundle_prefix="gs://${BUCKET}/release-cache/performance-prebuilt-v1/"
  if [[ ! "$PREBUILT_SHA256" =~ ^[0-9a-f]{64}$ ]]; then
    echo "performance worker lacks an exact-run prebuilt bundle" >&2
    exit 1
  fi
  if [[ "$PREBUILT_URI" == "${run_bundle_prefix}"* ]]; then
    : # Compatibility with run-scoped bundles created before cache rollout.
  elif [[ "$PREBUILT_URI" == "${cache_bundle_prefix}"* \
    && "$PREBUILT_URI" == *"/bundles/${PREBUILT_SHA256}.tar.gz" ]]; then
    : # Verified controller cache hits are content-addressed by this digest.
  else
    echo "performance worker lacks an exact-run or content-addressed cached bundle" >&2
    exit 1
  fi
  prebuilt_object="${PREBUILT_URI#"gs://${BUCKET}/"}"
  download "$prebuilt_object" /tmp/performance-prebuilt.tar.gz
  python3 scripts/release/prebuilt_performance.py install-bundle \
    --archive /tmp/performance-prebuilt.tar.gz \
    --archive-sha256 "$PREBUILT_SHA256" \
    --destination /opt/rvoip-performance-prebuilt \
    --workspace "$WORKSPACE" \
    --candidate "$CANDIDATE" \
    --environment-id "$ENVIRONMENT_ID"
  export RVOIP_PERF_PREBUILT_MANIFEST=/opt/rvoip-performance-prebuilt/manifest.json
  export RVOIP_PERF_PREBUILT_BUNDLE_SHA256="$PREBUILT_SHA256"
  export RVOIP_PERF_PREBUILT_MANIFEST_SHA256
  RVOIP_PERF_PREBUILT_MANIFEST_SHA256="$(sha256sum \
    "$RVOIP_PERF_PREBUILT_MANIFEST" | awk '{print $1}')"
fi

# Every ephemeral worker previously spent roughly twenty-one minutes compiling
# the same release graph. Use a dedicated, lifecycle-managed GCS bucket as a
# content-addressed compiler cache. Cache availability is an optimization only:
# a download, authentication, or backend failure falls back to direct rustc so
# release correctness never depends on cached state.
SCCACHE_VERSION=0.15.0
SCCACHE_ARCHIVE="sccache-v${SCCACHE_VERSION}-x86_64-unknown-linux-musl.tar.gz"
SCCACHE_SHA256=782d2b5dd7ae0a55ebe368ab258114d0928d019ac2d949ab85d5d02f3926709e
SCCACHE_URL="https://github.com/mozilla/sccache/releases/download/v${SCCACHE_VERSION}/${SCCACHE_ARCHIVE}"
install_sccache() {
  curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
    "$SCCACHE_URL" -o "/tmp/$SCCACHE_ARCHIVE" || return 1
  echo "$SCCACHE_SHA256  /tmp/$SCCACHE_ARCHIVE" | sha256sum --check --status \
    || return 1
  tar -C /tmp -xzf "/tmp/$SCCACHE_ARCHIVE" || return 1
  install -m 0755 \
    "/tmp/sccache-v${SCCACHE_VERSION}-x86_64-unknown-linux-musl/sccache" \
    /usr/local/bin/sccache || return 1
}

mkdir -p "$EVIDENCE" /var/cache/rvoip-sccache
if install_sccache; then
  export CARGO_INCREMENTAL=0
  export RUSTC_WRAPPER=sccache
  export SCCACHE_BASEDIRS="$WORKSPACE"
  export SCCACHE_CACHE_SIZE=20G
  export SCCACHE_DIR=/var/cache/rvoip-sccache
  export SCCACHE_GCS_BUCKET="$CACHE_BUCKET"
  export SCCACHE_GCS_KEY_PREFIX=rvoip-release-v2-lld/rust-1.91.0/x86_64-unknown-linux-gnu
  export SCCACHE_GCS_RW_MODE=READ_WRITE
  cache_service_account="$(curl --fail --silent --show-error \
    -H 'Metadata-Flavor: Google' \
    http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/email)" \
    || cache_service_account=""
  export SCCACHE_IDLE_TIMEOUT=0
  export SCCACHE_MULTILEVEL_CHAIN=disk,gcs
  export SCCACHE_MULTILEVEL_WRITE_ERROR_POLICY=ignore
  if [[ -n "$cache_service_account" ]]; then
    export SCCACHE_GCS_SERVICE_ACCOUNT="$cache_service_account"
  fi
  if [[ -n "$cache_service_account" ]] && sccache --start-server; then
    export RVOIP_SCCACHE_ACTIVE=1
    echo "shared GCS compiler cache enabled"
  else
    unset RUSTC_WRAPPER
    echo "shared compiler cache unavailable; continuing with direct rustc" >&2
  fi
else
  echo "verified sccache install unavailable; continuing with direct rustc" >&2
fi

# The stock Ubuntu startup service inherits a soft nofile limit of 1024.
# SIPp and high-density media qualification legitimately require thousands of
# concurrent sockets, so make the worker limit explicit and fail closed if a
# future image cannot provide it.
ulimit -n 262144
test "$(ulimit -n)" -ge 262144

# Stateful proxy qualification captures a dense packet burst on both the
# loopback and aggregate interfaces.  Raise the receive ceiling before
# tcpdump requests its explicit 32 MiB capture buffer; otherwise a healthy
# capacity-overload run can lose evidence in the kernel and fail spuriously.
sysctl -w net.core.rmem_max=67108864
test "$(sysctl -n net.core.rmem_max)" -ge 67108864

export RVOIP_RELEASE_CANDIDATE="$CANDIDATE"
export RVOIP_RELEASE_ENVIRONMENT_ID="$ENVIRONMENT_ID"
export RVOIP_RELEASE_GATES="$GATES"
export RVOIP_RELEASE_RESOURCE_CLASS="$RESOURCE_CLASS"
export RVOIP_RELEASE_RUN_ID="$RUN_ID"
export RVOIP_RELEASE_SHARD_ID="$SHARD_ID"

case "$MIMALLOC_ALLOW_THP_OVERRIDE" in
  "") ;;
  0|1)
    export MIMALLOC_ALLOW_THP="$MIMALLOC_ALLOW_THP_OVERRIDE"
    echo "mimalloc THP override set to ${MIMALLOC_ALLOW_THP} for diagnostic A/B"
    ;;
  *)
    echo "rvoip-mimalloc-allow-thp must be 0, 1, or unset" >&2
    exit 1
    ;;
esac

if [[ "$EXTERNAL_MEMORY_DIAGNOSTICS" == "1" ]]; then
  capture_host_memory_policy
  capture_external_memory &
  EXTERNAL_MEMORY_SAMPLER_PID="$!"
  echo "external process memory and THP diagnostics enabled"
fi

set +e
python3 scripts/release/gates.py run-shard \
  --candidate "$CANDIDATE" \
  --environment-id "$ENVIRONMENT_ID" \
  --gates "$GATES" \
  --output "$EVIDENCE"
SHARD_STATUS=$?
set -e

if [[ -n "$EXTERNAL_MEMORY_SAMPLER_PID" ]]; then
  kill "$EXTERNAL_MEMORY_SAMPLER_PID" >/dev/null 2>&1 || true
  wait "$EXTERNAL_MEMORY_SAMPLER_PID" 2>/dev/null || true
  EXTERNAL_MEMORY_SAMPLER_PID=""
fi
exit "$SHARD_STATUS"
