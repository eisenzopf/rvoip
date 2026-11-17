#!/usr/bin/env python3
"""Keep a bench host's evidence disk safe during long (180 s) trials.

Ported from the E74 harness (disk_guard.py). A sustained trial at ~5k sps
leaves about 8 GiB of evidence; a full root disk stops the agent and the
orchestrator mid-search. RFC7502_BENCHMARK.md §2: refuse a trial under
20 GiB free on any host.

usage: disk_guard.py <path> [--min-gib 20]     exit 0 when enough is free, 1 otherwise

  require_free(path, minimum_gib)  raises DiskGuardError below the minimum
  prune_unpacked(dirs, ...)        after a trial's archives are stored and
                                   verified, delete large unpacked copies
Standard library only.
"""
import argparse
import json
from pathlib import Path
import shutil
import sys

MINIMUM_FREE_GIB = 20
BIG_BYTES = 2 * 1024 * 1024


class DiskGuardError(RuntimeError):
    """Not enough free disk to start a trial."""


def free_gib(path):
    return shutil.disk_usage(str(path)).free / 2 ** 30


def require_free(path, minimum_gib=MINIMUM_FREE_GIB):
    """Free GiB under `path`; raises DiskGuardError below `minimum_gib`."""
    free = free_gib(path)
    if free < minimum_gib:
        raise DiskGuardError('refusing to start: %.1f GiB free under %s (< %s GiB); prune verified evidence first'
                             % (free, path, minimum_gib))
    return free


def prune_unpacked(directories, min_bytes=BIG_BYTES, keep=()):
    """Delete regular files of at least `min_bytes` under `directories` (never symlinks, never names in `keep`).
    Only call it once the archives these files came from are stored and their sha256 verified. Returns bytes deleted."""
    trimmed = 0
    for directory in directories:
        directory = Path(directory)
        if not directory.is_dir():
            continue
        for path in sorted(directory.rglob('*')):
            if path.name in keep or path.is_symlink() or not path.is_file():
                continue
            size = path.stat().st_size
            if size >= min_bytes:
                path.unlink()
                trimmed += size
    return trimmed


def main(argv=None):
    parser = argparse.ArgumentParser(description='Refuse a trial when a host has too little free disk.')
    parser.add_argument('path')
    parser.add_argument('--min-gib', type=float, default=MINIMUM_FREE_GIB)
    args = parser.parse_args(argv)
    free = free_gib(args.path)
    ok = free >= args.min_gib
    print(json.dumps({'path': args.path, 'free_gib': round(free, 1), 'minimum_gib': args.min_gib, 'ok': ok}))
    return 0 if ok else 1


if __name__ == '__main__':
    sys.exit(main())
