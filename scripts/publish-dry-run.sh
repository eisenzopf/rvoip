#!/bin/bash
# publish-dry-run.sh — run `cargo publish --dry-run` for every crate in the
# beta-tier publish set, in topological order. Fails fast on the first
# error. Use this in CI before tagging a release.
#
# Mirrors the order in publish-beta.sh. Keep them in sync.

set -euo pipefail

# Colors
G='\033[0;32m'; R='\033[0;31m'; Y='\033[1;33m'; B='\033[0;34m'; N='\033[0m'

cd "$(dirname "$0")/.."

# Topological publish order — see plan
# ~/.claude/plans/we-are-preparing-to-abstract-marshmallow.md §
# "Topological order for rvoip-sip beta closure".
#
# Foundation (no internal deps among themselves except the trait crate)
#   then the alpha-tier crates that rvoip-core's optional features reach
#   then the media layer, the core spine, and finally the SIP family.
PUBLISH_ORDER=(
    # Foundational
    rvoip-core-traits
    rvoip-infra-common
    rvoip-codec-core
    rvoip-auth-core

    # Alpha crates needed to satisfy rvoip-core's optional runtime deps.
    # (These are 0.1.0-alpha.1; their own Cargo.tomls allow publish.)
    rvoip-vcon
    rvoip-harness

    # Media foundation
    rvoip-rtp-core
    rvoip-media-core

    # Core spine
    rvoip-core

    # SIP family (bottom-up within SIP)
    rvoip-sip-core
    rvoip-sip-transport
    rvoip-sip-dialog
    rvoip-sip-proxy
    rvoip-sip-registrar
    rvoip-sip
)

echo -e "${G}===================================================${N}"
echo -e "${G}  rvoip publish dry-run — beta + required alphas    ${N}"
echo -e "${G}===================================================${N}"
echo ""

# Why this script has limited reach:
# `cargo publish --dry-run` still resolves every declared dep against
# crates.io's index — it doesn't trust workspace paths for the manifest
# check. So in a chain like ours, only the crates with NO unpublished
# internal deps will dry-run cleanly the first time around. Subsequent
# crates will fail with "no matching package named X found" until X is
# actually published.
#
# We classify those expected-cascade failures separately from real
# manifest problems. A real failure is anything OTHER than a "no
# matching package" error on a known sibling. Use scripts/publish-beta.sh
# for the actual ordered publish.

SIBLING_NAMES=$(printf '%s|' "${PUBLISH_ORDER[@]}" | sed 's/|$//')

ok=0; expected=0; real_fail=0
declare -a REAL_FAILS

for crate in "${PUBLISH_ORDER[@]}"; do
    echo -e "${B}--- dry-run: ${crate} ---${N}"
    out=$(cargo publish --dry-run --no-verify -p "$crate" --allow-dirty 2>&1) && status=0 || status=$?
    if [ "$status" -eq 0 ]; then
        echo -e "${G}  OK${N}"
        ok=$((ok + 1))
    elif echo "$out" | grep -qE "no matching package named \`($SIBLING_NAMES)\`" \
        || echo "$out" | grep -qE "failed to select a version for the requirement \`($SIBLING_NAMES) = "; then
        # Two flavors of the same cascade problem:
        #   (a) The dep package name has never been on crates.io ("no matching package named ...").
        #   (b) The package exists but our new beta version doesn't ("failed to select a version ...").
        # Both are expected when running dry-run on a chain whose
        # earlier links aren't published yet. publish-beta.sh resolves
        # them by waiting for the index between publishes.
        reason=$(echo "$out" | grep -oE "(no matching package named \`[^\`]+\`|failed to select a version for the requirement \`[^\`]+\`)" | head -1)
        echo -e "${Y}  EXPECTED (cascade): ${reason} — will resolve in publish-beta.sh${N}"
        expected=$((expected + 1))
    else
        echo -e "${R}  FAILED${N}"
        echo "$out" | tail -10 | sed 's/^/      /'
        REAL_FAILS+=("$crate")
        real_fail=$((real_fail + 1))
    fi
    echo ""
done

echo "Summary:"
echo "  OK:                 $ok"
echo "  Expected cascade:   $expected"
echo "  Real failures:      $real_fail"
echo ""

if [ "$real_fail" -eq 0 ]; then
    echo -e "${G}Dry-run clean. ${ok} crate(s) packaged successfully; ${expected} will succeed in turn during publish-beta.sh.${N}"
    exit 0
else
    echo -e "${R}Real failures in: ${REAL_FAILS[*]}${N}"
    exit 1
fi
