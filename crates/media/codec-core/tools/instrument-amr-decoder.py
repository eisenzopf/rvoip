#!/usr/bin/env python3
"""Insert trace points into a copy of TS 26.173's dec_main.c.

Emits every per-subframe intermediate to stderr under a name the Rust decoder
also uses, so the two can be diffed directly. Not part of the reference; only
ever applied to a scratch copy.
"""
import sys

path = sys.argv[1]
s = open(path).read()

HELPER = '''
/* --- trace instrumentation, not part of TS 26.173 --- */
#include <stdio.h>
static void TRC(const char *name, const Word16 *v, int n) {
    int i;
    fprintf(stderr, "T %s", name);
    for (i = 0; i < n; i++) fprintf(stderr, " %d", v[i]);
    fprintf(stderr, "\\n");
}
static void TRC1(const char *name, long v) { fprintf(stderr, "T %s %ld\\n", name, v); }
/* --- end trace instrumentation --- */
'''

# Anchor on exact source lines. Every insertion asserts, because a silently
# missing trace point reads as "this stage agrees" when it was never compared.
POINTS = [
    ("    Isf_isp(isf, ispnew, M);",
     '\n    TRC("isf", isf, M); TRC("ispnew", ispnew, M);'),
    ("    Int_isp(st->ispold, ispnew, interpol_frac, Aq);",
     '\n    TRC("Aq", Aq, 4 * (M + 1)); TRC1("stab_fac", stab_fac);'),
    ("        Pred_lt4(&exc[i_subfr], T0, T0_frac, L_SUBFR + 1);",
     '\n        TRC1("T0", T0); TRC1("T0_frac", T0_frac);'
     ' TRC("pred", &exc[i_subfr], L_SUBFR + 1);'),
    ("            Copy(code, &exc[i_subfr], L_SUBFR);",
     '\n            TRC("adapt", &exc[i_subfr], L_SUBFR);'),
    ("        Pit_shrp(code, tmp, PIT_SHARP, L_SUBFR);",
     '\n        TRC1("tilt_code_used", st->tilt_code); TRC("code", code, L_SUBFR);'),
    ("        st->Q_old = Q_new;                 move16();",
     '\n        TRC1("Q_new", Q_new); TRC1("gain_code_scaled", gain_code);'),
    ("        voice_fac = voice_factor(exc2, -3, gain_pit, code, gain_code, L_SUBFR);",
     '\n        TRC1("voice_fac", voice_fac); TRC("exc2_adapt", exc2, L_SUBFR);'),
    ("        Phase_dispersion(gain_code, gain_pit, code, j, st->disp_mem);",
     '\n        TRC1("disp_gain_code", gain_code); TRC1("disp_mode", j);'
     ' TRC("code_disp", code, L_SUBFR);'),
    ("    Deemph_32(synth_hi + M, synth_lo + M, synth, PREEMPH_FAC, L_SUBFR, &(st->mem_deemph));",
     '\n    TRC("synhi", synth_hi + M, L_SUBFR); TRC("deemph", synth, L_SUBFR);'),
    ("    HP50_12k8(synth, L_SUBFR, st->mem_sig_out);",
     '\n    TRC("hp50", synth, L_SUBFR);'),
    ("    Oversamp_16k(synth, L_SUBFR, synth16k, st->mem_oversamp);",
     '\n    TRC("upsampled", synth16k, L_SUBFR16k);'),
    ("    Filt_6k_7k(HF, L_SUBFR16k, st->mem_hf);", '\n    TRC("hfband", HF, L_SUBFR16k);'),
]

for gain_bits in (6, 7):
    POINTS.append((
        "            D_gain2(index, %d, code, L_SUBFR, &gain_pit, &L_gain_code, bfi, "
        "st->prev_bfi, st->state, unusable_frame, st->vad_hist, st->dec_gain);" % gain_bits,
        '\n            TRC1("gain_pit", gain_pit); TRC1("L_gain_code", L_gain_code);'))

assert "static Word16 isf_init[M] =" in s, "dec_main.c layout changed"
s = s.replace("static Word16 isf_init[M] =", HELPER + "\nstatic Word16 isf_init[M] =", 1)

for anchor, code in POINTS:
    assert anchor in s, f"anchor not found: {anchor[:70]}"
    s = s.replace(anchor, anchor + code, 1)

open(path, "w").write(s)
print(f"instrumented {len(POINTS)} trace points")
