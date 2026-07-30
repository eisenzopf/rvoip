/*
 * Deterministic libSRTP side of scripts/test_libsrtp_interop.sh.
 *
 * The vectors below are libSRTP's own srtp_validate vectors. This helper is
 * compiled against the exact source commit fetched by the shell gate and
 * exchanges the resulting wire packets with rvoip's Rust driver.
 */

#include "srtp.h"

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define BUFFER_CAPACITY 256

static const uint8_t master_key[30] = {
    0xe1, 0xf9, 0x7a, 0x0d, 0x3e, 0x01, 0x8b, 0xe0,
    0xd6, 0x4f, 0xa3, 0x2c, 0x06, 0xde, 0x41, 0x39,
    0x0e, 0xc6, 0x75, 0xad, 0x49, 0x8a, 0xfe, 0xeb,
    0xb6, 0x96, 0x0b, 0x3a, 0xab, 0xe6,
};

static const uint8_t rtp_plaintext[28] = {
    0x80, 0x0f, 0x12, 0x34, 0xde, 0xca, 0xfb, 0xad,
    0xca, 0xfe, 0xba, 0xbe, 0xab, 0xab, 0xab, 0xab,
    0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab,
    0xab, 0xab, 0xab, 0xab,
};
static const uint8_t srtp_ciphertext[38] = {
    0x80, 0x0f, 0x12, 0x34, 0xde, 0xca, 0xfb, 0xad,
    0xca, 0xfe, 0xba, 0xbe, 0x4e, 0x55, 0xdc, 0x4c,
    0xe7, 0x99, 0x78, 0xd8, 0x8c, 0xa4, 0xd2, 0x15,
    0x94, 0x9d, 0x24, 0x02, 0xb7, 0x8d, 0x6a, 0xcc,
    0x99, 0xea, 0x17, 0x9b, 0x8d, 0xbb,
};

static const uint8_t rtcp_plaintext[24] = {
    0x81, 0xc8, 0x00, 0x0b, 0xca, 0xfe, 0xba, 0xbe,
    0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab,
    0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab,
};
static const uint8_t srtcp_ciphertext[38] = {
    0x81, 0xc8, 0x00, 0x0b, 0xca, 0xfe, 0xba, 0xbe,
    0x71, 0x28, 0x03, 0x5b, 0xe4, 0x87, 0xb9, 0xbd,
    0xbe, 0xf8, 0x90, 0x41, 0xf9, 0x77, 0xa5, 0xa8,
    0x80, 0x00, 0x00, 0x01, 0x99, 0x3e, 0x08, 0xcd,
    0x54, 0xd6, 0xc1, 0x23, 0x07, 0x98,
};

static void fail_status(const char *operation, srtp_err_status_t status)
{
    fprintf(stderr, "%s failed with libSRTP status %d\n", operation, status);
    exit(EXIT_FAILURE);
}

static void require_status(const char *operation, srtp_err_status_t status)
{
    if (status != srtp_err_status_ok) {
        fail_status(operation, status);
    }
}

static void fprint_hex(FILE *stream, const uint8_t *bytes, size_t length)
{
    size_t index;
    for (index = 0; index < length; ++index) {
        fprintf(stream, "%02x", bytes[index]);
    }
    fputc('\n', stream);
}

static int hex_nibble(char value)
{
    if (value >= '0' && value <= '9') {
        return value - '0';
    }
    if (value >= 'a' && value <= 'f') {
        return value - 'a' + 10;
    }
    if (value >= 'A' && value <= 'F') {
        return value - 'A' + 10;
    }
    return -1;
}

static size_t parse_hex(const char *input, uint8_t *output, size_t capacity)
{
    size_t input_length = strlen(input);
    size_t index;

    if ((input_length % 2) != 0 || input_length / 2 > capacity) {
        fprintf(stderr, "invalid or oversized hex packet\n");
        exit(EXIT_FAILURE);
    }

    for (index = 0; index < input_length / 2; ++index) {
        int high = hex_nibble(input[index * 2]);
        int low = hex_nibble(input[index * 2 + 1]);
        if (high < 0 || low < 0) {
            fprintf(stderr, "invalid hex packet at byte %zu\n", index);
            exit(EXIT_FAILURE);
        }
        output[index] = (uint8_t)((high << 4) | low);
    }

    return input_length / 2;
}

static void require_bytes(const char *label,
                          const uint8_t *actual,
                          size_t actual_length,
                          const uint8_t *expected,
                          size_t expected_length)
{
    if (actual_length == expected_length &&
        memcmp(actual, expected, expected_length) == 0) {
        return;
    }

    fprintf(stderr, "%s did not match libSRTP v2.8.0 known answer\n", label);
    fprintf(stderr, "expected: ");
    fprint_hex(stderr, expected, expected_length);
    fprintf(stderr, "actual:   ");
    fprint_hex(stderr, actual, actual_length);
    exit(EXIT_FAILURE);
}

static srtp_t create_context(void)
{
    srtp_policy_t policy;
    srtp_t context = NULL;

    memset(&policy, 0, sizeof(policy));
    srtp_crypto_policy_set_rtp_default(&policy.rtp);
    srtp_crypto_policy_set_rtcp_default(&policy.rtcp);
    policy.ssrc.type = ssrc_specific;
    policy.ssrc.value = 0xcafebabe;
    policy.key = (uint8_t *)master_key;
    policy.window_size = 128;
    policy.allow_repeat_tx = 0;
    policy.next = NULL;

    require_status("srtp_create", srtp_create(&context, &policy));
    return context;
}

static void protect_rtp(void)
{
    uint8_t packet[BUFFER_CAPACITY] = {0};
    int length = (int)sizeof(rtp_plaintext);
    srtp_t context = create_context();

    memcpy(packet, rtp_plaintext, sizeof(rtp_plaintext));
    require_status("srtp_protect", srtp_protect(context, packet, &length));
    require_bytes("SRTP ciphertext", packet, (size_t)length,
                  srtp_ciphertext, sizeof(srtp_ciphertext));
    fprint_hex(stdout, packet, (size_t)length);
    require_status("srtp_dealloc", srtp_dealloc(context));
}

static void unprotect_rtp(const char *input)
{
    uint8_t packet[BUFFER_CAPACITY] = {0};
    int length = (int)parse_hex(input, packet, sizeof(packet));
    srtp_t context = create_context();

    require_status("srtp_unprotect", srtp_unprotect(context, packet, &length));
    require_bytes("RTP plaintext", packet, (size_t)length,
                  rtp_plaintext, sizeof(rtp_plaintext));
    puts("ok");
    require_status("srtp_dealloc", srtp_dealloc(context));
}

static void protect_rtcp(void)
{
    uint8_t packet[BUFFER_CAPACITY] = {0};
    int length = (int)sizeof(rtcp_plaintext);
    srtp_t context = create_context();

    memcpy(packet, rtcp_plaintext, sizeof(rtcp_plaintext));
    require_status("srtp_protect_rtcp",
                   srtp_protect_rtcp(context, packet, &length));
    require_bytes("SRTCP ciphertext", packet, (size_t)length,
                  srtcp_ciphertext, sizeof(srtcp_ciphertext));
    fprint_hex(stdout, packet, (size_t)length);
    require_status("srtp_dealloc", srtp_dealloc(context));
}

static void unprotect_rtcp(const char *input)
{
    uint8_t packet[BUFFER_CAPACITY] = {0};
    int length = (int)parse_hex(input, packet, sizeof(packet));
    srtp_t context = create_context();

    require_status("srtp_unprotect_rtcp",
                   srtp_unprotect_rtcp(context, packet, &length));
    require_bytes("RTCP plaintext", packet, (size_t)length,
                  rtcp_plaintext, sizeof(rtcp_plaintext));
    puts("ok");
    require_status("srtp_dealloc", srtp_dealloc(context));
}

static void usage(const char *program)
{
    fprintf(stderr,
            "usage: %s <version|protect-rtp|unprotect-rtp|protect-rtcp|unprotect-rtcp> [hex-packet]\n",
            program);
}

int main(int argc, char **argv)
{
    require_status("srtp_init", srtp_init());

    if (argc == 2 && strcmp(argv[1], "version") == 0) {
        puts(srtp_get_version_string());
    } else if (argc == 2 && strcmp(argv[1], "protect-rtp") == 0) {
        protect_rtp();
    } else if (argc == 3 && strcmp(argv[1], "unprotect-rtp") == 0) {
        unprotect_rtp(argv[2]);
    } else if (argc == 2 && strcmp(argv[1], "protect-rtcp") == 0) {
        protect_rtcp();
    } else if (argc == 3 && strcmp(argv[1], "unprotect-rtcp") == 0) {
        unprotect_rtcp(argv[2]);
    } else {
        usage(argv[0]);
        require_status("srtp_shutdown", srtp_shutdown());
        return EXIT_FAILURE;
    }

    require_status("srtp_shutdown", srtp_shutdown());
    return EXIT_SUCCESS;
}
