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
PREFIX="$(metadata rvoip-prefix)"
GATES="$(metadata rvoip-gates-b64 | base64 --decode)"
ENVIRONMENT_ID="$(metadata rvoip-environment-b64 | base64 --decode)"
WORKSPACE=/opt/rvoip
EVIDENCE=/tmp/release-shard
ARCHIVE=/tmp/release-shard.tar.gz
RESULT=/tmp/result.json
STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
START_SECONDS="$(date +%s)"

upload() {
  local source="$1"
  local object="$2"
  local token encoded
  token="$(curl --fail --silent --show-error \
    -H 'Metadata-Flavor: Google' \
    http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["access_token"])')"
  encoded="$(python3 -c 'import sys,urllib.parse; print(urllib.parse.quote(sys.argv[1], safe=""))' "$object")"
  curl --fail --silent --show-error \
    -X POST \
    -H "Authorization: Bearer ${token}" \
    -H 'Content-Type: application/octet-stream' \
    --data-binary "@${source}" \
    "https://storage.googleapis.com/upload/storage/v1/b/${BUCKET}/o?uploadType=media&name=${encoded}"
}

finish() {
  local exit_code=$?
  local ended_at duration archive_sha
  trap - EXIT
  ended_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  duration="$(( $(date +%s) - START_SECONDS ))"
  archive_sha=""
  if [[ -d "$EVIDENCE" ]]; then
    if [[ -d "$WORKSPACE/target/perf-results" ]]; then
      mkdir -p "$EVIDENCE/_perf-results/$SHARD_ID"
      (
        cd "$WORKSPACE/target/perf-results"
        find . -type f \
          \( -name '*.json' -o -name '*.md' -o -name '*.tsv' \
             -o -name '*.csv' -o -name '*.log' -o -name '*.txt' \) \
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

export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y --no-install-recommends \
  build-essential ca-certificates cmake curl git jq libasound2-dev libopus-dev \
  libssl-dev pkg-config protobuf-compiler

if [[ "$RESOURCE_CLASS" == "gcp-interop" ]]; then
  # Ubuntu packages the SIPp binary as `sip-tester`; `sipp` is not a package.
  # Keep these heavyweight, network-facing tools off performance-only workers.
  apt-get install -y --no-install-recommends \
    baresip docker.io netcat-openbsd openssl sip-tester
  command -v sipp >/dev/null
elif [[ ",$GATES," == *",perf.sipp-parity,"* ]]; then
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

git clone --filter=blob:none https://github.com/eisenzopf/rvoip.git "$WORKSPACE"
cd "$WORKSPACE"
git fetch --depth=1 origin "$CANDIDATE"
git checkout --detach "$CANDIDATE"
test "$(git rev-parse HEAD)" = "$CANDIDATE"

python3 scripts/release/gates.py run-shard \
  --candidate "$CANDIDATE" \
  --environment-id "$ENVIRONMENT_ID" \
  --gates "$GATES" \
  --output "$EVIDENCE"
