//! SRTP/DTLS setup and security transport methods for MediaManager

use crate::api::types::SessionId;
use super::super::types::*;
use super::super::MediaError;
use std::sync::Arc;
use std::net::SocketAddr;
use tokio::sync::Mutex;
use super::MediaManager;
use super::super::MediaResult;
use super::super::srtp_bridge::SrtpMediaBridge;
use rvoip_rtp_core::dtls::DtlsRole;
use rvoip_rtp_core::dtls::adapter::SrtpKeyMaterial;
use rvoip_rtp_core::transport::{RtpTransport, RtpTransportConfig, UdpRtpTransport};
use rvoip_rtp_core::transport::security_transport::SecurityRtpTransport;
use rvoip_rtp_core::srtp::SrtpContext;

impl MediaManager {
    // -----------------------------------------------------------------------
    // SRTP bridge integration
    // -----------------------------------------------------------------------

    /// Set up an SRTP bridge for a session after SDP negotiation.
    ///
    /// Inspects the remote SDP for DTLS-SRTP indicators (`a=fingerprint`,
    /// `RTP/SAVP`, `a=setup`).  If secure media is required a bridge is
    /// created and stored.  The actual DTLS handshake is *not* started here
    /// -- call `perform_srtp_handshake` when the transport is ready.
    pub async fn setup_srtp_from_sdp(
        &self,
        session_id: &SessionId,
        remote_sdp: &str,
    ) -> super::super::MediaResult<bool> {
        let (srtp_required, remote_fingerprint, remote_role) =
            super::super::srtp_bridge::extract_dtls_params_from_sdp(remote_sdp);

        if !srtp_required {
            // If this session was previously negotiated with SRTP (e.g. initial
            // INVITE used DTLS-SRTP) but a re-INVITE now negotiates plain RTP,
            // we must remove the stale SRTP-required flag and clean up the old
            // bridge so that subsequent protect_rtp/unprotect_rtp calls allow
            // plain RTP through.
            let was_srtp = self.srtp_required_sessions.write().await.remove(session_id);
            if was_srtp {
                tracing::info!(
                    session = %session_id,
                    "Re-INVITE downgraded session from SRTP to plain RTP; \
                     removing SRTP requirement and cleaning up bridge/transport"
                );
                self.srtp_bridges.write().await.remove(session_id);
                self.security_transports.write().await.remove(session_id);
            } else {
                tracing::debug!(
                    "No DTLS-SRTP indicators in SDP for session {} -- plain RTP",
                    session_id
                );
            }
            return Ok(false);
        }

        // The *local* DTLS role is the inverse of the remote setup role:
        //   remote=actpass (offerer) -> we answer with active (client)
        //   remote=active           -> we are passive (server)
        //   remote=passive          -> we are active  (client)
        let local_role = match remote_role {
            Some(DtlsRole::Client) => DtlsRole::Server,
            Some(DtlsRole::Server) | None => DtlsRole::Client,
        };

        let bridge = SrtpMediaBridge::new(true, local_role, remote_fingerprint);

        tracing::info!(
            session = %session_id,
            role = ?local_role,
            "SRTP bridge created for session (DTLS handshake pending)"
        );

        // Record that SRTP was negotiated for this session so that
        // protect_rtp / unprotect_rtp refuse to fall back to plain RTP
        // if the bridge is later missing (RFC 5764 security requirement).
        self.srtp_required_sessions.write().await.insert(session_id.clone());

        let mut bridges = self.srtp_bridges.write().await;
        bridges.insert(session_id.clone(), Arc::new(Mutex::new(bridge)));

        Ok(true)
    }

    /// Drive the DTLS handshake for a session that has an SRTP bridge.
    ///
    /// This must be called *after* `setup_srtp_from_sdp` and *before* media
    /// starts flowing.  The `socket` should be the same UDP socket that will
    /// carry RTP/SRTP traffic.
    ///
    /// Returns the extracted `SrtpKeyMaterial` on success so callers can
    /// install keys into a `SecurityRtpTransport`.
    pub async fn perform_srtp_handshake(
        &self,
        session_id: &SessionId,
        socket: Arc<tokio::net::UdpSocket>,
        remote_addr: SocketAddr,
    ) -> super::super::MediaResult<Option<SrtpKeyMaterial>> {
        let bridge_arc = {
            let bridges = self.srtp_bridges.read().await;
            bridges.get(session_id).cloned()
        };

        let bridge_arc = match bridge_arc {
            Some(b) => b,
            None => {
                tracing::debug!(
                    "No SRTP bridge for session {} -- skipping handshake",
                    session_id
                );
                return Ok(None);
            }
        };

        let mut bridge = bridge_arc.lock().await;
        let keys = bridge.perform_dtls_handshake(socket, remote_addr).await?;

        tracing::info!(
            session = %session_id,
            "DTLS-SRTP handshake completed, SRTP keys installed in bridge"
        );
        Ok(keys)
    }

    /// Check whether a session has an active (post-handshake) SRTP bridge.
    pub async fn is_srtp_active(&self, session_id: &SessionId) -> bool {
        let bridges = self.srtp_bridges.read().await;
        if let Some(b) = bridges.get(session_id) {
            let bridge = b.lock().await;
            bridge.is_active()
        } else {
            false
        }
    }

    /// Retrieve the UDP socket handle for a session's RTP transport.
    ///
    /// Returns `None` when no RTP session exists for this session yet.
    pub async fn get_rtp_socket(
        &self,
        session_id: &SessionId,
    ) -> Option<Arc<tokio::net::UdpSocket>> {
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            mapping.get(session_id).cloned()
        };

        let dialog_id = dialog_id?;

        let rtp_session_arc = self.controller.get_rtp_session(&dialog_id).await?;
        let session = rtp_session_arc.lock().await;
        session.get_socket_handle().await.ok()
    }

    /// One-shot helper: set up an SRTP bridge from the remote SDP, create
    /// a `SecurityRtpTransport`, perform DTLS handshake, and install keys.
    ///
    /// This is the primary entry point that the coordinator should call
    /// after SDP negotiation completes.  It is a no-op when the remote SDP
    /// does not indicate DTLS-SRTP.
    ///
    /// When SRTP is needed, this method:
    ///   1. Creates the SRTP bridge from SDP parameters.
    ///   2. Stops the existing plain-RTP media session.
    ///   3. Creates a `UdpRtpTransport`, wraps it in `SecurityRtpTransport`.
    ///   4. Restarts the media session via `start_media_with_transport`.
    ///   5. Performs the DTLS handshake using the transport's socket.
    ///   6. Installs the derived SRTP keys into `SecurityRtpTransport`.
    ///
    /// `remote_addr` is the far-end RTP address (parsed from SDP).
    pub async fn initiate_srtp_for_session(
        &self,
        session_id: &SessionId,
        remote_sdp: &str,
        remote_addr: SocketAddr,
    ) -> super::super::MediaResult<()> {
        // Step 1 -- inspect SDP and create the bridge (no-op if plain RTP).
        let srtp_needed = self.setup_srtp_from_sdp(session_id, remote_sdp).await?;
        if !srtp_needed {
            return Ok(());
        }

        // Step 2 -- resolve dialog ID for media-core operations.
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            mapping.get(session_id).cloned()
        };
        let dialog_id = match dialog_id {
            Some(d) => d,
            None => {
                tracing::warn!(
                    session = %session_id,
                    "SRTP bridge created but no media session mapping yet -- \
                     handshake deferred until media session is ready"
                );
                return Ok(());
            }
        };

        // Step 3 -- stop existing plain-RTP media session so we can
        //           restart it with a SecurityRtpTransport.
        if let Err(e) = self.controller.stop_media(&dialog_id).await {
            tracing::debug!(
                session = %session_id,
                error = %e,
                "Could not stop existing media session (may not exist yet)"
            );
        }

        // Step 4 -- create UdpRtpTransport with the same local address.
        let transport_config = RtpTransportConfig {
            local_rtp_addr: self.local_bind_addr,
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some(format!("srtp-{}", session_id)),
            use_port_allocator: true,
        };

        let udp_transport = UdpRtpTransport::new(transport_config)
            .await
            .map_err(|e| MediaError::Configuration {
                message: format!("Failed to create UDP transport for SRTP: {e}"),
            })?;
        let udp_transport = Arc::new(udp_transport);

        // Step 5 -- wrap in SecurityRtpTransport.
        let security_transport = SecurityRtpTransport::new(udp_transport.clone(), true)
            .await
            .map_err(|e| MediaError::Configuration {
                message: format!("Failed to create SecurityRtpTransport: {e}"),
            })?;
        let security_transport = Arc::new(security_transport);

        // Store the SecurityRtpTransport for later key installation.
        self.security_transports.write().await
            .insert(session_id.clone(), security_transport.clone());

        // Step 6 -- restart media session with SecurityRtpTransport.
        let media_config = convert_to_media_core_config(
            &self.media_config,
            self.local_bind_addr,
            None,
        );

        self.controller.start_media_with_transport(
            dialog_id.clone(),
            media_config,
            security_transport.clone() as Arc<dyn RtpTransport>,
        ).await.map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;

        tracing::info!(
            session = %session_id,
            "Media session restarted with SecurityRtpTransport"
        );

        // Step 7 -- obtain the socket for DTLS handshake.
        let socket = udp_transport.get_socket();

        // Step 8 -- perform the DTLS handshake.
        tracing::info!(
            session = %session_id,
            remote = %remote_addr,
            "Initiating DTLS-SRTP handshake"
        );
        let keys = self.perform_srtp_handshake(session_id, socket, remote_addr).await?;

        // Step 9 -- install SRTP keys into SecurityRtpTransport.
        if let Some(key_material) = keys {
            let srtp_context = SrtpContext::from_dtls_key_material(&key_material)
                .map_err(|e| MediaError::Configuration {
                    message: format!("Failed to create SrtpContext from DTLS keys: {e}"),
                })?;

            security_transport.set_srtp_context(srtp_context).await;

            tracing::info!(
                session = %session_id,
                "SRTP keys installed in SecurityRtpTransport -- SRTP is active"
            );
        } else {
            tracing::warn!(
                session = %session_id,
                "DTLS handshake completed but no key material returned"
            );
        }

        Ok(())
    }

    /// Remove and clean up the SRTP bridge and security transport for a session.
    pub(crate) async fn cleanup_srtp_bridge(&self, session_id: &SessionId) {
        let mut bridges = self.srtp_bridges.write().await;
        if bridges.remove(session_id).is_some() {
            tracing::debug!("Cleaned up SRTP bridge for session {}", session_id);
        }
        self.srtp_required_sessions.write().await.remove(session_id);

        let mut transports = self.security_transports.write().await;
        if transports.remove(session_id).is_some() {
            tracing::debug!("Cleaned up SecurityRtpTransport for session {}", session_id);
        }
    }
}
