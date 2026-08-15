#!/usr/bin/env bash
# Run the AMR `#[ignore]`d verification tests that are NOT the six normative
# conformance sequences.
#
# # Why this exists
#
# `run-amr-conformance.sh` has a strict contract — exactly six normative
# sequences pass or the run fails — and diluting it with anything else would
# weaken the strongest claim in the status doc. But four more `#[ignore]`d
# tests existed with *no* runner at all, which is the shape of rot this branch
# keeps finding: work that exists and is never checked. This script is their
# one command.
#
# # What it runs by default
#
#   nb  independently_reproduce_the_reference_files   committed testdata
#   wb  independently_reproduce_the_reference_files   committed testdata
#
# Both rebuild a reference-produced storage file byte for byte — magic and
# table of contents included — from committed PCM, written independently of
# the encoder assemblies' own tests. Anything other than exactly two passes is
# a failure.
#
# # --with-traces [DIR]
#
# Additionally regenerates the narrowband per-stage traces with
# `trace-amrnb-encoder.sh` (which fetches and builds the TS 26.073 reference
# if needed) and runs
#
#   nb  all_rates_all_frames_against_regenerated_traces   RVOIP_NB_TRACE_DIR
#
# raising the required pass count to three.
#
# # The diagnostics this deliberately does not run
#
# Three more `#[ignore]`d tests print rather than judge, so counting them as
# "passes" would be theater. Run them by hand when debugging a divergence:
#
#   cargo test -p rvoip-codec-core --all-features --lib -- --ignored --exact \
#     codecs::amr::nb::decoder::tests::nb_where_does_it_diverge --nocapture
#   AMR_TRACE_MODE=4 cargo test -p rvoip-codec-core --all-features --lib -- \
#     --ignored --exact codecs::amr::nb::decoder::tests::nb_dump_trace --nocapture
#   cargo test -p rvoip-codec-core --all-features --lib -- --ignored --exact \
#     codecs::amr::wb::decoder::tests::where_does_it_diverge --nocapture

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../../../.." && pwd)"

with_traces=0
trace_dir=""
if [[ "${1:-}" == "--with-traces" ]]; then
  with_traces=1
  trace_dir="${2:-${TMPDIR:-/tmp}/rvoip-nb-traces}"
fi

tests=(
  codecs::amr::nb::enc::encoder::tests::independently_reproduce_the_reference_files
  codecs::amr::wb::enc::encoder::tests::independently_reproduce_the_reference_files
)
required=2
env_args=()

if [[ "$with_traces" -eq 1 ]]; then
  echo "==> regenerating narrowband traces into $trace_dir"
  mkdir -p "$trace_dir"
  for mode in 0 1 2 3 4 5 6 7; do
    if [[ ! -f "$trace_dir/nbtrace$mode/trace.txt" ]]; then
      "$HERE/trace-amrnb-encoder.sh" "$mode" "$trace_dir/nbtrace$mode"
    fi
  done
  tests+=(codecs::amr::nb::enc::encoder::tests::all_rates_all_frames_against_regenerated_traces)
  required=3
  env_args=(RVOIP_NB_TRACE_DIR="$trace_dir")
fi

echo "==> running ${#tests[@]} verification tests"
# The `+` expansion keeps macOS bash 3.2's `set -u` happy on an empty array.
output="$(cd "$REPO" && env ${env_args[@]+"${env_args[@]}"} \
  cargo test --locked -p rvoip-codec-core --all-features --lib -- \
  --ignored --exact "${tests[@]}" 2>&1)"

echo "$output" | grep -E "^test codecs::amr|^test result" || true

passed="$(echo "$output" | grep -cE '^test codecs::amr.* \.\.\. ok$' || true)"
if [[ "$passed" -ne "$required" ]]; then
  echo >&2
  echo "expected $required verification tests to pass, saw $passed" >&2
  echo "a verification run that checks fewer than everything is not a pass" >&2
  echo "$output" >&2
  exit 1
fi

echo
echo "all $required AMR verification tests pass"
