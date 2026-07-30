//! Client connection management
//!
//! This module handles client connection establishment, management, and disconnection.

use dashmap::DashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::{broadcast, Mutex, RwLock};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::api::common::error::MediaTransportError;
use crate::api::common::frame::MediaFrame;
use crate::api::server::config::ServerConfig;
use crate::api::server::security::ClientSecurityContext;
use crate::api::server::transport::ClientInfo;
use crate::session::{RtpSession, RtpSessionBufferConfig, RtpSessionConfig, RtpSessionEvent};
use crate::transport::{RtpTransportBufferConfig, UdpRtpTransport};
// payload registry moved to media-core

/// Client connection in the server
#[allow(dead_code)] // retained (liveness/Drop hold or reserved); not read
pub struct ClientConnection {
    /// Client ID
    pub(crate) id: String,
    /// Remote address
    pub(crate) address: SocketAddr,
    /// RTP session for this client
    pub(crate) session: Arc<Mutex<RtpSession>>,
    /// Security context for this client
    pub(crate) security: Option<Arc<dyn ClientSecurityContext + Send + Sync>>,
    /// Task handle for packet forwarding
    pub(crate) task: Option<JoinHandle<()>>,
    /// Is connected
    pub(crate) connected: bool,
    /// Creation time
    #[allow(dead_code)] // retained (liveness/Drop hold or reserved); not read
    pub(crate) created_at: SystemTime,
    /// Last activity time
    #[allow(dead_code)] // retained (liveness/Drop hold or reserved); not read
    pub(crate) last_activity: Arc<Mutex<SystemTime>>,
}

/// Static helper function to handle a new client connection
#[allow(dead_code)] // public compatibility entry point; the server uses the secure-aware helper
pub async fn handle_client_static(
    addr: SocketAddr,
    clients: &Arc<DashMap<String, ClientConnection>>,
    frame_sender: &broadcast::Sender<(String, MediaFrame)>,
    session_buffer_config: RtpSessionBufferConfig,
    transport_buffer_config: RtpTransportBufferConfig,
) -> Result<String, crate::api::common::error::MediaTransportError> {
    handle_client_static_with_security_requirement(
        addr,
        clients,
        frame_sender,
        session_buffer_config,
        transport_buffer_config,
        false,
        None,
    )
    .await
}

pub(crate) async fn handle_client_static_with_security_requirement(
    addr: SocketAddr,
    clients: &Arc<DashMap<String, ClientConnection>>,
    frame_sender: &broadcast::Sender<(String, MediaFrame)>,
    session_buffer_config: RtpSessionBufferConfig,
    transport_buffer_config: RtpTransportBufferConfig,
    secure_media_required: bool,
    pre_shared_srtp: Option<(crate::srtp::SrtpCryptoSuite, Vec<u8>)>,
) -> Result<String, crate::api::common::error::MediaTransportError> {
    info!("Handling new client from {}", addr);

    let client_id = format!("client-{}", Uuid::new_v4());
    debug!("Assigned client ID: {}", client_id);

    // Create RTP session config for this client - bind to 0.0.0.0:0 to let OS choose a port
    let session_config = RtpSessionConfig {
        local_addr: "0.0.0.0:0".parse().unwrap(),
        remote_addr: Some(addr),
        ssrc: Some(rand::random()),
        payload_type: 8,                       // Default payload type
        clock_rate: 8000,                      // Default clock rate
        jitter_buffer_size: Some(50 as usize), // Default buffer size
        max_packet_age_ms: Some(200),          // Default max packet age
        enable_jitter_buffer: true,
        session_buffer_config,
        transport_buffer_config,
    };

    // Create RTP session
    debug!("Creating RTP session for client {}", client_id);
    let rtp_session = RtpSession::new(session_config).await.map_err(|e| {
        MediaTransportError::Transport(format!("Failed to create client RTP session: {}", e))
    })?;
    if secure_media_required {
        let transport = rtp_session.transport();
        let udp = transport
            .as_any()
            .downcast_ref::<UdpRtpTransport>()
            .ok_or_else(|| {
                MediaTransportError::Security(
                    "secure server session is not backed by UDP transport".to_string(),
                )
            })?;
        udp.require_srtp();
        if let Some((crypto_suite, combined_key)) = pre_shared_srtp {
            let expected_length = crypto_suite.key_length + 14;
            if combined_key.len() != expected_length {
                return Err(MediaTransportError::Security(format!(
                    "SRTP key material for {crypto_suite:?} must be exactly {expected_length} bytes, got {}",
                    combined_key.len(),
                )));
            }
            let key = combined_key[..crypto_suite.key_length].to_vec();
            let salt = combined_key[crypto_suite.key_length..].to_vec();
            udp.set_srtp_contexts(
                crate::srtp::SrtpContext::new(
                    crypto_suite.clone(),
                    crate::srtp::SrtpCryptoKey::new(key.clone(), salt.clone()),
                )
                .map_err(|error| MediaTransportError::Security(error.to_string()))?,
                crate::srtp::SrtpContext::new(
                    crypto_suite,
                    crate::srtp::SrtpCryptoKey::new(key, salt),
                )
                .map_err(|error| MediaTransportError::Security(error.to_string()))?,
            )
            .await
            .map_err(|error| MediaTransportError::Security(error.to_string()))?;
        }
    }

    let rtp_session = Arc::new(Mutex::new(rtp_session));

    // Create client connection without security for now (will be added later)
    let client = ClientConnection {
        id: client_id.clone(),
        address: addr,
        session: rtp_session,
        security: None,
        task: None,
        connected: true,
        created_at: SystemTime::now(),
        last_activity: Arc::new(Mutex::new(SystemTime::now())),
    };

    // Start a task to forward frames from this client
    let frame_sender_clone = frame_sender.clone();
    let client_id_clone = client_id.clone();
    let session_clone = client.session.clone();

    debug!("Starting packet forwarding task for client {}", client_id);
    let forward_task = tokio::spawn(async move {
        let session = session_clone.lock().await;

        // Get session details for debugging
        debug!(
            "Session details - SSRC: {}, Target: {}",
            session.get_ssrc(),
            addr
        );

        let mut event_rx = session.subscribe();
        drop(session);

        debug!(
            "Starting packet receive loop for client {}",
            client_id_clone
        );
        let mut packets_received = 0;

        while let Ok(event) = event_rx.recv().await {
            match event {
                RtpSessionEvent::PacketReceived(packet) => {
                    packets_received += 1;

                    // Determine frame type from payload type
                    let frame_type = crate::api::common::frame::MediaFrameType::Audio; // Default to Audio, media-core handles frame type

                    // Log packet details
                    debug!(
                        "Client {}: Received packet #{} - PT: {}, Seq: {}, TS: {}, Size: {} bytes",
                        client_id_clone,
                        packets_received,
                        packet.header.payload_type,
                        packet.header.sequence_number,
                        packet.header.timestamp,
                        packet.payload.len()
                    );

                    // Convert to MediaFrame
                    let frame = MediaFrame {
                        frame_type,
                        data: packet.payload,
                        timestamp: packet.header.timestamp,
                        sequence: packet.header.sequence_number,
                        marker: packet.header.marker,
                        payload_type: packet.header.payload_type,
                        ssrc: packet.header.ssrc,
                        csrcs: packet.header.csrc.clone(),
                    };

                    // Forward to server via broadcast channel
                    match frame_sender_clone.send((client_id_clone.clone(), frame)) {
                        Ok(receiver_count) => {
                            debug!(
                                "Broadcast packet to {} receivers - Client: {}, Seq: {}",
                                receiver_count, client_id_clone, packet.header.sequence_number
                            );
                        }
                        Err(e) => {
                            // This is expected if no subscribers are listening
                            debug!(
                                "No receivers for frame from client {}: {}",
                                client_id_clone, e
                            );
                        }
                    }
                }
                other_event => {
                    debug!(
                        "Client {}: Received non-packet event: {:?}",
                        client_id_clone, other_event
                    );
                }
            }
        }

        debug!(
            "Packet forwarding task ended for client {}",
            client_id_clone
        );
    });

    // Update the client with the task
    let mut client_with_task = client;
    client_with_task.task = Some(forward_task);

    // Add to clients (DashMap insert is sharded).
    debug!("Adding client {} to clients map", client_id);
    clients.insert(client_id.clone(), client_with_task);

    info!("Successfully added client {}", client_id);
    Ok(client_id)
}

/// Disconnect a client
pub async fn disconnect_client(
    client_id: &str,
    clients: &Arc<DashMap<String, ClientConnection>>,
    client_disconnected_callbacks: &Arc<RwLock<Vec<Box<dyn Fn(ClientInfo) + Send + Sync>>>>,
) -> Result<(), MediaTransportError> {
    // Remove client from the shard. The returned `client` is owned —
    // shard guard is released by the `remove` call returning, so all
    // subsequent `.await` calls are safe.
    let mut client = clients.remove(client_id).map(|(_, c)| c).ok_or_else(|| {
        MediaTransportError::Transport(format!("Client not found: {}", client_id))
    })?;

    // Abort task
    if let Some(task) = client.task.take() {
        task.abort();
    }

    // Close session
    {
        let mut session = client.session.lock().await;
        if let Err(e) = session.close().await {
            warn!("Error closing client session {}: {}", client_id, e);
        }
    }

    // Close security context if it exists
    if let Some(security_ctx) = &client.security {
        if let Err(e) = security_ctx.close().await {
            warn!("Error closing client security {}: {}", client_id, e);
        }
    }

    // Notify callbacks
    let callbacks_guard = client_disconnected_callbacks.read().await;
    let client_info = ClientInfo {
        id: client.id.clone(),
        address: client.address,
        secure: client.security.is_some(),
        security_info: None,
        connected: false,
    };

    for callback in &*callbacks_guard {
        callback(client_info.clone());
    }

    Ok(())
}

/// Get client information
pub async fn get_clients_info(
    clients: &Arc<DashMap<String, ClientConnection>>,
    config: &ServerConfig,
) -> Result<Vec<ClientInfo>, MediaTransportError> {
    // Snapshot the per-client primitives (id, addr, connected,
    // security) out of the DashMap before any `.await`. Holding a
    // DashMap iter guard across `security_ctx.get_remote_fingerprint()
    // .await` would taint the surrounding future (shard `Ref` is
    // `!Send`).
    let snapshot: Vec<(
        String,
        SocketAddr,
        bool,
        Option<Arc<dyn ClientSecurityContext + Send + Sync>>,
    )> = clients
        .iter()
        .map(|e| {
            let v = e.value();
            (e.key().clone(), v.address, v.connected, v.security.clone())
        })
        .collect();

    let mut result = Vec::with_capacity(snapshot.len());
    for (id, address, connected, security) in snapshot {
        let security_info = if let Some(security_ctx) = &security {
            let fingerprint = security_ctx.get_remote_fingerprint().await.ok().flatten();
            let context_info = security_ctx.get_security_info();
            Some(crate::api::common::config::SecurityInfo {
                mode: config.security_config.security_mode,
                fingerprint,
                fingerprint_algorithm: (config.security_config.security_mode
                    == crate::api::common::config::SecurityMode::DtlsSrtp)
                    .then(|| config.security_config.fingerprint_algorithm.clone()),
                crypto_suites: context_info.crypto_suites,
                key_params: context_info.key_params,
                srtp_profile: context_info.srtp_profile,
            })
        } else {
            None
        };

        result.push(ClientInfo {
            id,
            address,
            secure: security.is_some(),
            security_info,
            connected,
        });
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn direct_secure_server_session_never_emits_plaintext_rtcp() {
        let peer = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let clients = Arc::new(DashMap::new());
        let (frame_sender, _) = broadcast::channel(8);

        let client_id = handle_client_static_with_security_requirement(
            peer.local_addr().unwrap(),
            &clients,
            &frame_sender,
            RtpSessionBufferConfig::default(),
            RtpTransportBufferConfig::default(),
            true,
            None,
        )
        .await
        .unwrap();
        let session = clients.get(&client_id).unwrap().session.clone();

        assert!(matches!(
            session.lock().await.send_sender_report().await,
            Err(crate::Error::InvalidState(_))
        ));
        assert!(matches!(
            session.lock().await.send_receiver_report().await,
            Err(crate::Error::InvalidState(_))
        ));

        let mut wire = [0u8; 2048];
        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(200),
            peer.recv_from(&mut wire)
        )
        .await
        .is_err());

        let (_, mut client) = clients.remove(&client_id).unwrap();
        if let Some(task) = client.task.take() {
            task.abort();
        }
        client.session.lock().await.close().await.unwrap();
    }

    #[tokio::test]
    async fn direct_secure_server_session_emits_authenticated_srtcp() {
        let peer = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let clients = Arc::new(DashMap::new());
        let (frame_sender, _) = broadcast::channel(8);

        let combined_key = [vec![0x42; 16], vec![0x37; 14]].concat();
        let client_id = handle_client_static_with_security_requirement(
            peer.local_addr().unwrap(),
            &clients,
            &frame_sender,
            RtpSessionBufferConfig::default(),
            RtpTransportBufferConfig::default(),
            true,
            Some((crate::srtp::SRTP_AES128_CM_SHA1_80, combined_key)),
        )
        .await
        .unwrap();
        let session = clients.get(&client_id).unwrap().session.clone();
        let key = crate::srtp::SrtpCryptoKey::new(vec![0x42; 16], vec![0x37; 14]);
        let suite = crate::srtp::SRTP_AES128_CM_SHA1_80;
        let mut peer_receive = crate::srtp::SrtpContext::new(suite.clone(), key.clone()).unwrap();

        {
            let session = session.lock().await;
            session.send_sender_report().await.unwrap();
        }

        let mut wire = [0u8; 2048];
        let (length, _) =
            tokio::time::timeout(std::time::Duration::from_secs(1), peer.recv_from(&mut wire))
                .await
                .unwrap()
                .unwrap();
        let plaintext = peer_receive.unprotect_rtcp(&wire[..length]).unwrap();
        assert!(matches!(
            crate::packet::rtcp::RtcpPacket::parse(&plaintext).unwrap(),
            crate::packet::rtcp::RtcpPacket::SenderReport(_)
        ));
        assert_ne!(&wire[..length], plaintext.as_ref());

        let (_, mut client) = clients.remove(&client_id).unwrap();
        if let Some(task) = client.task.take() {
            task.abort();
        }
        client.session.lock().await.close().await.unwrap();
    }
}
