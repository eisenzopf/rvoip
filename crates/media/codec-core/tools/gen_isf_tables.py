#!/usr/bin/env python3
"""Emit the AMR-WB ISP/ISF conversion tables as Rust from TS 26.173."""
import re
import sys

tab_path, out_path = sys.argv[1], sys.argv[2]


def values(raw, array):
    """Numbers inside one C array initialiser, ignoring comment banners."""
    body = raw.split(array, 1)[1].split("{", 1)[1].split("}", 1)[0]
    return [int(x) for x in re.findall(r"-?\d+", body)]


raw = open(tab_path).read()
cos = values(raw, "table[129]")
slope = values(raw, "slope[128]")
assert len(cos) == 129, f"cos table has {len(cos)} entries"
assert len(slope) == 128, f"slope table has {len(slope)} entries"


def table(name, doc, ty, vals, per_line=8):
    lines = [doc, f"pub const {name}: [{ty}; {len(vals)}] = ["]
    for i in range(0, len(vals), per_line):
        lines.append("    " + ", ".join(str(v) for v in vals[i : i + per_line]) + ",")
    lines.append("];")
    return "\n".join(lines)


HEADER = '''//! Normative tables for the AMR-WB ISP/ISF conversions.
//!
//! From the TS 26.173 fixed-point reference (`isp_isf.tab`), which is what
//! defines the codec. Both conversions are table-and-interpolate rather than a
//! real trigonometric evaluation, so these numbers *are* the transform: a more
//! accurate cosine would give different bits and fail conformance.

'''

COS_DOC = '''/// `cos(x)` sampled at 129 points, Q15, from 0 to pi.
///
/// Indexed by the top bits of an ISF, with the low 7 bits interpolating between
/// neighbours -- which is why there are 129 entries and not 128: the last one
/// is the right-hand end of the final interval.'''

SLOPE_DOC = '''/// Slope of `acos` over each of the 128 table intervals, Q11.
///
/// The inverse direction cannot interpolate the cosine table directly, since
/// its spacing is uneven in `x`. Each entry is the local reciprocal derivative,
/// so the search finds the interval and this converts the residual. The values
/// blow up at both ends (-26214 against -326 in the middle) because `acos` has
/// infinite slope at +/-1.'''

with open(out_path, "w") as f:
    f.write(HEADER)
    f.write(table("COS_TABLE", COS_DOC, "i16", cos) + "\n\n")
    f.write(table("ACOS_SLOPE", SLOPE_DOC, "i16", slope) + "\n")

print(f"wrote {out_path}: cos={len(cos)} slope={len(slope)}")
