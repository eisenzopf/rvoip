#!/usr/bin/env python3
"""Emit the AMR-NB decoder tables as Rust from TS 26.073.

Two hazards this guards against explicitly.

**Symbol collision.** `mean_lsf` and `dico1_lsf`/`dico2_lsf`/`dico3_lsf` exist
in *both* `q_plsf_3.tab` and `q_plsf_5.tab` with different contents. The
reference gets away with it because only one is included per translation unit.
Emitting both into one Rust module does not, so they are suffixed `_3` and `_5`
— and the generator asserts the two `mean_lsf` tables actually differ, which is
what catches reading the same file twice.

**Inferred lengths.** Several arrays are declared `[]` in the reference. Their
lengths are reported rather than assumed, and any table whose expected count is
known is asserted.
"""
import re
import sys

src_dir, out_path = sys.argv[1], sys.argv[2]

# (file, C name, Rust name, expected count or None, per-line, doc)
TABLES = [
    ("q_plsf_3.tab", "past_rq_init", "PAST_RQ_INIT", 80, 10,
     "Reset state for the LSF predictor, eight sets of ten."),
    ("q_plsf_3.tab", "mean_lsf", "MEAN_LSF_3", 10, 10,
     "Long-term mean LSF for the 3-split quantiser.\\n"
     "///\\n"
     "/// Distinct from [`MEAN_LSF_5`] despite sharing a name in the reference,\\n"
     "/// where the two live in different translation units."),
    ("q_plsf_3.tab", "pred_fac", "PRED_FAC_3", 10, 10,
     "Per-coefficient MA prediction factors for the 3-split quantiser.\\n"
     "///\\n"
     "/// Note these are per-coefficient, where AMR-WB uses a single scalar for\\n"
     "/// all sixteen."),
    ("q_plsf_3.tab", "dico1_lsf", "DICO1_LSF_3", None, 9, "First split codebook, 3-split quantiser."),
    ("q_plsf_3.tab", "dico2_lsf", "DICO2_LSF_3", None, 9, "Second split codebook, 3-split quantiser."),
    ("q_plsf_3.tab", "dico3_lsf", "DICO3_LSF_3", None, 8, "Third split codebook, 3-split quantiser."),
    ("q_plsf_3.tab", "mr515_3_lsf", "MR515_3_LSF", None, 8,
     "Third split codebook used only at 5.15 kbit/s."),
    ("q_plsf_3.tab", "mr795_1_lsf", "MR795_1_LSF", None, 9,
     "First split codebook used only at 7.95 kbit/s."),
    ("q_plsf_5.tab", "mean_lsf", "MEAN_LSF_5", 10, 10,
     "Long-term mean LSF for the 5-split quantiser, used at 12.2 kbit/s."),
    ("q_plsf_5.tab", "dico1_lsf", "DICO1_LSF_5", None, 8, "First split codebook, 5-split quantiser."),
    ("q_plsf_5.tab", "dico2_lsf", "DICO2_LSF_5", None, 8, "Second split codebook, 5-split quantiser."),
    ("q_plsf_5.tab", "dico3_lsf", "DICO3_LSF_5", None, 8, "Third split codebook, 5-split quantiser."),
    ("q_plsf_5.tab", "dico4_lsf", "DICO4_LSF_5", None, 8, "Fourth split codebook, 5-split quantiser."),
    ("q_plsf_5.tab", "dico5_lsf", "DICO5_LSF_5", None, 8, "Fifth split codebook, 5-split quantiser."),
    ("lsp_lsf.tab", "table", "COS_TABLE", 65, 10,
     "`cos(x)` sampled at 65 points, Q15.\\n"
     "///\\n"
     "/// Half the resolution of the wideband table, because narrowband has ten\\n"
     "/// line frequencies rather than sixteen. Not interchangeable."),
    ("lsp_lsf.tab", "slope", "ACOS_SLOPE", 64, 10,
     "Slope of `acos` over each of the 64 intervals, Q12."),
    ("lsp.tab", "lsp_init_data", "LSP_INIT", 10, 10,
     "The decoder's reset-state LSPs — a flat spectrum."),
    ("inter_36.tab", "inter_6", "INTER_6", None, 6,
     "Fractional-delay interpolation filter for the adaptive codebook, Q15.\\n"
     "///\\n"
     "/// One sixth resolution, where the wideband filter is one quarter."),
    ("qua_gain.tab", "table_gain_highrates", "GAIN_HIGHRATES", None, 4,
     "Joint pitch/code gain codebook for the higher rates, four words per entry."),
    ("qua_gain.tab", "table_gain_lowrates", "GAIN_LOWRATES", None, 4,
     "Joint pitch/code gain codebook for the lower rates."),
    ("gray.tab", "gray", "GRAY", 8, 8, "Gray code used by the pulse-sign encoding."),
    ("gray.tab", "dgray", "DGRAY", 8, 8, "Inverse of [`GRAY`], used by the decoder."),
]


def values(path, name):
    raw = open(f"{src_dir}/{path}").read()
    # Anchor on the declaration, not a bare name: several of these appear in
    # comments and in neighbouring declarations.
    m = re.search(rf"\b{re.escape(name)}\s*\[[^\]]*\]\s*=\s*\{{", raw)
    assert m, f"{path}: declaration of {name} not found"
    body = raw[m.end():]
    depth = 1
    out = []
    for ch in body:
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                break
        out.append(ch)
    body = re.sub(r"/\*.*?\*/", "", "".join(out), flags=re.S)
    body = re.sub(r"//[^\n]*", "", body)
    # Strip casts before tokenising. `(Word16) 0x8000` would otherwise yield
    # three numbers -- 16, 0, 8000 -- in place of one, which shows up as a
    # count mismatch far from its cause.
    body = re.sub(r"\(\s*U?Word(?:16|32)\s*\)", " ", body)

    vals = []
    for tok in re.finditer(r"-?0[xX][0-9a-fA-F]+|-?\d+", body):
        text = tok.group(0)
        if "x" in text.lower():
            v = int(text, 16)
            # Hex literals in these tables are written as unsigned bit
            # patterns; 0x8000 means -32768, not 32768.
            if v >= 0x8000:
                v -= 0x10000
        else:
            v = int(text)
        vals.append(v)
    return vals


parts = []
extracted = {}

for path, c_name, rust_name, count, per_line, doc in TABLES:
    vals = values(path, c_name)
    if count is not None:
        assert len(vals) == count, f"{c_name}: got {len(vals)}, want {count}"
    extracted[rust_name] = vals
    lines = [f"/// {doc}", f"pub const {rust_name}: [i16; {len(vals)}] = ["]
    for i in range(0, len(vals), per_line):
        lines.append("    " + ", ".join(str(v) for v in vals[i : i + per_line]) + ",")
    lines.append("];")
    parts.append("\n".join(lines))

# The collision guard. If these come out equal, the same file was read twice.
assert extracted["MEAN_LSF_3"] != extracted["MEAN_LSF_5"], (
    "MEAN_LSF_3 and MEAN_LSF_5 are identical — the two q_plsf tables were not "
    "read separately"
)
assert extracted["DICO1_LSF_3"] != extracted["DICO1_LSF_5"], (
    "DICO1_LSF_3 and DICO1_LSF_5 are identical — same problem"
)

HEADER = '''//! Decoder tables for AMR-NB, from the TS 26.073 reference.
//!
//! These are normative constants: the codec cannot be conformant without
//! exactly these numbers.
//!
//! # Two names, two tables
//!
//! `mean_lsf` and `dico1..3_lsf` exist in *both* `q_plsf_3.tab` and
//! `q_plsf_5.tab` with different contents. The reference gets away with the
//! collision because only one is included per translation unit; a single Rust
//! module does not, so they carry `_3` and `_5` suffixes. The generator
//! asserts the two `mean_lsf` tables actually differ, which is what catches
//! reading one file twice.
//!
//! # Not interchangeable with the wideband tables
//!
//! The cosine table here has 65 entries against wideband's 129, because
//! narrowband has ten line frequencies rather than sixteen. The interpolation
//! filter is one-sixth resolution against wideband's one-quarter. Sharing
//! either would give a codec that sounds nearly right and fails conformance.
//!
//! Generated by `tools/gen_nb_decoder_tables.py`.

'''

with open(out_path, "w") as f:
    f.write(HEADER)
    f.write("\n\n".join(parts) + "\n")

print(f"wrote {out_path}: {len(TABLES)} tables")
for path, c_name, rust_name, count, _, _ in TABLES:
    if count is None:
        print(f"    {rust_name}: {len(extracted[rust_name])} (inferred)")
