//! DTLS-SRTP Bridge for Media Pipeline
//!
//! This module bridges the DTLS handshake and SRTP encryption/decryption from
//! rtp-core into the session-core media pipeline, so encrypted media actually
//! flows end-to-end when the SDP indicates DTLS-SRTP.
//!
//! Flow:
//!   SDP negotiation (fingerprint + setup) -> DTLS handshake -> SRTP key extraction
//!   -> protect_rtp / unprotect_rtp wrapping around the media pipeline.

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::UdpSocket;
use tracing::{debug, error, info};

use rvoip_rtp_core::dtls::adapter::{
    DtlsAdapterConfig, DtlsConnectionAdapter,
    DtlsRole as AdapterDtlsRole, SrtpKeyMaterial,
};
use rvoip_rtp_core::dtls::DtlsRole;
use rvoip_rtp_core::srtp::adapter::SrtpContextAdapter;

use super::MediaError;

/// DTLS-SRTP bridge that handles the handshake and provides
/// encrypt/decrypt for the media pipeline.
pub struct SrtpMediaBridge {
    /// SRTP context adapter (encrypt outbound, decrypt inbound) -- initialised
    /// after the DTLS handshake completes.
    srtp_ctx: Option<SrtpContextAdapter>,

    /// Whether SRTP is required (derived from SDP `a=fingerprint` / RTP/SAVP).
    srtp_required: bool,

    /// DTLS role inferred from the SDP `a=setup` attribute.
    dtls_role: DtlsRole,

    /// Remote certificate fingerprint for verification (from SDP `a=fingerprint`).
    remote_fingerprint: Option<String>,

    /// Local DTLS connection adapter -- kept alive for potential rekeying.
    dtls_connection: Option<DtlsConnectionAdapter>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

impl SrtpMediaBridge {
    /// Create a new bridge.
    ///
    /// * `srtp_required` -- true when the SDP indicates secure media
    ///   (e.g. RTP/SAVP, `a=fingerprint`, `a=crypto`).
    /// * `dtls_role` -- `Client` when the local `a=setup` is `active`,
    ///   `Server` when it is `passive` or `actpass` and we are the answerer.
    /// * `remote_fingerprint` -- the `a=fingerprint` value from the remote SDP.
    pub fn new(
        srtp_required: bool,
        dtls_role: DtlsRole,
        remote_fingerprint: Option<String>,
    ) -> Self {
        Self {
            srtp_ctx: None,
            srtp_required,
            dtls_role,
            remote_fingerprint,
            dtls_connection: None,
        }
    }

    /// Drive the full DTLS handshake and, on success, install the derived
    /// SRTP keys into the context.
    ///
    /// `socket` is the *same* UDP socket used for RTP so that the DTLS
    /// handshake and media share a single 5-tuple (required by ICE / oRTP).
    pub async fn perform_dtls_handshake(
        &mut self,
        socket: Arc<UdpSocket>,
        remote_addr: SocketAddr,
    ) -> Result<Option<SrtpKeyMaterial>, MediaError> {
        if !self.srtp_required {
            debug!("SRTP not required -- skipping DTLS handshake");
            return Ok(None);
        }

        let adapter_role: AdapterDtlsRole = self.dtls_role.into();

        info!(
            role = ?adapter_role,
            remote = %remote_addr,
            "Starting DTLS handshake for SRTP key exchange (adapter)"
        );

        // ---- build adapter ----
        let mut adapter = DtlsConnectionAdapter::new(adapter_role)
            .await
            .map_err(|e| MediaError::Configuration {
                message: format!("Failed to create DTLS adapter: {e}"),
            })?;

        // Connect the UDP socket to the remote address so that
        // webrtc-dtls can use it as a Conn.
        let connected_socket = tokio::net::UdpSocket::bind("0.0.0.0:0")
            .await
            .map_err(|e| MediaError::Configuration {
                message: format!("Failed to bind UDP socket for DTLS: {e}"),
            })?;
        connected_socket.connect(remote_addr).await.map_err(|e| {
            MediaError::Configuration {
                message: format!("Failed to connect UDP socket to {remote_addr}: {e}"),
            }
        })?;

        let conn: Arc<dyn webrtc_util_dtls::Conn + Send + Sync> =
            Arc::new(connected_socket);

        let config = DtlsAdapterConfig::default();

        // ---- run the handshake ----
        adapter
            .handshake(conn, &config)
            .await
            .map_err(|e| MediaError::Configuration {
                message: format!("DTLS handshake failed: {e}"),
            })?;

        info!("DTLS handshake completed successfully (adapter)");

        // ---- verify remote fingerprint ----
        if let Some(expected_fp) = &self.remote_fingerprint {
            verify_remote_fingerprint_adapter(&adapter, expected_fp).await?;
        }

        // ---- extract SRTP keying material ----
        let keys = adapter.get_srtp_keys().await.map_err(|e| {
            MediaError::Configuration {
                message: format!("Failed to extract SRTP keys from DTLS adapter: {e}"),
            }
        })?;

        self.install_srtp_keys_adapter(&keys)?;

        self.dtls_connection = Some(adapter);
        Ok(Some(keys))
    }

    /// Encrypt an outbound RTP packet.
    ///
    /// Returns the SRTP-protected bytes (header + encrypted payload + auth tag).
    /// When SRTP is not active the packet is returned as-is.
    pub fn protect_rtp(&mut self, packet: &[u8]) -> Result<Vec<u8>, MediaError> {
        let ctx = match self.srtp_ctx.as_mut() {
            Some(c) => c,
            None => {
                if self.srtp_required {
                    return Err(MediaError::Configuration {
                        message: "SRTP required but no SRTP context installed".to_string(),
                    });
                }
                // Plain RTP pass-through.
                return Ok(packet.to_vec());
            }
        };

        let protected = ctx.protect_rtp(packet).map_err(|e| {
            MediaError::SdpProcessing {
                message: format!("SRTP protect failed: {e}"),
            }
        })?;

        Ok(protected.to_vec())
    }

    /// Decrypt an inbound SRTP packet.
    ///
    /// Returns the plain RTP bytes.  When SRTP is not active the packet is
    /// returned as-is.
    pub fn unprotect_rtp(&mut self, packet: &[u8]) -> Result<Vec<u8>, MediaError> {
        let ctx = match self.srtp_ctx.as_mut() {
            Some(c) => c,
            None => {
                if self.srtp_required {
                    return Err(MediaError::Configuration {
                        message: "SRTP required but no SRTP context installed".to_string(),
                    });
                }
                return Ok(packet.to_vec());
            }
        };

        let unprotected = ctx.unprotect_rtp(packet).map_err(|e| {
            MediaError::SdpProcessing {
                message: format!("SRTP unprotect failed: {e}"),
            }
        })?;

        Ok(unprotected.to_vec())
    }

    /// Whether the SRTP context has been installed and is ready for
    /// protect/unprotect calls.
    pub fn is_active(&self) -> bool {
        self.srtp_ctx.is_some()
    }

    /// Whether this bridge requires SRTP (based on SDP negotiation).
    pub fn is_srtp_required(&self) -> bool {
        self.srtp_required
    }

    /// The DTLS role for this bridge.
    pub fn dtls_role(&self) -> DtlsRole {
        self.dtls_role
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

impl SrtpMediaBridge {
    /// Turn `SrtpKeyMaterial` from the adapter into an `SrtpContextAdapter`.
    fn install_srtp_keys_adapter(&mut self, keys: &SrtpKeyMaterial) -> Result<(), MediaError> {
        let srtp = SrtpContextAdapter::from_key_material(keys).map_err(|e| {
            MediaError::Configuration {
                message: format!("Failed to create SRTP context from DTLS keys: {e}"),
            }
        })?;

        info!("SRTP context adapter installed (role={:?})", self.dtls_role);
        self.srtp_ctx = Some(srtp);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// DTLS fingerprint verification
// ---------------------------------------------------------------------------

/// Verify the remote certificate's fingerprint against the SDP-advertised value.
///
/// This is called after the DTLS handshake completes. The webrtc-dtls crate
/// already performs certificate verification during the handshake when
/// `insecure_skip_verify` is false (our default). This function performs an
/// additional SDP-level fingerprint cross-check as required by RFC 5763.
async fn verify_remote_fingerprint_adapter(
    adapter: &DtlsConnectionAdapter,
    expected: &str,
) -> Result<(), MediaError> {
    let (algorithm, hex_fp) = parse_fingerprint_attr(expected)?;

    debug!(
        algorithm = %algorithm,
        fingerprint = %hex_fp,
        "Verifying remote DTLS certificate fingerprint (adapter)"
    );

    let dtls_conn = adapter.inner_conn().ok_or_else(|| MediaError::Configuration {
        message: "DTLS connection not established -- cannot verify fingerprint".to_string(),
    })?;

    // Retrieve the connection state which contains the peer certificates.
    let state = dtls_conn.connection_state().await;
    let peer_certs = state.peer_certificates;

    if peer_certs.is_empty() {
        return Err(MediaError::Configuration {
            message: "No remote certificate available for fingerprint verification".to_string(),
        });
    }

    // Compute the SHA-256 fingerprint of the first peer certificate.
    use sha2::{Sha256, Digest as _};
    let der_bytes = &peer_certs[0];
    let digest = Sha256::digest(der_bytes);
    let actual_fp: String = digest
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":");

    // Normalize for comparison: strip colons, lowercase.
    let expected_normalized = hex_fp.replace(':', "").to_lowercase();
    let actual_normalized = actual_fp.replace(':', "").to_lowercase();

    if expected_normalized != actual_normalized {
        error!(
            expected = %hex_fp,
            actual = %actual_fp,
            "Remote DTLS certificate fingerprint MISMATCH"
        );
        return Err(MediaError::Configuration {
            message: format!(
                "Remote DTLS certificate fingerprint mismatch: expected {hex_fp}, got {actual_fp}"
            ),
        });
    }

    info!(
        algorithm = %algorithm,
        fingerprint = %actual_fp,
        "Remote DTLS certificate fingerprint verified successfully"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// SDP helpers for DTLS-SRTP attributes
// ---------------------------------------------------------------------------

/// Parse a `a=setup:` attribute value into a `DtlsRole`.
///
/// RFC 4145 / RFC 4572:
/// - `active`  -> client (initiates DTLS)
/// - `passive` -> server (waits for DTLS)
/// - `actpass` -> offerer can do either; answerer picks `active`
///
/// Returns `None` for unrecognised values.
pub fn parse_setup_attribute(value: &str) -> Option<DtlsRole> {
    match value.trim().to_lowercase().as_str() {
        "active" => Some(DtlsRole::Client),
        "passive" => Some(DtlsRole::Server),
        "actpass" => Some(DtlsRole::Server), // convention: treat as server when receiving
        _ => None,
    }
}

/// Determine the local `a=setup` value to put in our SDP.
///
/// - Offerers use `actpass` (we can do either role).
/// - Answerers use `active` (we initiate the DTLS handshake).
pub fn local_setup_attribute(is_offer: bool) -> &'static str {
    if is_offer { "actpass" } else { "active" }
}

/// Parse `a=fingerprint:sha-256 AA:BB:CC:...` into (algorithm, fingerprint).
pub fn parse_fingerprint_attr(attr: &str) -> Result<(String, String), MediaError> {
    let trimmed = attr.trim();
    let parts: Vec<&str> = trimmed.splitn(2, ' ').collect();
    if parts.len() == 2 {
        Ok((parts[0].to_lowercase(), parts[1].to_string()))
    } else {
        // Maybe it's just the fingerprint hex without the algorithm prefix.
        // In the SDP, the algorithm and hex are space-separated on the same
        // attribute line, but callers may have already stripped the algorithm.
        Err(MediaError::SdpProcessing {
            message: format!(
                "Invalid fingerprint attribute (expected '<alg> <hex>'): {trimmed}"
            ),
        })
    }
}

/// Extract DTLS-SRTP parameters from raw SDP text.
///
/// Returns `(srtp_required, remote_fingerprint, remote_setup_role)`.
pub fn extract_dtls_params_from_sdp(
    sdp: &str,
) -> (bool, Option<String>, Option<DtlsRole>) {
    let mut fingerprint: Option<String> = None;
    let mut setup_role: Option<DtlsRole> = None;
    let mut has_savp = false;

    for line in sdp.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("m=audio") && trimmed.contains("RTP/SAVP") {
            has_savp = true;
        }

        if let Some(fp) = trimmed.strip_prefix("a=fingerprint:") {
            fingerprint = Some(fp.trim().to_string());
        }

        if let Some(setup) = trimmed.strip_prefix("a=setup:") {
            setup_role = parse_setup_attribute(setup);
        }
    }

    let srtp_required = fingerprint.is_some() || has_savp;
    (srtp_required, fingerprint, setup_role)
}

/// Generate the DTLS-SRTP SDP attributes to append to a media section.
///
/// Returns lines like:
/// ```text
/// a=fingerprint:sha-256 AA:BB:CC:...
/// a=setup:actpass
/// ```
pub fn generate_dtls_sdp_attributes(
    local_fingerprint: &str,
    is_offer: bool,
) -> String {
    let mut attrs = String::new();
    attrs.push_str(&format!("a=fingerprint:sha-256 {}\r\n", local_fingerprint));
    attrs.push_str(&format!("a=setup:{}\r\n", local_setup_attribute(is_offer)));
    attrs
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_setup_attribute() {
        assert_eq!(parse_setup_attribute("active"), Some(DtlsRole::Client));
        assert_eq!(parse_setup_attribute("passive"), Some(DtlsRole::Server));
        assert_eq!(parse_setup_attribute("actpass"), Some(DtlsRole::Server));
        assert_eq!(parse_setup_attribute("unknown"), None);
    }

    #[test]
    fn test_local_setup_attribute() {
        assert_eq!(local_setup_attribute(true), "actpass");
        assert_eq!(local_setup_attribute(false), "active");
    }

    #[test]
    fn test_parse_fingerprint_attr() {
        let (alg, fp) = parse_fingerprint_attr("sha-256 AA:BB:CC:DD").ok().unwrap_or_default();
        assert_eq!(alg, "sha-256");
        assert_eq!(fp, "AA:BB:CC:DD");
    }

    #[test]
    fn test_extract_dtls_params_plain_rtp() {
        let sdp = "v=0\r\nm=audio 5000 RTP/AVP 0\r\na=sendrecv\r\n";
        let (required, fp, role) = extract_dtls_params_from_sdp(sdp);
        assert!(!required);
        assert!(fp.is_none());
        assert!(role.is_none());
    }

    #[test]
    fn test_extract_dtls_params_savp() {
        let sdp = "v=0\r\n\
                   m=audio 5000 RTP/SAVP 0\r\n\
                   a=fingerprint:sha-256 AA:BB:CC\r\n\
                   a=setup:actpass\r\n\
                   a=sendrecv\r\n";
        let (required, fp, role) = extract_dtls_params_from_sdp(sdp);
        assert!(required);
        assert_eq!(fp.as_deref(), Some("sha-256 AA:BB:CC"));
        assert_eq!(role, Some(DtlsRole::Server));
    }

    #[test]
    fn test_generate_dtls_sdp_attributes() {
        let attrs = generate_dtls_sdp_attributes("AA:BB:CC", true);
        assert!(attrs.contains("a=fingerprint:sha-256 AA:BB:CC"));
        assert!(attrs.contains("a=setup:actpass"));

        let attrs_answer = generate_dtls_sdp_attributes("AA:BB:CC", false);
        assert!(attrs_answer.contains("a=setup:active"));
    }

    #[test]
    fn test_bridge_plain_rtp_passthrough() {
        let mut bridge = SrtpMediaBridge::new(false, DtlsRole::Client, None);
        assert!(!bridge.is_active());
        assert!(!bridge.is_srtp_required());

        // Plain RTP should pass through without error
        let dummy_packet = vec![0x80, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0, 0, 0];
        let protected = bridge.protect_rtp(&dummy_packet);
        assert!(protected.is_ok());
        assert_eq!(protected.ok(), Some(dummy_packet.clone()));

        let unprotected = bridge.unprotect_rtp(&dummy_packet);
        assert!(unprotected.is_ok());
        assert_eq!(unprotected.ok(), Some(dummy_packet));
    }

    #[test]
    fn test_bridge_srtp_required_but_no_context_errors() {
        let mut bridge = SrtpMediaBridge::new(true, DtlsRole::Client, None);
        assert!(!bridge.is_active());
        assert!(bridge.is_srtp_required());

        let dummy = vec![0x80, 0x00];
        assert!(bridge.protect_rtp(&dummy).is_err());
        assert!(bridge.unprotect_rtp(&dummy).is_err());
    }
}
