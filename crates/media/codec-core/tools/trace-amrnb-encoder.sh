#!/usr/bin/env bash
# Build an instrumented TS 26.073 encoder that dumps its per-subframe
# intermediates, for diffing against the Rust encoder's own trace.
#
# Narrowband twin of trace-amrwb-encoder.sh. Note 4.75 kbit/s codes two
# subframes jointly and re-runs the first subframe's post-processing with the
# jointly chosen gains, so some subframes appear twice in the trace and the
# second occurrence is the one that counts.
#
# Usage: trace-amrnb-encoder.sh [mode-index] [workdir]
set -euo pipefail

MODE="${1:-4}"
WORK="${2:-${TMPDIR:-/tmp}/rvoip-amrnb-enc-trace}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TESTDATA="$HERE/../src/codecs/amr/testdata"
REF="${TMPDIR:-/tmp}/rvoip-amrnb-reference/c-code"
MODES=(MR475 MR515 MR59 MR67 MR74 MR795 MR102 MR122)

if [ ! -d "$REF" ]; then
  echo "reference source missing; run build-amrnb-reference.sh first" >&2
  exit 1
fi
if [ ! -f "$TESTDATA/amrnb_enc_input.pcm" ]; then
  echo "encoder input missing; run build-amrnb-encoder-reference.sh first" >&2
  exit 1
fi

rm -rf "$WORK" && mkdir -p "$WORK"
cp "$REF"/*.c "$REF"/*.h "$REF"/*.tab "$WORK"/

python3 "$HERE/instrument-amrnb-encoder.py" "$WORK/cod_amr.c" "$WORK/sp_enc.c"

# shellcheck disable=SC2046
( cd "$WORK" && cc -O1 -w -DMMS_IO -o "$WORK/amrnb_enc_trace" \
    $(ls ./*.c | grep -v '/decoder\.c$') -lm )

"$WORK/amrnb_enc_trace" "${MODES[$MODE]}" "$TESTDATA/amrnb_enc_input.pcm" \
    "$WORK/out.amr" 2>"$WORK/raw.txt" >/dev/null
grep '^T ' "$WORK/raw.txt" > "$WORK/trace.txt"

echo "wrote $WORK/trace.txt ($(wc -l < "$WORK/trace.txt") lines)"

# The instrumented build must still produce the committed bitstream, or the
# trace points changed behaviour rather than only observing it.
if cmp -s "$WORK/out.amr" "$TESTDATA/amrnb_enc_mode$MODE.amr"; then
  echo "instrumented encoder reproduces the committed bitstream exactly"
else
  echo "WARNING: instrumented output differs from the committed bitstream" >&2
  exit 1
fi

echo
echo "Frame 0, subframe 0 scalars:"
for k in T_op0 T0 T0_frac gain_pit_ol gain_pit gain_code; do
  awk -v k="$k" '$4==k{print "  "k" = "$5; exit}' "$WORK/trace.txt"
done
