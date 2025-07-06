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

use super::{MediaSessionController, audio_generation::{AudioTransmitter, AudioTransmitterConfig, AudioSource}};

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
        
        // Start audio transmission in pass-through mode by default
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
    
    /// Start audio transmission for a dialog with default configuration (pass-through mode)
    pub async fn start_audio_transmission(&self, dialog_id: &DialogId) -> Result<()> {
        let config = AudioTransmitterConfig::default(); // Uses pass-through mode
        self.start_audio_transmission_with_config(dialog_id, config).await
    }
    
    /// Start audio transmission for a dialog with tone generation (for backward compatibility)
    pub async fn start_audio_transmission_with_tone(&self, dialog_id: &DialogId) -> Result<()> {
        let config = AudioTransmitterConfig {
            source: AudioSource::Tone { frequency: 440.0, amplitude: 0.5 },
            ..Default::default()
        };
        self.start_audio_transmission_with_config(dialog_id, config).await
    }
    
    /// Start audio transmission for a dialog with custom configuration
    pub async fn start_audio_transmission_with_config(&self, dialog_id: &DialogId, config: AudioTransmitterConfig) -> Result<()> {
        info!("🎵 Starting audio transmission for dialog: {}", dialog_id);
        
        let mut rtp_sessions = self.rtp_sessions.write().await;
        let wrapper = rtp_sessions.get_mut(dialog_id)
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;
        
        if wrapper.transmission_enabled {
            return Ok(()); // Already started
        }
        
        // Create audio transmitter with custom configuration
        let mut audio_transmitter = AudioTransmitter::new_with_config(wrapper.session.clone(), config);
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
    
    /// Set custom audio samples for transmission
    pub async fn set_custom_audio(&self, dialog_id: &DialogId, samples: Vec<u8>, repeat: bool) -> Result<()> {
        info!("🎵 Setting custom audio for dialog: {} ({} samples, repeat: {})", dialog_id, samples.len(), repeat);
        
        let rtp_sessions = self.rtp_sessions.read().await;
        let wrapper = rtp_sessions.get(dialog_id)
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;
        
        if let Some(transmitter) = &wrapper.audio_transmitter {
            transmitter.set_custom_audio(samples, repeat).await;
            info!("✅ Custom audio set for dialog: {}", dialog_id);
        } else {
            return Err(Error::config("Audio transmission not active for dialog".to_string()));
        }
        
        Ok(())
    }
    
    /// Set tone generation parameters for a dialog
    pub async fn set_tone_generation(&self, dialog_id: &DialogId, frequency: f64, amplitude: f64) -> Result<()> {
        info!("🎵 Setting tone generation for dialog: {} ({}Hz, amplitude: {})", dialog_id, frequency, amplitude);
        
        let rtp_sessions = self.rtp_sessions.read().await;
        let wrapper = rtp_sessions.get(dialog_id)
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;
        
        if let Some(transmitter) = &wrapper.audio_transmitter {
            transmitter.set_tone(frequency, amplitude).await;
            info!("✅ Tone generation set for dialog: {}", dialog_id);
        } else {
            return Err(Error::config("Audio transmission not active for dialog".to_string()));
        }
        
        Ok(())
    }
    
    /// Enable pass-through mode for a dialog (no audio generation)
    pub async fn set_pass_through_mode(&self, dialog_id: &DialogId) -> Result<()> {
        info!("🔄 Setting pass-through mode for dialog: {}", dialog_id);
        
        let rtp_sessions = self.rtp_sessions.read().await;
        let wrapper = rtp_sessions.get(dialog_id)
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;
        
        if let Some(transmitter) = &wrapper.audio_transmitter {
            transmitter.set_pass_through().await;
            info!("✅ Pass-through mode enabled for dialog: {}", dialog_id);
        } else {
            return Err(Error::config("Audio transmission not active for dialog".to_string()));
        }
        
        Ok(())
    }
    
    /// Start audio transmission with custom audio samples
    pub async fn start_audio_transmission_with_custom_audio(&self, dialog_id: &DialogId, samples: Vec<u8>, repeat: bool) -> Result<()> {
        let config = AudioTransmitterConfig {
            source: AudioSource::CustomSamples { samples, repeat },
            ..Default::default()
        };
        self.start_audio_transmission_with_config(dialog_id, config).await
    }
} 