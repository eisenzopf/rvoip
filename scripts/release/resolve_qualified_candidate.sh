#!/usr/bin/env bash
# Verify downloaded qualification evidence and check out its candidate.
#
# Shared by both publication jobs. Reads target/downloaded-qualification,
# verifies the aggregate's attestation and identity, checks out the
# candidate as local `main`, and exports QUALIFIED_CANDIDATE, RELEASE_TOOL
# (a copy of the trusted workflow revision's release.py) and
# REMOTE_QUALIFICATION through $GITHUB_ENV.
#
# Environment: GH_TOKEN, REPOSITORY, GITHUB_ENV.
set -euo pipefail

aggregate="$(find target/downloaded-qualification -name aggregate.json -type f -print -quit)"
test -n "$aggregate" || {
  echo "qualification aggregate is missing" >&2
  exit 1
}
gh attestation verify "$aggregate" --repo "${REPOSITORY:?}"
test "$(jq -r .schema "$aggregate")" = rvoip-release-qualification-v1
test "$(jq -r .status "$aggregate")" = PASS
# release.py verify enforces which profile this version needs.
case "$(jq -r .profile "$aggregate")" in
  remote-release|release-patch|release-minor|release-patch-smoke) ;;
  *) echo "qualification is not a release profile" >&2; exit 1 ;;
esac
candidate="$(jq -r .candidate_sha "$aggregate")"
[[ "$candidate" =~ ^[0-9a-f]{40}$ ]] || {
  echo "qualification candidate is not a full commit SHA" >&2
  exit 1
}
git cat-file -e "$candidate^{commit}"
git merge-base --is-ancestor "$candidate" origin/main || {
  echo "qualified candidate is not an ancestor of origin/main" >&2
  exit 1
}
qualification="$(realpath "$aggregate")"
mkdir -p target
cp scripts/release.py target/release-tool.py
release_tool="$(realpath target/release-tool.py)"
git checkout -B main "$candidate"
test "$(git rev-parse HEAD)" = "$candidate"
{
  echo "QUALIFIED_CANDIDATE=$candidate"
  echo "RELEASE_TOOL=$release_tool"
  echo "REMOTE_QUALIFICATION=$qualification"
} >> "${GITHUB_ENV:?}"
