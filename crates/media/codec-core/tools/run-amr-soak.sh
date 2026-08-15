#!/usr/bin/env bash
# Run the AMR long-run stability tests.
#
# # What it proves
#
# Both variants encode and decode a continuous call with the rate changing
# every 7 frames, DTX enabled, and packets dropped throughout (an isolated
# frame every 43, a 3-frame burst every 349). The assertions are the ones that
# only a long run can make: resident memory stays flat, every frame comes back
# full-length, and the decoded stream never degrades into silence.
#
# The state-drift failure mode this is aimed at does not announce itself. A
# gain or LSF predictor walking toward saturation over thousands of frames
# produces audio that gets quietly worse, which is why the silence check is a
# rolling one-second window rather than a per-frame assertion.
#
# # Usage
#
#   run-amr-soak.sh                    # 30 speech-seconds per variant (~10s wall)
#   AMR_SOAK_SECS=3600 run-amr-soak.sh # one hour of speech per variant
#
# The tests are `#[ignore]`d, so they never run in a normal `cargo test`.
#
# # Baseline (2026-08-12, M-series macOS)
#
# The memory claim is not "RSS was small once" — it is "RSS does not grow with
# frame count", which needs runs of different lengths to say anything. Release
# build, --test-threads=1:
#
#   AMR_SOAK_SECS      frames/variant   NB rss (kB)     WB rss (kB)    wall
#   300                15 000           2656 -> 3632    3744 -> 4368   9.8s
#   3000               150 000          2656 -> 3616    3728 -> 4320   98s
#
# 100x the frames, the same ~1 MB delta and the same peak: the growth is
# process startup and allocator warm-up, not the codecs. Debug builds sit
# ~2 MB higher throughout and are ~10x slower; use release for anything above
# a few hundred seconds.
#
# Concealment and DTX are exercised throughout, not sampled: the 3000s run
# concealed 4746 frames and emitted ~15 000 non-speech (SID/NO_DATA) frames
# per variant.
set -euo pipefail

SECS="${AMR_SOAK_SECS:-30}"
PROFILE_ARG=""
if [ "${CARGO_PROFILE:-}" = "release" ]; then
  PROFILE_ARG="--release"
fi

ROOT="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
cd "$ROOT"

echo "AMR soak: ${SECS} speech-seconds per variant${PROFILE_ARG:+ (release)}"
RVOIP_AMR_SOAK_SECS="$SECS" cargo test -p rvoip-codec-core --all-features \
  ${PROFILE_ARG:+$PROFILE_ARG} --lib -- --ignored --nocapture --test-threads=1 soak
