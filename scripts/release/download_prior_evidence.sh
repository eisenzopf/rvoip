#!/usr/bin/env bash
# Download the evidence of prior qualification runs into target/prior-evidence.
#
# Used by the plan job (to reuse exact receipts) and the collect job (to stage
# the reused gates' evidence beside the fresh evidence for the reports).
#
# Release profiles read only `release-gate-evidence`, the attested artifact of
# a reusable run. A diagnostic may also read `diagnostic-gate-evidence`, so a
# version-bump diagnostic can prove it reuses its parent's receipts; nothing a
# diagnostic produces is ever reusable for release.
#
# Environment: GH_TOKEN, PRIOR_RUN_IDS (comma-separated), PROFILE, REPOSITORY.
set -euo pipefail

IFS=, read -r -a run_ids <<< "${PRIOR_RUN_IDS:?}"
if (( ${#run_ids[@]} > 5 )); then
  echo "at most five prior evidence runs may be combined" >&2
  exit 1
fi
for value in "${run_ids[@]}"; do
  run_id="${value//[[:space:]]/}"
  [[ "$run_id" =~ ^[1-9][0-9]*$ ]] || {
    echo "invalid prior run ID: $value" >&2
    exit 1
  }
  if gh run download "$run_id" \
    --repo "${REPOSITORY:?}" \
    --name release-gate-evidence \
    --dir "target/prior-evidence/$run_id"; then
    continue
  fi
  if [[ "${PROFILE:?}" == remote-diagnostic ]]; then
    gh run download "$run_id" \
      --repo "$REPOSITORY" \
      --name diagnostic-gate-evidence \
      --dir "target/prior-evidence/$run_id"
  else
    echo "run $run_id has no reusable release-gate-evidence artifact" >&2
    exit 1
  fi
done
