/* Oracle for AMR-NB VAD2: drives TS 26.073's own vad2() over committed PCM and
 * dumps the full per-half-frame state, not just the decision.
 *
 * Same reasoning as nb_vad1_probe.c, only more so. The decision appears
 * nowhere in the bitstream, and VAD2's is additionally the OR of two calls per
 * frame, so a half-frame that is wrong can be masked entirely by its partner.
 * Comparing the boolean alone would agree with a constant most of the time.
 *
 * What is printed, per call: the decision, the counters, and all three
 * sixteen-element state arrays. A divergence in any of them localises to a
 * stage; a divergence in only the decision does not.
 *
 * vad2() consumes 80 samples -- half a 20 ms frame -- so a 160-sample frame is
 * two calls. cod_amr.c does exactly that pairing.
 */
#include <stdio.h>
#include <stdlib.h>
#include "typedef.h"
#include "basic_op.h"
#include "cnst.h"
#include "vad2.h"

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "usage: nb_vad2_probe <pcm-file>\n");
        return 1;
    }
    vadState2 *st = NULL;
    if (vad2_init(&st) != 0) return 1;
    FILE *f = fopen(argv[1], "rb");
    if (!f) return 2;

    printf("# half vad | tsnr hangover burstcount update_cnt hyster_cnt "
           "negSNRvar negSNRbias shift_state | ch_enrg[16] | ch_noise[16] | "
           "ch_enrg_long_db[16]\n");

    short raw[FRM_LEN];
    int half = 0;
    while (fread(raw, sizeof(short), FRM_LEN, f) == (size_t)FRM_LEN) {
        Word16 buf[FRM_LEN];
        for (int i = 0; i < FRM_LEN; i++) buf[i] = raw[i];

        Word16 vad = vad2(buf, st);

        printf("%d %d |", half, (int)vad);
        printf(" %d %d %d %d %d %d %d %d |",
               (int)st->tsnr, (int)st->hangover, (int)st->burstcount,
               (int)st->update_cnt, (int)st->hyster_cnt,
               (int)st->negSNRvar, (int)st->negSNRbias, (int)st->shift_state);
        for (int i = 0; i < NUM_CHAN; i++) printf(" %ld", (long)st->Lch_enrg[i]);
        printf(" |");
        for (int i = 0; i < NUM_CHAN; i++) printf(" %ld", (long)st->Lch_noise[i]);
        printf(" |");
        for (int i = 0; i < NUM_CHAN; i++) printf(" %d", (int)st->ch_enrg_long_db[i]);
        printf("\n");
        half++;
    }

    fclose(f);
    vad2_exit(&st);
    return 0;
}
