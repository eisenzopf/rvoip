#!/bin/bash
# check-publish-readiness.sh — pre-flight gate for crates.io publication.
# Exits 0 if every beta-tier crate is ready to publish, non-zero on the
# first problem with a clear pointer to what to fix.
#
# Checks:
#   1. Every beta-tier crate has populated description / license /
#      repository / categories / keywords / readme.
#   2. No internal dep uses a bare `path = "..."` without a sibling
#      `version` (or `.workspace = true`) — these break `cargo publish`.
#   3. `publish` flag matches the expected tier (beta = allowed,
#      alpha-hold = false, alpha-publish = allowed).
#   4. Workspace member version matches the workspace-relative version
#      in [workspace.dependencies].
#   5. `cargo metadata --no-deps` succeeds.
#
# This script does NOT run `cargo publish --dry-run` — that's
# scripts/publish-dry-run.sh. Run both before tagging.

set -euo pipefail

G='\033[0;32m'; R='\033[0;31m'; Y='\033[1;33m'; N='\033[0m'

cd "$(dirname "$0")/.."

BETA_CRATES=(
    rvoip-core-traits rvoip-infra-common rvoip-codec-core rvoip-auth-core
    rvoip-rtp-core rvoip-media-core rvoip-core
    rvoip-sip-core rvoip-sip-transport rvoip-sip-dialog
    rvoip-sip-proxy rvoip-sip-registrar rvoip-sip
)
ALPHA_PUBLISH=(rvoip-vcon rvoip-harness)
ALPHA_HOLD=(
    rvoip rvoip-client rvoip-uctp rvoip-quic
    rvoip-webtransport rvoip-websocket rvoip-webrtc
    rvoip-identity rvoip-stir-shaken users-core rvoip-audio-core
)

errors=0
err() { echo -e "${R}  ERROR: $*${N}"; errors=$((errors + 1)); }
ok()  { echo -e "${G}  OK: $*${N}"; }
warn(){ echo -e "${Y}  WARN: $*${N}"; }

echo "[1/5] cargo metadata --no-deps"
if ! cargo metadata --no-deps --format-version 1 > /tmp/.rvoip-meta.json 2>&1; then
    err "cargo metadata failed — fix workspace before proceeding"
    cat /tmp/.rvoip-meta.json
    exit 1
fi
ok "metadata resolves; $(jq '.packages | length' /tmp/.rvoip-meta.json) members"

echo ""
echo "[2/5] required package metadata on beta-tier crates"
for crate in "${BETA_CRATES[@]}"; do
    pkg=$(jq --arg n "$crate" '.packages[] | select(.name == $n)' /tmp/.rvoip-meta.json)
    if [ -z "$pkg" ]; then
        err "$crate: not found in workspace metadata"
        continue
    fi
    for field in description repository license keywords categories; do
        value=$(echo "$pkg" | jq -r ".${field} // empty")
        if [ -z "$value" ] || [ "$value" = "[]" ] || [ "$value" = "null" ]; then
            err "$crate: missing or empty .${field}"
        fi
    done
    manifest=$(echo "$pkg" | jq -r '.manifest_path')
    readme_field=$(echo "$pkg" | jq -r '.readme // empty')
    crate_dir=$(dirname "$manifest")
    # Cargo treats README.md in the same dir as the implicit readme. Either
    # the explicit `readme = "..."` field or the file presence is fine.
    if [ -z "$readme_field" ] && [ ! -f "$crate_dir/README.md" ]; then
        err "$crate: no README (set readme = \"...\" or add README.md at $crate_dir)"
    fi
done

echo ""
echo "[3/5] publish flag matches tier"
check_publish() {
    local crate="$1"
    local want="$2"   # "allowed" or "false"
    local actual
    actual=$(jq -r --arg n "$crate" '.packages[] | select(.name == $n) | (.publish // ["any"]) | tostring' /tmp/.rvoip-meta.json)
    case "$want" in
        allowed)
            # publish=null (no override) is allowed-everywhere → cargo
            # metadata renders it as "null". publish=[] (empty array)
            # means "no registry" → blocked. We want either null OR a
            # non-empty list that includes the default registry.
            if [ "$actual" = "[]" ]; then
                err "$crate: should be publishable but has publish = false"
            else
                ok "$crate: publishable"
            fi
            ;;
        false)
            if [ "$actual" = "[]" ]; then
                ok "$crate: publish = false (as expected)"
            else
                err "$crate: should be publish = false but isn't (publish=$actual)"
            fi
            ;;
    esac
}

for c in "${BETA_CRATES[@]}";   do check_publish "$c" allowed; done
for c in "${ALPHA_PUBLISH[@]}"; do check_publish "$c" allowed; done
for c in "${ALPHA_HOLD[@]}";    do check_publish "$c" false;   done

echo ""
echo "[4/5] no bare path deps in beta-tier crates"
for crate in "${BETA_CRATES[@]}"; do
    manifest=$(jq -r --arg n "$crate" '.packages[] | select(.name == $n) | .manifest_path' /tmp/.rvoip-meta.json)
    # Grep for `path = "..."` in a dep line that does NOT also carry
    # `version =` or `.workspace = true`. Multi-line table form
    # (`[dependencies.foo]\npath = ...`) is rare here and not covered by
    # this simple grep — that's a known gap.
    bare=$(grep -nE '^[a-z][a-z0-9_-]+ *= *\{[^}]*path *= *"[^"]+"[^}]*\}' "$manifest" \
        | grep -vE 'version *=|workspace *= *true' || true)
    if [ -n "$bare" ]; then
        err "$crate: bare path dep(s) in $manifest:"
        echo "$bare" | sed 's/^/    /'
    fi
done

echo ""
echo "[5/5] workspace.dependencies versions match member crate versions"
for crate in "${BETA_CRATES[@]}" "${ALPHA_PUBLISH[@]}" "${ALPHA_HOLD[@]}"; do
    member_version=$(jq -r --arg n "$crate" '.packages[] | select(.name == $n) | .version' /tmp/.rvoip-meta.json)
    # Look up workspace.dependencies entry in root Cargo.toml. Skips
    # entries that don't appear there (some crates aren't reused
    # internally as workspace deps).
    dep_version=$(grep -E "^${crate} *=" Cargo.toml | sed -E 's/.*version *= *"([^"]+)".*/\1/' || true)
    if [ -z "$dep_version" ]; then
        continue
    fi
    if [ "$member_version" != "$dep_version" ]; then
        err "$crate: member version $member_version != workspace.dependencies version $dep_version"
    fi
done

echo ""
if [ "$errors" -eq 0 ]; then
    echo -e "${G}All checks passed.${N}"
    exit 0
else
    echo -e "${R}${errors} check(s) failed.${N}"
    exit 1
fi
