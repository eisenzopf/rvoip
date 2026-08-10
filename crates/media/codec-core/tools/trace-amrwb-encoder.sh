#!/usr/bin/env bash
# Build an instrumented TS 26.173 encoder that dumps its per-subframe
# intermediates, for diffing against the Rust encoder's own trace.
#
# The decoder's six defects were all found this way and none of them by
# reasoning about output. An encoder is worse: it has to reproduce a *decision*
# as well as an arithmetic result, so a wrong codebook search still produces
# perfectly plausible speech at the far end. Diffing intermediates is the only
# instrument that distinguishes "different" from "wrong".
#
# Usage: trace-amrwb-encoder.sh [mode-index] [workdir]
set -euo pipefail

MODE="${1:-2}"
WORK="${2:-${TMPDIR:-/tmp}/rvoip-amrwb-enc-trace}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TESTDATA="$HERE/../src/codecs/amr/testdata"
REF="$(dirname "$(find "${TMPDIR:-/tmp}/rvoip-amr-reference" -name isp_az.c 2>/dev/null | head -1)")"

if [ ! -d "$REF" ]; then
  echo "reference source missing; run build-amr-reference.sh first" >&2
  exit 1
fi
if [ ! -f "$TESTDATA/amrwb_enc_input.pcm" ]; then
  echo "encoder input missing; run build-amrwb-encoder-reference.sh first" >&2
  exit 1
fi

rm -rf "$WORK" && mkdir -p "$WORK"
cp "$REF"/*.c "$REF"/*.h "$REF"/*.tab "$WORK"/ 2>/dev/null || cp "$REF"/*.c "$REF"/*.h "$WORK"/

python3 "$HERE/instrument-amrwb-encoder.py" "$WORK/cod_main.c"

# shellcheck disable=SC2046
( cd "$WORK" && cc -O1 -w -o "$WORK/amrwb_enc_trace" \
    $(ls ./*.c | grep -v '/decoder\.c$') -lm )

"$WORK/amrwb_enc_trace" -mime "$MODE" "$TESTDATA/amrwb_enc_input.pcm" \
    "$WORK/out.amr" 2>"$WORK/raw.txt" >/dev/null
grep '^T ' "$WORK/raw.txt" > "$WORK/trace.txt"

echo "wrote $WORK/trace.txt ($(wc -l < "$WORK/trace.txt") lines)"

# The instrumented build must still produce the committed bitstream, or the
# trace points changed behaviour rather than only observing it.
if cmp -s "$WORK/out.amr" "$TESTDATA/amrwb_enc_mode$MODE.amr"; then
  echo "instrumented encoder reproduces the committed bitstream exactly"
else
  echo "WARNING: instrumented output differs from the committed bitstream" >&2
  exit 1
fi

echo
echo "Frame 0, subframe 0 scalars:"
for k in Q_new stab_fac T_op_med shift T0 T0_frac T0_min T0_max gain1 gain2 \
         select gain_pit gain_code voice_fac; do
  awk -v k="$k" '$4==k{print "  "k" = "$5; exit}' "$WORK/trace.txt"
done
