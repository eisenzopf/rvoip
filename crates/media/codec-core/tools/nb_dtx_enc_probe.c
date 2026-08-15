/* Oracle for the AMR-NB DTX encoder decision, TS 26.073 `cod_amr.c`.
 *
 * The committed bitstream says which frames came out as comfort noise, but not
 * *why*: a wrong VAD decision and a wrong hangover count produce the same
 * frame-type sequence when they cancel, and a Rust port that diverges at one
 * frame gives no way to tell which of the two moved. This dumps the VAD flag,
 * the state machine's two counters and the resulting used_mode per frame.
 *
 * Note the VAD's input: `cod_amr` passes `st->new_speech`, which is the
 * *pre-processed* speech -- high-passed and halved. Feeding it raw PCM gives a
 * detector that works and disagrees.
 */
#include <stdio.h>
#include <stdlib.h>
#include "typedef.h"
#include "cnst.h"
#include "mode.h"
#include "sp_enc.h"
#include "cod_amr.h"
#include "dtx_enc.h"
#include "vad.h"

int main(int argc, char **argv) {
    if (argc < 3) { fprintf(stderr, "usage: %s input.pcm mode\n", argv[0]); return 1; }
    Speech_Encode_FrameState *st = NULL;
    if (Speech_Encode_Frame_init(&st, 1, "probe") != 0) return 1;

    FILE *f = fopen(argv[1], "rb");
    if (!f) return 2;
    enum Mode mode = (enum Mode)atoi(argv[2]);

    short raw[L_FRAME];
    Word16 serial[250];
    enum Mode usedMode;
    int frame = 0;
    printf("# frame vad_flag hangover elapsed used_mode\n");
    while (fread(raw, sizeof(short), L_FRAME, f) == L_FRAME) {
        Word16 speech[L_FRAME];
        for (int i = 0; i < L_FRAME; i++) speech[i] = raw[i];
        Speech_Encode_Frame(st, mode, speech, serial, &usedMode);

        dtx_encState *d = st->cod_amr_state->dtx_encSt;
        vadState1 *v = (vadState1 *)st->cod_amr_state->vadSt;
        printf("%d %d %d %d %d\n", frame,
               (int)v->vadreg, (int)d->dtxHangoverCount,
               (int)d->decAnaElapsedCount, (int)usedMode);
        frame++;
    }
    fclose(f);
    Speech_Encode_Frame_exit(&st);
    return 0;
}
