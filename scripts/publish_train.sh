#!/usr/bin/env bash
# Deprecated compatibility wrapper. Split alpha/beta trains no longer exist.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [[ $# -eq 1 && "$1" == "--audit" ]]; then
  echo "NOTICE: publish_train.sh is deprecated; using the unified release audit." >&2
  exec "$script_dir/release.sh" audit
fi

for argument in "$@"; do
  case "$argument" in
    --train|alpha|beta|--closure|--no-bump|--allow-dirty|--skip-validate)
      echo "error: split alpha/beta publication is retired." >&2
      echo "use: scripts/release.sh publish --version X.Y.Z [--execute]" >&2
      exit 2
      ;;
  esac
done

echo "NOTICE: publish_train.sh is deprecated; publishing the unified workspace." >&2
exec "$script_dir/release.sh" publish "$@"
