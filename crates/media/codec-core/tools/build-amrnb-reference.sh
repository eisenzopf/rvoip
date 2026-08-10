#!/usr/bin/env bash
# Fetch the TS 26.073 fixed-point reference (AMR-NB) and generate ground truth.
#
# TS 26.073 is the ANSI-C source that *defines* AMR-NB, exactly as TS 26.173
# defines AMR-WB. The prose spec (TS 26.090) describes the algorithm; this code
# is the specification, and conformance means matching its integers.
#
# 3GPP permits in-house use for product design but not redistribution, so the
# source is fetched into a working directory and never committed. Only the
# generated vectors are checked in -- they are output, not source.
#
# ETSI serves 403 to curl's default user agent; a browser one is required.
# That is bot filtering, not an access restriction.
#
# Usage: build-amrnb-reference.sh [workdir]
set -euo pipefail

WORK="${1:-${TMPDIR:-/tmp}/rvoip-amrnb-reference}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TESTDATA="$HERE/../src/codecs/amr/testdata"
UA="Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36"
URL="https://www.etsi.org/deliver/etsi_ts/126000_126099/126073/19.00.00_60/ts_126073v190000p0.zip"

mkdir -p "$WORK"
cd "$WORK"

if [ ! -d c-code ]; then
  echo "==> fetching TS 26.073"
  curl -fsSL -A "$UA" --max-time 300 -o ts26073.zip "$URL"
  unzip -oq ts26073.zip
  unzip -oq ./*ANSI_C_source_code.zip
fi

SRC="$WORK/c-code"
test -d "$SRC" || { echo "reference source not found" >&2; exit 1; }

# The 2001-era typedefs.h predates arm64 and stops with
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

echo "==> building the reference decoder"
# Everything except coder.c, which carries the encoder's own main().
# shellcheck disable=SC2046
# -DMMS_IO selects the MIME/storage bitstream format (the "#!AMR" files we
# ship as fixtures) over the ETSI serial one the reference defaults to.
( cd "$SRC" && cc -O1 -w -DMMS_IO -o "$WORK/amrnb_dec" \
    $(ls ./*.c | grep -v '/coder\.c$') -lm )

echo "==> decoding the fixtures to reference PCM"
# End-to-end ground truth: 8 kHz mono little-endian, 160 samples per frame.
for m in 0 1 2 3 4 5 6 7; do
  "$WORK/amrnb_dec" "$TESTDATA/amrnb_mode$m.amr" \
      "$TESTDATA/amrnb_mode$m.pcm" >/dev/null 2>&1
done
ls -l "$TESTDATA"/amrnb_mode0.pcm | awk '{print "    " $5 " bytes per mode"}'
