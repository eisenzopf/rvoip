#!/usr/bin/env bash
# Fetch the TS 26.173 fixed-point reference and regenerate the per-stage vectors.
#
# TS 26.173 is the ANSI-C source that *defines* AMR-WB. TS 26.190 §8.1: the
# codec is "described in a bit-exact arithmetic". The prose spec describes the
# algorithm; this code is the specification, and conformance means matching its
# integers exactly.
#
# 3GPP permits in-house use for product design but not redistribution, so the
# source is fetched into a working directory and never committed. Only the
# generated vectors are checked in — they are output, not source.
#
# Note ETSI serves 403 to curl's default user agent; a browser one is required.
# That is bot filtering, not an access restriction.
#
# Usage:
#   crates/media/codec-core/tools/build-amr-reference.sh [workdir]
set -euo pipefail

WORK="${1:-${TMPDIR:-/tmp}/rvoip-amr-reference}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TESTDATA="$HERE/../src/codecs/amr/testdata"
UA="Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36"
URL="https://www.etsi.org/deliver/etsi_ts/126100_126199/126173/19.00.00_60/ts_126173v190000p0.zip"

mkdir -p "$WORK"
cd "$WORK"

if [ ! -d c-code ]; then
  echo "==> fetching TS 26.173"
  curl -fsSL -A "$UA" --max-time 300 -o ts26173.zip "$URL"
  unzip -oq ts26173.zip
  unzip -oq ./*ANSI-C_source_code.zip
fi

SRC="$(dirname "$(find "$WORK" -name isp_az.c | head -1)")"
test -n "$SRC" || { echo "reference source not found" >&2; exit 1; }

# The 2002-era typedefs.h predates arm64 and stops with
# #error "can't determine architecture; adapt typedefs.h to your platform".
# Its integer widths come from limits.h and are already correct; only the
# platform/endianness block needs extending, which is what the error asks for.
if ! grep -q "__aarch64__" "$SRC/typedefs.h"; then
  echo "==> adding a little-endian platform branch to typedefs.h"
  python3 - "$SRC/typedefs.h" <<'PYEOF'
import sys
p = sys.argv[1]
s = open(p).read()
old = """#elif defined(linux) && defined(i386)
#define PC
#define PLATFORM "PC"
#define LSBFIRST
#else"""
new = """#elif defined(linux) && defined(i386)
#define PC
#define PLATFORM "PC"
#define LSBFIRST
#elif defined(__APPLE__) || defined(__linux__) || defined(__aarch64__) || defined(__x86_64__)
#define PC
#define PLATFORM "PC"
#define LSBFIRST
#else"""
assert old in s, "typedefs.h layout changed"
open(p, "w").write(s.replace(old, new, 1))
PYEOF
fi

echo "==> building the stage oracle"
cc -O1 -w -I"$SRC" -o "$WORK/stage_oracle" "$HERE/amr_stage_oracle.c" \
   "$SRC/isp_az.c" "$SRC/az_isp.c" "$SRC/levinson.c" "$SRC/autocorr.c" \
   "$SRC/lag_wind.c" "$SRC/isp_isf.c" "$SRC/int_lpc.c" "$SRC/qpisf_2s.c" \
   "$SRC/weight_a.c" "$SRC/bits.c" "$SRC/d4t64fx.c" "$SRC/d2t64fx.c" \
   "$SRC/q_pulse.c" "$SRC/d_gain2.c" "$SRC/math_op.c" "$SRC/log2.c" \
   "$SRC/p_med_ol.c" "$SRC/hp_wsp.c" "$SRC/pred_lt4.c" "$SRC/preemph.c" \
   "$SRC/pit_shrp.c" "$SRC/syn_filt.c" "$SRC/deemph.c" "$SRC/hp50.c" \
   "$SRC/scale.c" "$SRC/decim54.c" "$SRC/hp400.c" "$SRC/hp6k.c" \
   "$SRC/hp7k.c" "$SRC/random.c" "$SRC/ph_disp.c" "$SRC/voicefac.c" \
   "$SRC/basicop2.c" \
   "$SRC/oper_32b.c" "$SRC/count.c" "$SRC/util.c" -lm

echo "==> generating vectors"
mkdir -p "$TESTDATA"
"$WORK/stage_oracle" "$TESTDATA" > "$TESTDATA/lp_stages_wb.txt"
wc -l < "$TESTDATA/lp_stages_wb.txt" | xargs echo "    lines:"

echo "==> building the full reference decoder"
# Everything except coder.c, which carries the encoder's own main().
# shellcheck disable=SC2046
cc -O1 -w -I"$SRC" -o "$WORK/amrwb_dec" \
   $(ls "$SRC"/*.c | grep -v '/coder\.c$') -lm

echo "==> decoding the fixtures to reference PCM"
# End-to-end ground truth: 16 kHz mono little-endian, 320 samples per frame.
# Per-stage vectors cannot reach the state coupling between stages; this can.
for m in 0 1 2 3 4 5 6 7 8; do
  "$WORK/amrwb_dec" -mime "$TESTDATA/amrwb_mode$m.amr" \
      "$TESTDATA/amrwb_mode$m.pcm" >/dev/null 2>&1
done
ls -l "$TESTDATA"/amrwb_mode0.pcm | awk '{print "    " $5 " bytes per mode"}'

echo "==> generating the ISP/ISF tables"
python3 "$HERE/gen_isf_tables.py" "$SRC/isp_isf.tab" \
    "$HERE/../src/codecs/amr/wb/lp/isf_tables.rs"
python3 "$HERE/gen_isf_codebooks.py" "$SRC/qpisf_2s.tab" \
    "$HERE/../src/codecs/amr/wb/lp/isf_codebooks.rs"
python3 "$HERE/gen_sort_tables.py" "$SRC/mime_io.tab" \
    "$HERE/../src/codecs/amr/wb/sort_tables.rs"
python3 "$HERE/gen_gain_tables.py" "$SRC" \
    "$HERE/../src/codecs/amr/wb/gain_tables.rs"

echo
echo "==> verify with:"
echo "    cargo test -p rvoip-codec-core --all-features wb::lp"
