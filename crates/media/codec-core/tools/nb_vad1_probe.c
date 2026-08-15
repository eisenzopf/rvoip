/* Oracle for AMR-NB VAD1: drives TS 26.073's own vad1() over the committed DTX
 * input and dumps the full per-frame state, not just the decision.
 *
 * The decision alone is nearly vacuous -- roughly 85% of frames agree by
 * chance -- and it appears nowhere in the bitstream, so a divergence has no
 * other way to be localised.
 */
#include <stdio.h>
#include <stdlib.h>
#include "typedef.h"
#include "basic_op.h"
#include "cnst.h"
#include "vad.h"
#include "cnst_vad.h"

int main(int argc, char **argv) {
    vadState1 *st = NULL;
    if (vad1_init(&st) != 0) return 1;
    FILE *f = fopen(argv[1], "rb");
    if (!f) return 2;

    /* vad1() indexes in_buf[i - LOOKAHEAD], so the pointer it is given is the
     * start of the *current* frame and forty samples of the previous one sit
     * behind it -- exactly how cod_amr.c passes st->new_speech. Handing it a
     * frame with lookahead ahead instead reads off the front of the buffer. */
    Word16 buf[40 + L_FRAME];
    for (int i = 0; i < 40 + L_FRAME; i++) buf[i] = 0;
    short raw[L_FRAME];
    int frame = 0;
    printf("# frame vad | vadreg pitch tone complex_high complex_low stat burst hang | bckr_est[9]\n");
    while (fread(raw, sizeof(short), L_FRAME, f) == L_FRAME) {
        /* Slide the previous frame's tail into the history. */
        for (int i = 0; i < 40; i++) buf[i] = buf[L_FRAME + i];
        for (int i = 0; i < L_FRAME; i++) buf[40 + i] = raw[i];

        /* The hooks the encoder's open-loop pitch stage drives. Without
         * them the pitch, tone and complex registers never leave zero and a
         * third of the state comparison proves nothing. Deterministic
         * synthetic inputs, chosen to make each register move. */
        vad_tone_detection_update(st, (Word16)(frame % 3 == 0));
        {
            Word32 t0 = (Word32)(200000 + (frame * 37013) % 900000);
            Word32 t1 = (Word32)(150000 + (frame * 11317) % 300000);
            vad_tone_detection(st, t0, t1);
        }
        vad_complex_detection_update(st, (Word16)(8000 + (frame * 613) % 20000));
        {
            /* Lags that agree with each other for a stretch and then jump,
             * so the pitch flag both sets and clears. Wandering lags never
             * satisfy the closeness threshold and leave the register at
             * zero. */
            Word16 base = (Word16)(60 + (frame / 12) * 23);
            Word16 lags[2];
            lags[0] = (Word16)(base + (frame % 12 < 8 ? 0 : 17));
            lags[1] = (Word16)(base + (frame % 12 < 8 ? 1 : 30));
            vad_pitch_detection(st, lags);
        }

        Word16 decision = vad1(st, &buf[40]);
        printf("%d %d | %d %d %d %d %d %d %d %d |", frame, decision,
               st->vadreg, st->pitch, st->tone, st->complex_high, st->complex_low,
               st->stat_count, st->burst_count, st->hang_count);
        for (int i = 0; i < COMPLEN; i++) printf(" %d", st->bckr_est[i]);
        printf("\n");
        frame++;
    }
    fclose(f);
    vad1_exit(&st);
    return 0;
}
