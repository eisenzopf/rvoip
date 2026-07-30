/*
 * Deterministic libSRTP side of scripts/test_libsrtp_interop.sh.
 *
 * The RTP vectors below are libSRTP's own srtp_validate vectors. The RTCP
 * vector uses a structurally valid Sender Report because the upstream
 * crypto-only fixture's RTCP length field does not match its byte array.
 * This helper is compiled against the exact source commit fetched by the
 * shell gate and exchanges the resulting wire packets with rvoip's driver.
 */

#include "srtp.h"

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define BUFFER_CAPACITY 256

typedef enum {
    profile_sha1_80,
    profile_sha1_32,
    profile_aes256_sha1_80,
    profile_aes256_sha1_32,
} interop_profile_t;

static const uint8_t master_key[30] = {
    0xe1, 0xf9, 0x7a, 0x0d, 0x3e, 0x01, 0x8b, 0xe0,
    0xd6, 0x4f, 0xa3, 0x2c, 0x06, 0xde, 0x41, 0x39,
    0x0e, 0xc6, 0x75, 0xad, 0x49, 0x8a, 0xfe, 0xeb,
    0xb6, 0x96, 0x0b, 0x3a, 0xab, 0xe6,
};

static const uint8_t master_key_256[46] = {
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
    0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17,
    0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
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
static const uint8_t srtp_sha1_32_ciphertext[32] = {
    0x80, 0x0f, 0x12, 0x34, 0xde, 0xca, 0xfb, 0xad,
    0xca, 0xfe, 0xba, 0xbe, 0x4e, 0x55, 0xdc, 0x4c,
    0xe7, 0x99, 0x78, 0xd8, 0x8c, 0xa4, 0xd2, 0x15,
    0x94, 0x9d, 0x24, 0x02, 0xb7, 0x8d, 0x6a, 0xcc,
};
static const uint8_t srtp_aes256_sha1_80_ciphertext[38] = {
    0x80, 0x0f, 0x12, 0x34, 0xde, 0xca, 0xfb, 0xad,
    0xca, 0xfe, 0xba, 0xbe, 0x2e, 0xaf, 0xab, 0x4c,
    0x54, 0x11, 0xba, 0xca, 0x23, 0x55, 0xd5, 0x53,
    0xeb, 0x99, 0xf2, 0x52, 0x0e, 0x6b, 0x00, 0xdb,
    0xd5, 0x1d, 0x0c, 0x94, 0xfc, 0x25,
};
static const uint8_t srtp_aes256_sha1_32_ciphertext[32] = {
    0x80, 0x0f, 0x12, 0x34, 0xde, 0xca, 0xfb, 0xad,
    0xca, 0xfe, 0xba, 0xbe, 0x2e, 0xaf, 0xab, 0x4c,
    0x54, 0x11, 0xba, 0xca, 0x23, 0x55, 0xd5, 0x53,
    0xeb, 0x99, 0xf2, 0x52, 0x0e, 0x6b, 0x00, 0xdb,
};

static const uint8_t rtcp_plaintext[28] = {
    0x80, 0xc8, 0x00, 0x06, 0xca, 0xfe, 0xba, 0xbe,
    0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab,
    0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab,
    0xab, 0xab, 0xab, 0xab,
};
static const uint8_t srtcp_ciphertext[42] = {
    0x80, 0xc8, 0x00, 0x06, 0xca, 0xfe, 0xba, 0xbe,
    0x71, 0x28, 0x03, 0x5b, 0xe4, 0x87, 0xb9, 0xbd,
    0xbe, 0xf8, 0x90, 0x41, 0xf9, 0x77, 0xa5, 0xa8,
    0xd5, 0xef, 0xb8, 0x81, 0x80, 0x00, 0x00, 0x01,
    0x90, 0x5c, 0x6f, 0x4d, 0x79, 0x32, 0xc1, 0xca,
    0xa1, 0x82,
};
static const uint8_t srtcp_aes256_ciphertext[42] = {
    0x80, 0xc8, 0x00, 0x06, 0xca, 0xfe, 0xba, 0xbe,
    0x90, 0x22, 0x6f, 0x76, 0x90, 0x76, 0x83, 0x87,
    0x99, 0xdb, 0x59, 0xe0, 0x76, 0x80, 0x16, 0x3b,
    0xc3, 0x03, 0x8c, 0x6e, 0x80, 0x00, 0x00, 0x01,
    0x02, 0x5f, 0xed, 0xdf, 0x1a, 0xd3, 0x45, 0x0e,
    0x4d, 0x12,
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

static void fprint_hex_pair(FILE *stream,
                            const uint8_t *first,
                            size_t first_length,
                            const uint8_t *second,
                            size_t second_length)
{
    size_t index;
    for (index = 0; index < first_length; ++index) {
        fprintf(stream, "%02x", first[index]);
    }
    fputc(':', stream);
    for (index = 0; index < second_length; ++index) {
        fprintf(stream, "%02x", second[index]);
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

static void parse_hex_pair(const char *input,
                           uint8_t *first,
                           size_t *first_length,
                           uint8_t *second,
                           size_t *second_length)
{
    const char *separator = strchr(input, ':');
    char first_hex[BUFFER_CAPACITY * 2 + 1];
    size_t first_hex_length;

    if (separator == NULL || strchr(separator + 1, ':') != NULL) {
        fprintf(stderr, "rollover input must contain exactly two packets\n");
        exit(EXIT_FAILURE);
    }
    first_hex_length = (size_t)(separator - input);
    if (first_hex_length >= sizeof(first_hex)) {
        fprintf(stderr, "first rollover packet is oversized\n");
        exit(EXIT_FAILURE);
    }
    memcpy(first_hex, input, first_hex_length);
    first_hex[first_hex_length] = '\0';

    *first_length = parse_hex(first_hex, first, BUFFER_CAPACITY);
    *second_length = parse_hex(separator + 1, second, BUFFER_CAPACITY);
}

static void rtp_plaintext_with_sequence(uint16_t sequence,
                                        uint8_t output[sizeof(rtp_plaintext)])
{
    memcpy(output, rtp_plaintext, sizeof(rtp_plaintext));
    output[2] = (uint8_t)(sequence >> 8);
    output[3] = (uint8_t)sequence;
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

static interop_profile_t parse_profile(const char *profile)
{
    if (strcmp(profile, "sha1-80") == 0) {
        return profile_sha1_80;
    }
    if (strcmp(profile, "sha1-32") == 0) {
        return profile_sha1_32;
    }
    if (strcmp(profile, "aes256-sha1-80") == 0) {
        return profile_aes256_sha1_80;
    }
    if (strcmp(profile, "aes256-sha1-32") == 0) {
        return profile_aes256_sha1_32;
    }

    fprintf(stderr, "unsupported SRTP profile: %s\n", profile);
    exit(EXIT_FAILURE);
}

static srtp_t create_context(interop_profile_t profile)
{
    srtp_policy_t policy;
    srtp_t context = NULL;

    memset(&policy, 0, sizeof(policy));
    if (profile == profile_aes256_sha1_32) {
        srtp_crypto_policy_set_aes_cm_256_hmac_sha1_32(&policy.rtp);
        srtp_crypto_policy_set_aes_cm_256_hmac_sha1_80(&policy.rtcp);
    } else if (profile == profile_aes256_sha1_80) {
        srtp_crypto_policy_set_aes_cm_256_hmac_sha1_80(&policy.rtp);
        srtp_crypto_policy_set_aes_cm_256_hmac_sha1_80(&policy.rtcp);
    } else if (profile == profile_sha1_32) {
        srtp_crypto_policy_set_aes_cm_128_hmac_sha1_32(&policy.rtp);
        srtp_crypto_policy_set_rtcp_default(&policy.rtcp);
    } else {
        srtp_crypto_policy_set_rtp_default(&policy.rtp);
        srtp_crypto_policy_set_rtcp_default(&policy.rtcp);
    }
    policy.ssrc.type = ssrc_specific;
    policy.ssrc.value = 0xcafebabe;
    policy.key = (uint8_t *)(profile == profile_aes256_sha1_80 ||
                                     profile == profile_aes256_sha1_32
                                 ? master_key_256
                                 : master_key);
    policy.window_size = 128;
    policy.allow_repeat_tx = 0;
    policy.next = NULL;

    require_status("srtp_create", srtp_create(&context, &policy));
    return context;
}

static void protect_rtp(interop_profile_t profile)
{
    uint8_t packet[BUFFER_CAPACITY] = {0};
    int length = (int)sizeof(rtp_plaintext);
    const uint8_t *expected =
        profile == profile_sha1_32
            ? srtp_sha1_32_ciphertext
            : profile == profile_aes256_sha1_32
                  ? srtp_aes256_sha1_32_ciphertext
                  : profile == profile_aes256_sha1_80
                        ? srtp_aes256_sha1_80_ciphertext
                        : srtp_ciphertext;
    size_t expected_length =
        profile == profile_sha1_32 || profile == profile_aes256_sha1_32
            ? sizeof(srtp_sha1_32_ciphertext)
            : sizeof(srtp_ciphertext);
    srtp_t context = create_context(profile);

    memcpy(packet, rtp_plaintext, sizeof(rtp_plaintext));
    require_status("srtp_protect", srtp_protect(context, packet, &length));
    require_bytes("SRTP ciphertext", packet, (size_t)length,
                  expected, expected_length);
    fprint_hex(stdout, packet, (size_t)length);
    require_status("srtp_dealloc", srtp_dealloc(context));
}

static void unprotect_rtp(interop_profile_t profile, const char *input)
{
    uint8_t packet[BUFFER_CAPACITY] = {0};
    int length = (int)parse_hex(input, packet, sizeof(packet));
    srtp_t context = create_context(profile);

    require_status("srtp_unprotect", srtp_unprotect(context, packet, &length));
    require_bytes("RTP plaintext", packet, (size_t)length,
                  rtp_plaintext, sizeof(rtp_plaintext));
    puts("ok");
    require_status("srtp_dealloc", srtp_dealloc(context));
}

static void protect_rtcp(interop_profile_t profile)
{
    uint8_t packet[BUFFER_CAPACITY] = {0};
    int length = (int)sizeof(rtcp_plaintext);
    srtp_t context = create_context(profile);

    memcpy(packet, rtcp_plaintext, sizeof(rtcp_plaintext));
    require_status("srtp_protect_rtcp",
                   srtp_protect_rtcp(context, packet, &length));
    const uint8_t *expected =
        profile == profile_aes256_sha1_80 || profile == profile_aes256_sha1_32
            ? srtcp_aes256_ciphertext
            : srtcp_ciphertext;
    require_bytes("SRTCP ciphertext", packet, (size_t)length,
                  expected, sizeof(srtcp_ciphertext));
    fprint_hex(stdout, packet, (size_t)length);
    require_status("srtp_dealloc", srtp_dealloc(context));
}

static void unprotect_rtcp(interop_profile_t profile, const char *input)
{
    uint8_t packet[BUFFER_CAPACITY] = {0};
    int length = (int)parse_hex(input, packet, sizeof(packet));
    srtp_t context = create_context(profile);

    require_status("srtp_unprotect_rtcp",
                   srtp_unprotect_rtcp(context, packet, &length));
    require_bytes("RTCP plaintext", packet, (size_t)length,
                  rtcp_plaintext, sizeof(rtcp_plaintext));
    puts("ok");
    require_status("srtp_dealloc", srtp_dealloc(context));
}

static void protect_rtp_rollover(interop_profile_t profile)
{
    uint8_t first[BUFFER_CAPACITY] = {0};
    uint8_t second[BUFFER_CAPACITY] = {0};
    int first_length = (int)sizeof(rtp_plaintext);
    int second_length = (int)sizeof(rtp_plaintext);
    srtp_t context = create_context(profile);

    rtp_plaintext_with_sequence(UINT16_MAX, first);
    rtp_plaintext_with_sequence(0, second);
    require_status("srtp_protect rollover 65535",
                   srtp_protect(context, first, &first_length));
    require_status("srtp_protect rollover 0",
                   srtp_protect(context, second, &second_length));
    fprint_hex_pair(stdout, first, (size_t)first_length,
                    second, (size_t)second_length);
    require_status("srtp_dealloc", srtp_dealloc(context));
}

static void unprotect_rtp_rollover(interop_profile_t profile, const char *input)
{
    uint8_t first[BUFFER_CAPACITY] = {0};
    uint8_t second[BUFFER_CAPACITY] = {0};
    uint8_t first_plaintext[sizeof(rtp_plaintext)];
    uint8_t second_plaintext[sizeof(rtp_plaintext)];
    size_t parsed_first_length;
    size_t parsed_second_length;
    int first_length;
    int second_length;
    srtp_t context = create_context(profile);

    parse_hex_pair(input, first, &parsed_first_length,
                   second, &parsed_second_length);
    first_length = (int)parsed_first_length;
    second_length = (int)parsed_second_length;
    require_status("srtp_unprotect rollover 65535",
                   srtp_unprotect(context, first, &first_length));
    require_status("srtp_unprotect rollover 0",
                   srtp_unprotect(context, second, &second_length));

    rtp_plaintext_with_sequence(UINT16_MAX, first_plaintext);
    rtp_plaintext_with_sequence(0, second_plaintext);
    require_bytes("rollover RTP plaintext 65535", first, (size_t)first_length,
                  first_plaintext, sizeof(first_plaintext));
    require_bytes("rollover RTP plaintext 0", second, (size_t)second_length,
                  second_plaintext, sizeof(second_plaintext));
    puts("ok");
    require_status("srtp_dealloc", srtp_dealloc(context));
}

static void usage(const char *program)
{
    fprintf(stderr,
            "usage: %s version | <sha1-80|sha1-32|aes256-sha1-80|aes256-sha1-32> <protect-rtp|unprotect-rtp|protect-rtcp|unprotect-rtcp|protect-rtp-rollover|unprotect-rtp-rollover> [hex-packet]\n",
            program);
}

int main(int argc, char **argv)
{
    require_status("srtp_init", srtp_init());

    if (argc == 2 && strcmp(argv[1], "version") == 0) {
        puts(srtp_get_version_string());
    } else if (argc == 3 && strcmp(argv[2], "protect-rtp") == 0) {
        protect_rtp(parse_profile(argv[1]));
    } else if (argc == 4 && strcmp(argv[2], "unprotect-rtp") == 0) {
        unprotect_rtp(parse_profile(argv[1]), argv[3]);
    } else if (argc == 3 && strcmp(argv[2], "protect-rtcp") == 0) {
        protect_rtcp(parse_profile(argv[1]));
    } else if (argc == 4 && strcmp(argv[2], "unprotect-rtcp") == 0) {
        unprotect_rtcp(parse_profile(argv[1]), argv[3]);
    } else if (argc == 3 && strcmp(argv[2], "protect-rtp-rollover") == 0) {
        protect_rtp_rollover(parse_profile(argv[1]));
    } else if (argc == 4 && strcmp(argv[2], "unprotect-rtp-rollover") == 0) {
        unprotect_rtp_rollover(parse_profile(argv[1]), argv[3]);
    } else {
        usage(argv[0]);
        require_status("srtp_shutdown", srtp_shutdown());
        return EXIT_FAILURE;
    }

    require_status("srtp_shutdown", srtp_shutdown());
    return EXIT_SUCCESS;
}
