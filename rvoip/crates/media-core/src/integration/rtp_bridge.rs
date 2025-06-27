//! RTP Bridge - Integration with rtp-core
//!
//! This module provides the bridge between media-core and rtp-core for
//! RTP packet handling and media transport.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tracing::{debug, warn, error};

use crate::error::{Result, IntegrationError};
use crate::types::{MediaSessionId, MediaPacket};
use super::events::{IntegrationEvent, IntegrationEventType, RtpParameters, PacketInfo};

/// Configuration for RTP bridge
#[derive(Debug, Clone)]
pub struct RtpBridgeConfig {
    /// Maximum pending packets per session
    pub max_pending_packets: usize,
    /// Enable packet statistics
    pub enable_statistics: bool,
    /// RTP session timeout in seconds
    pub session_timeout_secs: u64,
}

impl Default for RtpBridgeConfig {
    fn default() -> Self {
        Self {
            max_pending_packets: 100,
            enable_statistics: true,
            session_timeout_secs: 300, // 5 minutes
        }
    }
}

/// RTP session information
#[derive(Debug, Clone)]
struct RtpSessionInfo {
    /// Session parameters
    params: RtpParameters,
    /// Packets sent counter
    packets_sent: u64,
    /// Packets received counter
    packets_received: u64,
    /// Bytes sent counter
    bytes_sent: u64,
    /// Bytes received counter
    bytes_received: u64,
    /// Last activity timestamp
    last_activity: std::time::Instant,
}

/// Bridge between media-core and rtp-core
pub struct RtpBridge {
    /// Bridge configuration
    config: RtpBridgeConfig,
    /// Active RTP sessions
    sessions: Arc<RwLock<HashMap<MediaSessionId, RtpSessionInfo>>>,
    /// Event channel for integration events
    event_tx: mpsc::UnboundedSender<IntegrationEvent>,
    /// Incoming packet channel (from rtp-core)
    incoming_packet_rx: Arc<RwLock<Option<mpsc::UnboundedReceiver<(MediaSessionId, MediaPacket)>>>>,
    /// Outgoing packet channel (to rtp-core)
    outgoing_packet_tx: Arc<RwLock<Option<mpsc::UnboundedSender<(MediaSessionId, Vec<u8>, u32)>>>>,
}

impl RtpBridge {
    /// Create a new RTP bridge
    pub fn new(
        config: RtpBridgeConfig,
        event_tx: mpsc::UnboundedSender<IntegrationEvent>,
    ) -> Self {
        debug!("Creating RtpBridge with config: {:?}", config);
        
        Self {
            config,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
            incoming_packet_rx: Arc::new(RwLock::new(None)),
            outgoing_packet_tx: Arc::new(RwLock::new(None)),
        }
    }
    
    /// Set up packet channels for rtp-core communication
    pub async fn setup_channels(
        &self,
        incoming_rx: mpsc::UnboundedReceiver<(MediaSessionId, MediaPacket)>,
        outgoing_tx: mpsc::UnboundedSender<(MediaSessionId, Vec<u8>, u32)>,
    ) {
        {
            let mut incoming = self.incoming_packet_rx.write().await;
            *incoming = Some(incoming_rx);
        }
        
        {
            let mut outgoing = self.outgoing_packet_tx.write().await;
            *outgoing = Some(outgoing_tx);
        }
        
        debug!("RtpBridge channels configured");
    }
    
    /// Register an RTP session
    pub async fn register_session(
        &self,
        session_id: MediaSessionId,
        params: RtpParameters,
    ) -> Result<()> {
        let session_info = RtpSessionInfo {
            params: params.clone(),
            packets_sent: 0,
            packets_received: 0,
            bytes_sent: 0,
            bytes_received: 0,
            last_activity: std::time::Instant::now(),
        };
        
        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(session_id.clone(), session_info);
        }
        
        // Send integration event
        let event = IntegrationEvent::rtp_session_register(session_id.clone(), params);
        if let Err(e) = self.event_tx.send(event) {
            warn!("Failed to send RTP session register event: {}", e);
        }
        
        debug!("RTP session {} registered", session_id);
        Ok(())
    }
    
    /// Unregister an RTP session
    pub async fn unregister_session(&self, session_id: &MediaSessionId) -> Result<()> {
        {
            let mut sessions = self.sessions.write().await;
            sessions.remove(session_id);
        }
        
        // Send integration event
        let event = IntegrationEvent::new(
            IntegrationEventType::RtpSessionUnregister {
                session_id: session_id.clone(),
            },
            "media-core",
            "rtp-core",
        );
        if let Err(e) = self.event_tx.send(event) {
            warn!("Failed to send RTP session unregister event: {}", e);
        }
        
        debug!("RTP session {} unregistered", session_id);
        Ok(())
    }
    
    /// Send media packet via RTP
    pub async fn send_media_packet(
        &self,
        session_id: &MediaSessionId,
        encoded_data: Vec<u8>,
        timestamp: u32,
    ) -> Result<()> {
        // Check if session is registered
        let is_registered = {
            let sessions = self.sessions.read().await;
            sessions.contains_key(session_id)
        };
        
        if !is_registered {
            return Err(IntegrationError::RtpCore {
                details: format!("RTP session {} not registered", session_id),
            }.into());
        }
        
        // Send packet via outgoing channel
        {
            let outgoing = self.outgoing_packet_tx.read().await;
            if let Some(tx) = outgoing.as_ref() {
                if let Err(e) = tx.send((session_id.clone(), encoded_data.clone(), timestamp)) {
                    error!("Failed to send packet to rtp-core: {}", e);
                    return Err(IntegrationError::RtpCore {
                        details: "Failed to send packet to rtp-core".to_string(),
                    }.into());
                }
            } else {
                return Err(IntegrationError::RtpCore {
                    details: "Outgoing packet channel not configured".to_string(),
                }.into());
            }
        }
        
        // Update statistics
        {
            let mut sessions = self.sessions.write().await;
            if let Some(session_info) = sessions.get_mut(session_id) {
                session_info.packets_sent += 1;
                session_info.bytes_sent += encoded_data.len() as u64;
                session_info.last_activity = std::time::Instant::now();
            }
        }
        
        // Send integration event
        let event = IntegrationEvent::new(
            IntegrationEventType::MediaPacketSend {
                session_id: session_id.clone(),
                encoded_data,
                timestamp,
            },
            "media-core",
            "rtp-core",
        );
        if let Err(e) = self.event_tx.send(event) {
            warn!("Failed to send media packet send event: {}", e);
        }
        
        Ok(())
    }
    
    /// Process incoming media packet from RTP
    pub async fn process_incoming_packet(
        &self,
        session_id: &MediaSessionId,
        packet: MediaPacket,
    ) -> Result<()> {
        // Update statistics
        {
            let mut sessions = self.sessions.write().await;
            if let Some(session_info) = sessions.get_mut(session_id) {
                session_info.packets_received += 1;
                session_info.bytes_received += packet.payload.len() as u64;
                session_info.last_activity = std::time::Instant::now();
            }
        }
        
        // Send integration event
        let packet_info = PacketInfo {
            payload_type: packet.payload_type,
            sequence_number: packet.sequence_number,
            timestamp: packet.timestamp,
            ssrc: packet.ssrc,
            size: packet.payload.len(),
        };
        
        let event = IntegrationEvent::new(
            IntegrationEventType::MediaPacketReceived {
                session_id: session_id.clone(),
                packet_info,
            },
            "rtp-core",
            "media-core",
        );
        if let Err(e) = self.event_tx.send(event) {
            warn!("Failed to send media packet received event: {}", e);
        }
        
        Ok(())
    }
    
    /// Get session statistics
    pub async fn get_session_stats(&self, session_id: &MediaSessionId) -> Option<RtpSessionStats> {
        let sessions = self.sessions.read().await;
        sessions.get(session_id).map(|info| RtpSessionStats {
            packets_sent: info.packets_sent,
            packets_received: info.packets_received,
            bytes_sent: info.bytes_sent,
            bytes_received: info.bytes_received,
            last_activity: info.last_activity,
        })
    }
    
    /// Get all active sessions
    pub async fn get_active_sessions(&self) -> Vec<MediaSessionId> {
        let sessions = self.sessions.read().await;
        sessions.keys().cloned().collect()
    }
    
    /// Clean up expired sessions
    pub async fn cleanup_expired_sessions(&self) -> Result<()> {
        let now = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(self.config.session_timeout_secs);
        
        let expired_sessions: Vec<MediaSessionId> = {
            let sessions = self.sessions.read().await;
            sessions
                .iter()
                .filter(|(_, info)| now.duration_since(info.last_activity) > timeout)
                .map(|(id, _)| id.clone())
                .collect()
        };
        
        for session_id in expired_sessions {
            warn!("Cleaning up expired RTP session: {}", session_id);
            self.unregister_session(&session_id).await?;
        }
        
        Ok(())
    }
}

/// RTP session statistics
#[derive(Debug, Clone)]
pub struct RtpSessionStats {
    /// Packets sent
    pub packets_sent: u64,
    /// Packets received
    pub packets_received: u64,
    /// Bytes sent
    pub bytes_sent: u64,
    /// Bytes received
    pub bytes_received: u64,
    /// Last activity timestamp
    pub last_activity: std::time::Instant,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;
    
    #[tokio::test]
    async fn test_rtp_bridge_creation() {
        let (event_tx, _event_rx) = mpsc::unbounded_channel();
        let bridge = RtpBridge::new(RtpBridgeConfig::default(), event_tx);
        
        let sessions = bridge.get_active_sessions().await;
        assert!(sessions.is_empty());
    }
    
    #[tokio::test]
    async fn test_session_registration() {
        let (event_tx, _event_rx) = mpsc::unbounded_channel();
        let bridge = RtpBridge::new(RtpBridgeConfig::default(), event_tx);
        
        let session_id = MediaSessionId::new("test-session");
        let params = RtpParameters {
            local_port: 5004,
            remote_address: "192.168.1.100".to_string(),
            remote_port: 5004,
            payload_type: 0,
            ssrc: 12345,
        };
        
        bridge.register_session(session_id.clone(), params).await.unwrap();
        
        let active_sessions = bridge.get_active_sessions().await;
        assert_eq!(active_sessions.len(), 1);
        assert_eq!(active_sessions[0], session_id);
        
        bridge.unregister_session(&session_id).await.unwrap();
        
        let active_sessions = bridge.get_active_sessions().await;
        assert!(active_sessions.is_empty());
    }
} 