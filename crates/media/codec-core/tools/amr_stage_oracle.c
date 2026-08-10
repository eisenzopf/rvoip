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
    printf("bitstream%d\n", mode_no);
    for (int f = 0; f < FRAMES_PER_MODE; f++) {
        Word16 ok = Read_serial(fp, prms, &frame_type, &mode, rx, 2);
        int nb_bits;
        Word16 *p = prms;
        Word16 ind[7];
        int n_ind, i;
        char name[24];

        if (ok == 0) break;
        nb_bits = unpacked_size[mode];

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

        printf("  meta%d %d %d\n", f, mode, nb_bits);
        sprintf(name, "bits%d", f);
        dump_bits_hex(name, prms, nb_bits);
        sprintf(name, "isfind%d", f);
        dump(name, ind, n_ind);
        (void)i;
    }
    Close_read_serial(rx);
    fclose(fp);
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

    if (argc > 1) {
        for (int m = 0; m < 9; m++) dump_bitstream(argv[1], m);
    } else {
        fprintf(stderr, "no testdata dir given; skipping bitstream vectors\n");
    }
    return 0;
}
