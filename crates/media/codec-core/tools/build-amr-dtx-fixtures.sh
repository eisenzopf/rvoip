#!/usr/bin/env bash
# Generate the DTX ground truth for both variants: an input signal that actually
# goes quiet, and what the reference encoders and decoders make of it with DTX
# enabled.
#
# # Why a new signal
#
# Every committed fixture is speech from end to end. The encoders were driven
# without `-dtx`, so `tx_dtx_handler` has never run, no SID frame has ever been
# produced, and the wideband VAD -- though fully implemented and on the
# byte-exactness path, since its flag is codec bit 0 -- is only weakly
# differentiated by those fixtures: byte-exactness would still pass with the
# detector wired to a constant. A signal with real silence in it is what closes
# both gaps at once.
#
# # The shape of the signal, and why each part is there
#
#   frames   0-19  low-level shaped noise      cold start: the first SID_FIRST,
#                                              the first SID_UPDATE, two full
#                                              8-frame update periods
#   frames  20-49  pseudo-speech at full level VAD to speech, hangover resets
#   frames  50-119 noise again                 the 7-frame hangover, then
#                                              SID_FIRST, NO_DATA, SID_UPDATE,
#                                              and eight more periods
#   frames 120-149 pseudo-speech               the DTX->speech transition
#
# The long quiet run is 70 frames rather than 40 on purpose. `DTX_MUTE` engages
# after `DTX_MAX_EMPTY_THRESH` == 50 frames with no SID update, and a clean
# encoder sends one every 8 -- so the mute path is unreachable unless a stream
# both stays quiet long enough and loses its SIDs. The `_mute` fixture below
# supplies the second half of that; this run supplies the first.
#
# Not digital silence: `Log2(0)` is a degenerate corner, and an all-0x0008 frame
# would trip the encoder's own homing-frame test. The noise is a deterministic
# LCG, shaped, at roughly 1/300th of the speech level.
#
# # Wideband mode 8 is not optional
#
# At 23.85 kbit/s the high-band gain index depends on `dtxHangoverCount`, which
# never leaves its constant while DTX is off. So mode 8's bitstream differs
# between DTX on and off *on ordinary speech frames following silence*, and a
# fixture that skipped it would leave that path untested.
#
# Usage: build-amr-dtx-fixtures.sh
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TESTDATA="$HERE/../src/codecs/amr/testdata"
NB_WORK="${TMPDIR:-/tmp}/rvoip-amrnb-reference"
WB_WORK="${TMPDIR:-/tmp}/rvoip-amr-reference"

for pair in "$NB_WORK/amrnb_enc:build-amrnb-encoder-reference.sh" \
            "$WB_WORK/amrwb_enc:build-amrwb-encoder-reference.sh"; do
  bin="${pair%%:*}"
  script="${pair##*:}"
  test -x "$bin" || { echo "missing $bin; run $script first" >&2; exit 1; }
done

FRAMES=150

echo "==> generating the DTX input signal"
# In C, and committed as PCM, for the same reason as the speech generator next
# door: the envelope uses `sin` and libm's last bit is not guaranteed to agree
# across languages, so regenerating it in Rust would quietly change the fixture.
cat > "$NB_WORK/gen_dtx_input.c" <<'CEOF'
#include <stdio.h>
#include <stdlib.h>
#include <math.h>

/* Deterministic, and deliberately not libc's rand(): the same sequence has to
 * come out on every machine that regenerates the fixture. */
static unsigned int lcg = 12345u;
static double noise(void) {
    lcg = lcg * 1103515245u + 12345u;
    return (double)((int)((lcg >> 16) & 0x7FFF) - 16384) / 16384.0;
}

int main(int argc, char **argv) {
    int rate = atoi(argv[2]);          /* 8000 or 16000 */
    int frames = atoi(argv[3]);
    int per_frame = rate / 50;         /* 20 ms */
    FILE *f = fopen(argv[1], "wb");
    for (int n = 0; n < frames; n++) {
        /* Speech for 20..49 and 90..119, background otherwise. */
        int speech = (n >= 20 && n < 50) || (n >= 120);
        for (int i = 0; i < per_frame; i++) {
            double t = (double)(n * per_frame + i) / (double)rate;
            double v;
            if (speech) {
                double env = 0.5 + 0.5 * sin(2.0 * M_PI * 3.0 * t);
                double s = 0.6 * sin(2.0 * M_PI * 310.0 * t) +
                           0.3 * sin(2.0 * M_PI * 1150.0 * t) +
                           0.1 * sin(2.0 * M_PI * 2700.0 * t);
                v = env * s * 12000.0;
            } else {
                /* Shaped background: a one-pole low pass over the LCG, so the
                 * noise estimator sees a spectrum rather than white noise, at
                 * a level the VAD reads as silence. */
                static double state = 0.0;
                state = 0.85 * state + 0.15 * noise();
                v = state * 40.0;
            }
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
cc -O2 -o "$NB_WORK/gen_dtx_input" "$NB_WORK/gen_dtx_input.c" -lm
"$NB_WORK/gen_dtx_input" "$TESTDATA/amrnb_dtx_input.pcm" 8000 "$FRAMES"
"$NB_WORK/gen_dtx_input" "$TESTDATA/amrwb_dtx_input.pcm" 16000 "$FRAMES"
ls -l "$TESTDATA/amrnb_dtx_input.pcm" "$TESTDATA/amrwb_dtx_input.pcm" \
  | awk '{print "    " $9 ": " $5 " bytes"}'

# The signal must not be digital silence anywhere, or the encoder's homing test
# fires and Log2 hits its degenerate corner.
python3 - "$TESTDATA/amrnb_dtx_input.pcm" "$TESTDATA/amrwb_dtx_input.pcm" <<'SILEOF'
import struct, sys
for path in sys.argv[1:]:
    raw = open(path, "rb").read()
    samples = struct.unpack(f"<{len(raw)//2}h", raw)
    per_frame = len(samples) // 150
    peaks = [max(abs(s) for s in samples[f*per_frame:(f+1)*per_frame]) for f in range(150)]
    quiet = [p for f, p in enumerate(peaks) if not (20 <= f < 50 or f >= 120)]
    loud = [p for f, p in enumerate(peaks) if 20 <= f < 50 or f >= 120]
    if min(quiet) == 0:
        sys.exit(f"    {path}: a background frame is digital silence")
    # The speech section carries a 3 Hz envelope that passes through zero, so
    # a few of its frames are legitimately quiet -- that is a talk spurt with
    # gaps in it, which is more interesting for the VAD than a flat one, not a
    # defect. What has to hold is that the background never approaches the
    # loudest speech.
    if max(quiet) * 20 > max(loud):
        sys.exit(f"    {path}: background peak {max(quiet)} is not far below speech {max(loud)}")
    print(f"    {path.split('/')[-1]}: background peaks <= {max(quiet)}, "
          f"speech peaks up to {max(loud)}")
SILEOF

# Frame-type histogram of a MIME/storage .amr, and the transitions in it. This
# is what stops the fixture from being vacuous: a DTX fixture in which the
# encoder never actually entered DTX looks exactly like one that works.
histogram() {
  python3 - "$1" "$2" <<'HISTEOF'
import sys
path, variant = sys.argv[1], sys.argv[2]
NB = [12, 13, 15, 17, 19, 20, 26, 31, 5] + [0] * 7
WB = [17, 23, 32, 36, 40, 46, 50, 58, 60, 5] + [0] * 6
sizes, magic = (NB, 6) if variant == "nb" else (WB, 9)
sid_ft = 8 if variant == "nb" else 9

data = open(path, "rb").read()
pos, kinds = magic, []
while pos < len(data):
    ft = (data[pos] >> 3) & 0x0F
    kinds.append(ft)
    pos += 1 + sizes[ft]

speech = sum(1 for k in kinds if k < sid_ft)
sid = sum(1 for k in kinds if k == sid_ft)
nodata = sum(1 for k in kinds if k == 15)
# A talk spurt ending: speech followed by anything that is not speech.
to_dtx = sum(1 for a, b in zip(kinds, kinds[1:]) if a < sid_ft and b >= sid_ft)
to_speech = sum(1 for a, b in zip(kinds, kinds[1:]) if a >= sid_ft and b < sid_ft)
print(f"    {len(kinds)} frames: {speech} speech, {sid} SID, {nodata} NO_DATA, "
      f"{to_dtx} speech->DTX, {to_speech} DTX->speech")

problems = []
if sid < 2:
    problems.append(f"only {sid} SID frames")
if nodata < 60:
    problems.append(f"only {nodata} NO_DATA frames")
if to_dtx < 2:
    problems.append(f"only {to_dtx} speech->DTX transitions")
if to_speech < 1:
    problems.append("no DTX->speech transition")
if problems:
    sys.exit("    fixture is vacuous: " + "; ".join(problems))
HISTEOF
}

echo "==> AMR-NB: encoding at every rate, DTX on"
NB_MODES=(MR475 MR515 MR59 MR67 MR74 MR795 MR102 MR122)
for m in 0 1 2 3 4 5 6 7; do
  "$NB_WORK/amrnb_enc" -dtx "${NB_MODES[$m]}" "$TESTDATA/amrnb_dtx_input.pcm" \
      "$TESTDATA/amrnb_dtx_mode$m.amr" >/dev/null 2>&1
  "$NB_WORK/amrnb_dec" "$TESTDATA/amrnb_dtx_mode$m.amr" \
      "$TESTDATA/amrnb_dtx_mode$m.pcm" >/dev/null 2>&1
done
histogram "$TESTDATA/amrnb_dtx_mode4.amr" nb

echo "==> AMR-WB: encoding at every rate, DTX on"
for m in 0 1 2 3 4 5 6 7 8; do
  "$WB_WORK/amrwb_enc" -dtx -mime "$m" "$TESTDATA/amrwb_dtx_input.pcm" \
      "$TESTDATA/amrwb_dtx_mode$m.amr" >/dev/null 2>&1
  "$WB_WORK/amrwb_dec" -mime "$TESTDATA/amrwb_dtx_mode$m.amr" \
      "$TESTDATA/amrwb_dtx_mode$m.pcm" >/dev/null 2>&1
done
histogram "$TESTDATA/amrwb_dtx_mode2.amr" wb

echo "==> the DTX-on bitstreams must differ from the DTX-off ones"
# Otherwise `-dtx` did nothing and every DTX test would pass against a fixture
# that never entered DTX. Wideband mode 8 is checked explicitly because it is
# the one rate where the difference reaches ordinary speech frames.
for m in 0 1 2 3 4 5 6 7; do
  "$NB_WORK/amrnb_enc" "${NB_MODES[$m]}" "$TESTDATA/amrnb_dtx_input.pcm" \
      "$NB_WORK/nodtx_mode$m.amr" >/dev/null 2>&1
  if cmp -s "$NB_WORK/nodtx_mode$m.amr" "$TESTDATA/amrnb_dtx_mode$m.amr"; then
    echo "    AMR-NB mode $m: -dtx changed nothing" >&2
    exit 1
  fi
done
for m in 0 1 2 3 4 5 6 7 8; do
  "$WB_WORK/amrwb_enc" -mime "$m" "$TESTDATA/amrwb_dtx_input.pcm" \
      "$WB_WORK/nodtx_mode$m.amr" >/dev/null 2>&1
  if cmp -s "$WB_WORK/nodtx_mode$m.amr" "$TESTDATA/amrwb_dtx_mode$m.amr"; then
    echo "    AMR-WB mode $m: -dtx changed nothing" >&2
    exit 1
  fi
done
echo "    all seventeen rates differ with DTX on"

echo "==> the narrowband fixture must be able to tell VAD1 from VAD2"
# AMR-NB's VAD decision appears nowhere in the bitstream. Its only observable is
# which frames become SID or NO_DATA, and that is filtered through a 7-frame
# hangover -- so a VAD that is wrong in the middle of a talk spurt is invisible.
# Before the port is written, establish that this signal is one the two
# detectors actually disagree about; otherwise a VAD1 port that accidentally
# implemented VAD2 would pass.
#
# VAD2 is not otherwise supported and is not a fallback: it rewires the
# open-loop pitch stage (`cod_amr.c`'s `#ifdef VAD2` branches change what
# `Lag_max` accumulates and replace the tone detector with `LTP_flag_update`).
# It is built here only as a discriminator.
# shellcheck disable=SC2046
cc -O1 -w -DMMS_IO -DVAD2 -I"$NB_WORK/c-code" -o "$NB_WORK/amrnb_enc_vad2" \
   $(ls "$NB_WORK/c-code"/*.c | grep -v '/decoder\.c$') -lm
"$NB_WORK/amrnb_enc_vad2" -dtx MR74 "$TESTDATA/amrnb_dtx_input.pcm" \
    "$NB_WORK/vad2_mode4.amr" >/dev/null 2>&1
if cmp -s "$NB_WORK/vad2_mode4.amr" "$TESTDATA/amrnb_dtx_mode4.amr"; then
  echo "    VAD1 and VAD2 agree on this signal; the fixture cannot qualify a VAD port" >&2
  exit 1
fi
python3 - "$TESTDATA/amrnb_dtx_mode4.amr" "$NB_WORK/vad2_mode4.amr" <<'VADEOF'
import sys
NB = [12, 13, 15, 17, 19, 20, 26, 31, 5] + [0] * 7
def kinds(path):
    data = open(path, "rb").read()
    pos, out = 6, []
    while pos < len(data):
        ft = (data[pos] >> 3) & 0x0F
        out.append(ft)
        pos += 1 + NB[ft]
    return out
one, two = kinds(sys.argv[1]), kinds(sys.argv[2])
differ = sum(1 for a, b in zip(one, two) if a != b)
print(f"    the two detectors choose different frame types on {differ} of {len(one)} frames")
if differ < 10:
    sys.exit(f"    only {differ} frames differ; too weak to qualify a VAD port")
VADEOF

echo "==> the DTX_MUTE fixtures: a quiet stream whose SID updates never arrive"
# `DTX_MAX_EMPTY_THRESH` == 50, and a clean encoder sends a SID every 8 frames,
# so the mute fade is unreachable on any stream this or any other encoder
# produces. It needs damage: rewrite every SID in the long quiet run to NO_DATA
# and the decoder counts past the threshold. Without this, the DTX_MUTE branch
# is code no test can reach -- which is how it would come to be wrong.
mute() {
  python3 - "$1" "$2" "$3" <<'MUTEEOF'
import sys
src, dst, variant = sys.argv[1], sys.argv[2], sys.argv[3]
NB = [12, 13, 15, 17, 19, 20, 26, 31, 5] + [0] * 7
WB = [17, 23, 32, 36, 40, 46, 50, 58, 60, 5] + [0] * 6
sizes, magic = (NB, 6) if variant == "nb" else (WB, 9)
sid_ft = 8 if variant == "nb" else 9

data = open(src, "rb").read()
out = bytearray(data[:magic])
pos, frame, dropped, run, longest = magic, 0, 0, 0, 0
kept_opening_sid = False
while pos < len(data):
    toc = data[pos]
    ft = (toc >> 3) & 0x0F
    body = sizes[ft]
    # Only inside the long quiet run: a SID dropped during the short opening
    # silence would not accumulate enough empty frames to matter, and dropping
    # them everywhere would make the fixture about something else.
    #
    # And the run's *first* SID is kept. Dropping it too was the original
    # mistake here: without it the decoder never learns that DTX has begun, so
    # every following empty frame reads as a lost speech frame rather than as
    # silence, the state machine stays in SPEECH, and DTX_MUTE -- the whole
    # point of this fixture -- is never reached. The stream still decoded
    # differently from the intact one, so the guard at the bottom of this
    # script passed while the fixture tested nothing it was built for.
    if ft == sid_ft and frame >= 50 and kept_opening_sid:
        out.append((15 << 3) | 0x04)
        dropped += 1
        run += 1
    else:
        if ft == sid_ft and frame >= 50:
            kept_opening_sid = True
        out.append(toc)
        out += data[pos + 1 : pos + 1 + body]
        run = run + 1 if ft == 15 else 0
    longest = max(longest, run)
    pos += 1 + body
    frame += 1
open(dst, "wb").write(bytes(out))
print(f"    {dropped} SID updates dropped, longest empty run {longest} frames")
if longest <= 50:
    sys.exit(f"    longest empty run is {longest}; DTX_MUTE needs more than 50")
MUTEEOF
}
mute "$TESTDATA/amrnb_dtx_mode4.amr" "$TESTDATA/amrnb_dtx_mute.amr" nb
"$NB_WORK/amrnb_dec" "$TESTDATA/amrnb_dtx_mute.amr" "$TESTDATA/amrnb_dtx_mute.pcm" >/dev/null 2>&1
mute "$TESTDATA/amrwb_dtx_mode2.amr" "$TESTDATA/amrwb_dtx_mute.amr" wb
"$WB_WORK/amrwb_dec" -mime "$TESTDATA/amrwb_dtx_mute.amr" "$TESTDATA/amrwb_dtx_mute.pcm" >/dev/null 2>&1

# And the mute stream must not decode to the same audio as the intact one --
# otherwise the missing SIDs had no effect and the fixture proves nothing.
for pair in "amrnb_dtx_mute.pcm amrnb_dtx_mode4.pcm" "amrwb_dtx_mute.pcm amrwb_dtx_mode2.pcm"; do
  set -- $pair
  if cmp -s "$TESTDATA/$1" "$TESTDATA/$2"; then
    echo "    $1 is identical to the intact stream; the dropped SIDs did nothing" >&2
    exit 1
  fi
done
echo "    both mute streams differ from their intact originals"

echo "==> sanity check: the decoders produce a full frame for every input frame"
for f in "$TESTDATA"/amrnb_dtx_mode*.pcm; do
  got=$(wc -c < "$f")
  test "$got" -eq $(( FRAMES * 320 )) || { echo "    $f is $got bytes" >&2; exit 1; }
done
for f in "$TESTDATA"/amrwb_dtx_mode*.pcm; do
  got=$(wc -c < "$f")
  test "$got" -eq $(( FRAMES * 640 )) || { echo "    $f is $got bytes" >&2; exit 1; }
done
echo "    every rate decodes back to $FRAMES frames"
