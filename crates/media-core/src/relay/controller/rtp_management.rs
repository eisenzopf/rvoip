//! RTP session management functionality
//!
//! This module handles all RTP-related operations including session management,
//! packet transmission, remote address updates, and media flow control.

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};
use bytes::Bytes;

use crate::error::{Error, Result};
use crate::types::DialogId;
use rvoip_rtp_core::RtpSession;

use super::{MediaSessionController, audio_generation::AudioTransmitter};

impl MediaSessionController {
    /// Get RTP session for a dialog (for packet transmission)
    pub async fn get_rtp_session(&self, dialog_id: &DialogId) -> Option<Arc<tokio::sync::Mutex<RtpSession>>> {
        let rtp_sessions = self.rtp_sessions.read().await;
        rtp_sessions.get(dialog_id).map(|wrapper| wrapper.session.clone())
    }
    
    /// Send RTP packet for a dialog
    pub async fn send_rtp_packet(&self, dialog_id: &DialogId, payload: Vec<u8>, timestamp: u32) -> Result<()> {
        let rtp_session = self.get_rtp_session(dialog_id).await
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;
        
        let mut session = rtp_session.lock().await;
        session.send_packet(timestamp, Bytes::from(payload), false).await
            .map_err(|e| Error::config(format!("Failed to send RTP packet: {}", e)))?;
        
        debug!("✅ Sent RTP packet for dialog: {} (timestamp: {})", dialog_id, timestamp);
        Ok(())
    }
    
    /// Update remote address for RTP session
    pub async fn update_rtp_remote_addr(&self, dialog_id: &DialogId, remote_addr: SocketAddr) -> Result<()> {
        let rtp_session = self.get_rtp_session(dialog_id).await
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;
        
        let mut session = rtp_session.lock().await;
        session.set_remote_addr(remote_addr).await;
        
        // Update wrapper info
        {
            let mut rtp_sessions = self.rtp_sessions.write().await;
            if let Some(wrapper) = rtp_sessions.get_mut(dialog_id) {
                wrapper.remote_addr = Some(remote_addr);
            }
        }
        
        info!("✅ Updated RTP remote address for dialog: {} -> {}", dialog_id, remote_addr);
        Ok(())
    }
    
    /// Set remote address and start audio transmission (called when call is established)
    pub async fn establish_media_flow(&self, dialog_id: &DialogId, remote_addr: SocketAddr) -> Result<()> {
        info!("🔗 Establishing media flow for dialog: {} -> {}", dialog_id, remote_addr);
        
        // Update remote address
        self.update_rtp_remote_addr(dialog_id, remote_addr).await?;
        
        // Start audio transmission
        self.start_audio_transmission(dialog_id).await?;
        
        info!("✅ Media flow established for dialog: {}", dialog_id);
        Ok(())
    }
    
    /// Terminate media flow (called when call ends)
    pub async fn terminate_media_flow(&self, dialog_id: &DialogId) -> Result<()> {
        info!("🛑 Terminating media flow for dialog: {}", dialog_id);
        
        // Stop audio transmission
        self.stop_audio_transmission(dialog_id).await?;
        
        // Clean up advanced processors if they exist
        {
            let mut processors = self.advanced_processors.write().await;
            if processors.remove(dialog_id).is_some() {
                info!("🧹 Cleaned up advanced processors for dialog: {}", dialog_id);
            }
        }
        
        info!("✅ Media flow terminated for dialog: {}", dialog_id);
        Ok(())
    }
    
    /// Start audio transmission for a dialog
    pub async fn start_audio_transmission(&self, dialog_id: &DialogId) -> Result<()> {
        info!("🎵 Starting audio transmission for dialog: {}", dialog_id);
        
        let mut rtp_sessions = self.rtp_sessions.write().await;
        let wrapper = rtp_sessions.get_mut(dialog_id)
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;
        
        if wrapper.transmission_enabled {
            return Ok(()); // Already started
        }
        
        // Create audio transmitter
        let mut audio_transmitter = AudioTransmitter::new(wrapper.session.clone());
        audio_transmitter.start().await;
        
        wrapper.audio_transmitter = Some(audio_transmitter);
        wrapper.transmission_enabled = true;
        
        info!("✅ Audio transmission started for dialog: {}", dialog_id);
        Ok(())
    }
    
    /// Stop audio transmission for a dialog
    pub async fn stop_audio_transmission(&self, dialog_id: &DialogId) -> Result<()> {
        info!("🛑 Stopping audio transmission for dialog: {}", dialog_id);
        
        let mut rtp_sessions = self.rtp_sessions.write().await;
        let wrapper = rtp_sessions.get_mut(dialog_id)
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;
        
        if let Some(transmitter) = &wrapper.audio_transmitter {
            transmitter.stop().await;
        }
        
        wrapper.audio_transmitter = None;
        wrapper.transmission_enabled = false;
        
        info!("✅ Audio transmission stopped for dialog: {}", dialog_id);
        Ok(())
    }
    
    /// Check if audio transmission is active for a dialog
    pub async fn is_audio_transmission_active(&self, dialog_id: &DialogId) -> bool {
        let rtp_sessions = self.rtp_sessions.read().await;
        if let Some(wrapper) = rtp_sessions.get(dialog_id) {
            if let Some(transmitter) = &wrapper.audio_transmitter {
                return transmitter.is_active().await;
            }
        }
        false
    }
} 