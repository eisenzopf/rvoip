//! Known-answer coverage from libSRTP's `srtp_validate` test.
//!
//! Source: Cisco libSRTP v2.8.0, commit
//! `24b3bf8f19b6f5ab4cd2bcceb4f4064efca86fd5`.

use rvoip_rtp_core::packet::RtpPacket;
use rvoip_rtp_core::srtp::{
    SrtpContext, SrtpCryptoKey, SrtpCryptoSuite, SRTP_AES128_CM_SHA1_32, SRTP_AES128_CM_SHA1_80,
};

const MASTER_KEY: [u8; 16] = [
    0xe1, 0xf9, 0x7a, 0x0d, 0x3e, 0x01, 0x8b, 0xe0, 0xd6, 0x4f, 0xa3, 0x2c, 0x06, 0xde, 0x41, 0x39,
];
const MASTER_SALT: [u8; 14] = [
    0x0e, 0xc6, 0x75, 0xad, 0x49, 0x8a, 0xfe, 0xeb, 0xb6, 0x96, 0x0b, 0x3a, 0xab, 0xe6,
];
const RTP_PLAINTEXT: [u8; 28] = [
    0x80, 0x0f, 0x12, 0x34, 0xde, 0xca, 0xfb, 0xad, 0xca, 0xfe, 0xba, 0xbe, 0xab, 0xab, 0xab, 0xab,
    0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab,
];
const SRTP_CIPHERTEXT: [u8; 38] = [
    0x80, 0x0f, 0x12, 0x34, 0xde, 0xca, 0xfb, 0xad, 0xca, 0xfe, 0xba, 0xbe, 0x4e, 0x55, 0xdc, 0x4c,
    0xe7, 0x99, 0x78, 0xd8, 0x8c, 0xa4, 0xd2, 0x15, 0x94, 0x9d, 0x24, 0x02, 0xb7, 0x8d, 0x6a, 0xcc,
    0x99, 0xea, 0x17, 0x9b, 0x8d, 0xbb,
];
const SRTP_SHA1_32_CIPHERTEXT: [u8; 32] = [
    0x80, 0x0f, 0x12, 0x34, 0xde, 0xca, 0xfb, 0xad, 0xca, 0xfe, 0xba, 0xbe, 0x4e, 0x55, 0xdc, 0x4c,
    0xe7, 0x99, 0x78, 0xd8, 0x8c, 0xa4, 0xd2, 0x15, 0x94, 0x9d, 0x24, 0x02, 0xb7, 0x8d, 0x6a, 0xcc,
];
const RTCP_PLAINTEXT: [u8; 24] = [
    0x81, 0xc8, 0x00, 0x0b, 0xca, 0xfe, 0xba, 0xbe, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab,
    0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab,
];
const SRTCP_CIPHERTEXT: [u8; 38] = [
    0x81, 0xc8, 0x00, 0x0b, 0xca, 0xfe, 0xba, 0xbe, 0x71, 0x28, 0x03, 0x5b, 0xe4, 0x87, 0xb9, 0xbd,
    0xbe, 0xf8, 0x90, 0x41, 0xf9, 0x77, 0xa5, 0xa8, 0x80, 0x00, 0x00, 0x01, 0x99, 0x3e, 0x08, 0xcd,
    0x54, 0xd6, 0xc1, 0x23, 0x07, 0x98,
];

fn context(suite: SrtpCryptoSuite) -> SrtpContext {
    SrtpContext::new(
        suite,
        SrtpCryptoKey::new(MASTER_KEY.to_vec(), MASTER_SALT.to_vec()),
    )
    .expect("libSRTP vector key must create a context")
}

#[test]
fn protects_rtp_to_libsrtp_known_answer() {
    let packet = RtpPacket::parse(&RTP_PLAINTEXT).expect("valid RTP vector");
    let wire = context(SRTP_AES128_CM_SHA1_80)
        .protect(&packet)
        .expect("protect RTP vector")
        .serialize()
        .expect("serialize protected RTP vector");
    assert_eq!(wire.as_ref(), SRTP_CIPHERTEXT);
}

#[test]
fn unprotects_libsrtp_srtp_known_answer() {
    let packet = context(SRTP_AES128_CM_SHA1_80)
        .unprotect(&SRTP_CIPHERTEXT)
        .expect("unprotect libSRTP vector");
    assert_eq!(packet.serialize().unwrap().as_ref(), RTP_PLAINTEXT);
}

#[test]
fn protects_rtcp_to_libsrtp_known_answer() {
    let wire = context(SRTP_AES128_CM_SHA1_80)
        .protect_rtcp(&RTCP_PLAINTEXT)
        .expect("protect RTCP vector");
    assert_eq!(wire.as_ref(), SRTCP_CIPHERTEXT);
}

#[test]
fn unprotects_libsrtp_srtcp_known_answer() {
    let packet = context(SRTP_AES128_CM_SHA1_80)
        .unprotect_rtcp(&SRTCP_CIPHERTEXT)
        .expect("unprotect libSRTP SRTCP vector");
    assert_eq!(packet.as_ref(), RTCP_PLAINTEXT);
}

#[test]
fn sha1_32_protects_rtp_to_libsrtp_known_answer() {
    let packet = RtpPacket::parse(&RTP_PLAINTEXT).expect("valid RTP vector");
    let wire = context(SRTP_AES128_CM_SHA1_32)
        .protect(&packet)
        .expect("protect SHA1-32 RTP vector")
        .serialize()
        .expect("serialize protected SHA1-32 RTP vector");
    assert_eq!(wire.as_ref(), SRTP_SHA1_32_CIPHERTEXT);
}

#[test]
fn sha1_32_unprotects_libsrtp_srtp_known_answer() {
    let packet = context(SRTP_AES128_CM_SHA1_32)
        .unprotect(&SRTP_SHA1_32_CIPHERTEXT)
        .expect("unprotect libSRTP SHA1-32 vector");
    assert_eq!(packet.serialize().unwrap().as_ref(), RTP_PLAINTEXT);
}

#[test]
fn sha1_32_protects_srtcp_with_the_libsrtp_80_bit_tag() {
    let wire = context(SRTP_AES128_CM_SHA1_32)
        .protect_rtcp(&RTCP_PLAINTEXT)
        .expect("protect SHA1-32 RTCP vector");
    assert_eq!(wire.as_ref(), SRTCP_CIPHERTEXT);
}

#[test]
fn sha1_32_unprotects_libsrtp_srtcp_known_answer() {
    let packet = context(SRTP_AES128_CM_SHA1_32)
        .unprotect_rtcp(&SRTCP_CIPHERTEXT)
        .expect("unprotect libSRTP SHA1-32 SRTCP vector");
    assert_eq!(packet.as_ref(), RTCP_PLAINTEXT);
}
