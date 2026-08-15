#!/usr/bin/env bash
# Build TS 26.173's own encoder and generate the AMR-WB encoder ground truth.
#
# The decoder had it easy: the `.amr` fixtures were already the input, and the
# reference decoder supplied the output. An encoder needs the opposite pair —
# a committed PCM input and the bitstream the normative encoder produces from
# it — and neither existed.
#
# Both are generated here and committed, because they are output rather than
# 3GPP source. The reference itself is fetched by build-amr-reference.sh and
# never committed (3GPP permits in-house use for product design, not
# redistribution).
#
# The input is a deterministic pseudo-speech signal, the same generator the
# Apache-2.0 oracle uses at 16 kHz. It is committed as PCM rather than
# regenerated in Rust on purpose: the formula uses `sin`, and libm's last bit
# is not guaranteed to agree between C and Rust, so regenerating it would make
# every encoder comparison depend on a floating-point coincidence.
#
# Usage: build-amrwb-encoder-reference.sh [workdir]
set -euo pipefail

WORK="${1:-${TMPDIR:-/tmp}/rvoip-amr-reference}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TESTDATA="$HERE/../src/codecs/amr/testdata"
SRC="$(dirname "$(find "$WORK" -name isp_az.c 2>/dev/null | head -1)")"

if [ ! -d "$SRC" ]; then
  echo "TS 26.173 source missing; run build-amr-reference.sh first" >&2
  exit 1
fi

FRAMES=50

echo "==> generating the deterministic input signal"
cat > "$WORK/gen_input.c" <<'CEOF'
/* The same pseudo-speech generator amr_gen_vectors.c uses, at 16 kHz: two
 * formant-ish tones under a 3 Hz envelope. Real spectral structure for the LP
 * analysis and the pitch search to work on, and reproducible without shipping
 * an audio file. */
#include <stdio.h>
#include <stdlib.h>
#include <math.h>
int main(int argc, char **argv) {
    int frames = atoi(argv[2]);
    FILE *f = fopen(argv[1], "wb");
    for (int n = 0; n < frames; n++) {
        for (int i = 0; i < 320; i++) {
            double t = (double)(n * 320 + i) / 16000.0;
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
cc -O2 -o "$WORK/gen_input" "$WORK/gen_input.c" -lm
"$WORK/gen_input" "$TESTDATA/amrwb_enc_input.pcm" "$FRAMES"
ls -l "$TESTDATA/amrwb_enc_input.pcm" | awk '{print "    " $5 " bytes"}'

echo "==> building the reference encoder"
# Everything except decoder.c, which carries the decoder's own main().
# shellcheck disable=SC2046
cc -O1 -w -I"$SRC" -o "$WORK/amrwb_enc" \
   $(ls "$SRC"/*.c | grep -v '/decoder\.c$') -lm

echo "==> encoding at every rate, DTX off"
for m in 0 1 2 3 4 5 6 7 8; do
  "$WORK/amrwb_enc" -mime "$m" "$TESTDATA/amrwb_enc_input.pcm" \
      "$TESTDATA/amrwb_enc_mode$m.amr" >/dev/null 2>&1
done
ls -l "$TESTDATA"/amrwb_enc_mode8.amr | awk '{print "    mode 8: " $5 " bytes"}'

echo "==> sanity check: the reference decodes its own output back"
# A fixture that does not survive the reference's own round trip is not ground
# truth, it is a bug in this script.
for m in 0 1 2 3 4 5 6 7 8; do
  "$WORK/amrwb_dec" -mime "$TESTDATA/amrwb_enc_mode$m.amr" \
      "$WORK/roundtrip$m.pcm" >/dev/null 2>&1
  want=$(( FRAMES * 640 ))
  got=$(wc -c < "$WORK/roundtrip$m.pcm")
  if [ "$got" -ne "$want" ]; then
    echo "    mode $m round trip gave $got bytes, want $want" >&2
    exit 1
  fi
done
echo "    all nine modes decode back to $(( FRAMES * 320 )) samples"

echo "==> capturing a per-stage trace as a committed fixture"
# Three frames at 12.65 kbit/s: enough to exercise every frame- and
# subframe-level stage with real state carried between them, small enough to
# commit. The full trace, for any mode, is a script away
# (tools/trace-amrwb-encoder.sh) and is the instrument to reach for when the
# assembled encoder is wrong.
"$HERE/trace-amrwb-encoder.sh" 2 >/dev/null
TRACE="${TMPDIR:-/tmp}/rvoip-amrwb-enc-trace/trace.txt"
awk '$2 < 3' "$TRACE" > "$TESTDATA/wb_enc_trace.txt"
wc -l < "$TESTDATA/wb_enc_trace.txt" | xargs echo "    lines:"
ls -l "$TESTDATA/wb_enc_trace.txt" | awk '{print "    " $5 " bytes"}'

echo
echo "==> the full trace, for any mode:"
echo "    tools/trace-amrwb-encoder.sh <mode>"
echo
echo "==> verify with:"
echo "    cargo test -p rvoip-codec-core --all-features amr::wb::enc"
