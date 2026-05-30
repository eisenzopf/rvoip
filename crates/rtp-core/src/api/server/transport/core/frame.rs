//! Frame processing
//!
//! This module handles frame sending, receiving, and broadcasting functionality.

use bytes::Bytes;
use dashmap::DashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex, RwLock};
use tracing::{debug, warn};

use crate::api::common::error::MediaTransportError;
use crate::api::common::frame::MediaFrame;
use crate::api::server::transport::core::connection::ClientConnection;
use crate::packet::RtpPacket;
use crate::session::RtpSession;
use crate::transport::RtpTransport;
use crate::{CsrcManager, MAX_CSRC_COUNT};

/// Send a media frame to a specific client
pub async fn send_frame_to(
    client_id: &str,
    frame: MediaFrame,
    clients: &Arc<DashMap<String, ClientConnection>>,
    ssrc_demultiplexing_enabled: &Arc<AtomicBool>,
    csrc_management_enabled: &Arc<AtomicBool>,
    csrc_manager: &Arc<parking_lot::RwLock<CsrcManager>>,
    main_socket: &Arc<RwLock<Option<Arc<dyn RtpTransport>>>>,
) -> Result<(), MediaTransportError> {
    // Extract the addr + session Arc out of the clients map, then
    // drop the shard guard. With DashMap the shard guard is even
    // more important to drop quickly — it's `!Send` and would taint
    // the surrounding future.
    let (addr, session) = {
        let client = clients
            .get(client_id)
            .ok_or_else(|| MediaTransportError::ClientNotFound(client_id.to_string()))?;
        if !client.connected {
            return Err(MediaTransportError::ClientNotConnected(
                client_id.to_string(),
            ));
        }
        (client.address, client.session.clone())
    };

    // Resolve the egress SSRC. Skip the session lock entirely when
    // SSRC demultiplexing supplies a per-frame SSRC.
    let demux_enabled = ssrc_demultiplexing_enabled.load(Ordering::Relaxed);
    let ssrc = if demux_enabled && frame.ssrc != 0 {
        frame.ssrc
    } else {
        let session_guard = session.lock().await;
        session_guard.get_ssrc()
    };

    // Create RTP header
    let mut header =
        crate::packet::RtpHeader::new(frame.payload_type, frame.sequence, frame.timestamp, ssrc);

    // Set marker bit
    if frame.marker {
        header.marker = true;
    }

    // Add CSRCs if CSRC management is enabled
    if csrc_management_enabled.load(Ordering::Relaxed) {
        // Snapshot the active SSRCs into an owned Vec while we hold
        // the clients read guard, so we can drop it before locking the
        // CSRC manager + sending the packet. Avoids holding any RwLock
        // guard across `.await`.
        let active_ssrcs = collect_active_ssrcs(clients).await;

        if !active_ssrcs.is_empty() {
            // Get CSRC values from the manager
            let csrc_manager_guard = csrc_manager.read();
            let csrcs = csrc_manager_guard.get_active_csrcs(&active_ssrcs);

            // Take only up to MAX_CSRC_COUNT
            let csrcs = if csrcs.len() > MAX_CSRC_COUNT as usize {
                csrcs[0..MAX_CSRC_COUNT as usize].to_vec()
            } else {
                csrcs
            };

            // Add CSRCs to the header if we have any
            if !csrcs.is_empty() {
                debug!("Adding {} CSRCs to outgoing packet", csrcs.len());
                header.add_csrcs(&csrcs);
            }
        }
    }

    // Store frame data length before it's moved
    let data_len = frame.data.len();

    // Create RTP packet
    let packet = RtpPacket::new(header, Bytes::from(frame.data));

    // Snapshot the socket out from under main_socket's RwLock so we
    // don't hold the guard across the outbound send.
    let socket = {
        let socket_guard = main_socket.read().await;
        socket_guard
            .as_ref()
            .cloned()
            .ok_or_else(|| MediaTransportError::Transport("Server is not running".to_string()))?
    };

    // Send packet
    socket
        .send_rtp(&packet, addr)
        .await
        .map_err(|e| MediaTransportError::SendError(format!("Failed to send RTP packet: {}", e)))?;

    // We don't have update_sent_stats method in RtpSession, so we'll just log
    debug!(
        "Sent frame to client {}: PT={}, TS={}, SEQ={}, Size={} bytes",
        client_id, frame.payload_type, frame.timestamp, frame.sequence, data_len
    );

    Ok(())
}

/// Snapshot active SSRCs from every connected client. Extracts session
/// Arcs under the clients read guard, then drops the guard before
/// locking individual sessions — keeps us out of the
/// guard-held-across-session-lock pattern.
async fn collect_active_ssrcs(
    clients: &Arc<DashMap<String, ClientConnection>>,
) -> Vec<crate::RtpSsrc> {
    // Collect session Arcs by iterating the DashMap; shard guards
    // are dropped at the end of `iter()`'s scope.
    let sessions: Vec<Arc<Mutex<RtpSession>>> = clients
        .iter()
        .filter(|entry| entry.value().connected)
        .map(|entry| entry.value().session.clone())
        .collect();

    let mut active_ssrcs = Vec::with_capacity(sessions.len());
    for session in sessions {
        let s = session.lock().await;
        active_ssrcs.push(s.get_ssrc());
    }
    active_ssrcs
}

/// Broadcast a media frame to all connected clients
pub async fn broadcast_frame(
    frame: MediaFrame,
    clients: &Arc<DashMap<String, ClientConnection>>,
    csrc_management_enabled: &Arc<AtomicBool>,
    csrc_manager: &Arc<parking_lot::RwLock<CsrcManager>>,
    main_socket: &Arc<RwLock<Option<Arc<dyn RtpTransport>>>>,
) -> Result<(), MediaTransportError> {
    // Create a base header with frame info
    let mut base_header = crate::packet::RtpHeader::new(
        frame.payload_type,
        frame.sequence,
        frame.timestamp,
        frame.ssrc,
    );

    // Set marker bit
    if frame.marker {
        base_header.marker = true;
    }

    // Add CSRCs if CSRC management is enabled
    if csrc_management_enabled.load(Ordering::Relaxed) {
        // Snapshot active SSRCs without holding the clients guard
        // across per-session locks — see `collect_active_ssrcs`.
        let active_ssrcs = collect_active_ssrcs(clients).await;

        if !active_ssrcs.is_empty() {
            // Get CSRC values from the manager
            let csrc_manager_guard = csrc_manager.read();
            let csrcs = csrc_manager_guard.get_active_csrcs(&active_ssrcs);

            // Take only up to MAX_CSRC_COUNT
            let csrcs = if csrcs.len() > MAX_CSRC_COUNT as usize {
                csrcs[0..MAX_CSRC_COUNT as usize].to_vec()
            } else {
                csrcs
            };

            // Add CSRCs to the header if we have any
            if !csrcs.is_empty() {
                debug!("Adding {} CSRCs to outgoing broadcast packet", csrcs.len());
                base_header.add_csrcs(&csrcs);
            }
        }
    }

    // Create RTP packet once with shared data
    let shared_data = Arc::new(Bytes::from(frame.data));

    // Get main socket
    let socket_guard = main_socket.read().await;
    let socket = socket_guard
        .as_ref()
        .ok_or_else(|| MediaTransportError::Transport("Server is not running".to_string()))?;

    // Snapshot (id, addr) for every connected client into an owned
    // Vec so we don't hold DashMap shard guards across the per-task
    // spawn or `.await` points.
    let targets: Vec<(String, SocketAddr)> = clients
        .iter()
        .filter(|e| e.value().connected)
        .map(|e| (e.key().clone(), e.value().address))
        .collect();

    // Send to each client (in parallel)
    let mut send_tasks = Vec::with_capacity(targets.len());

    for (client_id_clone, addr) in targets {
        // Clone header for each client
        let header = base_header.clone();

        // Clone data reference
        let data = shared_data.clone();

        // Create RTP packet
        let packet = crate::packet::RtpPacket::new(header, Bytes::clone(&data));

        // Clone socket reference
        let socket_clone = socket.clone();

        let payload_type = frame.payload_type;
        let data_len = data.len();

        // Spawn task to send packet and update stats
        let task = tokio::spawn(async move {
            // Send packet
            if let Err(e) = socket_clone.send_rtp(&packet, addr).await {
                warn!(
                    "Failed to send broadcast frame to client {}: {}",
                    client_id_clone, e
                );
                return;
            }

            // We don't have update_sent_stats method in RtpSession, so we'll just log
            debug!(
                "Sent broadcast frame to client {}: PT={}, Size={} bytes",
                client_id_clone, payload_type, data_len
            );
        });

        send_tasks.push(task);
    }

    // Wait for all sends to complete
    for task in send_tasks {
        let _ = task.await;
    }

    Ok(())
}

/// Receive a media frame from any client
pub async fn receive_frame(
    frame_sender: &broadcast::Sender<(String, MediaFrame)>,
) -> Result<(String, MediaFrame), MediaTransportError> {
    // Create a new receiver from the broadcast channel
    let mut receiver = frame_sender.subscribe();

    // Wait for a frame with a shorter timeout (500ms instead of 2s)
    match tokio::time::timeout(std::time::Duration::from_millis(500), receiver.recv()).await {
        Ok(Ok(frame)) => {
            // Successfully received frame
            Ok(frame)
        }
        Ok(Err(e)) => {
            // Error receiving from the broadcast channel
            Err(MediaTransportError::Transport(format!(
                "Broadcast channel error: {}",
                e
            )))
        }
        Err(_) => {
            // Timeout occurred
            Err(MediaTransportError::Timeout(
                "No frame received within timeout period".to_string(),
            ))
        }
    }
}
