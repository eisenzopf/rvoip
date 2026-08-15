#!/usr/bin/env bash
# Build an instrumented TS 26.173 decoder that dumps its per-subframe
# intermediates, for diffing against the Rust decoder's own trace.
#
# Reasoning from output PCM alone found none of the assembly bugs in this
# project; diffing intermediates found all of them in one pass. When the
# assembled decoder is wrong, start here.
#
# The Rust side emits the same names: run
#   cargo test -p rvoip-codec-core --all-features where_does_it_diverge \
#       -- --ignored --nocapture
#
# Usage: trace-amr-reference.sh [mode-index] [workdir]
set -euo pipefail

MODE="${1:-2}"
WORK="${2:-${TMPDIR:-/tmp}/rvoip-amr-trace}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TESTDATA="$HERE/../src/codecs/amr/testdata"
REF="${TMPDIR:-/tmp}/rvoip-amr-reference/c-code"

if [ ! -d "$REF" ]; then
  echo "reference source missing; run build-amr-reference.sh first" >&2
  exit 1
fi

rm -rf "$WORK" && mkdir -p "$WORK"
cp "$REF"/*.c "$REF"/*.h "$REF"/*.tab "$WORK"/

python3 "$HERE/instrument-amr-decoder.py" "$WORK/dec_main.c"

( cd "$WORK" && cc -O1 -w -o "$WORK/amrwb_trace" \
    $(ls ./*.c | grep -v '/coder\.c$') -lm )

"$WORK/amrwb_trace" -mime "$TESTDATA/amrwb_mode$MODE.amr" "$WORK/out.pcm" \
    2>&1 >/dev/null | grep '^T ' > "$WORK/trace.txt"

echo "wrote $WORK/trace.txt ($(wc -l < "$WORK/trace.txt") lines)"
echo
echo "Frame 0, subframe 0 scalars:"
for k in stab_fac T0 T0_frac tilt_code_used gain_pit L_gain_code Q_new \
         gain_code_scaled voice_fac disp_gain_code disp_mode; do
  awk -v k="$k" '$2==k{print "  "k" = "$3; exit}' "$WORK/trace.txt"
done
