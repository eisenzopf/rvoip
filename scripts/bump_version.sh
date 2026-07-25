#!/usr/bin/env bash
# Deprecated compatibility wrapper. Use scripts/release.sh prepare.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ $# -ne 1 ]]; then
  echo "usage: $0 <X.Y.Z>" >&2
  echo "replacement: scripts/release.sh prepare --version X.Y.Z" >&2
  exit 2
fi
echo "NOTICE: bump_version.sh is deprecated; using the unified release tool." >&2
exec "$script_dir/release.sh" prepare --version "$1"
