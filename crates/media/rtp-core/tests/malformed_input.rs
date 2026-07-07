//! Malformed / truncated input regression guards for the rtp-core wire parsers.
//!
//! Each test asserts a public parse entry point returns `Err` (never panics) on
//! adversarial input. Several were surfaced by the `crates/media/fuzz` targets —
//! `rtcp_xr_statistics_summary_truncated_is_err` is the exact byte string the
//! `rtcp_packet` fuzzer used to crash `parse_statistics_summary_block` before its
//! mandatory-field length guard was corrected (16 -> 18 bytes).

use rvoip_rtp_core::srtp::{
    SrtpAuthenticationAlgorithm, SrtpContext, SrtpCryptoKey, SrtpCryptoSuite,
    SrtpEncryptionAlgorithm,
};
use rvoip_rtp_core::{RtcpPacket, RtpPacket};

#[test]
fn rtp_truncated_headers_are_err() {
    // The RTP fixed header is 12 bytes; anything shorter must be rejected.
    for len in 0..12usize {
        let buf = vec![0x80u8; len];
        assert!(
            RtpPacket::parse(&buf).is_err(),
            "expected Err for {len}-byte RTP packet"
        );
    }
}

#[test]
fn rtp_overclaimed_csrc_count_is_err() {
    // First byte 0x8F = version 2, CC = 15 -> the header claims 15 CSRC
    // identifiers (60 bytes) that are absent from a bare 12-byte buffer.
    let mut buf = vec![0u8; 12];
    buf[0] = 0x8F;
    assert!(RtpPacket::parse(&buf).is_err());
}

#[test]
fn rtcp_truncated_headers_are_err() {
    // The RTCP common header is 4 bytes.
    for len in 0..4usize {
        let buf = vec![0x80u8; len];
        assert!(RtcpPacket::parse(&buf).is_err());
    }
}

#[test]
fn rtcp_xr_statistics_summary_truncated_is_err() {
    // Exact input the fuzzer used to panic parse_statistics_summary_block:
    // v2 + XR (PT 207) with a StatisticsSummary block whose body is short of the
    // 18 mandatory fixed bytes. Must be rejected, not over-read.
    let input: &[u8] = &[
        0x89, 0xcf, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0x06, 0x00, 0x00, 0x00, 0x00, 0x09, 0x00,
        0x00, 0x89, 0xff, 0x06, 0x00, 0x00, 0x00, 0x00, 0x09, 0xcc, 0xbd, 0x00, 0xb6,
    ];
    assert!(RtcpPacket::parse(input).is_err());
}

#[test]
fn rtcp_xr_voip_metrics_truncated_is_err() {
    // v2 | XR(207) | len=0 ; XR ssrc ; block type 7 (VoIP metrics), block_len=0 ;
    // only 8 body bytes follow (< the 32 mandatory VoIP-metrics bytes).
    let input: &[u8] = &[
        0x80, 0xcf, 0x00, 0x00, // common header (length words-1 = 0)
        0x00, 0x00, 0x00, 0x00, // XR sender SSRC
        0x07, 0x00, 0x00, 0x00, // block header: type = 7, reserved, block_length = 0
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // 8 body bytes (< 32)
    ];
    assert!(RtcpPacket::parse(input).is_err());
}

#[test]
fn rtcp_sender_report_overclaimed_report_count_is_err() {
    // 0x9F = v2, RC = 31 ; PT 200 = SR ; a full 24-byte SR base is present but
    // zero of the 31 claimed report blocks follow -> the report-block loop must
    // return Err, not panic.
    let mut input = vec![0x9F, 0xC8, 0x00, 0x00];
    input.extend_from_slice(&[0u8; 24]); // SR base: ssrc + sender info
    assert!(RtcpPacket::parse(&input).is_err());
}

#[test]
fn srtp_suite_with_oversized_tag_length_is_rejected() {
    // A hand-built HMAC-SHA1 suite whose tag_length exceeds the 20-byte digest
    // would panic when the tag is sliced out; SrtpContext::new must reject it.
    let bad = SrtpCryptoSuite {
        encryption: SrtpEncryptionAlgorithm::AesCm,
        authentication: SrtpAuthenticationAlgorithm::HmacSha1_80,
        key_length: 16,
        tag_length: 99,
    };
    let key = SrtpCryptoKey::new(vec![0u8; 16], vec![0u8; 14]);
    assert!(SrtpContext::new(bad, key).is_err());
}
