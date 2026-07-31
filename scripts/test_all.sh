#!/usr/bin/env bash

# Optional local workspace check. CI is the release authority; this script is
# deliberately small so it does not rediscover targets or run the same crate
# twice merely to determine whether a target exists.
set -euo pipefail

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"

run() {
  printf '\n==>'
  printf ' %q' "$@"
  printf '\n'
  "$@"
}

if [[ "${RVOIP_USE_NEXTEST:-0}" == "1" ]]; then
  command -v cargo-nextest >/dev/null 2>&1 || {
    echo "RVOIP_USE_NEXTEST=1 requires cargo-nextest" >&2
    exit 2
  }
  run cargo nextest run --workspace --locked --profile ci
else
  run cargo test --workspace --locked --lib --tests --bins --examples
fi

# Doctests are intentionally separate because cargo-nextest does not execute
# them and they have different compiler/test-harness behavior.
run cargo test --workspace --doc --locked

if [[ "${RVOIP_SKIP_CLIPPY:-0}" != "1" ]]; then
  run cargo clippy --workspace --all-targets --locked
fi

echo
echo "Workspace checks passed. For PR/release decisions, use GitHub's PR Gate or Main Gate receipts."
