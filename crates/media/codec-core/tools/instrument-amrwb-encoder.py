#!/usr/bin/env python3
"""Insert trace points into a scratch copy of TS 26.173's cod_main.c.

The wideband decoder reached bit-exactness by diffing traced intermediates
against the instrumented reference; reasoning from output found none of its six
defects. The encoder is a harder problem — its codebook search has to reproduce
not just an arithmetic result but a *decision* — so the same instrument is built
before the first stage rather than after the first failure.

Not part of the reference; only ever applied to a scratch copy. Every insertion
asserts its anchor, because a silently missing trace point reads as "this stage
agrees" when it was never compared at all.
"""
import sys

path = sys.argv[1]
s = open(path).read()

HELPER = '''
/* --- trace instrumentation, not part of the reference --- */
#include <stdio.h>
#include <stdlib.h>
static long rvoip_frame = -1;
static long rvoip_subfr = -1;
/* The trace gets its own file rather than stderr. The reference writes its
 * own progress to stderr *without* newlines, so it merges into whatever
 * trace line follows and silently removes it from any line-prefix filter.
 * Fifty rows vanished that way before it was noticed, and a missing row
 * reads as "this stage agrees" when it was never compared at all. */
static FILE *rvoip_trace_file(void) {
    static FILE *f = NULL;
    if (!f) {
        const char *p = getenv("RVOIP_TRACE");
        f = fopen(p ? p : "rvoip_trace.txt", "w");
        if (!f) { perror("rvoip trace"); exit(1); }
    }
    return f;
}
static void TRC(const char *name, const Word16 *v, int n) {
    FILE *f = rvoip_trace_file();
    int i;
    fprintf(f, "T %ld %ld %s", rvoip_frame, rvoip_subfr, name);
    for (i = 0; i < n; i++) fprintf(f, " %d", v[i]);
    fprintf(f, "\\n");
}
static void TRC1(const char *name, long v) {
    fprintf(rvoip_trace_file(), "T %ld %ld %s %ld\\n", rvoip_frame, rvoip_subfr, name, v);
}
/* --- end trace instrumentation --- */
'''

# (anchor, code, where). "after" appends, "before" prepends.
POINTS = [
    # Frame counter, and the preprocessing chain: 16 kHz in, 12.8 kHz
    # pre-emphasised and scaled out. Every later stage inherits its scaling
    # from here, so a wrong `Q_new` mis-scales the whole frame while leaving it
    # perfectly speech-shaped.
    ("    st = (Coder_State *) spe_state;",
     '\n    rvoip_frame++; rvoip_subfr = -1;', "after"),
    ("    Decim_12k8(speech16k, L_FRAME16k, new_speech, st->mem_decim);",
     '\n    TRC("speech16k", speech16k, L_FRAME16k);', "after"),
    ("    HP50_12k8(new_speech, L_FRAME, st->mem_sig_in);",
     '\n    TRC("decimated", new_speech, L_FRAME);', "after"),

    # LP analysis. The autocorrelation is double precision, so both halves are
    # dumped: comparing only the high half hides a whole class of error.
    ("    Autocorr(p_window, M, r_h, r_l);       /* Autocorrelations */",
     '\n    TRC1("Q_new", Q_new); TRC("window", p_window, L_WINDOW);', "after"),
    ("    Lag_window(r_h, r_l);                  /* Lag windowing    */",
     '\n    TRC("r_h_pre", r_h, M + 1); TRC("r_l_pre", r_l, M + 1);', "after"),
    ("    Levinson(r_h, r_l, A, rc, st->mem_levinson);        /* Levinson Durbin  */",
     '\n    TRC("r_h", r_h, M + 1); TRC("r_l", r_l, M + 1);', "after"),
    ("    Az_isp(A, ispnew, st->ispold);         /* From A(z) to ISP */",
     '\n    TRC("A", A, M + 1); TRC("rc", rc, M);', "after"),
    ("    Int_isp(st->ispold, ispnew, interpol_frac, A);",
     '\n    TRC("ispnew", ispnew, M);', "after"),
    ("    Isp_isf(ispnew, isf, M);",
     '\n    TRC("A_interp", A, NB_SUBFR * (M + 1));', "after"),

    # ISF quantisation, at both bit budgets. The 36-bit and 46-bit quantisers
    # are separate functions rather than one parameterised by budget.
    ("        Qpisf_2s_36b(isf, isf, st->past_isfq, indice, 4);",
     '\n        TRC("isf_unq36", isf, M);', "before"),
    ("        Qpisf_2s_46b(isf, isf, st->past_isfq, indice, 4);",
     '\n        TRC("isf_unq46", isf, M);', "before"),
    ("    /* Convert ISFs to the cosine domain */\n    Isf_isp(isf, ispnew_q, M);",
     '    TRC("isf_q", isf, M); TRC1("stab_fac", stab_fac);\n', "before"),
    ("    Int_isp(st->ispold_q, ispnew_q, interpol_frac, Aq);",
     '\n    TRC("ispnew_q", ispnew_q, M); TRC("Aq", Aq, NB_SUBFR * (M + 1));', "after"),

    # Open-loop pitch and the voice-activity decision that gates it.
    ("        st->old_T0_med = Med_olag(T_op, st->old_ol_lag);        move16();",
     '\n        TRC1("T_op_med", st->old_T0_med);', "after"),

    # The weighted speech the open-loop pitch search runs on, and the
    # decimated-by-two copy it actually scans. Without these the pitch search
    # has an input nothing can check it against.
    ("    Scale_sig(wsp, L_FRAME / OPL_DECIM, shift);",
     '\n    TRC("wsp", wsp, L_FRAME / OPL_DECIM); TRC1("wsp_shift", shift);', "after"),

    # The quantiser indices, at both budgets: an ISF vector that matches while
    # the indices differ is a real possibility, since two codebook entries can
    # dequantise to the same rounded vector.
    ("        Parm_serial(indice[4], 6, &prms);",
     '\n        TRC("isf_indice36", indice, 5);', "after"),
    ("        Parm_serial(indice[6], 5, &prms);",
     '\n        TRC("isf_indice46", indice, 7);', "after"),

    # Per-subframe. The subframe counter is set from the loop variable rather
    # than incremented, so a `continue` cannot desynchronise it.
    ("        Weight_a(p_A, Ap, GAMMA1, M);\n        Residu(Ap, M, error + M, xn, L_SUBFR);",
     '        rvoip_subfr = i_subfr / L_SUBFR;\n', "before"),
    ("        Scale_sig(h1, L_SUBFR, add(1, shift));  /* set h1[] in Q15 with scaling for convolution */",
     '\n        TRC("xn", xn, L_SUBFR); TRC("h1", h1, L_SUBFR);'
     ' TRC1("shift", shift);', "after"),
    ("        Pred_lt4(&exc[i_subfr], T0, T0_frac, L_SUBFR + 1);",
     '\n        TRC1("T0", T0); TRC1("T0_frac", T0_frac);'
     ' TRC1("T0_min", T0_min); TRC1("T0_max", T0_max);', "after"),
    ("        Updt_tar(xn, xn2, y2, gain2, L_SUBFR);",
     '\n        TRC1("gain1", gain1); TRC1("gain2", gain2);'
     ' TRC("adapt", &exc[i_subfr], L_SUBFR); TRC("y1", y1, L_SUBFR);', "after"),
    ("        cor_h_x(h2, xn2, dn);",
     '\n        TRC("xn2", xn2, L_SUBFR); TRC("cn", cn, L_SUBFR);'
     ' TRC("h2", h2, L_SUBFR); TRC("dn", dn, L_SUBFR);', "after"),
    # `select` -- the LTP low-pass decision -- is assigned *after* the adaptive
    # codebook is filtered, so tracing it alongside gain1/gain2 would read the
    # previous subframe's value. It is traced here instead, where it is set.
    ("        Pit_shrp(code, T0, PIT_SHARP, L_SUBFR);",
     '\n        TRC1("select", select);'
     ' TRC("code", code, L_SUBFR); TRC("y2", y2, L_SUBFR);', "after"),
    ("        Syn_filt(p_Aq, M, &exc[i_subfr], synth, L_SUBFR, st->mem_syn, 1);",
     '\n        TRC1("gain_pit", gain_pit); TRC1("gain_code", gain_code);'
     ' TRC1("L_gain_code", L_gain_code); TRC1("voice_fac", voice_fac);'
     ' TRC("exc_total", &exc[i_subfr], L_SUBFR); TRC("synth", synth, L_SUBFR);', "after"),
]

assert "void coder(" in s, "cod_main.c layout changed; the helper has nowhere to go"
s = s.replace("void coder(", HELPER + "\nvoid coder(", 1)

for anchor, code, where in POINTS:
    count = s.count(anchor)
    assert count == 1, f"anchor matched {count} times: {anchor[:70]!r}"
    s = s.replace(anchor, (anchor + code) if where == "after" else (code + anchor), 1)

open(path, "w").write(s)
print(f"instrumented cod_main.c: {len(POINTS)} trace points")
