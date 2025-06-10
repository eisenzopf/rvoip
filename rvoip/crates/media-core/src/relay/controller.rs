//! Media Session Controller for Session-Core Integration
//!
//! This module provides the high-level interface for session-core to control
//! media sessions. It manages the lifecycle of media sessions tied to SIP dialogs.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tracing::{debug, error, info, warn};
use rand::Rng;

use crate::error::{Error, Result};
use super::{MediaRelay, RelaySessionConfig, RelayEvent, RelayStats, generate_session_id, create_relay_config};

// Import RTP session capabilities
use rvoip_rtp_core::{RtpSession, RtpSessionConfig};

// Audio generation imports
use std::time::{Duration, Instant};
use tokio::time::interval;

use crate::types::{AudioFrame, MediaPacket, DialogId, MediaSessionId, payload_types};
use crate::types::conference::{
    ParticipantId, AudioStream, ConferenceMixingConfig, ConferenceMixingStats,
    ConferenceError, ConferenceResult, ConferenceMixingEvent, MixingQuality
};
use crate::processing::audio::{AudioMixer, AudioStreamManager};
use crate::quality::QualityMonitor;
use bytes::Bytes;

// NEW: Performance library imports
use crate::performance::{
    metrics::PerformanceMetrics,
    pool::{AudioFramePool, PoolConfig, PoolStats, RtpBufferPool, PooledRtpBuffer},
    simd::SimdProcessor,
    zero_copy::ZeroCopyAudioFrame,
};

// NEW: Advanced v2 processor imports
use crate::processing::audio::{
    AdvancedVoiceActivityDetector, AdvancedVadConfig,
    AdvancedAutomaticGainControl, AdvancedAgcConfig,
    AdvancedAcousticEchoCanceller, AdvancedAecConfig,
    AdvancedVadResult, AdvancedAgcResult, AdvancedAecResult
};

// NEW: G.711 codec import for zero-copy processing
use crate::codec::audio::{G711Codec, G711Config, G711Variant};
use crate::types::SampleRate;

// Import rtp-core's sophisticated port allocator instead of implementing our own
use rvoip_rtp_core::transport::{GlobalPortAllocator, PortAllocator, PortAllocatorConfig, AllocationStrategy};

// NEW: Import rtp-core types for zero-copy processing
use rvoip_rtp_core as rtp_core;
use rvoip_rtp_core::{RtpPacket, RtpHeader};

/// Audio generator for creating test tones and audio streams
pub struct AudioGenerator {
    /// Sample rate (Hz)
    sample_rate: u32,
    /// Current phase for sine wave generation
    phase: f64,
    /// Frequency of the generated tone (Hz)
    frequency: f64,
    /// Amplitude (0.0 to 1.0)
    amplitude: f64,
}

impl AudioGenerator {
    /// Create a new audio generator
    pub fn new(sample_rate: u32, frequency: f64, amplitude: f64) -> Self {
        Self {
            sample_rate,
            phase: 0.0,
            frequency,
            amplitude,
        }
    }
    
    /// Generate audio samples for PCMU (G.711 μ-law) encoding
    pub fn generate_pcmu_samples(&mut self, num_samples: usize) -> Vec<u8> {
        let mut samples = Vec::with_capacity(num_samples);
        let phase_increment = 2.0 * std::f64::consts::PI * self.frequency / self.sample_rate as f64;
        
        for _ in 0..num_samples {
            // Generate sine wave sample
            let sample = (self.phase.sin() * self.amplitude * 32767.0) as i16;
            
            // Convert to μ-law (simplified implementation)
            let pcmu_sample = Self::linear_to_ulaw(sample);
            samples.push(pcmu_sample);
            
            // Update phase
            self.phase += phase_increment;
            if self.phase >= 2.0 * std::f64::consts::PI {
                self.phase -= 2.0 * std::f64::consts::PI;
            }
        }
        
        samples
    }
    
    /// Convert linear PCM to μ-law (G.711)
    fn linear_to_ulaw(pcm: i16) -> u8 {
        // Simplified μ-law encoding
        let sign = if pcm < 0 { 0x80u8 } else { 0x00u8 };
        let magnitude = pcm.abs() as u16;
        
        // Find the segment
        let mut segment = 0u8;
        let mut temp = magnitude >> 5;
        while temp != 0 && segment < 7 {
            segment += 1;
            temp >>= 1;
        }
        
        // Calculate quantization value
        let quantization = if segment == 0 {
            (magnitude >> 1) as u8
        } else {
            (((magnitude >> (segment + 1)) & 0x0F) + 0x10) as u8
        };
        
        // Combine sign, segment, and quantization
        sign | (segment << 4) | (quantization & 0x0F)
    }
}

/// Audio transmission task for RTP sessions
pub struct AudioTransmitter {
    /// RTP session for transmission
    rtp_session: Arc<tokio::sync::Mutex<RtpSession>>,
    /// Audio generator
    audio_generator: AudioGenerator,
    /// Transmission interval (20ms for standard audio)
    interval: Duration,
    /// Current RTP timestamp
    timestamp: u32,
    /// Samples per packet (160 samples for 20ms at 8kHz)
    samples_per_packet: usize,
    /// Whether transmission is active
    is_active: Arc<tokio::sync::RwLock<bool>>,
}

impl AudioTransmitter {
    /// Create a new audio transmitter
    pub fn new(rtp_session: Arc<tokio::sync::Mutex<RtpSession>>) -> Self {
        Self {
            rtp_session,
            audio_generator: AudioGenerator::new(8000, 440.0, 0.5), // 440Hz tone at 8kHz
            interval: Duration::from_millis(20), // 20ms packets
            timestamp: 0,
            samples_per_packet: 160, // 20ms * 8000 samples/sec = 160 samples
            is_active: Arc::new(tokio::sync::RwLock::new(false)),
        }
    }
    
    /// Start audio transmission
    pub async fn start(&mut self) {
        *self.is_active.write().await = true;
        info!("🎵 Started audio transmission (440Hz tone, 20ms packets)");
        
        let rtp_session = self.rtp_session.clone();
        let is_active = self.is_active.clone();
        let mut interval_timer = interval(self.interval);
        let mut timestamp = self.timestamp;
        let mut audio_gen = AudioGenerator::new(8000, 440.0, 0.5);
        
        tokio::spawn(async move {
            while *is_active.read().await {
                interval_timer.tick().await;
                
                // Generate audio samples
                let audio_samples = audio_gen.generate_pcmu_samples(160); // 160 samples for 20ms
                
                // Send RTP packet
                {
                    let mut session = rtp_session.lock().await;
                    if let Err(e) = session.send_packet(timestamp, bytes::Bytes::from(audio_samples), false).await {
                        error!("Failed to send RTP audio packet: {}", e);
                    } else {
                        debug!("📡 Sent RTP audio packet (timestamp: {}, 160 samples)", timestamp);
                    }
                }
                
                // Update timestamp (160 samples at 8kHz = 20ms)
                timestamp = timestamp.wrapping_add(160);
            }
            
            info!("🛑 Stopped audio transmission");
        });
    }
    
    /// Stop audio transmission
    pub async fn stop(&self) {
        *self.is_active.write().await = false;
        info!("🛑 Stopping audio transmission");
    }
    
    /// Check if transmission is active
    pub async fn is_active(&self) -> bool {
        *self.is_active.read().await
    }
}

/// Media configuration for a session
#[derive(Debug, Clone)]
pub struct MediaConfig {
    /// Local RTP address
    pub local_addr: SocketAddr,
    /// Remote RTP address (if known)
    pub remote_addr: Option<SocketAddr>,
    /// Preferred codec (for future implementation)
    pub preferred_codec: Option<String>,
    /// Additional media parameters
    pub parameters: HashMap<String, String>,
}

/// Media session status
#[derive(Debug, Clone, PartialEq)]
pub enum MediaSessionStatus {
    /// Session is being created
    Creating,
    /// Session is active and relaying media
    Active,
    /// Session is on hold
    OnHold,
    /// Session has ended
    Ended,
    /// Session failed
    Failed(String),
}

/// Information about an active media session
#[derive(Debug, Clone)]
pub struct MediaSessionInfo {
    /// Dialog ID this session is associated with
    pub dialog_id: DialogId,
    /// Media relay session IDs (if this is a relay session)
    pub relay_session_ids: Option<(String, String)>,
    /// Current status
    pub status: MediaSessionStatus,
    /// Media configuration
    pub config: MediaConfig,
    /// RTP port allocated for this session
    pub rtp_port: Option<u16>,
    /// Session statistics
    pub stats: Option<RelayStats>,
    /// Creation time
    pub created_at: std::time::Instant,
}

/// RTP session wrapper for MediaSessionController
struct RtpSessionWrapper {
    /// The actual RTP session
    session: Arc<tokio::sync::Mutex<RtpSession>>,
    /// Local RTP address
    local_addr: SocketAddr,
    /// Remote RTP address (if known)
    remote_addr: Option<SocketAddr>,
    /// Session creation time
    created_at: std::time::Instant,
    /// Audio transmitter for outgoing audio
    audio_transmitter: Option<AudioTransmitter>,
    /// Whether audio transmission is enabled
    transmission_enabled: bool,
}

impl Default for MediaSessionInfo {
    fn default() -> Self {
        Self {
            dialog_id: DialogId::new(""),
            relay_session_ids: None,
            status: MediaSessionStatus::Creating,
            config: MediaConfig {
                local_addr: SocketAddr::from(([0, 0, 0, 0], 0)),
                remote_addr: None,
                preferred_codec: None,
                parameters: HashMap::new(),
            },
            rtp_port: None,
            stats: None,
            created_at: std::time::Instant::now(),
        }
    }
}

/// Events emitted by the media session controller
#[derive(Debug, Clone)]
pub enum MediaSessionEvent {
    /// Media session created
    SessionCreated {
        dialog_id: DialogId,
        session_id: DialogId,
    },
    /// Media session destroyed
    SessionDestroyed {
        dialog_id: DialogId,
        session_id: DialogId,
    },
    /// Media session failed
    SessionFailed {
        dialog_id: DialogId,
        error: String,
    },
    /// Remote address updated
    RemoteAddressUpdated {
        dialog_id: DialogId,
        remote_addr: SocketAddr,
    },
}

/// Advanced processor set for v2 processors per session
#[derive(Debug)]
pub struct AdvancedProcessorSet {
    /// Advanced voice activity detector (v2)
    pub vad: Option<Arc<RwLock<AdvancedVoiceActivityDetector>>>,
    /// Advanced automatic gain control (v2)
    pub agc: Option<Arc<RwLock<AdvancedAutomaticGainControl>>>,
    /// Advanced acoustic echo canceller (v2)
    pub aec: Option<Arc<RwLock<AdvancedAcousticEchoCanceller>>>,
    /// Session-specific frame pool (shared reference)
    pub frame_pool: Arc<AudioFramePool>,
    /// SIMD processor for this session
    pub simd_processor: SimdProcessor,
    /// Performance metrics for this session
    pub metrics: Arc<RwLock<PerformanceMetrics>>,
    /// Configuration used to create these processors
    pub config: AdvancedProcessorConfig,
}

/// Configuration for advanced processors in a session
#[derive(Debug, Clone)]
pub struct AdvancedProcessorConfig {
    /// Enable advanced VAD
    pub enable_advanced_vad: bool,
    /// Advanced VAD configuration
    pub vad_config: AdvancedVadConfig,
    /// Enable advanced AGC
    pub enable_advanced_agc: bool,
    /// Advanced AGC configuration
    pub agc_config: AdvancedAgcConfig,
    /// Enable advanced AEC
    pub enable_advanced_aec: bool,
    /// Advanced AEC configuration
    pub aec_config: AdvancedAecConfig,
    /// Enable SIMD optimizations
    pub enable_simd: bool,
    /// Frame pool size for this session
    pub frame_pool_size: usize,
    /// Sample rate for processing
    pub sample_rate: u32,
}

impl Default for AdvancedProcessorConfig {
    fn default() -> Self {
        Self {
            enable_advanced_vad: false,
            vad_config: AdvancedVadConfig::default(),
            enable_advanced_agc: false,
            agc_config: AdvancedAgcConfig::default(),
            enable_advanced_aec: false,
            aec_config: AdvancedAecConfig::default(),
            enable_simd: true,
            frame_pool_size: 16,
            sample_rate: 8000,
        }
    }
}

impl AdvancedProcessorSet {
    /// Create a new advanced processor set
    pub async fn new(config: AdvancedProcessorConfig, frame_pool: Arc<AudioFramePool>) -> Result<Self> {
        debug!("Creating AdvancedProcessorSet with config: {:?}", config);
        
        // Create SIMD processor
        let simd_processor = SimdProcessor::new();
        
        // Create performance metrics
        let metrics = Arc::new(RwLock::new(PerformanceMetrics::new()));
        
        // Create advanced processors based on configuration
        let vad = if config.enable_advanced_vad {
            let vad_detector = AdvancedVoiceActivityDetector::new(
                config.vad_config.clone(),
                config.sample_rate as f32,
            )?;
            Some(Arc::new(RwLock::new(vad_detector)))
        } else {
            None
        };
        
        let agc = if config.enable_advanced_agc {
            let agc_processor = AdvancedAutomaticGainControl::new(
                config.agc_config.clone(),
                config.sample_rate as f32,
            )?;
            Some(Arc::new(RwLock::new(agc_processor)))
        } else {
            None
        };
        
        let aec = if config.enable_advanced_aec {
            let aec_processor = AdvancedAcousticEchoCanceller::new(
                config.aec_config.clone(),
            )?;
            Some(Arc::new(RwLock::new(aec_processor)))
        } else {
            None
        };
        
        debug!("AdvancedProcessorSet created: VAD={}, AGC={}, AEC={}, SIMD={}",
               vad.is_some(), agc.is_some(), aec.is_some(), simd_processor.is_simd_available());
        
        Ok(Self {
            vad,
            agc,
            aec,
            frame_pool,
            simd_processor,
            metrics,
            config,
        })
    }
    
    /// Process audio frame with advanced processors
    pub async fn process_audio(&self, input_frame: &AudioFrame) -> Result<AudioFrame> {
        let start_time = std::time::Instant::now();
        
        let mut processed_frame = input_frame.clone();
        
        // Process with advanced AEC first (if enabled and far-end reference available)
        if let Some(aec) = &self.aec {
            // TODO: Add far-end reference when available
            debug!("AEC v2 processing skipped - far-end reference not available");
        }
        
        // Process with advanced AGC
        if let Some(agc) = &self.agc {
            let mut agc_processor = agc.write().await;
            let result = agc_processor.process_frame(&processed_frame)?;
            // TODO: Apply AGC result to frame
            debug!("AGC v2 processed frame with {} band gains", result.band_gains_db.len());
        }
        
        // Process with advanced VAD
        let mut vad_result = None;
        if let Some(vad) = &self.vad {
            let mut vad_detector = vad.write().await;
            vad_result = Some(vad_detector.analyze_frame(&processed_frame)?);
        }
        
        // Apply SIMD optimizations if enabled
        if self.config.enable_simd && self.simd_processor.is_simd_available() {
            // Apply SIMD-optimized operations
            let mut simd_samples = vec![0i16; processed_frame.samples.len()];
            self.simd_processor.apply_gain(&processed_frame.samples, 1.0, &mut simd_samples);
            processed_frame.samples = simd_samples;
        }
        
        // Update performance metrics
        let processing_time = start_time.elapsed();
        {
            let mut metrics = self.metrics.write().await;
            metrics.add_timing(processing_time);
            metrics.add_allocation(processed_frame.samples.len() as u64 * 2); // 2 bytes per i16
        }
        
        if let Some(vad) = vad_result {
            debug!("Advanced VAD result: voice={}, confidence={:.2}", vad.is_voice, vad.confidence);
        }
        
        Ok(processed_frame)
    }
    
    /// Get performance metrics for this processor set
    pub async fn get_metrics(&self) -> PerformanceMetrics {
        self.metrics.read().await.clone()
    }
    
    /// Reset performance metrics
    pub async fn reset_metrics(&self) {
        let mut metrics = self.metrics.write().await;
        *metrics = PerformanceMetrics::new();
    }
    
    /// Check if any advanced processors are enabled
    pub fn has_advanced_processors(&self) -> bool {
        self.vad.is_some() || self.agc.is_some() || self.aec.is_some()
    }
}

/// Media Session Controller for managing media sessions and conference audio mixing
pub struct MediaSessionController {
    /// Underlying media relay (optional)
    relay: Option<Arc<MediaRelay>>,
    /// Active media sessions indexed by dialog ID
    sessions: RwLock<HashMap<DialogId, MediaSessionInfo>>,
    /// Active RTP sessions indexed by dialog ID
    rtp_sessions: RwLock<HashMap<DialogId, RtpSessionWrapper>>,
    /// Event channel for media session events
    event_tx: mpsc::UnboundedSender<MediaSessionEvent>,
    /// Event receiver (taken by the user)
    event_rx: RwLock<Option<mpsc::UnboundedReceiver<MediaSessionEvent>>>,
    /// Audio mixer for conference calls (Phase 5.2 addition)
    audio_mixer: Option<Arc<AudioMixer>>,
    /// Conference mixing configuration
    conference_config: ConferenceMixingConfig,
    /// Conference event sender
    conference_event_tx: mpsc::UnboundedSender<ConferenceMixingEvent>,
    /// Conference event receiver
    conference_event_rx: RwLock<Option<mpsc::UnboundedReceiver<ConferenceMixingEvent>>>,
    /// Quality monitor for conference sessions
    quality_monitor: Option<Arc<QualityMonitor>>,
    
    // NEW: Performance library integration fields
    /// Global performance metrics for all sessions
    performance_metrics: Arc<RwLock<PerformanceMetrics>>,
    /// Global frame pool for efficient allocation (shared across sessions)
    frame_pool: Arc<AudioFramePool>,
    /// RTP output buffer pool for zero-copy encoding
    rtp_buffer_pool: Arc<RtpBufferPool>,
    /// Advanced processors per dialog
    advanced_processors: RwLock<HashMap<DialogId, AdvancedProcessorSet>>,
    /// Default configuration for advanced processors
    default_processor_config: AdvancedProcessorConfig,
    /// G.711 codec for zero-copy audio processing
    g711_codec: Arc<tokio::sync::Mutex<crate::codec::audio::G711Codec>>,
    /// SIMD processor for audio operations
    simd_processor: SimdProcessor,
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
            .allocate_port_pair(&dialog_session_id, Some("127.0.0.1".parse().unwrap()))
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
            created_at: std::time::Instant::now(),
        };

        // Port assignment is handled by GlobalPortAllocator

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
                rtp_session.close().await;
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
        
        // If remote address was set/changed, emit event
        if config.remote_addr != old_remote {
            if let Some(remote_addr) = config.remote_addr {
                let _ = self.event_tx.send(MediaSessionEvent::RemoteAddressUpdated {
                    dialog_id: dialog_id.clone(),
                    remote_addr,
                });
            }
        }
        
        debug!("Media session updated for dialog: {}", dialog_id);
        Ok(())
    }
    
    /// Create relay between two dialogs
    pub async fn create_relay(&self, dialog_a: String, dialog_b: String) -> Result<()> {
        info!("Creating relay between dialogs: {} <-> {}", dialog_a, dialog_b);

        // Verify both sessions exist and get their configs
        let (session_a_config, session_b_config) = {
            let sessions = self.sessions.read().await;
            let dialog_a_id = DialogId::new(dialog_a.clone());
            let dialog_b_id = DialogId::new(dialog_b.clone());
            let session_a = sessions.get(&dialog_a_id)
                .ok_or_else(|| Error::session_not_found(dialog_a.clone()))?;
            let session_b = sessions.get(&dialog_b_id)
                .ok_or_else(|| Error::session_not_found(dialog_b.clone()))?;
            (session_a.config.clone(), session_b.config.clone())
        };
        
        // Generate relay session IDs
        let relay_session_a = generate_session_id();
        let relay_session_b = generate_session_id();
        
        // Create relay configuration
        let relay_config = create_relay_config(
            relay_session_a.clone(),
            relay_session_b.clone(),
            session_a_config.local_addr,
            session_b_config.local_addr,
        );
        
        // Create the relay session pair if relay is available
        if let Some(relay) = &self.relay {
            relay.create_session_pair(relay_config).await?;
        }
        
        // Update session infos with relay session IDs
        {
            let mut sessions = self.sessions.write().await;
            let dialog_a_id = DialogId::new(dialog_a.clone());
            let dialog_b_id = DialogId::new(dialog_b.clone());
            if let Some(session_a_info) = sessions.get_mut(&dialog_a_id) {
                session_a_info.relay_session_ids = Some((relay_session_a.clone(), relay_session_b.clone()));
                session_a_info.status = MediaSessionStatus::Active;
            }
            if let Some(session_b_info) = sessions.get_mut(&dialog_b_id) {
                session_b_info.relay_session_ids = Some((relay_session_b, relay_session_a));
                session_b_info.status = MediaSessionStatus::Active;
            }
        }
        
        info!("Media relay created between dialogs: {} <-> {}", dialog_a, dialog_b);
        Ok(())
    }
    
    /// Get session information for a dialog
    pub async fn get_session_info(&self, dialog_id: &DialogId) -> Option<MediaSessionInfo> {
        let sessions = self.sessions.read().await;
        sessions.get(dialog_id).cloned()
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
    
    /// Get media relay reference (for advanced usage)
    pub fn relay(&self) -> Option<&Arc<MediaRelay>> {
        self.relay.as_ref()
    }
    
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
        session.send_packet(timestamp, bytes::Bytes::from(payload), false).await
            .map_err(|e| Error::config(format!("Failed to send RTP packet: {}", e)))?;
        
        debug!("✅ Sent RTP packet for dialog: {} (timestamp: {})", dialog_id, timestamp);
        Ok(())
    }
    
    /// Update remote address for RTP session
    pub async fn update_rtp_remote_addr(&self, dialog_id: &DialogId, remote_addr: SocketAddr) -> Result<()> {
        let rtp_session = self.get_rtp_session(dialog_id).await
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;
        
        let mut session = rtp_session.lock().await;
        session.set_remote_addr(remote_addr);
        
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
    
    /// Get RTP session statistics
    pub async fn get_rtp_stats(&self, dialog_id: &DialogId) -> Option<String> {
        let rtp_session = self.get_rtp_session(dialog_id).await?;
        let session = rtp_session.lock().await;
        
        // Get basic session info
        let local_addr = session.local_addr().ok()?;
        let ssrc = session.get_ssrc();
        
        Some(format!("RTP Session - Local: {}, SSRC: 0x{:08x}", local_addr, ssrc))
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
    
    // ================================
    // ADVANCED PROCESSING METHODS (Phase 1.3)
    // ================================
    
    /// Start advanced media session with custom processor configuration
    pub async fn start_advanced_media(&self, dialog_id: DialogId, config: MediaConfig, processor_config: Option<AdvancedProcessorConfig>) -> Result<()> {
        info!("Starting advanced media session for dialog: {}", dialog_id);
        
        // Start regular media session first
        self.start_media(dialog_id.clone(), config).await?;
        
        // Create advanced processors if configuration provided
        if let Some(proc_config) = processor_config {
            // Create session-specific frame pool for advanced processors or use global pool
            let session_frame_pool: Arc<AudioFramePool> = if proc_config.frame_pool_size > 0 {
                // Create dedicated pool for this session
                let session_pool_config = PoolConfig {
                    initial_size: proc_config.frame_pool_size,
                    max_size: proc_config.frame_pool_size * 2,
                    sample_rate: proc_config.sample_rate,
                    channels: 1,
                    samples_per_frame: 160, // 20ms at 8kHz
                };
                AudioFramePool::new(session_pool_config)
            } else {
                // Use global shared pool
                self.frame_pool.clone()
            };
            
            let processor_set = AdvancedProcessorSet::new(proc_config, session_frame_pool).await?;
            
            {
                let mut processors = self.advanced_processors.write().await;
                processors.insert(dialog_id.clone(), processor_set);
            }
            
            info!("✅ Created advanced processors for dialog: {}", dialog_id);
        } else {
            info!("⚠️ No processor configuration provided - using basic media session");
        }
        
        Ok(())
    }
    
    /// Process audio frame with advanced processors (if enabled for this dialog)
    pub async fn process_advanced_audio(&self, dialog_id: &DialogId, audio_frame: AudioFrame) -> Result<AudioFrame> {
        let start_time = std::time::Instant::now();
        
        // Check if dialog has advanced processors
        let processed_frame = {
            let processors = self.advanced_processors.read().await;
            if let Some(processor_set) = processors.get(dialog_id) {
                // Process with session-specific advanced processors
                let processed = processor_set.process_audio(&audio_frame).await?;
                debug!("Processed audio frame for {} with advanced processors", dialog_id);
                processed
            } else {
                // Use global frame pool for zero-copy optimization even without advanced processors
                debug!("Processed audio frame for {} with global pool only", dialog_id);
                audio_frame // Return as-is if no advanced processors
            }
        };
        
        // Update global performance metrics
        let processing_time = start_time.elapsed();
        {
            let mut metrics = self.performance_metrics.write().await;
            metrics.add_timing(processing_time);
            metrics.add_allocation(processed_frame.samples.len() as u64 * 2); // 2 bytes per i16
        }
        
        Ok(processed_frame)
    }
    
    /// Get performance metrics for a specific dialog
    pub async fn get_dialog_performance_metrics(&self, dialog_id: &DialogId) -> Option<PerformanceMetrics> {
        let processors = self.advanced_processors.read().await;
        if let Some(processor_set) = processors.get(dialog_id) {
            Some(processor_set.get_metrics().await)
        } else {
            None
        }
    }
    
    /// Get global performance metrics for all sessions
    pub async fn get_global_performance_metrics(&self) -> PerformanceMetrics {
        self.performance_metrics.read().await.clone()
    }
    
    /// Reset performance metrics for a specific dialog
    pub async fn reset_dialog_metrics(&self, dialog_id: &DialogId) -> Result<()> {
        let processors = self.advanced_processors.read().await;
        if let Some(processor_set) = processors.get(dialog_id) {
            processor_set.reset_metrics().await;
            Ok(())
        } else {
            Err(Error::session_not_found(&format!("No advanced processors for dialog: {}", dialog_id)))
        }
    }
    
    /// Reset global performance metrics
    pub async fn reset_global_metrics(&self) {
        let mut metrics = self.performance_metrics.write().await;
        *metrics = PerformanceMetrics::new();
    }
    
    /// Check if dialog has advanced processors enabled
    pub async fn has_advanced_processors(&self, dialog_id: &DialogId) -> bool {
        let processors = self.advanced_processors.read().await;
        processors.get(dialog_id)
            .map(|p| p.has_advanced_processors())
            .unwrap_or(false)
    }
    
    /// Get frame pool statistics
    pub fn get_frame_pool_stats(&self) -> crate::performance::pool::PoolStats {
        self.frame_pool.get_stats()
    }
    
    /// Update default processor configuration for new sessions
    pub async fn set_default_processor_config(&mut self, config: AdvancedProcessorConfig) {
        self.default_processor_config = config;
        info!("Updated default processor configuration");
    }
    
    /// Get current default processor configuration
    pub fn get_default_processor_config(&self) -> &AdvancedProcessorConfig {
        &self.default_processor_config
    }
    
    // ===== CONFERENCE AUDIO MIXING METHODS (Phase 5.2) =====
    
    /// Enable conference audio mixing with the given configuration
    pub async fn enable_conference_mixing(&mut self, config: ConferenceMixingConfig) -> Result<()> {
        info!("🎙️ Enabling conference audio mixing");
        
        if self.audio_mixer.is_some() {
            return Err(Error::config("Conference mixing already enabled"));
        }
        
        // Create audio mixer
        let audio_mixer = Arc::new(AudioMixer::new(config.clone()).await
            .map_err(|e| Error::config(format!("Failed to create audio mixer: {}", e)))?);
        
        // Set up event forwarding
        audio_mixer.set_event_sender(self.conference_event_tx.clone()).await;
        
        self.audio_mixer = Some(audio_mixer);
        self.conference_config = config;
        
        info!("✅ Conference audio mixing enabled");
        Ok(())
    }
    
    /// Disable conference audio mixing
    pub async fn disable_conference_mixing(&mut self) -> Result<()> {
        info!("🔇 Disabling conference audio mixing");
        
        if let Some(mixer) = &self.audio_mixer {
            // Clean up all participants
            let participants = mixer.get_active_participants().await
                .map_err(|e| Error::config(format!("Failed to get participants: {}", e)))?;
            
            for participant_id in participants {
                let _ = mixer.remove_audio_stream(&participant_id).await;
            }
        }
        
        self.audio_mixer = None;
        
        info!("✅ Conference audio mixing disabled");
        Ok(())
    }
    
    /// Add a dialog to the conference (participant joins)
    pub async fn add_to_conference(&self, dialog_id: &str) -> Result<()> {
        info!("🎤 Adding dialog {} to conference", dialog_id);
        
        let mixer = self.audio_mixer.as_ref()
            .ok_or_else(|| Error::config("Conference mixing not enabled"))?;
        
        // Convert to DialogId for session lookup
        let dialog_id_typed = DialogId::new(dialog_id);
        
        // Check if session exists
        let session_info = self.get_session_info(&dialog_id_typed).await
            .ok_or_else(|| Error::session_not_found(dialog_id.to_string()))?;
        
        if session_info.status != MediaSessionStatus::Active {
            return Err(Error::config(format!(
                "Cannot add inactive session to conference: {}", dialog_id
            )));
        }
        
        // Create audio stream for this participant
        let participant_id = ParticipantId::new(dialog_id);
        let audio_stream = AudioStream::new(
            participant_id.clone(),
            self.conference_config.output_sample_rate,
            self.conference_config.output_channels,
        );
        
        // Add to mixer
        mixer.add_audio_stream(participant_id, audio_stream).await
            .map_err(|e| Error::config(format!("Failed to add to conference: {}", e)))?;
        
        // Flush events to ensure synchronous delivery for testing
        mixer.flush_events().await;
        
        info!("✅ Added dialog {} to conference", dialog_id);
        Ok(())
    }
    
    /// Remove a dialog from the conference (participant leaves)
    pub async fn remove_from_conference(&self, dialog_id: &str) -> Result<()> {
        info!("👋 Removing dialog {} from conference", dialog_id);
        
        let mixer = self.audio_mixer.as_ref()
            .ok_or_else(|| Error::config("Conference mixing not enabled"))?;
        
        let participant_id = ParticipantId::new(dialog_id);
        
        // Validate that participant exists before attempting removal
        let active_participants = mixer.get_active_participants().await
            .map_err(|e| Error::config(format!("Failed to get participants: {}", e)))?;
        
        if !active_participants.contains(&participant_id) {
            return Err(Error::config(format!(
                "Participant {} not found in conference", dialog_id
            )));
        }
        
        // Remove from mixer
        mixer.remove_audio_stream(&participant_id).await
            .map_err(|e| Error::config(format!("Failed to remove from conference: {}", e)))?;
        
        // Flush events to ensure synchronous delivery for testing
        mixer.flush_events().await;
        
        info!("✅ Removed dialog {} from conference", dialog_id);
        Ok(())
    }
    
    /// Process incoming audio for conference mixing
    pub async fn process_conference_audio(&self, dialog_id: &str, audio_frame: AudioFrame) -> Result<()> {
        let mixer = self.audio_mixer.as_ref()
            .ok_or_else(|| Error::config("Conference mixing not enabled"))?;
        
        let participant_id = ParticipantId::new(dialog_id);
        
        // Validate that participant exists in conference
        let active_participants = mixer.get_active_participants().await
            .map_err(|e| Error::config(format!("Failed to get participants: {}", e)))?;
        
        if !active_participants.contains(&participant_id) {
            return Err(Error::config(format!(
                "Participant {} not found in conference", dialog_id
            )));
        }
        
        // Process audio through mixer
        mixer.process_audio_frame(&participant_id, audio_frame).await
            .map_err(|e| Error::config(format!("Failed to process conference audio: {}", e)))?;
        
        // Trigger mixing if we have enough participants
        if active_participants.len() >= 2 {
            let empty_inputs = Vec::new(); // AudioMixer gets its inputs from stream manager
            let _mixed_outputs = mixer.mix_participants(&empty_inputs).await
                .map_err(|e| Error::config(format!("Failed to perform mixing: {}", e)))?;
        }
        
        Ok(())
    }
    
    /// Get mixed audio for a specific participant (everyone except themselves)
    pub async fn get_conference_mixed_audio(&self, dialog_id: &str) -> Result<Option<AudioFrame>> {
        let mixer = self.audio_mixer.as_ref()
            .ok_or_else(|| Error::config("Conference mixing not enabled"))?;
        
        let participant_id = ParticipantId::new(dialog_id);
        
        // Validate that participant exists in conference
        let active_participants = mixer.get_active_participants().await
            .map_err(|e| Error::config(format!("Failed to get participants: {}", e)))?;
        
        if !active_participants.contains(&participant_id) {
            return Err(Error::config(format!(
                "Participant {} not found in conference", dialog_id
            )));
        }
        
        // Get mixed audio from mixer
        mixer.get_mixed_audio(&participant_id).await
            .map_err(|e| Error::config(format!("Failed to get mixed audio: {}", e)))
    }
    
    /// Get list of conference participants
    pub async fn get_conference_participants(&self) -> Result<Vec<String>> {
        let mixer = self.audio_mixer.as_ref()
            .ok_or_else(|| Error::config("Conference mixing not enabled"))?;
        
        let participants = mixer.get_active_participants().await
            .map_err(|e| Error::config(format!("Failed to get participants: {}", e)))?;
        
        Ok(participants.into_iter().map(|p| p.0).collect())
    }
    
    /// Get conference mixing statistics
    pub async fn get_conference_stats(&self) -> Result<ConferenceMixingStats> {
        let mixer = self.audio_mixer.as_ref()
            .ok_or_else(|| Error::config("Conference mixing not enabled"))?;
        
        mixer.get_mixing_stats().await
            .map_err(|e| Error::config(format!("Failed to get conference stats: {}", e)))
    }
    
    /// Get conference event receiver (can only be called once)
    pub async fn take_conference_event_receiver(&self) -> Option<mpsc::UnboundedReceiver<ConferenceMixingEvent>> {
        let mut event_rx = self.conference_event_rx.write().await;
        event_rx.take()
    }
    
    /// Check if conference mixing is enabled
    pub fn is_conference_mixing_enabled(&self) -> bool {
        self.audio_mixer.is_some()
    }
    
    /// Clean up inactive conference participants
    pub async fn cleanup_conference_participants(&self) -> Result<Vec<String>> {
        let mixer = self.audio_mixer.as_ref()
            .ok_or_else(|| Error::config("Conference mixing not enabled"))?;
        
        let removed = mixer.cleanup_inactive_participants().await
            .map_err(|e| Error::config(format!("Failed to cleanup participants: {}", e)))?;
        
        Ok(removed.into_iter().map(|p| p.0).collect())
    }
    
    // ================================
    // ZERO-COPY RTP PROCESSING METHODS (Phase 3.2)
    // ================================
    
    /// Process RTP packet with zero-copy optimization
    /// 
    /// This method implements true zero-copy processing by:
    /// 1. Using pooled frames for audio processing (reuse)
    /// 2. Decoding directly into pooled buffer (zero-copy decode)
    /// 3. Processing in-place with SIMD (zero-copy processing)
    /// 4. Encoding to pre-allocated output buffer (zero-copy encode)
    /// 5. Creating RTP packet with buffer reference (zero-copy)
    pub async fn process_rtp_packet_zero_copy(&self, packet: &rtp_core::RtpPacket) -> Result<rtp_core::RtpPacket> {
        let start_time = std::time::Instant::now();
        
        // Step 1: Get pooled frame (reuses pre-allocated memory)
        let mut pooled_frame = self.frame_pool.get_frame_with_params(
            8000, // Sample rate
            1,    // Channels
            160,  // Frame size (20ms at 8kHz)
        );
        
        // Step 2: Decode RTP payload directly into pooled frame buffer (zero-copy)
        let payload_bytes: &[u8] = &packet.payload;
        {
            let mut codec = self.g711_codec.lock().await;
            codec.decode_to_buffer(payload_bytes, pooled_frame.samples_mut())?;
        }
        
        // Step 3: Apply SIMD processing in-place (zero-copy)
        self.simd_processor.apply_gain_in_place(pooled_frame.samples_mut(), 1.2);
        
        // Step 4: Encode from pooled buffer to pre-allocated output (zero-copy)
        let mut output_buffer = self.rtp_buffer_pool.get_buffer();
        let encoded_size = {
            let mut codec = self.g711_codec.lock().await;
            codec.encode_to_buffer(pooled_frame.samples(), output_buffer.as_mut())?
        };
        
        // Step 5: Create RTP packet with buffer reference (zero-copy)
        let new_payload = output_buffer.slice(encoded_size);
        let output_header = rtp_core::RtpHeader::new(
            packet.header.payload_type,
            packet.header.sequence_number + 1,
            packet.header.timestamp,
            packet.header.ssrc,
        );
        
        // Update performance metrics
        let processing_time = start_time.elapsed();
        {
            let mut metrics = self.performance_metrics.write().await;
            metrics.add_timing(processing_time);
            // Note: Zero allocations! We only track buffer reuse
            metrics.operation_count += 1;
        }
        
        debug!("Zero-copy RTP processing completed in {:?}", processing_time);
        Ok(rtp_core::RtpPacket::new(output_header, new_payload))
        // pooled_frame automatically returns to pool here
    }
    
    /// Process RTP packet with traditional approach (for comparison)
    /// 
    /// This method uses the traditional approach with allocations for comparison:
    /// 1. Extract payload to Vec<u8> (COPY)
    /// 2. Decode to Vec<i16> (COPY + ALLOCATION)
    /// 3. Create AudioFrame with Vec<i16> (COPY)
    /// 4. Process to Vec<i16> (COPY)
    /// 5. Encode to Vec<u8> (COPY + ALLOCATION)
    /// 6. Create RTP packet with Bytes (COPY)
    pub async fn process_rtp_packet_traditional(&self, packet: &rtp_core::RtpPacket) -> Result<rtp_core::RtpPacket> {
        let start_time = std::time::Instant::now();
        
        // Step 1: Extract payload → Vec<u8> (COPY)
        let payload_bytes = packet.payload.to_vec();
        
        // Step 2: Decode → Vec<i16> (COPY + ALLOCATION)
        let decoded_samples = {
            let mut codec = self.g711_codec.lock().await;
            let mut samples = vec![0i16; payload_bytes.len()];
            codec.decode_to_buffer(&payload_bytes, &mut samples)?;
            samples
        };
        let decoded_len = decoded_samples.len(); // Store length before move
        
        // Step 3: Create AudioFrame → Vec<i16> (COPY)
        let audio_frame = crate::types::AudioFrame::new(
            decoded_samples,
            8000,
            1,
            packet.header.timestamp,
        );
        
        // Step 4: Process → Vec<i16> (COPY)
        let mut processed_samples = audio_frame.samples.clone();
        self.simd_processor.apply_gain(&audio_frame.samples, 1.2, &mut processed_samples);
        
        // Step 5: Encode → Vec<u8> (COPY + ALLOCATION)
        let encoded_payload = {
            let mut codec = self.g711_codec.lock().await;
            let mut output = vec![0u8; processed_samples.len()];
            let encoded_size = codec.encode_to_buffer(&processed_samples, &mut output)?;
            output.truncate(encoded_size);
            output
        };
        
        // Step 6: Create RTP packet → Bytes (COPY)
        let new_payload = bytes::Bytes::from(encoded_payload);
        let output_header = rtp_core::RtpHeader::new(
            packet.header.payload_type,
            packet.header.sequence_number + 1,
            packet.header.timestamp,
            packet.header.ssrc,
        );
        
        // Update performance metrics
        let processing_time = start_time.elapsed();
        {
            let mut metrics = self.performance_metrics.write().await;
            metrics.add_timing(processing_time);
            metrics.add_allocation(payload_bytes.len() as u64);      // Allocation 1
            metrics.add_allocation(decoded_len as u64 * 2);          // Allocation 2 (i16) - use stored length
            metrics.add_allocation(processed_samples.len() as u64 * 2); // Allocation 3 (i16)
            metrics.add_allocation(new_payload.len() as u64);        // Allocation 4
            metrics.operation_count += 1;
        }
        
        debug!("Traditional RTP processing completed in {:?}", processing_time);
        Ok(rtp_core::RtpPacket::new(output_header, new_payload))
    }
    
    /// Get RTP buffer pool statistics
    pub fn get_rtp_buffer_pool_stats(&self) -> PoolStats {
        self.rtp_buffer_pool.get_stats()
    }
    
    /// Reset RTP buffer pool statistics
    pub fn reset_rtp_buffer_pool_stats(&self) {
        self.rtp_buffer_pool.reset_stats();
    }
}

impl Default for MediaSessionController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};
    
    #[tokio::test]
    async fn test_start_stop_session() {
        let controller = MediaSessionController::new();
        
        let config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: None,
            parameters: HashMap::new(),
        };
        
        // Start session
        let result = controller.start_media(DialogId::new("dialog1"), config).await;
        assert!(result.is_ok());
        
        // Check session exists
        let session_info = controller.get_session_info(&DialogId::new("dialog1")).await;
        assert!(session_info.is_some());
        
        // Stop session
        let result = controller.stop_media(&DialogId::new("dialog1")).await;
        assert!(result.is_ok());
        
        // Check session is removed
        let session_info = controller.get_session_info(&DialogId::new("dialog1")).await;
        assert!(session_info.is_none());
    }
    
    #[tokio::test]
    async fn test_create_relay() {
        let controller = MediaSessionController::new();
        
        let config_a = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: None,
            parameters: HashMap::new(),
        };
        
        let config_b = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: None,
            parameters: HashMap::new(),
        };
        
        // Start both sessions
        controller.start_media(DialogId::new("dialog1"), config_a).await.unwrap();
        controller.start_media(DialogId::new("dialog2"), config_b).await.unwrap();
        
        // Create relay should succeed but not actually create relay since no MediaRelay is configured
        let result = controller.create_relay("dialog1".to_string(), "dialog2".to_string()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_dynamic_port_allocation() {
        println!("🧪 Testing dynamic port allocation integration");
        
        let controller = MediaSessionController::new();
        
        // Create multiple sessions to verify different ports are allocated
        let mut session_infos = Vec::new();
        
        for i in 0..3 {
            let dialog_id = format!("test_dialog_{}", i);
            let config = MediaConfig {
                local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
                remote_addr: None,
                preferred_codec: None,
                parameters: HashMap::new(),
            };
            
            println!("📞 Creating session: {}", dialog_id);
            controller.start_media(DialogId::new(dialog_id.clone()), config).await
                .expect("Failed to start media session");
            
            let session_info = controller.get_session_info(&DialogId::new(dialog_id)).await
                .expect("Session should exist");
            
            println!("✅ Session created with port: {:?}", session_info.rtp_port);
            assert!(session_info.rtp_port.is_some(), "Port should be allocated");
            
            session_infos.push(session_info);
        }
        
        // Verify different ports were allocated
        let mut ports = Vec::new();
        for session_info in &session_infos {
            if let Some(port) = session_info.rtp_port {
                ports.push(port);
            }
        }
        
        // Remove duplicates and check that we have unique ports
        ports.sort();
        ports.dedup();
        assert_eq!(ports.len(), 3, "All sessions should have unique ports");
        
        println!("🎯 Allocated ports: {:?}", ports);
        
        // Verify all ports are in valid range (no privileged ports)
        for &port in &ports {
            assert!(port >= 1024, "Port should be >= 1024 (non-privileged)");
            assert!(port <= 65535, "Port should be <= 65535 (valid range)");
        }
        
        println!("✅ All ports are in valid range and unique");
        
        // Clean up sessions
        for i in 0..3 {
            let dialog_id = format!("test_dialog_{}", i);
            controller.stop_media(&DialogId::new(dialog_id)).await
                .expect("Failed to stop media session");
        }
        
        println!("✨ Dynamic port allocation test completed successfully!");
        println!("🔧 rtp-core's PortAllocator is providing conflict-free dynamic allocation");
    }
} 