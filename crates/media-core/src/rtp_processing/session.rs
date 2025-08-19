//! Media session management (moved from rtp-core)

use std::collections::HashMap;
use std::time::Instant;

/// Configuration for a media session
#[derive(Debug, Clone)]
pub struct MediaSessionConfig {
    /// Session identifier
    pub session_id: String,
    
    /// Clock rate for media
    pub clock_rate: u32,
    
    /// Number of channels
    pub channels: u8,
    
    /// Payload type
    pub payload_type: u8,
    
    /// Enable quality monitoring
    pub enable_quality_monitoring: bool,
}

/// Media session state
#[derive(Debug, Clone)]
pub enum MediaSessionState {
    /// Session is initializing
    Initializing,
    
    /// Session is active
    Active,
    
    /// Session is paused/on hold
    Paused,
    
    /// Session is terminating
    Terminating,
    
    /// Session is terminated
    Terminated,
}

/// Media session for handling RTP streams
pub struct MediaSession {
    config: MediaSessionConfig,
    state: MediaSessionState,
    created_at: Instant,
    statistics: MediaSessionStatistics,
}

/// Statistics for a media session
#[derive(Debug, Clone)]
pub struct MediaSessionStatistics {
    /// Total packets received
    pub packets_received: u64,
    
    /// Total packets sent
    pub packets_sent: u64,
    
    /// Total bytes received
    pub bytes_received: u64,
    
    /// Total bytes sent
    pub bytes_sent: u64,
    
    /// Current jitter estimate
    pub jitter_ms: f32,
    
    /// Packet loss rate
    pub loss_rate: f32,
}

impl MediaSession {
    /// Create a new media session
    pub fn new(config: MediaSessionConfig) -> Self {
        Self {
            config,
            state: MediaSessionState::Initializing,
            created_at: Instant::now(),
            statistics: MediaSessionStatistics {
                packets_received: 0,
                packets_sent: 0,
                bytes_received: 0,
                bytes_sent: 0,
                jitter_ms: 0.0,
                loss_rate: 0.0,
            },
        }
    }
    
    /// Start the media session
    pub fn start(&mut self) -> Result<(), String> {
        if matches!(self.state, MediaSessionState::Initializing) {
            self.state = MediaSessionState::Active;
            Ok(())
        } else {
            Err("Session not in initializing state".to_string())
        }
    }
    
    /// Pause the media session
    pub fn pause(&mut self) -> Result<(), String> {
        if matches!(self.state, MediaSessionState::Active) {
            self.state = MediaSessionState::Paused;
            Ok(())
        } else {
            Err("Session not active".to_string())
        }
    }
    
    /// Resume the media session
    pub fn resume(&mut self) -> Result<(), String> {
        if matches!(self.state, MediaSessionState::Paused) {
            self.state = MediaSessionState::Active;
            Ok(())
        } else {
            Err("Session not paused".to_string())
        }
    }
    
    /// Terminate the media session
    pub fn terminate(&mut self) {
        self.state = MediaSessionState::Terminated;
    }
    
    /// Get session state
    pub fn state(&self) -> &MediaSessionState {
        &self.state
    }
    
    /// Get session statistics
    pub fn statistics(&self) -> &MediaSessionStatistics {
        &self.statistics
    }
    
    /// Record a received packet
    pub fn record_received_packet(&mut self, size_bytes: usize) {
        self.statistics.packets_received += 1;
        self.statistics.bytes_received += size_bytes as u64;
    }
    
    /// Record a sent packet
    pub fn record_sent_packet(&mut self, size_bytes: usize) {
        self.statistics.packets_sent += 1;
        self.statistics.bytes_sent += size_bytes as u64;
    }
}