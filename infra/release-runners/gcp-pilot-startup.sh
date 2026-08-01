#!/usr/bin/env bash
set -Eeuo pipefail

LOG=/var/log/rvoip-qualification.log
exec > >(tee -a "$LOG") 2>&1

metadata() {
  curl --fail --silent --show-error \
    -H 'Metadata-Flavor: Google' \
    "http://metadata.google.internal/computeMetadata/v1/instance/attributes/$1"
}

metadata_optional() {
  curl --fail --silent --show-error \
    -H 'Metadata-Flavor: Google' \
    "http://metadata.google.internal/computeMetadata/v1/instance/attributes/$1" \
    2>/dev/null || true
}

CANDIDATE="$(metadata rvoip-candidate)"
PROFILE="$(metadata rvoip-profile)"
RUN_ID="$(metadata rvoip-run-id)"
BUCKET="$(metadata rvoip-evidence-bucket)"
PREFIX="$(metadata_optional rvoip-prefix)"
PREFIX="${PREFIX:-pilot/${RUN_ID}}"
SHARD_ID="$(metadata_optional rvoip-shard-id)"
PACKAGES_B64="$(metadata_optional rvoip-packages-b64)"
PACKAGES=""
if [[ -n "$PACKAGES_B64" ]]; then
  PACKAGES="$(printf '%s' "$PACKAGES_B64" | base64 --decode)"
fi
STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
START_SECONDS="$(date +%s)"
WORKSPACE=/opt/rvoip
RESULT=/tmp/rvoip-result.json
COMMAND_RECEIPT=/tmp/rvoip-command-receipt.json

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
  local ended_at duration rust_version receipt_sha
  trap - EXIT
  ended_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  duration="$(( $(date +%s) - START_SECONDS ))"
  rust_version="$(rustc --version 2>/dev/null || printf unavailable)"
  receipt_sha=""
  if [[ -f "$COMMAND_RECEIPT" ]]; then
    receipt_sha="$(sha256sum "$COMMAND_RECEIPT" | awk '{print $1}')"
  fi
  python3 - "$RESULT" "$CANDIDATE" "$PROFILE" "$RUN_ID" "$STARTED_AT" "$ended_at" "$duration" "$exit_code" "$rust_version" "$SHARD_ID" "$PACKAGES" "$receipt_sha" <<'PY'
import json
import sys

path, candidate, profile, run_id, started, ended, duration, code, rust, shard_id, packages, receipt_sha = sys.argv[1:]
payload = {
    "schema": "rvoip-gcp-qualification-pilot-v1",
    "candidate_sha": candidate,
    "profile": profile,
    "github_run_id": run_id,
    "started_at": started,
    "ended_at": ended,
    "duration_seconds": int(duration),
    "exit_code": int(code),
    "status": "PASS" if int(code) == 0 else "FAIL",
    "rustc": rust,
    "shard_id": shard_id or None,
    "packages": [value for value in packages.split(",") if value],
    "command_receipt_sha256": receipt_sha or None,
    "publishing_attempted": False,
}
with open(path, "w", encoding="utf-8") as handle:
    json.dump(payload, handle, indent=2, sort_keys=True)
    handle.write("\n")
PY
  upload "$LOG" "${PREFIX}/qualification.log" || true
  if [[ -f "$COMMAND_RECEIPT" ]]; then
    upload "$COMMAND_RECEIPT" "${PREFIX}/command-receipt.json" || true
  fi
  upload "$RESULT" "${PREFIX}/result.json" || true
  sync
  shutdown -h now || true
  exit "$exit_code"
}
trap finish EXIT

export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y --no-install-recommends \
  build-essential ca-certificates cmake curl git jq \
  libasound2-dev libopus-dev libssl-dev pkg-config protobuf-compiler

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

if [[ "$PROFILE" == smoke ]]; then
  python3 scripts/release.py audit
  python3 -m unittest \
    scripts/ci/test_aggregate_receipts.py \
    scripts/ci/test_compare_nextest_inventory.py \
    scripts/ci/test_pr_plan.py \
    scripts/ci/test_run_checks.py \
    scripts/ci/test_workflow_policy.py \
    scripts/test_release.py \
    scripts/test_release_carry_forward_attestation.py \
    scripts/test_release_exception_attestation.py \
    scripts/test_release_gates.py
  cargo check -p rvoip-rtc --all-targets --locked
elif [[ "$PROFILE" == workspace ]]; then
  python3 scripts/release.py audit
  python3 -m unittest \
    scripts/ci/test_aggregate_receipts.py \
    scripts/ci/test_compare_nextest_inventory.py \
    scripts/ci/test_pr_plan.py \
    scripts/ci/test_run_checks.py \
    scripts/ci/test_workflow_policy.py \
    scripts/test_release.py \
    scripts/test_release_carry_forward_attestation.py \
    scripts/test_release_exception_attestation.py \
    scripts/test_release_gates.py
  scripts/test_all.sh
  cargo test -p rvoip-users-core \
    --test security_timing_attack_tests \
    --locked \
    -- --ignored --test-threads=1 --nocapture
elif [[ "$PROFILE" == workspace-policy ]]; then
  python3 scripts/release.py audit
  python3 -m unittest \
    scripts/ci/test_aggregate_receipts.py \
    scripts/ci/test_compare_nextest_inventory.py \
    scripts/ci/test_pr_plan.py \
    scripts/ci/test_run_checks.py \
    scripts/ci/test_workflow_policy.py \
    scripts/test_release.py \
    scripts/test_release_carry_forward_attestation.py \
    scripts/test_release_exception_attestation.py \
    scripts/test_release_gates.py
elif [[ "$PROFILE" == workspace-shard-test || "$PROFILE" == workspace-shard-clippy ]]; then
  [[ -n "$PACKAGES" ]] || {
    echo "workspace shard is missing its package selection" >&2
    exit 2
  }
  kind="shard-test"
  if [[ "$PROFILE" == workspace-shard-clippy ]]; then
    kind="shard-clippy"
  fi
  python3 scripts/ci/run_checks.py "$kind" \
    --name "$SHARD_ID" \
    --packages "$PACKAGES" \
    --output "$COMMAND_RECEIPT"
elif [[ "$PROFILE" == workspace-doctest ]]; then
  python3 scripts/ci/run_checks.py doctest \
    --name "$SHARD_ID" \
    --output "$COMMAND_RECEIPT"
elif [[ "$PROFILE" == workspace-security-timing ]]; then
  cargo test -p rvoip-users-core \
    --test security_timing_attack_tests \
    --locked \
    -- --ignored --test-threads=1 --nocapture
else
  echo "unsupported fixed qualification profile: $PROFILE" >&2
  exit 2
fi
