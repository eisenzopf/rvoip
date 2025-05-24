//! Media Session implementation
//!
//! This module provides the core MediaSession implementation for handling
//! real-time media sessions.

use std::sync::{Arc, Mutex};
use tokio::sync::{RwLock, mpsc};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use bytes::Bytes;
use tokio::task;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::codec::{Codec, AudioCodec, VideoCodec};
use crate::rtp::RtpContext;
use rvoip_rtp_core::RtpSession;
use super::{
    MediaDirection, 
    MediaType, 
    MediaState, 
    MediaSessionId, 
    MediaSessionStats,
    MediaSessionConfig,
    MediaFlow,
};
use super::events::{MediaEvent, MediaEventType, MediaSessionEvent};

/// Core media session implementation
///
/// A MediaSession represents a bidirectional media session between two endpoints.
/// It manages the lifecycle of the media streams, handles codec negotiation,
/// and coordinates RTP/RTCP communication.
#[derive(Debug)]
pub struct MediaSession {
    /// Unique session identifier
    pub id: MediaSessionId,
    
    /// Current session state
    state: RwLock<MediaState>,
    
    /// Media direction
    direction: RwLock<MediaDirection>,
    
    /// Media type (audio, video, etc.)
    media_type: MediaType,
    
    /// Active codec
    codec: RwLock<Option<Arc<dyn Codec + Send + Sync>>>,
    
    /// Session configuration
    config: MediaSessionConfig,
    
    /// RTP context for media transport
    rtp_context: RwLock<Option<RtpContext>>,
    
    /// Session statistics
    stats: RwLock<MediaSessionStats>,
    
    /// Event sender
    event_tx: mpsc::Sender<MediaEvent>,
    
    /// Event receiver (held by the media manager)
    _event_rx: Option<mpsc::Receiver<MediaEvent>>,
    
    /// RTP session for media transport
    rtp_session: Arc<RtpSession>,
    
    /// Audio codec
    audio_codec: RwLock<Option<Box<dyn AudioCodec>>>,
    
    /// Video codec
    video_codec: RwLock<Option<Box<dyn VideoCodec>>>,
    
    /// Event sender for session events
    event_sender: mpsc::UnboundedSender<MediaSessionEvent>,
    
    /// Metrics collector
    metrics: Arc<Mutex<SessionMetrics>>,
    
    /// Session start time
    start_time: Instant,
}

/// Media session metrics
#[derive(Debug, Default, Clone)]
pub struct SessionMetrics {
    /// Total packets sent
    pub packets_sent: u64,
    
    /// Total packets received
    pub packets_received: u64,
    
    /// Packets lost
    pub packets_lost: u64,
    
    /// Bytes sent
    pub bytes_sent: u64,
    
    /// Bytes received
    pub bytes_received: u64,
    
    /// Jitter in milliseconds
    pub jitter_ms: f32,
    
    /// Round trip time in milliseconds
    pub rtt_ms: Option<f32>,
    
    /// Media quality score (0-5, where 5 is best)
    pub quality_score: f32,
    
    /// Last packet received time
    pub last_packet_received: Option<Instant>,
    
    /// Last packet sent time
    pub last_packet_sent: Option<Instant>,
}

impl MediaSession {
    /// Create a new media session with the given configuration
    pub fn new(config: MediaSessionConfig, media_type: MediaType) -> Self {
        let id = MediaSessionId::new();
        let (event_tx, event_rx) = mpsc::channel(100);
        let (session_event_tx, _) = mpsc::unbounded_channel();
        
        // Create a basic RTP session - we'll need to configure this properly later
        let rtp_session = Arc::new(RtpSession::new());
        
        Self {
            id,
            state: RwLock::new(MediaState::Creating),
            direction: RwLock::new(MediaDirection::SendRecv),
            media_type,
            codec: RwLock::new(None),
            config,
            rtp_context: RwLock::new(None),
            stats: RwLock::new(MediaSessionStats::default()),
            event_tx,
            _event_rx: Some(event_rx),
            rtp_session,
            audio_codec: RwLock::new(None),
            video_codec: RwLock::new(None),
            event_sender: session_event_tx,
            metrics: Arc::new(Mutex::new(SessionMetrics::default())),
            start_time: Instant::now(),
        }
    }
    
    /// Get the current session state
    pub async fn state(&self) -> MediaState {
        *self.state.read().await
    }
    
    /// Set the session state
    pub async fn set_state(&self, state: MediaState) -> Result<()> {
        let mut state_guard = self.state.write().await;
        let old_state = *state_guard;
        *state_guard = state;
        
        // Emit state change event
        self.emit_event(MediaEventType::StateChanged {
            old_state,
            new_state: state,
        }).await?;
        
        Ok(())
    }
    
    /// Get the current media direction
    pub async fn direction(&self) -> MediaDirection {
        *self.direction.read().await
    }
    
    /// Set the media direction
    pub async fn set_direction(&self, direction: MediaDirection) -> Result<()> {
        let mut dir_guard = self.direction.write().await;
        let old_direction = *dir_guard;
        *dir_guard = direction;
        
        // Emit direction change event
        self.emit_event(MediaEventType::DirectionChanged {
            old_direction,
            new_direction: direction,
        }).await?;
        
        Ok(())
    }
    
    /// Get the media type for this session
    pub fn media_type(&self) -> MediaType {
        self.media_type
    }
    
    /// Get the current codec
    pub async fn codec(&self) -> Option<Arc<dyn Codec + Send + Sync>> {
        self.codec.read().await.clone()
    }
    
    /// Set the codec to use for this session
    pub async fn set_codec(&self, codec: Arc<dyn Codec + Send + Sync>) -> Result<()> {
        let mut codec_guard = self.codec.write().await;
        *codec_guard = Some(codec);
        Ok(())
    }
    
    /// Start the media session
    pub async fn start(&self) -> Result<()> {
        // Ensure we have a codec
        if self.codec.read().await.is_none() {
            return Err(Error::NoCodecSelected);
        }
        
        // Create RTP context if needed
        if self.rtp_context.read().await.is_none() {
            // TODO: Initialize RTP context
        }
        
        // Update state
        self.set_state(MediaState::Active).await?;
        
        // Emit started event
        self.emit_event(MediaEventType::Started).await?;
        
        Ok(())
    }
    
    /// Stop the media session
    pub async fn stop(&self) -> Result<()> {
        // Update state
        self.set_state(MediaState::Terminated).await?;
        
        // Emit stopped event
        self.emit_event(MediaEventType::Stopped).await?;
        
        Ok(())
    }
    
    /// Hold the media session
    pub async fn hold(&self) -> Result<()> {
        // Update state
        self.set_state(MediaState::Held).await?;
        
        // Emit held event
        self.emit_event(MediaEventType::Held).await?;
        
        Ok(())
    }
    
    /// Resume the media session
    pub async fn resume(&self) -> Result<()> {
        // Update state
        self.set_state(MediaState::Active).await?;
        
        // Emit resumed event
        self.emit_event(MediaEventType::Resumed).await?;
        
        Ok(())
    }
    
    /// Get the current session statistics
    pub async fn stats(&self) -> MediaSessionStats {
        self.stats.read().await.clone()
    }
    
    /// Emit a media event
    async fn emit_event(&self, event_type: MediaEventType) -> Result<()> {
        let event = MediaEvent {
            session_id: self.id,
            timestamp: std::time::SystemTime::now(),
            event_type,
        };
        
        // Try to send the event, but don't block if the channel is full
        self.event_tx.try_send(event)
            .map_err(|_| Error::EventChannelFull)?;
            
        Ok(())
    }
    
    /// Get the event receiver, consuming this session
    pub fn take_event_receiver(mut self) -> Option<mpsc::Receiver<MediaEvent>> {
        self._event_rx.take()
    }
    
    /// Set audio codec
    pub async fn set_audio_codec(&self, codec: Box<dyn AudioCodec>) {
        let mut audio_codec = self.audio_codec.write().await;
        *audio_codec = Some(codec);
        
        // Update RTP parameters if needed
        if let Some(codec) = audio_codec.as_ref() {
            // Note: RtpSession API may need to be checked for these methods
            // let _ = self.rtp_session.set_payload_type(codec.payload_type());
            // let _ = self.rtp_session.set_clock_rate(codec.clock_rate());
        }
    }
    
    /// Set video codec
    pub async fn set_video_codec(&self, codec: Box<dyn VideoCodec>) {
        let mut video_codec = self.video_codec.write().await;
        *video_codec = Some(codec);
    }
    
    /// Get audio codec
    pub async fn audio_codec(&self) -> Option<Box<dyn AudioCodec>> {
        let audio_codec = self.audio_codec.read().await;
        audio_codec.as_ref().map(|c| c.box_clone() as Box<dyn AudioCodec>)
    }
    
    /// Get video codec
    pub async fn video_codec(&self) -> Option<Box<dyn VideoCodec>> {
        let video_codec = self.video_codec.read().await;
        video_codec.as_ref().map(|c| c.box_clone() as Box<dyn VideoCodec>)
    }
    
    /// Send media data
    pub async fn send_media(&self, media_type: MediaType, data: Bytes) -> Result<()> {
        // Check if session is active
        {
            let state = self.state.read().await;
            if *state != MediaState::Active {
                return Err(Error::InvalidState(format!(
                    "Cannot send media in state: {:?}", *state
                )));
            }
        }
        
        // Process media with codec
        let processed_data = match media_type {
            MediaType::Audio => {
                let audio_codec = self.audio_codec.read().await;
                if let Some(codec) = audio_codec.as_ref() {
                    // In a real implementation, this would encode the audio
                    // For now, just pass it through
                    data
                } else {
                    return Err(Error::CodecNotFound("No audio codec configured".to_string()));
                }
            },
            MediaType::Video => {
                let video_codec = self.video_codec.read().await;
                if let Some(codec) = video_codec.as_ref() {
                    // In a real implementation, this would encode the video
                    // For now, just pass it through
                    data
                } else {
                    return Err(Error::CodecNotFound("No video codec configured".to_string()));
                }
            },
        };
        
        // Send via RTP - will need to implement this based on actual RTP session API
        // self.rtp_session.send_media(processed_data)
        Ok(())
    }
    
    /// Set remote address
    pub fn set_remote_address(&self, addr: SocketAddr) -> Result<()> {
        // TODO: Configure RTP session with remote address
        // self.rtp_session.set_remote_addr(addr)
        Ok(())
    }
    
    /// Get session metrics
    pub fn metrics(&self) -> SessionMetrics {
        let metrics = self.metrics.lock().unwrap();
        metrics.clone()
    }
    
    /// Get session duration
    pub fn duration(&self) -> Duration {
        self.start_time.elapsed()
    }
    
    /// Get session events
    pub fn events(&self) -> mpsc::UnboundedReceiver<MediaSessionEvent> {
        // TODO: This should return a receiver from the actual RTP session
        // For now, create a dummy receiver
        let (_, rx) = mpsc::unbounded_channel();
        rx
    }
} 