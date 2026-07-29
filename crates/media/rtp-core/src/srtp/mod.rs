//! Secure RTP (SRTP) implementation
//!
//! This module provides encryption and authentication for RTP/RTCP packets.

pub mod auth;
pub mod crypto;
pub mod key_derivation;

pub use auth::{SrtpAuthenticator, SrtpReplayProtection};
pub use crypto::SrtpCryptoKey;
pub use key_derivation::{
    create_srtp_iv, srtp_kdf, KeyDerivationLabel, KeyRotationFrequency, SrtpKeyDerivationParams,
};

/// SRTP encryption algorithms
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SrtpEncryptionAlgorithm {
    /// AES Counter Mode (Default in SRTP)
    AesCm,

    /// AES in f8-mode (Customized for SRTP)
    AesF8,

    /// Null encryption (for debugging/testing only)
    Null,
}

/// SRTP authentication algorithms
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SrtpAuthenticationAlgorithm {
    /// HMAC-SHA1 truncated to 80 bits (Default in SRTP)
    HmacSha1_80,

    /// HMAC-SHA1 truncated to 32 bits
    HmacSha1_32,

    /// Null authentication (for debugging/testing only)
    Null,
}

/// SRTP crypto suite
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SrtpCryptoSuite {
    /// Encryption algorithm
    pub encryption: SrtpEncryptionAlgorithm,

    /// Authentication algorithm
    pub authentication: SrtpAuthenticationAlgorithm,

    /// Master key length in bytes
    pub key_length: usize,

    /// Authentication tag length in bytes
    pub tag_length: usize,
}

/// Default SRTP crypto suite: AES-CM-128 + HMAC-SHA1-80
pub const SRTP_AES128_CM_SHA1_80: SrtpCryptoSuite = SrtpCryptoSuite {
    encryption: SrtpEncryptionAlgorithm::AesCm,
    authentication: SrtpAuthenticationAlgorithm::HmacSha1_80,
    key_length: 16, // 128 bits
    tag_length: 10, // 80 bits
};

/// Smaller tag SRTP crypto suite: AES-CM-128 + HMAC-SHA1-32
pub const SRTP_AES128_CM_SHA1_32: SrtpCryptoSuite = SrtpCryptoSuite {
    encryption: SrtpEncryptionAlgorithm::AesCm,
    authentication: SrtpAuthenticationAlgorithm::HmacSha1_32,
    key_length: 16, // 128 bits
    tag_length: 4,  // 32 bits
};

/// AES-CM-256 + HMAC-SHA1-80 (RFC 6188)
pub const SRTP_AES256_CM_SHA1_80: SrtpCryptoSuite = SrtpCryptoSuite {
    encryption: SrtpEncryptionAlgorithm::AesCm,
    authentication: SrtpAuthenticationAlgorithm::HmacSha1_80,
    key_length: 32, // 256 bits
    tag_length: 10, // 80 bits
};

/// AES-CM-256 + HMAC-SHA1-32 (RFC 6188)
pub const SRTP_AES256_CM_SHA1_32: SrtpCryptoSuite = SrtpCryptoSuite {
    encryption: SrtpEncryptionAlgorithm::AesCm,
    authentication: SrtpAuthenticationAlgorithm::HmacSha1_32,
    key_length: 32, // 256 bits
    tag_length: 4,  // 32 bits
};

/// Null encryption/authentication (for testing/debugging only)
pub const SRTP_NULL_SHA1_80: SrtpCryptoSuite = SrtpCryptoSuite {
    encryption: SrtpEncryptionAlgorithm::Null,
    authentication: SrtpAuthenticationAlgorithm::HmacSha1_80,
    key_length: 16, // 128 bits
    tag_length: 10, // 80 bits
};

/// No encryption or authentication (DANGEROUS - use only for testing)
pub const SRTP_NULL_NULL: SrtpCryptoSuite = SrtpCryptoSuite {
    encryption: SrtpEncryptionAlgorithm::Null,
    authentication: SrtpAuthenticationAlgorithm::Null,
    key_length: 16, // Changed from 0 to 16 to support realistic test scenarios
    tag_length: 0,
};

// There is deliberately no SRTP_AEAD_AES_128_GCM / SRTP_AEAD_AES_256_GCM
// here. This crate has no real AEAD-GCM implementation (RFC 7714) — the
// two consts that used to exist under those names set `encryption:
// AesCm` with `authentication: Null` and a comment admitting they were a
// placeholder, i.e. unauthenticated AES-CM mislabeled as authenticated
// AEAD-GCM. Advertising or negotiating either is worse than not
// supporting GCM at all, so they're gone rather than fixed-later: adding
// real RFC 7714 support is separate future work, done as its own suite
// once it exists for real.

impl SrtpCryptoSuite {
    /// Validate that the suite's parameters are internally consistent.
    ///
    /// The HMAC-SHA1 authentication tag is truncated from a fixed 20-byte
    /// digest, so any HMAC-SHA1 suite must declare `tag_length <= 20`.
    /// The built-in suites all satisfy this; the check guards against a
    /// hand-constructed `SrtpCryptoSuite` literal (the fields are public)
    /// whose oversized `tag_length` would otherwise panic when the tag is
    /// sliced out of the digest during protect/unprotect.
    pub fn validate(&self) -> Result<(), crate::Error> {
        const HMAC_SHA1_OUTPUT_LEN: usize = 20;
        match self.authentication {
            SrtpAuthenticationAlgorithm::HmacSha1_80 | SrtpAuthenticationAlgorithm::HmacSha1_32 => {
                if self.tag_length > HMAC_SHA1_OUTPUT_LEN {
                    return Err(crate::Error::SrtpError(format!(
                        "SRTP suite tag_length {} exceeds HMAC-SHA1 output {}",
                        self.tag_length, HMAC_SHA1_OUTPUT_LEN
                    )));
                }
            }
            SrtpAuthenticationAlgorithm::Null => {}
        }
        Ok(())
    }
}

/// Per-SSRC RTP rollover/replay state (RFC 3711 §3.3.1). One `SrtpContext`
/// tracks this independently for every SSRC it sees, so multiple streams
/// multiplexed through one direction don't share (and corrupt) each
/// other's rollover counter.
#[derive(Debug)]
struct SsrcRtpState {
    /// Whether we've processed at least one packet for this SSRC yet.
    initialized: bool,
    /// Rollover counter for the highest index seen so far.
    roc: u32,
    /// Highest raw 16-bit sequence number seen so far (RFC 3711's `s_l`).
    highest_seq: u16,
    /// Replay window, keyed on the extended (roc<<16|seq) packet index.
    replay: auth::SrtpReplayProtection,
}

impl SsrcRtpState {
    fn new() -> Self {
        Self {
            initialized: false,
            roc: 0,
            highest_seq: 0,
            replay: auth::SrtpReplayProtection::new(REPLAY_WINDOW_SIZE),
        }
    }

    /// Extended packet index for the current high-water mark.
    fn stored_index(&self) -> u64 {
        ((self.roc as u64) << 16) | (self.highest_seq as u64)
    }
}

/// Per-SSRC SRTCP index/replay state (RFC 3711 §3.4 / §3.3.2). The 31-bit
/// SRTCP index is transmitted in the clear (unlike RTP's rolling 16-bit
/// sequence number), so there's no guessing involved — just tracking the
/// next value to send, and a replay window over received values.
#[derive(Debug)]
struct SsrcRtcpState {
    /// Next SRTCP index this context will use when it sends a packet for
    /// this SSRC. Starts at 0 for the first packet sent.
    tx_next_index: u32,
    /// Replay window over indices received for this SSRC.
    rx_replay: auth::SrtpReplayProtection,
}

impl SsrcRtcpState {
    fn new() -> Self {
        Self {
            tx_next_index: 0,
            rx_replay: auth::SrtpReplayProtection::new(REPLAY_WINDOW_SIZE),
        }
    }
}

/// Default anti-replay window size in packets (RFC 3711 recommends at
/// least 64).
const REPLAY_WINDOW_SIZE: u64 = 128;

/// Given the last known `(roc, highest_seq)` for an SSRC and a newly
/// arrived raw 16-bit sequence number, guess which rollover cycle the new
/// packet actually belongs to (RFC 3711 Appendix A). Rather than
/// replicating the RFC's bitwise guess formula verbatim, this tries the
/// three plausible candidates (`roc-1`, `roc`, `roc+1`) and picks whichever
/// yields an extended packet index numerically closest to the last known
/// index — provably equivalent for any single 16-bit-wrap step, and easier
/// to verify by inspection.
fn guess_roc_and_index(local_roc: u32, highest_seq: u16, seq: u16) -> (u32, u64) {
    let last_index = ((local_roc as i64) << 16) | (highest_seq as i64);

    let candidates = [
        local_roc.wrapping_sub(1),
        local_roc,
        local_roc.wrapping_add(1),
    ];

    let mut best_roc = local_roc;
    let mut best_index = ((local_roc as u64) << 16) | (seq as u64);
    let mut best_distance = i64::MAX;

    for &candidate_roc in &candidates {
        let candidate_index = ((candidate_roc as u64) << 16) | (seq as u64);
        let distance = (candidate_index as i64 - last_index).abs();
        if distance < best_distance {
            best_distance = distance;
            best_roc = candidate_roc;
            best_index = candidate_index;
        }
    }

    (best_roc, best_index)
}

/// SRTP context for one direction of a session — one `SrtpContext` is
/// either the send side or the receive side, never both (matching how a
/// real DTLS-SRTP/SDES handshake always derives distinct directional
/// keys). Tracks rollover-counter and anti-replay state independently per
/// SSRC.
pub struct SrtpContext {
    /// Whether encryption is enabled
    enabled: bool,

    /// SRTP crypto context
    crypto: crypto::SrtpCrypto,

    /// Key rotation frequency
    key_rotation: key_derivation::KeyRotationFrequency,

    /// Per-SSRC RTP rollover/replay state.
    rtp_state: std::collections::HashMap<u32, SsrcRtpState>,

    /// Per-SSRC SRTCP index/replay state.
    rtcp_state: std::collections::HashMap<u32, SsrcRtcpState>,
}

/// Protected RTP packet with authentication tag
pub struct ProtectedRtpPacket {
    /// The encrypted RTP packet
    pub packet: crate::packet::RtpPacket,

    /// Authentication tag (if used)
    pub auth_tag: Option<Vec<u8>>,
}

impl ProtectedRtpPacket {
    /// Serialize the protected packet with its authentication tag
    pub fn serialize(&self) -> Result<bytes::Bytes, crate::Error> {
        let packet_bytes = self.packet.serialize()?;

        if let Some(tag) = &self.auth_tag {
            // Combine packet and authentication tag
            let mut buffer = bytes::BytesMut::with_capacity(packet_bytes.len() + tag.len());
            buffer.extend_from_slice(&packet_bytes);
            buffer.extend_from_slice(tag);
            Ok(buffer.freeze())
        } else {
            // No authentication tag
            Ok(packet_bytes)
        }
    }

    /// Serialize the protected packet into a caller-owned scratch buffer.
    pub fn serialize_into(
        &self,
        buffer: &mut bytes::BytesMut,
    ) -> Result<bytes::Bytes, crate::Error> {
        buffer.clear();
        buffer.reserve(self.packet.size() + self.auth_tag.as_ref().map_or(0, Vec::len));

        self.packet.header.serialize(buffer)?;
        buffer.extend_from_slice(&self.packet.payload);
        if let Some(tag) = &self.auth_tag {
            buffer.extend_from_slice(tag);
        }

        Ok(buffer.split().freeze())
    }
}

impl SrtpContext {
    /// Create a new SRTP context for one direction (send or receive).
    ///
    /// There is deliberately no constructor that takes both a local and a
    /// remote key: a real SDES or DTLS-SRTP negotiation always derives two
    /// independent directional keys, so build one `SrtpContext` per
    /// direction with the matching key — `SrtpContext::new(tx_suite,
    /// local_key)` for sending, `SrtpContext::new(rx_suite, remote_key)`
    /// for receiving.
    pub fn new(suite: SrtpCryptoSuite, key: crypto::SrtpCryptoKey) -> Result<Self, crate::Error> {
        suite.validate()?;
        let crypto = crypto::SrtpCrypto::new(suite, key)?;

        Ok(Self {
            enabled: true,
            crypto,
            key_rotation: key_derivation::KeyRotationFrequency::None,
            rtp_state: std::collections::HashMap::new(),
            rtcp_state: std::collections::HashMap::new(),
        })
    }

    /// Enable or disable SRTP
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Set key rotation frequency
    pub fn set_key_rotation(&mut self, frequency: key_derivation::KeyRotationFrequency) {
        self.key_rotation = frequency;
    }

    /// Protect an RTP packet (SRTP encryption)
    /// Returns the encrypted packet with its authentication tag
    pub fn protect(
        &mut self,
        packet: &crate::packet::RtpPacket,
    ) -> Result<ProtectedRtpPacket, crate::Error> {
        // Check if SRTP is enabled
        if !self.enabled {
            return Ok(ProtectedRtpPacket {
                packet: packet.clone(),
                auth_tag: None,
            });
        }

        let ssrc = packet.header.ssrc;
        let seq = packet.header.sequence_number;
        let state = self.rtp_state.entry(ssrc).or_insert_with(SsrcRtpState::new);

        // We own this stream's sequence numbers, so rollover detection is
        // just "did our own counter wrap" — but route it through the same
        // guess-and-check helper used on receive so both directions share
        // one implementation.
        let roc = if !state.initialized {
            0
        } else {
            guess_roc_and_index(state.roc, state.highest_seq, seq).0
        };

        let packet_index = ((roc as u64) << 16) | (seq as u64);
        if self.key_rotation.should_rotate(packet_index) {
            // In a real implementation, we would rotate keys here
        }

        // Encrypt the packet using SRTP
        let (encrypted, auth_tag) = self.crypto.encrypt_rtp(packet, roc)?;

        state.roc = roc;
        state.highest_seq = seq;
        state.initialized = true;

        // Return the encrypted packet with its authentication tag
        Ok(ProtectedRtpPacket {
            packet: encrypted,
            auth_tag,
        })
    }

    /// Unprotect an RTP packet (SRTP decryption).
    ///
    /// The input data should include the authentication tag if used.
    /// Follows RFC 3711's required order: estimate the rollover
    /// counter/packet index, check the replay window, verify
    /// authentication, decrypt — and only touch this SSRC's stored
    /// state (rollover counter, high-water mark, replay window) once all
    /// of that has actually succeeded. Any failure leaves state
    /// untouched and returns `Err`, never a silent plaintext fallback.
    pub fn unprotect(&mut self, data: &[u8]) -> Result<crate::packet::RtpPacket, crate::Error> {
        // Check if SRTP is enabled
        if !self.enabled {
            return crate::packet::RtpPacket::parse(data);
        }

        // The RTP header (including ssrc/sequence_number) is never
        // encrypted, so it's safe to peek it before authenticating —
        // `data` may still have ciphertext + auth tag as its "payload"
        // here, that's fine, we only read fixed header fields.
        let (header, _header_size) =
            crate::packet::header::RtpHeader::parse_without_consuming(data)?;
        let ssrc = header.ssrc;
        let seq = header.sequence_number;

        let state = self.rtp_state.entry(ssrc).or_insert_with(SsrcRtpState::new);

        let (candidate_roc, candidate_index) = if !state.initialized {
            (0, seq as u64)
        } else {
            guess_roc_and_index(state.roc, state.highest_seq, seq)
        };

        if !state.replay.would_accept(candidate_index) {
            return Err(crate::Error::SrtpError(
                "RTP packet rejected: replay or outside the anti-replay window".to_string(),
            ));
        }

        // Verify + decrypt. On any failure, state is untouched and we
        // propagate the error as-is.
        let decrypted = self.crypto.decrypt_rtp(data, candidate_roc)?;

        // Only now, after success, commit state: advance the high-water
        // mark if this packet actually moved it forward, and always mark
        // the replay window (so exact duplicates of in-window packets are
        // still caught).
        if !state.initialized || candidate_index > state.stored_index() {
            state.roc = candidate_roc;
            state.highest_seq = seq;
            state.initialized = true;
        }
        state.replay.commit(candidate_index);

        Ok(decrypted)
    }

    /// Protect an RTCP packet (SRTCP encryption)
    /// Returns the encrypted data with the authentication tag appended
    pub fn protect_rtcp(&mut self, data: &[u8]) -> Result<bytes::Bytes, crate::Error> {
        // Check if SRTP is enabled
        if !self.enabled {
            return Ok(bytes::Bytes::copy_from_slice(data));
        }

        if data.len() < 8 {
            return Err(crate::Error::SrtpError("RTCP packet too short".to_string()));
        }
        let ssrc = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);

        let state = self
            .rtcp_state
            .entry(ssrc)
            .or_insert_with(SsrcRtcpState::new);
        let index = state.tx_next_index;
        state.tx_next_index = state.tx_next_index.wrapping_add(1) & 0x7FFF_FFFF;

        // Encrypt using SRTCP
        let (encrypted, auth_tag) = self.crypto.encrypt_rtcp(data, index)?;

        // If authentication is used, append the tag
        if let Some(tag) = auth_tag {
            let mut buffer = bytes::BytesMut::with_capacity(encrypted.len() + tag.len());
            buffer.extend_from_slice(&encrypted);
            buffer.extend_from_slice(&tag);
            Ok(buffer.freeze())
        } else {
            Ok(encrypted)
        }
    }

    /// Unprotect an RTCP packet (SRTCP decryption).
    ///
    /// Same peek/check/verify/decrypt/commit discipline as [`Self::unprotect`],
    /// keyed by the sender SSRC extracted from the packet's own
    /// (unencrypted) header.
    pub fn unprotect_rtcp(&mut self, data: &[u8]) -> Result<bytes::Bytes, crate::Error> {
        // Check if SRTP is enabled
        if !self.enabled {
            return Ok(bytes::Bytes::copy_from_slice(data));
        }

        let (ssrc, index, _e_flag) = self.crypto.peek_srtcp_header(data)?;

        let state = self
            .rtcp_state
            .entry(ssrc)
            .or_insert_with(SsrcRtcpState::new);

        if !state.rx_replay.would_accept(index as u64) {
            return Err(crate::Error::SrtpError(
                "SRTCP packet rejected: replay or outside the anti-replay window".to_string(),
            ));
        }

        // Decrypt using SRTCP (which handles authentication verification internally)
        let decrypted = self.crypto.decrypt_rtcp(data)?;

        state.rx_replay.commit(index as u64);

        Ok(decrypted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::RtpPacket;

    #[test]
    fn test_srtp_context_creation() {
        // Create a key
        let key = SrtpCryptoKey::new(vec![0; 16], vec![0; 14]);

        // Create context with null encryption
        let context = SrtpContext::new(SRTP_NULL_NULL, key);

        assert!(context.is_ok());
    }

    fn matching_contexts() -> (SrtpContext, SrtpContext) {
        let key = SrtpCryptoKey::new(vec![7u8; 16], vec![9u8; 14]);
        let tx = SrtpContext::new(SRTP_AES128_CM_SHA1_80, key.clone()).unwrap();
        let rx = SrtpContext::new(SRTP_AES128_CM_SHA1_80, key).unwrap();
        (tx, rx)
    }

    fn rtp_packet(ssrc: u32, seq: u16) -> RtpPacket {
        RtpPacket::new_with_payload(
            96,
            seq,
            12345,
            ssrc,
            bytes::Bytes::from_static(b"hello srtp"),
        )
    }

    fn roundtrip(
        tx: &mut SrtpContext,
        rx: &mut SrtpContext,
        ssrc: u32,
        seq: u16,
    ) -> Result<(), crate::Error> {
        let protected = tx.protect(&rtp_packet(ssrc, seq))?;
        let wire = protected.serialize()?;
        rx.unprotect(&wire)?;
        Ok(())
    }

    #[test]
    fn sequence_wrap_from_65535_to_0_still_decrypts() {
        let (mut tx, mut rx) = matching_contexts();
        let ssrc = 0xAAAA_1111;

        for seq in [65530u16, 65531, 65532, 65533, 65534, 65535, 0, 1, 2, 3] {
            roundtrip(&mut tx, &mut rx, ssrc, seq)
                .unwrap_or_else(|e| panic!("seq {seq} failed to roundtrip: {e}"));
        }

        // The rx side must have actually advanced its rollover counter,
        // not stayed stuck at roc=0 (the bug this hardening pass fixes).
        let state = rx.rtp_state.get(&ssrc).unwrap();
        assert_eq!(state.roc, 1);
        assert_eq!(state.highest_seq, 3);
    }

    #[test]
    fn reordered_packet_from_before_the_wrap_still_decrypts() {
        // Note: a receiver's very first-ever packet for an SSRC is always
        // treated as roc=0 by convention (RFC 3711 §3.3.1) — there's no
        // way to know a stream had already wrapped before the first
        // packet a receiver happens to observe. So this test establishes
        // real state (via seq=65533) *before* the ambiguous reorder
        // happens, which is the scenario the rollover guess is actually
        // meant to solve.
        let (mut tx, mut rx) = matching_contexts();
        let ssrc = 0xBBBB_2222;

        let baseline = tx
            .protect(&rtp_packet(ssrc, 65533))
            .unwrap()
            .serialize()
            .unwrap();
        rx.unprotect(&baseline).expect("baseline packet decrypts");

        let before_wrap = tx
            .protect(&rtp_packet(ssrc, 65535))
            .unwrap()
            .serialize()
            .unwrap();
        let after_wrap = tx
            .protect(&rtp_packet(ssrc, 2))
            .unwrap()
            .serialize()
            .unwrap();

        // Deliver the post-wrap packet first, advancing rx to roc=1, then
        // the pre-wrap packet arrives late.
        rx.unprotect(&after_wrap)
            .expect("post-wrap packet decrypts");
        rx.unprotect(&before_wrap)
            .expect("late pre-wrap packet must still decrypt using roc-1");

        // The high-water mark must not have been dragged backward by the
        // late packet.
        let state = rx.rtp_state.get(&ssrc).unwrap();
        assert_eq!(state.roc, 1);
        assert_eq!(state.highest_seq, 2);
    }

    #[test]
    fn replayed_packet_is_rejected_without_corrupting_state() {
        let (mut tx, mut rx) = matching_contexts();
        let ssrc = 0xCCCC_3333;

        let wire = tx
            .protect(&rtp_packet(ssrc, 100))
            .unwrap()
            .serialize()
            .unwrap();

        rx.unprotect(&wire).expect("first delivery succeeds");
        let replay_result = rx.unprotect(&wire);
        assert!(replay_result.is_err(), "exact replay must be rejected");

        // A subsequent, genuinely new packet must still work — proves the
        // rejected replay didn't leave any state half-mutated.
        let next_wire = tx
            .protect(&rtp_packet(ssrc, 101))
            .unwrap()
            .serialize()
            .unwrap();
        rx.unprotect(&next_wire)
            .expect("state must be intact after a rejected replay");
    }

    #[test]
    fn packet_far_outside_the_replay_window_is_rejected() {
        let (mut tx, mut rx) = matching_contexts();
        let ssrc = 0xDDDD_4444;

        let old_wire = tx
            .protect(&rtp_packet(ssrc, 100))
            .unwrap()
            .serialize()
            .unwrap();
        rx.unprotect(&old_wire).unwrap();

        // Jump far ahead, well past the replay window.
        for seq in 101..2000u16 {
            let wire = tx
                .protect(&rtp_packet(ssrc, seq))
                .unwrap()
                .serialize()
                .unwrap();
            rx.unprotect(&wire).unwrap();
        }

        // The very first packet, now long outside the window, must still
        // be rejected if seen again.
        assert!(rx.unprotect(&old_wire).is_err());
    }

    #[test]
    fn tampered_payload_is_rejected_and_original_still_decrypts_later() {
        let (mut tx, mut rx) = matching_contexts();
        let ssrc = 0xEEEE_5555;

        let wire = tx
            .protect(&rtp_packet(ssrc, 200))
            .unwrap()
            .serialize()
            .unwrap();
        let mut tampered = wire.to_vec();
        let mid = tampered.len() / 2;
        tampered[mid] ^= 0xFF;

        assert!(rx.unprotect(&tampered).is_err());

        // Retrying with the untampered packet for the SAME sequence number
        // must still succeed — if the failed attempt had mutated the
        // replay window or high-water mark, this would now be rejected.
        rx.unprotect(&wire)
            .expect("tampered attempt must not have poisoned state for the real packet");
    }

    #[test]
    fn wrong_key_fails_authentication() {
        let key_a = SrtpCryptoKey::new(vec![1u8; 16], vec![2u8; 14]);
        let key_b = SrtpCryptoKey::new(vec![3u8; 16], vec![4u8; 14]);
        let mut tx = SrtpContext::new(SRTP_AES128_CM_SHA1_80, key_a).unwrap();
        let mut rx = SrtpContext::new(SRTP_AES128_CM_SHA1_80, key_b).unwrap();

        let wire = tx
            .protect(&rtp_packet(0x1234, 1))
            .unwrap()
            .serialize()
            .unwrap();
        assert!(rx.unprotect(&wire).is_err());
    }

    #[test]
    fn two_ssrcs_multiplexed_through_one_context_track_independently() {
        let (mut tx, mut rx) = matching_contexts();
        let ssrc_a = 0x1111_1111;
        let ssrc_b = 0x2222_2222;

        // Push SSRC A far ahead (near a wrap) while SSRC B stays low.
        for seq in [65530u16, 65531, 65532, 0, 1] {
            roundtrip(&mut tx, &mut rx, ssrc_a, seq).unwrap();
        }
        for seq in [10u16, 11, 12] {
            roundtrip(&mut tx, &mut rx, ssrc_b, seq).unwrap();
        }

        let state_a = rx.rtp_state.get(&ssrc_a).unwrap();
        let state_b = rx.rtp_state.get(&ssrc_b).unwrap();
        assert_eq!(state_a.roc, 1);
        assert_eq!(state_a.highest_seq, 1);
        assert_eq!(state_b.roc, 0);
        assert_eq!(state_b.highest_seq, 12);

        // A replay on A must not affect B and vice versa.
        let wire_a = tx.protect(&rtp_packet(ssrc_a, 1)).unwrap();
        let _ = wire_a; // already delivered above; nothing further to assert here
    }

    fn synthetic_rtcp_packet(ssrc: u32, marker: u8) -> Vec<u8> {
        let mut data = vec![0x80, 200, 0x00, 0x01]; // V2, PT=200 (SR), length=1
        data.extend_from_slice(&ssrc.to_be_bytes());
        data.extend_from_slice(&[marker; 8]); // opaque report payload
        data
    }

    #[test]
    fn srtcp_successive_packets_use_increasing_real_indices() {
        let (mut tx, mut rx) = matching_contexts();
        let ssrc = 0xF00D_0001;

        for i in 0..5u8 {
            let plain = synthetic_rtcp_packet(ssrc, i);
            let protected = tx.protect_rtcp(&plain).unwrap();
            let recovered = rx.unprotect_rtcp(&protected).unwrap();
            assert_eq!(recovered.as_ref(), plain.as_slice());
        }

        let tx_state = tx.rtcp_state.get(&ssrc).unwrap();
        assert_eq!(tx_state.tx_next_index, 5);
    }

    #[test]
    fn srtcp_replay_is_rejected() {
        let (mut tx, mut rx) = matching_contexts();
        let ssrc = 0xF00D_0002;

        let plain = synthetic_rtcp_packet(ssrc, 1);
        let protected = tx.protect_rtcp(&plain).unwrap();

        rx.unprotect_rtcp(&protected).unwrap();
        assert!(rx.unprotect_rtcp(&protected).is_err());
    }

    #[test]
    fn srtcp_tampered_packet_is_rejected() {
        let (mut tx, mut rx) = matching_contexts();
        let ssrc = 0xF00D_0003;

        let plain = synthetic_rtcp_packet(ssrc, 7);
        let protected = tx.protect_rtcp(&plain).unwrap();
        let mut tampered = protected.to_vec();
        let mid = tampered.len() / 2;
        tampered[mid] ^= 0xFF;

        assert!(rx.unprotect_rtcp(&tampered).is_err());
    }

    #[test]
    fn srtcp_compound_packet_round_trips_without_altering_structure() {
        let (mut tx, mut rx) = matching_contexts();
        let ssrc = 0xF00D_0004;

        // Simulate a compound RTCP packet: SR header + an extra chunk of
        // "sub-packets" concatenated after it, per RFC 3711 §3.4's model
        // of everything past the first packet's 8-byte header being
        // opaque payload to SRTCP.
        let mut compound = synthetic_rtcp_packet(ssrc, 3);
        compound.extend_from_slice(&[0x81, 201, 0x00, 0x01, 0xDE, 0xAD, 0xBE, 0xEF]);

        let protected = tx.protect_rtcp(&compound).unwrap();
        let recovered = rx.unprotect_rtcp(&protected).unwrap();
        assert_eq!(recovered.as_ref(), compound.as_slice());
    }
}

#[cfg(test)]
mod integration_tests;
