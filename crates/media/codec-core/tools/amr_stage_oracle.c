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

void Autocorr(Word16 x[], Word16 m, Word16 r_h[], Word16 r_l[]);
void Lag_window(Word16 r_h[], Word16 r_l[]);
void Levinson(Word16 Rh[], Word16 Rl[], Word16 A[], Word16 rc[], Word16 *old_A,
              Word16 *old_rc);
void Az_isp(Word16 a[], Word16 isp[], Word16 old_isp[]);
void Isp_Az(Word16 isp[], Word16 a[], Word16 m, Word16 adaptive_scaling);

#define L_WIN 384

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

int main(void) {
    for (int seed = 0; seed < 4; seed++) {
        Word16 x[L_WIN];
        Word16 r_h[M + 1], r_l[M + 1];
        Word16 A[M + 1], rc[M];
        Word16 isp[M], old_isp[M], a_back[M + 1];
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
    }
    return 0;
}
