#!/usr/bin/env python3
"""Insert trace points into scratch copies of TS 26.073's dec_amr.c and sp_dec.c.

Emits every per-subframe intermediate to stderr under a name the Rust decoder
also uses, so the two can be diffed directly. Not part of the reference; only
ever applied to a scratch copy.

This is the narrowband twin of `instrument-amr-decoder.py`. Building it was the
first move rather than the last, because on the wideband side a full turn spent
reasoning about output PCM produced one speculative lead and no fixes, while
diffing traced intermediates found every remaining defect in a single pass.

Every insertion asserts its anchor. A silently missing trace point reads as
"this stage agrees" when it was never compared at all, which is worse than no
trace.
"""
import sys

dec_amr_path, sp_dec_path = sys.argv[1], sys.argv[2]

HELPER = '''
/* --- trace instrumentation, not part of TS 26.073 --- */
#include <stdio.h>
int rvoip_trace_frame = -1;
static void TRC(const char *name, const Word16 *v, int n) {
    int i;
    fprintf(stderr, "T %d %s", rvoip_trace_frame, name);
    for (i = 0; i < n; i++) fprintf(stderr, " %d", v[i]);
    fprintf(stderr, "\\n");
}
static void TRC1(const char *name, long v) {
    fprintf(stderr, "T %d %s %ld\\n", rvoip_trace_frame, name, v);
}
/* --- end trace instrumentation --- */
'''

SP_HELPER = '''
/* --- trace instrumentation, not part of TS 26.073 --- */
#include <stdio.h>
extern int rvoip_trace_frame;
static void SPTRC(const char *name, const Word16 *v, int n) {
    int i;
    fprintf(stderr, "T %d %s", rvoip_trace_frame, name);
    for (i = 0; i < n; i++) fprintf(stderr, " %d", v[i]);
    fprintf(stderr, "\\n");
}
/* --- end trace instrumentation --- */
'''

# (anchor, code, where) — "after" appends, "before" prepends.
POINTS = [
    # Spectral path. Two branches: every mode but 12.2 decodes one LSF set per
    # frame, 12.2 decodes two and interpolates differently.
    ("       Int_lpc_1to3(st->lsp_old, lsp_new, A_t);",
     '\n       TRC("lsp_new", lsp_new, M); TRC("A_t", A_t, AZ_SIZE);', "after"),
    ("       Int_lpc_1and3 (st->lsp_old, lsp_mid, lsp_new, A_t);",
     '\n       TRC("lsp_mid", lsp_mid, M); TRC("lsp_new", lsp_new, M);'
     ' TRC("A_t", A_t, AZ_SIZE);', "after"),

    # Adaptive codebook. `st->exc` is a moving pointer into old_exc, so the
    # trace deliberately reads through it rather than at a fixed offset.
    ("          Pred_lt_3or6 (st->exc, T0, T0_frac, L_SUBFR, 1);",
     '\n          TRC1("T0", T0); TRC1("T0_frac", T0_frac);'
     ' TRC("adapt", st->exc, L_SUBFR);', "after"),
    ("          Pred_lt_3or6 (st->exc, T0, T0_frac, L_SUBFR, 0);",
     '\n          TRC1("T0", T0); TRC1("T0_frac", T0_frac);'
     ' TRC("adapt", st->exc, L_SUBFR);', "after"),

    # The algebraic codevector before pitch sharpening is applied to it. Taken
    # before the loop, since sharpening rewrites code[] in place.
    ("        for (i = T0; i < L_SUBFR; i++)",
     '        TRC("code_raw", code, L_SUBFR); TRC1("pit_sharp_pre", pit_sharp);\n',
     "before"),

    # Gains and the assembled excitation, immediately before phase dispersion.
    ("        ph_disp_release(st->ph_disp_st);",
     '        TRC1("gain_pit", gain_pit); TRC1("gain_code", gain_code);\n'
     '        TRC1("gain_code_mix", gain_code_mix); TRC1("pit_sharp", pit_sharp);\n'
     '        TRC1("pitch_fac", pitch_fac); TRC1("tmp_shift", tmp_shift);\n'
     '        TRC("code", code, L_SUBFR); TRC("exc_total", st->exc, L_SUBFR);\n'
     '        TRC("ltp_unscaled", exc_enhanced, L_SUBFR);\n',
     "before"),

    # After phase dispersion: exc_enhanced is what synthesis consumes, and it
    # is deliberately not what feeds back into the adaptive codebook history.
    ("        L_temp = 0;                                   move32 ();",
     '        TRC("exc_enhanced", exc_enhanced, L_SUBFR);\n', "before"),

    # Synthesis output for this subframe, read after the overflow-retry path so
    # both branches are covered.
    ("        Copy (&st->old_exc[L_SUBFR], &st->old_exc[0], PIT_MAX + L_INTERPOL);",
     '        TRC("syn", &synth[i_subfr], L_SUBFR); TRC1("excEnergy", excEnergy);\n',
     "before"),
]

SP_POINTS = [
    ("  Post_Filter(st->post_state, mode, synth, Az_dec);",
     '\n  SPTRC("postfilter", synth, L_FRAME);', "after"),
    ("  Post_Process(st->postHP_state, synth, L_FRAME);",
     '\n  SPTRC("postproc", synth, L_FRAME);', "after"),
    # The frame counter has to advance before Decoder_amr runs, so every trace
    # line in the frame carries the same index.
    ("  /* Synthesis */",
     '  rvoip_trace_frame++;\n', "before"),
]


def instrument(path, helper, helper_anchor, points, label):
    s = open(path).read()
    assert helper_anchor in s, f"{label}: layout changed, anchor for the helper is gone"
    s = s.replace(helper_anchor, helper + "\n" + helper_anchor, 1)

    for anchor, code, where in points:
        assert s.count(anchor) == 1, (
            f"{label}: anchor is not unique ({s.count(anchor)} matches): {anchor[:60]}"
        )
        s = s.replace(anchor, (anchor + code) if where == "after" else (code + anchor), 1)

    open(path, "w").write(s)
    print(f"instrumented {label}: {len(points)} trace points")


instrument(dec_amr_path, HELPER, "int Decoder_amr (", POINTS, "dec_amr.c")
instrument(sp_dec_path, SP_HELPER, "int Speech_Decode_Frame (", SP_POINTS, "sp_dec.c")
