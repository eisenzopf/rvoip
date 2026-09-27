#!/usr/bin/env bash
set -Eeuo pipefail

# systemd runs this script as the ExecStop of rvoip-release-shutdown.service
# during an ACPI stop, which is what a controller `stop-instances` delivers to
# a deferred or cut-off worker. The normal startup script uploads a final
# PASS/FAIL result from its EXIT trap, but an instance stop is not required to
# let that shell trap complete. Snapshot whatever gate receipts have already
# been paid for before the delete-on-termination root volume is gone.
#
# The unit must be ordered after network-online.target so this ExecStop runs
# while the instance still has a route to S3 and IMDS.

set -a
# shellcheck source=/dev/null
source /etc/rvoip-release.env
set +a

CANDIDATE="$RVOIP_CANDIDATE"
RUN_ID="$RVOIP_RUN_ID"
SHARD_ID="$RVOIP_SHARD_ID"
BUCKET="$RVOIP_EVIDENCE_BUCKET"
PREFIX="$RVOIP_PREFIX"
GATES="$(printf '%s' "$RVOIP_GATES_B64" | base64 --decode)"
AWS_REGION="$RVOIP_AWS_REGION"
export AWS_REGION
export AWS_DEFAULT_REGION="$AWS_REGION"
EVIDENCE=/tmp/release-shard
ARCHIVE=/tmp/release-shard-partial.tar.gz
RESULT=/tmp/result-partial.json
LOG=/var/log/rvoip-release-qualification.log
S3_CP_ATTEMPTS=3

# The startup script installed the pinned, signature-verified AWS CLI under
# /usr/local/bin before any evidence could exist; uploads use the runner
# instance role with no explicit credential.
upload() {
  local source="$1"
  local object="$2"
  local attempt
  for attempt in $(seq 1 "$S3_CP_ATTEMPTS"); do
    if aws s3 cp --only-show-errors "$source" "s3://${BUCKET}/${object}"; then
      return 0
    fi
    echo "upload attempt ${attempt}/${S3_CP_ATTEMPTS} failed: ${object}" >&2
    if (( attempt < S3_CP_ATTEMPTS )); then
      sleep "$(( attempt * 5 ))"
    fi
  done
  return 1
}

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
    "schema": "rvoip-ec2-release-shard-v1",
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
