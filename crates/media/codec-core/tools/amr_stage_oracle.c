/* Per-stage oracle for the AMR-WB LP analysis chain.
 *
 * Runs the TS 26.173 reference functions in sequence on a deterministic input
 * and dumps every intermediate exactly, so a Rust implementation can be
 * compared stage by stage rather than only at the end. A whole-chain mismatch
 * says "something is wrong"; a per-stage dump says which stage.
 *
 * Inputs are generated here rather than fed in, so the vectors are reproducible
 * from this file alone. Crucially the ISPs are produced by running the
 * reference's own analysis, not synthesised: hand-made "ordered values" are not
 * generally the roots of a minimum-phase filter, and feeding those to Isp_Az
 * produces saturated nonsense that looks like output but tests nothing.
 */
#include <stdio.h>
#include <string.h>

/* Not <math.h>: the reference's basic_op.h declares its own `round(Word32)`,
 * which collides with the C library's `round(double)`. Only sin and cos are
 * needed here, so declare them directly. */
extern double sin(double);
extern double cos(double);
#define PI 3.14159265358979323846

#include "typedef.h"
#include "basic_op.h"
#include "cnst.h"
#include "bits.h"   /* RX_State, Read_serial, Serial_parm */
#include "oper_32b.h"  /* L_Extract, Mpy_32_16 */

void Autocorr(Word16 x[], Word16 m, Word16 r_h[], Word16 r_l[]);
void Lag_window(Word16 r_h[], Word16 r_l[]);
void Levinson(Word16 Rh[], Word16 Rl[], Word16 A[], Word16 rc[], Word16 *old_A,
              Word16 *old_rc);
void Az_isp(Word16 a[], Word16 isp[], Word16 old_isp[]);
void Isp_Az(Word16 isp[], Word16 a[], Word16 m, Word16 adaptive_scaling);
void Isp_isf(Word16 isp[], Word16 isf[], Word16 m);
void Isf_isp(Word16 isf[], Word16 isp[], Word16 m);
void Int_isp(Word16 isp_old[], Word16 isp_new[], Word16 frac[], Word16 Az[]);
void Dpisf_2s_46b(Word16 *indice, Word16 *isf_q, Word16 *past_isfq,
                  Word16 *isfold, Word16 *isf_buf, Word16 bfi, Word16 enc_dec);
void Dpisf_2s_36b(Word16 *indice, Word16 *isf_q, Word16 *past_isfq,
                  Word16 *isfold, Word16 *isf_buf, Word16 bfi, Word16 enc_dec);
void Set_zero(Word16 x[], Word16 L);
void Copy(Word16 x[], Word16 y[], Word16 L);
void DEC_ACELP_2t64_fx(Word16 index, Word16 code[]);
void DEC_ACELP_4t64_fx(Word16 index[], Word16 nbbits, Word16 code[]);
void Init_D_gain2(Word16 *mem);
void Pred_lt4(Word16 exc[], Word16 T0, Word16 frac, Word16 L_subfr);
void Preemph(Word16 x[], Word16 mu, Word16 lg, Word16 *mem);
void Pit_shrp(Word16 *x, Word16 pit_lag, Word16 sharp, Word16 L_subfr);
void Syn_filt_32(Word16 a[], Word16 m, Word16 exc[], Word16 Qnew,
                 Word16 sig_hi[], Word16 sig_lo[], Word16 lg);
void Deemph_32(Word16 x_hi[], Word16 x_lo[], Word16 y[], Word16 mu, Word16 L,
               Word16 *mem);
void Init_HP50_12k8(Word16 mem[]);
void Scale_sig(Word16 x[], Word16 lg, Word16 exp);
void Init_Oversamp_16k(Word16 mem[]);
Word16 Random(Word16 *seed);
Word16 voice_factor(Word16 exc[], Word16 Q_exc, Word16 gain_pit, Word16 code[],
                    Word16 gain_code, Word16 L_subfr);
void Phase_dispersion(Word16 gain_code, Word16 gain_pit, Word16 code[],
                      Word16 mode, Word16 disp_mem[]);
void Isf_Extrapolation(Word16 HfIsf[]);
void Init_HP400_12k8(Word16 mem[]);
void HP400_12k8(Word16 signal[], Word16 lg, Word16 mem[]);
void Weight_a(Word16 a[], Word16 ap[], Word16 gamma, Word16 m);
void Syn_filt(Word16 a[], Word16 m, Word16 x[], Word16 y[], Word16 lg,
              Word16 mem[], Word16 update);
void Init_Filt_6k_7k(Word16 mem[]);
void Filt_6k_7k(Word16 signal[], Word16 lg, Word16 mem[]);
void Init_Filt_7k(Word16 mem[]);
void Filt_7k(Word16 signal[], Word16 lg, Word16 mem[]);
Word16 div_s(Word16 var1, Word16 var2);
Word32 Dot_product12(Word16 x[], Word16 y[], Word16 lg, Word16 *exp);
void Isqrt_n(Word32 *frac, Word16 *exp);
void Oversamp_16k(Word16 sig12k8[], Word16 lg, Word16 sig16k[], Word16 mem[]);
void HP50_12k8(Word16 signal[], Word16 lg, Word16 mem[]);
void D_gain2(Word16 index, Word16 nbits, Word16 code[], Word16 L_subfr,
             Word16 *gain_pit, Word32 *gain_cod, Word16 bfi, Word16 prev_bfi,
             Word16 state, Word16 unusable_frame, Word16 vad_hist, Word16 *mem);
/* dec_main.c keeps this static, so it cannot be linked against; it is short
 * and exactly reproduced here. Evenly spaced ISFs -- a flat spectrum, which is
 * the decoder's documented reset state. */
static Word16 isf_init[M] = {
    1024, 2048, 3072, 4096, 5120, 6144, 7168, 8192,
    9216, 10240, 11264, 12288, 13312, 14336, 15360, 3840
};

#define L_WIN 384

/* The interpolation weights the decoder uses, from dec_main.c. Not uniform
 * quarters: {0.45, 0.8, 0.96, 1.0}, weighted hard toward the new frame. */
static Word16 interpol_frac[4] = {14746, 26214, 31457, 32767};

/* Deterministic speech-like input: two formants under a slow envelope. */
static void make_speech(Word16 x[L_WIN], int seed) {
    double f1 = 300.0 + (seed % 7) * 40.0;
    double f2 = 1100.0 + (seed % 5) * 90.0;
    for (int n = 0; n < L_WIN; n++) {
        double t = (double)n / 12800.0;
        double env = 0.5 + 0.5 * sin(2.0 * PI * 3.0 * t);
        double v = env * (0.6 * sin(2.0 * PI * f1 * t) +
                          0.3 * sin(2.0 * PI * f2 * t)) * 12000.0;
        if (v > 32767.0) v = 32767.0;
        if (v < -32768.0) v = -32768.0;
        x[n] = (Word16)v;
    }
}

static void dump(const char *name, const Word16 *v, int n) {
    printf("  %s", name);
    for (int i = 0; i < n; i++) printf(" %d", v[i]);
    printf("\n");
}

/* ISF dequantisation.
 *
 * This is stateful: the quantiser is predictive, so a frame's output depends
 * on the residual of the frame before it. Dumping a single frame would test
 * almost nothing -- it would miss whether the state update is right, which is
 * where a decoder silently diverges from its encoder. So each case runs a
 * sequence of frames from the documented reset state, and the last frame is
 * marked bad, which exercises the concealment path *and* its distinct state
 * update.
 *
 * Indices are derived arithmetically rather than randomly so the vectors are
 * reproducible from this file alone.
 */
#define N_ISF_FRAMES 6
#define SIZE_BK1  256
#define SIZE_BK2  256

static const Word16 sizes_46b[7] = {SIZE_BK1, SIZE_BK2, 64, 128, 128, 32, 32};
static const Word16 sizes_36b[5] = {SIZE_BK1, SIZE_BK2, 128, 128, 64};

static void dump_isf_dequant(int variant) {
    Word16 past_isfq[M], isfold[M], isf_buf[L_MEANBUF * M], isf_q[M];
    Word16 indice[7];
    int n_ind = variant == 0 ? 7 : 5;
    const Word16 *sizes = variant == 0 ? sizes_46b : sizes_36b;

    /* The reset state from dec_main.c: no past residual, and the ISF history
     * primed with the initial vector rather than left at zero. */
    Set_zero(past_isfq, M);
    Copy(isf_init, isfold, M);
    for (int j = 0; j < L_MEANBUF; j++) Copy(isf_init, &isf_buf[j * M], M);

    printf("isfdq%d\n", variant);
    for (int f = 0; f < N_ISF_FRAMES; f++) {
        Word16 bfi = (f == N_ISF_FRAMES - 1) ? 1 : 0;
        char name[24];

        for (int i = 0; i < n_ind; i++) {
            indice[i] = (Word16)((f * 37 + i * 101 + 13) % sizes[i]);
        }

        if (variant == 0)
            Dpisf_2s_46b(indice, isf_q, past_isfq, isfold, isf_buf, bfi, 1);
        else
            Dpisf_2s_36b(indice, isf_q, past_isfq, isfold, isf_buf, bfi, 1);

        sprintf(name, "ind%d", f);
        dump(name, indice, n_ind);
        sprintf(name, "isfq%d", f);
        dump(name, isf_q, M);
        sprintf(name, "past%d", f);
        dump(name, past_isfq, M);

        /* The decoder carries the result forward as the next frame's history,
         * which is what makes concealment reach for a plausible spectrum. */
        Copy(isf_q, isfold, M);
    }
}

/* Bitstream unpacking.
 *
 * The payload carries codec bits sorted by subjective importance (TS 26.201),
 * not in the order the decoder reads them, so unpacking is a permutation
 * followed by a field walk. The sorting tables live in mime_io.tab inside the
 * reference itself, so this needs no secondary source.
 *
 * The input is the committed .amr fixtures, which were produced by the
 * *other* oracles (opencore-amr and vo-amrwbenc). Running real third-party
 * bitstreams through the reference's own unsorter is a much stronger check
 * than round-tripping synthetic parameters through my own understanding of
 * the format: it would catch a permutation that is self-consistent but wrong.
 */
#define MAX_PRM 477
#define FRAMES_PER_MODE 3

/* Speech bits per mode. In MIME mode Read_serial returns 1 for "parsed a
 * frame", not a bit count, so the length has to come from here. Static inside
 * mime_io.tab, hence reproduced. */
static const int unpacked_size[9] = {132, 177, 253, 285, 317, 365, 397, 461, 477};

/* Emit a bit array as hex, MSB of each nibble first, so a whole frame fits on
 * one line and a Rust test can compare it as a string. */
static void dump_bits_hex(const char *name, const Word16 *bits, int n) {
    printf("  %s ", name);
    for (int i = 0; i < n; i += 4) {
        int nibble = 0;
        for (int b = 0; b < 4; b++) {
            nibble <<= 1;
            if (i + b < n && bits[i + b] == BIT_1) nibble |= 1;
        }
        printf("%x", nibble);
    }
    printf("\n");
}

static void dump_bitstream(const char *dir, int mode_no) {
    char path[512];
    FILE *fp;
    RX_State *rx = NULL;
    Word16 prms[MAX_PRM], frame_type, mode;
    Word16 dec_gain[23];
    char magic[16];

    sprintf(path, "%s/amrwb_mode%d.amr", dir, mode_no);
    fp = fopen(path, "rb");
    if (!fp) { fprintf(stderr, "missing %s\n", path); return; }

    /* Read_serial expects the caller to have consumed the magic. */
    if (fread(magic, 1, 9, fp) != 9 || strncmp(magic, "#!AMR-WB\n", 9)) {
        fprintf(stderr, "%s: not an AMR-WB storage file\n", path);
        fclose(fp);
        return;
    }

    Init_read_serial(&rx);
    Init_D_gain2(dec_gain);
    printf("bitstream%d\n", mode_no);
    for (int f = 0; f < FRAMES_PER_MODE; f++) {
        Word16 ok = Read_serial(fp, prms, &frame_type, &mode, rx, 2);
        int nb_bits;
        Word16 *p = prms;
        Word16 ind[7];
        Word16 T0_min_carry = 0, vad_flag;
        int n_ind, i;
        char name[24];

        if (ok == 0) break;
        nb_bits = unpacked_size[mode];

        /* The VAD flag is the very first bit of a speech frame -- read in
         * dec_main.c before the ISFs, not after. Skipping it shifts every
         * later field by one bit, which still yields plausible indices. */
        vad_flag = Serial_parm(1, &p);

        /* The 6.60 kbit/s mode spends 36 bits on the spectrum, the rest 46. */
        n_ind = (mode == 0) ? 5 : 7;
        if (n_ind == 5) {
            ind[0] = Serial_parm(8, &p); ind[1] = Serial_parm(8, &p);
            ind[2] = Serial_parm(7, &p); ind[3] = Serial_parm(7, &p);
            ind[4] = Serial_parm(6, &p);
        } else {
            ind[0] = Serial_parm(8, &p); ind[1] = Serial_parm(8, &p);
            ind[2] = Serial_parm(6, &p); ind[3] = Serial_parm(7, &p);
            ind[4] = Serial_parm(7, &p); ind[5] = Serial_parm(5, &p);
            ind[6] = Serial_parm(5, &p);
        }

        printf("  meta%d %d %d %d\n", f, mode, nb_bits, vad_flag);
        sprintf(name, "bits%d", f);
        dump_bits_hex(name, prms, nb_bits);
        sprintf(name, "isfind%d", f);
        dump(name, ind, n_ind);

        /* The excitation parameters, subframe by subframe, exactly as
         * dec_main.c walks them. Pitch lag width and the presence of an LTP
         * filter bit both depend on the mode, and subframes 1 and 3 (plus 2 at
         * 6.60 kbit/s) code the lag relative to a window around the previous
         * one -- so a layout error shows up as a plausible but wrong lag
         * rather than an overrun. */
        for (int sf = 0; sf < 4; sf++) {
            Word16 T0, T0_frac, T0_min = 0, T0_max = 0, select, gain_index, hf_gain;
            Word16 pulses[8];
            int n_pulses = 0, pit_flag = sf * 64;
            int index;

            if (sf == 2 && nb_bits > NBBITS_7k) pit_flag = 0;

            if (pit_flag == 0) {
                if (nb_bits <= NBBITS_9k) {
                    index = Serial_parm(8, &p);
                    if (index < (PIT_FR1_8b - PIT_MIN) * 2) {
                        T0 = PIT_MIN + (index >> 1);
                        T0_frac = (index - ((T0 - PIT_MIN) << 1)) << 1;
                    } else {
                        T0 = index + PIT_FR1_8b - ((PIT_FR1_8b - PIT_MIN) * 2);
                        T0_frac = 0;
                    }
                } else {
                    index = Serial_parm(9, &p);
                    if (index < (PIT_FR2 - PIT_MIN) * 4) {
                        T0 = PIT_MIN + (index >> 2);
                        T0_frac = index - ((T0 - PIT_MIN) << 2);
                    } else if (index < ((PIT_FR2 - PIT_MIN) * 4 + (PIT_FR1_9b - PIT_FR2) * 2)) {
                        index -= (PIT_FR2 - PIT_MIN) * 4;
                        T0 = PIT_FR2 + (index >> 1);
                        T0_frac = (index - ((T0 - PIT_FR2) << 1)) << 1;
                    } else {
                        T0 = index + (PIT_FR1_9b - ((PIT_FR2 - PIT_MIN) * 4)
                                      - ((PIT_FR1_9b - PIT_FR2) * 2));
                        T0_frac = 0;
                    }
                }
                T0_min = T0 - 8;
                if (T0_min < PIT_MIN) T0_min = PIT_MIN;
                T0_max = T0_min + 15;
                if (T0_max > PIT_MAX) { T0_max = PIT_MAX; T0_min = T0_max - 15; }
            } else {
                if (nb_bits <= NBBITS_9k) {
                    index = Serial_parm(5, &p);
                    T0 = T0_min_carry + (index >> 1);
                    T0_frac = (index - ((T0 - T0_min_carry) << 1)) << 1;
                } else {
                    index = Serial_parm(6, &p);
                    T0 = T0_min_carry + (index >> 2);
                    T0_frac = index - ((T0 - T0_min_carry) << 2);
                }
            }
            if (pit_flag == 0) T0_min_carry = T0_min;

            select = (nb_bits <= NBBITS_9k) ? 0 : Serial_parm(1, &p);

            if (nb_bits <= NBBITS_7k) {
                pulses[0] = Serial_parm(12, &p); n_pulses = 1;
            } else if (nb_bits <= NBBITS_9k) {
                for (i = 0; i < 4; i++) pulses[i] = Serial_parm(5, &p);
                n_pulses = 4;
            } else if (nb_bits <= NBBITS_12k) {
                for (i = 0; i < 4; i++) pulses[i] = Serial_parm(9, &p);
                n_pulses = 4;
            } else if (nb_bits <= NBBITS_14k) {
                pulses[0] = Serial_parm(13, &p); pulses[1] = Serial_parm(13, &p);
                pulses[2] = Serial_parm(9, &p);  pulses[3] = Serial_parm(9, &p);
                n_pulses = 4;
            } else if (nb_bits <= NBBITS_16k) {
                for (i = 0; i < 4; i++) pulses[i] = Serial_parm(13, &p);
                n_pulses = 4;
            } else if (nb_bits <= NBBITS_18k) {
                for (i = 0; i < 4; i++) pulses[i] = Serial_parm(2, &p);
                for (i = 4; i < 8; i++) pulses[i] = Serial_parm(14, &p);
                n_pulses = 8;
            } else if (nb_bits <= NBBITS_20k) {
                pulses[0] = Serial_parm(10, &p); pulses[1] = Serial_parm(10, &p);
                pulses[2] = Serial_parm(2, &p);  pulses[3] = Serial_parm(2, &p);
                pulses[4] = Serial_parm(10, &p); pulses[5] = Serial_parm(10, &p);
                pulses[6] = Serial_parm(14, &p); pulses[7] = Serial_parm(14, &p);
                n_pulses = 8;
            } else {
                for (i = 0; i < 4; i++) pulses[i] = Serial_parm(11, &p);
                for (i = 4; i < 8; i++) pulses[i] = Serial_parm(11, &p);
                n_pulses = 8;
            }

            gain_index = (nb_bits <= NBBITS_9k) ? Serial_parm(6, &p)
                                                : Serial_parm(7, &p);

            /* The 23.85 high-band gain is read HERE, inside the subframe, not
             * grouped at the end of the frame. dec_main.c reads it immediately
             * before calling synthesis() for this subframe. */
            hf_gain = (nb_bits >= NBBITS_24k) ? Serial_parm(4, &p) : -1;

            printf("  sf%d_%d %d %d %d %d %d", f, sf, T0, T0_frac, select,
                   gain_index, hf_gain);
            for (i = 0; i < n_pulses; i++) printf(" %d", pulses[i]);
            printf("\n");

            /* The algebraic codebook vector these indices expand to. Sparse --
             * two pulses at 6.60 kbit/s, twenty-four at 23.85 -- but dumped in
             * full, because a decoder that puts a pulse in the wrong track
             * still produces the right number of pulses. */
            {
                Word16 code[64];
                if (nb_bits <= NBBITS_7k)       DEC_ACELP_2t64_fx(pulses[0], code);
                else if (nb_bits <= NBBITS_9k)  DEC_ACELP_4t64_fx(pulses, 20, code);
                else if (nb_bits <= NBBITS_12k) DEC_ACELP_4t64_fx(pulses, 36, code);
                else if (nb_bits <= NBBITS_14k) DEC_ACELP_4t64_fx(pulses, 44, code);
                else if (nb_bits <= NBBITS_16k) DEC_ACELP_4t64_fx(pulses, 52, code);
                else if (nb_bits <= NBBITS_18k) DEC_ACELP_4t64_fx(pulses, 64, code);
                else if (nb_bits <= NBBITS_20k) DEC_ACELP_4t64_fx(pulses, 72, code);
                else                            DEC_ACELP_4t64_fx(pulses, 88, code);
                sprintf(name, "code%d_%d", f, sf);
                dump(name, code, 64);

                /* Gains. The predictor runs across subframes and frames, so
                 * dec_gain state is carried for the whole file rather than
                 * reset per subframe -- decoding one in isolation would give a
                 * different answer. */
                {
                    Word16 gain_pit;
                    Word32 gain_cod;
                    D_gain2(gain_index, (nb_bits <= NBBITS_9k) ? 6 : 7, code, 64,
                            &gain_pit, &gain_cod, 0, 0, 0, 0, 0, dec_gain);
                    /* The code gain is Q16 and does not fit a Word16, so it
                     * is printed as a plain signed 32-bit value. */
                    printf("  gain%d_%d %d %ld\n", f, sf, gain_pit, (long)gain_cod);
                }
            }
        }

        /* Whatever is left is the high-band gain, present only at 23.85. */
        printf("  tail%d %d\n", f, nb_bits - (int)(p - prms));
    }
    Close_read_serial(rx);
    fclose(fp);
}

/* Long-term prediction: the adaptive codebook and the three filters that
 * shape a subframe's excitation.
 *
 * These are driven from a synthetic but deterministic excitation history
 * rather than a real decode, because the full excitation loop carries adaptive
 * Q scaling across subframes -- that belongs with synthesis. What is covered
 * here is the arithmetic each operation performs, over lag and fraction
 * combinations chosen to hit every branch: integer lags, all four fractions,
 * and lags both shorter and longer than a subframe.
 */
#define PIT_MAX_L   231
#define L_INTERP_L  17

static void dump_ltp(void) {
    static const Word16 lags[6]  = {34, 60, 64, 100, 128, 231};
    static const Word16 fracs[4] = {0, 1, 2, 3};
    Word16 buf[PIT_MAX_L + L_INTERP_L + 65];
    Word16 hist[PIT_MAX_L + L_INTERP_L];
    Word16 *exc = buf + PIT_MAX_L + L_INTERP_L;
    int n, li, fi, c = 0;
    char name[32];

    /* A pitch-like history: a decaying pulse train under a slow drift, so the
     * interpolator has real structure to work on rather than a constant. */
    for (n = 0; n < PIT_MAX_L + L_INTERP_L; n++) {
        int phase = n % 57;
        double v = (phase < 3 ? 8000.0 - phase * 2000.0 : -300.0 + phase * 12.0);
        v *= 0.6 + 0.4 * sin(2.0 * PI * n / 313.0);
        hist[n] = (Word16)v;
    }

    printf("ltp\n");
    for (li = 0; li < 6; li++) {
        for (fi = 0; fi < 4; fi++) {
            Word16 code[64];
            Word16 mem = 0, sharp_lag;

            memcpy(buf, hist, sizeof hist);
            memset(exc, 0, 65 * sizeof(Word16));

            /* 65, not 64: the LTP low-pass filter reads one sample ahead. */
            Pred_lt4(exc, lags[li], fracs[fi], 65);
            sprintf(name, "pred%d", c);
            printf("  meta%d %d %d\n", c, lags[li], fracs[fi]);
            dump(name, exc, 65);

            /* The low-pass variant the higher rates can select. */
            {
                Word16 filt[64];
                for (n = 0; n < 64; n++) {
                    Word32 t = L_mult(5898, exc[n - 1]);
                    t = L_mac(t, 20972, exc[n]);
                    t = L_mac(t, 5898, exc[n + 1]);
                    filt[n] = round(t);
                }
                sprintf(name, "ltpf%d", c);
                dump(name, filt, 64);
            }

            /* Preemphasis then pitch sharpening, applied to the innovation.
             * A deterministic sparse vector stands in for a decoded one. */
            memset(code, 0, sizeof code);
            for (n = 0; n < 8; n++) {
                code[(n * 7 + li * 3 + fi) % 64] += (n % 2) ? -512 : 512;
            }
            mem = 0;
            Preemph(code, 6554, 64, &mem);    /* tilt_code = 0.2 in Q15 */
            sharp_lag = lags[li];
            if (fracs[fi] > 2) sharp_lag++;
            Pit_shrp(code, sharp_lag, 27853, 64);
            sprintf(name, "shrp%d", c);
            dump(name, code, 64);

            c++;
        }
    }
}

/* Low-band synthesis: excitation to 12.8 kHz speech.
 *
 * Three stages -- the LP synthesis filter, de-emphasis, and a 50 Hz high-pass
 * -- each carrying state across subframes, so this runs a sequence rather than
 * one block. The filter is driven by the real coefficient sets already dumped
 * for the LP stages, and by a deterministic excitation.
 */
static void dump_synthesis(void) {
    Word16 mem_syn_hi[M], mem_syn_lo[M], mem_deemph = 0, mem_hp[6];
    Word16 synth_hi[M + 64], synth_lo[M + 64], synth[64], exc[64];
    Word16 a[M + 1];
    int blk, n, i;
    char name[24];

    /* A stable low-order predictor, expressed the way the codec carries it:
     * Q12 with a[0] = 1.0. Taken from a real analysis so the filter has a
     * plausible spectrum rather than an arbitrary one. */
    static const Word16 a_real[M + 1] = {
        4096, -3559, 1097, -175, -313, 292, -73, -119, 158, -83,
        -20, 72, -60, 12, 25, -31, 12
    };

    memset(mem_syn_hi, 0, sizeof mem_syn_hi);
    memset(mem_syn_lo, 0, sizeof mem_syn_lo);
    Init_HP50_12k8(mem_hp);
    Copy((Word16 *)a_real, a, M + 1);

    printf("synth\n");
    for (blk = 0; blk < 4; blk++) {
        /* Deterministic excitation: a pulse train plus a drifting tone, which
         * is close enough to a real one to exercise the filter's range. */
        for (n = 0; n < 64; n++) {
            int t = blk * 64 + n;
            double v = (t % 53 == 0) ? 6000.0 : 0.0;
            v += 900.0 * sin(2.0 * PI * t / 41.0);
            exc[n] = (Word16)v;
        }
        sprintf(name, "exc%d", blk);
        dump(name, exc, 64);

        Copy(mem_syn_hi, synth_hi, M);
        Copy(mem_syn_lo, synth_lo, M);
        Syn_filt_32(a, M, exc, 0, synth_hi + M, synth_lo + M, 64);
        Copy(synth_hi + 64, mem_syn_hi, M);
        Copy(synth_lo + 64, mem_syn_lo, M);
        sprintf(name, "synhi%d", blk);
        dump(name, synth_hi + M, 64);
        sprintf(name, "synlo%d", blk);
        dump(name, synth_lo + M, 64);

        Deemph_32(synth_hi + M, synth_lo + M, synth, 22282, 64, &mem_deemph);
        sprintf(name, "deemph%d", blk);
        dump(name, synth, 64);

        HP50_12k8(synth, 64, mem_hp);
        sprintf(name, "hp50_%d", blk);
        dump(name, synth, 64);
        (void)i;
    }
}

/* Excitation assembly and 12.8 -> 16 kHz upsampling.
 *
 * The assembly is where the decoder's adaptive scaling lives: the excitation
 * buffer is kept at a per-subframe shift chosen from the code gain and the
 * recent peak, and rescaled whole whenever that shift moves. Both the shift
 * history and the buffer carry across subframes, so this replays a sequence.
 */
#define Q_MAX_L 8

static void dump_excitation(void) {
    Word16 buf[PIT_MAX_L + L_INTERP_L + 4 * 64];
    Word16 *exc = buf + PIT_MAX_L + L_INTERP_L;
    /* The decoder's documented reset state is Q_MAX, not zero -- see
     * Reset_decoder in dec_main.c. Seeding these to zero pins the shift to
     * zero until four subframes have run and mis-scales the start of a
     * stream, so the fixture must start where the decoder does. */
    Word16 Qsubfr[4] = {Q_MAX_L, Q_MAX_L, Q_MAX_L, Q_MAX_L};
    Word16 Q_old = Q_MAX_L;
    Word16 up_mem[24], sig16k[80];
    Word16 code[64];
    int sf, n, i;
    char name[24];

    memset(buf, 0, sizeof buf);
    Init_Oversamp_16k(up_mem);

    printf("excasm\n");
    for (sf = 0; sf < 4; sf++) {
        Word16 gain_pit = (Word16)(4000 + sf * 3000);      /* Q14 */
        Word32 L_gain_code = 300000L * (sf + 1);           /* Q16 */
        Word16 gain_code, Q_new, tmp, max;
        Word16 speech[64];

        /* A deterministic adaptive contribution and innovation. */
        for (n = 0; n < 64; n++) {
            exc[sf * 64 + n] = (Word16)(2000.0 * sin(2.0 * PI * (sf * 64 + n) / 37.0));
            code[n] = ((n % 11) == sf % 11) ? ((n & 1) ? -512 : 512) : 0;
        }

        /* Pick the shift: the smallest of the last four subframes' headroom,
         * capped, then grown while the code gain still has room. */
        tmp = Qsubfr[0];
        for (i = 1; i < 4; i++) if (Qsubfr[i] < tmp) tmp = Qsubfr[i];
        if (tmp > Q_MAX_L) tmp = Q_MAX_L;

        Q_new = 0;
        {
            Word32 L_tmp = L_gain_code;
            while ((L_tmp < 0x08000000L) && (Q_new < tmp)) {
                L_tmp = L_shl(L_tmp, 1);
                Q_new = add(Q_new, 1);
            }
            gain_code = round(L_tmp);
        }

        Scale_sig(exc + sf * 64 - (PIT_MAX_L + L_INTERP_L),
                  PIT_MAX_L + L_INTERP_L + 64, sub(Q_new, Q_old));
        Q_old = Q_new;

        for (n = 0; n < 64; n++) {
            Word32 L_tmp = L_mult(code[n], gain_code);
            L_tmp = L_shl(L_tmp, 5);
            L_tmp = L_mac(L_tmp, exc[n + sf * 64], gain_pit);
            L_tmp = L_shl(L_tmp, 1);
            exc[n + sf * 64] = round(L_tmp);
        }

        max = 1;
        for (n = 0; n < 64; n++) {
            Word16 a = abs_s(exc[n + sf * 64]);
            if (sub(a, max) > 0) max = a;
        }
        tmp = sub(add(norm_s(max), Q_new), 1);
        Qsubfr[3] = Qsubfr[2]; Qsubfr[2] = Qsubfr[1];
        Qsubfr[1] = Qsubfr[0]; Qsubfr[0] = tmp;

        printf("  meta%d %d %d %ld\n", sf, gain_pit, gain_code, (long)L_gain_code);
        sprintf(name, "code%d", sf);
        dump(name, code, 64);
        sprintf(name, "exc%d", sf);
        dump(name, exc + sf * 64, 64);
        sprintf(name, "q%d", sf);
        dump(name, Qsubfr, 4);

        /* Upsampling, driven by the assembled excitation so the two stages
         * are exercised on the same data. */
        Copy(exc + sf * 64, speech, 64);
        Oversamp_16k(speech, 64, sig16k, up_mem);
        sprintf(name, "up%d", sf);
        dump(name, sig16k, 80);
    }
}

/* High-band synthesis.
 *
 * The 5.5-7.5 kHz band is not transmitted at all below 23.85 kbit/s: it is
 * generated from noise whose energy matches the excitation and whose loudness
 * is set by the low band's spectral tilt. This is what makes the codec
 * wideband rather than 12.8 kHz speech resampled.
 *
 * Driven from deterministic excitation and synthesis, with the reference's own
 * random generator so the noise sequence matches exactly.
 */
static void dump_highband(void) {
    static const Word16 a_real[M + 1] = {
        4096, -3559, 1097, -175, -313, 292, -73, -119, 158, -83,
        -20, 72, -60, 12, 25, -31, 12
    };
    Word16 seed = 21845, mem_hp400[6], mem_syn_hf[M16k], mem_hf[30], mem_hf3[30];
    Word16 exc[64], synth[64], HF[80], Ap[M16k + 1], a[M + 1];
    Word16 ener, exp_ener, exp, tmp, fac, gain1, gain2, weight1, weight2;
    Word32 L_tmp;
    int blk, n, i;
    char name[24];

    Init_HP400_12k8(mem_hp400);
    memset(mem_syn_hf, 0, sizeof mem_syn_hf);
    Init_Filt_6k_7k(mem_hf);
    Init_Filt_7k(mem_hf3);
    Copy((Word16 *)a_real, a, M + 1);

    printf("highband\n");
    for (blk = 0; blk < 3; blk++) {
        Word16 Q_new = 0;
        Word16 vad_hist = (blk == 2) ? 3 : 0;   /* exercise both weightings */

        for (n = 0; n < 64; n++) {
            int t = blk * 64 + n;
            exc[n]   = (Word16)(3000.0 * sin(2.0 * PI * t / 29.0));
            synth[n] = (Word16)(6000.0 * sin(2.0 * PI * t / 61.0));
        }
        sprintf(name, "hexc%d", blk);
        dump(name, exc, 64);
        sprintf(name, "hsyn%d", blk);
        dump(name, synth, 64);

        for (i = 0; i < 80; i++) HF[i] = shr(Random(&seed), 3);
        sprintf(name, "noise%d", blk);
        dump(name, HF, 80);

        /* Match the noise energy to the excitation's. */
        Scale_sig(exc, 64, -3);
        Q_new = sub(Q_new, 3);
        ener = extract_h(Dot_product12(exc, exc, 64, &exp_ener));
        exp_ener = sub(exp_ener, add(Q_new, Q_new));

        tmp = extract_h(Dot_product12(HF, HF, 80, &exp));
        if (sub(tmp, ener) > 0) { tmp = shr(tmp, 1); exp = add(exp, 1); }
        L_tmp = L_deposit_h(div_s(tmp, ener));
        exp = sub(exp, exp_ener);
        Isqrt_n(&L_tmp, &exp);
        L_tmp = L_shl(L_tmp, add(exp, 1));
        tmp = extract_h(L_tmp);
        for (i = 0; i < 80; i++) HF[i] = mult(HF[i], tmp);
        sprintf(name, "matched%d", blk);
        dump(name, HF, 80);

        /* Tilt of the synthesis: r[1]/r[0] after a 400 Hz high-pass. */
        HP400_12k8(synth, 64, mem_hp400);
        sprintf(name, "hp400_%d", blk);
        dump(name, synth, 64);

        L_tmp = 1L;
        for (i = 0; i < 64; i++) L_tmp = L_mac(L_tmp, synth[i], synth[i]);
        exp = norm_l(L_tmp);
        ener = extract_h(L_shl(L_tmp, exp));

        L_tmp = 1L;
        for (i = 1; i < 64; i++) L_tmp = L_mac(L_tmp, synth[i], synth[i - 1]);
        tmp = extract_h(L_shl(L_tmp, exp));
        fac = (tmp > 0) ? div_s(tmp, ener) : 0;

        gain1 = sub(32767, fac);
        gain2 = mult(sub(32767, fac), 20480);
        gain2 = shl(gain2, 1);
        if (vad_hist > 0) { weight1 = 0; weight2 = 32767; }
        else              { weight1 = 32767; weight2 = 0; }
        tmp = mult(weight1, gain1);
        tmp = add(tmp, mult(weight2, gain2));
        if (tmp != 0) tmp = add(tmp, 1);
        if (sub(tmp, 3277) < 0) tmp = 3277;

        printf("  hmeta%d %d %d\n", blk, fac, tmp);
        for (i = 0; i < 80; i++) HF[i] = mult(HF[i], tmp);
        sprintf(name, "tilted%d", blk);
        dump(name, HF, 80);

        /* Shape the noise with a bandwidth-expanded copy of the LP filter. */
        Weight_a(a, Ap, 19661, M);
        sprintf(name, "weighted%d", blk);
        dump(name, Ap, M + 1);
        Syn_filt(Ap, M, HF, HF, 80, mem_syn_hf + (M16k - M), 1);
        sprintf(name, "shaped%d", blk);
        dump(name, HF, 80);

        Filt_6k_7k(HF, 80, mem_hf);
        sprintf(name, "band%d", blk);
        dump(name, HF, 80);

        Filt_7k(HF, 80, mem_hf3);
        sprintf(name, "lp7k%d", blk);
        dump(name, HF, 80);
    }
}

/* The enhancement stages between gain decoding and synthesis.
 *
 * These sit on the main data path even though the prose spec presents them as
 * refinements, and they are what makes the excitation fed to the synthesis
 * filter differ from the one written back to the adaptive-codebook history.
 */
static void dump_enhancers(void) {
    Word16 disp_mem[8];
    Word16 exc[64], code[64], code2[64];
    int blk, n, i;
    char name[24];

    memset(disp_mem, 0, sizeof disp_mem);

    printf("enhance\n");
    for (blk = 0; blk < 5; blk++) {
        /* Sweep the pitch gain across the dispersion state thresholds (0.6 and
         * 0.9 in Q14) so every branch is exercised, and vary the code gain to
         * trip the onset test. */
        Word16 gain_pit  = (Word16)(4000 + blk * 3000);
        Word16 gain_code = (Word16)(500 + blk * blk * 900);
        Word16 voice_fac, tmp;

        for (n = 0; n < 64; n++) {
            exc[n]  = (Word16)(2500.0 * sin(2.0 * PI * (blk * 64 + n) / 31.0));
            code[n] = ((n % 9) == blk % 9) ? ((n & 1) ? -512 : 512) : 0;
        }
        printf("  emeta%d %d %d\n", blk, gain_pit, gain_code);
        sprintf(name, "eexc%d", blk);
        dump(name, exc, 64);
        sprintf(name, "ecode%d", blk);
        dump(name, code, 64);

        voice_fac = voice_factor(exc, -3, gain_pit, code, gain_code, 64);
        printf("  vfac%d %d\n", blk, voice_fac);

        /* Dispersion level by rate: 0 high, 1 low, 2 off. Cycle it so all
         * three appear, and so the carried state sees each. */
        Phase_dispersion(gain_code, gain_pit, code, (Word16)(blk % 3), disp_mem);
        sprintf(name, "disp%d", blk);
        dump(name, code, 64);
        sprintf(name, "dmem%d", blk);
        dump(name, disp_mem, 8);

        /* Pitch enhancer: an HP filter of the innovation whose strength tracks
         * voicing. */
        tmp = add(shr(voice_fac, 3), 4096);
        {
            Word32 L_tmp = L_deposit_h(code[0]);
            L_tmp = L_msu(L_tmp, code[1], tmp);
            code2[0] = round(L_tmp);
            for (i = 1; i < 63; i++) {
                L_tmp = L_deposit_h(code[i]);
                L_tmp = L_msu(L_tmp, code[i + 1], tmp);
                L_tmp = L_msu(L_tmp, code[i - 1], tmp);
                code2[i] = round(L_tmp);
            }
            L_tmp = L_deposit_h(code[63]);
            L_tmp = L_msu(L_tmp, code[62], tmp);
            code2[63] = round(L_tmp);
        }
        sprintf(name, "pitchenh%d", blk);
        dump(name, code2, 64);

        /* Noise enhancer, with the ISF-distance stability factor it needs.
         * The threshold state is carried across blocks. */
        {
            static Word32 L_gc_thres = 0;
            Word16 isf[M], isfold[M], stab_fac, fac;
            Word32 L_tmp, L_gain_code = 400000L * (blk + 1);
            Word16 gc_hi, gc_lo;

            /* Two ISF sets whose distance grows with the block, so both the
             * stable and unstable ends of the range are covered. */
            for (i = 0; i < M; i++) {
                isf[i]    = (Word16)(1000 + i * 900);
                /* The perturbation has to be large: stab_fac saturates at 1.0
                 * for anything close to stable, so small differences would
                 * leave every block at 32767 and test nothing. */
                isfold[i] = (Word16)(1000 + i * 900 + blk * blk * 220 * (i % 3));
            }

            L_tmp = 0;
            for (i = 0; i < M - 1; i++) {
                Word16 d = sub(isf[i], isfold[i]);
                L_tmp = L_mac(L_tmp, d, d);
            }
            tmp = extract_h(L_shl(L_tmp, 8));
            tmp = mult(tmp, 26214);
            tmp = sub(20480, tmp);
            stab_fac = shl(tmp, 1);
            if (stab_fac < 0) stab_fac = 0;

            tmp = sub(16384, shr(voice_fac, 1));
            fac = mult(stab_fac, tmp);

            L_Extract(L_gain_code, &gc_hi, &gc_lo);
            L_tmp = L_gain_code;
            if (L_sub(L_tmp, L_gc_thres) < 0) {
                L_tmp = L_add(L_tmp, Mpy_32_16(gc_hi, gc_lo, 6226));
                if (L_sub(L_tmp, L_gc_thres) > 0) L_tmp = L_gc_thres;
            } else {
                L_tmp = Mpy_32_16(gc_hi, gc_lo, 27536);
                if (L_sub(L_tmp, L_gc_thres) < 0) L_tmp = L_gc_thres;
            }
            L_gc_thres = L_tmp;

            {
                Word32 out = Mpy_32_16(gc_hi, gc_lo, sub(32767, fac));
                Word16 t_hi, t_lo;
                L_Extract(L_tmp, &t_hi, &t_lo);
                out = L_add(out, Mpy_32_16(t_hi, t_lo, fac));
                printf("  nenh%d %d %d %ld %ld %ld\n", blk, stab_fac, fac,
                       (long)L_gain_code, (long)L_gc_thres, (long)out);
            }
        }
    }
}

/* ISF extrapolation: the 6.60 kbit/s high-band branch.
 *
 * Every other mode shapes the high band with a bandwidth-expanded copy of the
 * low band's filter. At 6.60 there is not enough spectral detail to borrow, so
 * an order-20 filter is extrapolated from the low band's ISF *spacing* --
 * whichever of three lag hypotheses correlates best is continued outward.
 */
static void dump_isf_extrap(void) {
    Word16 isf[M16k];
    int c, i;
    char name[24];

    /* Real dequantised ISF sets, not synthesised ones. A hand-made ascending
     * ramp is not a valid ISF vector: the sixteenth value is the halved
     * trailing predictor coefficient, not another line frequency, and
     * continuing the ramp into it drives Isf_isp's table index out of range.
     * C reads past the table silently, so such vectors look plausible and mean
     * nothing. These come from running the quantiser. */
    {
        Word16 past_isfq[M], isfold[M], isf_buf[L_MEANBUF * M];
        Word16 indice[7];
        Set_zero(past_isfq, M);
        Copy(isf_init, isfold, M);
        for (i = 0; i < L_MEANBUF; i++) Copy(isf_init, &isf_buf[i * M], M);

        printf("isfextrp\n");
        for (c = 0; c < 4; c++) {
            int k;
            for (k = 0; k < 7; k++)
                indice[k] = (Word16)((c * 53 + k * 31 + 7) % sizes_46b[k]);
            Dpisf_2s_46b(indice, isf, past_isfq, isfold, isf_buf, 0, 1);
            Copy(isf, isfold, M);
            for (i = M; i < M16k; i++) isf[i] = 0;

            sprintf(name, "xin%d", c);
            dump(name, isf, M);
            Isf_Extrapolation(isf);
            sprintf(name, "xout%d", c);
            dump(name, isf, M16k);
        }
    }
}

int main(int argc, char **argv) {
    for (int seed = 0; seed < 4; seed++) {
        Word16 x[L_WIN];
        Word16 r_h[M + 1], r_l[M + 1];
        Word16 A[M + 1], rc[M];
        Word16 isp[M], old_isp[M], a_back[M + 1];
        Word16 isf[M], isp_rt[M], az_int[4 * (M + 1)];
        Word16 old_A[M + 1], old_rc[2];

        make_speech(x, seed);

        /* Levinson carries state across frames; start it from the defined
         * initial condition rather than whatever is on the stack. */
        memset(old_A, 0, sizeof old_A);
        old_A[0] = 4096;                   /* 1.0 in Q12 */
        memset(old_rc, 0, sizeof old_rc);

        /* Az_isp needs a previous-frame ISP set; the reference initialises it
         * to evenly spaced values, which is the codec's reset state. */
        for (int i = 0; i < M; i++) {
            old_isp[i] = (Word16)(cos(PI * (i + 1) / (M + 1)) * 32767.0);
        }

        printf("case %d\n", seed);
        dump("x", x, L_WIN);               /* full input, so Rust consumes the exact samples */

        Autocorr(x, M, r_h, r_l);
        dump("r_h_prelag", r_h, M + 1);
        dump("r_l_prelag", r_l, M + 1);

        Lag_window(r_h, r_l);
        dump("r_h", r_h, M + 1);
        dump("r_l", r_l, M + 1);

        Levinson(r_h, r_l, A, rc, old_A, old_rc);
        dump("A", A, M + 1);
        dump("rc", rc, M);

        Az_isp(A, isp, old_isp);
        dump("isp", isp, M);

        Isp_Az(isp, a_back, M, 0);
        dump("a_back", a_back, M + 1);

        /* ISP to ISF and back. The round trip is not the identity -- both
         * directions interpolate a 129-entry table -- so dump both, and let
         * the Rust match each direction rather than only their composition. */
        Isp_isf(isp, isf, M);
        dump("isf", isf, M);
        Isf_isp(isf, isp_rt, M);
        dump("isp_rt", isp_rt, M);

        /* Interpolation across the four subframes. old_isp is the reset-state
         * ISP set, so this exercises a genuine frame-to-frame transition. */
        Int_isp(old_isp, isp, interpol_frac, az_int);
        for (int k = 0; k < 4; k++) {
            char name[16];
            sprintf(name, "az_int%d", k);
            dump(name, az_int + k * (M + 1), M + 1);
        }
    }

    dump_isf_dequant(0);
    dump_isf_dequant(1);
    dump_ltp();
    dump_synthesis();
    dump_excitation();
    dump_highband();
    dump_enhancers();
    dump_isf_extrap();

    if (argc > 1) {
        for (int m = 0; m < 9; m++) dump_bitstream(argv[1], m);
    } else {
        fprintf(stderr, "no testdata dir given; skipping bitstream vectors\n");
    }
    return 0;
}
