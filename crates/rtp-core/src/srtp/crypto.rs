use bytes::{Bytes, BytesMut, Buf, BufMut};
use std::collections::HashMap;
use std::sync::Arc;
use aes::{Aes128, cipher::{KeyIvInit, StreamCipher, generic_array::GenericArray}};
use ctr::Ctr64BE;
use hmac::{Hmac, Mac};
use sha1::Sha1;
use crate::error::Error;
use crate::Result;
use crate::packet::RtpPacket;
use super::{SrtpCryptoSuite, SrtpEncryptionAlgorithm, SrtpAuthenticationAlgorithm};
use super::auth::SrtpReplayProtection;

// Define types for AES-CM
type Aes128Ctr64BE = Ctr64BE<Aes128>;
// Define type for HMAC-SHA1
type HmacSha1 = Hmac<Sha1>;

/// Basic cryptographic key/salt for SRTP
#[derive(Debug, Clone)]
pub struct SrtpCryptoKey {
    /// Raw key material
    key: Vec<u8>,
    
    /// Salt for the key
    salt: Vec<u8>,
}

impl SrtpCryptoKey {
    /// Create a new SRTP key from raw bytes
    pub fn new(key: Vec<u8>, salt: Vec<u8>) -> Self {
        Self { key, salt }
    }
    
    /// Get a reference to the key material
    pub fn key(&self) -> &[u8] {
        &self.key
    }
    
    /// Get a reference to the salt
    pub fn salt(&self) -> &[u8] {
        &self.salt
    }
    
    /// Create a key from a base64 string (as used in SDP)
    pub fn from_base64(data: &str) -> Result<Self> {
        let decoded = base64::decode(data)
            .map_err(|e| Error::SrtpError(format!("Failed to decode base64 key: {}", e)))?;
        
        // Typical format is 30 bytes = 16 bytes key + 14 bytes salt
        if decoded.len() < 16 {
            return Err(Error::SrtpError("Key material too short".to_string()));
        }
        
        // Split into key and salt
        let key = decoded[0..16].to_vec();
        let salt = if decoded.len() > 16 {
            decoded[16..].to_vec()
        } else {
            Vec::new()
        };
        
        Ok(Self { key, salt })
    }
}

/// Per-SSRC rollover counter (ROC) and sequence tracking state.
///
/// RFC 3711 Section 3.3.1: the ROC increments each time the 16-bit RTP
/// sequence number wraps past 65535.  We keep the highest sequence number
/// seen so far so that we can detect the wrap.
#[derive(Debug, Clone)]
struct SsrcState {
    /// Rollover counter - incremented on sequence number wrap
    roc: u32,
    /// Highest RTP sequence number observed for this SSRC
    highest_seq: u16,
    /// Whether we have received at least one packet (to seed highest_seq)
    initialized: bool,
    /// Per-SSRC replay protection window
    replay: SrtpReplayProtection,
}

impl SsrcState {
    fn new() -> Self {
        Self {
            roc: 0,
            highest_seq: 0,
            initialized: false,
            // 64-packet replay window per RFC 3711 Section 3.3.2
            replay: SrtpReplayProtection::new(64),
        }
    }
}

/// SRTP context for encryption/decryption
pub struct SrtpCrypto {
    /// Crypto suite in use
    suite: SrtpCryptoSuite,

    /// Master key for encryption
    master_key: SrtpCryptoKey,

    /// Session keys derived from master key
    session_keys: Option<SrtpSessionKeys>,

    /// Per-SSRC ROC + sequence tracking (for RTP)
    ssrc_states: HashMap<u32, SsrcState>,

    /// SRTCP index counter - incremented on every SRTCP packet sent
    srtcp_send_index: u32,

    /// SRTCP replay protection for received packets
    srtcp_replay: SrtpReplayProtection,
}

/// Derived session keys for SRTP
#[derive(Debug, Clone)]
struct SrtpSessionKeys {
    /// Key for RTP encryption
    rtp_enc_key: Vec<u8>,
    
    /// Key for RTP authentication
    rtp_auth_key: Vec<u8>,
    
    /// Salt for RTP encryption
    rtp_salt: Vec<u8>,
    
    /// Key for RTCP encryption
    rtcp_enc_key: Vec<u8>,
    
    /// Key for RTCP authentication
    rtcp_auth_key: Vec<u8>,
    
    /// Salt for RTCP encryption
    rtcp_salt: Vec<u8>,
}

impl SrtpCrypto {
    /// Create a new SRTP crypto context
    pub fn new(suite: SrtpCryptoSuite, master_key: SrtpCryptoKey) -> Result<Self> {
        // Reject unimplemented AEAD-GCM profiles that currently use placeholder primitives.
        // These suites advertise AesCm + Null auth which is NOT real AEAD-GCM and would
        // silently provide the wrong security properties.
        if suite.encryption == SrtpEncryptionAlgorithm::AesCm
            && suite.authentication == SrtpAuthenticationAlgorithm::Null
            && suite.tag_length == 16
        {
            return Err(Error::NotImplemented(
                "AEAD_AES_GCM SRTP profiles are not yet implemented; \
                 use SRTP_AES128_CM_SHA1_80 or SRTP_AES128_CM_SHA1_32 instead"
                    .to_string(),
            ));
        }

        // Validate key length
        if master_key.key().len() != suite.key_length {
            return Err(Error::SrtpError(format!(
                "Key length mismatch: expected {} but got {}",
                suite.key_length, master_key.key().len()
            )));
        }
        
        let mut crypto = Self {
            suite,
            master_key,
            session_keys: None,
            ssrc_states: HashMap::new(),
            srtcp_send_index: 0,
            // 64-packet replay window for SRTCP
            srtcp_replay: SrtpReplayProtection::new(64),
        };
        
        // Derive session keys
        crypto.derive_keys()?;
        
        Ok(crypto)
    }
    
    /// Derive session keys from master key
    fn derive_keys(&mut self) -> Result<()> {
        // Use our KDF to derive session keys according to RFC 3711
        
        // Derive RTP encryption key
        let rtp_enc_params = super::SrtpKeyDerivationParams {
            label: super::KeyDerivationLabel::RtpEncryption,
            key_derivation_rate: 0,
            index: 0,
        };
        let rtp_enc_key = super::srtp_kdf(&self.master_key, &rtp_enc_params, self.suite.key_length)?;
        
        // Derive RTP authentication key (20 bytes for HMAC-SHA1)
        let rtp_auth_params = super::SrtpKeyDerivationParams {
            label: super::KeyDerivationLabel::RtpAuthentication,
            key_derivation_rate: 0,
            index: 0,
        };
        let rtp_auth_key = super::srtp_kdf(&self.master_key, &rtp_auth_params, 20)?;
        
        // Derive RTP salt
        let rtp_salt_params = super::SrtpKeyDerivationParams {
            label: super::KeyDerivationLabel::RtpSalt,
            key_derivation_rate: 0,
            index: 0,
        };
        let rtp_salt = super::srtp_kdf(&self.master_key, &rtp_salt_params, 14)?;
        
        // Derive RTCP encryption key
        let rtcp_enc_params = super::SrtpKeyDerivationParams {
            label: super::KeyDerivationLabel::RtcpEncryption,
            key_derivation_rate: 0,
            index: 0,
        };
        let rtcp_enc_key = super::srtp_kdf(&self.master_key, &rtcp_enc_params, self.suite.key_length)?;
        
        // Derive RTCP authentication key
        let rtcp_auth_params = super::SrtpKeyDerivationParams {
            label: super::KeyDerivationLabel::RtcpAuthentication,
            key_derivation_rate: 0,
            index: 0,
        };
        let rtcp_auth_key = super::srtp_kdf(&self.master_key, &rtcp_auth_params, 20)?;
        
        // Derive RTCP salt
        let rtcp_salt_params = super::SrtpKeyDerivationParams {
            label: super::KeyDerivationLabel::RtcpSalt,
            key_derivation_rate: 0,
            index: 0,
        };
        let rtcp_salt = super::srtp_kdf(&self.master_key, &rtcp_salt_params, 14)?;
        
        // Store the derived keys
        let session_keys = SrtpSessionKeys {
            rtp_enc_key,
            rtp_auth_key,
            rtp_salt,
            rtcp_enc_key,
            rtcp_auth_key,
            rtcp_salt,
        };
        
        self.session_keys = Some(session_keys);
        Ok(())
    }
    
    /// Update per-SSRC ROC state for an outgoing packet and return
    /// (roc, packet_index).  The sender always uses the current ROC
    /// because the sender controls its own sequence numbering.
    fn update_send_roc(&mut self, ssrc: u32, seq: u16) -> (u32, u64) {
        let state = self.ssrc_states.entry(ssrc).or_insert_with(SsrcState::new);

        if !state.initialized {
            state.highest_seq = seq;
            state.initialized = true;
        } else if seq == 0 && state.highest_seq == u16::MAX {
            // Sequence wrapped 65535 -> 0
            state.roc = state.roc.wrapping_add(1);
        } else if seq < state.highest_seq
            && state.highest_seq.wrapping_sub(seq) > 0x8000
        {
            // Large backwards jump also signals a wrap
            state.roc = state.roc.wrapping_add(1);
        }

        if seq > state.highest_seq
            || (state.highest_seq.wrapping_sub(seq) > 0x8000)
        {
            state.highest_seq = seq;
        }

        let roc = state.roc;
        let packet_index = (roc as u64) << 16 | (seq as u64);
        (roc, packet_index)
    }

    /// Estimate the ROC for an incoming packet and return
    /// (estimated_roc, packet_index).  Per RFC 3711 Section 3.3.1
    /// we compare the received sequence number to the highest seen
    /// to decide whether a rollover has occurred.
    ///
    /// This method is *tentative*: it does NOT mutate any state.
    /// After successful authentication the caller must invoke
    /// `commit_recv_state` to persist the changes.
    fn estimate_recv_roc(&self, ssrc: u32, seq: u16) -> Result<(u32, u64)> {
        let state = self.ssrc_states.get(&ssrc);

        let estimated_roc;
        match state {
            None => {
                // First packet for this SSRC
                estimated_roc = 0;
            }
            Some(st) if !st.initialized => {
                estimated_roc = 0;
            }
            Some(st) => {
                let s_l = st.highest_seq;
                // RFC 3711: estimate v using s_l (highest_seq) and the ROC
                if seq > s_l {
                    // Normal case: sequence advanced
                    if seq.wrapping_sub(s_l) < 0x8000 {
                        estimated_roc = st.roc;
                    } else {
                        // Backwards wrap: ROC-1, but clamp to 0 to prevent underflow
                        estimated_roc = st.roc.saturating_sub(1);
                    }
                } else if s_l.wrapping_sub(seq) > 0x8000 {
                    // Forward wrap: ROC+1
                    estimated_roc = st.roc.wrapping_add(1);
                } else {
                    // seq <= s_l, within normal range
                    estimated_roc = st.roc;
                }
            }
        }

        let packet_index = (estimated_roc as u64) << 16 | (seq as u64);

        // Tentative replay check — does NOT mutate replay state
        let replay_ok = match state {
            Some(st) => st.replay.check_tentative(packet_index)
                .map_err(|e| Error::SrtpError(format!("Replay check failed: {}", e)))?,
            None => true, // no state yet, first packet is always ok
        };

        if !replay_ok {
            return Err(Error::SrtpError("Packet rejected by replay protection".to_string()));
        }

        Ok((estimated_roc, packet_index))
    }

    /// Commit receive-side state after successful authentication.
    /// Updates ROC, highest_seq, and replay window for the given SSRC.
    fn commit_recv_state(&mut self, ssrc: u32, seq: u16, estimated_roc: u32, packet_index: u64) {
        let state = self.ssrc_states.entry(ssrc).or_insert_with(SsrcState::new);

        // Commit to replay window
        state.replay.commit(packet_index);

        // Update ROC / highest_seq
        if !state.initialized {
            state.highest_seq = seq;
            state.initialized = true;
            state.roc = estimated_roc;
        } else if estimated_roc > state.roc
            || (estimated_roc == state.roc && seq > state.highest_seq)
        {
            state.roc = estimated_roc;
            state.highest_seq = seq;
        }
    }

    /// Encrypt an RTP packet
    pub fn encrypt_rtp(&mut self, packet: &RtpPacket) -> Result<(RtpPacket, Option<Vec<u8>>)> {
        let ssrc = packet.header.ssrc;
        let seq = packet.header.sequence_number;
        let (roc, packet_index) = self.update_send_roc(ssrc, seq);

        if self.suite.encryption == SrtpEncryptionAlgorithm::Null {
            // Null encryption, just return the original packet
            return if self.suite.authentication == SrtpAuthenticationAlgorithm::Null {
                // No authentication either
                Ok((packet.clone(), None))
            } else {
                // Authentication is enabled, calculate tag
                let serialized = packet.serialize()?;
                let auth_tag = self.calculate_auth_tag(&serialized, roc)?;
                Ok((packet.clone(), Some(auth_tag)))
            };
        }

        // Get session keys
        let session_keys = self.session_keys.as_ref()
            .ok_or_else(|| Error::SrtpError("Session keys not derived".to_string()))?;

        // Extract header
        let header = packet.header.clone();

        // Create an IV using salt and packet info
        let iv = match self.suite.encryption {
            SrtpEncryptionAlgorithm::AesCm => {
                super::create_srtp_iv(&session_keys.rtp_salt, ssrc, packet_index)?
            },
            _ => return Err(Error::SrtpError("Unsupported encryption algorithm".to_string())),
        };

        // Create a mutable copy of the payload for encryption
        let mut encrypted_payload = BytesMut::from(&packet.payload[..]);

        // Encrypt the payload
        match self.suite.encryption {
            SrtpEncryptionAlgorithm::AesCm => {
                aes_cm_encrypt(&mut encrypted_payload, &session_keys.rtp_enc_key, &iv)?;
            },
            _ => return Err(Error::SrtpError("Unsupported encryption algorithm".to_string())),
        }

        // Create a new packet with the encrypted payload
        let encrypted_packet = RtpPacket::new(header, encrypted_payload.freeze());

        // Calculate authentication tag if authentication is enabled
        let auth_tag = if self.suite.authentication != SrtpAuthenticationAlgorithm::Null {
            // Serialize the encrypted packet for authentication
            let encrypted_serialized = encrypted_packet.serialize()?;

            // Calculate the authentication tag (includes ROC per RFC 3711 Section 4.2)
            let auth_tag = self.calculate_auth_tag(&encrypted_serialized, roc)?;
            Some(auth_tag)
        } else {
            None
        };

        Ok((encrypted_packet, auth_tag))
    }
    
    /// Calculate authentication tag for a packet
    fn calculate_auth_tag(&self, packet_data: &[u8], roc: u32) -> Result<Vec<u8>> {
        if self.suite.authentication == SrtpAuthenticationAlgorithm::Null {
            return Err(Error::SrtpError("Authentication is not enabled".to_string()));
        }
        
        // Get session keys
        let session_keys = self.session_keys.as_ref()
            .ok_or_else(|| Error::SrtpError("Session keys not derived".to_string()))?;
        
        // Create an authenticator
        let authenticator = super::auth::SrtpAuthenticator::new(
            self.suite.authentication,
            session_keys.rtp_auth_key.clone(),
            self.suite.tag_length
        );
        
        // Calculate the authentication tag
        authenticator.calculate_auth_tag(packet_data, roc)
    }
    
    /// Decrypt an SRTP packet
    pub fn decrypt_rtp(&mut self, data: &[u8]) -> Result<RtpPacket> {
        if self.suite.encryption == SrtpEncryptionAlgorithm::Null
            && self.suite.authentication == SrtpAuthenticationAlgorithm::Null
        {
            // Null encryption and authentication, just parse the packet
            return RtpPacket::parse(data);
        }

        // Get session keys
        let session_keys = self.session_keys.as_ref()
            .ok_or_else(|| Error::SrtpError("Session keys not derived".to_string()))?;

        // Determine authentication tag size
        let auth_tag_size = if self.suite.authentication != SrtpAuthenticationAlgorithm::Null {
            self.suite.tag_length
        } else {
            0
        };

        // Check if the packet has an authentication tag
        if auth_tag_size > 0 && data.len() < auth_tag_size {
            return Err(Error::SrtpError("Packet too short to contain authentication tag".to_string()));
        }

        // Split data into packet and authentication tag
        let (packet_data, auth_tag) = if auth_tag_size > 0 {
            let tag_start = data.len() - auth_tag_size;
            (&data[0..tag_start], &data[tag_start..])
        } else {
            (data, &[][..])
        };

        // We need to peek at the header to get SSRC + sequence for ROC estimation.
        // RTP header: V(2) P(1) X(1) CC(4) M(1) PT(7) SEQ(16) => first 4 bytes,
        // then timestamp(32), then SSRC(32).  Minimum 12 bytes.
        if packet_data.len() < 12 {
            return Err(Error::SrtpError("Packet too short for RTP header".to_string()));
        }
        let seq = u16::from_be_bytes([packet_data[2], packet_data[3]]);
        let ssrc = u32::from_be_bytes([packet_data[8], packet_data[9], packet_data[10], packet_data[11]]);

        // Tentatively estimate ROC and check replay window (no state mutation)
        let (roc, packet_index) = self.estimate_recv_roc(ssrc, seq)?;

        let session_keys = self.session_keys.as_ref()
            .ok_or_else(|| Error::SrtpError("Session keys not derived".to_string()))?;

        // Verify authentication BEFORE committing any state
        if self.suite.authentication != SrtpAuthenticationAlgorithm::Null {
            let authenticator = super::auth::SrtpAuthenticator::new(
                self.suite.authentication,
                session_keys.rtp_auth_key.clone(),
                self.suite.tag_length,
            );

            let is_valid = authenticator.verify_auth_tag(packet_data, auth_tag, roc)?;
            if !is_valid {
                return Err(Error::SrtpError("Authentication failed".to_string()));
            }
        }

        // Authentication succeeded — now commit ROC, highest_seq, and replay state
        self.commit_recv_state(ssrc, seq, roc, packet_index);

        // Parse the RTP header (not encrypted)
        let packet = RtpPacket::parse(packet_data)?;

        if self.suite.encryption == SrtpEncryptionAlgorithm::Null {
            return Ok(packet);
        }

        let session_keys = self.session_keys.as_ref()
            .ok_or_else(|| Error::SrtpError("Session keys not derived".to_string()))?;

        // Create IV for decryption
        let iv = match self.suite.encryption {
            SrtpEncryptionAlgorithm::AesCm => {
                super::create_srtp_iv(&session_keys.rtp_salt, ssrc, packet_index)?
            },
            _ => return Err(Error::SrtpError("Unsupported encryption algorithm".to_string())),
        };

        // Create a mutable copy of the payload for decryption
        let mut decrypted_payload = BytesMut::from(&packet.payload[..]);

        // Decrypt the payload
        match self.suite.encryption {
            SrtpEncryptionAlgorithm::AesCm => {
                aes_cm_decrypt(&mut decrypted_payload, &session_keys.rtp_enc_key, &iv)?;
            },
            _ => return Err(Error::SrtpError("Unsupported encryption algorithm".to_string())),
        }

        // Create a new packet with the decrypted payload
        let decrypted_packet = RtpPacket::new(packet.header, decrypted_payload.freeze());

        Ok(decrypted_packet)
    }
    
    /// Extract SSRC from an RTCP packet.
    /// RTCP compound packets always start with SR or RR; the SSRC of the
    /// sender/reporter is at bytes 4..8.
    fn extract_rtcp_ssrc(data: &[u8]) -> Result<u32> {
        if data.len() < 8 {
            return Err(Error::SrtpError("RTCP packet too short to extract SSRC".to_string()));
        }
        Ok(u32::from_be_bytes([data[4], data[5], data[6], data[7]]))
    }

    /// Encrypt an RTCP packet
    pub fn encrypt_rtcp(&mut self, data: &[u8]) -> Result<(Bytes, Option<Vec<u8>>)> {
        // Allocate the next SRTCP index (31-bit, wraps at 2^31)
        let srtcp_index = self.srtcp_send_index;
        self.srtcp_send_index = self.srtcp_send_index.wrapping_add(1) & 0x7FFFFFFF;

        // Extract the actual SSRC from the RTCP header
        let ssrc = Self::extract_rtcp_ssrc(data)?;

        if self.suite.encryption == SrtpEncryptionAlgorithm::Null {
            return if self.suite.authentication == SrtpAuthenticationAlgorithm::Null {
                Ok((Bytes::copy_from_slice(data), None))
            } else {
                // For unencrypted SRTCP we still append the index+E word before auth
                let mut buf = BytesMut::with_capacity(data.len() + 4);
                buf.extend_from_slice(data);
                // E=0 (not encrypted), with the SRTCP index
                buf.put_u32(srtcp_index & 0x7FFFFFFF);
                let auth_tag = self.calculate_rtcp_auth_tag(&buf, srtcp_index)?;
                Ok((buf.freeze(), Some(auth_tag)))
            };
        }

        // Get session keys
        let session_keys = self.session_keys.as_ref()
            .ok_or_else(|| Error::SrtpError("Session keys not derived".to_string()))?;

        if data.len() <= 8 {
            return Err(Error::SrtpError("RTCP packet too short".to_string()));
        }

        // Extract header (first 8 bytes) and payload
        let header = &data[0..8];
        let payload = &data[8..];

        // Create a mutable buffer for our result
        let mut result = BytesMut::with_capacity(data.len() + 4);

        // Copy the header
        result.extend_from_slice(header);

        // Create a mutable copy of the payload for encryption
        let mut encrypted_payload = BytesMut::from(payload);

        // Create IV using actual SSRC and SRTCP index
        let iv = match self.suite.encryption {
            SrtpEncryptionAlgorithm::AesCm => {
                super::create_srtp_iv(&session_keys.rtcp_salt, ssrc, srtcp_index as u64)?
            },
            _ => return Err(Error::SrtpError("Unsupported encryption algorithm".to_string())),
        };

        // Encrypt the payload
        match self.suite.encryption {
            SrtpEncryptionAlgorithm::AesCm => {
                aes_cm_encrypt(&mut encrypted_payload, &session_keys.rtcp_enc_key, &iv)?;
            },
            _ => return Err(Error::SrtpError("Unsupported encryption algorithm".to_string())),
        }

        // Add encrypted payload to result
        result.extend_from_slice(&encrypted_payload);

        // Add SRTCP index with E flag set (bit 31 = 1 means encrypted)
        result.put_u32(0x80000000 | srtcp_index);

        // Calculate authentication tag if needed
        let auth_tag = if self.suite.authentication != SrtpAuthenticationAlgorithm::Null {
            let auth_tag = self.calculate_rtcp_auth_tag(&result, srtcp_index)?;
            Some(auth_tag)
        } else {
            None
        };

        Ok((result.freeze(), auth_tag))
    }
    
    /// Calculate authentication tag for an RTCP packet
    fn calculate_rtcp_auth_tag(&self, data: &[u8], index: u32) -> Result<Vec<u8>> {
        if self.suite.authentication == SrtpAuthenticationAlgorithm::Null {
            return Err(Error::SrtpError("Authentication is not enabled".to_string()));
        }
        
        // Get session keys
        let session_keys = self.session_keys.as_ref()
            .ok_or_else(|| Error::SrtpError("Session keys not derived".to_string()))?;
        
        // Create HMAC-SHA1 instance
        let tag = hmac_sha1(data, &session_keys.rtcp_auth_key, self.suite.tag_length)?;
        
        Ok(tag)
    }
    
    /// Decrypt an SRTCP packet
    pub fn decrypt_rtcp(&mut self, data: &[u8]) -> Result<Bytes> {
        if self.suite.encryption == SrtpEncryptionAlgorithm::Null
            && self.suite.authentication == SrtpAuthenticationAlgorithm::Null
        {
            return Ok(Bytes::copy_from_slice(data));
        }

        // Check packet minimum length (header + index + auth tag)
        let min_len = 8 + 4 + (if self.suite.authentication != SrtpAuthenticationAlgorithm::Null {
            self.suite.tag_length
        } else {
            0
        });

        if data.len() < min_len {
            return Err(Error::SrtpError(format!("SRTCP packet too short: {} bytes", data.len())));
        }

        // Calculate authentication tag position
        let auth_tag_pos = data.len() - self.suite.tag_length;

        // Verify authentication tag if authentication is enabled
        if self.suite.authentication != SrtpAuthenticationAlgorithm::Null {
            let session_keys = self.session_keys.as_ref()
                .ok_or_else(|| Error::SrtpError("Session keys not derived".to_string()))?;

            let packet_data = &data[0..auth_tag_pos];
            let auth_tag = &data[auth_tag_pos..];

            // Extract the SRTCP index from the packet for the auth tag calculation
            let index_pos = auth_tag_pos - 4;
            let idx_bytes = [data[index_pos], data[index_pos+1], data[index_pos+2], data[index_pos+3]];
            let index_value = u32::from_be_bytes(idx_bytes);
            let srtcp_index = index_value & 0x7FFFFFFF;

            let calculated_tag = self.calculate_rtcp_auth_tag(packet_data, srtcp_index)?;

            // Constant-time comparison
            if calculated_tag.len() != auth_tag.len() {
                return Err(Error::SrtpError("Authentication tag length mismatch".to_string()));
            }
            let mut cmp_result = 0u8;
            for (a, b) in calculated_tag.iter().zip(auth_tag.iter()) {
                cmp_result |= a ^ b;
            }
            if cmp_result != 0 {
                return Err(Error::SrtpError("SRTCP authentication failed".to_string()));
            }
        }

        // Get the index and E flag
        let index_pos = auth_tag_pos - 4;
        let index_bytes = [data[index_pos], data[index_pos+1], data[index_pos+2], data[index_pos+3]];
        let index_value = u32::from_be_bytes(index_bytes);
        let e_flag = (index_value & 0x80000000) != 0;
        let srtcp_index = index_value & 0x7FFFFFFF;

        // Replay protection for SRTCP
        if !self.srtcp_replay.check(srtcp_index as u64)
            .map_err(|e| Error::SrtpError(format!("SRTCP replay check failed: {}", e)))?
        {
            return Err(Error::SrtpError("SRTCP packet rejected by replay protection".to_string()));
        }

        // If E flag is not set, packet is not encrypted
        if !e_flag {
            let mut result = BytesMut::with_capacity(index_pos);
            result.extend_from_slice(&data[0..index_pos]);
            return Ok(result.freeze());
        }

        // Extract the actual SSRC from the RTCP header (bytes 4..8)
        let ssrc = Self::extract_rtcp_ssrc(data)?;

        let session_keys = self.session_keys.as_ref()
            .ok_or_else(|| Error::SrtpError("Session keys not derived".to_string()))?;

        // Extract header and encrypted payload
        let header = &data[0..8];
        let payload = &data[8..index_pos];

        let mut result = BytesMut::with_capacity(index_pos);
        result.extend_from_slice(header);

        let mut decrypted_payload = BytesMut::from(payload);

        // Create IV using actual SSRC and SRTCP index from the packet
        let iv = match self.suite.encryption {
            SrtpEncryptionAlgorithm::AesCm => {
                super::create_srtp_iv(&session_keys.rtcp_salt, ssrc, srtcp_index as u64)?
            },
            _ => return Err(Error::SrtpError("Unsupported encryption algorithm".to_string())),
        };

        match self.suite.encryption {
            SrtpEncryptionAlgorithm::AesCm => {
                aes_cm_decrypt(&mut decrypted_payload, &session_keys.rtcp_enc_key, &iv)?;
            },
            _ => return Err(Error::SrtpError("Unsupported encryption algorithm".to_string())),
        }

        result.extend_from_slice(&decrypted_payload);

        Ok(result.freeze())
    }
}

/// AES Counter Mode encryption for SRTP
fn aes_cm_encrypt(data: &mut [u8], key: &[u8], iv: &[u8]) -> Result<()> {
    // Convert key and iv to the format required by the cipher
    let key = GenericArray::from_slice(key);
    let iv = GenericArray::from_slice(&iv[0..16]);
    
    // Create a new AES-CM cipher instance
    let mut cipher = Aes128Ctr64BE::new(key, iv);
    
    // Encrypt data in-place
    cipher.apply_keystream(data);
    
    Ok(())
}

/// AES Counter Mode decryption for SRTP
fn aes_cm_decrypt(data: &mut [u8], key: &[u8], iv: &[u8]) -> Result<()> {
    // AES-CM is symmetric, so encryption and decryption are the same
    aes_cm_encrypt(data, key, iv)
}

/// HMAC-SHA1 authentication for SRTP
fn hmac_sha1(data: &[u8], key: &[u8], tag_length: usize) -> Result<Vec<u8>> {
    // Create a new HMAC-SHA1 instance
    let mut mac = HmacSha1::new_from_slice(key)
        .map_err(|e| Error::SrtpError(format!("Failed to create HMAC: {}", e)))?;
    
    // Update with data
    mac.update(data);
    
    // Finalize and get the result
    let result = mac.finalize().into_bytes();
    
    // Truncate to the requested tag length
    let tag = result.as_slice()[..tag_length].to_vec();
    
    Ok(tag)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_srtp_key_from_base64() {
        // Example base64 key
        let base64_key = "YUJjRGVGZ0hpSmtMbU5vUHFSc1R1Vndv";
        
        let key = SrtpCryptoKey::from_base64(base64_key);
        assert!(key.is_ok());
        
        let key = key.unwrap();
        assert_eq!(key.key().len(), 16);
        
        // Invalid base64
        let invalid_key = "invalid-base64!";
        let key = SrtpCryptoKey::from_base64(invalid_key);
        assert!(key.is_err());
    }
    
    #[test]
    fn test_null_encryption() {
        // Create a key
        let key = SrtpCryptoKey::new(vec![0; 16], vec![0; 14]);

        // Use a modified SRTP_NULL_NULL with correct key length for testing
        let null_suite = SrtpCryptoSuite {
            encryption: SrtpEncryptionAlgorithm::Null,
            authentication: SrtpAuthenticationAlgorithm::Null,
            key_length: 16, // Changed from 0 to 16 to match our test key
            tag_length: 0,
        };

        // Create crypto context with null encryption
        let mut crypto = SrtpCrypto::new(
            null_suite,
            key
        ).unwrap();

        // Create a test packet
        let header = crate::packet::RtpHeader::new(96, 1000, 12345, 0xabcdef01);
        let payload = Bytes::from_static(b"test payload");
        let packet = RtpPacket::new(header, payload);

        // Encrypt and verify it returns the same packet (null encryption)
        let encrypted_result = crypto.encrypt_rtp(&packet);
        assert!(encrypted_result.is_ok());
        let (encrypted, _auth_tag) = encrypted_result.unwrap();

        // Packets should be equal with null encryption
        assert_eq!(encrypted.header.payload_type, packet.header.payload_type);
        assert_eq!(encrypted.header.sequence_number, packet.header.sequence_number);
        assert_eq!(encrypted.header.timestamp, packet.header.timestamp);
        assert_eq!(encrypted.header.ssrc, packet.header.ssrc);
        assert_eq!(encrypted.payload, packet.payload);
    }
    
    #[test]
    fn test_aes_cm_encryption() {
        // Test data
        let mut data = vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        let key = vec![0; 16]; // 16-byte AES key (all zeros)
        let iv = vec![0; 16];  // 16-byte IV (all zeros)
        
        // Encrypt
        let result = aes_cm_encrypt(&mut data, &key, &iv);
        assert!(result.is_ok());
        
        // Data should now be encrypted - it should differ from the original
        assert_ne!(data, vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        
        // Make a copy of the encrypted data
        let encrypted = data.clone();
        
        // Now decrypt
        let result = aes_cm_decrypt(&mut data, &key, &iv);
        assert!(result.is_ok());
        
        // Data should now be decrypted back to the original
        assert_eq!(data, vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
    }
    
    #[test]
    fn test_hmac_sha1() {
        // Test data
        let data = vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        let key = vec![0; 20]; // 20-byte key (all zeros)
        
        // Calculate tag with length 10 (80 bits)
        let tag = hmac_sha1(&data, &key, 10);
        assert!(tag.is_ok());
        let tag = tag.unwrap();
        
        // Tag should be 10 bytes long
        assert_eq!(tag.len(), 10);
        
        // Calculate tag with length 4 (32 bits)
        let tag32 = hmac_sha1(&data, &key, 4);
        assert!(tag32.is_ok());
        let tag32 = tag32.unwrap();
        
        // Tag should be 4 bytes long
        assert_eq!(tag32.len(), 4);
        
        // First 4 bytes should match between the two tags
        assert_eq!(tag[0..4], tag32[0..4]);
    }
    
    #[test]
    fn test_complete_srtp_process() {
        // Create a master key and salt
        let master_key = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 
                             0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10];
        let master_salt = vec![0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 
                               0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D];
        
        let srtp_key = SrtpCryptoKey::new(master_key, master_salt);
        
        // Test both AES-CM suites
        let suites = vec![
            super::super::SRTP_AES128_CM_SHA1_80,
            super::super::SRTP_AES128_CM_SHA1_32,
        ];
        
        for suite in suites {
            // Use separate encrypt/decrypt contexts (as in real usage with SrtpContext)
            let mut enc_crypto = SrtpCrypto::new(suite.clone(), srtp_key.clone()).unwrap();
            let mut dec_crypto = SrtpCrypto::new(suite.clone(), srtp_key.clone()).unwrap();

            // Create a test packet
            let header = crate::packet::RtpHeader::new(96, 1000, 12345, 0xabcdef01);
            let payload = Bytes::from_static(b"Hello SRTP World! This is a test of SRTP encryption and decryption.");
            let packet = RtpPacket::new(header, payload);

            // Encrypt the packet
            let encrypted_result = enc_crypto.encrypt_rtp(&packet).unwrap();
            let (encrypted_packet, auth_tag) = encrypted_result;

            // Payload should be encrypted (different from original)
            assert_ne!(encrypted_packet.payload, packet.payload);

            // Header should not be encrypted
            assert_eq!(encrypted_packet.header.payload_type, packet.header.payload_type);
            assert_eq!(encrypted_packet.header.sequence_number, packet.header.sequence_number);
            assert_eq!(encrypted_packet.header.timestamp, packet.header.timestamp);
            assert_eq!(encrypted_packet.header.ssrc, packet.header.ssrc);

            // Serialize the packet
            let serialized = encrypted_packet.serialize().unwrap();

            // Add authentication tag (if provided)
            let mut protected_data = BytesMut::with_capacity(serialized.len() + 10);
            protected_data.extend_from_slice(&serialized);
            if let Some(tag) = auth_tag {
                protected_data.extend_from_slice(&tag);
            }

            // Decrypt the packet with a separate context
            let decrypted = dec_crypto.decrypt_rtp(&protected_data);
            assert!(decrypted.is_ok());
            let decrypted = decrypted.unwrap();

            // Decrypted packet should match original
            assert_eq!(decrypted.header.payload_type, packet.header.payload_type);
            assert_eq!(decrypted.header.sequence_number, packet.header.sequence_number);
            assert_eq!(decrypted.header.timestamp, packet.header.timestamp);
            assert_eq!(decrypted.header.ssrc, packet.header.ssrc);
            assert_eq!(decrypted.payload, packet.payload);
        }
    }

    #[test]
    fn test_tamper_detection() {
        // Create master key and crypto contexts (separate for encrypt/decrypt)
        let master_key = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
                             0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10];
        let master_salt = vec![0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17,
                               0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D];

        let srtp_key = SrtpCryptoKey::new(master_key, master_salt);
        let mut enc_crypto = SrtpCrypto::new(super::super::SRTP_AES128_CM_SHA1_80, srtp_key.clone()).unwrap();

        // Create a test packet
        let header = crate::packet::RtpHeader::new(96, 1000, 12345, 0xabcdef01);
        let payload = Bytes::from_static(b"Protected data");
        let packet = RtpPacket::new(header, payload);

        // Encrypt the packet
        let encrypted_result = enc_crypto.encrypt_rtp(&packet).unwrap();
        let (encrypted_packet, auth_tag) = encrypted_result;

        // Ensure auth tag is present
        assert!(auth_tag.is_some());

        // Clone the auth tag for later use
        let auth_tag_clone = auth_tag.clone();

        // Serialize the packet
        let serialized = encrypted_packet.serialize().unwrap();

        // Create protected data with auth tag
        let mut protected_data = BytesMut::with_capacity(serialized.len() + 10);
        protected_data.extend_from_slice(&serialized);

        // Add the auth tag to the protected data
        if let Some(tag) = auth_tag {
            protected_data.extend_from_slice(&tag);
        }
        let protected_data = protected_data.freeze();

        // Test 1: Verify normal decryption works (fresh decrypt context)
        let mut dec_crypto = SrtpCrypto::new(super::super::SRTP_AES128_CM_SHA1_80, srtp_key.clone()).unwrap();
        let decrypted = dec_crypto.decrypt_rtp(&protected_data);
        assert!(decrypted.is_ok());

        // Test 2: Tamper with the payload and verify it fails authentication
        let mut dec_crypto2 = SrtpCrypto::new(super::super::SRTP_AES128_CM_SHA1_80, srtp_key.clone()).unwrap();
        let mut tampered = protected_data.to_vec();

        // Change one byte in the middle of the packet
        let middle = tampered.len() / 2;
        tampered[middle] ^= 0xFF;

        let decrypted = dec_crypto2.decrypt_rtp(&tampered);
        assert!(decrypted.is_err());

        // Test 3: Tamper with the authentication tag and verify it fails
        let mut dec_crypto3 = SrtpCrypto::new(super::super::SRTP_AES128_CM_SHA1_80, srtp_key).unwrap();
        let mut tampered = protected_data.to_vec();
        if let Some(_tag) = auth_tag_clone {
            // Calculate position of the last byte in the auth tag
            let tag_idx = tampered.len() - 1;
            let tag_value = tampered[tag_idx];
            tampered[tag_idx] = tag_value ^ 0xFF;

            let decrypted = dec_crypto3.decrypt_rtp(&tampered);
            assert!(decrypted.is_err());
        }
    }

    #[test]
    fn test_roc_tracking_across_sequence_wrap() {
        // Verify that ROC increments when sequence number wraps from 65535 to 0
        let master_key = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
                             0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10];
        let master_salt = vec![0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17,
                               0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D];

        let srtp_key = SrtpCryptoKey::new(master_key, master_salt);
        let mut enc_crypto = SrtpCrypto::new(super::super::SRTP_AES128_CM_SHA1_80, srtp_key.clone()).unwrap();
        let mut dec_crypto = SrtpCrypto::new(super::super::SRTP_AES128_CM_SHA1_80, srtp_key).unwrap();

        let ssrc = 0xdeadbeef;

        // Send packet at seq 65534
        let header1 = crate::packet::RtpHeader::new(96, 65534, 100, ssrc);
        let packet1 = RtpPacket::new(header1, Bytes::from_static(b"pkt1"));
        let (enc1, tag1) = enc_crypto.encrypt_rtp(&packet1).unwrap();

        // Send packet at seq 65535
        let header2 = crate::packet::RtpHeader::new(96, 65535, 200, ssrc);
        let packet2 = RtpPacket::new(header2, Bytes::from_static(b"pkt2"));
        let (enc2, tag2) = enc_crypto.encrypt_rtp(&packet2).unwrap();

        // Send packet at seq 0 (wrap!)
        let header3 = crate::packet::RtpHeader::new(96, 0, 300, ssrc);
        let packet3 = RtpPacket::new(header3, Bytes::from_static(b"pkt3"));
        let (enc3, tag3) = enc_crypto.encrypt_rtp(&packet3).unwrap();

        // Verify ROC incremented in the encrypt context
        let enc_state = enc_crypto.ssrc_states.get(&ssrc).unwrap();
        assert_eq!(enc_state.roc, 1, "ROC should be 1 after sequence wrap");

        // Decrypt all three in order
        let mut build_wire = |enc_pkt: &RtpPacket, tag: &Option<Vec<u8>>| -> Vec<u8> {
            let ser = enc_pkt.serialize().unwrap();
            let mut buf = BytesMut::with_capacity(ser.len() + 10);
            buf.extend_from_slice(&ser);
            if let Some(t) = tag {
                buf.extend_from_slice(t);
            }
            buf.to_vec()
        };

        let wire1 = build_wire(&enc1, &tag1);
        let wire2 = build_wire(&enc2, &tag2);
        let wire3 = build_wire(&enc3, &tag3);

        let d1 = dec_crypto.decrypt_rtp(&wire1).unwrap();
        assert_eq!(d1.payload, Bytes::from_static(b"pkt1"));

        let d2 = dec_crypto.decrypt_rtp(&wire2).unwrap();
        assert_eq!(d2.payload, Bytes::from_static(b"pkt2"));

        let d3 = dec_crypto.decrypt_rtp(&wire3).unwrap();
        assert_eq!(d3.payload, Bytes::from_static(b"pkt3"));

        // Verify ROC also incremented in the decrypt context
        let dec_state = dec_crypto.ssrc_states.get(&ssrc).unwrap();
        assert_eq!(dec_state.roc, 1, "Decrypt ROC should be 1 after wrap");
    }

    #[test]
    fn test_replay_rejection() {
        // Verify that replaying the same SRTP packet is rejected
        let master_key = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
                             0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10];
        let master_salt = vec![0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17,
                               0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D];

        let srtp_key = SrtpCryptoKey::new(master_key, master_salt);
        let mut enc_crypto = SrtpCrypto::new(super::super::SRTP_AES128_CM_SHA1_80, srtp_key.clone()).unwrap();
        let mut dec_crypto = SrtpCrypto::new(super::super::SRTP_AES128_CM_SHA1_80, srtp_key).unwrap();

        let header = crate::packet::RtpHeader::new(96, 42, 12345, 0xdeadbeef);
        let packet = RtpPacket::new(header, Bytes::from_static(b"unique"));
        let (enc_pkt, tag) = enc_crypto.encrypt_rtp(&packet).unwrap();

        let ser = enc_pkt.serialize().unwrap();
        let mut wire = BytesMut::with_capacity(ser.len() + 10);
        wire.extend_from_slice(&ser);
        if let Some(t) = &tag {
            wire.extend_from_slice(t);
        }
        let wire = wire.to_vec();

        // First decrypt should succeed
        let d1 = dec_crypto.decrypt_rtp(&wire);
        assert!(d1.is_ok());

        // Replaying the same packet should fail
        let d2 = dec_crypto.decrypt_rtp(&wire);
        assert!(d2.is_err(), "Replayed packet should be rejected");
    }

    #[test]
    fn test_srtcp_index_increments() {
        // Verify that each SRTCP encrypt uses a unique index
        let master_key = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
                             0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10];
        let master_salt = vec![0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17,
                               0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D];

        let srtp_key = SrtpCryptoKey::new(master_key, master_salt);
        let mut crypto = SrtpCrypto::new(super::super::SRTP_AES128_CM_SHA1_80, srtp_key).unwrap();

        // Minimal RTCP-like packet (>8 bytes, with SSRC at bytes 4..8)
        let rtcp_data = vec![
            0x80, 0xC8, 0x00, 0x06, // V=2, PT=200(SR), length=6
            0xDE, 0xAD, 0xBE, 0xEF, // SSRC
            0x00, 0x00, 0x00, 0x00, // payload
            0x01, 0x02, 0x03, 0x04,
        ];

        let (enc1, _tag1) = crypto.encrypt_rtcp(&rtcp_data).unwrap();
        let (enc2, _tag2) = crypto.encrypt_rtcp(&rtcp_data).unwrap();

        // The encrypted outputs should differ because the SRTCP index differs
        assert_ne!(enc1, enc2, "Two SRTCP encryptions of the same data must produce different output");

        // Verify the SRTCP index counter advanced
        assert_eq!(crypto.srtcp_send_index, 2);
    }
} 