#!/usr/bin/env bash
# Build the Apache-2.0 AMR oracle libraries and regenerate the reference vectors.
#
# The oracle is development-time tooling only. Nothing it produces is linked
# into the shipped crate: it emits data files, which are checked in, and the
# test suite reads only those. A normal `cargo test` needs neither these
# libraries nor a C toolchain.
#
#   opencore-amr  0.1.6  AMR-NB encode/decode, AMR-WB decode   Apache-2.0
#   vo-amrwbenc   0.1.3  AMR-WB encode                          Apache-2.0
#
# Sources come from the Debian archive rather than SourceForge or GitHub:
# SourceForge redirects to download mirrors and GitHub is unreachable from some
# environments, while deb.debian.org serves the pristine upstream tarballs
# directly.
#
# Usage:
#   crates/media/codec-core/tools/build-amr-oracle.sh [workdir]
#
# Regenerated vectors land in src/codecs/amr/testdata/. Commit them only if they
# change for a reason you understand — they are the reference the Rust
# implementation is checked against.
set -euo pipefail

WORK="${1:-${TMPDIR:-/tmp}/rvoip-amr-oracle}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TESTDATA="$HERE/../src/codecs/amr/testdata"

OPENCORE_VER=0.1.6
VOAMRWB_VER=0.1.3
DEBIAN_POOL=http://deb.debian.org/debian/pool/main

mkdir -p "$WORK"
cd "$WORK"

fetch() { # url, output
  [ -f "$2" ] || curl -fsSL --max-time 300 -o "$2" "$1"
}

echo "==> fetching sources into $WORK"
fetch "$DEBIAN_POOL/o/opencore-amr/opencore-amr_${OPENCORE_VER}.orig.tar.gz" opencore-amr.tar.gz
fetch "$DEBIAN_POOL/v/vo-amrwbenc/vo-amrwbenc_${VOAMRWB_VER}.orig.tar.gz" vo-amrwbenc.tar.gz
[ -d "opencore-amr-$OPENCORE_VER" ] || tar xzf opencore-amr.tar.gz
[ -d "vo-amrwbenc-$VOAMRWB_VER" ] || tar xzf vo-amrwbenc.tar.gz

echo "==> building (static, into $WORK/install)"
for d in "opencore-amr-$OPENCORE_VER" "vo-amrwbenc-$VOAMRWB_VER"; do
  ( cd "$d"
    [ -f config.status ] || ./configure --prefix="$WORK/install" --disable-shared --enable-static >/dev/null
    make -j"$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)" >/dev/null
    make install >/dev/null )
done

for lib in libopencore-amrnb.a libopencore-amrwb.a libvo-amrwbenc.a; do
  test -f "$WORK/install/lib/$lib" || { echo "missing $lib" >&2; exit 1; }
done
echo "    all three libraries built"

echo "==> generating vectors"
cc -O2 -o "$WORK/gen_vectors" "$HERE/amr_gen_vectors.c" \
   -I"$WORK/install/include" -L"$WORK/install/lib" \
   -lvo-amrwbenc -lopencore-amrwb -lopencore-amrnb -lm
mkdir -p "$TESTDATA"
"$WORK/gen_vectors" "$TESTDATA"

echo
echo "==> done. Verify with:"
echo "    cargo test -p rvoip-codec-core --all-features storage::"
