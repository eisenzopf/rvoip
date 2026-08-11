/* Oracle for AMR-WB encoder-side DTX: drives TS 26.173's own dtx_buffer and
 * dtx_enc over a deterministic sequence and dumps every SID frame's parameters
 * plus the first eight excitation samples.
 *
 * Build against the fetched reference (never committed):
 *   SRC=$TMPDIR/rvoip-amr-reference/c-code
 *   cc -O1 -w -I"$SRC" -o probe wb_dtx_enc_probe.c \
 *      $(ls "$SRC"/*.c | grep -vE '/(coder|decoder)\.c$') -lm
 *   ./probe > ../src/codecs/amr/testdata/wb_dtx_enc_vectors.txt
 *
 * The input sweep is generated here rather than captured from a stream: one
 * signal's background exercises one corner of the codebooks, and the dithering
 * flag would never leave whichever value that signal produced. The first
 * sixteen frames are deliberately stationary so CN_dith reaches 0.
 *
 * Note Parm_serial writes BIT_0/BIT_1 sentinels (-127/127), not 0/1. Reading
 * them with `& 1` yields all-ones for every field, which looks like a
 * saturated quantiser rather than like a decoding bug.
 */
#include <stdio.h>
#include <stdlib.h>
#include "typedef.h"
#include "basic_op.h"
#include "cnst.h"
#include "dtx.h"
#include "bits.h"

static Word16 isf_init_local[M] = {
    1024, 2048, 3072, 4096, 5120, 6144, 7168, 8192,
    9216, 10240, 11264, 12288, 13312, 14336, 15360, 3840};

int main(void) {
    dtx_encState *st = NULL;
    if (dtx_enc_init(&st, isf_init_local) != 0) return 1;

    unsigned int lcg = 24680u;
    printf("# frame | isf_idx[5] | log_en_index | CN_dith | exc2[0..8]\n");
    for (int frame = 0; frame < 40; frame++) {
        /* A drifting spectrum and a wandering energy, so the outlier logic and
         * the dithering thresholds both get exercised rather than sitting at
         * their reset values. */
        /* Perturbations around the comfort-noise mean, which is where a real
         * background spectrum sits -- far enough to select different
         * codevectors, near enough that the search does not saturate. */
        static const Word16 mean_ns[M] = {478, 1100, 2213, 3267, 4219, 5222,
            6198, 7240, 8229, 9153, 10098, 11108, 12144, 13184, 14165, 3803};
        Word16 isf[M];
        Word32 enr;
        if (frame < 16) {
            /* A stationary background: identical spectrum, identical energy.
             * Both dithering tests must read "not moving", so CN_dith is 0 --
             * without this the flag never varies and the test proves nothing
             * about it. */
            for (int i = 0; i < M; i++) isf[i] = mean_ns[i];
            enr = 300000L;
        } else {
            for (int i = 0; i < M; i++) {
                lcg = lcg * 1103515245u + 12345u;
                Word16 jitter = (Word16)(((lcg >> 18) & 0x3FF) - 512) + (Word16)(frame * 11);
                isf[i] = (Word16)(mean_ns[i] + jitter);
            }
            lcg = lcg * 1103515245u + 12345u;
            enr = (Word32)(1L << (8 + (frame % 12))) + (Word32)((lcg >> 20) & 0x3FF);
        }
        dtx_buffer(st, isf, enr, 2 /* codec mode 12.65k */);

        Word16 out_isf[M], exc2[L_FRAME];
        Word16 bits[64], *p = bits;
        for (int i = 0; i < 64; i++) bits[i] = -1;
        dtx_enc(st, out_isf, exc2, &p);

        printf("%d |", frame);
        /* Parm_serial wrote MSB-first; rebuild the fields. */
        int at = 0;
        int widths[7] = {6, 6, 6, 5, 5, 6, 1};
        int vals[7];
        for (int f = 0; f < 7; f++) {
            int v = 0;
            for (int b = 0; b < widths[f]; b++) v = (v << 1) | (bits[at++] == BIT_1 ? 1 : 0);
            vals[f] = v;
        }
        for (int f = 0; f < 5; f++) printf(" %d", vals[f]);
        printf(" | %d | %d |", vals[5], vals[6]);
        for (int i = 0; i < 8; i++) printf(" %d", exc2[i]);
        printf("\n");
    }
    dtx_enc_exit(&st);
    return 0;
}
