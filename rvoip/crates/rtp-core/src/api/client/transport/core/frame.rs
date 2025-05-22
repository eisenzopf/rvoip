//! Frame handling for client transport
//!
//! This module handles the sending and receiving of media frames, including
//! RTP packet construction, sequence number management, and frame processing.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use std::sync::atomic::{AtomicBool, Ordering};
use std::collections::HashMap;
use tokio::sync::{Mutex, mpsc};
use bytes::Bytes;
use tracing::{debug, error, info, warn};
use rand::Rng;

use crate::api::common::frame::{MediaFrame, MediaFrameType};
use crate::api::common::error::MediaTransportError;
use crate::transport::{RtpTransport, UdpRtpTransport};
use crate::packet::RtpPacket;
use crate::session::RtpSession;
use crate::CsrcManager;
use crate::api::client::config::ClientConfig;
use crate::{CsrcMapping, RtpSsrc, RtpCsrc, MAX_CSRC_COUNT};

/// Send a media frame to the remote peer
///
/// This function constructs an RTP packet from the given media frame and sends
/// it to the remote peer. It handles SSRC selection, sequence number generation,
/// and CSRC management.
pub async fn send_frame(
    frame: MediaFrame,
    connected: &Arc<AtomicBool>,
    session: &Arc<Mutex<RtpSession>>,
    transport: &Arc<Mutex<Option<Arc<UdpRtpTransport>>>>,
    config: &ClientConfig,
    sequence_numbers: &Arc<Mutex<HashMap<u32, u16>>>,
    remote_address: SocketAddr,
    csrc_manager: &Arc<Mutex<CsrcManager>>,
    csrc_management_enabled: bool,
) -> Result<(), MediaTransportError> {
    // Check if connected
    if !connected.load(Ordering::SeqCst) {
        return Err(MediaTransportError::NotConnected);
    }
    
    debug!("Send frame called: PT={}, TS={}, size={}, marker={}", 
           frame.payload_type, frame.timestamp, frame.data.len(), frame.marker);
    
    let mut session_guard = session.lock().await;
    
    // Select SSRC
    let ssrc = if config.ssrc_demultiplexing_enabled.unwrap_or(false) && frame.ssrc != 0 {
        // Use custom SSRC from the frame
        frame.ssrc
    } else {
        // Use default SSRC from session
        session_guard.get_ssrc()
    };
    
    // Get sequence number
    let sequence = if config.ssrc_demultiplexing_enabled.unwrap_or(false) && frame.ssrc != 0 {
        // Use sequence number mapping for this SSRC
        let mut seq_map = sequence_numbers.lock().await;
        let sequence = if let Some(seq) = seq_map.get_mut(&frame.ssrc) {
            // Increment sequence number
            *seq = seq.wrapping_add(1);
            *seq
        } else {
            // Start with a random sequence number for this SSRC
            let sequence = rand::random::<u16>();
            seq_map.insert(frame.ssrc, sequence);
            sequence
        };
        sequence
    } else {
        // Default: Use sequence number from the frame or generate a new one
        if frame.sequence != 0 {
            frame.sequence
        } else {
            // Generate a new sequence number
            rand::random::<u16>()
        }
    };
    
    // Store frame data length before it's moved
    let data_len = frame.data.len();
    
    // Get transport
    let transport_guard = transport.lock().await;
    let transport = transport_guard.as_ref()
        .ok_or_else(|| MediaTransportError::Transport("Transport not connected".to_string()))?;
    
    // Create RTP header
    let mut header = crate::packet::RtpHeader::new(
        frame.payload_type,
        sequence,
        frame.timestamp,
        ssrc
    );
    
    // Set marker flag if present in frame
    if frame.marker {
        header.marker = true;
    }
    
    // Add CSRCs if CSRC management is enabled
    if csrc_management_enabled {
        // For simplicity, we'll just use all active SSRCs as active sources
        // In a real conference mixer, this would be based on audio activity
        // Get all SSRCs from the session (we don't have get_active_streams)
        let active_ssrcs = session_guard.get_all_ssrcs().await;
        
        if !active_ssrcs.is_empty() {
            // Get CSRC values from the manager
            let csrc_manager = csrc_manager.lock().await;
            let csrcs = csrc_manager.get_active_csrcs(&active_ssrcs);
            
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
    
    // Create RTP packet
    let packet = crate::packet::RtpPacket::new(
        header,
        Bytes::from(frame.data),
    );
    
    // Send packet
    transport.send_rtp(&packet, remote_address).await
        .map_err(|e| MediaTransportError::SendError(format!("Failed to send RTP packet: {}", e)))?;
    
    debug!("Sent frame: PT={}, TS={}, SEQ={}, Size={} bytes", 
           frame.payload_type, frame.timestamp, sequence, data_len);
    
    Ok(())
}

/// Receive a media frame from the remote peer
///
/// This function waits for a media frame to be received from the remote peer.
/// It returns the frame if one is available within the timeout, or None if
/// the timeout expires.
pub async fn receive_frame(
    timeout: Duration,
    receiver: &Arc<Mutex<mpsc::Receiver<MediaFrame>>>,
) -> Result<Option<MediaFrame>, MediaTransportError> {
    let mut receiver = receiver.lock().await;
    
    match tokio::time::timeout(timeout, receiver.recv()).await {
        Ok(Some(frame)) => Ok(Some(frame)),
        Ok(None) => Err(MediaTransportError::ReceiveError("Channel closed".to_string())),
        Err(_) => Ok(None), // Timeout occurred
    }
}

/// Process an incoming RTP packet
///
/// This function parses an incoming RTP packet and converts it into a MediaFrame
/// for delivery to the application. It handles payload type mapping and frame type
/// determination.
pub async fn process_packet(
    packet: &[u8],
    session: &Arc<Mutex<RtpSession>>,
    frame_sender: &mpsc::Sender<MediaFrame>,
) -> Result<(), MediaTransportError> {
    let mut session = session.lock().await;
    
    // Handle the processing here manually since we have raw packet data
    match crate::packet::RtpPacket::parse(packet) {
        Ok(rtp_packet) => {
            // Create a simplified MediaFrame from the RTP packet
            let frame = MediaFrame {
                frame_type: get_frame_type_from_payload_type(rtp_packet.header.payload_type),
                data: rtp_packet.payload.to_vec(),
                timestamp: rtp_packet.header.timestamp,
                sequence: rtp_packet.header.sequence_number,
                marker: rtp_packet.header.marker,
                payload_type: rtp_packet.header.payload_type,
                ssrc: rtp_packet.header.ssrc,
                csrcs: rtp_packet.header.csrc.clone(),
            };
            
            // Forward frame to the application
            if let Err(e) = frame_sender.send(frame).await {
                warn!("Error sending frame to application: {}", e);
            }
            
            Ok(())
        },
        Err(e) => {
            warn!("Error parsing RTP packet: {}", e);
            Err(MediaTransportError::ReceiveError(format!("Failed to parse RTP packet: {}", e)))
        }
    }
}

/// Determine frame type from payload type
///
/// This function maps RTP payload types to media frame types based on common
/// payload type assignments and configuration.
pub fn get_frame_type_from_payload_type(payload_type: u8) -> MediaFrameType {
    match payload_type {
        // Audio payload types (common)
        0..=34 => MediaFrameType::Audio,
        // Video payload types (common)
        35..=50 => MediaFrameType::Video,
        // Dynamic payload types - default to audio
        96..=127 => MediaFrameType::Audio,
        // All other types
        _ => MediaFrameType::Data,
    }
} 