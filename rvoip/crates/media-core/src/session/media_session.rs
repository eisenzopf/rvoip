//! Media Session Implementation
//!
//! This module implements MediaSession, which manages media processing for individual
//! SIP dialogs, including codec lifecycle, and quality monitoring.

use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tracing::{debug, warn, info, error};

use crate::error::{Result, MediaSessionError};
use crate::types::{MediaSessionId, DialogId, AudioFrame, MediaPacket, MediaType, MediaDirection};
use crate::codec::audio::common::AudioCodec;
use crate::processing::audio::AudioProcessor;
use crate::quality::{QualityMonitor, QualityMetrics};
use super::events::{MediaSessionEvent, MediaSessionEventType, QualitySeverity};

/// Media session state
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaSessionState {
    /// Session is being created
    Creating,
    /// Session is active and processing media
    Active,
    /// Session is paused (no media processing)
    Paused,
    /// Session is being destroyed
    Destroying,
    /// Session has been destroyed
    Destroyed,
}

/// Media session configuration
#[derive(Debug, Clone)]
pub struct MediaSessionConfig {
    /// Enable audio processing (AEC, AGC, VAD)
    pub enable_audio_processing: bool,
    /// Enable quality monitoring
    pub enable_quality_monitoring: bool,
    /// Enable automatic codec adaptation
    pub enable_codec_adaptation: bool,
    /// Maximum packet buffer size
    pub max_packet_buffer_size: usize,
    /// Processing thread pool size
    pub processing_threads: usize,
}

impl Default for MediaSessionConfig {
    fn default() -> Self {
        Self {
            enable_audio_processing: true,
            enable_quality_monitoring: true,
            enable_codec_adaptation: true,
            max_packet_buffer_size: 1000,
            processing_threads: 2,
        }
    }
}

/// Media session for per-dialog media management
pub struct MediaSession {
    /// Unique session identifier
    session_id: MediaSessionId,
    
    /// Associated SIP dialog ID
    dialog_id: DialogId,
    
    /// Current session state
    state: Arc<RwLock<MediaSessionState>>,
    
    /// Session configuration
    config: MediaSessionConfig,
    
    /// Audio codec for this session
    audio_codec: Arc<RwLock<Option<Box<dyn AudioCodec>>>>,
    
    /// Audio processor
    audio_processor: Option<Arc<AudioProcessor>>,
    
    /// Quality monitor
    quality_monitor: Option<Arc<QualityMonitor>>,
    
    /// Event channel for notifying other components
    event_tx: mpsc::UnboundedSender<MediaSessionEvent>,
    
    /// Media session statistics
    stats: Arc<RwLock<MediaSessionStats>>,
}

/// Media session statistics
#[derive(Debug, Default, Clone)]
pub struct MediaSessionStats {
    /// Packets sent
    pub packets_sent: u64,
    /// Packets received
    pub packets_received: u64,
    /// Bytes sent
    pub bytes_sent: u64,
    /// Bytes received
    pub bytes_received: u64,
    /// Processing errors
    pub processing_errors: u64,
    /// Quality warnings
    pub quality_warnings: u64,
}

impl MediaSession {
    /// Create a new media session
    pub fn new(
        session_id: MediaSessionId,
        dialog_id: DialogId,
        config: MediaSessionConfig,
        event_tx: mpsc::UnboundedSender<MediaSessionEvent>,
    ) -> Result<Self> {
        debug!("Creating MediaSession {} for dialog {}", session_id, dialog_id);
        
        // Create audio processor if enabled
        let audio_processor = if config.enable_audio_processing {
            Some(Arc::new(AudioProcessor::new(Default::default())?))
        } else {
            None
        };
        
        // Create quality monitor if enabled
        let quality_monitor = if config.enable_quality_monitoring {
            Some(Arc::new(QualityMonitor::new(Default::default())))
        } else {
            None
        };
        
        let session = Self {
            session_id: session_id.clone(),
            dialog_id: dialog_id.clone(),
            state: Arc::new(RwLock::new(MediaSessionState::Creating)),
            config,
            audio_codec: Arc::new(RwLock::new(None)),
            audio_processor,
            quality_monitor,
            event_tx,
            stats: Arc::new(RwLock::new(MediaSessionStats::default())),
        };
        
        // Send session created event
        let event = MediaSessionEvent::session_created(session_id, dialog_id);
        if let Err(e) = session.event_tx.send(event) {
            warn!("Failed to send session created event: {}", e);
        }
        
        Ok(session)
    }
    
    /// Get session ID
    pub fn session_id(&self) -> &MediaSessionId {
        &self.session_id
    }
    
    /// Get dialog ID
    pub fn dialog_id(&self) -> &DialogId {
        &self.dialog_id
    }
    
    /// Get current session state
    pub async fn state(&self) -> MediaSessionState {
        self.state.read().await.clone()
    }
    
    /// Set session state
    async fn set_state(&self, new_state: MediaSessionState) {
        let mut state = self.state.write().await;
        if *state != new_state {
            debug!("MediaSession {} state changed: {:?} -> {:?}", 
                   self.session_id, *state, new_state);
            *state = new_state;
        }
    }
    
    /// Start the media session
    pub async fn start(&self) -> Result<()> {
        let current_state = self.state().await;
        if current_state != MediaSessionState::Creating {
            return Err(MediaSessionError::InvalidState {
                state: format!("{:?}", current_state),
            }.into());
        }
        
        self.set_state(MediaSessionState::Active).await;
        info!("MediaSession {} started", self.session_id);
        
        Ok(())
    }
    
    /// Pause the media session
    pub async fn pause(&self) -> Result<()> {
        let current_state = self.state().await;
        if current_state != MediaSessionState::Active {
            return Err(MediaSessionError::InvalidState {
                state: format!("{:?}", current_state),
            }.into());
        }
        
        self.set_state(MediaSessionState::Paused).await;
        info!("MediaSession {} paused", self.session_id);
        
        Ok(())
    }
    
    /// Resume the media session
    pub async fn resume(&self) -> Result<()> {
        let current_state = self.state().await;
        if current_state != MediaSessionState::Paused {
            return Err(MediaSessionError::InvalidState {
                state: format!("{:?}", current_state),
            }.into());
        }
        
        self.set_state(MediaSessionState::Active).await;
        info!("MediaSession {} resumed", self.session_id);
        
        Ok(())
    }
    
    /// Stop the media session
    pub async fn stop(&self) -> Result<()> {
        self.set_state(MediaSessionState::Destroying).await;
        
        // Send session destroyed event
        let event = MediaSessionEvent::session_destroyed(
            self.session_id.clone(),
            self.dialog_id.clone(),
        );
        if let Err(e) = self.event_tx.send(event) {
            warn!("Failed to send session destroyed event: {}", e);
        }
        
        self.set_state(MediaSessionState::Destroyed).await;
        info!("MediaSession {} stopped", self.session_id);
        
        Ok(())
    }
    
    /// Set audio codec for this session
    pub async fn set_audio_codec(&self, codec: Box<dyn AudioCodec>) -> Result<()> {
        let codec_info = codec.get_info();
        
        // Check if codec changed
        let old_codec_name = {
            let current_codec = self.audio_codec.read().await;
            current_codec.as_ref().map(|c| c.get_info().name).unwrap_or_else(|| "None".to_string())
        };
        
        // Update codec
        {
            let mut audio_codec = self.audio_codec.write().await;
            *audio_codec = Some(codec);
        }
        
        // Send codec changed event
        let event = MediaSessionEvent::codec_changed(
            self.session_id.clone(),
            self.dialog_id.clone(),
            MediaType::Audio,
            old_codec_name,
            codec_info.name.clone(),
        );
        if let Err(e) = self.event_tx.send(event) {
            warn!("Failed to send codec changed event: {}", e);
        }
        
        info!("MediaSession {} audio codec set to {}", self.session_id, codec_info.name);
        
        Ok(())
    }
    
    /// Process incoming media packet
    pub async fn process_incoming_media(&self, packet: MediaPacket) -> Result<AudioFrame> {
        // Check session state
        let state = self.state().await;
        if state != MediaSessionState::Active {
            return Err(MediaSessionError::InvalidState {
                state: format!("{:?}", state),
            }.into());
        }
        
        // Update statistics
        {
            let mut stats = self.stats.write().await;
            stats.packets_received += 1;
            stats.bytes_received += packet.payload.len() as u64;
        }
        
        // Quality monitoring
        if let Some(quality_monitor) = &self.quality_monitor {
            if let Err(e) = quality_monitor.analyze_media_packet(&self.session_id, &packet).await {
                warn!("Quality analysis failed: {}", e);
            }
        }
        
        // Decode audio
        let audio_frame = {
            let mut codec_guard = self.audio_codec.write().await;
            match codec_guard.as_mut() {
                Some(codec) => codec.decode(&packet.payload)?,
                None => {
                    return Err(MediaSessionError::OperationFailed {
                        operation: "decode".to_string(),
                    }.into());
                }
            }
        };
        
        // Audio processing
        let processed_frame = if let Some(audio_processor) = &self.audio_processor {
            let result = audio_processor.process_playback_audio(&audio_frame).await?;
            result.frame
        } else {
            audio_frame
        };
        
        Ok(processed_frame)
    }
    
    /// Process outgoing media frame
    pub async fn process_outgoing_media(&self, audio_frame: AudioFrame) -> Result<Vec<u8>> {
        // Check session state
        let state = self.state().await;
        if state != MediaSessionState::Active {
            return Err(MediaSessionError::InvalidState {
                state: format!("{:?}", state),
            }.into());
        }
        
        // Audio processing
        let processed_frame = if let Some(audio_processor) = &self.audio_processor {
            let result = audio_processor.process_capture_audio(&audio_frame).await?;
            result.frame
        } else {
            audio_frame
        };
        
        // Encode audio
        let encoded_data = {
            let mut codec_guard = self.audio_codec.write().await;
            match codec_guard.as_mut() {
                Some(codec) => codec.encode(&processed_frame)?,
                None => {
                    return Err(MediaSessionError::OperationFailed {
                        operation: "encode".to_string(),
                    }.into());
                }
            }
        };
        
        // Update statistics
        {
            let mut stats = self.stats.write().await;
            stats.packets_sent += 1;
            stats.bytes_sent += encoded_data.len() as u64;
        }
        
        Ok(encoded_data)
    }
    
    /// Get quality metrics for this session
    pub async fn get_quality_metrics(&self) -> Option<QualityMetrics> {
        match &self.quality_monitor {
            Some(monitor) => {
                match monitor.get_session_metrics(&self.session_id).await {
                    Some(session_metrics) => Some(session_metrics.current),
                    None => None,
                }
            }
            None => None,
        }
    }
    
    /// Get session statistics
    pub async fn get_statistics(&self) -> MediaSessionStats {
        self.stats.read().await.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SampleRate;
    use crate::codec::audio::G711Codec;
    
    #[tokio::test]
    async fn test_media_session_creation() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let session_id = MediaSessionId::new("test-session");
        let dialog_id = DialogId::new("test-dialog");
        
        let session = MediaSession::new(
            session_id.clone(),
            dialog_id.clone(),
            MediaSessionConfig::default(),
            tx,
        ).unwrap();
        
        assert_eq!(session.session_id(), &session_id);
        assert_eq!(session.dialog_id(), &dialog_id);
        assert_eq!(session.state().await, MediaSessionState::Creating);
    }
    
    #[tokio::test]
    async fn test_media_session_lifecycle() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let session_id = MediaSessionId::new("test-session");
        let dialog_id = DialogId::new("test-dialog");
        
        let session = MediaSession::new(
            session_id,
            dialog_id,
            MediaSessionConfig::default(),
            tx,
        ).unwrap();
        
        // Test state transitions
        session.start().await.unwrap();
        assert_eq!(session.state().await, MediaSessionState::Active);
        
        session.pause().await.unwrap();
        assert_eq!(session.state().await, MediaSessionState::Paused);
        
        session.resume().await.unwrap();
        assert_eq!(session.state().await, MediaSessionState::Active);
        
        session.stop().await.unwrap();
        assert_eq!(session.state().await, MediaSessionState::Destroyed);
    }
    
    #[tokio::test]
    async fn test_audio_codec_management() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let session_id = MediaSessionId::new("test-session");
        let dialog_id = DialogId::new("test-dialog");
        
        let session = MediaSession::new(
            session_id,
            dialog_id,
            MediaSessionConfig::default(),
            tx,
        ).unwrap();
        
        // Set audio codec
        let codec = Box::new(G711Codec::mu_law(SampleRate::Rate8000, 1).unwrap());
        session.set_audio_codec(codec).await.unwrap();
        
        // Verify codec is set
        let codec_guard = session.audio_codec.read().await;
        assert!(codec_guard.is_some());
        let info = codec_guard.as_ref().unwrap().get_info();
        assert_eq!(info.name, "PCMU");
    }
} 