#!/usr/bin/env python3
"""Insert trace points into scratch copies of TS 26.073's cod_amr.c and sp_enc.c.

The narrowband twin of `instrument-amrwb-encoder.py`, built for the same reason:
an encoder that makes the wrong *decision* still produces plausible speech at
the far end, so only a per-stage diff separates "different" from "wrong".

Narrowband adds a complication wideband does not have. At 4.75 kbit/s the
encoder codes two subframes jointly, which it implements by saving state,
running subframe 0, running subframe 1, and then *re-running* subframe 0's
post-processing with the jointly chosen gains. The trace therefore sees some
subframes twice, and the second pass is the one that counts. Anything comparing
against it has to expect that rather than assume one pass per subframe.

Not part of the reference; only ever applied to a scratch copy. Every insertion
asserts its anchor: a silently missing trace point reads as agreement.
"""
import sys

cod_path, sp_path = sys.argv[1], sys.argv[2]

HELPER = '''
/* --- trace instrumentation, not part of TS 26.073 --- */
#include <stdio.h>
long rvoip_enc_frame = -1;
static long rvoip_enc_subfr = -1;
static void TRC(const char *name, const Word16 *v, int n) {
    int i;
    fprintf(stderr, "T %ld %ld %s", rvoip_enc_frame, rvoip_enc_subfr, name);
    for (i = 0; i < n; i++) fprintf(stderr, " %d", v[i]);
    fprintf(stderr, "\\n");
}
static void TRC1(const char *name, long v) {
    fprintf(stderr, "T %ld %ld %s %ld\\n", rvoip_enc_frame, rvoip_enc_subfr, name, v);
}
/* --- end trace instrumentation --- */
'''

SP_HELPER = '''
/* --- trace instrumentation, not part of TS 26.073 --- */
#include <stdio.h>
extern long rvoip_enc_frame;
/* --- end trace instrumentation --- */
'''

POINTS = [
    # Frame-level: the LP analysis and the LSF quantiser, before the subframe
    # loop starts. `subfrNr` is still -1 here, which is how a reader tells
    # frame-level rows from subframe rows.
    ("   lpc(st->lpcSt, mode, st->p_window, st->p_window_12k2, A_t);",
     '\n   rvoip_enc_subfr = -1; TRC("speech", st->new_speech, L_FRAME);', "after"),
    ("   lsp(st->lspSt, mode, *usedMode, A_t, Aq_t, lsp_new, &ana);",
     '\n   TRC("A_t", A_t, AZ_SIZE);', "after"),

    # Open-loop pitch, computed over half-frames rather than subframes.
    ("      T_op[1] = T_op[0];                                     move16 ();",
     '\n      TRC1("T_op0", T_op[0]);', "after"),

    # Per-subframe. The counter is derived from the loop variable, so the MR475
    # re-run below cannot desynchronise it.
    ("      /* copy the LP residual (res2 is modified in the CL LTP search)    */\n      Copy (res, res2, L_SUBFR);",
     '      rvoip_enc_subfr = i_subfr / L_SUBFR;\n'
     '      TRC("lsp_new", lsp_new, M); TRC("xn", xn, L_SUBFR);\n'
     '      TRC("h1", st->h1, L_SUBFR); TRC("res", res, L_SUBFR);\n', "before"),
    ("      cbsearch(xn2, st->h1, T0, st->sharp, gain_pit, res2,",
     '      TRC1("T0", T0); TRC1("T0_frac", T0_frac);\n'
     '      TRC1("gain_pit_ol", gain_pit); TRC("xn2", xn2, L_SUBFR);\n'
     '      TRC("y1", y1, L_SUBFR); TRC("adapt", &st->exc[i_subfr], L_SUBFR);\n',
     "before"),
    ("      gainQuant(st->gainQuantSt, *usedMode, res, &st->exc[i_subfr], code,",
     '      TRC("code", code, L_SUBFR); TRC("y2", y2, L_SUBFR);\n', "before"),
    ("      /* update gain history */\n      update_gp_clipping(st->tonStabSt, gain_pit);",
     '      TRC1("gain_pit", gain_pit); TRC1("gain_code", gain_code);\n', "before"),
]

SP_POINTS = [
    # The frame counter advances once per encoded frame, before any of the
    # per-frame trace points fire.
    ("  /* Call the speech encoder */\n  cod_amr(st->cod_amr_state, mode, new_speech, prm, usedMode, syn);",
     '  rvoip_enc_frame++;\n', "before"),
]


def instrument(path, helper, helper_anchor, points, label):
    s = open(path).read()
    assert helper_anchor in s, f"{label}: layout changed, the helper has nowhere to go"
    s = s.replace(helper_anchor, helper + "\n" + helper_anchor, 1)

    for anchor, code, where in points:
        count = s.count(anchor)
        assert count == 1, f"{label}: anchor matched {count} times: {anchor[:70]!r}"
        s = s.replace(anchor, (anchor + code) if where == "after" else (code + anchor), 1)

    open(path, "w").write(s)
    print(f"instrumented {label}: {len(points)} trace points")


instrument(cod_path, HELPER, "int cod_amr(", POINTS, "cod_amr.c")
instrument(sp_path, SP_HELPER, "int Speech_Encode_Frame_First (", SP_POINTS, "sp_enc.c")
