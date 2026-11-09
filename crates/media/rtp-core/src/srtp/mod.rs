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

    /// AEAD AES-128-GCM (RFC 7714).
    ///
    /// The profile identity is retained so configuration and negotiation can
    /// fail closed. Encryption is not implemented in this release.
    AeadAes128Gcm,

    /// AEAD AES-256-GCM (RFC 7714).
    ///
    /// The profile identity is retained so configuration and negotiation can
    /// fail closed. Encryption is not implemented in this release.
    AeadAes256Gcm,
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

    /// SRTP authentication tag length in bytes.
    ///
    /// SRTCP can require a different length for the same profile; use
    /// [`Self::srtcp_tag_length`] for RTCP packets.
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

/// AEAD AES-128 GCM
pub const SRTP_AEAD_AES_128_GCM: SrtpCryptoSuite = SrtpCryptoSuite {
    encryption: SrtpEncryptionAlgorithm::AeadAes128Gcm,
    authentication: SrtpAuthenticationAlgorithm::Null, // Authentication is part of AEAD
    key_length: 16,                                    // 128 bits
    tag_length: 16,                                    // 128 bits for GCM
};

/// AEAD AES-256 GCM
pub const SRTP_AEAD_AES_256_GCM: SrtpCryptoSuite = SrtpCryptoSuite {
    encryption: SrtpEncryptionAlgorithm::AeadAes256Gcm,
    authentication: SrtpAuthenticationAlgorithm::Null, // Authentication is part of AEAD
    key_length: 32,                                    // 256 bits
    tag_length: 16,                                    // 128 bits for GCM
};

impl SrtpCryptoSuite {
    /// Authentication tag length used by SRTCP.
    ///
    /// The RFC 5764 `AES_CM_128_HMAC_SHA1_32` profile shortens only the SRTP
    /// tag. SRTCP continues to use the 80-bit HMAC-SHA1 tag required by its
    /// crypto policy. Keeping this derived from the public suite fields avoids
    /// changing the construction API for downstream users.
    #[must_use]
    pub const fn srtcp_tag_length(&self) -> usize {
        match self.authentication {
            SrtpAuthenticationAlgorithm::HmacSha1_80 | SrtpAuthenticationAlgorithm::HmacSha1_32 => {
                10
            }
            SrtpAuthenticationAlgorithm::Null => 0,
        }
    }

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
        if matches!(
            self.encryption,
            SrtpEncryptionAlgorithm::AeadAes128Gcm | SrtpEncryptionAlgorithm::AeadAes256Gcm
        ) {
            return Err(crate::Error::UnsupportedFeature(
                "SRTP AEAD AES-GCM profiles are not implemented".to_string(),
            ));
        }

        if self.encryption == SrtpEncryptionAlgorithm::AesF8 {
            return Err(crate::Error::UnsupportedFeature(
                "SRTP AES-f8 is not implemented".to_string(),
            ));
        }

        if !matches!(self.key_length, 16 | 32) {
            return Err(crate::Error::SrtpError(format!(
                "unsupported SRTP master key length: {} bytes",
                self.key_length
            )));
        }

        let expected_tag_length = match self.authentication {
            SrtpAuthenticationAlgorithm::HmacSha1_80 => 10,
            SrtpAuthenticationAlgorithm::HmacSha1_32 => 4,
            SrtpAuthenticationAlgorithm::Null => 0,
        };
        if self.tag_length != expected_tag_length {
            return Err(crate::Error::SrtpError(format!(
                "SRTP authentication mode requires a {expected_tag_length}-byte tag, got {}",
                self.tag_length
            )));
        }
        if self.tag_length > HMAC_SHA1_OUTPUT_LEN {
            return Err(crate::Error::SrtpError(format!(
                "SRTP suite tag_length {} exceeds HMAC-SHA1 output {}",
                self.tag_length, HMAC_SHA1_OUTPUT_LEN
            )));
        }

        if self != &SRTP_AES128_CM_SHA1_80
            && self != &SRTP_AES128_CM_SHA1_32
            && self != &SRTP_AES256_CM_SHA1_80
            && self != &SRTP_AES256_CM_SHA1_32
        {
            return Err(crate::Error::UnsupportedFeature(
                "unreviewed SRTP suite combination is unavailable; use an exact AES-CM/HMAC-SHA1 built-in profile"
                    .to_string(),
            ));
        }
        Ok(())
    }
}

/// SRTP context for a session
pub struct SrtpContext {
    /// Whether encryption is enabled
    enabled: bool,

    /// Local transmit keys and derived material.
    outbound_crypto: crypto::SrtpCrypto,

    /// Remote transmit keys and derived material used for receive.
    inbound_crypto: crypto::SrtpCrypto,

    /// Key rotation frequency
    key_rotation: key_derivation::KeyRotationFrequency,

    /// Per-SSRC outbound RTP rollover and IV-reuse state.
    outbound_rtp: crate::ssrc_hash::SsrcHashMap<RtpStreamState>,

    /// Per-SSRC inbound RTP rollover and replay state.
    inbound_rtp: crate::ssrc_hash::SsrcHashMap<RtpStreamState>,

    /// Per-SSRC outbound SRTCP indexes.
    outbound_srtcp: crate::ssrc_hash::SsrcHashMap<SrtcpSendState>,

    /// Per-SSRC inbound SRTCP replay windows.
    inbound_srtcp: crate::ssrc_hash::SsrcHashMap<ReplayWindow>,

    /// Hot call state ([`MediaExecutorConfig::hot_state`]): the per-packet
    /// state and key material of both RTP directions, packed (see
    /// [`SrtpHot`]). `None` (the default) keeps every packet on the maps and
    /// session keys above.
    ///
    /// [`MediaExecutorConfig::hot_state`]: crate::executor::MediaExecutorConfig::hot_state
    hot: Option<Box<SrtpHot>>,
}

/// Hot call state: one RTP direction's per-packet SRTP state, packed into
/// three cache lines instead of the eight or nine the session-key path
/// touches (the context's suite and session-key headers, the heap key, salt
/// and authentication key `Vec`s, the HMAC key schedule run per packet, and
/// the per-SSRC map's header, control bytes and bucket).
///
/// - Line 0 ([`HotStream`]): the active SSRC's rollover/replay state, held
///   directly instead of in the per-SSRC map (any other SSRC still lives in
///   the map, and each SSRC's state is in exactly one of the two places),
///   plus the salt, the key and tag lengths and the authentication
///   algorithm.
/// - Line 1: the AES key.
/// - Line 2: the HMAC-SHA1 inner and outer states keyed once with the
///   authentication key (40 bytes).
///
/// The key material is a copy of the immutable session keys, so the
/// keystream, tags, replay and rollover decisions and every error are those
/// of the session-key path; the tests compare them byte for byte.
#[repr(C)]
pub(crate) struct SrtpDirectionHot {
    stream: crate::hot_state::CacheAligned<HotStream>,
    key: crate::hot_state::CacheAligned<[u8; 32]>,
    mac: crate::hot_state::CacheAligned<auth::HmacSha1Core>,
}

#[derive(Clone)]
struct HotStream {
    /// Whether `ssrc`/`state` hold a stream (otherwise every SSRC is in the
    /// map).
    occupied: bool,
    key_len: u8,
    tag_len: u8,
    auth: SrtpAuthenticationAlgorithm,
    ssrc: u32,
    state: RtpStreamState,
    salt: [u8; 14],
}

/// Hot call state: both directions (a context protects with one and
/// unprotects with the other; only the one in use is touched).
pub(crate) struct SrtpHot {
    tx: SrtpDirectionHot,
    rx: SrtpDirectionHot,
}

impl SrtpDirectionHot {
    fn build(crypto: &crypto::SrtpCrypto) -> Option<Self> {
        let (key, salt, mac, tag_len) = crypto.rtp_hot_material()?;
        if key.len() > 32 || salt.len() != 14 || tag_len > 20 {
            return None;
        }
        let mac = mac.clone();
        let mut key_line = [0u8; 32];
        key_line[..key.len()].copy_from_slice(key);
        let mut salt_copy = [0u8; 14];
        salt_copy.copy_from_slice(salt);
        Some(Self {
            stream: crate::hot_state::CacheAligned(HotStream {
                occupied: false,
                key_len: key.len() as u8,
                tag_len: tag_len as u8,
                auth: crypto.suite().authentication,
                ssrc: 0,
                state: RtpStreamState::default(),
                salt: salt_copy,
            }),
            key: crate::hot_state::CacheAligned(key_line),
            mac: crate::hot_state::CacheAligned(mac),
        })
    }

    #[inline]
    fn key(&self) -> &[u8] {
        &self.key.0[..usize::from(self.stream.key_len)]
    }

    /// The state of `ssrc` if it is the one held directly.
    #[inline]
    fn state(&self, ssrc: u32) -> Option<&RtpStreamState> {
        (self.stream.occupied && self.stream.ssrc == ssrc).then_some(&self.stream.state)
    }

    /// `entry(ssrc).or_default()` over "held directly, else the map": the
    /// stream held so far (if another) moves to the map and `ssrc` moves in.
    fn state_for<'a>(
        &'a mut self,
        ssrc: u32,
        map: &mut crate::ssrc_hash::SsrcHashMap<RtpStreamState>,
    ) -> &'a mut RtpStreamState {
        let stream = &mut self.stream.0;
        if !(stream.occupied && stream.ssrc == ssrc) {
            if stream.occupied {
                map.insert(stream.ssrc, std::mem::take(&mut stream.state));
            }
            stream.state = map.remove(&ssrc).unwrap_or_default();
            stream.ssrc = ssrc;
            stream.occupied = true;
        }
        &mut stream.state
    }

    /// Give the directly held stream back to the map.
    fn spill_into(&mut self, map: &mut crate::ssrc_hash::SsrcHashMap<RtpStreamState>) {
        let stream = &mut self.stream.0;
        if stream.occupied {
            map.insert(stream.ssrc, std::mem::take(&mut stream.state));
            stream.occupied = false;
        }
    }

    /// `SrtpCrypto::decrypt_rtp_in_place_with_roc` for AES-CM/HMAC-SHA1
    /// with the packed key material: the same checks in the same order, the
    /// same errors, the same bounds.
    fn decrypt_in_place(
        &self,
        data: &mut [u8],
        roc: u32,
    ) -> Result<crate::packet::rtp::RtpPacketBounds, crate::Error> {
        let tag_len = usize::from(self.stream.tag_len);
        if data.len() < tag_len {
            return Err(crate::Error::SrtpError(
                "Packet too short to contain authentication tag".to_string(),
            ));
        }
        let tag_start = data.len() - tag_len;
        let (packet_data, auth_tag) = data.split_at_mut(tag_start);
        let is_valid = auth::SrtpAuthenticator::verify_auth_tag_in_place_with(
            self.stream.auth,
            auth::HmacSha1Key::Keyed(&self.mac),
            tag_len,
            packet_data,
            auth_tag,
            roc,
        )?;
        if !is_valid {
            return Err(crate::Error::AuthenticationFailed(
                "SRTP authentication tag mismatch".to_string(),
            ));
        }
        let (header, header_size) = crate::packet::RtpHeader::parse_without_consuming(packet_data)?;
        let packet_index = (u64::from(roc) << 16) | u64::from(header.sequence_number);
        let iv =
            key_derivation::create_srtp_iv_array(&self.stream.salt, header.ssrc, packet_index)?;
        let encrypted = &mut packet_data[header_size..];
        crypto::aes_cm_encrypt(encrypted, self.key(), &iv)?;
        crypto::rtp_bounds_after_decrypt(header, header_size, encrypted)
    }

    /// `SrtpCrypto::encrypt_rtp_in_place_with_roc` for AES-CM/HMAC-SHA1
    /// with the packed key material: the payload after `header_size` is
    /// encrypted in place and the truncated tag over `buffer[start..]` and
    /// the ROC appended.
    fn encrypt_in_place(
        &self,
        buffer: &mut Vec<u8>,
        start: usize,
        header_size: usize,
        ssrc: u32,
        sequence: u16,
        roc: u32,
    ) -> Result<(), crate::Error> {
        let packet_index = (u64::from(roc) << 16) | u64::from(sequence);
        let iv = key_derivation::create_srtp_iv_array(&self.stream.salt, ssrc, packet_index)?;
        crypto::aes_cm_encrypt(&mut buffer[start + header_size..], self.key(), &iv)?;
        auth::SrtpAuthenticator::append_auth_tag_with(
            self.stream.auth,
            auth::HmacSha1Key::Keyed(&self.mac),
            usize::from(self.stream.tag_len),
            buffer,
            start,
            roc,
        )
    }
}

impl SrtpHot {
    /// Both directions, or `None` when either has no AES-CM/HMAC-SHA1 keys.
    fn build(outbound: &crypto::SrtpCrypto, inbound: &crypto::SrtpCrypto) -> Option<Box<Self>> {
        Some(Box::new(Self {
            tx: SrtpDirectionHot::build(outbound)?,
            rx: SrtpDirectionHot::build(inbound)?,
        }))
    }
}

#[derive(Debug, Clone, Default)]
struct RtpStreamState {
    roc: u32,
    highest_seq: Option<u16>,
    replay: ReplayWindow,
}

#[derive(Debug, Clone)]
struct SrtcpSendState {
    next_index: u32,
    exhausted: bool,
}

impl Default for SrtcpSendState {
    fn default() -> Self {
        // libSRTP and its published interoperability vectors place index 1
        // on the first protected RTCP packet.
        Self {
            next_index: 1,
            exhausted: false,
        }
    }
}

/// A fixed 64-packet replay window. Querying it is side-effect free so an
/// authentication failure can never advance receive state.
#[derive(Debug, Clone, Default)]
struct ReplayWindow {
    highest: Option<u64>,
    bitmap: u64,
}

impl ReplayWindow {
    fn accepts(&self, index: u64) -> bool {
        let Some(highest) = self.highest else {
            return true;
        };
        if index > highest {
            return true;
        }
        let delta = highest - index;
        delta < 64 && (self.bitmap & (1_u64 << delta)) == 0
    }

    fn commit(&mut self, index: u64) {
        match self.highest {
            None => {
                self.highest = Some(index);
                self.bitmap = 1;
            }
            Some(highest) if index > highest => {
                let shift = index - highest;
                self.bitmap = if shift >= 64 {
                    1
                } else {
                    (self.bitmap << shift) | 1
                };
                self.highest = Some(index);
            }
            Some(highest) => {
                let delta = highest - index;
                debug_assert!(delta < 64);
                self.bitmap |= 1_u64 << delta;
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RtpIndexCandidate {
    roc: u32,
    index: u64,
    before_roc_zero: bool,
}

fn candidate_rtp_index(state: Option<&RtpStreamState>, seq: u16) -> RtpIndexCandidate {
    let Some(state) = state else {
        return RtpIndexCandidate {
            roc: 0,
            index: u64::from(seq),
            before_roc_zero: false,
        };
    };
    let Some(highest_seq) = state.highest_seq else {
        return RtpIndexCandidate {
            roc: state.roc,
            index: (u64::from(state.roc) << 16) | u64::from(seq),
            before_roc_zero: false,
        };
    };

    // RFC 3711 Appendix A.1 index estimation. A large apparent jump across
    // the half-range belongs to the adjacent rollover cycle.
    let (candidate_roc, before_roc_zero) = if highest_seq < 0x8000 {
        if seq > highest_seq && seq - highest_seq > 0x8000 {
            // RFC 3711 performs this subtraction modulo 2^32. At ROC zero
            // that wrapped value is still required for authentication, but
            // it cannot represent a packet in the receiver's valid history.
            (state.roc.wrapping_sub(1), state.roc == 0)
        } else {
            (state.roc, false)
        }
    } else if highest_seq > seq && highest_seq - seq > 0x8000 {
        (state.roc.wrapping_add(1), false)
    } else {
        (state.roc, false)
    };

    RtpIndexCandidate {
        roc: candidate_roc,
        index: (u64::from(candidate_roc) << 16) | u64::from(seq),
        before_roc_zero,
    }
}

fn commit_rtp_index(state: &mut RtpStreamState, roc: u32, seq: u16, index: u64) {
    if state.replay.highest.is_none_or(|highest| index > highest) {
        state.roc = roc;
        state.highest_seq = Some(seq);
    }
    state.replay.commit(index);
}

/// Validate every plaintext RTCP member before SRTCP state can change.
///
/// This intentionally accepts both RFC 3550 compound packets and RFC 5506
/// reduced-size RTCP. It still validates each common header, declared length,
/// version, packet-specific body, and the rule that padding may appear only
/// on the final member.
fn validate_plaintext_rtcp(data: &[u8]) -> Result<(), crate::Error> {
    if data.is_empty() {
        return Err(crate::Error::RtcpError(
            "RTCP packet must not be empty".to_string(),
        ));
    }

    let mut offset = 0usize;
    while offset < data.len() {
        let remaining = data.len() - offset;
        if remaining < 4 {
            return Err(crate::Error::BufferTooSmall {
                required: 4,
                available: remaining,
            });
        }

        let packet = &data[offset..];
        let words_minus_one = usize::from(u16::from_be_bytes([packet[2], packet[3]]));
        let packet_size = words_minus_one
            .checked_add(1)
            .and_then(|words| words.checked_mul(4))
            .ok_or_else(|| {
                crate::Error::RtcpError("RTCP packet length overflows usize".to_string())
            })?;
        if offset == 0 && packet_size < 8 {
            return Err(crate::Error::RtcpError(
                "SRTCP requires the first RTCP member to carry an SSRC".to_string(),
            ));
        }
        if packet_size > remaining {
            return Err(crate::Error::BufferTooSmall {
                required: packet_size,
                available: remaining,
            });
        }
        if packet[0] & 0x20 != 0 && packet_size != remaining {
            return Err(crate::Error::RtcpError(
                "Only the final packet in an RTCP datagram may be padded".to_string(),
            ));
        }

        if crate::packet::rtcp::RtcpPacketType::try_from(packet[1]).is_ok() {
            crate::packet::rtcp::RtcpPacket::parse(&packet[..packet_size])?;
        } else {
            crate::packet::rtcp::RtcpUnknownPacket::parse(&packet[..packet_size])?;
        }
        offset += packet_size;
    }

    Ok(())
}

/// Executor v2: one outbound packet protected by
/// [`SrtpContext::protect_rtp_into`] whose rollover/replay state is not yet
/// committed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct OutboundRtpCommit {
    ssrc: u32,
    roc: u32,
    sequence: u16,
    index: u64,
}

/// Size of the header [`write_minimal_rtp_header`] writes.
pub(crate) const MINIMAL_RTP_HEADER_SIZE: usize = 12;

/// Append the 12-byte header `RtpHeader::new(payload_type, sequence,
/// timestamp, ssrc)` with `marker` serialises to: version 2, no padding, no
/// extension, no CSRCs.
pub(crate) fn write_minimal_rtp_header(
    out: &mut Vec<u8>,
    payload_type: u8,
    marker: bool,
    sequence: u16,
    timestamp: u32,
    ssrc: u32,
) {
    out.push(0x80);
    out.push((u8::from(marker) << 7) | (payload_type & 0x7F));
    out.extend_from_slice(&sequence.to_be_bytes());
    out.extend_from_slice(&timestamp.to_be_bytes());
    out.extend_from_slice(&ssrc.to_be_bytes());
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
        self.serialize_into(&mut bytes::BytesMut::new())
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
        if !self.packet.header.padding && self.packet.padding_size != 0 {
            return Err(crate::Error::InvalidParameter(format!(
                "RTP padding flag ({}) does not match padding length ({})",
                self.packet.header.padding, self.packet.padding_size
            )));
        }
        if self.packet.header.padding
            && self.packet.padding_size == 0
            && self.packet.payload.is_empty()
        {
            return Err(crate::Error::InvalidParameter(
                "protected RTP padding flag is set but the encrypted payload is empty".to_string(),
            ));
        }
        if self.packet.padding_size != 0 {
            for _ in 1..self.packet.padding_size {
                buffer.extend_from_slice(&[0]);
            }
            buffer.extend_from_slice(&[self.packet.padding_size]);
        }
        if let Some(tag) = &self.auth_tag {
            buffer.extend_from_slice(tag);
        }

        Ok(buffer.split().freeze())
    }
}

impl SrtpContext {
    /// Create a new SRTP context
    pub fn new(suite: SrtpCryptoSuite, key: crypto::SrtpCryptoKey) -> Result<Self, crate::Error> {
        suite.validate()?;
        let outbound_crypto = crypto::SrtpCrypto::new(suite.clone(), key.clone())?;
        let inbound_crypto = crypto::SrtpCrypto::new(suite, key)?;

        Ok(Self {
            enabled: true,
            outbound_crypto,
            inbound_crypto,
            key_rotation: key_derivation::KeyRotationFrequency::None,
            outbound_rtp: crate::ssrc_hash::ssrc_hash_map(),
            inbound_rtp: crate::ssrc_hash::ssrc_hash_map(),
            outbound_srtcp: crate::ssrc_hash::ssrc_hash_map(),
            inbound_srtcp: crate::ssrc_hash::ssrc_hash_map(),
            hot: None,
        }
        .with_configured_hot_state())
    }

    /// Create an SRTP context from directional key objects.
    ///
    /// `local` protects outbound traffic and `remote` unprotects inbound
    /// traffic. This is the preferred constructor for offer/answer exchanges
    /// that advertise independent transmit keys.
    pub fn new_directional(
        suite: SrtpCryptoSuite,
        local: crypto::SrtpCryptoKey,
        remote: crypto::SrtpCryptoKey,
    ) -> Result<Self, crate::Error> {
        Self::new_from_keys(
            local.key().to_vec(),
            remote.key().to_vec(),
            local.salt().to_vec(),
            remote.salt().to_vec(),
            suite,
        )
    }

    /// Create a new SRTP context from separate local and remote keys
    pub fn new_from_keys(
        local_key: Vec<u8>,
        remote_key: Vec<u8>,
        local_salt: Vec<u8>,
        remote_salt: Vec<u8>,
        profile: SrtpCryptoSuite,
    ) -> Result<Self, crate::Error> {
        profile.validate()?;
        let outbound_crypto = crypto::SrtpCrypto::new(
            profile.clone(),
            crypto::SrtpCryptoKey::new(local_key, local_salt),
        )?;
        let inbound_crypto =
            crypto::SrtpCrypto::new(profile, crypto::SrtpCryptoKey::new(remote_key, remote_salt))?;

        Ok(Self {
            enabled: true,
            outbound_crypto,
            inbound_crypto,
            key_rotation: key_derivation::KeyRotationFrequency::None,
            outbound_rtp: crate::ssrc_hash::ssrc_hash_map(),
            inbound_rtp: crate::ssrc_hash::ssrc_hash_map(),
            outbound_srtcp: crate::ssrc_hash::ssrc_hash_map(),
            inbound_srtcp: crate::ssrc_hash::ssrc_hash_map(),
            hot: None,
        }
        .with_configured_hot_state())
    }

    /// Hot call state: pack the per-direction state when the media executor
    /// configuration in effect for this thread asks for it (decided per
    /// context, when it is built).
    fn with_configured_hot_state(mut self) -> Self {
        if crate::executor::hot_state_enabled() {
            self.set_hot_state(true);
        }
        self
    }

    /// Hot call state: pack (or unpack) this context's per-direction state.
    /// Safe at any point: unpacking gives the directly held streams back to
    /// the maps and packing starts with every stream in the maps, so each
    /// SSRC's state is in exactly one place throughout. Returns whether the
    /// context is packed now (contexts without AES-CM/HMAC-SHA1 keys never
    /// are).
    pub(crate) fn set_hot_state(&mut self, enabled: bool) -> bool {
        match (enabled, self.hot.take()) {
            (true, Some(hot)) => self.hot = Some(hot),
            (true, None) => {
                self.hot = SrtpHot::build(&self.outbound_crypto, &self.inbound_crypto);
                if self.hot.is_some() {
                    crate::hot_state::record_hot_srtp_context();
                }
            }
            (false, Some(mut hot)) => {
                hot.tx.spill_into(&mut self.outbound_rtp);
                hot.rx.spill_into(&mut self.inbound_rtp);
            }
            (false, None) => {}
        }
        self.hot.is_some()
    }

    /// Whether the per-direction state is packed (hot call state).
    #[cfg(test)]
    pub(crate) fn hot_state_packed(&self) -> bool {
        self.hot.is_some()
    }

    /// Test seam: no RTP rollover/replay state at all in one direction
    /// (`outbound`), held directly or mapped.
    #[cfg(test)]
    fn rtp_state_is_empty(&self, outbound: bool) -> bool {
        let hot = self.hot.as_deref();
        let (held, map) = if outbound {
            (
                hot.is_some_and(|hot| hot.tx.stream.occupied),
                &self.outbound_rtp,
            )
        } else {
            (
                hot.is_some_and(|hot| hot.rx.stream.occupied),
                &self.inbound_rtp,
            )
        };
        !held && map.is_empty()
    }

    /// Outbound rollover/replay state of `ssrc` (held directly or mapped).
    #[inline]
    fn outbound_state(&self, ssrc: u32) -> Option<&RtpStreamState> {
        if let Some(state) = self.hot.as_deref().and_then(|hot| hot.tx.state(ssrc)) {
            return Some(state);
        }
        self.outbound_rtp.get(&ssrc)
    }

    /// `outbound_rtp.entry(ssrc).or_default()`, packed or not.
    #[inline]
    fn outbound_state_mut(&mut self, ssrc: u32) -> &mut RtpStreamState {
        match self.hot.as_deref_mut() {
            Some(hot) => hot.tx.state_for(ssrc, &mut self.outbound_rtp),
            None => self.outbound_rtp.entry(ssrc).or_default(),
        }
    }

    /// Inbound rollover/replay state of `ssrc` (held directly or mapped).
    #[inline]
    fn inbound_state(&self, ssrc: u32) -> Option<&RtpStreamState> {
        if let Some(state) = self.hot.as_deref().and_then(|hot| hot.rx.state(ssrc)) {
            return Some(state);
        }
        self.inbound_rtp.get(&ssrc)
    }

    /// `inbound_rtp.entry(ssrc).or_default()`, packed or not.
    #[inline]
    fn inbound_state_mut(&mut self, ssrc: u32) -> &mut RtpStreamState {
        match self.hot.as_deref_mut() {
            Some(hot) => hot.rx.state_for(ssrc, &mut self.inbound_rtp),
            None => self.inbound_rtp.entry(ssrc).or_default(),
        }
    }

    /// Enable or disable SRTP
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Validate that this context may be installed in a secure transport.
    pub(crate) fn validate_for_secure_transport(&self) -> Result<(), crate::Error> {
        if !self.enabled {
            return Err(crate::Error::InvalidState(
                "disabled SRTP context cannot be installed as secure media".to_string(),
            ));
        }
        self.outbound_crypto.suite().validate()?;
        self.inbound_crypto.suite().validate()
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
            return Err(crate::Error::InvalidState(
                "disabled SRTP context cannot pass RTP through as plaintext".to_string(),
            ));
        }

        let ssrc = packet.header.ssrc;
        let seq = packet.header.sequence_number;
        let candidate = candidate_rtp_index(self.outbound_state(ssrc), seq);
        if candidate.before_roc_zero {
            return Err(crate::Error::SrtpError(
                "outbound SRTP sequence maps before ROC zero".to_string(),
            ));
        }
        let roc = candidate.roc;
        let packet_index = candidate.index;
        if self
            .outbound_state(ssrc)
            .is_some_and(|state| !state.replay.accepts(packet_index))
        {
            return Err(crate::Error::SrtpError(
                "refusing to reuse an outbound SRTP packet index".to_string(),
            ));
        }

        // Check for key rotation
        if self.key_rotation.should_rotate(packet_index) {
            return Err(crate::Error::UnsupportedFeature(
                "SRTP key rotation is not implemented; refusing to reuse the old key after the configured rotation boundary"
                    .to_string(),
            ));
        }

        // Encrypt the packet using SRTP
        let (encrypted, auth_tag) = self.outbound_crypto.encrypt_rtp_with_roc(packet, roc)?;
        let state = self.outbound_state_mut(ssrc);
        commit_rtp_index(state, roc, seq, packet_index);

        // Return the encrypted packet with its authentication tag
        Ok(ProtectedRtpPacket {
            packet: encrypted,
            auth_tag,
        })
    }

    /// Executor v2 twin of [`Self::protect`] + `serialize_into` for the
    /// synchronous TX path: appends the protected packet for a minimal,
    /// unpadded header (`RtpHeader::new` plus the marker) and `payload` to
    /// `out`, **without** committing the outbound rollover/replay state.
    ///
    /// `pending` are packets of the same batch protected earlier in this
    /// call and not committed yet; index estimation and the reuse check see
    /// them exactly as if they had been committed one by one. The caller
    /// commits (with [`Self::commit_outbound`]) only the packets the kernel
    /// accepted, so a packet that could not be sent leaves no trace and is
    /// protected again, with the same index, by whichever path sends it.
    /// Checks, their order and their errors are those of [`Self::protect`].
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn protect_rtp_into(
        &mut self,
        payload_type: u8,
        marker: bool,
        sequence: u16,
        timestamp: u32,
        ssrc: u32,
        payload: &[u8],
        pending: &[OutboundRtpCommit],
        out: &mut Vec<u8>,
    ) -> Result<OutboundRtpCommit, crate::Error> {
        if !self.enabled {
            return Err(crate::Error::InvalidState(
                "disabled SRTP context cannot pass RTP through as plaintext".to_string(),
            ));
        }
        let mut scratch = self.outbound_state(ssrc).cloned();
        for earlier in pending.iter().filter(|earlier| earlier.ssrc == ssrc) {
            commit_rtp_index(
                scratch.get_or_insert_with(RtpStreamState::default),
                earlier.roc,
                earlier.sequence,
                earlier.index,
            );
        }
        let candidate = candidate_rtp_index(scratch.as_ref(), sequence);
        if candidate.before_roc_zero {
            return Err(crate::Error::SrtpError(
                "outbound SRTP sequence maps before ROC zero".to_string(),
            ));
        }
        if scratch
            .as_ref()
            .is_some_and(|state| !state.replay.accepts(candidate.index))
        {
            return Err(crate::Error::SrtpError(
                "refusing to reuse an outbound SRTP packet index".to_string(),
            ));
        }
        if self.key_rotation.should_rotate(candidate.index) {
            return Err(crate::Error::UnsupportedFeature(
                "SRTP key rotation is not implemented; refusing to reuse the old key after the configured rotation boundary"
                    .to_string(),
            ));
        }
        let start = out.len();
        write_minimal_rtp_header(out, payload_type, marker, sequence, timestamp, ssrc);
        out.extend_from_slice(payload);
        let encrypted = match self.hot.as_deref() {
            // Hot call state: the packed key material (same keystream and
            // tag).
            Some(hot) => hot.tx.encrypt_in_place(
                out,
                start,
                MINIMAL_RTP_HEADER_SIZE,
                ssrc,
                sequence,
                candidate.roc,
            ),
            None => self.outbound_crypto.encrypt_rtp_in_place_with_roc(
                out,
                start,
                MINIMAL_RTP_HEADER_SIZE,
                ssrc,
                sequence,
                candidate.roc,
            ),
        };
        if let Err(error) = encrypted {
            out.truncate(start);
            return Err(error);
        }
        Ok(OutboundRtpCommit {
            ssrc,
            roc: candidate.roc,
            sequence,
            index: candidate.index,
        })
    }

    /// Commit packets protected by [`Self::protect_rtp_into`], in order.
    pub(crate) fn commit_outbound(&mut self, commits: &[OutboundRtpCommit]) {
        for commit in commits {
            let state = self.outbound_state_mut(commit.ssrc);
            commit_rtp_index(state, commit.roc, commit.sequence, commit.index);
        }
    }

    /// Unprotect an RTP packet (SRTP decryption)
    /// The input data should include the authentication tag if used
    pub fn unprotect(&mut self, data: &[u8]) -> Result<crate::packet::RtpPacket, crate::Error> {
        // Check if SRTP is enabled
        if !self.enabled {
            return Err(crate::Error::InvalidState(
                "disabled SRTP context cannot accept RTP as plaintext".to_string(),
            ));
        }

        let (header, _) = crate::packet::RtpHeader::parse_without_consuming(data)?;
        let ssrc = header.ssrc;
        let seq = header.sequence_number;
        let candidate = candidate_rtp_index(self.inbound_state(ssrc), seq);

        // Authentication deliberately happens before the replay decision is
        // applied or any state entry is inserted.
        let packet = self
            .inbound_crypto
            .decrypt_rtp_with_roc(data, candidate.roc)?;
        if candidate.before_roc_zero {
            return Err(crate::Error::SrtpError(
                "SRTP packet maps before ROC zero".to_string(),
            ));
        }
        let roc = candidate.roc;
        let packet_index = candidate.index;
        if self
            .inbound_state(ssrc)
            .is_some_and(|state| !state.replay.accepts(packet_index))
        {
            return Err(crate::Error::SrtpError(
                "SRTP packet is a duplicate or outside the replay window".to_string(),
            ));
        }
        let state = self.inbound_state_mut(ssrc);
        commit_rtp_index(state, roc, seq, packet_index);
        Ok(packet)
    }

    /// In-place twin of [`Self::unprotect`] for the executor v2 inline receive
    /// path: `data` is the caller's reusable buffer, authenticated and
    /// decrypted where it lies, and the result points into it. ROC estimation,
    /// authentication before any state change, the ROC-zero guard, the replay
    /// window and the commit are the same steps in the same order.
    pub(crate) fn unprotect_in_place(
        &mut self,
        data: &mut [u8],
    ) -> Result<crate::packet::rtp::RtpPacketBounds, crate::Error> {
        if !self.enabled {
            return Err(crate::Error::InvalidState(
                "disabled SRTP context cannot accept RTP as plaintext".to_string(),
            ));
        }

        let (header, _) = crate::packet::RtpHeader::parse_without_consuming(data)?;
        let ssrc = header.ssrc;
        let seq = header.sequence_number;
        let candidate = candidate_rtp_index(self.inbound_state(ssrc), seq);

        let packet = match self.hot.as_deref() {
            // Hot call state: the packed key material (same checks, order and
            // errors).
            Some(hot) => hot.rx.decrypt_in_place(data, candidate.roc)?,
            None => self
                .inbound_crypto
                .decrypt_rtp_in_place_with_roc(data, candidate.roc)?,
        };
        if candidate.before_roc_zero {
            return Err(crate::Error::SrtpError(
                "SRTP packet maps before ROC zero".to_string(),
            ));
        }
        let roc = candidate.roc;
        let packet_index = candidate.index;
        if self
            .inbound_state(ssrc)
            .is_some_and(|state| !state.replay.accepts(packet_index))
        {
            return Err(crate::Error::SrtpError(
                "SRTP packet is a duplicate or outside the replay window".to_string(),
            ));
        }
        let state = self.inbound_state_mut(ssrc);
        commit_rtp_index(state, roc, seq, packet_index);
        Ok(packet)
    }

    /// Protect an RTCP packet (SRTCP encryption)
    /// Returns the encrypted data with the authentication tag appended
    pub fn protect_rtcp(&mut self, data: &[u8]) -> Result<bytes::Bytes, crate::Error> {
        // Check if SRTP is enabled
        if !self.enabled {
            return Err(crate::Error::InvalidState(
                "disabled SRTP context cannot pass RTCP through as plaintext".to_string(),
            ));
        }

        // Parse the complete plaintext before choosing or consuming an index.
        validate_plaintext_rtcp(data)?;
        let ssrc = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
        let index = match self.outbound_srtcp.get(&ssrc) {
            Some(send_state) if send_state.exhausted => {
                return Err(crate::Error::SrtpError(
                    "SRTCP index exhausted; rekey before sending".to_string(),
                ));
            }
            Some(send_state) => send_state.next_index,
            None => 1,
        };
        if index > 0x7fff_ffff {
            return Err(crate::Error::SrtpError(
                "SRTCP index exhausted; rekey before sending".to_string(),
            ));
        }

        // Encrypt using SRTCP
        let (encrypted, auth_tag) = self.outbound_crypto.encrypt_rtcp_with_index(data, index)?;
        let send_state = self.outbound_srtcp.entry(ssrc).or_default();
        if index == 0x7fff_ffff {
            send_state.exhausted = true;
        } else {
            send_state.next_index += 1;
        }

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

    /// Unprotect an RTCP packet (SRTCP decryption)
    /// The input data should include the authentication tag if used
    pub fn unprotect_rtcp(&mut self, data: &[u8]) -> Result<bytes::Bytes, crate::Error> {
        // Check if SRTP is enabled
        if !self.enabled {
            return Err(crate::Error::InvalidState(
                "disabled SRTP context cannot accept RTCP as plaintext".to_string(),
            ));
        }

        // Authentication and decryption complete before parsing, replay
        // decisions, or any state entry is inserted.
        let (packet, ssrc, index) = self.inbound_crypto.decrypt_rtcp_with_index(data)?;
        validate_plaintext_rtcp(&packet)?;
        if self
            .inbound_srtcp
            .get(&ssrc)
            .is_some_and(|replay| !replay.accepts(u64::from(index)))
        {
            return Err(crate::Error::SrtpError(
                "SRTCP packet is a duplicate or outside the replay window".to_string(),
            ));
        }
        let replay = self.inbound_srtcp.entry(ssrc).or_default();
        replay.commit(u64::from(index));
        Ok(packet)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    fn contexts_with_suite(suite: SrtpCryptoSuite) -> (SrtpContext, SrtpContext) {
        let a_key = vec![0x11; 16];
        let a_salt = vec![0x22; 14];
        let b_key = vec![0x33; 16];
        let b_salt = vec![0x44; 14];
        let a = SrtpContext::new_from_keys(
            a_key.clone(),
            b_key.clone(),
            a_salt.clone(),
            b_salt.clone(),
            suite.clone(),
        )
        .unwrap();
        let b = SrtpContext::new_from_keys(b_key, a_key, b_salt, a_salt, suite).unwrap();
        (a, b)
    }

    fn contexts() -> (SrtpContext, SrtpContext) {
        contexts_with_suite(SRTP_AES128_CM_SHA1_80)
    }

    /// `protect_rtp_into` + `commit_outbound` produce the bytes and the
    /// state `protect` + `serialize` do, for every built-in suite, across a
    /// sequence-number wrap (ROC 0 -> 1), with and without a marker.
    #[test]
    fn in_place_protect_matches_the_allocating_path_across_a_rollover() {
        for suite in [
            SRTP_AES128_CM_SHA1_80,
            SRTP_AES128_CM_SHA1_32,
            SRTP_AES256_CM_SHA1_80,
            SRTP_AES256_CM_SHA1_32,
        ] {
            let key_len = suite.key_length;
            // Random keys and salts per run, shared by both contexts.
            let random = |len: usize| -> Vec<u8> {
                use rand::RngCore;
                let mut bytes = vec![0u8; len];
                rand::thread_rng().fill_bytes(&mut bytes);
                bytes
            };
            let (rtp_key, rtcp_key, rtp_salt, rtcp_salt) =
                (random(key_len), random(key_len), random(14), random(14));
            let make = || {
                SrtpContext::new_from_keys(
                    rtp_key.clone(),
                    rtcp_key.clone(),
                    rtp_salt.clone(),
                    rtcp_salt.clone(),
                    suite.clone(),
                )
                .unwrap()
            };
            let (mut legacy, mut inline) = (make(), make());
            let mut out = Vec::new();
            for (step, seq) in (65_530u16..=65_535).chain(0..6).enumerate() {
                let marker = step == 0;
                let mut reference = packet(0x0bad_cafe, seq);
                reference.header.marker = marker;
                let expected = protect_bytes(&mut legacy, &reference);
                out.clear();
                let commit = inline
                    .protect_rtp_into(
                        96,
                        marker,
                        seq,
                        u32::from(seq).wrapping_mul(160),
                        0x0bad_cafe,
                        b"authenticated media",
                        &[],
                        &mut out,
                    )
                    .unwrap();
                inline.commit_outbound(&[commit]);
                assert_eq!(&out[..], &expected[..], "seq {seq} differs");
            }
            let legacy_state = legacy.outbound_state(0x0bad_cafe).cloned().unwrap();
            let inline_state = inline.outbound_state(0x0bad_cafe).cloned().unwrap();
            assert_eq!(legacy_state.roc, 1);
            assert_eq!(inline_state.roc, legacy_state.roc);
            assert_eq!(inline_state.highest_seq, legacy_state.highest_seq);
            assert_eq!(inline_state.replay.highest, legacy_state.replay.highest);
            assert_eq!(inline_state.replay.bitmap, legacy_state.replay.bitmap);
        }
    }

    /// a batch protected against uncommitted predecessors equals the
    /// packets protected one by one, and an uncommitted packet leaves no
    /// state: protecting it again yields the same bytes (same index).
    #[test]
    fn batch_protection_sees_pending_packets_and_uncommitted_packets_leave_no_trace() {
        let (mut legacy, _) = contexts();
        let (mut inline, _) = contexts();
        let sequences = [65_534u16, 65_535, 0, 1];
        let mut expected = Vec::new();
        for seq in sequences {
            expected.push(protect_bytes(&mut legacy, &packet(7, seq)));
        }
        let mut arena = Vec::new();
        let mut commits = Vec::new();
        let mut ranges = Vec::new();
        for seq in sequences {
            let start = arena.len();
            let commit = inline
                .protect_rtp_into(
                    96,
                    false,
                    seq,
                    u32::from(seq).wrapping_mul(160),
                    7,
                    b"authenticated media",
                    &commits,
                    &mut arena,
                )
                .unwrap();
            commits.push(commit);
            ranges.push(start..arena.len());
        }
        for (range, expected) in ranges.iter().zip(&expected) {
            assert_eq!(&arena[range.clone()], &expected[..]);
        }
        // Only the first two "left": commit those; the last two are
        // protected again later and must come out identical.
        inline.commit_outbound(&commits[..2]);
        for (offset, seq) in sequences[2..].iter().enumerate() {
            let mut again = Vec::new();
            let commit = inline
                .protect_rtp_into(
                    96,
                    false,
                    *seq,
                    u32::from(*seq).wrapping_mul(160),
                    7,
                    b"authenticated media",
                    &[],
                    &mut again,
                )
                .unwrap();
            inline.commit_outbound(&[commit]);
            assert_eq!(&again[..], &expected[2 + offset][..]);
        }
        // Reuse is still refused exactly as `protect` refuses it.
        let mut reused = Vec::new();
        let error = inline
            .protect_rtp_into(96, false, 0, 0, 7, b"x", &[], &mut reused)
            .unwrap_err();
        assert!(error.to_string().contains("refusing to reuse"), "{error}");
        assert!(reused.is_empty(), "a refused packet writes nothing");
    }

    fn packet(ssrc: u32, seq: u16) -> crate::packet::RtpPacket {
        crate::packet::RtpPacket::new_with_payload(
            96,
            seq,
            u32::from(seq).wrapping_mul(160),
            ssrc,
            Bytes::from_static(b"authenticated media"),
        )
    }

    fn sender_report(ssrc: u32) -> Vec<u8> {
        let mut packet = vec![0x80, 200, 0, 6];
        packet.extend_from_slice(&ssrc.to_be_bytes());
        packet.extend_from_slice(&[0x5a; 20]);
        packet
    }

    fn inbound_srtcp_snapshot(context: &SrtpContext, ssrc: u32) -> Option<(Option<u64>, u64)> {
        context
            .inbound_srtcp
            .get(&ssrc)
            .map(|state| (state.highest, state.bitmap))
    }

    fn protect_bytes(context: &mut SrtpContext, packet: &crate::packet::RtpPacket) -> Bytes {
        context.protect(packet).unwrap().serialize().unwrap()
    }

    fn protect_bytes_with_roc(
        context: &SrtpContext,
        packet: &crate::packet::RtpPacket,
        roc: u32,
    ) -> Bytes {
        let (encrypted, auth_tag) = context
            .outbound_crypto
            .encrypt_rtp_with_roc(packet, roc)
            .unwrap();
        ProtectedRtpPacket {
            packet: encrypted,
            auth_tag,
        }
        .serialize()
        .unwrap()
    }

    fn inbound_state_snapshot(
        context: &SrtpContext,
        ssrc: u32,
    ) -> Option<(u32, Option<u16>, Option<u64>, u64)> {
        context.inbound_state(ssrc).map(|state| {
            (
                state.roc,
                state.highest_seq,
                state.replay.highest,
                state.replay.bitmap,
            )
        })
    }

    #[test]
    fn null_profiles_are_retained_but_fail_closed() {
        let key = SrtpCryptoKey::new(vec![0; 16], vec![0; 14]);
        let context = SrtpContext::new(SRTP_NULL_NULL, key);
        assert!(matches!(context, Err(crate::Error::UnsupportedFeature(_))));
    }

    #[test]
    fn aead_gcm_profiles_fail_closed() {
        let key = SrtpCryptoKey::new(vec![0; 16], vec![0; 14]);
        let error = SrtpContext::new(SRTP_AEAD_AES_128_GCM, key)
            .err()
            .expect("placeholder GCM profile must be rejected");
        assert!(matches!(error, crate::Error::UnsupportedFeature(_)));
    }

    #[test]
    fn disabled_context_rejects_plaintext_passthrough_without_state() {
        let (mut context, _) = contexts();
        let rtp = packet(0x0102_0304, 1);
        let plain_rtp = rtp.serialize().unwrap();
        let rtcp = sender_report(0x0102_0304);
        context.set_enabled(false);

        assert!(matches!(
            context.protect(&rtp),
            Err(crate::Error::InvalidState(_))
        ));
        assert!(matches!(
            context.unprotect(&plain_rtp),
            Err(crate::Error::InvalidState(_))
        ));
        assert!(matches!(
            context.protect_rtcp(&rtcp),
            Err(crate::Error::InvalidState(_))
        ));
        assert!(matches!(
            context.unprotect_rtcp(&rtcp),
            Err(crate::Error::InvalidState(_))
        ));
        assert!(context.rtp_state_is_empty(true));
        assert!(context.rtp_state_is_empty(false));
        assert!(context.outbound_srtcp.is_empty());
        assert!(context.inbound_srtcp.is_empty());
    }

    #[test]
    fn unsupported_key_rotation_does_not_mutate_outbound_state() {
        let rtp = packet(0x0102_0304, 0);
        for power in [8, 64, 255] {
            let (mut context, _) = contexts();
            context.set_key_rotation(KeyRotationFrequency::Power2(power));
            assert!(matches!(
                context.protect(&rtp),
                Err(crate::Error::UnsupportedFeature(_))
            ));
            assert!(context.rtp_state_is_empty(true));

            context.set_key_rotation(KeyRotationFrequency::None);
            context.protect(&rtp).unwrap();
        }
    }

    #[test]
    fn directional_keys_are_used_for_protect_and_unprotect() {
        let (mut a, mut b) = contexts();
        let wire = protect_bytes(&mut a, &packet(0x1020_3040, 7));
        assert_eq!(
            b.unprotect(&wire).unwrap().payload,
            Bytes::from_static(b"authenticated media")
        );
        assert!(a.unprotect(&wire).is_err(), "local key must not unprotect");
    }

    #[test]
    fn rollover_reordering_and_replay_are_per_ssrc() {
        let (mut a, mut b) = contexts();
        let ssrc = 0x0102_0304;
        for seq in [65_534, 65_535, 0, 1, 65_533] {
            let wire = protect_bytes(&mut a, &packet(ssrc, seq));
            b.unprotect(&wire).unwrap();
        }
        let state = b.inbound_state(ssrc).unwrap();
        assert_eq!(state.roc, 1);
        assert_eq!(state.highest_seq, Some(1));
        assert_eq!(state.replay.highest, Some((1_u64 << 16) | 1));

        let other_wire = protect_bytes(&mut a, &packet(0xaabb_ccdd, 0));
        b.unprotect(&other_wire).unwrap();

        let duplicate = protect_bytes(&mut a, &packet(0x9999_0001, 9));
        b.unprotect(&duplicate).unwrap();
        assert!(b.unprotect(&duplicate).is_err());
    }

    #[test]
    fn roc_zero_uses_wrapped_candidate_for_auth_without_state_poisoning() {
        let (mut a, mut b) = contexts();
        let ssrc = 0x1357_2468;

        let first = protect_bytes(&mut a, &packet(ssrc, 0));
        b.unprotect(&first).unwrap();
        let stable = inbound_state_snapshot(&b, ssrc);

        // The old saturating ROC calculation authenticated this packet under
        // ROC zero. RFC 3711 requires the wrapped previous-cycle ROC instead.
        let wrong_roc = protect_bytes_with_roc(&a, &packet(ssrc, u16::MAX), 0);
        assert!(matches!(
            b.unprotect(&wrong_roc),
            Err(crate::Error::AuthenticationFailed(_))
        ));
        assert_eq!(inbound_state_snapshot(&b, ssrc), stable);

        // A packet carrying a valid tag for the RFC candidate authenticates,
        // but that impossible pre-zero cycle must not advance either ROC or
        // the replay window.
        let wrapped_roc = protect_bytes_with_roc(&a, &packet(ssrc, u16::MAX), u32::MAX);
        assert!(matches!(
            b.unprotect(&wrapped_roc),
            Err(crate::Error::SrtpError(message)) if message.contains("before ROC zero")
        ));
        assert_eq!(inbound_state_snapshot(&b, ssrc), stable);

        let next = protect_bytes(&mut a, &packet(ssrc, 1));
        b.unprotect(&next)
            .expect("valid current-cycle packet succeeds after both failures");
        let state = b.inbound_state(ssrc).unwrap();
        assert_eq!(state.roc, 0);
        assert_eq!(state.highest_seq, Some(1));
    }

    #[test]
    fn outbound_packet_before_roc_zero_is_rejected_without_state_mutation() {
        let (mut a, _) = contexts();
        let ssrc = 0x2468_1357;
        protect_bytes(&mut a, &packet(ssrc, 0));
        let stable = a.outbound_state(ssrc).cloned().unwrap();

        assert!(matches!(
            a.protect(&packet(ssrc, u16::MAX)),
            Err(crate::Error::SrtpError(message)) if message.contains("before ROC zero")
        ));
        let current = a.outbound_state(ssrc).unwrap();
        assert_eq!(current.roc, stable.roc);
        assert_eq!(current.highest_seq, stable.highest_seq);
        assert_eq!(current.replay.highest, stable.replay.highest);
        assert_eq!(current.replay.bitmap, stable.replay.bitmap);

        a.protect(&packet(ssrc, 1))
            .expect("valid current-cycle packet succeeds after rejection");
    }

    #[test]
    fn failed_authentication_does_not_advance_rollover_or_replay_state() {
        let (mut a, mut b) = contexts();
        let ssrc = 0x1234_5678;
        for seq in [65_534, 65_535] {
            let wire = protect_bytes(&mut a, &packet(ssrc, seq));
            b.unprotect(&wire).unwrap();
        }

        let valid = protect_bytes(&mut a, &packet(ssrc, 0));
        let mut tampered = valid.to_vec();
        *tampered.last_mut().unwrap() ^= 0x80;
        assert!(matches!(
            b.unprotect(&tampered),
            Err(crate::Error::AuthenticationFailed(_))
        ));
        b.unprotect(&valid)
            .expect("valid rollover packet must succeed after failed authentication");
    }

    // ---- Hot call state: the packed per-direction block ----

    const HOT_SUITES: [SrtpCryptoSuite; 4] = [
        SRTP_AES128_CM_SHA1_80,
        SRTP_AES128_CM_SHA1_32,
        SRTP_AES256_CM_SHA1_80,
        SRTP_AES256_CM_SHA1_32,
    ];

    /// Random master keys and salts for both ends, drawn per call (no key
    /// material is hard-coded).
    struct HotKeys {
        a_key: Vec<u8>,
        b_key: Vec<u8>,
        a_salt: Vec<u8>,
        b_salt: Vec<u8>,
    }

    fn hot_keys(suite: &SrtpCryptoSuite) -> HotKeys {
        use rand::Rng;
        let random =
            |len: usize| -> Vec<u8> { (0..len).map(|_| rand::thread_rng().gen()).collect() };
        HotKeys {
            a_key: random(suite.key_length),
            b_key: random(suite.key_length),
            a_salt: random(14),
            b_salt: random(14),
        }
    }

    /// (sender, receiver) for `suite` and `keys`, packed or not (explicitly,
    /// whatever the executor configuration says).
    fn hot_pair(
        suite: &SrtpCryptoSuite,
        keys: &HotKeys,
        packed: bool,
    ) -> (SrtpContext, SrtpContext) {
        let mut a = SrtpContext::new_from_keys(
            keys.a_key.clone(),
            keys.b_key.clone(),
            keys.a_salt.clone(),
            keys.b_salt.clone(),
            suite.clone(),
        )
        .unwrap();
        let mut b = SrtpContext::new_from_keys(
            keys.b_key.clone(),
            keys.a_key.clone(),
            keys.b_salt.clone(),
            keys.a_salt.clone(),
            suite.clone(),
        )
        .unwrap();
        assert_eq!(a.set_hot_state(packed), packed);
        assert_eq!(b.set_hot_state(packed), packed);
        (a, b)
    }

    type StateView = Option<(u32, Option<u16>, Option<u64>, u64)>;

    fn view(state: Option<&RtpStreamState>) -> StateView {
        state.map(|state| {
            (
                state.roc,
                state.highest_seq,
                state.replay.highest,
                state.replay.bitmap,
            )
        })
    }

    #[test]
    fn the_packed_block_puts_each_part_on_its_own_cache_line() {
        use crate::hot_state::{CacheAligned, CACHE_LINE};
        assert_eq!(std::mem::align_of::<SrtpDirectionHot>(), CACHE_LINE);
        assert_eq!(
            std::mem::size_of::<CacheAligned<HotStream>>(),
            CACHE_LINE,
            "stream state, salt and lengths share line 0"
        );
        assert_eq!(std::mem::offset_of!(SrtpDirectionHot, stream), 0);
        assert_eq!(
            std::mem::offset_of!(SrtpDirectionHot, key),
            CACHE_LINE,
            "key on line 1"
        );
        assert_eq!(
            std::mem::offset_of!(SrtpDirectionHot, mac),
            2 * CACHE_LINE,
            "keyed HMAC on line 2"
        );
        // The inner and outer SHA-1 states.
        assert_eq!(std::mem::size_of::<auth::HmacSha1Core>(), 40);
        assert_eq!(std::mem::size_of::<SrtpDirectionHot>(), 3 * CACHE_LINE);
        let suite = SRTP_AES256_CM_SHA1_80;
        let (packed, _) = hot_pair(&suite, &hot_keys(&suite), true);
        let hot = packed.hot.as_deref().unwrap();
        for direction in [&hot.tx, &hot.rx] {
            assert_eq!(
                direction as *const SrtpDirectionHot as usize % CACHE_LINE,
                0
            );
        }
        assert_eq!(hot.tx.key().len(), 32);
        assert_eq!(usize::from(hot.tx.stream.tag_len), suite.tag_length);
    }

    #[test]
    fn the_configuration_decides_at_build_and_v1_never_packs() {
        use crate::executor::{override_media_executor_config_on_this_thread, MediaExecutorConfig};
        let suite = SRTP_AES128_CM_SHA1_80;
        let keys = hot_keys(&suite);
        let key = || SrtpCryptoKey::new(keys.a_key.clone(), keys.a_salt.clone());
        let build = |config: MediaExecutorConfig| {
            let _config = override_media_executor_config_on_this_thread(config);
            SrtpContext::new(suite.clone(), key()).unwrap()
        };
        let before = crate::hot_state::hot_state_counters().srtp_contexts;
        let packed = build(MediaExecutorConfig::v2());
        assert!(packed.hot_state_packed());
        assert!(crate::hot_state::hot_state_counters().srtp_contexts > before);
        assert!(!build(MediaExecutorConfig::v2().with_hot_state(false)).hot_state_packed());
        assert!(!build(MediaExecutorConfig::v1().with_hot_state(true)).hot_state_packed());
        assert!(!build(MediaExecutorConfig::v1()).hot_state_packed());
        // Decided at build: a later configuration does not repack it.
        let _v1 = override_media_executor_config_on_this_thread(MediaExecutorConfig::v1());
        assert!(packed.hot_state_packed());
    }

    /// TX: `protect`, `protect_rtp_into` (with pending, uncommitted and
    /// reused indices) and `commit_outbound` give the same bytes, errors and
    /// rollover/replay state packed or not, for every suite, across a
    /// sequence wrap and an SSRC change and back (the held stream spills to
    /// the map and returns).
    #[test]
    fn packed_tx_bytes_errors_and_state_match_the_session_key_path() {
        for suite in HOT_SUITES.iter() {
            let keys = hot_keys(suite);
            let (mut plain, _) = hot_pair(suite, &keys, false);
            let (mut packed, _) = hot_pair(suite, &keys, true);
            let mut transcript: Vec<[String; 2]> = Vec::new();
            let script: Vec<(u32, u16, bool)> = (65_530u16..=65_535)
                .chain(0..4)
                .map(|seq| (0x0a0a_0a0a, seq, true))
                .chain([(0x0b0b_0b0b, 7, true), (0x0b0b_0b0b, 8, false)])
                .chain((4..8).map(|seq| (0x0a0a_0a0a, seq, seq % 2 == 0)))
                // Index reuse is refused identically.
                .chain([(0x0a0a_0a0a, 5, true), (0x0b0b_0b0b, 7, true)])
                .collect();
            for (step, &(ssrc, seq, commit)) in script.iter().enumerate() {
                let mut outcome = [String::new(), String::new()];
                for (slot, context) in [&mut plain, &mut packed].into_iter().enumerate() {
                    let mut out = vec![0xee; 3];
                    let result = context.protect_rtp_into(
                        96,
                        step == 0,
                        seq,
                        u32::from(seq).wrapping_mul(160),
                        ssrc,
                        b"hot state media payload",
                        &[],
                        &mut out,
                    );
                    outcome[slot] = match result {
                        Ok(pending) => {
                            if commit {
                                context.commit_outbound(&[pending]);
                            }
                            format!("ok {:?} {:02x?}", pending, out)
                        }
                        Err(error) => format!("err {error:?} {:02x?}", out),
                    };
                }
                assert_eq!(outcome[0], outcome[1], "{suite:?} step {step}");
                transcript.push(outcome);
                for probe in [0x0a0a_0a0a, 0x0b0b_0b0b, 0x0c0c_0c0c] {
                    assert_eq!(
                        view(plain.outbound_state(probe)),
                        view(packed.outbound_state(probe)),
                        "{suite:?} step {step} ssrc {probe:08x}"
                    );
                }
            }
            // The allocating `protect` path shares the state.
            for seq in [8u16, 9] {
                let a = protect_bytes(&mut plain, &packet(0x0a0a_0a0a, seq));
                let b = protect_bytes(&mut packed, &packet(0x0a0a_0a0a, seq));
                assert_eq!(a, b, "{suite:?} protect seq {seq}");
            }
            // A batch with pending predecessors, then a partial commit.
            let mut arenas = [Vec::new(), Vec::new()];
            for (slot, context) in [&mut plain, &mut packed].into_iter().enumerate() {
                let mut pending = Vec::new();
                for seq in [10u16, 11, 12] {
                    let commit = context
                        .protect_rtp_into(
                            96,
                            false,
                            seq,
                            0,
                            0x0a0a_0a0a,
                            b"x",
                            &pending,
                            &mut arenas[slot],
                        )
                        .unwrap();
                    pending.push(commit);
                }
                context.commit_outbound(&pending[..2]);
            }
            assert_eq!(arenas[0], arenas[1], "{suite:?} batch");
            assert_eq!(
                view(plain.outbound_state(0x0a0a_0a0a)),
                view(packed.outbound_state(0x0a0a_0a0a))
            );
            assert!(
                transcript.iter().any(|[line, _]| line.starts_with("err")),
                "reuse was exercised"
            );
        }
    }

    /// RX: in order, reordered, replayed, tampered, truncated, padded,
    /// across a ROC rollover, outside the replay window, and with a second
    /// SSRC interleaved: `unprotect_in_place` and `unprotect` return the same
    /// bounds/packets and errors and leave the same state, packed or not.
    #[test]
    fn packed_rx_bounds_errors_and_state_match_the_session_key_path() {
        for suite in HOT_SUITES.iter() {
            let keys = hot_keys(suite);
            let (mut sender, _) = hot_pair(suite, &keys, false);
            let ssrc_a = 0x0a0a_0a0a;
            let ssrc_b = 0x0b0b_0b0b;
            let mut wires: Vec<Vec<u8>> = Vec::new();
            let mut a_wires: Vec<Vec<u8>> = Vec::new();
            for seq in 65_520u16..=65_535 {
                a_wires.push(protect_bytes(&mut sender, &packet(ssrc_a, seq)).to_vec());
            }
            for seq in 0u16..40 {
                a_wires.push(protect_bytes(&mut sender, &packet(ssrc_a, seq)).to_vec());
            }
            let mut padded = packet(ssrc_a, 40);
            padded.set_padding(4);
            let padded = sender
                .protect(&padded)
                .unwrap()
                .serialize()
                .unwrap()
                .to_vec();
            let b_wires: Vec<Vec<u8>> = (500u16..506)
                .map(|seq| protect_bytes(&mut sender, &packet(ssrc_b, seq)).to_vec())
                .collect();
            // In order across the wrap, with a reorder.
            wires.extend(a_wires[..10].iter().cloned());
            wires.push(a_wires[12].clone());
            wires.push(a_wires[11].clone());
            wires.push(a_wires[10].clone());
            // Replay of an accepted packet.
            wires.push(a_wires[11].clone());
            // SSRC B interleaved (the held stream changes and changes back).
            wires.push(b_wires[0].clone());
            wires.extend(a_wires[13..20].iter().cloned());
            wires.push(b_wires[1].clone());
            wires.push(b_wires[3].clone());
            // Tampered payload, then its genuine twin.
            let mut tampered = a_wires[20].clone();
            let middle = tampered.len() / 2;
            tampered[middle] ^= 0x01;
            wires.push(tampered);
            wires.push(a_wires[20].clone());
            // Tampered tag, truncated below a tag, below a header, empty.
            let mut bad_tag = a_wires[21].clone();
            let last = bad_tag.len() - 1;
            bad_tag[last] ^= 0x80;
            wires.push(bad_tag);
            wires.push(a_wires[21][..14].to_vec());
            wires.push(a_wires[21][..5].to_vec());
            wires.push(a_wires[21][..2].to_vec());
            wires.push(Vec::new());
            wires.extend(a_wires[21..55].iter().cloned());
            wires.push(padded);
            // Older than the 64-packet window, then B again.
            wires.push(a_wires[22].clone());
            wires.push(b_wires[2].clone());
            wires.push(b_wires[5].clone());

            for in_place in [true, false] {
                let (_, mut plain) = hot_pair(suite, &keys, false);
                let (_, mut packed) = hot_pair(suite, &keys, true);
                let mut accepted = 0;
                for (index, wire) in wires.iter().enumerate() {
                    let mut outcome = [String::new(), String::new()];
                    for (slot, context) in [&mut plain, &mut packed].into_iter().enumerate() {
                        outcome[slot] = if in_place {
                            let mut buffer = wire.clone();
                            match context.unprotect_in_place(&mut buffer) {
                                Ok(bounds) => format!(
                                    "ok {:?} {:02x?} pad {}",
                                    bounds.header,
                                    &buffer[bounds.payload.clone()],
                                    bounds.padding_size
                                ),
                                Err(error) => format!("err {error:?}"),
                            }
                        } else {
                            match context.unprotect(wire) {
                                Ok(packet) => format!("ok {packet:?}"),
                                Err(error) => format!("err {error:?}"),
                            }
                        };
                    }
                    assert_eq!(
                        outcome[0], outcome[1],
                        "{suite:?} in_place={in_place} wire {index}"
                    );
                    accepted += usize::from(outcome[0].starts_with("ok"));
                    for probe in [ssrc_a, ssrc_b] {
                        assert_eq!(
                            view(plain.inbound_state(probe)),
                            view(packed.inbound_state(probe)),
                            "{suite:?} in_place={in_place} wire {index} ssrc {probe:08x}"
                        );
                    }
                }
                assert!(
                    accepted > 40 && accepted < wires.len(),
                    "{suite:?}: {accepted} of {}",
                    wires.len()
                );
                // Unpacking gives the held stream back to the map exactly.
                let held = view(packed.inbound_state(ssrc_b));
                assert!(!packed.set_hot_state(false));
                assert_eq!(view(packed.inbound_rtp.get(&ssrc_b)), held);
                assert_eq!(
                    view(packed.inbound_rtp.get(&ssrc_a)),
                    view(plain.inbound_state(ssrc_a))
                );
                assert!(packed.set_hot_state(true), "and it packs again");
                // ... and keeps unprotecting exactly like the plain context.
                let next =
                    protect_bytes(&mut sender, &packet(ssrc_a, 41 + u16::from(!in_place))).to_vec();
                let mut plain_buffer = next.clone();
                let mut packed_buffer = next;
                assert_eq!(
                    format!("{:?}", plain.unprotect_in_place(&mut plain_buffer)),
                    format!("{:?}", packed.unprotect_in_place(&mut packed_buffer))
                );
                assert_eq!(plain_buffer, packed_buffer);
            }
        }
    }

    #[test]
    fn rtp_padding_is_encrypted_and_restored_after_unprotect() {
        let (mut a, mut b) = contexts();
        let mut original = packet(0x4567_89ab, 42);
        original.set_padding(4);
        let plaintext = original.serialize().unwrap();
        let header_size = original.header.size();

        let protected = a.protect(&original).unwrap();
        assert_eq!(protected.packet.padding_size, 0);
        let wire = protected.serialize().unwrap();
        let ciphertext_end = wire.len() - SRTP_AES128_CM_SHA1_80.tag_length;
        assert_ne!(
            &wire[header_size..ciphertext_end],
            &plaintext[header_size..],
            "the RTP padding octets must be encrypted with the payload"
        );

        let restored = b.unprotect(&wire).unwrap();
        assert_eq!(restored.payload, original.payload);
        assert_eq!(restored.padding_size, 4);
        assert_eq!(restored.serialize().unwrap(), plaintext);
    }

    #[test]
    fn unseen_packet_outside_the_replay_window_is_rejected() {
        let (mut a, mut b) = contexts();
        let ssrc = 0x0bad_f00d;
        b.unprotect(&protect_bytes(&mut a, &packet(ssrc, 0)))
            .unwrap();
        let delayed = protect_bytes(&mut a, &packet(ssrc, 1));
        for seq in 2..=70 {
            b.unprotect(&protect_bytes(&mut a, &packet(ssrc, seq)))
                .unwrap();
        }
        assert!(b.unprotect(&delayed).is_err());
    }

    #[test]
    fn srtcp_index_changes_ciphertext_and_replay_is_rejected() {
        let (mut a, mut b) = contexts();
        let plain = sender_report(0x1234_5678);
        let first = a.protect_rtcp(&plain).unwrap();
        let second = a.protect_rtcp(&plain).unwrap();
        assert_ne!(first, second);
        assert_ne!(&first[8..plain.len()], &plain[8..]);
        assert_eq!(b.unprotect_rtcp(&first).unwrap().as_ref(), plain);
        assert_eq!(b.unprotect_rtcp(&second).unwrap().as_ref(), plain);
        assert!(b.unprotect_rtcp(&first).is_err());
    }

    #[test]
    fn srtcp_directional_keys_are_used() {
        let (mut a, mut b) = contexts();
        let plain = sender_report(0x1020_3040);
        let wire = a.protect_rtcp(&plain).unwrap();
        assert_eq!(b.unprotect_rtcp(&wire).unwrap().as_ref(), plain);
        assert!(a.unprotect_rtcp(&wire).is_err());
    }

    #[test]
    fn srtcp_reordering_duplicates_and_old_packets_use_a_per_ssrc_window() {
        let (mut a, mut b) = contexts();
        let ssrc = 0x7654_3210;
        let plain = sender_report(ssrc);
        let wires: Vec<_> = (0..70).map(|_| a.protect_rtcp(&plain).unwrap()).collect();

        b.unprotect_rtcp(&wires[0]).unwrap();
        b.unprotect_rtcp(&wires[2]).unwrap();
        b.unprotect_rtcp(&wires[1])
            .expect("reordered packet inside the replay window");
        assert!(b.unprotect_rtcp(&wires[1]).is_err());

        for wire in &wires[3..] {
            b.unprotect_rtcp(wire).unwrap();
        }
        assert!(
            b.unprotect_rtcp(&wires[1]).is_err(),
            "packet older than the 64-packet window must remain rejected"
        );
    }

    #[test]
    fn malformed_authenticated_srtcp_does_not_mutate_receive_state() {
        let (a, mut b) = contexts();
        let ssrc = 0x1112_1314;
        let malformed = [0x80, 200, 0, 6, 0x11, 0x12, 0x13, 0x14];
        let (encrypted, tag) = a
            .outbound_crypto
            .encrypt_rtcp_with_index(&malformed, 1)
            .unwrap();
        let mut wire = encrypted.to_vec();
        wire.extend_from_slice(&tag.unwrap());

        assert!(b.unprotect_rtcp(&wire).is_err());
        assert_eq!(inbound_srtcp_snapshot(&b, ssrc), None);

        let mut sender = a;
        let valid = sender.protect_rtcp(&sender_report(ssrc)).unwrap();
        assert!(b.unprotect_rtcp(&valid).is_ok());
    }

    #[test]
    fn malformed_sdes_cannot_consume_srtcp_send_or_receive_state() {
        let (sender, mut receiver) = contexts();
        let ssrc = 0x2122_2324;
        // One declared SDES chunk containing only its SSRC and no mandatory
        // END item. The common header and declared packet length are valid.
        let malformed = [0x81, 202, 0, 1, 0x21, 0x22, 0x23, 0x24];

        let mut outbound = sender;
        assert!(outbound.protect_rtcp(&malformed).is_err());
        assert!(!outbound.outbound_srtcp.contains_key(&ssrc));

        let (encrypted, tag) = outbound
            .outbound_crypto
            .encrypt_rtcp_with_index(&malformed, 1)
            .unwrap();
        let mut wire = encrypted.to_vec();
        wire.extend_from_slice(&tag.unwrap());
        assert!(receiver.unprotect_rtcp(&wire).is_err());
        assert_eq!(inbound_srtcp_snapshot(&receiver, ssrc), None);
    }

    #[test]
    fn authenticated_cleartext_srtcp_is_rejected_without_state_mutation() {
        use hmac::{Hmac, Mac};
        use sha1::Sha1;

        let (_, mut receiver) = contexts();
        let ssrc = 0x3141_5926;
        let plain = sender_report(ssrc);
        let mut wire = plain.clone();
        // Index one with E=0: the packet is correctly authenticated below but
        // contradicts the negotiated AES-CM encryption policy.
        wire.extend_from_slice(&1_u32.to_be_bytes());

        let remote_master = SrtpCryptoKey::new(vec![0x11; 16], vec![0x22; 14]);
        let auth_key = srtp_kdf(
            &remote_master,
            &SrtpKeyDerivationParams {
                label: KeyDerivationLabel::RtcpAuthentication,
                key_derivation_rate: 0,
                index: 0,
            },
            20,
        )
        .unwrap();
        let mut mac = Hmac::<Sha1>::new_from_slice(&auth_key).unwrap();
        mac.update(&wire);
        wire.extend_from_slice(&mac.finalize().into_bytes()[..10]);

        assert!(matches!(
            receiver.unprotect_rtcp(&wire),
            Err(crate::Error::AuthenticationFailed(message))
                if message.contains("requires the E flag")
        ));
        assert_eq!(inbound_srtcp_snapshot(&receiver, ssrc), None);
    }

    #[test]
    fn malformed_plaintext_rtcp_does_not_consume_an_outbound_index() {
        let (mut a, _) = contexts();
        let ssrc = 0x2233_4455;
        assert!(a.protect_rtcp(&[0x80, 205, 0, 0]).is_err());
        assert!(a.outbound_srtcp.is_empty());
        let malformed = [0x80, 200, 0, 6, 0x22, 0x33, 0x44, 0x55];
        assert!(a.protect_rtcp(&malformed).is_err());
        assert!(!a.outbound_srtcp.contains_key(&ssrc));

        let wire = a.protect_rtcp(&sender_report(ssrc)).unwrap();
        let index_offset = wire.len() - SRTP_AES128_CM_SHA1_80.srtcp_tag_length() - 4;
        assert_eq!(
            u32::from_be_bytes(wire[index_offset..index_offset + 4].try_into().unwrap())
                & 0x7fff_ffff,
            1
        );
    }

    #[test]
    fn unknown_well_formed_rtcp_type_survives_srtcp_round_trip() {
        let (mut sender, mut receiver) = contexts();
        let unknown = [0x80, 208, 0, 1, 0x11, 0x22, 0x33, 0x44];

        let wire = sender.protect_rtcp(&unknown).unwrap();
        assert_eq!(receiver.unprotect_rtcp(&wire).unwrap().as_ref(), unknown);
    }

    #[test]
    fn sha1_32_uses_four_byte_srtp_and_ten_byte_srtcp_tags() {
        assert_eq!(SRTP_AES128_CM_SHA1_32.tag_length, 4);
        assert_eq!(SRTP_AES128_CM_SHA1_32.srtcp_tag_length(), 10);

        let (mut a, mut b) = contexts_with_suite(SRTP_AES128_CM_SHA1_32);
        let rtp = protect_bytes(&mut a, &packet(0x1020_3040, 1));
        assert_eq!(rtp.len(), packet(0x1020_3040, 1).size() + 4);
        b.unprotect(&rtp).unwrap();

        let rtcp = [0x80, 201, 0, 1, 0x12, 0x34, 0x56, 0x78];
        let srtcp = a.protect_rtcp(&rtcp).unwrap();
        assert_eq!(srtcp.len(), rtcp.len() + 4 + 10);
        assert_eq!(b.unprotect_rtcp(&srtcp).unwrap().as_ref(), rtcp);
    }

    #[test]
    fn srtcp_auth_failure_does_not_consume_an_index() {
        let (mut a, mut b) = contexts();
        let plain = [0x80, 201, 0, 1, 0x12, 0x34, 0x56, 0x78];
        let ssrc = 0x1234_5678;
        let valid = a.protect_rtcp(&plain).unwrap();
        let mut tampered = valid.to_vec();
        *tampered.last_mut().unwrap() ^= 0x01;
        assert!(matches!(
            b.unprotect_rtcp(&tampered),
            Err(crate::Error::AuthenticationFailed(_))
        ));
        assert_eq!(inbound_srtcp_snapshot(&b, ssrc), None);
        assert_eq!(b.unprotect_rtcp(&valid).unwrap().as_ref(), plain);
    }

    #[test]
    fn srtcp_indexes_are_independent_per_ssrc_and_never_wrap() {
        let (mut a, mut b) = contexts();
        let first = [0x80, 201, 0, 1, 0x11, 0x11, 0x11, 0x11];
        let second = [0x80, 201, 0, 1, 0x22, 0x22, 0x22, 0x22];
        let first_wire = a.protect_rtcp(&first).unwrap();
        let second_wire = a.protect_rtcp(&second).unwrap();
        assert_eq!(
            u32::from_be_bytes(
                first_wire[first_wire.len() - 14..first_wire.len() - 10]
                    .try_into()
                    .unwrap()
            ) & 0x7fff_ffff,
            1
        );
        assert_eq!(
            u32::from_be_bytes(
                second_wire[second_wire.len() - 14..second_wire.len() - 10]
                    .try_into()
                    .unwrap()
            ) & 0x7fff_ffff,
            1
        );
        assert_eq!(b.unprotect_rtcp(&first_wire).unwrap().as_ref(), first);
        assert_eq!(b.unprotect_rtcp(&second_wire).unwrap().as_ref(), second);

        a.outbound_srtcp.insert(
            0x1111_1111,
            SrtcpSendState {
                next_index: 0x7fff_ffff,
                exhausted: false,
            },
        );
        a.protect_rtcp(&first).expect("last SRTCP index is usable");
        assert!(a.protect_rtcp(&first).is_err());
    }

    #[test]
    fn in_place_unprotect_matches_allocating_unprotect_for_order_replay_tamper_and_padding() {
        const SSRC: u32 = 0x5157_0001;
        for suite in [SRTP_AES128_CM_SHA1_80, SRTP_AES128_CM_SHA1_32] {
            let (mut sender, mut heap) = contexts_with_suite(suite.clone());
            let (_, mut in_place) = contexts_with_suite(suite.clone());
            let mut wires: Vec<Vec<u8>> = Vec::new();
            for seq in [1_u16, 2, 4, 3] {
                wires.push(protect_bytes(&mut sender, &packet(SSRC, seq)).to_vec());
            }
            // Replay of an already accepted packet.
            wires.push(wires[1].clone());
            // A tampered packet must fail without advancing state; the
            // genuine one with the same sequence must then succeed.
            let genuine = protect_bytes(&mut sender, &packet(SSRC, 5)).to_vec();
            let mut tampered = genuine.clone();
            let middle = tampered.len() / 2;
            tampered[middle] ^= 0x01;
            wires.push(tampered);
            wires.push(genuine);
            let mut padded = packet(SSRC, 6);
            padded.set_padding(4);
            wires.push(protect_bytes(&mut sender, &padded).to_vec());
            // A late packet outside the 64-packet replay window.
            let mut stale = Vec::new();
            for seq in 7_u16..80 {
                let wire = protect_bytes(&mut sender, &packet(SSRC, seq)).to_vec();
                if seq == 8 {
                    stale = wire.clone();
                }
                wires.push(wire);
            }
            wires.push(stale);

            for (index, wire) in wires.iter().enumerate() {
                let heap_result = heap.unprotect(wire);
                let mut buffer = wire.clone();
                let in_place_result = in_place.unprotect_in_place(&mut buffer);
                match (heap_result, in_place_result) {
                    (Ok(owned), Ok(bounds)) => {
                        assert_eq!(owned.header, bounds.header, "wire {index}");
                        assert_eq!(
                            &owned.payload[..],
                            &buffer[bounds.payload.clone()],
                            "wire {index}"
                        );
                        assert_eq!(owned.padding_size, bounds.padding_size, "wire {index}");
                    }
                    (Err(_), Err(_)) => {}
                    (heap_result, in_place_result) => panic!(
                        "wire {index}: allocating ok={} in-place ok={}",
                        heap_result.is_ok(),
                        in_place_result.is_ok()
                    ),
                }
                assert_eq!(
                    inbound_state_snapshot(&heap, SSRC),
                    inbound_state_snapshot(&in_place, SSRC),
                    "wire {index}"
                );
            }
            // Spot-check the expected verdicts: replay, tamper and stale fail.
            let mut probe = wires[4].clone();
            assert!(in_place.unprotect_in_place(&mut probe).is_err());
        }
    }
}

#[cfg(test)]
mod integration_tests;
