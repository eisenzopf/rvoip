#!/usr/bin/env bash
# Generate erased-frame fixtures and the reference PCM for them, both variants.
#
# Both decoders implement concealment and neither can be shown to run it: every
# committed fixture is a clean stream, so `FrameQuality::Good` is the only path
# a test has ever taken. Concealment is also where a decoder is least likely to
# be right by accident — it is pure state machine, it only runs when something
# has already gone wrong, and getting it wrong sounds like a bad network rather
# than like a bug.
#
# The storage format carries the frame quality in the ToC byte's bit 2 (RFC 4867
# §5.3, and `decoder.c`: `q = (toc >> 2) & 1`). Clearing it marks the frame
# SPEECH_BAD, which is exactly what an RTP receiver does when the payload's Q bit
# is zero — so the fixture is the real signal, not a simulation of one.
#
# The erasure pattern is deliberate rather than random:
#   frame 5        a single loss in a clean run
#   frames 10-12   a burst, so the erasure state machine climbs past one
#   frame 13       the first good frame after a burst, where both gain
#                  concealers limit against the last known-good value
#   frames 20,22   alternating, which keeps the machine oscillating instead of
#                  settling
#
# Usage: build-amr-erasure-fixtures.sh
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TESTDATA="$HERE/../src/codecs/amr/testdata"
NB_WORK="${TMPDIR:-/tmp}/rvoip-amrnb-reference"
WB_WORK="${TMPDIR:-/tmp}/rvoip-amr-reference"

for d in "$NB_WORK/c-code" "$WB_WORK"; do
  test -e "$d" || { echo "reference missing at $d; run the build scripts first" >&2; exit 1; }
done

ERASED="5 10 11 12 20 22"

erase() { # in, out, magic-length, frame-size-table-lookup handled in python
  python3 - "$1" "$2" "$3" "$ERASED" <<'PYEOF'
import sys
src, dst, magic_len, erased = sys.argv[1], sys.argv[2], int(sys.argv[3]), set(map(int, sys.argv[4].split()))

# Frame body sizes by frame type, excluding the ToC byte. Narrowband and
# wideband disagree on every entry, so the table is chosen by magic length.
NB = [12, 13, 15, 17, 19, 20, 26, 31, 5, 0, 0, 0, 0, 0, 0, 0]
WB = [17, 23, 32, 36, 40, 46, 50, 58, 60, 5, 0, 0, 0, 0, 0, 0]
sizes = NB if magic_len == 6 else WB

data = open(src, "rb").read()
out = bytearray(data[:magic_len])
pos, frame = magic_len, 0
while pos < len(data):
    toc = data[pos]
    ft = (toc >> 3) & 0x0F
    body = sizes[ft]
    # Bit 2 is the quality bit: clearing it says "these bits arrived damaged".
    out.append(toc & ~0x04 if frame in erased else toc)
    out += data[pos + 1 : pos + 1 + body]
    pos += 1 + body
    frame += 1
open(dst, "wb").write(bytes(out))
print(f"    {frame} frames, {len(erased)} erased")
PYEOF
}

echo "==> AMR-NB: erasing frames $ERASED from 7.40 kbit/s"
erase "$TESTDATA/amrnb_mode4.amr" "$TESTDATA/amrnb_erased.amr" 6
"$NB_WORK/amrnb_dec" "$TESTDATA/amrnb_erased.amr" "$TESTDATA/amrnb_erased.pcm" >/dev/null 2>&1
ls -l "$TESTDATA/amrnb_erased.pcm" | awk '{print "    reference PCM: " $5 " bytes"}'

echo "==> AMR-WB: erasing the same frames from 12.65 kbit/s"
erase "$TESTDATA/amrwb_mode2.amr" "$TESTDATA/amrwb_erased.amr" 9
"$WB_WORK/amrwb_dec" -mime "$TESTDATA/amrwb_erased.amr" "$TESTDATA/amrwb_erased.pcm" >/dev/null 2>&1
ls -l "$TESTDATA/amrwb_erased.pcm" | awk '{print "    reference PCM: " $5 " bytes"}'

echo "==> sanity check: the erased streams must NOT decode to the clean ones"
# Otherwise the fixture proves nothing — the erasures would have had no effect,
# which is what a mis-set quality bit looks like.
for pair in "amrnb_erased.pcm amrnb_mode4.pcm" "amrwb_erased.pcm amrwb_mode2.pcm"; do
  set -- $pair
  if cmp -s "$TESTDATA/$1" "$TESTDATA/$2"; then
    echo "    $1 is identical to the clean stream — the quality bit did not take" >&2
    exit 1
  fi
done
echo "    both differ from their clean streams, as they must"
