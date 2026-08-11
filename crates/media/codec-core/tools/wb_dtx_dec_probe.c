/* Oracle for AMR-WB decoder-side DTX: drives TS 26.173's own rx_dtx_handler and
 * dtx_dec over a frame-type sequence and dumps the state it reaches plus the
 * comfort noise it synthesises. */
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

/* One SID payload, MSB-first, in the Parm_serial sentinel encoding. */
static void put(Word16 **p, int value, int bits) {
    for (int i = bits - 1; i >= 0; i--)
        *(*p)++ = ((value >> i) & 1) ? BIT_1 : BIT_0;
}

int main(void) {
    dtx_decState *st = NULL;
    if (dtx_dec_init(&st, isf_init_local) != 0) return 1;

    /* Speech, then a SID_FIRST, gaps, updates, a long gap into DTX_MUTE. */
    int seq[128], n = 0;
    for (int i = 0; i < 12; i++) seq[n++] = RX_SPEECH_GOOD;
    seq[n++] = RX_SID_FIRST;
    seq[n++] = RX_NO_DATA; seq[n++] = RX_NO_DATA;
    seq[n++] = RX_SID_UPDATE;
    for (int i = 0; i < 7; i++) seq[n++] = RX_NO_DATA;
    seq[n++] = RX_SID_UPDATE;
    for (int i = 0; i < 4; i++) seq[n++] = RX_NO_DATA;
    seq[n++] = RX_SID_BAD;
    /* A gap long enough to cross DTX_MAX_EMPTY_THRESH and fade to mute, then
     * a SID_UPDATE to come back out of it. */
    for (int i = 0; i < 55; i++) seq[n++] = RX_NO_DATA;
    seq[n++] = RX_SID_UPDATE;
    for (int i = 0; i < 3; i++) seq[n++] = RX_NO_DATA;
    seq[n++] = RX_SPEECH_GOOD;

    printf("# frame ft | state | isf[0..4] | exc2[0..4]\n");
    for (int f = 0; f < n; f++) {
        Word16 newState = rx_dtx_handler(st, (Word16)seq[f]);
        printf("%d %d | %d |", f, seq[f], newState);
        if (newState != SPEECH) {
            Word16 bits[64], *p = bits;
            /* A fixed SID payload, so the comparison is about the machine
             * rather than about which indices happen to be chosen. */
            put(&p, 10, 6); put(&p, 20, 6); put(&p, 30, 6);
            put(&p, 5, 5);  put(&p, 7, 5);
            put(&p, 30, 6); put(&p, 0, 1);
            Word16 *pp = bits;
            Word16 isf[M], exc2[L_FRAME];
            dtx_dec(st, exc2, newState, isf, &pp);
            for (int i = 0; i < 5; i++) printf(" %d", isf[i]);
            printf(" |");
            for (int i = 0; i < 5; i++) printf(" %d", exc2[i]);
        } else {
            /* Speech: feed the activity update so the ring fills, as the
             * decoder does after the ACELP path. */
            Word16 isf[M], exc[L_FRAME];
            for (int i = 0; i < M; i++) isf[i] = (Word16)(500 + i * 900 + f * 7);
            for (int i = 0; i < L_FRAME; i++) exc[i] = (Word16)(((i * 37 + f * 11) % 401) - 200);
            dtx_dec_activity_update(st, isf, exc);
            printf(" - | -");
        }
        printf("\n");
        st->dtxGlobalState = newState;
    }
    dtx_dec_exit(&st);
    return 0;
}
