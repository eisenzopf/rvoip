/* Isolated per-stage vectors for the AMR-NB decoder, from TS 26.073.
 *
 * The companion `amrnb_oracle.c` replays the committed .amr fixtures through
 * the reference's bitstream and spectral path. This file does the opposite: it
 * drives individual reference functions directly, over inputs chosen to sweep
 * their branches, so a failure localises to one function instead of to a frame.
 *
 * Both are needed. Per-stage vectors cannot reach the state coupling *between*
 * stages -- that is what the end-to-end PCM comparison and the decode trace are
 * for -- but end-to-end comparison cannot say which stage is wrong.
 *
 * Determinism: every synthetic input comes from the ETSI pseudo-noise
 * recurrence `seed = (seed * 31821 + 13849) mod 2^16`, which the Rust side
 * reaches as `crate::fixed_point::random::random`. Seeds are stated in the
 * output so the Rust test regenerates exactly the same inputs rather than
 * carrying a copy of them.
 *
 * Note on reading the reference: it interleaves instrumentation counters
 * (`test(); move16();`) on the same line as real assignments. Never filter
 * those lines out.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "typedef.h"
#include "basic_op.h"
#include "cnst.h"
#include "mode.h"
#include "dec_lag3.h"
#include "dec_lag6.h"
#include "pred_lt.h"
#include "d2_9pf.h"
#include "d2_11pf.h"
#include "d3_14pf.h"
#include "d4_17pf.h"
#include "d8_31pf.h"
#include "d1035pf.h"
#include "syn_filt.h"
#include "pstfilt.h"
#include "post_pro.h"
#include "ph_disp.h"
#include "d_plsf.h"
#include "dec_gain.h"
#include "d_gain_p.h"
#include "d_gain_c.h"
#include "gc_pred.h"
#include "int_lpc.h"
#include "ec_gains.h"
#include "c_g_aver.h"
#include "int_lsf.h"
#include "lsp_avg.h"
#include "bgnscd.h"
#include "agc.h"
#include "weight_a.h"
#include "residu.h"
#include "preemph.h"
#include "ex_ctrl.h"

/* bitno.tab is a header-style table file: including it gives prmno/bitno
 * without linking against anything. */
#define MMS_IO
#include "bitno.tab"

/* The post-filter's bandwidth-expansion factors are file-local to pstfilt.c in
 * the reference, so they are restated here for the isolated Weight_Ai vectors.
 * The Rust side gets them from the generated table module, and a test asserts
 * the two agree. */
static const Word16 pf_gamma3_MR122[M] = {
    22938, 16057, 11240, 7868, 5508, 3856, 2699, 1889, 1322, 925};
static const Word16 pf_gamma3[M] = {
    18022, 9912, 5451, 2998, 1649, 907, 499, 274, 151, 83};
static const Word16 pf_gamma4_MR122[M] = {
    24576, 18432, 13824, 10368, 7776, 5832, 4374, 3281, 2461, 1846};
static const Word16 pf_gamma4[M] = {
    22938, 16057, 11240, 7868, 5508, 3856, 2699, 1889, 1322, 925};

/* The ETSI pseudo-noise recurrence, spelled out in basic operators so it is
 * unambiguously the same sequence the Rust side generates. */
static Word16 rnd(Word16 *seed) {
    *seed = extract_l(L_add(L_shr(L_mult(*seed, 31821), 1), 13849L));
    return *seed;
}

static void dump(const char *name, const Word16 *v, int n) {
    printf("  %s", name);
    for (int i = 0; i < n; i++) printf(" %d", v[i]);
    printf("\n");
}

static void dump1(const char *name, long v) { printf("  %s %ld\n", name, v); }

/* An algebraic codevector is 40 samples of which at most ten are non-zero, so
 * the sparse form is several times smaller and carries exactly the same
 * information. Pulses can land on the same position and add, so the count is
 * emitted rather than implied by the mode. */
static void dump_pulses(const Word16 *cod, int n) {
    int count = 0;
    for (int i = 0; i < n; i++) if (cod[i]) count++;
    printf("  nz %d", count);
    for (int i = 0; i < n; i++) if (cod[i]) printf(" %d %d", i, cod[i]);
    printf("\n");
}

/* ---------------------------------------------------------------- lag ---- */

/* Dec_lag3 is delta-coded against the previous subframe's lag except in the
 * first (and, for most modes, third) subframe, and has a separate 4-bit
 * resolution path used only by 4.75/5.15/5.90/6.70. The sweep covers both
 * absolute and delta decoding, both resolutions, and the range clamping. */
static void gen_lag3(void) {
    printf("lag3\n");
    for (int flag4 = 0; flag4 <= 1; flag4++) {
        for (int pit_flag = 0; pit_flag <= 1; pit_flag++) {
            for (int t0_prev = 20; t0_prev <= 140; t0_prev += 30) {
                Word16 t0_min, t0_max, T0, T0_frac;
                int limit;

                t0_min = t0_prev - 5;
                if (t0_min < PIT_MIN) t0_min = PIT_MIN;
                t0_max = t0_min + 9;
                if (t0_max > PIT_MAX) { t0_max = PIT_MAX; t0_min = t0_max - 9; }

                /* The absolute path uses the whole 8-bit index space; the
                 * delta path only the low values. Sweeping past the end would
                 * read outside the reference's own domain. */
                /* Field widths from bitno.tab: the absolute lag is 8 bits,
                 * the delta lag 4 bits where flag4 is set and 5 or 6
                 * otherwise. */
                limit = pit_flag ? (flag4 ? 16 : 64) : 256;
                printf("  case %d %d %d %d %d %d\n",
                       flag4, pit_flag, t0_prev, t0_min, t0_max, limit);
                printf("  out");
                for (int index = 0; index < limit; index++) {
                    Dec_lag3((Word16)index, t0_min, t0_max, (Word16)pit_flag,
                             (Word16)t0_prev, &T0, &T0_frac, (Word16)flag4);
                    printf(" %d %d", T0, T0_frac);
                }
                printf("\n");
            }
        }
    }
}

/* Dec_lag6 is 12.2 kbit/s only: 1/6 resolution, and a wider index space. */
static void gen_lag6(void) {
    printf("lag6\n");
    for (int pit_flag = 0; pit_flag <= 1; pit_flag++) {
        int limit = pit_flag ? 64 : 512;
        printf("  case %d %d\n", pit_flag, limit);
        printf("  out");
        for (int index = 0; index < limit; index++) {
            /* T0 is in/out: the delta path reads the previous subframe's lag
             * from it, so it is seeded rather than left over from the last
             * index. */
            Word16 T0 = 60, T0_frac = 0;
            Dec_lag6((Word16)index, PIT_MIN_MR122, PIT_MAX, (Word16)pit_flag,
                     &T0, &T0_frac);
            printf(" %d %d", T0, T0_frac);
        }
        printf("\n");
    }
}

/* Pred_lt_3or6 writes into the same buffer it reads. When the lag is shorter
 * than a subframe the filter reads samples it wrote earlier in the same call,
 * which is how a short lag makes the waveform repeat -- so the vector has to
 * carry the whole buffer, not just the new subframe. */
static void gen_predlt(int flag3) {
    Word16 buf[PIT_MAX + L_INTERPOL + L_SUBFR + 1];
    const int exc_off = PIT_MAX + L_INTERPOL;
    int max_frac = flag3 ? 2 : 5;

    printf(flag3 ? "predlt3\n" : "predlt6\n");
    for (int T0 = flag3 ? PIT_MIN : PIT_MIN_MR122; T0 <= PIT_MAX; T0 += 17) {
        for (int frac = 0; frac <= max_frac; frac++) {
            Word16 seed = (Word16)(1234 + T0 * 7 + frac);
            for (int i = 0; i < exc_off + L_SUBFR + 1; i++) {
                buf[i] = shr(rnd(&seed), 3);
            }
            printf("  case %d %d %d\n", T0, frac, (Word16)(1234 + T0 * 7 + frac));
            Pred_lt_3or6(&buf[exc_off], (Word16)T0, (Word16)frac, L_SUBFR,
                         (Word16)flag3);
            dump("out", &buf[exc_off], L_SUBFR);
        }
    }
}

/* ----------------------------------------------------------- codebooks ---- */

/* Coverage shape, chosen once and applied to all four narrow codebooks: sweep
 * every position index at one sign value, then sweep every sign value at a
 * sample of indices.
 *
 * The two halves fail differently. Position decoding is where the classic
 * wrong-track bug lives -- one pulse a single sample off leaves the excitation
 * sparse and plausible -- so it gets exhaustive coverage. Sign decoding is a
 * few bits of gray-coded lookup shared across positions, so a sample of
 * indices exercises it just as well and the cross product would only cost
 * fixture size.
 */
static void gen_codebooks(void) {
    Word16 cod[L_SUBFR];

    printf("cb2i40_9\n");
    for (int subNr = 0; subNr < 4; subNr++) {
        for (int index = 0; index < 128; index++) {
            decode_2i40_9bits((Word16)subNr, 0, (Word16)index, cod);
            printf("  case %d %d %d\n", subNr, 0, index);
            dump_pulses(cod, L_SUBFR);
        }
        for (int sign = 1; sign < 4; sign++)
            for (int index = 3; index < 128; index += 11) {
                decode_2i40_9bits((Word16)subNr, (Word16)sign, (Word16)index, cod);
                printf("  case %d %d %d\n", subNr, sign, index);
                dump_pulses(cod, L_SUBFR);
            }
    }

    printf("cb2i40_11\n");
    for (int index = 0; index < 512; index++) {
        decode_2i40_11bits(0, (Word16)index, cod);
        printf("  case %d %d\n", 0, index);
        dump_pulses(cod, L_SUBFR);
    }
    for (int sign = 1; sign < 4; sign++)
        for (int index = 5; index < 512; index += 17) {
            decode_2i40_11bits((Word16)sign, (Word16)index, cod);
            printf("  case %d %d\n", sign, index);
            dump_pulses(cod, L_SUBFR);
        }

    printf("cb3i40_14\n");
    for (int index = 0; index < 2048; index += 2) {
        decode_3i40_14bits(0, (Word16)index, cod);
        printf("  case %d %d\n", 0, index);
        dump_pulses(cod, L_SUBFR);
    }
    for (int sign = 1; sign < 8; sign++)
        for (int index = 7; index < 2048; index += 53) {
            decode_3i40_14bits((Word16)sign, (Word16)index, cod);
            printf("  case %d %d\n", sign, index);
            dump_pulses(cod, L_SUBFR);
        }

    printf("cb4i40_17\n");
    for (int index = 0; index < 8192; index += 7) {
        decode_4i40_17bits(0, (Word16)index, cod);
        printf("  case %d %d\n", 0, index);
        dump_pulses(cod, L_SUBFR);
    }
    for (int sign = 1; sign < 16; sign++)
        for (int index = 11; index < 8192; index += 211) {
            decode_4i40_17bits((Word16)sign, (Word16)index, cod);
            printf("  case %d %d\n", sign, index);
            dump_pulses(cod, L_SUBFR);
        }

    /* The wide codebooks take a vector of sub-indices rather than a packed
     * word. bitno.tab gives 10.2 kbit/s the fields 1,1,1,1,10,10,7: four
     * one-bit signs and three compressed position words.
     *
     * The position words have a *legal range narrower than their field width*,
     * and it is load-bearing here. The 10-bit words carry 125x8 combinations,
     * so only 0..999 occur; the 7-bit word carries 25x4, so only 0..99. Feeding
     * the full field width makes `decompress_code` return position indices
     * above 9, and `dec_8i40_31bits` then writes past the end of a 40-sample
     * codevector -- which corrupted this generator's own stack before the
     * bound was applied. A decoder must therefore treat these as untrusted
     * input rather than assume the encoder's range. */
    printf("cb8i40_31\n");
    {
        Word16 idx[7];
        Word16 seed = 4242;
        for (int c = 0; c < 250; c++) {
            printf("  case");
            for (int k = 0; k < 4; k++) {
                idx[k] = (Word16)(((unsigned)rnd(&seed) >> 3) & 1);
                printf(" %d", idx[k]);
            }
            idx[4] = (Word16)((((unsigned)rnd(&seed) >> 3) & 0x3FF) % 1000);
            idx[5] = (Word16)((((unsigned)rnd(&seed) >> 3) & 0x3FF) % 1000);
            idx[6] = (Word16)((((unsigned)rnd(&seed) >> 3) & 0x7F) % 100);
            printf(" %d %d %d\n", idx[4], idx[5], idx[6]);
            dec_8i40_31bits(idx, cod);
            dump_pulses(cod, L_SUBFR);
        }
    }

    /* 12.2 kbit/s: ten fields, the first five 4 bits (three gray-coded
     * position bits plus a sign) and the last five 3 bits (position only,
     * the sign being inherited from the paired pulse). */
    printf("cb10i40_35\n");
    {
        Word16 idx[10];
        Word16 seed = 777;
        for (int c = 0; c < 250; c++) {
            printf("  case");
            for (int k = 0; k < 10; k++) {
                int mask = k < 5 ? 0x0F : 0x07;
                idx[k] = (Word16)(((unsigned)rnd(&seed) >> 3) & mask);
                printf(" %d", idx[k]);
            }
            printf("\n");
            dec_10i40_35bits(idx, cod);
            dump_pulses(cod, L_SUBFR);
        }
    }
}

/* -------------------------------------------------------------- gains ---- */

/* The gain path is stateful: the code gain is predicted from a four-entry MA
 * history of past quantised energies, so a single-index vector would miss the
 * state update -- the defect that makes a decoder drift away from its encoder
 * over seconds rather than failing outright. Each case therefore replays a
 * sequence from the reset state. */
static void gen_gains(void) {
    static const enum Mode joint[] = {MR475, MR515, MR59, MR67, MR74, MR102};
    /* Index widths from bitno.tab. They are not uniform, and the tables are
     * sized to them exactly: 4.75 indexes a 256-entry book with 8 bits, 5.15
     * and 5.90 a 64-entry book with 6, and the rest a 128-entry book with 7.
     * A wider sweep reads off the end of the table. */
    static const int joint_mask[] = {0xFF, 0x3F, 0x3F, 0x7F, 0x7F, 0x7F};
    Word16 code[L_SUBFR];

    printf("gains\n");
    for (unsigned m = 0; m < sizeof joint / sizeof joint[0]; m++) {
        gc_predState *ps = NULL;
        Word16 seed = (Word16)(101 + m * 37);
        gc_pred_init(&ps);
        gc_pred_reset(ps);
        printf("  seq joint %d %d\n", (int)joint[m], (int)seed);
        for (int n = 0; n < 24; n++) {
            Word16 gain_pit = 0, gain_cod = 0;
            Word16 index = (Word16)(((unsigned)rnd(&seed) >> 3) & joint_mask[m]);
            Word16 evenSubfr = (Word16)(1 - (n & 1));
            for (int i = 0; i < L_SUBFR; i++) code[i] = shr(rnd(&seed), 4);
            Dec_gain(ps, joint[m], index, code, evenSubfr, &gain_pit, &gain_cod);
            printf("    step %d %d %d %d\n", index, evenSubfr, gain_pit, gain_cod);
        }
        gc_pred_exit(&ps);
    }

    /* 7.95 and 12.2 quantise the two gains separately. */
    for (int m = 0; m < 2; m++) {
        enum Mode mode = m ? MR122 : MR795;
        gc_predState *ps = NULL;
        Word16 seed = (Word16)(555 + m * 13);
        gc_pred_init(&ps);
        gc_pred_reset(ps);
        printf("  seq split %d %d\n", (int)mode, (int)seed);
        for (int n = 0; n < 24; n++) {
            Word16 gain_cod = 0;
            Word16 pindex = (Word16)(((unsigned)rnd(&seed) >> 3) & 0x0F);
            Word16 cindex = (Word16)(((unsigned)rnd(&seed) >> 3) & 0x1F);
            Word16 gain_pit = d_gain_pitch(mode, pindex);
            for (int i = 0; i < L_SUBFR; i++) code[i] = shr(rnd(&seed), 4);
            d_gain_code(ps, mode, cindex, code, &gain_cod);
            printf("    step %d %d %d %d\n", pindex, cindex, gain_pit, gain_cod);
        }
        gc_pred_exit(&ps);
    }
}

/* --------------------------------------------------------- concealment ---- */

/* The concealment gain generators carry their own histories and a
 * six-state degradation machine, so these too are replayed rather than
 * evaluated pointwise. */
static void gen_conceal(void) {
    ec_gain_pitchState *gp = NULL;
    ec_gain_codeState *gc = NULL;
    gc_predState *ps = NULL;
    Word16 seed = 8191;

    printf("conceal\n");
    ec_gain_pitch_init(&gp);
    ec_gain_code_init(&gc);
    gc_pred_init(&ps);
    ec_gain_pitch_reset(gp);
    ec_gain_code_reset(gc);
    gc_pred_reset(ps);

    printf("  seq %d\n", (int)seed);
    for (int n = 0; n < 40; n++) {
        Word16 gain_pit = 0, gain_code = 0;
        /* A repeating good/bad pattern so the state machine climbs and resets
         * rather than sitting in one state. */
        Word16 bfi = (Word16)((n / 3) & 1);
        Word16 prev_bf = (Word16)(((n - 1) / 3) & 1);
        Word16 state = (Word16)(n % 7);
        if (bfi) {
            ec_gain_pitch(gp, state, &gain_pit);
            ec_gain_code(gc, ps, state, &gain_code);
        } else {
            gain_pit = (Word16)(((unsigned)rnd(&seed) >> 2) & 0x3FFF);
            gain_code = (Word16)(((unsigned)rnd(&seed) >> 3) & 0x0FFF);
        }
        ec_gain_pitch_update(gp, bfi, prev_bf, &gain_pit);
        ec_gain_code_update(gc, bfi, prev_bf, &gain_code);
        printf("    step %d %d %d %d %d\n", bfi, prev_bf, state, gain_pit, gain_code);
    }

    ec_gain_pitch_exit(&gp);
    ec_gain_code_exit(&gc);
    gc_pred_exit(&ps);
}

/* ------------------------------------------------------- synth / filter ---- */

/* Read a real interpolated LP filter set out of the committed fixtures'
 * spectral path, so the filters exercised here are ones the codec can actually
 * produce. Synthesising "plausible" coefficients gives a filter that is not
 * minimum phase, which the earlier wideband work found produces saturated
 * nonsense and proves nothing. */
static void real_az(Word16 Az[AZ_SIZE]) {
    /* 7.40 kbit/s, third frame: the mainstream 3-split path. */
    D_plsfState *lsf = NULL;
    Word16 lsp_old[M], lsp_new[M], prm[3];
    static const Word16 init[M] = {30000, 26000, 21000, 15000, 8000,
                                   0, -8000, -15000, -21000, -26000};
    Word16 seed = 31;

    D_plsf_init(&lsf);
    D_plsf_reset(lsf);
    memcpy(lsp_old, init, sizeof init);
    for (int f = 0; f < 3; f++) {
        for (int i = 0; i < 3; i++) prm[i] = (Word16)(((unsigned)rnd(&seed) >> 3) & 0xFF);
        D_plsf_3(lsf, MR74, 0, prm, lsp_new);
        Int_lpc_1to3(lsp_old, lsp_new, Az);
        memcpy(lsp_old, lsp_new, sizeof lsp_new);
    }
    D_plsf_exit(&lsf);
}

static void gen_synth(void) {
    Word16 Az[AZ_SIZE], mem[M], exc[L_SUBFR], syn[L_FRAME];
    Word16 seed = 24680;

    real_az(Az);

    printf("synfilt\n");
    dump("az", Az, AZ_SIZE);
    dump1("seed", seed);
    memset(mem, 0, sizeof mem);
    for (int sf = 0; sf < 4; sf++) {
        for (int i = 0; i < L_SUBFR; i++) exc[i] = shr(rnd(&seed), 5);
        Syn_filt(&Az[sf * MP1], exc, &syn[sf * L_SUBFR], L_SUBFR, mem, 1);
        dump("exc", exc, L_SUBFR);
        dump("out", &syn[sf * L_SUBFR], L_SUBFR);
    }

    /* agc and agc2 both rescale one signal to another's energy; agc carries a
     * smoothing memory across calls and agc2 does not. */
    printf("agc\n");
    {
        agcState *st = NULL;
        Word16 sig_in[L_SUBFR], sig_out[L_SUBFR];
        Word16 s = 1357;
        agc_init(&st);
        agc_reset(st);
        dump1("seed", s);
        for (int n = 0; n < 6; n++) {
            for (int i = 0; i < L_SUBFR; i++) sig_in[i] = shr(rnd(&s), 4);
            for (int i = 0; i < L_SUBFR; i++) sig_out[i] = shr(rnd(&s), 4);
            agc(st, sig_in, sig_out, AGC_FAC, L_SUBFR);
            dump("out", sig_out, L_SUBFR);
            dump1("mem", st->past_gain);
        }
        agc_exit(&st);
    }

    printf("agc2\n");
    {
        Word16 sig_in[L_SUBFR], sig_out[L_SUBFR];
        Word16 s = 2468;
        dump1("seed", s);
        for (int n = 0; n < 6; n++) {
            for (int i = 0; i < L_SUBFR; i++) sig_in[i] = shr(rnd(&s), 4);
            for (int i = 0; i < L_SUBFR; i++) sig_out[i] = shr(rnd(&s), 4);
            agc2(sig_in, sig_out, L_SUBFR);
            dump("out", sig_out, L_SUBFR);
        }
    }

    /* Weight_Az and Residu are shared by the post-filter; Preemph carries a
     * one-sample memory. */
    /* Weight_Ai takes a *vector* of expansion factors, not a scalar gamma
     * raised to successive powers, so the four post-filter factor sets are
     * swept rather than a gamma sweep. */
    printf("weightai\n");
    {
        Word16 ap[MP1];
        const Word16 *fac[4] = {pf_gamma3_MR122, pf_gamma3, pf_gamma4_MR122,
                                pf_gamma4};
        for (int g = 0; g < 4; g++) {
            Weight_Ai(Az, fac[g], ap);
            printf("  case %d\n", g);
            dump("out", ap, MP1);
        }
    }

    printf("residu\n");
    {
        Word16 x[L_SUBFR + M], y[L_SUBFR];
        Word16 s = 9753;
        dump1("seed", s);
        for (int i = 0; i < L_SUBFR + M; i++) x[i] = shr(rnd(&s), 5);
        dump("in", x, L_SUBFR + M);
        Residu(Az, &x[M], y, L_SUBFR);
        dump("out", y, L_SUBFR);
    }

    printf("preemph\n");
    {
        preemphasisState *st = NULL;
        Word16 x[L_SUBFR];
        Word16 s = 1111;
        preemphasis_init(&st);
        preemphasis_reset(st);
        dump1("seed", s);
        for (int n = 0; n < 4; n++) {
            /* The coefficient the post-filter passes is a tilt measure that
             * changes every subframe, so it is swept rather than fixed. */
            Word16 g = (Word16)(((unsigned)rnd(&s) >> 3) & 0x0FFF);
            for (int i = 0; i < L_SUBFR; i++) x[i] = shr(rnd(&s), 4);
            printf("  case %d\n", g);
            preemphasis(st, x, g, L_SUBFR);
            dump("out", x, L_SUBFR);
            dump1("mem", st->mem_pre);
        }
        preemphasis_exit(&st);
    }
}

/* The post-filter is the stage AMR-WB does not have, and it is heavily
 * stateful: a residual history, a synthesis memory, a preemphasis memory and
 * an AGC gain memory, all carried across frames. Replayed over four frames. */
static void gen_postfilter(void) {
    Post_FilterState *pst = NULL;
    Post_ProcessState *pp = NULL;
    Word16 Az[AZ_SIZE], syn[L_FRAME];
    Word16 seed = 13579;

    real_az(Az);

    printf("postfilter\n");
    dump("az", Az, AZ_SIZE);
    dump1("seed", seed);
    Post_Filter_init(&pst);
    Post_Filter_reset(pst);
    for (int f = 0; f < 4; f++) {
        for (int i = 0; i < L_FRAME; i++) syn[i] = shr(rnd(&seed), 4);
        dump("in", syn, L_FRAME);
        Post_Filter(pst, MR74, syn, Az);
        dump("out", syn, L_FRAME);
    }
    Post_Filter_exit(&pst);

    printf("postproc\n");
    {
        Word16 s = 2222;
        dump1("seed", s);
        Post_Process_init(&pp);
        Post_Process_reset(pp);
        for (int f = 0; f < 4; f++) {
            for (int i = 0; i < L_FRAME; i++) syn[i] = shr(rnd(&s), 4);
            dump("in", syn, L_FRAME);
            Post_Process(pp, syn, L_FRAME);
            dump("out", syn, L_FRAME);
        }
        Post_Process_exit(&pp);
    }
}

/* Phase dispersion picks one of three impulse responses from the rate and an
 * adaption state that only ever moves one step per subframe. */
static void gen_phdisp(void) {
    static const enum Mode modes[] = {MR475, MR515, MR59, MR67, MR74, MR795,
                                      MR102, MR122};
    printf("phdisp\n");
    for (unsigned m = 0; m < sizeof modes / sizeof modes[0]; m++) {
        ph_dispState *st = NULL;
        Word16 x[L_SUBFR], inno[L_SUBFR];
        Word16 seed = (Word16)(3000 + m * 91);
        ph_disp_init(&st);
        ph_disp_reset(st);
        printf("  seq %d %d\n", (int)modes[m], (int)seed);
        for (int n = 0; n < 5; n++) {
            Word16 cbGain = (Word16)(((unsigned)rnd(&seed) >> 4) & 0x0FFF);
            Word16 ltpGain = (Word16)(((unsigned)rnd(&seed) >> 2) & 0x3FFF);
            Word16 pitch_fac = ltpGain;
            Word16 tmp_shift = (Word16)(modes[m] == MR122 ? 2 : 1);
            for (int i = 0; i < L_SUBFR; i++) x[i] = shr(rnd(&seed), 5);
            for (int i = 0; i < L_SUBFR; i++) inno[i] = shr(rnd(&seed), 4);
            ph_disp_release(st);
            printf("    step %d %d %d\n", cbGain, ltpGain, tmp_shift);
            dump("x", x, L_SUBFR);
            dump("inno", inno, L_SUBFR);
            ph_disp(st, modes[m], x, cbGain, ltpGain, inno, pitch_fac, tmp_shift);
            dump("out", x, L_SUBFR);
        }
        ph_disp_exit(&st);
    }
}

/* -------------------------------------------------------- 12.2 spectral ---- */

/* D_plsf_5 decodes two LSP sets per frame and predicts across frames, so like
 * the 3-split quantiser it is replayed as a sequence, with a bad frame at the
 * end so concealment and its distinct state update are covered. */
static void gen_plsf5(void) {
    D_plsfState *st = NULL;
    Word16 indice[5], lsp1[M], lsp2[M], Az[AZ_SIZE];
    Word16 lsp_old[M];
    static const Word16 init[M] = {30000, 26000, 21000, 15000, 8000,
                                   0, -8000, -15000, -21000, -26000};
    Word16 seed = 6180;

    printf("plsf5\n");
    dump1("seed", seed);
    D_plsf_init(&st);
    D_plsf_reset(st);
    memcpy(lsp_old, init, sizeof init);
    for (int f = 0; f < 8; f++) {
        Word16 bfi = (Word16)(f == 6 ? 1 : 0);
        /* Field widths at 12.2: 7, 8, 8+1, 8, 6 bits. */
        indice[0] = (Word16)(((unsigned)rnd(&seed) >> 3) & 0x7F);
        indice[1] = (Word16)(((unsigned)rnd(&seed) >> 3) & 0xFF);
        indice[2] = (Word16)(((unsigned)rnd(&seed) >> 3) & 0x1FF);
        indice[3] = (Word16)(((unsigned)rnd(&seed) >> 3) & 0xFF);
        indice[4] = (Word16)(((unsigned)rnd(&seed) >> 3) & 0x3F);
        printf("  frame %d %d %d %d %d %d\n", bfi,
               indice[0], indice[1], indice[2], indice[3], indice[4]);
        D_plsf_5(st, bfi, indice, lsp1, lsp2);
        dump("lsp1", lsp1, M);
        dump("lsp2", lsp2, M);
        Int_lpc_1and3(lsp_old, lsp1, lsp2, Az);
        dump("az", Az, AZ_SIZE);
        memcpy(lsp_old, lsp2, sizeof lsp2);
    }
    D_plsf_exit(&st);
}

/* ------------------------------------------------------------- helpers ---- */

/* Int_lsf, lsp_avg, Cb_gain_average and Bgn_scd are the small stateful pieces
 * that sit between gain decoding and synthesis. Each carries state that only a
 * sequence can exercise. */
static void gen_helpers(void) {
    printf("intlsf\n");
    {
        Word16 lsf_old[M], lsf_new[M], out[M];
        Word16 seed = 4096;
        dump1("seed", seed);
        for (int i = 0; i < M; i++) lsf_old[i] = (Word16)(400 + i * 700);
        for (int i = 0; i < M; i++) lsf_new[i] = (Word16)(500 + i * 690);
        dump("old", lsf_old, M);
        dump("new", lsf_new, M);
        for (int sf = 0; sf < 4; sf++) {
            Int_lsf(lsf_old, lsf_new, (Word16)(sf * L_SUBFR), out);
            printf("  case %d\n", sf * L_SUBFR);
            dump("out", out, M);
        }
    }

    printf("lspavg\n");
    {
        lsp_avgState *st = NULL;
        Word16 lsf[M];
        Word16 seed = 271;
        lsp_avg_init(&st);
        lsp_avg_reset(st);
        dump1("seed", seed);
        for (int n = 0; n < 10; n++) {
            for (int i = 0; i < M; i++)
                lsf[i] = (Word16)(300 + i * 600 + (((unsigned)rnd(&seed) >> 8) & 0x3F));
            dump("lsf", lsf, M);
            lsp_avg(st, lsf);
            dump("mean", st->lsp_meanSave, M);
        }
        lsp_avg_exit(&st);
    }

    printf("cbgainav\n");
    {
        Cb_gain_averageState *st = NULL;
        Word16 lsp[M], lsp_avg_buf[M];
        Word16 seed = 909;
        Cb_gain_average_init(&st);
        Cb_gain_average_reset(st);
        dump1("seed", seed);
        for (int i = 0; i < M; i++) lsp_avg_buf[i] = (Word16)(2000 + i * 2500);
        dump("lspavg", lsp_avg_buf, M);
        for (int n = 0; n < 12; n++) {
            Word16 gain_code = (Word16)(((unsigned)rnd(&seed) >> 3) & 0x0FFF);
            Word16 bfi = (Word16)((n / 4) & 1);
            Word16 prev_bf = (Word16)(((n - 1) / 4) & 1);
            Word16 pdfi = (Word16)(n & 1);
            Word16 prev_pdf = (Word16)((n - 1) & 1);
            Word16 bgn = (Word16)((n / 6) & 1);
            Word16 hang = (Word16)(n % 8);
            Word16 out;
            for (int i = 0; i < M; i++)
                lsp[i] = (Word16)(1900 + i * 2510 + (((unsigned)rnd(&seed) >> 9) & 0x1F));
            dump("lsp", lsp, M);
            out = Cb_gain_average(st, MR67, gain_code, lsp, lsp_avg_buf, bfi,
                                  prev_bf, pdfi, prev_pdf, bgn, hang);
            printf("    step %d %d %d %d %d %d %d -> %d\n",
                   gain_code, bfi, prev_bf, pdfi, prev_pdf, bgn, hang, out);
        }
        Cb_gain_average_exit(&st);
    }

    printf("bgnscd\n");
    {
        Bgn_scdState *st = NULL;
        Word16 ltp_hist[9], syn[L_FRAME], hangover = 0;
        Word16 seed = 55555;
        Bgn_scd_init(&st);
        Bgn_scd_reset(st);
        dump1("seed", seed);
        for (int n = 0; n < 10; n++) {
            Word16 bgn;
            for (int i = 0; i < 9; i++)
                ltp_hist[i] = (Word16)(((unsigned)rnd(&seed) >> 2) & 0x3FFF);
            for (int i = 0; i < L_FRAME; i++) syn[i] = shr(rnd(&seed), 4);
            dump("ltp", ltp_hist, 9);
            dump("syn", syn, L_FRAME);
            bgn = Bgn_scd(st, ltp_hist, syn, &hangover);
            printf("    step %d %d\n", bgn, hangover);
        }
        Bgn_scd_exit(&st);
    }

    printf("exctrl\n");
    {
        Word16 exc[L_SUBFR], hist[9];
        Word16 seed = 31337;
        dump1("seed", seed);
        for (int n = 0; n < 8; n++) {
            Word16 energy;
            Word16 hangover = (Word16)(4 + (n % 5));
            Word16 prev_bf = (Word16)(n & 1);
            Word16 careful = (Word16)((n / 2) & 1);
            for (int i = 0; i < L_SUBFR; i++) exc[i] = shr(rnd(&seed), 5);
            for (int i = 0; i < 9; i++)
                hist[i] = (Word16)(((unsigned)rnd(&seed) >> 6) & 0x03FF);
            energy = (Word16)(((unsigned)rnd(&seed) >> 6) & 0x03FF);
            dump("exc", exc, L_SUBFR);
            dump("hist", hist, 9);
            printf("    step %d %d %d %d\n", energy, hangover, prev_bf, careful);
            Ex_ctrl(exc, energy, hist, hangover, prev_bf, careful);
            dump("out", exc, L_SUBFR);
        }
    }
}

int main(void) {
    gen_lag3();
    gen_lag6();
    gen_predlt(1);
    gen_predlt(0);
    gen_codebooks();
    gen_gains();
    gen_conceal();
    gen_synth();
    gen_postfilter();
    gen_phdisp();
    gen_plsf5();
    gen_helpers();
    return 0;
}
