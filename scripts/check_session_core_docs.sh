#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "Checking session-core rustdoc quality..."

cargo test -p rvoip-session-core --lib --locked
cargo test -p rvoip-session-core --doc --locked
RUSTDOCFLAGS="-D warnings" cargo doc -p rvoip-session-core --no-deps --locked

echo "session-core rustdoc checks passed"
