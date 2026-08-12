#!/usr/bin/env bash
# Run the AMR fuzz targets for a bounded window each and fail on any finding.
#
# Targets (crates/media/fuzz/fuzz_targets/):
#   amr_unpack — RFC 4867 depacketizer on attacker bytes
#   amr_decode — NB/WB decoder DSP on arbitrary coded bits, concealment mixed in
#   amr_encode — NB/WB encoders on arbitrary PCM, mode switches and DTX in play
#
# Usage:
#   run-amr-fuzz.sh                 # 60s per target, a quick regression pass
#   AMR_FUZZ_SECS=900 run-amr-fuzz.sh   # deeper run, e.g. nightly
#
# Requires: rustup nightly toolchain + cargo-fuzz (cargo install cargo-fuzz).
# libFuzzer exits non-zero on a crash/OOM/leak and writes the reproducer under
# crates/media/fuzz/artifacts/<target>/ — commit nothing from there; minimize
# and turn it into a unit test instead.
set -euo pipefail

SECS="${AMR_FUZZ_SECS:-60}"
ROOT="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
cd "$ROOT"

for target in amr_unpack amr_decode amr_encode; do
  echo "==> $target (${SECS}s)"
  cargo +nightly fuzz run --fuzz-dir crates/media/fuzz "$target" -- \
    -max_total_time="$SECS" -print_final_stats=1 2>&1 | tail -6
done

echo "All AMR fuzz targets completed ${SECS}s each with no findings."
