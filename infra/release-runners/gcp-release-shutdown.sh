#!/usr/bin/env bash
set -Eeuo pipefail

# GCE executes this script during an operator/controller VM stop. The normal
# startup script uploads a final PASS/FAIL result from its EXIT trap, but a VM
# stop is not required to let that shell trap complete. Snapshot whatever gate
# receipts have already been paid for before the boot disk is deleted.

metadata() {
  curl --fail --silent --show-error \
    -H 'Metadata-Flavor: Google' \
    "http://metadata.google.internal/computeMetadata/v1/instance/attributes/$1"
}

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

CANDIDATE="$(metadata rvoip-candidate)"
RUN_ID="$(metadata rvoip-run-id)"
SHARD_ID="$(metadata rvoip-shard-id)"
BUCKET="$(metadata rvoip-evidence-bucket)"
PREFIX="$(metadata rvoip-prefix)"
GATES="$(metadata rvoip-gates-b64 | base64 --decode)"
EVIDENCE=/tmp/release-shard
ARCHIVE=/tmp/release-shard-partial.tar.gz
RESULT=/tmp/result-partial.json
LOG=/var/log/rvoip-release-qualification.log

exec 9>/tmp/rvoip-release-result.lock
flock -w 60 9 || exit 0

# A normal final result always wins. The shutdown checkpoint exists only for a
# shard the controller interrupted before final evidence was committed.
if [[ -f /tmp/result.json ]]; then
  exit 0
fi

mkdir -p "$EVIDENCE"
tar -C /tmp -czf "$ARCHIVE" release-shard
archive_sha="$(sha256sum "$ARCHIVE" | awk '{print $1}')"
python3 - "$RESULT" "$CANDIDATE" "$RUN_ID" "$SHARD_ID" "$GATES" \
  "$archive_sha" "$EVIDENCE" <<'PY'
import json
from pathlib import Path
import sys

path, candidate, run_id, shard, gates, archive_sha, evidence = sys.argv[1:]
completed = set()
for receipt_path in Path(evidence).rglob("receipt.json"):
    try:
        receipt = json.loads(receipt_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        continue
    gate_id = receipt.get("gate_id")
    if isinstance(gate_id, str) and receipt.get("status") in {"PASS", "FAIL"}:
        completed.add(gate_id)
payload = {
    "schema": "rvoip-gcp-release-shard-v1",
    "candidate_sha": candidate,
    "github_run_id": run_id,
    "shard_id": shard,
    "gates": sorted(value for value in gates.split(",") if value),
    "completed_gates": sorted(completed),
    "exit_code": 143,
    "status": "PARTIAL",
    "termination_reason": "controller-stop",
    "evidence_archive_sha256": archive_sha,
    "publishing_attempted": False,
}
with open(path, "w", encoding="utf-8") as handle:
    json.dump(payload, handle, indent=2, sort_keys=True)
    handle.write("\n")
PY

upload "$ARCHIVE" "${PREFIX}/release-shard.tar.gz"
upload "$LOG" "${PREFIX}/qualification.log" || true
upload "$RESULT" "${PREFIX}/result.json"
sync
