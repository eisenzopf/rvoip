//! Production-grade SRTP adapter backed by `webrtc-srtp` 0.17.
//!
//! This module wraps the [`webrtc_srtp`] crate behind a simple API that
//! operates on raw packet bytes, replacing the self-built SRTP implementation
//! for new code.
//!
//! ## Usage
//!
//! ```ignore
//! use rvoip_rtp_core::srtp::adapter::SrtpContextAdapter;
//! use rvoip_rtp_core::dtls::adapter::SrtpKeyMaterial;
//! use webrtc_srtp::protection_profile::ProtectionProfile;
//!
//! // After DTLS handshake, extract keys:
//! let keys: SrtpKeyMaterial = dtls_adapter.get_srtp_keys().await?;
//!
//! // Create SRTP context:
//! let mut ctx = SrtpContextAdapter::from_key_material(&keys)?;
//!
//! // Protect outbound RTP:
//! let srtp_packet = ctx.protect_rtp(&rtp_bytes)?;
//!
//! // Unprotect inbound SRTP:
//! let rtp_bytes = ctx.unprotect_rtp(&srtp_bytes)?;
//! ```

use bytes::Bytes;
use tracing::debug;
use webrtc_srtp::context::Context as WebrtcSrtpContext;
use webrtc_srtp::protection_profile::ProtectionProfile;

use crate::dtls::adapter::{SrtpKeyMaterial, to_srtp_protection_profile};
use crate::error::Error;

/// Production-grade SRTP context backed by `webrtc-srtp`.
///
/// Maintains separate encrypt and decrypt contexts because `webrtc-srtp`
/// `Context` is one-directional (either encrypt-only or decrypt-only).
pub struct SrtpContextAdapter {
    /// Context for outbound (protect/encrypt) operations.
    encrypt_context: WebrtcSrtpContext,
    /// Context for inbound (unprotect/decrypt) operations.
    decrypt_context: WebrtcSrtpContext,
}

impl SrtpContextAdapter {
    /// Create a new SRTP context from raw key material.
    ///
    /// `local_key` + `local_salt` are used for outbound (protect) operations.
    /// `remote_key` + `remote_salt` are used for inbound (unprotect) operations.
    pub fn new(
        local_key: &[u8],
        local_salt: &[u8],
        remote_key: &[u8],
        remote_salt: &[u8],
        profile: ProtectionProfile,
    ) -> Result<Self, Error> {
        let encrypt_context = WebrtcSrtpContext::new(local_key, local_salt, profile, None, None)
            .map_err(|e| Error::SrtpError(format!("Failed to create SRTP encrypt context: {e}")))?;

        let decrypt_context = WebrtcSrtpContext::new(remote_key, remote_salt, profile, None, None)
            .map_err(|e| Error::SrtpError(format!("Failed to create SRTP decrypt context: {e}")))?;

        debug!(
            profile = ?profile,
            "Created webrtc-srtp context adapter"
        );

        Ok(Self {
            encrypt_context,
            decrypt_context,
        })
    }

    /// Create a new SRTP context from [`SrtpKeyMaterial`] extracted from a DTLS handshake.
    pub fn from_key_material(keys: &SrtpKeyMaterial) -> Result<Self, Error> {
        let profile = to_srtp_protection_profile(keys.profile)?;
        Self::new(
            &keys.local_key,
            &keys.local_salt,
            &keys.remote_key,
            &keys.remote_salt,
            profile,
        )
    }

    /// Protect (encrypt + authenticate) an outbound RTP packet.
    ///
    /// Takes a plain RTP packet as raw bytes and returns the SRTP-protected bytes.
    pub fn protect_rtp(&mut self, pkt: &[u8]) -> Result<Bytes, Error> {
        self.encrypt_context
            .encrypt_rtp(pkt)
            .map_err(|e| Error::SrtpError(format!("SRTP protect RTP failed: {e}")))
    }

    /// Unprotect (verify + decrypt) an inbound SRTP packet.
    ///
    /// Takes an SRTP packet as raw bytes and returns the plain RTP bytes.
    pub fn unprotect_rtp(&mut self, pkt: &[u8]) -> Result<Bytes, Error> {
        self.decrypt_context
            .decrypt_rtp(pkt)
            .map_err(|e| Error::SrtpError(format!("SRTP unprotect RTP failed: {e}")))
    }

    /// Protect (encrypt + authenticate) an outbound RTCP packet.
    ///
    /// Takes a plain RTCP packet as raw bytes and returns the SRTCP-protected bytes.
    pub fn protect_rtcp(&mut self, pkt: &[u8]) -> Result<Bytes, Error> {
        self.encrypt_context
            .encrypt_rtcp(pkt)
            .map_err(|e| Error::SrtpError(format!("SRTCP protect failed: {e}")))
    }

    /// Unprotect (verify + decrypt) an inbound SRTCP packet.
    ///
    /// Takes an SRTCP packet as raw bytes and returns the plain RTCP bytes.
    pub fn unprotect_rtcp(&mut self, pkt: &[u8]) -> Result<Bytes, Error> {
        self.decrypt_context
            .decrypt_rtcp(pkt)
            .map_err(|e| Error::SrtpError(format!("SRTCP unprotect failed: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_srtp_context_adapter_creation() {
        // AES-128-CM-HMAC-SHA1-80: 16-byte key, 14-byte salt
        let key = vec![0x01; 16];
        let salt = vec![0x02; 14];

        let ctx = SrtpContextAdapter::new(
            &key,
            &salt,
            &key,
            &salt,
            ProtectionProfile::Aes128CmHmacSha1_80,
        );
        assert!(ctx.is_ok(), "SRTP context creation should succeed");
    }

    #[test]
    fn test_srtp_context_adapter_wrong_key_len() {
        // Deliberately wrong key length
        let key = vec![0x01; 8]; // Should be 16
        let salt = vec![0x02; 14];

        let ctx = SrtpContextAdapter::new(
            &key,
            &salt,
            &key,
            &salt,
            ProtectionProfile::Aes128CmHmacSha1_80,
        );
        assert!(ctx.is_err(), "Should fail with wrong key length");
    }

    #[test]
    fn test_srtp_protect_unprotect_roundtrip() {
        // Create two contexts: one for each direction
        let key_a = vec![0xAA; 16];
        let salt_a = vec![0xBB; 14];
        let key_b = vec![0xCC; 16];
        let salt_b = vec![0xDD; 14];

        let mut sender = SrtpContextAdapter::new(
            &key_a, &salt_a,
            &key_b, &salt_b,
            ProtectionProfile::Aes128CmHmacSha1_80,
        )
        .expect("sender context");

        let mut receiver = SrtpContextAdapter::new(
            &key_b, &salt_b,
            &key_a, &salt_a,
            ProtectionProfile::Aes128CmHmacSha1_80,
        )
        .expect("receiver context");

        // Minimal valid RTP packet:
        // Version=2, PT=0, Seq=1, Timestamp=160, SSRC=1, payload=0x00...
        let rtp_packet: Vec<u8> = vec![
            0x80, 0x00, // V=2, P=0, X=0, CC=0, M=0, PT=0
            0x00, 0x01, // Sequence number = 1
            0x00, 0x00, 0x00, 0xA0, // Timestamp = 160
            0x00, 0x00, 0x00, 0x01, // SSRC = 1
            0x00, 0x01, 0x02, 0x03, // Payload
        ];

        // Protect
        let protected = sender.protect_rtp(&rtp_packet).expect("protect should succeed");
        assert_ne!(protected.as_ref(), &rtp_packet[..], "Protected should differ from plain");

        // Unprotect
        let unprotected = receiver.unprotect_rtp(&protected).expect("unprotect should succeed");
        assert_eq!(&unprotected[..], &rtp_packet[..], "Round-trip should produce original");
    }

    #[test]
    fn test_srtcp_protect_unprotect_roundtrip() {
        let key_a = vec![0xAA; 16];
        let salt_a = vec![0xBB; 14];
        let key_b = vec![0xCC; 16];
        let salt_b = vec![0xDD; 14];

        let mut sender = SrtpContextAdapter::new(
            &key_a, &salt_a,
            &key_b, &salt_b,
            ProtectionProfile::Aes128CmHmacSha1_80,
        )
        .expect("sender context");

        let mut receiver = SrtpContextAdapter::new(
            &key_b, &salt_b,
            &key_a, &salt_a,
            ProtectionProfile::Aes128CmHmacSha1_80,
        )
        .expect("receiver context");

        // Minimal valid RTCP SR packet (28 bytes):
        // V=2, P=0, RC=0, PT=200 (SR), Length=6 (words), SSRC=1
        let rtcp_packet: Vec<u8> = vec![
            0x80, 0xC8, // V=2, P=0, RC=0, PT=200(SR)
            0x00, 0x06, // Length = 6 (28 bytes total)
            0x00, 0x00, 0x00, 0x01, // SSRC = 1
            // NTP timestamp (8 bytes)
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            // RTP timestamp (4 bytes)
            0x00, 0x00, 0x00, 0x00,
            // Sender packet count (4 bytes)
            0x00, 0x00, 0x00, 0x00,
            // Sender octet count (4 bytes)
            0x00, 0x00, 0x00, 0x00,
        ];

        let protected = sender.protect_rtcp(&rtcp_packet).expect("protect RTCP should succeed");
        assert_ne!(protected.as_ref(), &rtcp_packet[..]);

        let unprotected = receiver.unprotect_rtcp(&protected).expect("unprotect RTCP should succeed");
        assert_eq!(&unprotected[..], &rtcp_packet[..]);
    }
}
