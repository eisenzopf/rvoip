#!/bin/bash
# publish-beta.sh — actually publish the beta-tier crates to crates.io,
# in topological order. After each publish, poll the registry until the
# new version is indexed (so the next dependent crate can resolve it).
#
# REQUIRES: `cargo login` with a token that owns or can claim the
# crate names. Run scripts/publish-dry-run.sh first.
#
# Usage:
#   scripts/publish-beta.sh           # publishes everything in order
#   scripts/publish-beta.sh --resume rvoip-media-core
#                                     # skip ahead to this crate (use
#                                     # when an earlier crate already
#                                     # made it onto crates.io)

set -euo pipefail

G='\033[0;32m'; R='\033[0;31m'; Y='\033[1;33m'; B='\033[0;34m'; N='\033[0m'

cd "$(dirname "$0")/.."

PUBLISH_ORDER=(
    rvoip-core-traits
    rvoip-infra-common
    rvoip-codec-core
    rvoip-auth-core
    rvoip-vcon            # alpha, but publishes (rvoip-core optional dep)
    rvoip-harness         # alpha, but publishes (rvoip-core optional dep)
    rvoip-rtp-core
    rvoip-media-core
    rvoip-core
    rvoip-sip-core
    rvoip-sip-transport
    rvoip-sip-dialog
    rvoip-sip-proxy
    rvoip-sip-registrar
    rvoip-sip
)

RESUME_FROM=""
if [ "${1:-}" = "--resume" ] && [ -n "${2:-}" ]; then
    RESUME_FROM="$2"
    echo -e "${Y}Resuming from: ${RESUME_FROM}${N}"
fi

# Wait until crates.io shows the just-published version. Index propagation
# can take a few seconds after `cargo publish` returns — without this loop
# the next dependent crate will fail with "no matching package".
wait_for_index() {
    local crate="$1"
    local want_version="$2"
    local tries=60
    echo -e "${Y}  Waiting for crates.io to index ${crate} ${want_version}...${N}"
    for ((i = 1; i <= tries; i++)); do
        # `cargo search` returns the latest published version. For pre-release
        # versions it may not show as "latest" — fall back to cargo info.
        if cargo info "$crate" 2>/dev/null | grep -q "$want_version"; then
            echo -e "${G}  Indexed.${N}"
            return 0
        fi
        sleep 5
    done
    echo -e "${R}  Timed out waiting for ${crate} ${want_version} on crates.io.${N}"
    return 1
}

skip=0
[ -n "$RESUME_FROM" ] && skip=1

for crate in "${PUBLISH_ORDER[@]}"; do
    if [ "$skip" -eq 1 ]; then
        if [ "$crate" = "$RESUME_FROM" ]; then
            skip=0
        else
            echo -e "${Y}skipping ${crate} (resuming from ${RESUME_FROM})${N}"
            continue
        fi
    fi

    # Look up the version cargo will publish for this crate.
    version=$(cargo metadata --no-deps --format-version 1 \
        | jq -r --arg n "$crate" '.packages[] | select(.name == $n) | .version')

    echo ""
    echo -e "${B}=========================================${N}"
    echo -e "${B}  Publishing ${crate} ${version}${N}"
    echo -e "${B}=========================================${N}"

    cargo publish -p "$crate"

    wait_for_index "$crate" "$version"
done

echo ""
echo -e "${G}All beta crates published.${N}"
