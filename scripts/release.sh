#!/usr/bin/env bash
# Single release entrypoint for every publishable rvoip workspace package.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec python3 "$script_dir/release.py" "$@"
