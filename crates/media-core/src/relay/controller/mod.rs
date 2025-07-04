//! Media Session Controller for Session-Core Integration
//!
//! This module provides the high-level interface for session-core to control
//! media sessions. It manages the lifecycle of media sessions tied to SIP dialogs.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{RwLock, mpsc};
use tracing::{debug, error, info, warn};
use rand::Rng;

use crate::error::{Error, Result};
use crate::types::{DialogId, MediaSessionId, AudioFrame, payload_types};
use crate::types::conference::{
    ParticipantId, AudioStream, ConferenceMixingConfig, ConferenceMixingStats,
    ConferenceError, ConferenceResult, ConferenceMixingEvent, MixingQuality
};
use crate::processing::audio::{AudioMixer, AudioStreamManager};
use crate::quality::QualityMonitor;
use crate::performance::{
    metrics::PerformanceMetrics,
    pool::{AudioFramePool, PoolConfig, PoolStats, RtpBufferPool, PooledRtpBuffer},
    simd::SimdProcessor,
    zero_copy::ZeroCopyAudioFrame,
};
use crate::processing::audio::{
    AdvancedVoiceActivityDetector, AdvancedVadConfig,
    AdvancedAutomaticGainControl, AdvancedAgcConfig,
    AdvancedAcousticEchoCanceller, AdvancedAecConfig,
    AdvancedVadResult, AdvancedAgcResult, AdvancedAecResult
};
use crate::codec::audio::{G711Codec, G711Config, G711Variant};
use crate::types::SampleRate;

use rvoip_rtp_core::{RtpSession, RtpSessionConfig};
use rvoip_rtp_core::session::{RtpSessionStats, RtpStreamStats};
use rvoip_rtp_core::transport::{GlobalPortAllocator, PortAllocator, PortAllocatorConfig, AllocationStrategy};
use rvoip_rtp_core as rtp_core;
use rvoip_rtp_core::{RtpPacket, RtpHeader};

use super::{MediaRelay, RelaySessionConfig, RelayEvent, RelayStats, generate_session_id, create_relay_config};

// Sub-modules
pub mod types;
pub mod audio_generation;
pub mod rtp_management;
pub mod statistics;
pub mod advanced_processing;
pub mod conference;
pub mod zero_copy;
pub mod relay;

#[cfg(test)]
mod tests;

// Re-export important types
pub use types::{
    MediaConfig, MediaSessionStatus, MediaSessionInfo, MediaSessionEvent,
    AdvancedProcessorConfig, AdvancedProcessorSet
};

use types::RtpSessionWrapper;
use audio_generation::{AudioGenerator, AudioTransmitter};

/// Media Session Controller for managing media sessions and conference audio mixing
pub struct MediaSessionController {
    /// Underlying media relay (optional)
    relay: Option<Arc<MediaRelay>>,
    /// Active media sessions indexed by dialog ID
    pub(super) sessions: RwLock<HashMap<DialogId, MediaSessionInfo>>,
    /// Active RTP sessions indexed by dialog ID
    pub(super) rtp_sessions: RwLock<HashMap<DialogId, RtpSessionWrapper>>,
    /// Event channel for media session events
    pub(super) event_tx: mpsc::UnboundedSender<MediaSessionEvent>,
    /// Event receiver (taken by the user)
    event_rx: RwLock<Option<mpsc::UnboundedReceiver<MediaSessionEvent>>>,
    /// Audio mixer for conference calls
    pub(super) audio_mixer: Option<Arc<AudioMixer>>,
    /// Conference mixing configuration
    pub(super) conference_config: ConferenceMixingConfig,
    /// Conference event sender
    pub(super) conference_event_tx: mpsc::UnboundedSender<ConferenceMixingEvent>,
    /// Conference event receiver
    conference_event_rx: RwLock<Option<mpsc::UnboundedReceiver<ConferenceMixingEvent>>>,
    /// Quality monitor for conference sessions
    pub(super) quality_monitor: Option<Arc<QualityMonitor>>,
    
    // Performance library integration fields
    /// Global performance metrics for all sessions
    pub(super) performance_metrics: Arc<RwLock<PerformanceMetrics>>,
    /// Global frame pool for efficient allocation (shared across sessions)
    pub(super) frame_pool: Arc<AudioFramePool>,
    /// RTP output buffer pool for zero-copy encoding
    pub(super) rtp_buffer_pool: Arc<RtpBufferPool>,
    /// Advanced processors per dialog
    pub(super) advanced_processors: RwLock<HashMap<DialogId, AdvancedProcessorSet>>,
    /// Default configuration for advanced processors
    pub(super) default_processor_config: AdvancedProcessorConfig,
    /// G.711 codec for zero-copy audio processing
    pub(super) g711_codec: Arc<tokio::sync::Mutex<crate::codec::audio::G711Codec>>,
    /// SIMD processor for audio operations
    pub(super) simd_processor: SimdProcessor,
}

impl MediaSessionController {
    /// Create a new media session controller
    pub fn new() -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (conference_event_tx, conference_event_rx) = mpsc::unbounded_channel();
        
        // Initialize performance components
        let performance_metrics = Arc::new(RwLock::new(PerformanceMetrics::new()));
        
        // Create global frame pool (shared across sessions)
        let pool_config = PoolConfig {
            initial_size: 32,
            max_size: 128,
            sample_rate: 8000,
            channels: 1,
            samples_per_frame: 160, // 20ms at 8kHz
        };
        let frame_pool: Arc<AudioFramePool> = AudioFramePool::new(pool_config);
        
        // Create RTP buffer pool
        let rtp_buffer_pool = RtpBufferPool::new(
            480, // Buffer size: max G.711 frame size (60ms at 8kHz)
            32,  // Initial buffer count (more for conference)
            128  // Max buffer count (more for conference)
        );
        
        // Default advanced processor configuration
        let default_processor_config = AdvancedProcessorConfig::default();
        
        // Create G.711 codec for zero-copy processing
        let g711_codec = Arc::new(tokio::sync::Mutex::new(
            G711Codec::new(
                SampleRate::Rate8000,
                1,
                G711Config {
                    variant: G711Variant::MuLaw,
                    sample_rate: 8000,
                    channels: 1,
                    frame_size_ms: 20.0,
                }
            ).expect("Failed to create G.711 codec")
        ));
        
        // Create SIMD processor
        let simd_processor = SimdProcessor::new();
        
        Self {
            relay: None,
            sessions: RwLock::new(HashMap::new()),
            rtp_sessions: RwLock::new(HashMap::new()),
            event_tx,
            event_rx: RwLock::new(Some(event_rx)),
            audio_mixer: None,
            conference_config: ConferenceMixingConfig::default(),
            conference_event_tx,
            conference_event_rx: RwLock::new(Some(conference_event_rx)),
            quality_monitor: None,
            // Performance fields
            performance_metrics,
            frame_pool,
            rtp_buffer_pool,
            advanced_processors: RwLock::new(HashMap::new()),
            default_processor_config,
            g711_codec,
            simd_processor,
        }
    }
    
    /// Create a new media session controller with custom port range (deprecated - use new() instead)
    pub fn with_port_range(_base_port: u16, _max_port: u16) -> Self {
        // Port allocation is now handled by rtp-core's GlobalPortAllocator
        // These parameters are ignored for compatibility
        Self::new()
    }
    
    /// Create a new media session controller with conference audio mixing enabled
    pub async fn with_conference_mixing(
        _base_port: u16, 
        _max_port: u16, 
        conference_config: ConferenceMixingConfig
    ) -> Result<Self> {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (conference_event_tx, conference_event_rx) = mpsc::unbounded_channel();
        
        // Create audio mixer with the provided configuration
        let audio_mixer = Arc::new(AudioMixer::new(conference_config.clone()).await?);
        
        // Set up conference event forwarding
        audio_mixer.set_event_sender(conference_event_tx.clone()).await;
        
        // Initialize performance components
        let performance_metrics = Arc::new(RwLock::new(PerformanceMetrics::new()));
        
        // Create global frame pool with larger capacity for conference mixing
        let pool_config = PoolConfig {
            initial_size: 64, // Larger pool for conference mixing
            max_size: 256,
            sample_rate: conference_config.output_sample_rate,
            channels: conference_config.output_channels as u8,
            samples_per_frame: conference_config.output_samples_per_frame as usize,
        };
        let frame_pool: Arc<AudioFramePool> = AudioFramePool::new(pool_config);
        
        // Create RTP buffer pool
        let rtp_buffer_pool = RtpBufferPool::new(
            480, // Buffer size: max G.711 frame size (60ms at 8kHz)
            32,  // Initial buffer count (more for conference)
            128  // Max buffer count (more for conference)
        );
        
        // Default advanced processor configuration for conference
        let mut default_processor_config = AdvancedProcessorConfig::default();
        default_processor_config.frame_pool_size = 32; // Per-session pool size
        default_processor_config.enable_simd = conference_config.enable_simd_optimization;
        
        // Create G.711 codec for zero-copy processing
        let g711_codec = Arc::new(tokio::sync::Mutex::new(
            G711Codec::new(
                SampleRate::Rate8000,
                1,
                G711Config {
                    variant: G711Variant::MuLaw,
                    sample_rate: 8000,
                    channels: 1,
                    frame_size_ms: 20.0,
                }
            ).expect("Failed to create G.711 codec")
        ));
        
        // Create SIMD processor
        let simd_processor = SimdProcessor::new();
        
        Ok(Self {
            relay: None,
            sessions: RwLock::new(HashMap::new()),
            rtp_sessions: RwLock::new(HashMap::new()),
            event_tx,
            event_rx: RwLock::new(Some(event_rx)),
            audio_mixer: Some(audio_mixer),
            conference_config,
            conference_event_tx,
            conference_event_rx: RwLock::new(Some(conference_event_rx)),
            quality_monitor: None,
            // Performance fields
            performance_metrics,
            frame_pool,
            rtp_buffer_pool,
            advanced_processors: RwLock::new(HashMap::new()),
            default_processor_config,
            g711_codec,
            simd_processor,
        })
    }
    
    /// Start a media session for a dialog
    pub async fn start_media(&self, dialog_id: DialogId, config: MediaConfig) -> Result<()> {
        info!("Starting media session for dialog: {}", dialog_id);
        
        // Check if media session already exists for this dialog
        {
            let sessions = self.sessions.read().await;
            if sessions.contains_key(&dialog_id) {
                return Err(Error::config(format!("Media session already exists for dialog: {}", dialog_id)));
            }
        }

        // Allocate RTP port using rtp-core's dynamic allocator
        let global_allocator = GlobalPortAllocator::instance().await;
        let dialog_session_id = format!("dialog_{}", dialog_id);
        let (local_rtp_addr, _) = global_allocator
            .allocate_port_pair(&dialog_session_id, Some(config.local_addr.ip()))
            .await
            .map_err(|e| Error::config(format!("Failed to allocate RTP port: {}", e)))?;
        
        let rtp_port = local_rtp_addr.port();
        
        // Create RTP session configuration
        let rtp_config = RtpSessionConfig {
            local_addr: local_rtp_addr,
            remote_addr: config.remote_addr,
            ssrc: Some(rand::random()), // Generate random SSRC
            payload_type: 0, // Default to PCMU
            clock_rate: 8000, // Default to 8kHz
            jitter_buffer_size: Some(50),
            max_packet_age_ms: Some(200),
            enable_jitter_buffer: true,
        };
        
        // Create actual RTP session
        let rtp_session = RtpSession::new(rtp_config).await
            .map_err(|e| Error::config(format!("Failed to create RTP session: {}", e)))?;
        
        // Wrap RTP session
        let rtp_wrapper = RtpSessionWrapper {
            session: Arc::new(tokio::sync::Mutex::new(rtp_session)),
            local_addr: local_rtp_addr,
            remote_addr: config.remote_addr,
            created_at: std::time::Instant::now(),
            audio_transmitter: None,
            transmission_enabled: false,
        };
        
        // Create media session info
        let session_info = MediaSessionInfo {
            dialog_id: dialog_id.clone(),
            status: MediaSessionStatus::Active,
            config: config.clone(),
            rtp_port: Some(rtp_port),
            relay_session_ids: None,
            stats: None,
            rtp_stats: None,
            stats_updated_at: None,
            created_at: std::time::Instant::now(),
        };

        // Store session and RTP session
        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(dialog_id.clone(), session_info);
        }
        
        {
            let mut rtp_sessions = self.rtp_sessions.write().await;
            rtp_sessions.insert(dialog_id.clone(), rtp_wrapper);
        }

        // Send event
        let _ = self.event_tx.send(MediaSessionEvent::SessionCreated {
            dialog_id: dialog_id.clone(),
            session_id: dialog_id.clone(),
        });

        info!("✅ Created media session with REAL RTP session: {} (port: {})", dialog_id, rtp_port);
        Ok(())
    }
    
    /// Stop media session for a dialog
    pub async fn stop_media(&self, dialog_id: &DialogId) -> Result<()> {
        info!("Stopping media session for dialog: {}", dialog_id);

        // Remove session and get info for cleanup
        let session_info = {
            let mut sessions = self.sessions.write().await;
            sessions.remove(dialog_id)
                .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?
        };
        
        // Stop and remove RTP session
        {
            let mut rtp_sessions = self.rtp_sessions.write().await;
            if let Some(rtp_wrapper) = rtp_sessions.remove(dialog_id) {
                // Close the RTP session
                let mut rtp_session = rtp_wrapper.session.lock().await;
                let _ = rtp_session.close().await;
                info!("✅ Stopped RTP session for dialog: {}", dialog_id);
            }
        }

        // Clean up relay if exists
        if let Some((session_a, session_b)) = &session_info.relay_session_ids {
            if let Some(relay) = &self.relay {
                let _ = relay.remove_session_pair(session_a, session_b).await;
            }
        }

        // Release port via GlobalPortAllocator
        if session_info.rtp_port.is_some() {
            let global_allocator = GlobalPortAllocator::instance().await;
            let dialog_session_id = format!("dialog_{}", dialog_id);
            if let Err(e) = global_allocator.release_session(&dialog_session_id).await {
                warn!("Failed to release ports for dialog {}: {}", dialog_id, e);
            }
        }

        // Clean up advanced processors if they exist
        {
            let mut processors = self.advanced_processors.write().await;
            if processors.remove(dialog_id).is_some() {
                info!("🧹 Cleaned up advanced processors for dialog: {}", dialog_id);
            }
        }

        // Send event
        let _ = self.event_tx.send(MediaSessionEvent::SessionDestroyed {
            dialog_id: dialog_id.clone(),
            session_id: dialog_id.clone(),
        });

        Ok(())
    }
    
    /// Update media configuration (e.g., when remote address becomes known)
    pub async fn update_media(&self, dialog_id: DialogId, config: MediaConfig) -> Result<()> {
        debug!("Updating media session for dialog: {}", dialog_id);
        
        let mut sessions = self.sessions.write().await;
        let session_info = sessions.get_mut(&dialog_id)
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;
        
        // Update configuration
        let old_remote = session_info.config.remote_addr;
        session_info.config = config.clone();
        
        // If remote address was set/changed, update the RTP session
        if config.remote_addr != old_remote {
            if let Some(remote_addr) = config.remote_addr {
                // Update the RTP session's remote address
                let mut rtp_sessions = self.rtp_sessions.write().await;
                if let Some(rtp_wrapper) = rtp_sessions.get_mut(&dialog_id) {
                    // Update the wrapper's remote address
                    rtp_wrapper.remote_addr = Some(remote_addr);
                    
                    // Update the actual RTP session
                    let mut rtp_session = rtp_wrapper.session.lock().await;
                    rtp_session.set_remote_addr(remote_addr).await;
                    
                    info!("✅ Updated RTP session remote address for dialog {}: {}", dialog_id, remote_addr);
                }
                
                // Emit event
                let _ = self.event_tx.send(MediaSessionEvent::RemoteAddressUpdated {
                    dialog_id: dialog_id.clone(),
                    remote_addr,
                });
            }
        }
        
        debug!("Media session updated for dialog: {}", dialog_id);
        Ok(())
    }
    
    /// Get information about a media session
    pub async fn get_session_info(&self, dialog_id: &DialogId) -> Option<MediaSessionInfo> {
        let sessions = self.sessions.read().await;
        let mut info = sessions.get(dialog_id).cloned()?;
        
        // Add current RTP statistics
        info.rtp_stats = self.get_rtp_statistics(dialog_id).await;
        info.stats_updated_at = Some(Instant::now());
        
        Some(info)
    }
    
    /// Get all active sessions
    pub async fn get_all_sessions(&self) -> Vec<MediaSessionInfo> {
        let sessions = self.sessions.read().await;
        sessions.values().cloned().collect()
    }
    
    /// Get event receiver (can only be called once)
    pub async fn take_event_receiver(&self) -> Option<mpsc::UnboundedReceiver<MediaSessionEvent>> {
        let mut event_rx = self.event_rx.write().await;
        event_rx.take()
    }
}

impl Default for MediaSessionController {
    fn default() -> Self {
        Self::new()
    }
}

// Implementation modules are in separate files 