#!/usr/bin/env bash
# Public API fence for the signaling cleanup.
#
# The compiler fixture is mandatory and uses only the workspace toolchain.
# cargo-public-api and cargo-semver-checks add complete structural/semantic
# comparisons when available. The full beta gate exports
# RVOIP_REQUIRE_API_TOOLS=1, which makes a missing or mismatched pinned tool an
# error; development invocations may leave it disabled.

set -euo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
crate_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
workspace_dir=$(CDPATH= cd -- "$crate_dir/../../.." && pwd)
baseline="$crate_dir/public-api/rvoip-sip.txt"
# Latest published tag. Advance after each release so the comparison measures
# the next candidate's drift, not additions that already shipped.
baseline_rev=${RVOIP_SIP_API_BASELINE_REV:-v0.3.7}
require_tools=${RVOIP_REQUIRE_API_TOOLS:-0}
public_api_version=cargo-public-api\ 0.52.0
semver_checks_version=cargo-semver-checks\ 0.49.0
rustdoc_version='rustc 1.97.0-nightly (e22c616e4 2026-04-19)'

cd "$workspace_dir"
cargo test -p rvoip-sip --test public_api_compatibility

missing=()
if command -v cargo-public-api >/dev/null 2>&1; then
    installed_version=$(cargo public-api --version)
    installed_rustdoc_version=$(rustc +nightly --version 2>/dev/null || true)
    if [[ "$installed_version" == "$public_api_version" \
        && "$installed_rustdoc_version" == "$rustdoc_version" ]]; then
        snapshot_dir=$(mktemp -d "${TMPDIR:-/tmp}/rvoip-sip-public-api.XXXXXX")
        trap 'rm -f "$snapshot_dir/default.txt" "$snapshot_dir/docs.txt"; rmdir "$snapshot_dir"' EXIT

        check_snapshot() {
            snapshot_name=$1
            baseline_key=$2
            shift 2
            current="$snapshot_dir/$snapshot_name.txt"
            cargo public-api \
                --manifest-path "$crate_dir/Cargo.toml" \
                -sss \
                --color never \
                "$@" >"$current"
            expected_hash=$(awk -v key="$baseline_key" '$1 == key { print $2 }' "$baseline")
            current_hash=$(git hash-object "$current")
            if [[ -z "$expected_hash" || "$current_hash" != "$expected_hash" ]]; then
                printf 'public API: %s snapshot changed (expected %s, current %s)\n' \
                    "$snapshot_name" "${expected_hash:-missing}" "$current_hash" >&2
                printf 'public API: inspect with cargo public-api, then update the baseline only for an approved API change\n' >&2
                exit 1
            fi
        }

        check_snapshot default default-git-blob
        check_snapshot docs docs-git-blob \
            --features generated-validation,dev-insecure-tls
    else
        printf 'public API: structural snapshot toolchain mismatch; snapshot skipped\n' >&2
        printf '  expected: %s / %s\n  found:    %s / %s\n' \
            "$public_api_version" "$rustdoc_version" \
            "$installed_version" "${installed_rustdoc_version:-nightly unavailable}" >&2
        if [[ "$require_tools" == 1 ]]; then
            exit 1
        fi
    fi
else
    missing+=(cargo-public-api)
fi

if command -v cargo-semver-checks >/dev/null 2>&1; then
    installed_semver_version=$(cargo semver-checks --version)
    if [[ "$installed_semver_version" == "$semver_checks_version" ]]; then
        if git cat-file -e "$baseline_rev^{commit}" 2>/dev/null; then
            cargo semver-checks check-release \
                --package rvoip-sip \
                --baseline-rev "$baseline_rev" \
                --features generated-validation,dev-insecure-tls
        else
            printf 'public API: baseline commit %s is unavailable; semver comparison skipped\n' \
                "$baseline_rev" >&2
            if [[ "$require_tools" == 1 ]]; then
                exit 1
            fi
        fi
    else
        printf 'public API: semantic toolchain mismatch; semver comparison skipped\n' >&2
        printf '  expected: %s\n  found:    %s\n' \
            "$semver_checks_version" "$installed_semver_version" >&2
        if [[ "$require_tools" == 1 ]]; then
            exit 1
        fi
    fi
else
    missing+=(cargo-semver-checks)
fi

if ((${#missing[@]})); then
    printf 'public API: optional tools unavailable (%s); compiler fixture still passed\n' \
        "$(IFS=,; printf '%s' "${missing[*]}")" >&2
    if [[ "$require_tools" == 1 ]]; then
        printf 'public API: RVOIP_REQUIRE_API_TOOLS=1 requires both optional tools\n' >&2
        exit 1
    fi
fi
