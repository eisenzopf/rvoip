#!/usr/bin/env bash
# Build an instrumented TS 26.073 decoder that dumps its per-subframe
# intermediates, for diffing against the Rust decoder's own trace.
#
# The narrowband twin of trace-amr-reference.sh, and for the same reason:
# reasoning from output PCM found none of the wideband assembly bugs, while
# diffing intermediates found all of them in one pass. When the assembled
# decoder is wrong, start here.
#
# The Rust side emits the same names: run
#   cargo test -p rvoip-codec-core --all-features nb_where_does_it_diverge \
#       -- --ignored --nocapture
#
# Usage: trace-amrnb-reference.sh [mode-index] [workdir]
set -euo pipefail

MODE="${1:-4}"
WORK="${2:-${TMPDIR:-/tmp}/rvoip-amrnb-trace}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TESTDATA="$HERE/../src/codecs/amr/testdata"
REF="${TMPDIR:-/tmp}/rvoip-amrnb-reference/c-code"

if [ ! -d "$REF" ]; then
  echo "reference source missing; run build-amrnb-reference.sh first" >&2
  exit 1
fi

rm -rf "$WORK" && mkdir -p "$WORK"
cp "$REF"/*.c "$REF"/*.h "$REF"/*.tab "$WORK"/

python3 "$HERE/instrument-amrnb-decoder.py" "$WORK/dec_amr.c" "$WORK/sp_dec.c"

# -DMMS_IO selects the MIME/storage bitstream format the .amr fixtures use.
# shellcheck disable=SC2046
( cd "$WORK" && cc -O1 -w -DMMS_IO -o "$WORK/amrnb_trace" \
    $(ls ./*.c | grep -v '/coder\.c$') -lm )

"$WORK/amrnb_trace" "$TESTDATA/amrnb_mode$MODE.amr" "$WORK/out.pcm" \
    2>&1 >/dev/null | grep '^T ' > "$WORK/trace.txt"

echo "wrote $WORK/trace.txt ($(wc -l < "$WORK/trace.txt") lines)"

# The reference PCM this run produces must match the committed fixture, or the
# instrumentation changed behaviour rather than only observing it.
if cmp -s "$WORK/out.pcm" "$TESTDATA/amrnb_mode$MODE.pcm"; then
  echo "instrumented decoder reproduces the committed PCM exactly"
else
  echo "WARNING: instrumented output differs from the committed PCM" >&2
  exit 1
fi

echo
echo "Frame 0, subframe 0 scalars:"
for k in T0 T0_frac pit_sharp_pre gain_pit gain_code gain_code_mix pit_sharp \
         pitch_fac tmp_shift excEnergy; do
  awk -v k="$k" '$3==k{print "  "k" = "$4; exit}' "$WORK/trace.txt"
done
