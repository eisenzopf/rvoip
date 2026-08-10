/* Per-stage oracle for AMR-NB, mirroring the AMR-WB one.
 *
 * Reads the committed .amr fixtures through the reference's own unsorter and
 * parameter unpacker and dumps what it gets. The fixtures were produced by
 * opencore-amr, so this compares three independent implementations on real
 * bitstreams rather than round-tripping one implementation's assumptions --
 * which is the failure mode that twice produced a self-consistent wrong answer
 * in the AMR-WB work.
 */
#include <stdio.h>
#include <string.h>

#include "typedef.h"
#include "basic_op.h"
#include "cnst.h"
#include "mode.h"
#include "bits2prm.h"
#include "d_plsf.h"
#include "lsp_az.h"
#include "int_lpc.h"
#include "lsp_lsf.h"

/* bitno.tab is a header-style table file; including it gives sort_ptr,
 * unpacked_size, packed_size and unused_size without linking. */
#define MMS_IO
#include "bitno.tab"
#include "lsp.tab"

#define MAX_BITS 244
#define FRAMES_PER_MODE 3

static void dump(const char *name, const Word16 *v, int n) {
    printf("  %s", name);
    for (int i = 0; i < n; i++) printf(" %d", v[i]);
    printf("\n");
}

/* Emit a bit array as hex so a whole frame fits on one line. */
static void dump_bits_hex(const char *name, const Word16 *bits, int n) {
    printf("  %s ", name);
    for (int i = 0; i < n; i += 4) {
        int nibble = 0;
        for (int b = 0; b < 4; b++) {
            nibble <<= 1;
            if (i + b < n && bits[i + b] == 1) nibble |= 1;
        }
        printf("%x", nibble);
    }
    printf("\n");
}

static void dump_mode(const char *dir, int mode_no) {
    char path[512], name[32];
    FILE *fp;
    UWord8 toc, packet[64];
    Word16 bits[MAX_BITS], prm[PRMNO_MR122];
    char magic[8];
    D_plsfState lsf_state;
    Word16 lsp_old[M];

    Init_D_plsf_3(&lsf_state, 0);
    memcpy(lsp_old, lsp_init_data, sizeof lsp_old);

    sprintf(path, "%s/amrnb_mode%d.amr", dir, mode_no);
    fp = fopen(path, "rb");
    if (!fp) { fprintf(stderr, "missing %s\n", path); return; }
    if (fread(magic, 1, 6, fp) != 6 || strncmp(magic, "#!AMR\n", 6)) {
        fprintf(stderr, "%s: not an AMR-NB storage file\n", path);
        fclose(fp);
        return;
    }

    printf("nb%d\n", mode_no);
    for (int f = 0; f < FRAMES_PER_MODE; f++) {
        int mode, nbits, payload, i;

        if (fread(&toc, 1, 1, fp) != 1) break;
        mode = (toc >> 3) & 0x0F;
        if (mode != mode_no) { fprintf(stderr, "mode drift at frame %d\n", f); break; }
        nbits = unpacked_size[mode];
        /* AMR-NB's packed_size INCLUDES the ToC byte; AMR-WB's does not. The
         * two reference tables use different conventions for the same name,
         * and reading packed_size payload bytes here walks the file out of
         * alignment one byte per frame. */
        payload = packed_size[mode] - 1;
        if ((int)fread(packet, 1, payload, fp) != payload) break;

        /* Unpack and unsort exactly as the reference's MMS reader does:
         * payload bit i belongs at codec position sort_ptr[mode][i]. */
        {
            UWord8 *p = packet;
            UWord8 temp = *p++;
            for (i = 1; i < nbits + 1; i++) {
                bits[sort_ptr[mode][i - 1]] = (temp & 0x80) ? 1 : 0;
                if (i % 8) temp <<= 1;
                else temp = *p++;
            }
        }

        Bits2prm((enum Mode)mode, bits, prm);

        printf("  meta%d %d %d %d\n", f, mode, nbits, prmno[mode]);
        sprintf(name, "bits%d", f);
        dump_bits_hex(name, bits, nbits);
        sprintf(name, "prm%d", f);
        dump(name, prm, prmno[mode]);

        /* Spectral path. The 3-split quantiser covers every mode except 12.2;
         * it carries MA-prediction state across frames, so this runs the whole
         * fixture in sequence rather than decoding one frame in isolation. */
        if (mode != MR122) {
            Word16 lsp_new[M], Az[AZ_SIZE];
            D_plsf_3(&lsf_state, (enum Mode)mode, 0, prm, lsp_new);
            sprintf(name, "lsp%d", f);
            dump(name, lsp_new, M);
            Int_lpc_1to3(lsp_old, lsp_new, Az);
            sprintf(name, "az%d", f);
            dump(name, Az, AZ_SIZE);
            memcpy(lsp_old, lsp_new, sizeof lsp_new);
        }
    }
    fclose(fp);
}

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "usage: amrnb_oracle <testdata-dir>\n");
        return 1;
    }
    for (int m = 0; m < 8; m++) dump_mode(argv[1], m);
    return 0;
}
