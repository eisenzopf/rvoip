#!/usr/bin/env bash
# Run the six normative AMR conformance tests, fetching and building the
# reference trees first if they are not already present.
#
# # Why this is not a CI job
#
# The TS 26.073 and TS 26.173 sequences are 3GPP copyright. Only generated
# output is committed to this repository; the sequences themselves are fetched
# for in-house design and never redistributed. So the tests that consume them
# are `#[ignore]`d and cannot run on a stock CI runner, and the six strongest
# claims in docs/AMR_IMPLEMENTATION_STATUS.md are the ones CI can least
# protect.
#
# What this script buys is that running them is one command rather than a
# procedure someone has to remember: the fetch, the build, the two environment
# variables and the `--ignored` flag are all here.
#
# # What it proves
#
#   AMR-NB encode   spch_dos, 425 frames, every transmitted bit
#   AMR-NB decode   spch_dos, 425 frames, every output sample
#   AMR-WB encode   all nine TS 26.173 vectors
#   AMR-WB decode   all nine, sample for sample
#   AMR-WB DTX enc  tst_md.cod, every frame type and payload
#   AMR-WB DTX dec  tst_md, 200 frames, sample for sample
#
# Anything less than six passing is a failure, and this script says so rather
# than reporting the count cargo happens to run.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../../../.." && pwd)"

WB_REF="${RVOIP_AMRWB_REFERENCE:-${TMPDIR:-/tmp}/rvoip-amr-reference}"
NB_REF="${RVOIP_AMRNB_REFERENCE:-${TMPDIR:-/tmp}/rvoip-amrnb-reference}"
WB_REF="${WB_REF%/}"
NB_REF="${NB_REF%/}"

if [[ ! -f "$WB_REF/testv/tst.inp" ]]; then
  echo "==> fetching and building the AMR-WB reference into $WB_REF"
  "$HERE/build-amr-reference.sh" >/dev/null
fi
if [[ ! -f "$NB_REF/c-code/spch_dos.inp" ]]; then
  echo "==> fetching and building the AMR-NB reference into $NB_REF"
  "$HERE/build-amrnb-encoder-reference.sh" >/dev/null
fi

for required in "$WB_REF/testv/tst.inp" "$NB_REF/c-code/spch_dos.inp"; do
  if [[ ! -f "$required" ]]; then
    echo "missing $required after the build step; cannot run conformance" >&2
    exit 2
  fi
done

echo "==> running the normative sequences"
output="$(cd "$REPO" && RVOIP_AMRWB_REFERENCE="$WB_REF" RVOIP_AMRNB_REFERENCE="$NB_REF" \
  cargo test --locked -p rvoip-codec-core --all-features --lib -- \
  --ignored --exact \
  codecs::amr::conformance::tests::narrowband_encoder_matches_the_reference_vectors \
  codecs::amr::conformance::tests::narrowband_decoder_matches_the_reference_vectors \
  codecs::amr::conformance::tests::wideband_encoder_matches_the_normative_vectors \
  codecs::amr::conformance::tests::wideband_decoder_matches_the_normative_vectors \
  codecs::amr::conformance::tests::wideband_dtx_matches_the_normative_vector \
  codecs::amr::conformance::tests::wideband_dtx_decoding_matches_the_normative_vector \
  2>&1)"

echo "$output" | grep -E "^test codecs::amr::conformance|^test result" || true

passed="$(echo "$output" | grep -cE '^test codecs::amr::conformance.* \.\.\. ok$' || true)"
if [[ "$passed" -ne 6 ]]; then
  echo >&2
  echo "expected six conformance tests to pass, saw $passed" >&2
  echo "a conformance run that checks fewer than everything is not a pass" >&2
  echo "$output" >&2
  exit 1
fi

echo
echo "all six normative AMR sequences pass"
