/* Generate AMR-WB and AMR-NB reference vectors with the Apache-2.0 oracle.
 *
 * Encodes a deterministic speech-like signal at every AMR-WB mode and writes
 * the frames in RFC 4867 storage format (`#!AMR-WB\n` plus one ToC-prefixed
 * record per frame), then decodes them back so the round trip is exercised
 * inside the reference itself.
 *
 * The point is to produce fixtures the Rust implementation can be checked
 * against, not to be a general tool.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>

#include <vo-amrwbenc/enc_if.h>
#include <opencore-amrwb/dec_if.h>
#include <opencore-amrnb/interf_enc.h>
#include <opencore-amrnb/interf_dec.h>

#define WB_FRAME_SAMPLES 320 /* 20 ms at 16 kHz */
#define NB_FRAME_SAMPLES 160 /* 20 ms at 8 kHz  */
#define FRAME_SAMPLES WB_FRAME_SAMPLES
#define NUM_FRAMES 25

/* Deterministic pseudo-speech: two formant-ish tones with an envelope, so the
 * encoder has real structure to work on rather than silence, and the output is
 * reproducible without shipping an audio file. */
static void fill_signal_at(short *buf, int frame_index, int n, double rate) {
    for (int i = 0; i < n; i++) {
        double t = (double)(frame_index * n + i) / rate;
        double env = 0.5 + 0.5 * sin(2.0 * M_PI * 3.0 * t);
        double s = 0.6 * sin(2.0 * M_PI * 310.0 * t) +
                   0.3 * sin(2.0 * M_PI * 1150.0 * t) +
                   0.1 * sin(2.0 * M_PI * 2700.0 * t);
        double v = env * s * 12000.0;
        if (v > 32767.0) v = 32767.0;
        if (v < -32768.0) v = -32768.0;
        buf[i] = (short)v;
    }
}

static void fill_signal(short *buf, int frame_index) {
    fill_signal_at(buf, frame_index, WB_FRAME_SAMPLES, 16000.0);
}

/* AMR-NB: same generator at 8 kHz, so the two vector sets come from the same
 * deterministic source rather than two unrelated signals. */
static int gen_narrowband(const char *dir) {
    static const int nb_payload[9] = {12,13,15,17,19,20,26,31,5};
    for (int mode = 0; mode <= 7; mode++) {
        void *enc = Encoder_Interface_init(0);
        if (!enc) { fprintf(stderr, "NB encoder init failed\n"); return 1; }

        char path[1024];
        snprintf(path, sizeof path, "%s/amrnb_mode%d.amr", dir, mode);
        FILE *f = fopen(path, "wb");
        if (!f) { perror("fopen"); return 1; }
        fwrite("#!AMR\n", 1, 6, f);

        short pcm[NB_FRAME_SAMPLES];
        unsigned char out[64];
        int total = 0, first_len = -1;
        for (int n = 0; n < NUM_FRAMES; n++) {
            fill_signal_at(pcm, n, NB_FRAME_SAMPLES, 8000.0);
            int len = Encoder_Interface_Encode(enc, (enum Mode)mode, pcm, out, 0);
            if (len <= 0) { fprintf(stderr, "NB encode failed mode %d\n", mode); return 1; }
            if (first_len < 0) first_len = len;
            fwrite(out, 1, (size_t)len, f);
            total += len;
        }
        fclose(f);
        Encoder_Interface_exit(enc);

        /* Decode back through the reference so the fixture is self-consistent
         * before the Rust side ever reads it. */
        f = fopen(path, "rb");
        char magic[6];
        if (fread(magic, 1, 6, f) != 6) { fprintf(stderr, "short read\n"); return 1; }
        void *dec = Decoder_Interface_init();
        short synth[NB_FRAME_SAMPLES];
        int frames = 0; long energy = 0;
        for (;;) {
            unsigned char hdr;
            if (fread(&hdr, 1, 1, f) != 1) break;
            int ft = (hdr >> 3) & 0x0F;
            if (ft > 8) { fprintf(stderr, "unexpected NB FT %d\n", ft); return 1; }
            int payload = nb_payload[ft];
            unsigned char buf[64];
            buf[0] = hdr;
            if (fread(buf + 1, 1, (size_t)payload, f) != (size_t)payload) break;
            Decoder_Interface_Decode(dec, buf, synth, 0);
            for (int i = 0; i < NB_FRAME_SAMPLES; i++) energy += (long)abs(synth[i]);
            frames++;
        }
        Decoder_Interface_exit(dec);
        fclose(f);

        printf("NB mode %d: frame=%d bytes (incl 1-byte ToC), %d frames, %d bytes, "
               "decoded %d frames, mean |sample| %ld\n",
               mode, first_len, NUM_FRAMES, total, frames,
               frames ? energy / (frames * NB_FRAME_SAMPLES) : 0);
    }
    return 0;
}

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "usage: %s <output-dir>\n", argv[0]);
        return 2;
    }
    const char *dir = argv[1];

    for (int mode = 0; mode <= 8; mode++) {
        void *enc = E_IF_init();
        if (!enc) { fprintf(stderr, "encoder init failed\n"); return 1; }

        char path[1024];
        snprintf(path, sizeof path, "%s/amrwb_mode%d.amr", dir, mode);
        FILE *f = fopen(path, "wb");
        if (!f) { perror("fopen"); return 1; }
        fwrite("#!AMR-WB\n", 1, 9, f);

        short pcm[FRAME_SAMPLES];
        unsigned char out[128];
        int total = 0, first_len = -1;

        for (int n = 0; n < NUM_FRAMES; n++) {
            fill_signal(pcm, n);
            int len = E_IF_encode(enc, mode, pcm, out, 0);
            if (len <= 0) { fprintf(stderr, "encode failed mode %d\n", mode); return 1; }
            if (first_len < 0) first_len = len;
            fwrite(out, 1, (size_t)len, f);
            total += len;
        }
        fclose(f);
        E_IF_exit(enc);

        /* Decode the file back through the reference decoder, so the fixture is
         * known to be self-consistent before the Rust side ever sees it. */
        f = fopen(path, "rb");
        char magic[9];
        if (fread(magic, 1, 9, f) != 9) { fprintf(stderr, "short read\n"); return 1; }
        void *dec = D_IF_init();
        short synth[FRAME_SAMPLES];
        int frames = 0;
        long energy = 0;
        for (;;) {
            unsigned char hdr;
            if (fread(&hdr, 1, 1, f) != 1) break;
            int ft = (hdr >> 3) & 0x0F;
            static const int sizes[16] = {17,23,32,36,40,46,50,58,60,5,
                                          -1,-1,-1,-1,0,0};
            int payload = sizes[ft];
            if (payload < 0) { fprintf(stderr, "bad FT %d\n", ft); return 1; }
            unsigned char buf[64];
            buf[0] = hdr;
            if (payload > 0 && fread(buf + 1, 1, (size_t)payload, f) != (size_t)payload) break;
            D_IF_decode(dec, buf, synth, _good_frame);
            for (int i = 0; i < FRAME_SAMPLES; i++) energy += (long)abs(synth[i]);
            frames++;
        }
        D_IF_exit(dec);
        fclose(f);

        printf("mode %d: frame=%d bytes (incl 1-byte ToC), %d frames, %d bytes, "
               "decoded %d frames, mean |sample| %ld\n",
               mode, first_len, NUM_FRAMES, total, frames,
               frames ? energy / (frames * FRAME_SAMPLES) : 0);
    }

    return gen_narrowband(dir);
}
