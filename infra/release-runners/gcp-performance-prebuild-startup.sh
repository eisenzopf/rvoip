#!/usr/bin/env bash
set -Eeuo pipefail

LOG=/var/log/rvoip-performance-prebuild.log
exec > >(tee -a "$LOG") 2>&1

metadata() {
  curl --fail --silent --show-error --retry 3 --retry-all-errors \
    -H 'Metadata-Flavor: Google' \
    "http://metadata.google.internal/computeMetadata/v1/instance/attributes/$1"
}

CANDIDATE="$(metadata rvoip-candidate)"
RUN_ID="$(metadata rvoip-run-id)"
BUCKET="$(metadata rvoip-evidence-bucket)"
CACHE_BUCKET="$(metadata rvoip-cache-bucket)"
PREFIX="$(metadata rvoip-prefix)"
CACHE_KEY="$(metadata rvoip-prebuild-cache-key)"
GATES="$(metadata rvoip-gates-b64 | base64 --decode)"
ENVIRONMENT_ID="$(metadata rvoip-environment-b64 | base64 --decode)"
WORKSPACE=/opt/rvoip
BUNDLE_ROOT=/tmp/performance-prebuilt
BUNDLE=/tmp/performance-prebuilt.tar.gz
RESULT=/tmp/prebuild-result.json
STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
START_SECONDS="$(date +%s)"
BUNDLE_SHA=""
MANIFEST_SHA=""
CACHE_PREFIX="release-cache/performance-prebuilt-v1/${CACHE_KEY}"
BUNDLE_URI=""
MANIFEST_URI=""

token() {
  curl --fail --silent --show-error \
    -H 'Metadata-Flavor: Google' \
    http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["access_token"])'
}

upload() {
  local source="$1"
  local object="$2"
  local access_token encoded
  access_token="$(token)"
  encoded="$(python3 -c 'import sys,urllib.parse; print(urllib.parse.quote(sys.argv[1], safe=""))' "$object")"
  curl --fail --silent --show-error --retry 3 --retry-all-errors \
    -X POST \
    -H "Authorization: Bearer ${access_token}" \
    -H 'Content-Type: application/octet-stream' \
    --upload-file "${source}" \
    "https://storage.googleapis.com/upload/storage/v1/b/${BUCKET}/o?uploadType=media&name=${encoded}"
}

download() {
  local object="$1"
  local destination="$2"
  local access_token encoded
  access_token="$(token)"
  encoded="$(python3 -c 'import sys,urllib.parse; print(urllib.parse.quote(sys.argv[1], safe=""))' "$object")"
  curl --fail --silent --show-error --retry 3 --retry-all-errors \
    -H "Authorization: Bearer ${access_token}" \
    "https://storage.googleapis.com/download/storage/v1/b/${BUCKET}/o/${encoded}?alt=media" \
    -o "$destination"
}

finish() {
  local exit_code=$?
  local ended_at duration status
  trap - EXIT
  ended_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  duration="$(( $(date +%s) - START_SECONDS ))"
  status=FAIL
  if (( exit_code == 0 )); then
    status=PASS
  fi
  if [[ "${RVOIP_SCCACHE_ACTIVE:-0}" == "1" ]]; then
    {
      sccache --show-stats
      sccache --stop-server
    } > /tmp/prebuild-sccache-stats.txt 2>&1 || true
    upload /tmp/prebuild-sccache-stats.txt "${PREFIX}/prebuild-sccache-stats.txt" || true
  fi
  python3 - "$RESULT" "$CANDIDATE" "$RUN_ID" "$ENVIRONMENT_ID" "$GATES" \
    "$CACHE_KEY" \
    "$STARTED_AT" "$ended_at" "$duration" "$exit_code" "$status" \
    "$BUNDLE_URI" "$BUNDLE_SHA" "$MANIFEST_URI" "$MANIFEST_SHA" <<'PY'
import json
import sys

(
    path,
    candidate,
    run_id,
    environment_id,
    gates,
    cache_key,
    started,
    ended,
    duration,
    code,
    status,
    bundle_uri,
    bundle_sha,
    manifest_uri,
    manifest_sha,
) = sys.argv[1:]
payload = {
    "schema": "rvoip-gcp-performance-prebuild-result-v1",
    "candidate_sha": candidate,
    "github_run_id": run_id,
    "environment_id": environment_id,
    "selected_gate_ids": sorted({value for value in gates.split(",") if value}),
    "cache_key_sha256": cache_key,
    "started_at": started,
    "ended_at": ended,
    "duration_seconds": int(duration),
    "exit_code": int(code),
    "status": status,
    "bundle_uri": bundle_uri if bundle_sha else None,
    "bundle_sha256": bundle_sha or None,
    "manifest_uri": manifest_uri if manifest_sha else None,
    "manifest_sha256": manifest_sha or None,
    "publishing_attempted": False,
}
with open(path, "w", encoding="utf-8") as handle:
    json.dump(payload, handle, indent=2, sort_keys=True)
    handle.write("\n")
PY
  upload "$LOG" "${PREFIX}/prebuild.log" || true
  upload "$RESULT" "${PREFIX}/prebuild-result.json" || true
  # The run-scoped result remains authoritative for this invocation. Publish
  # the cache pointer only after every content-addressed object exists and the
  # build has passed, so interrupted builders can never create a false hit.
  if (( exit_code == 0 )); then
    upload "$RESULT" "${CACHE_PREFIX}/prebuild-result.json" || true
  fi
  sync
  shutdown -h now || true
  exit "$exit_code"
}
trap finish EXIT

export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y --no-install-recommends \
  build-essential ca-certificates cmake curl git libasound2-dev libopus-dev \
  libssl-dev lld pigz pkg-config protobuf-compiler

curl --proto '=https' --tlsv1.2 --fail --silent --show-error \
  https://sh.rustup.rs -o /tmp/rustup-init.sh
sh /tmp/rustup-init.sh -y --profile minimal --default-toolchain 1.91.0
export PATH=/root/.cargo/bin:$PATH
command -v ld.lld >/dev/null
ld.lld --version
# GNU ld dominates an otherwise cache-hot exact-candidate prebuild. Keep the
# linker choice explicit and release-environment-versioned so evidence built
# with a different linker can never be reused accidentally.
export RUSTFLAGS="-C link-arg=-fuse-ld=lld"

git clone --filter=blob:none https://github.com/eisenzopf/rvoip.git "$WORKSPACE"
cd "$WORKSPACE"
git fetch --depth=1 origin "$CANDIDATE"
git checkout --detach "$CANDIDATE"
test "$(git rev-parse HEAD)" = "$CANDIDATE"

SCCACHE_VERSION=0.15.0
SCCACHE_ARCHIVE="sccache-v${SCCACHE_VERSION}-x86_64-unknown-linux-musl.tar.gz"
SCCACHE_SHA256=782d2b5dd7ae0a55ebe368ab258114d0928d019ac2d949ab85d5d02f3926709e
SCCACHE_URL="https://github.com/mozilla/sccache/releases/download/v${SCCACHE_VERSION}/${SCCACHE_ARCHIVE}"
if curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
    "$SCCACHE_URL" -o "/tmp/$SCCACHE_ARCHIVE" \
  && echo "$SCCACHE_SHA256  /tmp/$SCCACHE_ARCHIVE" | sha256sum --check --status \
  && tar -C /tmp -xzf "/tmp/$SCCACHE_ARCHIVE" \
  && install -m 0755 \
    "/tmp/sccache-v${SCCACHE_VERSION}-x86_64-unknown-linux-musl/sccache" \
    /usr/local/bin/sccache; then
  export CARGO_INCREMENTAL=0
  export RUSTC_WRAPPER=sccache
  export SCCACHE_BASEDIRS="$WORKSPACE"
  export SCCACHE_CACHE_SIZE=40G
  export SCCACHE_DIR=/var/cache/rvoip-sccache
  export SCCACHE_GCS_BUCKET="$CACHE_BUCKET"
  export SCCACHE_GCS_KEY_PREFIX=rvoip-release-v2-lld/rust-1.91.0/x86_64-unknown-linux-gnu
  export SCCACHE_GCS_RW_MODE=READ_WRITE
  export SCCACHE_IDLE_TIMEOUT=0
  export SCCACHE_MULTILEVEL_CHAIN=disk,gcs
  export SCCACHE_MULTILEVEL_WRITE_ERROR_POLICY=ignore
  cache_service_account="$(curl --fail --silent --show-error \
    -H 'Metadata-Flavor: Google' \
    http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/email)" \
    || cache_service_account=""
  if [[ -n "$cache_service_account" ]]; then
    export SCCACHE_GCS_SERVICE_ACCOUNT="$cache_service_account"
  fi
  if [[ -n "$cache_service_account" ]] && sccache --start-server; then
    export RVOIP_SCCACHE_ACTIVE=1
    echo "shared GCS compiler cache enabled"
  else
    unset RUSTC_WRAPPER
  fi
fi

echo "building exact-candidate performance executables once on ${CANDIDATE}"
python3 scripts/release/prebuilt_performance.py build \
  --workspace "$WORKSPACE" \
  --catalog "$WORKSPACE/scripts/release/gates.json" \
  --gates "$GATES" \
  --candidate "$CANDIDATE" \
  --environment-id "$ENVIRONMENT_ID" \
  --output "$BUNDLE_ROOT"

MANIFEST_SHA="$(sha256sum "$BUNDLE_ROOT/manifest.json" | awk '{print $1}')"
tar -C /tmp -I 'pigz -3' -cf "$BUNDLE" performance-prebuilt
BUNDLE_SHA="$(sha256sum "$BUNDLE" | awk '{print $1}')"
BUNDLE_OBJECT="${CACHE_PREFIX}/bundles/${BUNDLE_SHA}.tar.gz"
MANIFEST_OBJECT="${CACHE_PREFIX}/manifests/${MANIFEST_SHA}.json"
BUNDLE_URI="gs://${BUCKET}/${BUNDLE_OBJECT}"
MANIFEST_URI="gs://${BUCKET}/${MANIFEST_OBJECT}"
upload "$BUNDLE_ROOT/manifest.json" "$MANIFEST_OBJECT"
upload "$BUNDLE" "$BUNDLE_OBJECT"

# Prove that the runtime service account can read evidence before deleting the
# builder and creating the measurement fleet. Upload-only IAM otherwise fails
# one VM later and obscures an infrastructure defect as a gate failure.
download "$MANIFEST_OBJECT" /tmp/performance-manifest-readback.json
echo "${MANIFEST_SHA}  /tmp/performance-manifest-readback.json" \
  | sha256sum --check --status
