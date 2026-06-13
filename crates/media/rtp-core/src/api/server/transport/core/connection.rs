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
use crate::transport::RtpTransportBufferConfig;
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
pub async fn handle_client_static(
    addr: SocketAddr,
    clients: &Arc<DashMap<String, ClientConnection>>,
    frame_sender: &broadcast::Sender<(String, MediaFrame)>,
    session_buffer_config: RtpSessionBufferConfig,
    transport_buffer_config: RtpTransportBufferConfig,
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

            if let Some(fingerprint) = fingerprint {
                Some(crate::api::common::config::SecurityInfo {
                    mode: config.security_config.security_mode,
                    fingerprint: Some(fingerprint),
                    fingerprint_algorithm: Some(
                        config.security_config.fingerprint_algorithm.clone(),
                    ),
                    crypto_suites: security_ctx.get_security_info().crypto_suites.clone(),
                    key_params: None,
                    srtp_profile: Some("AES_CM_128_HMAC_SHA1_80".to_string()),
                })
            } else {
                None
            }
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
