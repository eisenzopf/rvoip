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
    ("pred_lt.c", "inter_6", "INTER_6_PRED", 61, 6,
     "Fractional-delay interpolation filter for the **adaptive codebook**,\\n"
     "/// one-sixth resolution, Q15. 61 taps.\\n"
     "///\\n"
     "/// The 1/3-resolution form every rate except 12.2 uses is this same\\n"
     "/// table subsampled by two, so there is one filter rather than two.\\n"
     "///\\n"
     "/// **Not [`INTER_6_SEARCH`].** The reference declares a table called\\n"
     "/// `inter_6` twice — this one file-local to `pred_lt.c`, the other in\\n"
     "/// `inter_36.tab` — with different lengths and different values. Using\\n"
     "/// the wrong one gives an adaptive codebook close enough to sound right\\n"
     "/// at every lag and conformant at none."),
    ("inter_36.tab", "inter_6", "INTER_6_SEARCH", 25, 6,
     "Fractional-delay interpolation filter for the **encoder's closed-loop\\n"
     "/// pitch search**, one-sixth resolution, Q15. 25 taps.\\n"
     "///\\n"
     "/// Shorter than [`INTER_6_PRED`] because the search only needs the\\n"
     "/// filter's central lobe to rank candidate lags, while the decoder needs\\n"
     "/// the whole response to reconstruct the excitation."),
    ("qua_gain.tab", "table_gain_highrates", "GAIN_HIGHRATES", None, 4,
     "Joint pitch/code gain codebook for the higher rates, four words per entry."),
    ("qua_gain.tab", "table_gain_lowrates", "GAIN_LOWRATES", None, 4,
     "Joint pitch/code gain codebook for the lower rates."),
    ("gray.tab", "gray", "GRAY", 8, 8, "Gray code used by the pulse-sign encoding."),
    ("gray.tab", "dgray", "DGRAY", 8, 8, "Inverse of [`GRAY`], used by the decoder."),
    ("qgain475.tab", "table_gain_MR475", "GAIN_MR475", 1024, 4,
     "Joint gain codebook for 4.75 kbit/s, four words per entry.\\n"
     "///\\n"
     "/// 4.75 is the one rate that quantises *two* subframes' gains with a\\n"
     "/// single index, so each entry carries two (pitch, code) pairs."),
    ("gains.tab", "qua_gain_pitch", "QUA_GAIN_PITCH", 16, 8,
     "Scalar pitch-gain codebook, Q14. Used at 7.95 and 12.2 kbit/s."),
    ("gains.tab", "qua_gain_code", "QUA_GAIN_CODE", 96, 3,
     "Scalar code-gain correction codebook, three words per entry.\\n"
     "///\\n"
     "/// The triple is `(gain factor, log2 integer, log2 fraction)`; the last\\n"
     "/// two feed the MA energy predictor's state update directly, which is\\n"
     "/// why they are tabulated rather than recomputed."),
    ("ph_disp.tab", "ph_imp_low_MR795", "PH_IMP_LOW_MR795", 40, 10,
     "Phase-dispersion impulse response, full dispersion, 7.95 kbit/s. Q15."),
    ("ph_disp.tab", "ph_imp_mid_MR795", "PH_IMP_MID_MR795", 40, 10,
     "Phase-dispersion impulse response, medium dispersion, 7.95 kbit/s. Q15."),
    ("ph_disp.tab", "ph_imp_low", "PH_IMP_LOW", 40, 10,
     "Phase-dispersion impulse response, full dispersion, 4.75–6.70 kbit/s. Q15."),
    ("ph_disp.tab", "ph_imp_mid", "PH_IMP_MID", 40, 10,
     "Phase-dispersion impulse response, medium dispersion, 4.75–6.70 kbit/s.\\n"
     "///\\n"
     "/// Identical in value to [`PH_IMP_MID_MR795`] in the reference, but kept\\n"
     "/// separate because nothing guarantees that; the generator asserts the\\n"
     "/// equality it observes rather than assuming it."),
    ("c2_9pf.tab", "startPos", "START_POS_2I40_9", 16, 4,
     "Per-subframe track start positions for the 9-bit two-pulse codebook.\\n"
     "///\\n"
     "/// Indexed by subframe and by the two pulses; 4.75 and 5.15 kbit/s vary\\n"
     "/// their tracks across the four subframes, which is why this decoder\\n"
     "/// alone takes a subframe number."),
    ("c2_11pf.tab", "startPos1", "START_POS1_2I40_11", 2, 2,
     "First-pulse track start positions for the 11-bit two-pulse codebook."),
    ("c2_11pf.tab", "startPos2", "START_POS2_2I40_11", 4, 4,
     "Second-pulse track start positions for the 11-bit two-pulse codebook."),
    ("log2.tab", "table", "LOG2_TABLE", 33, 9,
     "`log2` mantissa table, 33 points.\\n"
     "///\\n"
     "/// AMR-NB's own — deliberately not shared with G.729 or with AMR-WB,\\n"
     "/// each of which tabulates the same function over different points."),
    ("pow2.tab", "table", "POW2_TABLE", 33, 9,
     "`2^x` mantissa table, 33 points. AMR-NB's own; see [`LOG2_TABLE`]."),
    ("sqrt_l.tab", "table", "SQRT_L_TABLE", 49, 9,
     "`sqrt` mantissa table, 49 points, used by the excitation energy measure."),
    ("inv_sqrt.tab", "table", "INV_SQRT_TABLE", 49, 9,
     "`1/sqrt` mantissa table, 49 points, used by the gain predictor."),
    # The post-filter's bandwidth-expansion factors live in pstfilt.c rather
    # than in a .tab, because the reference keeps them file-local. They are
    # normative all the same.
    ("pstfilt.c", "gamma3_MR122", "GAMMA3_MR122", 10, 10,
     "Numerator bandwidth-expansion factors for the post-filter at 12.2 and\\n"
     "/// 10.2 kbit/s, Q15."),
    ("pstfilt.c", "gamma3", "GAMMA3", 10, 10,
     "Numerator bandwidth-expansion factors for the post-filter at every\\n"
     "/// other rate, Q15."),
    ("pstfilt.c", "gamma4_MR122", "GAMMA4_MR122", 10, 10,
     "Denominator bandwidth-expansion factors at 12.2 and 10.2 kbit/s, Q15."),
    ("pstfilt.c", "gamma4", "GAMMA4", 10, 10,
     "Denominator bandwidth-expansion factors at every other rate, Q15.\\n"
     "///\\n"
     "/// Numerically identical to [`GAMMA3_MR122`] — both are `0.7^n` — while\\n"
     "/// [`GAMMA4_MR122`] is `0.75^n`. The generator asserts that coincidence\\n"
     "/// rather than relying on it, so a revision where they diverge fails\\n"
     "/// here instead of quietly detuning one rate's post-filter."),
]

# Decoder homing frames: one parameter vector per mode, all in one file, so
# they are emitted as a group rather than listed individually above.
DHF = [
    ("MR475", "DHF_MR475"),
    ("MR515", "DHF_MR515"),
    ("MR59", "DHF_MR59"),
    ("MR67", "DHF_MR67"),
    ("MR74", "DHF_MR74"),
    ("MR795", "DHF_MR795"),
    ("MR102", "DHF_MR102"),
    ("MR122", "DHF_MR122"),
]
for c_suffix, rust_name in DHF:
    TABLES.append((
        "d_homing.tab", f"dhf_{c_suffix}", rust_name, None, 8,
        f"Decoder homing frame parameters for {c_suffix}, TS 26.101.\\n"
        "///\\n"
        "/// Two consecutive homing frames must drive every bit-exactly defined\\n"
        "/// function into its home state, so these double as a conformance\\n"
        "/// checkpoint that needs no test vectors."))


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
    # The doc strings above carry `\n` as two characters so the table list
    # stays readable; turn them into real line breaks here, or rustdoc
    # renders a paragraph with backslashes in it.
    lines = [f"/// {doc}".replace("\\n", "\n"),
             f"pub const {rust_name}: [i16; {len(vals)}] = ["]
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

# The opposite check, for a pair that *is* equal in this revision of the
# reference. Recording it as an assertion rather than as a comment means a
# future reference where they diverge fails here instead of silently making one
# of the two dispersion levels wrong.
assert extracted["PH_IMP_MID"] == extracted["PH_IMP_MID_MR795"], (
    "the two medium-dispersion impulse responses have diverged; they are "
    "separate tables in the reference and must now be treated as such"
)
assert extracted["PH_IMP_LOW"] != extracted["PH_IMP_LOW_MR795"], (
    "the two full-dispersion impulse responses are identical — the same table "
    "was read twice"
)
assert extracted["GAMMA4"] == extracted["GAMMA3_MR122"], (
    "the post-filter's 0.7^n factors no longer agree between the two roles "
    "they play; treat them as genuinely separate tables"
)
assert extracted["GAMMA4"] != extracted["GAMMA4_MR122"], (
    "GAMMA4 and GAMMA4_MR122 are identical — the same declaration was read twice"
)
# The second name collision in this reference, and the more dangerous one: both
# are called `inter_6`, both are 1/6-resolution interpolation filters, and they
# differ in length and in every coefficient.
assert len(extracted["INTER_6_PRED"]) != len(extracted["INTER_6_SEARCH"]), (
    "the two inter_6 tables now have the same length; check which file each "
    "was read from before trusting either"
)
assert extracted["INTER_6_PRED"][0] == 29443 and extracted["INTER_6_SEARCH"][0] == 29519, (
    "the two inter_6 tables no longer start where they did; the generator may "
    "have read one file twice"
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
