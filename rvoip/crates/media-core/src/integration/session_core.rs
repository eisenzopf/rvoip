//! Integration with session-core
//!
//! This module provides the main interface for controlling media sessions
//! from session-core.

use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::{RwLock, mpsc};
use uuid::Uuid;
use tracing::{debug, error, info, warn};
use std::sync::Mutex;

use crate::error::{Error, Result};
use crate::session::{MediaSession, MediaSessionId, MediaType, MediaSessionConfig};
use super::MediaFactoryConfig;
use crate::codec::registry::CodecRegistry;
use crate::codec::traits::{Codec, CodecCapability};
use crate::rtp::session::{RtpSession, RtpSessionConfig, RtpSessionEvent};
use crate::session::events::MediaSessionEvent;

/// Media manager configuration
#[derive(Debug, Clone)]
pub struct MediaManagerConfig {
    /// Maximum number of concurrent sessions
    pub max_sessions: usize,
    /// Whether to use hardware acceleration when available
    pub use_hardware_acceleration: bool,
    /// Preferred audio codecs in order of preference
    pub preferred_audio_codecs: Vec<String>,
    /// Preferred video codecs in order of preference
    pub preferred_video_codecs: Vec<String>,
    /// Whether to include video capability
    pub enable_video: bool,
    /// Bind address for RTP
    pub rtp_bind_address: String,
    /// Port range for RTP
    pub rtp_port_range: (u16, u16),
}

impl Default for MediaManagerConfig {
    fn default() -> Self {
        Self {
            max_sessions: 10,
            use_hardware_acceleration: true,
            preferred_audio_codecs: vec![
                "OPUS".to_string(),
                "G722".to_string(),
                "PCMA".to_string(),
                "PCMU".to_string(),
            ],
            preferred_video_codecs: vec![
                "VP8".to_string(),
                "H264".to_string(),
            ],
            enable_video: false,
            rtp_bind_address: "0.0.0.0".to_string(),
            rtp_port_range: (10000, 20000),
        }
    }
}

/// Media session state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaSessionState {
    /// Session is being initialized
    Initializing,
    /// Session is idle (no media flowing)
    Idle,
    /// Session is active (media flowing)
    Active,
    /// Session is on hold
    OnHold,
    /// Session has ended
    Ended,
    /// Session has failed
    Failed,
}

/// Media manager event
#[derive(Debug, Clone)]
pub enum MediaManagerEvent {
    /// A new session has been created
    SessionCreated(String),
    /// A session has been updated
    SessionUpdated(String),
    /// A session has ended
    SessionEnded(String),
    /// A session has failed
    SessionFailed {
        /// Session ID
        session_id: String,
        /// Error message
        error: String,
    },
    /// DTMF received
    DtmfReceived {
        /// Session ID
        session_id: String,
        /// DTMF digit
        digit: char,
    },
}

/// Media session information
#[derive(Debug, Clone)]
pub struct MediaSessionInfo {
    /// Session ID
    pub session_id: String,
    /// Associated dialog ID
    pub dialog_id: String,
    /// Session state
    pub state: MediaSessionState,
    /// Audio codec in use
    pub audio_codec: Option<String>,
    /// Video codec in use
    pub video_codec: Option<String>,
    /// Media direction
    pub direction: MediaDirection,
    /// Local media port
    pub local_port: u16,
    /// Remote media address
    pub remote_address: Option<String>,
    /// Remote media port
    pub remote_port: Option<u16>,
    /// Session creation time
    pub created_at: std::time::SystemTime,
}

/// Media direction
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaDirection {
    /// Send and receive media
    SendRecv,
    /// Send media only
    SendOnly,
    /// Receive media only
    RecvOnly,
    /// Inactive (no media)
    Inactive,
}

/// Media manager for integration with session-core
pub struct MediaManager {
    /// Configuration
    config: MediaManagerConfig,
    /// Media sessions
    sessions: RwLock<HashMap<String, Arc<MediaSession>>>,
    /// Session info
    session_info: RwLock<HashMap<String, MediaSessionInfo>>,
    /// Port allocator
    port_allocator: Mutex<PortAllocator>,
    /// Codec registry
    codec_registry: Arc<CodecRegistry>,
    /// Event sender
    event_sender: mpsc::UnboundedSender<MediaManagerEvent>,
    /// Event receiver
    event_receiver: Mutex<Option<mpsc::UnboundedReceiver<MediaManagerEvent>>>,
}

/// Port allocator for RTP sessions
struct PortAllocator {
    /// Available ports
    available_ports: Vec<u16>,
    /// Allocated ports
    allocated_ports: HashMap<String, u16>,
}

impl PortAllocator {
    /// Create a new port allocator
    fn new(port_range: (u16, u16)) -> Self {
        let mut available_ports = Vec::new();
        
        // Only use even ports for RTP (RTCP will use the next odd port)
        for port in (port_range.0..=port_range.1).step_by(2) {
            available_ports.push(port);
        }
        
        Self {
            available_ports,
            allocated_ports: HashMap::new(),
        }
    }
    
    /// Allocate a port for a session
    fn allocate(&mut self, session_id: &str) -> Option<u16> {
        if self.available_ports.is_empty() {
            return None;
        }
        
        let port = self.available_ports.remove(0);
        self.allocated_ports.insert(session_id.to_string(), port);
        
        Some(port)
    }
    
    /// Release a port
    fn release(&mut self, session_id: &str) {
        if let Some(port) = self.allocated_ports.remove(session_id) {
            self.available_ports.push(port);
            self.available_ports.sort_unstable();
        }
    }
    
    /// Get the allocated port for a session
    fn get_port(&self, session_id: &str) -> Option<u16> {
        self.allocated_ports.get(session_id).copied()
    }
}

impl MediaManager {
    /// Create a new media manager
    pub fn new(config: MediaManagerConfig) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        
        Self {
            config,
            sessions: RwLock::new(HashMap::new()),
            session_info: RwLock::new(HashMap::new()),
            port_allocator: Mutex::new(PortAllocator::new(config.rtp_port_range)),
            codec_registry: Arc::new(CodecRegistry::new()),
            event_sender: tx,
            event_receiver: Mutex::new(Some(rx)),
        }
    }
    
    /// Create a new media session for a dialog
    pub fn create_session(&self, dialog_id: &str) -> Result<String> {
        let sessions = self.sessions.read().unwrap();
        
        // Check if we've reached the maximum number of sessions
        if sessions.len() >= self.config.max_sessions {
            return Err(std::io::Error::new(
                std::io::ErrorKind::ResourceExhausted,
                "Maximum number of media sessions reached"
            ).into());
        }
        
        // Generate a unique session ID
        let session_id = Uuid::new_v4().to_string();
        
        // Drop read lock
        drop(sessions);
        
        // Allocate a port
        let port = {
            let mut allocator = self.port_allocator.lock().unwrap();
            allocator.allocate(&session_id).ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::AddrNotAvailable,
                    "No available ports for media session"
                )
            })?
        };
        
        // Create session info
        let session_info = MediaSessionInfo {
            session_id: session_id.clone(),
            dialog_id: dialog_id.to_string(),
            state: MediaSessionState::Initializing,
            audio_codec: None,
            video_codec: None,
            direction: MediaDirection::SendRecv,
            local_port: port,
            remote_address: None,
            remote_port: None,
            created_at: std::time::SystemTime::now(),
        };
        
        // Store session info
        {
            let mut info = self.session_info.write().unwrap();
            info.insert(session_id.clone(), session_info);
        }
        
        // Send session created event
        let _ = self.event_sender.send(MediaManagerEvent::SessionCreated(session_id.clone()));
        
        // Note: Actual media session creation happens during offer/answer
        
        info!("Created media session: id={}, dialog={}", session_id, dialog_id);
        
        Ok(session_id)
    }
    
    /// End a media session
    pub fn end_session(&self, session_id: &str) -> Result<()> {
        // Get the session
        let session = {
            let sessions = self.sessions.read().unwrap();
            sessions.get(session_id).cloned()
        };
        
        if let Some(session) = session {
            // End the session
            session.stop()?;
            
            // Remove from sessions
            {
                let mut sessions = self.sessions.write().unwrap();
                sessions.remove(session_id);
            }
            
            // Update session info
            {
                let mut info = self.session_info.write().unwrap();
                if let Some(info) = info.get_mut(session_id) {
                    info.state = MediaSessionState::Ended;
                }
            }
            
            // Release the port
            {
                let mut allocator = self.port_allocator.lock().unwrap();
                allocator.release(session_id);
            }
            
            // Send session ended event
            let _ = self.event_sender.send(MediaManagerEvent::SessionEnded(session_id.to_string()));
            
            info!("Ended media session: id={}", session_id);
        } else {
            warn!("Attempted to end non-existent media session: id={}", session_id);
        }
        
        Ok(())
    }
    
    /// Get session information
    pub fn get_session_info(&self, session_id: &str) -> Option<MediaSessionInfo> {
        let info = self.session_info.read().unwrap();
        info.get(session_id).cloned()
    }
    
    /// Get all sessions for a dialog
    pub fn get_sessions_for_dialog(&self, dialog_id: &str) -> Vec<MediaSessionInfo> {
        let info = self.session_info.read().unwrap();
        info.values()
            .filter(|s| s.dialog_id == dialog_id)
            .cloned()
            .collect()
    }
    
    /// Get all sessions
    pub fn get_all_sessions(&self) -> Vec<MediaSessionInfo> {
        let info = self.session_info.read().unwrap();
        info.values().cloned().collect()
    }
    
    /// Get the event receiver
    pub fn get_event_receiver(&self) -> mpsc::UnboundedReceiver<MediaManagerEvent> {
        let mut rx = self.event_receiver.lock().unwrap();
        rx.take().expect("Event receiver already taken")
    }
    
    /// Process an incoming SDP offer
    pub fn process_offer(&self, session_id: &str, sdp_offer: &str) -> Result<String> {
        // Parse the offer using the SDP module
        let offer = crate::integration::sdp::SdpHandler::parse_sdp(sdp_offer)?;
        
        // Find matching codecs
        let audio_codecs = offer.get_audio_codecs();
        let video_codecs = offer.get_video_codecs();
        
        let selected_audio = self.select_codec(MediaType::Audio, &audio_codecs)?;
        let selected_video = if self.config.enable_video {
            self.select_codec(MediaType::Video, &video_codecs).ok()
        } else {
            None
        };
        
        // Get session info
        let mut session_info = {
            let info = self.session_info.read().unwrap();
            info.get(session_id).cloned().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("Media session not found: {}", session_id)
                )
            })?
        };
        
        // Update session info
        session_info.audio_codec = selected_audio.as_ref().map(|c| c.capability().name);
        session_info.video_codec = selected_video.as_ref().map(|c| c.capability().name);
        session_info.remote_address = Some(offer.remote_address.clone());
        session_info.remote_port = Some(offer.audio_port);
        session_info.direction = match offer.audio_direction.as_str() {
            "sendrecv" => MediaDirection::SendRecv,
            "sendonly" => MediaDirection::RecvOnly, // They send only, we receive only
            "recvonly" => MediaDirection::SendOnly, // They receive only, we send only
            "inactive" => MediaDirection::Inactive,
            _ => MediaDirection::SendRecv,
        };
        
        // Store updated session info
        {
            let mut info = self.session_info.write().unwrap();
            info.insert(session_id.to_string(), session_info.clone());
        }
        
        // Create media session if it doesn't exist
        let session = {
            let sessions = self.sessions.read().unwrap();
            sessions.get(session_id).cloned()
        };
        
        if session.is_none() {
            // Create RTP session for audio
            let rtp_config = RtpSessionConfig {
                local_addr: format!("{}:{}", self.config.rtp_bind_address, session_info.local_port).parse()?,
                remote_addr: Some(format!("{}:{}", offer.remote_address, offer.audio_port).parse()?),
                ssrc: rand::random(),
                payload_type: selected_audio.as_ref().map(|c| c.capability().payload_type.unwrap_or(96)).unwrap_or(0),
                clock_rate: selected_audio.as_ref().map(|c| c.capability().clock_rate).unwrap_or(8000),
                // More configuration would be needed here
                ..Default::default()
            };
            
            let (rtp_session, rtp_events) = RtpSession::new(rtp_config).await?;
            
            // Create media session
            let media_config = MediaSessionConfig {
                session_id: session_id.to_string(),
                // More configuration would be needed here
                ..Default::default()
            };
            
            let media_session = MediaSession::new(media_config, rtp_session)?;
            
            // Set codecs
            if let Some(audio_codec) = selected_audio {
                media_session.set_audio_codec(audio_codec);
            }
            
            if let Some(video_codec) = selected_video {
                media_session.set_video_codec(video_codec);
            }
            
            // Store the session
            {
                let mut sessions = self.sessions.write().unwrap();
                sessions.insert(session_id.to_string(), Arc::new(media_session));
            }
            
            // Start event processing
            let session_id_clone = session_id.to_string();
            let event_sender = self.event_sender.clone();
            
            tokio::spawn(async move {
                Self::process_rtp_events(session_id_clone, rtp_events, event_sender).await;
            });
        }
        
        // Generate answer
        let answer = crate::integration::sdp::SdpHandler::generate_answer(
            &offer,
            session_info.local_port,
            selected_audio.as_ref(),
            selected_video.as_ref(),
        )?;
        
        // Send session updated event
        let _ = self.event_sender.send(MediaManagerEvent::SessionUpdated(session_id.to_string()));
        
        Ok(answer)
    }
    
    /// Process an incoming SDP answer
    pub fn process_answer(&self, session_id: &str, sdp_answer: &str) -> Result<()> {
        // Parse the answer
        let answer = crate::integration::sdp::SdpHandler::parse_sdp(sdp_answer)?;
        
        // Get session
        let session = {
            let sessions = self.sessions.read().unwrap();
            sessions.get(session_id).cloned().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("Media session not found: {}", session_id)
                )
            })?
        };
        
        // Update remote address if it has changed
        let remote_addr = format!("{}:{}", answer.remote_address, answer.audio_port).parse()?;
        session.set_remote_address(remote_addr)?;
        
        // Update session info
        {
            let mut info = self.session_info.write().unwrap();
            if let Some(info) = info.get_mut(session_id) {
                info.remote_address = Some(answer.remote_address);
                info.remote_port = Some(answer.audio_port);
                info.state = MediaSessionState::Active;
                
                // Update direction based on answer
                info.direction = match answer.audio_direction.as_str() {
                    "sendrecv" => MediaDirection::SendRecv,
                    "sendonly" => MediaDirection::RecvOnly,
                    "recvonly" => MediaDirection::SendOnly,
                    "inactive" => MediaDirection::Inactive,
                    _ => MediaDirection::SendRecv,
                };
            }
        }
        
        // Send session updated event
        let _ = self.event_sender.send(MediaManagerEvent::SessionUpdated(session_id.to_string()));
        
        Ok(())
    }
    
    /// Generate an SDP offer
    pub fn generate_offer(&self, session_id: &str) -> Result<String> {
        // Get session info
        let session_info = {
            let info = self.session_info.read().unwrap();
            info.get(session_id).cloned().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("Media session not found: {}", session_id)
                )
            })?
        };
        
        // Get available codecs
        let audio_codecs = self.get_supported_codecs(MediaType::Audio);
        let video_codecs = if self.config.enable_video {
            self.get_supported_codecs(MediaType::Video)
        } else {
            Vec::new()
        };
        
        // Generate offer
        let offer = crate::integration::sdp::SdpHandler::generate_offer(
            session_info.local_port,
            &audio_codecs,
            &video_codecs,
        )?;
        
        Ok(offer)
    }
    
    /// Process RTP session events
    async fn process_rtp_events(
        session_id: String,
        mut rx: mpsc::Receiver<RtpSessionEvent>,
        tx: mpsc::UnboundedSender<MediaManagerEvent>,
    ) {
        while let Some(event) = rx.recv().await {
            match event {
                RtpSessionEvent::DtmfReceived { digit, .. } => {
                    let _ = tx.send(MediaManagerEvent::DtmfReceived {
                        session_id: session_id.clone(),
                        digit,
                    });
                },
                RtpSessionEvent::Error(err) => {
                    error!("RTP session error: {}", err);
                    let _ = tx.send(MediaManagerEvent::SessionFailed {
                        session_id: session_id.clone(),
                        error: err.to_string(),
                    });
                },
                _ => {},
            }
        }
    }
    
    /// Select a codec based on capabilities
    fn select_codec(&self, media_type: MediaType, capabilities: &[CodecCapability]) -> Result<Box<dyn Codec>> {
        // Use the codec registry to select the best codec
        crate::codec::registry::negotiation::select_best_codec(
            &self.codec_registry,
            capabilities,
            media_type,
        )
    }
    
    /// Get supported codecs for a media type
    fn get_supported_codecs(&self, media_type: MediaType) -> Vec<CodecCapability> {
        // Use the codec registry to get capabilities
        match self.codec_registry.list_capabilities_by_type(media_type) {
            Ok(caps) => caps,
            Err(_) => Vec::new(),
        }
    }
} 