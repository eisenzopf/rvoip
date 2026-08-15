#!/usr/bin/env bash
# Regenerate the AMR-NB VAD2 reference trace from TS 26.073's own vad2().
#
# The committed trace (src/codecs/amr/testdata/amrnb_vad2_trace.txt) is what
# `nb::enc::vad2`'s bit-exactness test compares against, every half-frame and
# on the whole state rather than the decision. Run this after touching the
# probe, or to re-establish the trace on a fresh machine.
#
# The reference sources are fetched, built and never committed -- only their
# generated output is. build-amrnb-reference.sh populates the directory.
set -euo pipefail

REF="${1:-${TMPDIR:-/tmp}/rvoip-amrnb-reference}/c-code"
ROOT="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
CODEC="$ROOT/crates/media/codec-core"
OUT="$CODEC/src/codecs/amr/testdata/amrnb_vad2_trace.txt"

if [ ! -f "$REF/vad2.c" ]; then
  echo "reference not found at $REF -- run tools/build-amrnb-reference.sh first" >&2
  exit 1
fi

BIN="$(mktemp -d)/nb_vad2_probe"
cc -O1 -I"$REF" -o "$BIN" "$CODEC/tools/nb_vad2_probe.c" \
  "$REF/vad2.c" "$REF/r_fft.c" "$REF/basicop2.c" "$REF/oper_32b.c" \
  "$REF/log2.c" "$REF/pow2.c" "$REF/count.c"

"$BIN" "$CODEC/src/codecs/amr/testdata/amrnb_dtx_input.pcm" > "$OUT"
echo "wrote $(grep -vc '^#' "$OUT") half-frames to $OUT"
