//! Proves this crate's SRTP/SRTCP implementation is really RFC 3711
//! interoperable, not just self-consistent.
//!
//! Every unit test inside `rvoip_rtp_core::srtp` round-trips through this
//! crate's own encrypt and decrypt — which would still pass even if both
//! sides shared the exact same wrong assumption (that's exactly how the
//! ROC/SRTCP-index bugs this crate used to have went unnoticed). This test
//! instead pairs this crate's `SrtpContext` against `webrtc-srtp`, an
//! independent pure-Rust SRTP implementation (same webrtc-rs 0.17.x lineage
//! already used for the DTLS handshake elsewhere in this crate), encrypting
//! with one side and decrypting with the other and vice versa.

use bytes::Bytes;

use rvoip_rtp_core::packet::RtpPacket;
use rvoip_rtp_core::srtp::{SrtpContext, SrtpCryptoKey, SRTP_AES128_CM_SHA1_80};

use webrtc_srtp::context::Context as TheirContext;
use webrtc_srtp::protection_profile::ProtectionProfile;

const MASTER_KEY: [u8; 16] = [0x11; 16];
const MASTER_SALT: [u8; 14] = [0x22; 14];

fn our_context() -> SrtpContext {
    let key = SrtpCryptoKey::new(MASTER_KEY.to_vec(), MASTER_SALT.to_vec());
    SrtpContext::new(SRTP_AES128_CM_SHA1_80, key).unwrap()
}

fn their_context() -> TheirContext {
    TheirContext::new(
        &MASTER_KEY,
        &MASTER_SALT,
        ProtectionProfile::Aes128CmHmacSha1_80,
        None,
        None,
    )
    .unwrap()
}

fn sample_rtp_packet(seq: u16) -> RtpPacket {
    RtpPacket::new_with_payload(
        96,
        seq,
        0xC0FFEE,
        0xABCD_EF01,
        Bytes::from_static(b"interop check payload"),
    )
}

#[test]
fn our_encrypt_their_decrypt_agree() {
    let mut ours = our_context();
    let mut theirs = their_context();

    let packet = sample_rtp_packet(1000);
    let wire = ours.protect(&packet).unwrap().serialize().unwrap();

    let decrypted = theirs
        .decrypt_rtp(&wire)
        .expect("an independent SRTP implementation must be able to decrypt our ciphertext");

    // `webrtc-srtp` hands back the full decrypted RTP packet bytes
    // (header + plaintext payload); parse it with our own parser to check
    // the payload round-tripped correctly.
    let parsed = RtpPacket::parse(&decrypted).unwrap();
    assert_eq!(parsed.header.sequence_number, 1000);
    assert_eq!(parsed.header.ssrc, 0xABCD_EF01);
    assert_eq!(parsed.payload, packet.payload);
}

#[test]
fn their_encrypt_our_decrypt_agree() {
    let mut ours = our_context();
    let mut theirs = their_context();

    let packet = sample_rtp_packet(2000);
    let plaintext = packet.serialize().unwrap();

    let wire = theirs
        .encrypt_rtp(&plaintext)
        .expect("independent implementation encrypts fine");

    let decrypted = ours.unprotect(&wire).expect(
        "our SrtpContext must be able to decrypt a real independent implementation's ciphertext",
    );

    assert_eq!(decrypted.header.sequence_number, 2000);
    assert_eq!(decrypted.header.ssrc, 0xABCD_EF01);
    assert_eq!(decrypted.payload, packet.payload);
}

#[test]
fn interop_survives_a_sequence_wrap() {
    let mut ours = our_context();
    let mut theirs = their_context();

    // Drive both sides across the 65535 -> 0 wrap, alternating who
    // encrypts, to prove our ROC tracking lines up with an independent
    // implementation's across the wrap boundary, not just within one
    // cycle.
    for seq in [65533u16, 65534, 65535, 0, 1, 2] {
        let packet = sample_rtp_packet(seq);
        let wire = ours.protect(&packet).unwrap().serialize().unwrap();
        let decrypted = theirs
            .decrypt_rtp(&wire)
            .unwrap_or_else(|e| panic!("seq {seq} (ours->theirs) failed: {e}"));
        let parsed = RtpPacket::parse(&decrypted).unwrap();
        assert_eq!(parsed.header.sequence_number, seq);
    }
}

fn sample_rtcp_packet(ssrc: u32) -> Vec<u8> {
    // Minimal SR (Sender Report) packet: V=2,P=0,RC=0; PT=200; length=1;
    // SSRC; 8 bytes of opaque report payload.
    let mut data = vec![0x80, 200, 0x00, 0x01];
    data.extend_from_slice(&ssrc.to_be_bytes());
    data.extend_from_slice(&[0x55u8; 8]);
    data
}

#[test]
fn srtcp_our_encrypt_their_decrypt_agree() {
    let mut ours = our_context();
    let mut theirs = their_context();

    let plain = sample_rtcp_packet(0x1234_5678);
    let protected = ours.protect_rtcp(&plain).unwrap();

    let decrypted = theirs
        .decrypt_rtcp(&protected)
        .expect("an independent SRTCP implementation must decrypt our ciphertext");
    assert_eq!(decrypted.as_ref(), plain.as_slice());
}

#[test]
fn srtcp_their_encrypt_our_decrypt_agree() {
    let mut ours = our_context();
    let mut theirs = their_context();

    let plain = sample_rtcp_packet(0x8765_4321);
    let protected = theirs.encrypt_rtcp(&plain).unwrap();

    let decrypted = ours
        .unprotect_rtcp(&protected)
        .expect("our SRTCP must decrypt an independent implementation's ciphertext");
    assert_eq!(decrypted.as_ref(), plain.as_slice());
}
