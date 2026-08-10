#!/usr/bin/env bash
# Build TS 26.073's own encoder and generate the AMR-NB encoder ground truth.
#
# The narrowband twin of build-amrwb-encoder-reference.sh, and for the same
# reason: an encoder needs a committed input and the bitstream the normative
# encoder makes from it, and neither is implied by the decoder fixtures.
#
# The input is the same deterministic pseudo-speech generator at 8 kHz, so both
# variants' encoder tests run on one signal rather than two unrelated ones. It
# is committed as PCM rather than regenerated in Rust because the generator uses
# `sin` and libm's last bit is not guaranteed to agree across languages.
#
# Usage: build-amrnb-encoder-reference.sh [workdir]
set -euo pipefail

WORK="${1:-${TMPDIR:-/tmp}/rvoip-amrnb-reference}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TESTDATA="$HERE/../src/codecs/amr/testdata"
SRC="$WORK/c-code"

if [ ! -d "$SRC" ]; then
  echo "TS 26.073 source missing; run build-amrnb-reference.sh first" >&2
  exit 1
fi

FRAMES=50

echo "==> generating the deterministic input signal"
cat > "$WORK/gen_input_nb.c" <<'CEOF'
/* The 8 kHz form of the generator amr_gen_vectors.c uses: two formant-ish
 * tones under a 3 Hz envelope. Real spectral structure for the LP analysis and
 * the pitch search, reproducible without shipping an audio file. */
#include <stdio.h>
#include <stdlib.h>
#include <math.h>
int main(int argc, char **argv) {
    int frames = atoi(argv[2]);
    FILE *f = fopen(argv[1], "wb");
    for (int n = 0; n < frames; n++) {
        for (int i = 0; i < 160; i++) {
            double t = (double)(n * 160 + i) / 8000.0;
            double env = 0.5 + 0.5 * sin(2.0 * M_PI * 3.0 * t);
            double s = 0.6 * sin(2.0 * M_PI * 310.0 * t) +
                       0.3 * sin(2.0 * M_PI * 1150.0 * t) +
                       0.1 * sin(2.0 * M_PI * 2700.0 * t);
            double v = env * s * 12000.0;
            if (v > 32767.0) v = 32767.0;
            if (v < -32768.0) v = -32768.0;
            short w = (short)v;
            fputc(w & 0xFF, f);
            fputc((w >> 8) & 0xFF, f);
        }
    }
    fclose(f);
    return 0;
}
CEOF
cc -O2 -o "$WORK/gen_input_nb" "$WORK/gen_input_nb.c" -lm
"$WORK/gen_input_nb" "$TESTDATA/amrnb_enc_input.pcm" "$FRAMES"
ls -l "$TESTDATA/amrnb_enc_input.pcm" | awk '{print "    " $5 " bytes"}'

echo "==> building the reference encoder"
# Everything except decoder.c, which carries the decoder's own main().
# -DMMS_IO selects the MIME/storage bitstream format the .amr fixtures use.
# shellcheck disable=SC2046
cc -O1 -w -DMMS_IO -I"$SRC" -o "$WORK/amrnb_enc" \
   $(ls "$SRC"/*.c | grep -v '/decoder\.c$') -lm

echo "==> encoding at every rate, DTX off"
MODES=(MR475 MR515 MR59 MR67 MR74 MR795 MR102 MR122)
for m in 0 1 2 3 4 5 6 7; do
  "$WORK/amrnb_enc" "${MODES[$m]}" "$TESTDATA/amrnb_enc_input.pcm" \
      "$TESTDATA/amrnb_enc_mode$m.amr" >/dev/null 2>&1
done
ls -l "$TESTDATA"/amrnb_enc_mode7.amr | awk '{print "    mode 7: " $5 " bytes"}'

echo "==> sanity check: the reference decodes its own output back"
# A fixture that does not survive the reference's own round trip is not ground
# truth, it is a bug in this script.
for m in 0 1 2 3 4 5 6 7; do
  "$WORK/amrnb_dec" "$TESTDATA/amrnb_enc_mode$m.amr" \
      "$WORK/roundtrip_nb$m.pcm" >/dev/null 2>&1
  want=$(( FRAMES * 320 ))
  got=$(wc -c < "$WORK/roundtrip_nb$m.pcm")
  if [ "$got" -ne "$want" ]; then
    echo "    mode $m round trip gave $got bytes, want $want" >&2
    exit 1
  fi
done
echo "    all eight modes decode back to $(( FRAMES * 160 )) samples"

echo
echo "==> per-stage ground truth comes from the instrumented encoder:"
echo "    tools/trace-amrnb-encoder.sh <mode>"
