//! DTLS-SRTP (RFC 5764) via a real, external DTLS 1.2 engine.
//!
//! The in-house handshake in `crate::dtls` never completes a real handshake
//! between two independent parties: it never sends `Certificate`/
//! `CertificateVerify`, never activates record-layer encryption, and its
//! own `HandshakeState` has a `force_verification_buffer` escape hatch whose
//! doc comment admits it exists because independent `Finished`-message
//! verification was never made to work. This module replaces the handshake
//! itself with the `dtls` crate (the same lineage backing the webrtc-rs
//! project's "production" 0.17.x line), while reusing this crate's own,
//! already-correct SRTP crypto for everything downstream of the handshake.
//!
//! This module only drives the handshake and hands back derived SRTP keys
//! plus certificate fingerprints; it has no notion of SDP. The caller is
//! responsible for putting the local fingerprint into an outgoing
//! `a=fingerprint` line and checking the remote fingerprint against an
//! incoming one, exactly as it already builds `a=crypto` for the SDES path.
//!
//! Certificate generation is a separate step from running the handshake
//! ([`generate_identity`], then [`handshake_client`]/[`handshake_server`]
//! taking the result), not bundled into one call: a real SDP integration
//! needs its own `a=fingerprint` value to put in the offer/answer it sends
//! *before* it knows which role (`a=setup:active`/`passive`) it ends up
//! playing, let alone before the handshake itself can start.

use std::sync::Arc;

use sha2::{Digest, Sha256};

use dtls_ext::config::{ClientAuthType, Config, ExtendedMasterSecretType};
use dtls_ext::conn::DTLSConn;
use dtls_ext::crypto::Certificate as DtlsCertificate;
use webrtc_util::conn::Conn as UtilConn;
use webrtc_util::KeyingMaterialExporter;

pub use dtls_ext::extension::extension_use_srtp::SrtpProtectionProfile;

use crate::error::Error;
use crate::srtp::{
    SrtpAuthenticationAlgorithm, SrtpCryptoKey, SrtpCryptoSuite, SrtpEncryptionAlgorithm,
};
use crate::Result;

/// SRTP standard salt length in bytes (RFC 5764/3711).
const SRTP_SALT_LENGTH: usize = 14;

/// Default SRTP protection profiles to offer/accept, in preference order.
pub fn default_srtp_profiles() -> Vec<SrtpProtectionProfile> {
    vec![
        SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_80,
        SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_32,
    ]
}

/// A self-signed certificate plus its SHA-256 fingerprint, generated ahead
/// of the handshake so the fingerprint is available for an outgoing SDP
/// `a=fingerprint` line before the handshake (or even the DTLS role) is
/// decided. Typically one `DtlsIdentity` is generated per Connection/call
/// and reused if the handshake needs to restart (e.g. an ICE restart or a
/// retried connection), so the fingerprint you already sent stays valid.
pub struct DtlsIdentity {
    certificate: DtlsCertificate,
    /// SHA-256 fingerprint of `certificate`, for the outgoing SDP
    /// `a=fingerprint` line.
    pub fingerprint_sha256: [u8; 32],
}

/// Generate a fresh self-signed certificate and its fingerprint.
pub fn generate_identity() -> Result<DtlsIdentity> {
    let certificate =
        DtlsCertificate::generate_self_signed(vec!["rvoip".to_string()]).map_err(to_dtls_error)?;
    let fingerprint_sha256 = sha256_fingerprint(certificate.certificate[0].as_ref());
    Ok(DtlsIdentity {
        certificate,
        fingerprint_sha256,
    })
}

/// Result of a completed DTLS-SRTP handshake.
#[derive(Debug, Clone)]
pub struct DtlsSrtpHandshakeResult {
    /// Negotiated SRTP crypto suite.
    pub profile: SrtpCryptoSuite,
    /// Key the client (the handshake-initiating side) uses to encrypt.
    pub client_write_key: SrtpCryptoKey,
    /// Key the server (the handshake-accepting side) uses to encrypt.
    pub server_write_key: SrtpCryptoKey,
    /// SHA-256 fingerprint of the certificate the remote peer presented.
    /// Check this against the SDP `a=fingerprint` the peer advertised.
    pub remote_fingerprint_sha256: [u8; 32],
    /// SHA-256 fingerprint of our own certificate, for the outgoing SDP
    /// `a=fingerprint` line.
    pub local_fingerprint_sha256: [u8; 32],
}

fn to_dtls_error(e: dtls_ext::Error) -> Error {
    Error::DtlsHandshakeError(format!("dtls: {e}"))
}

fn sha256_fingerprint(der: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(der);
    hasher.finalize().into()
}

fn convert_profile(profile: SrtpProtectionProfile) -> Result<SrtpCryptoSuite> {
    match profile {
        SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_80 => Ok(SrtpCryptoSuite {
            encryption: SrtpEncryptionAlgorithm::AesCm,
            authentication: SrtpAuthenticationAlgorithm::HmacSha1_80,
            key_length: 16,
            tag_length: 10,
        }),
        SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_32 => Ok(SrtpCryptoSuite {
            encryption: SrtpEncryptionAlgorithm::AesCm,
            authentication: SrtpAuthenticationAlgorithm::HmacSha1_32,
            key_length: 16,
            tag_length: 4,
        }),
        other => Err(Error::UnsupportedFeature(format!(
            "SRTP profile {other:?} is not supported by rvoip-rtp-core's SRTP crypto module yet"
        ))),
    }
}

fn build_config(identity: DtlsIdentity, srtp_profiles: Vec<SrtpProtectionProfile>) -> Config {
    Config {
        certificates: vec![identity.certificate],
        srtp_protection_profiles: srtp_profiles,
        extended_master_secret: ExtendedMasterSecretType::Require,
        // WebRTC/SDP's trust model is the `a=fingerprint` line, not a CA
        // chain: any certificate is acceptable during the handshake itself,
        // and the caller checks `remote_fingerprint_sha256` against the SDP
        // fingerprint afterward. This mirrors how every WebRTC stack
        // handles DTLS-SRTP certificates.
        insecure_skip_verify: true,
        // Unlike plain TLS, DTLS-SRTP requires a certificate from BOTH
        // sides (RFC 5763 §5): the client's certificate is what the
        // server's `a=fingerprint` check binds to. The default
        // `ClientAuthType` doesn't request one at all, which is right for
        // ordinary TLS but wrong here.
        client_auth: ClientAuthType::RequireAnyClientCert,
        ..Default::default()
    }
}

async fn finish_handshake(
    conn: DTLSConn,
    local_fingerprint: [u8; 32],
) -> Result<DtlsSrtpHandshakeResult> {
    let profile = conn.selected_srtpprotection_profile();
    let suite = convert_profile(profile)?;

    let state = conn.connection_state().await;

    let remote_cert = state.peer_certificates.first().ok_or_else(|| {
        Error::DtlsHandshakeError("peer completed the handshake without a certificate".to_string())
    })?;
    let remote_fingerprint = sha256_fingerprint(remote_cert);

    let key_length = suite.key_length;
    let total_length = (key_length + SRTP_SALT_LENGTH) * 2;
    let keying_material = state
        .export_keying_material("EXTRACTOR-dtls_srtp", &[], total_length)
        .await
        .map_err(|e| Error::DtlsHandshakeError(format!("keying material export failed: {e}")))?;

    // RFC 5764 §4.2: client key, server key, client salt, server salt, in
    // that order, packed back-to-back in the exported keying material.
    let mut offset = 0;
    let client_key = keying_material[offset..offset + key_length].to_vec();
    offset += key_length;
    let server_key = keying_material[offset..offset + key_length].to_vec();
    offset += key_length;
    let client_salt = keying_material[offset..offset + SRTP_SALT_LENGTH].to_vec();
    offset += SRTP_SALT_LENGTH;
    let server_salt = keying_material[offset..offset + SRTP_SALT_LENGTH].to_vec();

    Ok(DtlsSrtpHandshakeResult {
        profile: suite,
        client_write_key: SrtpCryptoKey::new(client_key, client_salt),
        server_write_key: SrtpCryptoKey::new(server_key, server_salt),
        remote_fingerprint_sha256: remote_fingerprint,
        local_fingerprint_sha256: local_fingerprint,
    })
}

/// Perform a DTLS-SRTP handshake as the client (the offerer side that sends
/// `a=setup:active`, or the answerer when it sends `a=setup:passive` and
/// therefore acts as the DTLS server; see RFC 4145/5763 for the
/// active/passive-to-client/server mapping), over any transport that
/// behaves like a connected point-to-point socket (a real connected
/// `UdpSocket`, or a demux bridge over a shared socket — see
/// `transport::dtls_bridge` for the latter).
pub async fn handshake_client(
    conn: Arc<dyn UtilConn + Send + Sync>,
    identity: DtlsIdentity,
    srtp_profiles: Vec<SrtpProtectionProfile>,
) -> Result<DtlsSrtpHandshakeResult> {
    let local_fingerprint = identity.fingerprint_sha256;
    let config = build_config(identity, srtp_profiles);
    let dtls_conn = DTLSConn::new(conn, config, true, None)
        .await
        .map_err(to_dtls_error)?;
    finish_handshake(dtls_conn, local_fingerprint).await
}

/// Perform a DTLS-SRTP handshake as the server (the passive/"listen") side,
/// over any transport that behaves like a connected point-to-point socket
/// — see [`handshake_client`].
pub async fn handshake_server(
    conn: Arc<dyn UtilConn + Send + Sync>,
    identity: DtlsIdentity,
    srtp_profiles: Vec<SrtpProtectionProfile>,
) -> Result<DtlsSrtpHandshakeResult> {
    let local_fingerprint = identity.fingerprint_sha256;
    let config = build_config(identity, srtp_profiles);
    let dtls_conn = DTLSConn::new(conn, config, false, None)
        .await
        .map_err(to_dtls_error)?;
    finish_handshake(dtls_conn, local_fingerprint).await
}
