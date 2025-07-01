//! Transmit buffer management
//!
//! This module handles high-performance transmit buffer functionality,
//! including packet queuing, priority handling, and congestion control.

use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, warn, info};

use crate::api::common::error::MediaTransportError;
use crate::api::common::frame::MediaFrame;
use crate::transport::RtpTransport;
use crate::buffer::{
    GlobalBufferManager, BufferPool, TransmitBuffer, TransmitBufferConfig, 
    BufferLimits, PacketPriority
};

/// Initialize the transmit buffer
///
/// This function initializes the high-performance transmit buffer if it's enabled
/// in the configuration.
pub async fn init_transmit_buffer(
    buffer_manager: &Option<Arc<GlobalBufferManager>>,
    packet_pool: &Option<Arc<BufferPool>>,
    transmit_buffer: &Arc<RwLock<Option<TransmitBuffer>>>,
    ssrc: u32,
    buffer_config: TransmitBufferConfig,
) -> Result<(), MediaTransportError> {
    // Placeholder for the extracted init_transmit_buffer functionality
    if let Some(buffer_manager) = buffer_manager {
        if let Some(packet_pool) = packet_pool {
            // Create transmit buffer
            let tx_buffer = TransmitBuffer::with_buffer_manager(
                ssrc,
                buffer_config,
                buffer_manager.clone(),
                packet_pool.clone(),
            );
            
            // Store it
            let mut tx_buffer_guard = transmit_buffer.write().await;
            *tx_buffer_guard = Some(tx_buffer);
            
            debug!("Initialized high-performance transmit buffer for SSRC {:#x}", ssrc);
        }
    }
    
    Ok(())
}

/// Send a media frame with specified priority
///
/// This function allows controlling which frames get priority when buffer constraints apply.
/// High priority frames will be sent before normal or low priority frames.
pub async fn send_frame_with_priority(
    frame: MediaFrame,
    priority: PacketPriority,
    high_performance_buffers_enabled: bool,
    transmit_buffer: &Arc<RwLock<Option<TransmitBuffer>>>,
    transport: &Arc<tokio::sync::Mutex<Option<Arc<dyn RtpTransport>>>>,
    remote_address: std::net::SocketAddr,
    fallback_send_frame: impl Fn(MediaFrame) -> Result<(), MediaTransportError>,
) -> Result<(), MediaTransportError> {
    // Placeholder for the extracted send_frame_with_priority functionality
    // Check if high-performance buffers are enabled
    if high_performance_buffers_enabled {
        // Check if we have a transmit buffer
        let mut tx_buffer_guard = transmit_buffer.write().await;
        if let Some(tx_buffer) = tx_buffer_guard.as_mut() {
            // Convert MediaFrame to RtpPacket
            let mut header = crate::packet::RtpHeader::new(
                frame.payload_type,
                frame.sequence,
                frame.timestamp,
                frame.ssrc
            );
            
            // Set marker bit if present in frame
            if frame.marker {
                header.marker = true;
            }
            
            // Add CSRCs if present
            if !frame.csrcs.is_empty() {
                header.add_csrcs(&frame.csrcs);
            }
            
            // Create RTP packet
            let packet = crate::packet::RtpPacket::new(
                header,
                bytes::Bytes::from(frame.data),
            );
            
            debug!("Queuing packet with priority {:?}: PT={}, SEQ={}, TS={}", 
                  priority, packet.header.payload_type, packet.header.sequence_number, packet.header.timestamp);
            
            // Queue in transmit buffer with priority
            if tx_buffer.queue_packet(packet, priority).await {
                // Get the next packet to send
                if let Some(packet) = tx_buffer.get_next_packet().await {
                    // Send it through the transport
                    let transport_guard = transport.lock().await;
                    if let Some(transport) = transport_guard.as_ref() {
                        transport.send_rtp(&packet, remote_address).await
                            .map_err(|e| MediaTransportError::SendError(format!("Failed to send RTP packet: {}", e)))?;
                        
                        // Automatically acknowledge successful send (simplistic approach)
                        tx_buffer.acknowledge_packet(packet.header.sequence_number);
                        
                        return Ok(());
                    } else {
                        return Err(MediaTransportError::Transport("Transport not connected".to_string()));
                    }
                }
            }
            
            // If we get here, the packet wasn't queued or wasn't ready to send
            return Err(MediaTransportError::BufferFull("Transmit buffer is full".to_string()));
        }
    }
    
    // Fall back to regular send_frame if high-performance buffers aren't enabled
    // or if the transmit buffer isn't available
    debug!("Falling back to regular send_frame");
    fallback_send_frame(frame)
}

/// Update the transmit buffer configuration
///
/// This function allows changing buffer parameters at runtime to adjust to network conditions
/// or application requirements.
pub async fn update_transmit_buffer_config(
    high_performance_buffers_enabled: bool,
    transmit_buffer: &Arc<RwLock<Option<TransmitBuffer>>>,
    config: TransmitBufferConfig,
) -> Result<(), MediaTransportError> {
    // Placeholder for the extracted update_transmit_buffer_config functionality
    if high_performance_buffers_enabled {
        let mut tx_buffer_guard = transmit_buffer.write().await;
        if let Some(tx_buffer) = tx_buffer_guard.as_mut() {
            // update_config is not an async method, so no need to await
            tx_buffer.update_config(config);
            debug!("Updated transmit buffer configuration");
            return Ok(());
        }
    }
    
    Err(MediaTransportError::ConfigError("High-performance buffers not enabled".to_string()))
}

/// Set the packet priority threshold based on buffer fullness
///
/// This function configures when the buffer fullness exceeds the specified percentage (0.0-1.0),
/// only packets with priority greater than or equal to the given threshold will be sent.
pub async fn set_priority_threshold(
    high_performance_buffers_enabled: bool,
    transmit_buffer: &Arc<RwLock<Option<TransmitBuffer>>>,
    buffer_fullness: f32,
    priority: PacketPriority,
) -> Result<(), MediaTransportError> {
    // Placeholder for the extracted set_priority_threshold functionality
    if high_performance_buffers_enabled {
        let mut tx_buffer_guard = transmit_buffer.write().await;
        if let Some(tx_buffer) = tx_buffer_guard.as_mut() {
            // set_priority_threshold is not an async method, so no need to await
            tx_buffer.set_priority_threshold(buffer_fullness, priority);
            debug!("Set priority threshold: {}% fullness -> {:?} priority", buffer_fullness * 100.0, priority);
            return Ok(());
        }
    }
    
    Err(MediaTransportError::ConfigError("High-performance buffers not enabled".to_string()))
} 