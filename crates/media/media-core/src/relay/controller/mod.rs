//! Media Session Controller for Session-Core Integration
//!
//! This module provides the high-level interface for session-core to control
//! media sessions. It manages the lifecycle of media sessions tied to SIP dialogs.
//!
//! ## Audio Muting
//!
//! The controller implements production-ready audio muting using silence-based
//! approach. When `set_audio_muted()` is called, the RTP stream continues but
//! audio samples are replaced with silence before encoding. This maintains:
//!
//! - Continuous RTP sequence numbers and timestamps
//! - NAT traversal and firewall state
//! - Compatibility with all SIP endpoints
//! - Instant mute/unmute without renegotiation
//!
//! Use `set_audio_muted()` and `is_audio_muted()` for muting functionality.

use dashmap::DashMap;
use std::sync::{Arc, OnceLock};
use std::time::Instant;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, trace, warn};

#[cfg(feature = "g729")]
use crate::codec::audio::common::AudioCodec;
use crate::codec::audio::G711Codec;
#[cfg(feature = "g729")]
use crate::codec::audio::G729Codec;
use crate::codec::mapping::CodecMapper;
use crate::diagnostics;
use crate::error::{Error, Result};
use crate::integration::{RtpBridge, RtpBridgeConfig, RtpEventCallback};
use crate::performance::{
    metrics::ConcurrentPerformanceMetrics,
    pool::{AudioFramePool, PoolConfig, RtpBufferPool},
    simd::SimdProcessor,
};
use crate::processing::audio::AudioMixer;
use crate::quality::QualityMonitor;
use crate::relay::controller::codec_detection::CodecDetector;
use crate::relay::controller::codec_fallback::CodecFallbackManager;
use crate::types::conference::{ConferenceMixingConfig, ConferenceMixingEvent};
use crate::types::{AudioFrame, DialogId, MediaDirection, MediaSessionId};

use rvoip_rtp_core as rtp_core;
use rvoip_rtp_core::transport::{
    AllocationStrategy, GlobalPortAllocator, PortAllocator, PortAllocatorConfig, SymmetricRtpPolicy,
};
use rvoip_rtp_core::{
    RtpSession, RtpSessionBufferConfig, RtpSessionConfig, RtpTransportBufferConfig,
};

const RTP_SESSION_BIND_RETRIES: usize = 8;

/// Releases a media-controller port reservation if `start_media` is cancelled
/// before ownership is committed to the controller maps.
struct MediaPortReservationGuard {
    allocator: Arc<PortAllocator>,
    session_id: Option<String>,
}

impl MediaPortReservationGuard {
    fn new(allocator: Arc<PortAllocator>, session_id: String) -> Self {
        Self {
            allocator,
            session_id: Some(session_id),
        }
    }

    fn disarm(&mut self) {
        self.session_id = None;
    }
}

impl Drop for MediaPortReservationGuard {
    fn drop(&mut self) {
        let Some(session_id) = self.session_id.take() else {
            return;
        };
        let allocator = self.allocator.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = allocator.release_session(&session_id).await;
            });
        }
    }
}

#[cfg(feature = "g729")]
fn decode_g729_payload_to_buffer(
    decoder: &mut G729Codec,
    payload: &[u8],
    output: &mut Vec<i16>,
) -> Result<usize> {
    const G729_SAMPLES_PER_FRAME: usize = 80;
    const G729_SPEECH_FRAME_BYTES: usize = 10;
    const G729_SID_FRAME_BYTES: usize = 2;

    let frame_count = if payload.is_empty() || payload.len() == G729_SID_FRAME_BYTES {
        1
    } else if payload.len() % G729_SPEECH_FRAME_BYTES == 0 {
        payload.len() / G729_SPEECH_FRAME_BYTES
    } else {
        1
    };
    let needed = frame_count * G729_SAMPLES_PER_FRAME;
    if output.len() < needed {
        output.resize(needed, 0);
    }

    if payload.is_empty()
        || payload.len() == G729_SID_FRAME_BYTES
        || payload.len() % G729_SPEECH_FRAME_BYTES != 0
    {
        let frame = decoder.decode(payload)?;
        output[..frame.samples.len()].copy_from_slice(&frame.samples);
        return Ok(frame.samples.len());
    }

    let mut written = 0;
    for chunk in payload.chunks_exact(G729_SPEECH_FRAME_BYTES) {
        let frame = decoder.decode(chunk)?;
        let end = written + frame.samples.len();
        output[written..end].copy_from_slice(&frame.samples);
        written = end;
    }
    Ok(written)
}

/// Controller-level capacity and pool tuning.
#[derive(Debug, Clone)]
pub struct MediaSessionControllerConfig {
    /// Initial capacity for hot session indexes.
    pub capacity_hint: usize,
    /// Shared audio frame pool configuration.
    pub audio_frame_pool: PoolConfig,
    /// RTP output buffer size in bytes.
    pub rtp_buffer_size: usize,
    /// Initial RTP output buffer count.
    pub rtp_buffer_initial_count: usize,
    /// Maximum RTP output buffer count.
    pub rtp_buffer_max_count: usize,
    /// RTP session queue sizing.
    pub rtp_session_buffer_config: RtpSessionBufferConfig,
    /// RTP transport event and receive buffer sizing.
    pub rtp_transport_buffer_config: RtpTransportBufferConfig,
}

impl Default for MediaSessionControllerConfig {
    fn default() -> Self {
        Self {
            capacity_hint: 0,
            audio_frame_pool: PoolConfig {
                initial_size: 32,
                max_size: 128,
                sample_rate: 8000,
                channels: 1,
                samples_per_frame: 160,
            },
            rtp_buffer_size: 480,
            rtp_buffer_initial_count: 32,
            rtp_buffer_max_count: 128,
            rtp_session_buffer_config: RtpSessionBufferConfig::default(),
            rtp_transport_buffer_config: RtpTransportBufferConfig::default(),
        }
    }
}

fn configured_port_range_len(base_port: u16, max_port: u16) -> usize {
    if max_port < base_port {
        0
    } else {
        usize::from(max_port - base_port) + 1
    }
}

fn is_retryable_rtp_bind_error(error: &rtp_core::Error) -> bool {
    let text = error.to_string();
    text.contains("bind") || text.contains("Address already in use")
}

// Sub-modules
pub mod advanced_processing;
pub mod audio_generation;
pub mod bridge;
pub mod cn_gate;
pub mod cn_transmitter;
pub mod codec_detection;
pub mod codec_fallback;
pub mod conference;
pub mod dtmf_transmitter;
pub mod rtp_management;
pub mod statistics;
pub mod types;
pub mod zero_copy;

#[cfg(test)]
mod tests;

// Re-export important types
pub use audio_generation::{AudioSource, AudioTransmitterConfig};
pub use bridge::{BridgeError, BridgeHandle};
pub use types::{
    AdvancedProcessorConfig, AdvancedProcessorSet, MediaConfig, MediaSessionEvent,
    MediaSessionInfo, MediaSessionStatus,
};

use types::RtpSessionWrapper;

#[cfg(feature = "memory-diagnostics")]
fn spawn_memory_tracked<F>(kind: &'static str, future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    rvoip_infra_common::memory_diagnostics::spawn_tracked(kind, future)
}

#[cfg(not(feature = "memory-diagnostics"))]
fn spawn_memory_tracked<F>(_: &'static str, future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(future)
}

#[cfg(feature = "memory-diagnostics")]
fn record_transient_allocation(kind: &'static str, bytes: usize) {
    rvoip_infra_common::memory_diagnostics::record_transient_allocation(kind, bytes as u64);
}

#[cfg(not(feature = "memory-diagnostics"))]
fn record_transient_allocation(_: &'static str, _: usize) {}

fn perf_skip_audio_frame_delivery() -> bool {
    static SKIP: OnceLock<bool> = OnceLock::new();
    *SKIP.get_or_init(|| {
        if !cfg!(feature = "memory-diagnostics") {
            return false;
        }
        matches!(
            std::env::var("RVOIP_PERF_SKIP_AUDIO_FRAME_DELIVERY").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES") | Ok("on") | Ok("ON")
        )
    })
}

/// RFC 4733 DTMF event delivered from media-core to session-core.
#[derive(Debug, Clone, Copy)]
pub struct DtmfNotification {
    /// Decoded digit character ('0'-'9', '*', '#', 'A'-'D'). Unknown
    /// event codes (reserved for future use / fax / modem tones) are
    /// mapped to `'?'` so the callback consumer can choose to ignore.
    pub digit: char,
    /// Duration of the event at the time we observed `E=1`, converted
    /// from RTP timestamp units to milliseconds assuming the standard
    /// 8 kHz telephone-event clock.
    pub duration_ms: u32,
    /// Sending SSRC — for b2bua disambiguation when multiple streams
    /// share a socket.
    pub ssrc: u32,
}

/// Media Session Controller for managing media sessions and conference audio mixing
pub struct MediaSessionController {
    /// Active media sessions indexed by dialog ID. `DashMap` so
    /// per-dialog inserts/lookups don't serialise through one async
    /// RwLock on every bridge forward, RTCP report, stats query, or
    /// audio-frame callback dispatch.
    pub(super) sessions: Arc<DashMap<DialogId, MediaSessionInfo>>,
    /// Active RTP sessions indexed by dialog ID. Same `DashMap`
    /// rationale as [`Self::sessions`].
    pub(super) rtp_sessions: Arc<DashMap<DialogId, RtpSessionWrapper>>,
    /// Event channel for media session events
    pub(super) event_tx: mpsc::UnboundedSender<MediaSessionEvent>,
    /// Event receiver (taken by the user). Held here until
    /// `take_event_receiver` drains it on the consumer side.
    #[allow(dead_code)]
    event_rx: RwLock<Option<mpsc::UnboundedReceiver<MediaSessionEvent>>>,
    /// Event hub for global event coordination
    event_hub: Arc<RwLock<Option<Arc<crate::events::MediaEventHub>>>>,
    /// Session to media mapping
    session_to_media: Arc<DashMap<String, MediaSessionId>>,
    /// Media to session mapping
    media_to_session: Arc<DashMap<MediaSessionId, String>>,
    /// Audio mixer for conference calls
    pub(super) audio_mixer: Option<Arc<AudioMixer>>,
    /// Conference mixing configuration
    pub(super) conference_config: ConferenceMixingConfig,
    /// Conference event sender
    pub(super) conference_event_tx: mpsc::UnboundedSender<ConferenceMixingEvent>,
    /// Conference event receiver
    conference_event_rx: RwLock<Option<mpsc::UnboundedReceiver<ConferenceMixingEvent>>>,
    /// Quality monitor for conference sessions. Wired in but the
    /// per-controller reads land via the conference task; retained
    /// here so the controller can install/replace it at runtime.
    #[allow(dead_code)]
    pub(super) quality_monitor: Option<Arc<QualityMonitor>>,
    /// Port allocator for RTP ports (if custom range specified)
    port_allocator: Option<Arc<PortAllocator>>,

    // Performance library integration fields
    /// Global performance metrics for all sessions. Lock-free
    /// padded atomics — every per-frame `add_timing` /
    /// `add_allocation` is a handful of relaxed `fetch_add` /
    /// `fetch_min` / `fetch_max` calls on independent cache lines.
    /// Snapshot via `.snapshot()` materialises a `PerformanceMetrics`
    /// for external consumers.
    pub(super) performance_metrics: Arc<ConcurrentPerformanceMetrics>,
    /// Global frame pool for efficient allocation (shared across sessions)
    pub(super) frame_pool: Arc<AudioFramePool>,
    /// RTP output buffer pool for zero-copy encoding
    pub(super) rtp_buffer_pool: Arc<RtpBufferPool>,
    /// Advanced processors per dialog. `DashMap` so the per-frame
    /// AEC/AGC/VAD lookup doesn't take an outer async RwLock. The
    /// value is `Arc<AdvancedProcessorSet>` so callers can clone the
    /// Arc out of the shard guard and drop the guard before awaiting
    /// — the shard guard is `!Send` and would otherwise infect every
    /// task that holds one across `.await`.
    pub(super) advanced_processors: Arc<DashMap<DialogId, Arc<AdvancedProcessorSet>>>,
    /// Default configuration for advanced processors
    pub(super) default_processor_config: AdvancedProcessorConfig,
    /// SIMD processor for audio operations
    pub(super) simd_processor: SimdProcessor,

    /// Audio frame callbacks for sending decoded frames to session-core.
    /// `DashMap` so per-frame dispatch (called from the recv hot
    /// path) doesn't acquire an outer async RwLock just to find the
    /// per-dialog sender.
    pub(super) audio_frame_callbacks: Arc<DashMap<DialogId, mpsc::Sender<AudioFrame>>>,

    /// RFC 4733 DTMF callbacks for sending decoded digits to
    /// session-core. Fired once per digit (on the first observed
    /// end-of-event frame), deduping the two extra RFC 4733 §2.5.1.3
    /// retransmissions on the sender side.
    pub(super) dtmf_callbacks: Arc<DashMap<DialogId, mpsc::Sender<DtmfNotification>>>,

    /// Codec mapper for payload type resolution
    pub(super) codec_mapper: Arc<CodecMapper>,

    /// RTP bridge for processing incoming packets
    pub(super) rtp_bridge: Arc<RtpBridge>,

    /// Bidirectional partner map for sessions bridged at the RTP layer.
    /// Both directions are stored so lookup is O(1) from either end. Cleared
    /// on `BridgeHandle` drop or `stop_media` of a bridged session.
    pub(super) bridge_partners: Arc<DashMap<DialogId, DialogId>>,

    /// Sprint 3.6 C1 follow-up — RFC 3389 comfort-noise gate state per
    /// dialog. Lazily populated on first audio frame for sessions
    /// whose controller has [`comfort_noise_enabled`] set; the gate
    /// runs the simple energy/ZCR VAD over each outbound frame and
    /// returns `Send`/`Suppress`/`EmitCnThenSuppress`. Cleared on
    /// `stop_media` alongside the rest of the per-dialog state.
    ///
    /// [`comfort_noise_enabled`]: Self::comfort_noise_enabled
    pub(super) cn_gate_state:
        Arc<DashMap<DialogId, Arc<tokio::sync::Mutex<crate::relay::controller::cn_gate::CnGate>>>>,

    /// Sprint 3.6 C1 follow-up — global toggle. When `true`,
    /// `encode_and_send_audio_frame` consults the per-dialog
    /// [`cn_gate_state`](Self::cn_gate_state) to decide whether to
    /// send the audio packet, suppress it, or emit one PT 13 CN
    /// packet first. Wired from
    /// `session-core::Config::comfort_noise_enabled` via
    /// [`Self::set_comfort_noise_enabled`].
    pub(super) comfort_noise_enabled: Arc<std::sync::atomic::AtomicBool>,

    /// Per-dialog RTP/audio direction. This is the media-core enforcement
    /// point for SIP hold/resume and remote direction changes.
    pub(super) media_directions: Arc<DashMap<DialogId, MediaDirection>>,

    /// Per-dialog G.729 encoder state. G.729 is stateful, so unlike G.711 it
    /// cannot be safely recreated for each outbound RTP packet.
    #[cfg(feature = "g729")]
    pub(super) g729_tx_codecs: Arc<DashMap<DialogId, Arc<tokio::sync::Mutex<G729Codec>>>>,

    /// RTP session queue sizing for new sessions.
    rtp_session_buffer_config: RtpSessionBufferConfig,

    /// RTP transport event and receive buffer sizing for new sessions.
    rtp_transport_buffer_config: RtpTransportBufferConfig,

    /// Source-compatible policy sidecar for symmetric-RTP learning. This is
    /// configured through a controller builder instead of adding a field to
    /// the public `MediaSessionControllerConfig` struct.
    symmetric_rtp_policy: SymmetricRtpPolicy,
}

impl MediaSessionController {
    /// Create a new media session controller
    pub fn new() -> Self {
        Self::new_with_capacity_hint(0)
    }

    /// Create a media session controller with preallocated hot indexes.
    pub fn with_capacity(capacity: usize) -> Self {
        Self::new_with_capacity_hint(capacity)
    }

    /// Create a media session controller with explicit capacity and pool tuning.
    pub fn with_config(config: MediaSessionControllerConfig) -> Self {
        Self::new_with_config(config)
    }

    fn new_with_capacity_hint(capacity_hint: usize) -> Self {
        Self::new_with_config(MediaSessionControllerConfig {
            capacity_hint,
            ..Default::default()
        })
    }

    fn new_with_config(config: MediaSessionControllerConfig) -> Self {
        let capacity_hint = config.capacity_hint;
        let rtp_session_buffer_config = config.rtp_session_buffer_config;
        let rtp_transport_buffer_config = config.rtp_transport_buffer_config;
        let (event_tx, _event_rx) = mpsc::unbounded_channel();
        let (conference_event_tx, conference_event_rx) = mpsc::unbounded_channel();

        // Initialize performance components
        let performance_metrics = Arc::new(ConcurrentPerformanceMetrics::new());

        // Create global frame pool (shared across sessions)
        let frame_pool: Arc<AudioFramePool> = AudioFramePool::new(config.audio_frame_pool.clone());

        // Create RTP buffer pool
        let rtp_buffer_pool = RtpBufferPool::new(
            config.rtp_buffer_size,
            config.rtp_buffer_initial_count,
            config.rtp_buffer_max_count,
        );

        // Default advanced processor configuration
        let default_processor_config = AdvancedProcessorConfig::default();

        // G.711 µ-law / A-law codecs are stateless (just `variant` +
        // `frame_size`), so we instantiate fresh per call instead of
        // sharing one tokio::Mutex<G711Codec> across every session.
        // That mutex was the single biggest serialisation point on the
        // per-frame encode/decode hot path under load — see Phase C5.

        // Create SIMD processor
        let simd_processor = SimdProcessor::new();

        // Create codec mapper
        let codec_mapper = Arc::new(CodecMapper::new());

        // Create RTP bridge with its dependencies
        let (integration_event_tx, _integration_event_rx) = mpsc::unbounded_channel();
        let codec_detector = Arc::new(CodecDetector::new(codec_mapper.clone()));
        let fallback_manager = Arc::new(CodecFallbackManager::new(
            codec_detector.clone(),
            codec_mapper.clone(),
        ));
        let rtp_bridge = Arc::new(RtpBridge::new(
            RtpBridgeConfig::default(),
            integration_event_tx,
            codec_mapper.clone(),
            codec_detector,
            fallback_manager,
        ));

        Self {
            sessions: Arc::new(DashMap::with_capacity(capacity_hint)),
            rtp_sessions: Arc::new(DashMap::with_capacity(capacity_hint)),
            event_tx,
            event_rx: RwLock::new(None),
            event_hub: Arc::new(RwLock::new(None)),
            session_to_media: Arc::new(DashMap::with_capacity(capacity_hint)),
            media_to_session: Arc::new(DashMap::with_capacity(capacity_hint)),
            audio_mixer: None,
            conference_config: ConferenceMixingConfig::default(),
            conference_event_tx,
            conference_event_rx: RwLock::new(Some(conference_event_rx)),
            quality_monitor: None,
            port_allocator: None, // Use GlobalPortAllocator by default
            // Performance fields
            performance_metrics,
            frame_pool,
            rtp_buffer_pool,
            advanced_processors: Arc::new(DashMap::with_capacity(capacity_hint)),
            default_processor_config,
            simd_processor,
            audio_frame_callbacks: Arc::new(DashMap::with_capacity(capacity_hint)),
            dtmf_callbacks: Arc::new(DashMap::with_capacity(capacity_hint)),
            codec_mapper,
            rtp_bridge,
            bridge_partners: Arc::new(DashMap::with_capacity(capacity_hint)),
            cn_gate_state: Arc::new(DashMap::with_capacity(capacity_hint)),
            comfort_noise_enabled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            media_directions: Arc::new(DashMap::with_capacity(capacity_hint)),
            #[cfg(feature = "g729")]
            g729_tx_codecs: Arc::new(DashMap::with_capacity(capacity_hint)),
            rtp_session_buffer_config,
            rtp_transport_buffer_config,
            symmetric_rtp_policy: SymmetricRtpPolicy::default(),
        }
    }

    /// Configure bounded symmetric-RTP learning for sessions created by this
    /// controller.
    pub fn with_symmetric_rtp_policy(mut self, policy: SymmetricRtpPolicy) -> Self {
        self.symmetric_rtp_policy = policy;
        self
    }

    /// Toggle RFC 3389 Comfort Noise gating on the audio TX path.
    /// When enabled, [`encode_and_send_audio_frame`](Self::encode_and_send_audio_frame)
    /// runs a per-dialog VAD over each outbound PCM frame and either
    /// sends the audio normally, suppresses it (silence already
    /// covered), or emits one PT 13 CN packet then suppresses
    /// (speech→silence transition / §4.1 refresh). Wired from
    /// `session-core::Config::comfort_noise_enabled` at coordinator
    /// boot via the media adapter.
    pub fn set_comfort_noise_enabled(&self, enabled: bool) {
        self.comfort_noise_enabled
            .store(enabled, std::sync::atomic::Ordering::Relaxed);
        if !enabled {
            // Drop any per-dialog gate state — next time CN is
            // re-enabled the gate will rebuild fresh.
            self.cn_gate_state.clear();
        }
    }

    /// Register an RTP event callback with the RTP bridge
    /// This allows external subscribers (like session-core) to receive RTP events
    pub async fn add_rtp_event_callback(&self, callback: RtpEventCallback) {
        self.rtp_bridge.add_rtp_event_callback(callback).await;
    }

    // ===== Event Hub Helper Methods =====

    /// Set the event hub for global event coordination
    pub async fn set_event_hub(&self, event_hub: Arc<crate::events::MediaEventHub>) {
        *self.event_hub.write().await = Some(event_hub);
    }

    /// Store session to media mapping
    pub fn store_session_mapping(&self, session_id: String, media_id: MediaSessionId) {
        self.session_to_media
            .insert(session_id.clone(), media_id.clone());
        self.media_to_session.insert(media_id, session_id);
    }

    /// Get media session ID from session ID
    pub fn get_media_id(&self, session_id: &str) -> Option<MediaSessionId> {
        self.session_to_media
            .get(session_id)
            .map(|e| e.value().clone())
    }

    /// Get session ID from media session ID
    pub fn get_session_id(&self, media_id: &MediaSessionId) -> Option<String> {
        self.media_to_session
            .get(media_id)
            .map(|e| e.value().clone())
    }

    /// Feature-gated retained-object counts for perf leak investigations.
    #[cfg(feature = "perf-diagnostics")]
    #[doc(hidden)]
    pub fn diagnostic_counts(&self) -> serde_json::Value {
        let mut rtp_sender_queue_packets = 0usize;
        let mut rtp_sender_capacity_packets = 0usize;
        let mut rtp_receiver_queue_packets = 0usize;
        let mut rtp_receiver_capacity_packets = 0usize;
        let mut rtp_event_queue_events = 0usize;
        let mut rtp_event_receiver_count = 0usize;
        let mut rtp_sessions_locked_for_diag = 0usize;
        #[cfg(feature = "memory-diagnostics")]
        let mut rtp_streams = 0usize;

        for entry in self.rtp_sessions.iter() {
            match entry.value().session.try_lock() {
                Ok(session) => {
                    let counts = session.queue_diagnostics();
                    rtp_sender_queue_packets += counts.sender_queue_packets;
                    rtp_sender_capacity_packets += counts.sender_capacity_packets;
                    rtp_receiver_queue_packets += counts.receiver_queue_packets;
                    rtp_receiver_capacity_packets += counts.receiver_capacity_packets;
                    rtp_event_queue_events += counts.event_queue_events;
                    rtp_event_receiver_count += counts.event_receiver_count;
                    #[cfg(feature = "memory-diagnostics")]
                    {
                        rtp_streams += counts.stream_count;
                    }
                }
                Err(_) => {
                    rtp_sessions_locked_for_diag += 1;
                }
            }
        }

        let audio_callback_queue_frames: usize = self
            .audio_frame_callbacks
            .iter()
            .map(|entry| {
                let sender = entry.value();
                sender.max_capacity().saturating_sub(sender.capacity())
            })
            .sum();
        let audio_callback_capacity_frames: usize = self
            .audio_frame_callbacks
            .iter()
            .map(|entry| entry.value().max_capacity())
            .sum();

        #[cfg_attr(not(feature = "memory-diagnostics"), allow(unused_mut))]
        let mut value = serde_json::json!({
            "sessions": self.sessions.len(),
            "rtp_sessions": self.rtp_sessions.len(),
            "rtp_sender_queue_packets": rtp_sender_queue_packets,
            "rtp_sender_capacity_packets": rtp_sender_capacity_packets,
            "rtp_receiver_queue_packets": rtp_receiver_queue_packets,
            "rtp_receiver_capacity_packets": rtp_receiver_capacity_packets,
            "rtp_event_queue_events": rtp_event_queue_events,
            "rtp_event_receiver_count": rtp_event_receiver_count,
            "rtp_sessions_locked_for_diag": rtp_sessions_locked_for_diag,
            "session_to_media": self.session_to_media.len(),
            "media_to_session": self.media_to_session.len(),
            "audio_frame_callbacks": self.audio_frame_callbacks.len(),
            "audio_callback_queue_frames": audio_callback_queue_frames,
            "audio_callback_capacity_frames": audio_callback_capacity_frames,
            "dtmf_callbacks": self.dtmf_callbacks.len(),
            "bridge_partners": self.bridge_partners.len(),
            "cn_gate_state": self.cn_gate_state.len(),
            "advanced_processors": self.advanced_processors.len(),
            "media_directions": self.media_directions.len(),
        });
        #[cfg(feature = "memory-diagnostics")]
        if let Some(obj) = value.as_object_mut() {
            obj.insert("rtp_streams".into(), serde_json::json!(rtp_streams));
            obj.insert("frame_pool".into(), self.frame_pool.diagnostic_counts());
            obj.insert(
                "rtp_buffer_pool".into(),
                self.rtp_buffer_pool.diagnostic_counts(),
            );
        }
        value
    }

    /// Emit a media event through both channel and event hub. Held
    /// alongside the public `take_event_receiver` path; new emission
    /// sites will use this helper once dialog events get wired in.
    #[allow(dead_code)]
    async fn emit_event(&self, event: MediaSessionEvent) {
        // Send to channel (legacy)
        let _ = self.event_tx.send(event.clone());

        // Send to event hub (new global bus)
        if let Some(hub) = self.event_hub.read().await.as_ref() {
            if let Err(e) = hub.publish_media_event(event).await {
                warn!("Failed to publish media event to global bus: {}", e);
            }
        }
    }

    /// Create a new media session controller with custom port range
    pub fn with_port_range(base_port: u16, max_port: u16) -> Self {
        Self::with_port_range_and_capacity(base_port, max_port, 0)
    }

    /// Create a media session controller with custom port range and capacity.
    pub fn with_port_range_and_capacity(
        base_port: u16,
        max_port: u16,
        capacity_hint: usize,
    ) -> Self {
        Self::with_port_range_and_config(
            base_port,
            max_port,
            MediaSessionControllerConfig {
                capacity_hint,
                ..Default::default()
            },
        )
    }

    /// Create a media session controller with custom port range and explicit tuning.
    pub fn with_port_range_and_config(
        base_port: u16,
        max_port: u16,
        config: MediaSessionControllerConfig,
    ) -> Self {
        let capacity_hint = config.capacity_hint;
        let mut controller = Self::new_with_config(config);

        // Create a custom port allocator with the specified range
        let mut config = PortAllocatorConfig::default();
        config.port_range_start = base_port;
        config.port_range_end = max_port;
        config.validate_ports = false;
        config.capacity_hint = capacity_hint;
        if capacity_hint > 0 {
            config.allocation_strategy = AllocationStrategy::Incremental;
            config.prefer_port_reuse = false;
            config.allocation_retries = configured_port_range_len(base_port, max_port) as u32;
        }

        controller.port_allocator = Some(Arc::new(PortAllocator::with_config(config)));
        info!(
            "Created MediaSessionController with custom port range {}-{}",
            base_port, max_port
        );

        controller
    }

    /// Create a new media session controller with conference audio mixing enabled
    pub async fn with_conference_mixing(
        base_port: u16,
        max_port: u16,
        conference_config: ConferenceMixingConfig,
    ) -> Result<Self> {
        let (event_tx, _event_rx) = mpsc::unbounded_channel();
        let (conference_event_tx, conference_event_rx) = mpsc::unbounded_channel();

        // Create audio mixer with the provided configuration
        let audio_mixer: Arc<AudioMixer> =
            Arc::new(AudioMixer::new(conference_config.clone()).await?);

        // Set up conference event forwarding
        audio_mixer
            .set_event_sender(conference_event_tx.clone())
            .await;

        // Initialize performance components
        let performance_metrics = Arc::new(ConcurrentPerformanceMetrics::new());

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
            128, // Max buffer count (more for conference)
        );

        // Default advanced processor configuration for conference
        let mut default_processor_config = AdvancedProcessorConfig::default();
        default_processor_config.frame_pool_size = 32; // Per-session pool size
        default_processor_config.enable_simd = conference_config.enable_simd_optimization;

        // G.711 µ-law / A-law codecs are stateless (just `variant` +
        // `frame_size`); see Phase C5 comment on the matching block in
        // `Self::new()`. Per-call instantiation avoids the shared
        // tokio::Mutex<G711Codec> hot-path lock that previously
        // serialised every encode/decode in every concurrent session.

        // Create SIMD processor
        let simd_processor = SimdProcessor::new();

        // Create codec mapper
        let codec_mapper = Arc::new(CodecMapper::new());

        // Create RTP bridge with its dependencies
        let (integration_event_tx, _integration_event_rx) = mpsc::unbounded_channel();
        let codec_detector = Arc::new(CodecDetector::new(codec_mapper.clone()));
        let fallback_manager = Arc::new(CodecFallbackManager::new(
            codec_detector.clone(),
            codec_mapper.clone(),
        ));
        let rtp_bridge = Arc::new(RtpBridge::new(
            RtpBridgeConfig::default(),
            integration_event_tx,
            codec_mapper.clone(),
            codec_detector,
            fallback_manager,
        ));

        // Create a custom port allocator with the specified range
        let mut port_config = PortAllocatorConfig::default();
        port_config.port_range_start = base_port;
        port_config.port_range_end = max_port;
        port_config.validate_ports = false;
        port_config.allocation_strategy = AllocationStrategy::Incremental;
        port_config.prefer_port_reuse = false;
        port_config.allocation_retries = configured_port_range_len(base_port, max_port) as u32;
        let port_allocator = Some(Arc::new(PortAllocator::with_config(port_config)));

        Ok(Self {
            sessions: Arc::new(DashMap::new()),
            rtp_sessions: Arc::new(DashMap::new()),
            event_tx,
            event_rx: RwLock::new(None),
            event_hub: Arc::new(RwLock::new(None)),
            session_to_media: Arc::new(DashMap::new()),
            media_to_session: Arc::new(DashMap::new()),
            audio_mixer: Some(audio_mixer),
            conference_config,
            conference_event_tx,
            conference_event_rx: RwLock::new(Some(conference_event_rx)),
            quality_monitor: None,
            port_allocator,
            // Performance fields
            performance_metrics,
            frame_pool,
            rtp_buffer_pool,
            advanced_processors: Arc::new(DashMap::new()),
            default_processor_config,
            simd_processor,
            audio_frame_callbacks: Arc::new(DashMap::new()),
            dtmf_callbacks: Arc::new(DashMap::new()),
            codec_mapper,
            rtp_bridge,
            bridge_partners: Arc::new(DashMap::new()),
            cn_gate_state: Arc::new(DashMap::new()),
            comfort_noise_enabled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            media_directions: Arc::new(DashMap::new()),
            #[cfg(feature = "g729")]
            g729_tx_codecs: Arc::new(DashMap::new()),
            rtp_session_buffer_config: RtpSessionBufferConfig::default(),
            rtp_transport_buffer_config: RtpTransportBufferConfig::default(),
            symmetric_rtp_policy: SymmetricRtpPolicy::default(),
        })
    }

    /// Start a media session for a dialog
    pub async fn start_media(&self, dialog_id: DialogId, config: MediaConfig) -> Result<()> {
        let media_start_guard = diagnostics::MediaStartGuard::new();
        info!("Starting media session for dialog: {}", dialog_id);

        // Check if media session already exists for this dialog
        if self.sessions.contains_key(&dialog_id) {
            return Err(Error::config(format!(
                "Media session already exists for dialog: {}",
                dialog_id
            )));
        }

        // Allocate RTP port using either our local allocator or the global one
        let allocator = if let Some(ref port_alloc) = self.port_allocator {
            // Use our custom port allocator with configured range
            port_alloc.clone()
        } else {
            // Fall back to global allocator
            GlobalPortAllocator::instance().await
        };

        // Determine payload type from preferred codec
        let payload_type = config
            .preferred_codec
            .as_ref()
            .and_then(|codec| self.codec_mapper.codec_to_payload(codec))
            .unwrap_or(0); // Default to PCMU

        // Determine clock rate based on codec
        let clock_rate = config
            .preferred_codec
            .as_ref()
            .map(|codec| self.codec_mapper.get_clock_rate(codec))
            .unwrap_or(8000);

        let dialog_session_id = format!("dialog_{}", dialog_id);
        let mut last_bind_error: Option<rtp_core::Error> = None;
        let mut created_session = None;
        for attempt in 1..=RTP_SESSION_BIND_RETRIES {
            let allocate_started = Instant::now();
            let (local_rtp_addr, _) = allocator
                .allocate_port_pair(&dialog_session_id, Some(config.local_addr.ip()))
                .await
                .map_err(|e| Error::config(format!("Failed to allocate RTP port: {}", e)))?;
            diagnostics::record_rtp_port_allocate(allocate_started.elapsed());
            let mut reservation_guard =
                MediaPortReservationGuard::new(allocator.clone(), dialog_session_id.clone());

            // Create RTP session configuration
            let rtp_config = RtpSessionConfig {
                local_addr: local_rtp_addr,
                remote_addr: config.remote_addr,
                ssrc: Some(rand::random()),    // Generate random SSRC
                payload_type,                  // Use negotiated payload type
                clock_rate,                    // Use codec-appropriate clock rate
                jitter_buffer_size: Some(500), // Increased from 50 to handle burst traffic
                max_packet_age_ms: Some(1000), // Increased from 200ms to 1s for localhost testing
                enable_jitter_buffer: false,   // Disabled to reduce processing overhead
                session_buffer_config: self.rtp_session_buffer_config,
                transport_buffer_config: self.rtp_transport_buffer_config,
            };

            let session_started = Instant::now();
            match RtpSession::new_event_driven_with_symmetric_rtp_policy(
                rtp_config,
                self.symmetric_rtp_policy,
            )
            .await
            {
                Ok(rtp_session) => {
                    diagnostics::record_rtp_session_new(session_started.elapsed());
                    created_session = Some((local_rtp_addr, rtp_session, reservation_guard));
                    break;
                }
                Err(e) => {
                    diagnostics::record_rtp_session_new(session_started.elapsed());
                    reservation_guard.disarm();
                    let _ = allocator.release_session(&dialog_session_id).await;
                    let should_retry =
                        is_retryable_rtp_bind_error(&e) && attempt < RTP_SESSION_BIND_RETRIES;
                    if should_retry {
                        // Try the next reserved port. The allocator no longer probe-binds
                        // for media-controller sessions; the real RTP socket bind is the
                        // authoritative availability check.
                        debug!(
                            "RTP bind failed for {} on {} (attempt {}/{}); retrying with another port: {}",
                            dialog_id, local_rtp_addr, attempt, RTP_SESSION_BIND_RETRIES, e
                        );
                        last_bind_error = Some(e);
                        continue;
                    }

                    return Err(Error::config(format!(
                        "Failed to create RTP session on {}: {}",
                        local_rtp_addr, e
                    )));
                }
            }
        }

        let (local_rtp_addr, rtp_session, mut reservation_guard) =
            created_session.ok_or_else(|| {
                Error::config(format!(
                    "Failed to create RTP session after {} bind attempts: {}",
                    RTP_SESSION_BIND_RETRIES,
                    last_bind_error
                        .map(|e| e.to_string())
                        .unwrap_or_else(|| "no bind attempt completed".to_string())
                ))
            })?;

        let rtp_port = local_rtp_addr.port();

        // Subscribe to RTP session events before wrapping
        let subscribe_started = Instant::now();
        let rtp_events = rtp_session.subscribe();
        diagnostics::record_rtp_event_subscription(subscribe_started.elapsed());

        // Wrap RTP session
        let rtp_wrapper = RtpSessionWrapper {
            session: Arc::new(tokio::sync::Mutex::new(rtp_session)),
            local_addr: local_rtp_addr,
            remote_addr: config.remote_addr,
            created_at: std::time::Instant::now(),
            audio_transmitter: None,
            transmission_enabled: true, // Enable transmission by default
            is_muted: false,
            #[cfg(feature = "memory-diagnostics")]
            memory_guard: rvoip_infra_common::memory_diagnostics::ObjectGuard::new(
                "media_core.rtp_session_wrapper",
                std::mem::size_of::<RtpSessionWrapper>(),
            ),
        };
        self.media_directions
            .insert(dialog_id.clone(), MediaDirection::SendRecv);

        // Create media session info
        let session_info = MediaSessionInfo {
            dialog_id: dialog_id.clone(),
            status: MediaSessionStatus::Active,
            config: config.clone(),
            rtp_port: Some(rtp_port),
            rtp_stats: None,
            stats_updated_at: None,
            created_at: std::time::Instant::now(),
        };

        // Store session and RTP session (DashMap inserts are sharded
        // and don't take any outer guard).
        #[cfg(feature = "memory-diagnostics")]
        rvoip_infra_common::memory_diagnostics::record_created(
            "media_core.media_session_info",
            std::mem::size_of::<MediaSessionInfo>(),
        );
        self.sessions.insert(dialog_id.clone(), session_info);
        self.rtp_sessions.insert(dialog_id.clone(), rtp_wrapper);
        // The controller maps now own the RTP session and stop_media owns the
        // matching allocator release.
        reservation_guard.disarm();

        // Send event
        let _ = self.event_tx.send(MediaSessionEvent::SessionCreated {
            dialog_id: dialog_id.clone(),
            session_id: dialog_id.clone(),
        });

        // Spawn task to handle RTP events for this session
        let handler_started = Instant::now();
        self.spawn_rtp_event_handler(dialog_id.clone(), rtp_events, payload_type);
        diagnostics::record_rtp_event_handler_spawn(handler_started.elapsed());

        info!(
            "✅ Created media session with REAL RTP session: {} (port: {}, codec: {}, PT: {}, clock: {}Hz)",
            dialog_id,
            rtp_port,
            config.preferred_codec.as_deref().unwrap_or("PCMU"),
            payload_type,
            clock_rate
        );
        media_start_guard.finish_success();
        Ok(())
    }

    /// Install RFC 4568 SDES-SRTP contexts on the dialog's RTP
    /// session, switching its transport from plain RTP to encrypted
    /// SRTP for both directions.
    ///
    /// Must be called after [`Self::start_media`] (the RTP session
    /// must already exist) and before audio transmission begins so
    /// that no plaintext packets leak. The contexts are consumed —
    /// each call replaces any previously-installed pair (allowing
    /// re-keying after a session refresh, though that's not yet
    /// driven from session-core).
    pub async fn install_srtp_contexts(
        &self,
        dialog_id: &DialogId,
        send_ctx: rvoip_rtp_core::srtp::SrtpContext,
        recv_ctx: rvoip_rtp_core::srtp::SrtpContext,
    ) -> Result<()> {
        // Extract the session Arc from the DashMap shard, drop the
        // shard guard before any `.await`, then lock the per-session
        // mutex against just our caller — no cross-dialog
        // serialisation.
        let session_arc = self
            .rtp_sessions
            .get(dialog_id)
            .map(|r| r.value().session.clone())
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;

        // RtpSession exposes its transport via a typed accessor; we
        // need the UdpRtpTransport concrete type to reach
        // `set_srtp_contexts`. Downcast once — the only transport
        // type media-core constructs is UDP, so this is the
        // architecturally-correct narrowing.
        let session_guard = session_arc.lock().await;
        let transport = session_guard.transport();
        let udp_transport = transport
            .as_any()
            .downcast_ref::<rvoip_rtp_core::transport::UdpRtpTransport>()
            .ok_or_else(|| {
                Error::config(
                    "install_srtp_contexts: RTP session is not a UdpRtpTransport".to_string(),
                )
            })?;
        udp_transport.set_srtp_contexts(send_ctx, recv_ctx).await;
        info!("Installed SDES-SRTP contexts on dialog {}", dialog_id);
        Ok(())
    }

    fn cleanup_per_dialog_side_state(&self, dialog_id: &DialogId) {
        self.media_directions.remove(dialog_id);
        self.advanced_processors.remove(dialog_id);
        self.audio_frame_callbacks.remove(dialog_id);
        self.dtmf_callbacks.remove(dialog_id);
        self.cn_gate_state.remove(dialog_id);
        #[cfg(feature = "g729")]
        self.g729_tx_codecs.remove(dialog_id);

        let media_id = MediaSessionId::from_dialog(dialog_id);
        if let Some((_, session_id)) = self.media_to_session.remove(&media_id) {
            // A raw session identifier can be rebound to a newer media
            // generation before delayed cleanup of the old reverse entry is
            // observed. Remove the forward entry only when it still points at
            // the exact media id being stopped.
            self.session_to_media
                .remove_if(&session_id, |_, mapped| mapped == &media_id);
        }

        let stale_session_ids: Vec<String> = self
            .session_to_media
            .iter()
            .filter_map(|entry| {
                if entry.value() == &media_id {
                    Some(entry.key().clone())
                } else {
                    None
                }
            })
            .collect();
        for session_id in stale_session_ids {
            self.session_to_media
                .remove_if(&session_id, |_, mapped| mapped == &media_id);
        }
    }

    /// Stop media session for a dialog.
    ///
    /// Idempotent: repeated cleanup attempts still clear callbacks and side
    /// maps and then return `Ok(())`.
    pub async fn stop_media(&self, dialog_id: &DialogId) -> Result<()> {
        let stop_started = Instant::now();
        info!("Stopping media session for dialog: {}", dialog_id);

        // If the session is bridged, clear the partnership so the partner's
        // forwarder task can exit cleanly rather than sending to a dead
        // session. Does nothing when no bridge is active.
        self.clear_bridge_partner(dialog_id);

        // Remove session and get info for cleanup. DashMap removes
        // are sharded; no outer guard.
        let session_info = match self.sessions.remove(dialog_id) {
            Some((_, info)) => {
                #[cfg(feature = "memory-diagnostics")]
                rvoip_infra_common::memory_diagnostics::record_dropped(
                    "media_core.media_session_info",
                    std::mem::size_of::<MediaSessionInfo>(),
                );
                info
            }
            None => {
                self.cleanup_per_dialog_side_state(dialog_id);
                diagnostics::record_stop_media(stop_started.elapsed());
                debug!(
                    "Media session for dialog {} was already stopped; cleared side state",
                    dialog_id
                );
                return Ok(());
            }
        };

        // Stop and remove RTP session. Extract the wrapper out of the
        // shard, drop the guard, then close the underlying session
        // outside the map's locking territory.
        if let Some((_, mut rtp_wrapper)) = self.rtp_sessions.remove(dialog_id) {
            if let Some(transmitter) = rtp_wrapper.audio_transmitter.take() {
                transmitter.stop().await;
            }
            let mut rtp_session = rtp_wrapper.session.lock().await;
            let _ = rtp_session.close().await;
            info!("✅ Stopped RTP session for dialog: {}", dialog_id);
        }

        // Release port via the appropriate allocator
        if session_info.rtp_port.is_some() {
            let allocator = if let Some(ref port_alloc) = self.port_allocator {
                // Use our custom port allocator
                port_alloc.clone()
            } else {
                // Fall back to global allocator
                GlobalPortAllocator::instance().await
            };

            let dialog_session_id = format!("dialog_{}", dialog_id);
            let release_started = Instant::now();
            if let Err(e) = allocator.release_session(&dialog_session_id).await {
                warn!("Failed to release ports for dialog {}: {}", dialog_id, e);
            }
            diagnostics::record_port_release(release_started.elapsed());
        }

        self.cleanup_per_dialog_side_state(dialog_id);

        // Send event
        let _ = self.event_tx.send(MediaSessionEvent::SessionDestroyed {
            dialog_id: dialog_id.clone(),
            session_id: dialog_id.clone(),
        });

        diagnostics::record_stop_media(stop_started.elapsed());
        Ok(())
    }

    /// Update media configuration (e.g., when remote address becomes known or codec changes during re-INVITE).
    ///
    /// Restructured for DashMap: each shard guard is acquired,
    /// mutated synchronously, and dropped before any `.await`. The
    /// actual RTP-session mutations happen on the per-session Arc
    /// after the map guards are released.
    pub async fn update_media(&self, dialog_id: DialogId, config: MediaConfig) -> Result<()> {
        info!("Updating media session for dialog: {}", dialog_id);

        // Update the session config and snapshot the old values for
        // change detection. The shard guard is dropped at the end of
        // this block.
        let (old_remote, old_codec) = {
            let mut entry = self
                .sessions
                .get_mut(&dialog_id)
                .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;
            let session_info = entry.value_mut();
            let old_remote = session_info.config.remote_addr;
            let old_codec = session_info.config.preferred_codec.clone();
            session_info.config = config.clone();
            (old_remote, old_codec)
        };

        // Extract the per-session Arc + update the wrapper's remote
        // address in the same shard-guard scope. Drop the shard
        // guard before we touch the RTP session itself.
        let rtp_session_arc = {
            let Some(mut entry) = self.rtp_sessions.get_mut(&dialog_id) else {
                warn!(
                    "No RTP session found for dialog {} during update",
                    dialog_id
                );
                return Ok(());
            };
            let wrapper = entry.value_mut();
            if config.remote_addr != old_remote {
                wrapper.remote_addr = config.remote_addr;
            }
            wrapper.session.clone()
        };

        let mut updates_made = false;

        // Apply remote-address change.
        if config.remote_addr != old_remote {
            if let Some(remote_addr) = config.remote_addr {
                let mut rtp_session = rtp_session_arc.lock().await;
                rtp_session.set_remote_addr(remote_addr).await;
                drop(rtp_session);

                info!(
                    "✅ Updated RTP session remote address for dialog {}: {}",
                    dialog_id, remote_addr
                );
                updates_made = true;

                let _ = self.event_tx.send(MediaSessionEvent::RemoteAddressUpdated {
                    dialog_id: dialog_id.clone(),
                    remote_addr,
                });
            }
        }

        // Apply codec change.
        if config.preferred_codec != old_codec {
            #[cfg(feature = "g729")]
            self.g729_tx_codecs.remove(&dialog_id);

            let new_payload_type = config
                .preferred_codec
                .as_ref()
                .and_then(|codec| self.codec_mapper.codec_to_payload(codec))
                .unwrap_or(0);
            let new_clock_rate = config
                .preferred_codec
                .as_ref()
                .map(|codec| self.codec_mapper.get_clock_rate(codec))
                .unwrap_or(8000);

            {
                let mut rtp_session = rtp_session_arc.lock().await;
                rtp_session.set_payload_type(new_payload_type);

                if rtp_session.get_payload_type() != new_payload_type {
                    warn!("Failed to update payload type for dialog {}", dialog_id);
                } else {
                    debug!(
                        "Successfully updated payload type to {} for dialog {}",
                        new_payload_type, dialog_id
                    );
                }

                // TODO: Implement clock rate updates in rtp-core session
                debug!(
                    "Clock rate change noted for dialog {} ({}Hz), but full update requires rtp-core enhancement",
                    dialog_id, new_clock_rate
                );
            }

            updates_made = true;

            // Log codec change with detailed information
            let old_codec_name = old_codec.as_deref().unwrap_or("PCMU");
            let new_codec_name = config.preferred_codec.as_deref().unwrap_or("PCMU");
            let old_payload_type = old_codec
                .as_ref()
                .and_then(|codec| self.codec_mapper.codec_to_payload(codec))
                .unwrap_or(0);
            let old_clock_rate = old_codec
                .as_ref()
                .map(|codec| self.codec_mapper.get_clock_rate(codec))
                .unwrap_or(8000);

            info!(
                "🔄 Codec changed for dialog {}: {} -> {} (PT: {} -> {}, Clock: {}Hz -> {}Hz)",
                dialog_id,
                old_codec_name,
                new_codec_name,
                old_payload_type,
                new_payload_type,
                old_clock_rate,
                new_clock_rate
            );

            let _ = self.event_tx.send(MediaSessionEvent::CodecChanged {
                dialog_id: dialog_id.clone(),
                old_codec: old_codec.clone(),
                new_codec: config.preferred_codec.clone(),
                new_payload_type,
                new_clock_rate,
            });
        }

        if updates_made {
            info!(
                "✅ Media session successfully updated for dialog: {}",
                dialog_id
            );
        } else {
            debug!("No RTP session updates needed for dialog: {}", dialog_id);
        }

        Ok(())
    }

    /// Get information about a media session
    pub async fn get_session_info(&self, dialog_id: &DialogId) -> Option<MediaSessionInfo> {
        // Clone the MediaSessionInfo out of the shard, drop the
        // guard, then enrich with stats (an async call that may
        // re-touch other maps).
        let mut info = self.sessions.get(dialog_id).map(|r| r.value().clone())?;
        info.rtp_stats = self.get_rtp_statistics(dialog_id).await;
        info.stats_updated_at = Some(Instant::now());
        Some(info)
    }

    /// Get all active sessions
    pub async fn get_all_sessions(&self) -> Vec<MediaSessionInfo> {
        self.sessions.iter().map(|r| r.value().clone()).collect()
    }

    // REMOVED: Channel-based communication - use GlobalEventCoordinator instead
    // pub async fn take_event_receiver(&self) -> Option<mpsc::UnboundedReceiver<MediaSessionEvent>> {
    //     let mut event_rx = self.event_rx.write().await;
    //     event_rx.take()
    // }

    /// Set audio frame callback for a dialog
    pub async fn set_audio_frame_callback(
        &self,
        dialog_id: DialogId,
        sender: mpsc::Sender<AudioFrame>,
    ) -> Result<()> {
        self.audio_frame_callbacks.insert(dialog_id.clone(), sender);
        info!("🔊 Set audio frame callback for dialog: {}", dialog_id);
        Ok(())
    }

    /// Remove audio frame callback for a dialog
    pub async fn remove_audio_frame_callback(&self, dialog_id: &DialogId) -> Result<()> {
        if self.audio_frame_callbacks.remove(dialog_id).is_some() {
            debug!("🔇 Removed audio frame callback for dialog: {}", dialog_id);
        }
        Ok(())
    }

    /// Set RFC 4733 DTMF callback for a dialog. The callback fires once
    /// per digit (on the first observed `E=1` frame), so session-core
    /// can surface `Event::DtmfReceived` without dedup logic.
    pub async fn set_dtmf_callback(
        &self,
        dialog_id: DialogId,
        sender: mpsc::Sender<DtmfNotification>,
    ) -> Result<()> {
        self.dtmf_callbacks.insert(dialog_id.clone(), sender);
        info!("☎️  Set DTMF callback for dialog: {}", dialog_id);
        Ok(())
    }

    /// Remove RFC 4733 DTMF callback for a dialog.
    pub async fn remove_dtmf_callback(&self, dialog_id: &DialogId) -> Result<()> {
        if self.dtmf_callbacks.remove(dialog_id).is_some() {
            debug!("☎️  Removed DTMF callback for dialog: {}", dialog_id);
        }
        Ok(())
    }

    /// Send audio frame to session-core for a dialog. Extract the
    /// sender out of the DashMap shard and drop the guard before
    /// awaiting the send — `mpsc::Sender::send` is async and we don't
    /// want to hold a shard guard across it.
    pub async fn send_audio_frame(&self, dialog_id: &DialogId, frame: AudioFrame) -> Result<()> {
        let sender = self
            .audio_frame_callbacks
            .get(dialog_id)
            .map(|r| r.value().clone());
        if let Some(sender) = sender {
            if let Err(e) = sender.send(frame).await {
                warn!(
                    "Failed to send audio frame to session-core for dialog {}: {}",
                    dialog_id, e
                );
                return Err(Error::config(format!("Failed to send audio frame: {}", e)));
            }
            debug!(
                "📤 Sent audio frame to session-core for dialog: {}",
                dialog_id
            );
        }
        Ok(())
    }

    /// Spawn a task to handle RTP events and decode audio
    fn spawn_rtp_event_handler(
        &self,
        dialog_id: DialogId,
        mut rtp_events: tokio::sync::broadcast::Receiver<rtp_core::session::RtpSessionEvent>,
        _expected_payload_type: u8,
    ) {
        let audio_frame_callbacks = self.audio_frame_callbacks.clone();
        let dtmf_callbacks = self.dtmf_callbacks.clone();
        let _codec_mapper = self.codec_mapper.clone();
        let media_directions = self.media_directions.clone();

        // RFC 4733 §2.5.1.3 retransmit dedup formerly lived here as a
        // `(ssrc, rtp_timestamp)` seen-set. Sprint 2.5 P4 moved it
        // down to `rtp-core::transport::udp::UdpRtpTransport` (keyed
        // by `(peer_addr, ssrc, ts)`) so this handler now sees one
        // logical digit per tone — the upstream layer collapses the
        // three E=1 retransmits before they are even decoded into a
        // typed `DtmfEvent`.

        // Create G.711 codecs outside the loop for efficiency
        let mut g711_ulaw = G711Codec::mu_law(8000, 1).expect("Failed to create μ-law codec");
        let mut g711_alaw = G711Codec::a_law(8000, 1).expect("Failed to create A-law codec");
        #[cfg(feature = "g729")]
        let mut g729_decoder = G729Codec::new(
            crate::types::SampleRate::Rate8000,
            1,
            crate::codec::audio::G729Config::default(),
        )
        .expect("Failed to create G.729 decoder");
        let mut decode_buffer = vec![0i16; 160];
        let skip_audio_frame_delivery = perf_skip_audio_frame_delivery();
        let collect_audio_quality = diagnostics::audio_quality_enabled();

        #[cfg(feature = "memory-diagnostics")]
        let _decode_buffer_guard = rvoip_infra_common::memory_diagnostics::ObjectGuard::new(
            "media_core.audio.rx.decode_reusable_buffer",
            decode_buffer.capacity() * std::mem::size_of::<i16>(),
        );

        spawn_memory_tracked("media_core.rtp_event_handler_task", async move {
            info!("🎧 Started RTP event handler for dialog: {}", dialog_id);
            let mut rtp_count = 0u64;
            let mut decoded_audio_frame_count = 0u64;
            let mut logged_missing_audio_callback = false;
            let mut last_sequence_number: Option<u16> = None;
            let mut last_rtp_timestamp: Option<u32> = None;
            let mut last_rtp_arrival: Option<Instant> = None;
            let mut last_delivered_at: Option<Instant> = None;
            let mut jitter_ns = 0.0_f64;

            loop {
                match rtp_events.recv().await {
                    Ok(event) => {
                        match event {
                            rtp_core::session::RtpSessionEvent::PacketReceived(packet) => {
                                rtp_count += 1;
                                let packet_arrival = collect_audio_quality.then(Instant::now);

                                if rtp_count % 10 == 0
                                    || rtp_count == 100
                                    || rtp_count == 101
                                    || rtp_count > 100 && rtp_count < 110
                                {
                                    trace!(
                                        "📦 Received RTP packet #{} for dialog {}: PT={}, seq={}, ts={}, payload_size={}",
                                        rtp_count,
                                        dialog_id,
                                        packet.header.payload_type,
                                        packet.header.sequence_number,
                                        packet.header.timestamp,
                                        packet.payload.len()
                                    );
                                }

                                if let Some(arrival) = packet_arrival {
                                    let sequence_gap_packets =
                                        if let Some(previous) = last_sequence_number {
                                            let expected = previous.wrapping_add(1);
                                            if packet.header.sequence_number == expected {
                                                0
                                            } else {
                                                let gap = packet
                                                    .header
                                                    .sequence_number
                                                    .wrapping_sub(expected);
                                                if gap < 32_768 {
                                                    u64::from(gap)
                                                } else {
                                                    0
                                                }
                                            }
                                        } else {
                                            0
                                        };

                                    let interarrival_gap = last_rtp_arrival
                                        .map(|previous| arrival.duration_since(previous));
                                    if let (Some(previous_arrival), Some(previous_timestamp)) =
                                        (last_rtp_arrival, last_rtp_timestamp)
                                    {
                                        let arrival_delta =
                                            arrival.duration_since(previous_arrival).as_nanos()
                                                as f64;
                                        let rtp_delta_samples = packet
                                            .header
                                            .timestamp
                                            .wrapping_sub(previous_timestamp)
                                            as f64;
                                        let rtp_delta_ns =
                                            rtp_delta_samples * 1_000_000_000_f64 / 8_000_f64;
                                        let transit_delta = (arrival_delta - rtp_delta_ns).abs();
                                        jitter_ns += (transit_delta - jitter_ns) / 16.0;
                                    }

                                    diagnostics::record_audio_rx_packet(
                                        sequence_gap_packets,
                                        interarrival_gap,
                                        Some(std::time::Duration::from_nanos(
                                            jitter_ns.max(0.0).min(u64::MAX as f64) as u64,
                                        )),
                                    );
                                    last_sequence_number = Some(packet.header.sequence_number);
                                    last_rtp_timestamp = Some(packet.header.timestamp);
                                    last_rtp_arrival = Some(arrival);
                                }

                                if decode_buffer.len() < packet.payload.len() {
                                    let old_capacity = decode_buffer.capacity();
                                    decode_buffer.resize(packet.payload.len(), 0);
                                    let new_capacity = decode_buffer.capacity();
                                    if new_capacity > old_capacity {
                                        record_transient_allocation(
                                            "media_core.audio.rx.decode_reusable_buffer_grow",
                                            (new_capacity - old_capacity)
                                                * std::mem::size_of::<i16>(),
                                        );
                                    }
                                }

                                // Decode based on payload type into a reusable per-handler buffer.
                                let decoded_len = match packet.header.payload_type {
                                    0 => {
                                        // PCMU (μ-law)
                                        match g711_ulaw.decode_to_buffer(
                                            &packet.payload,
                                            &mut decode_buffer[..packet.payload.len()],
                                        ) {
                                            Ok(samples) => samples,
                                            Err(e) => {
                                                warn!(
                                                    "Failed to decode PCMU for dialog {}: {}",
                                                    dialog_id, e
                                                );
                                                continue;
                                            }
                                        }
                                    }
                                    8 => {
                                        // PCMA (A-law)
                                        match g711_alaw.decode_to_buffer(
                                            &packet.payload,
                                            &mut decode_buffer[..packet.payload.len()],
                                        ) {
                                            Ok(samples) => samples,
                                            Err(e) => {
                                                warn!(
                                                    "Failed to decode PCMA for dialog {}: {}",
                                                    dialog_id, e
                                                );
                                                continue;
                                            }
                                        }
                                    }
                                    #[cfg(feature = "g729")]
                                    18 => {
                                        match decode_g729_payload_to_buffer(
                                            &mut g729_decoder,
                                            &packet.payload,
                                            &mut decode_buffer,
                                        ) {
                                            Ok(samples) => samples,
                                            Err(e) => {
                                                warn!(
                                                    "Failed to decode G.729 for dialog {}: {}",
                                                    dialog_id, e
                                                );
                                                continue;
                                            }
                                        }
                                    }
                                    _ => {
                                        debug!(
                                            "Unsupported payload type {} for dialog {}",
                                            packet.header.payload_type, dialog_id
                                        );
                                        continue;
                                    }
                                };

                                let receive_enabled = media_directions
                                    .get(&dialog_id)
                                    .map(|direction| {
                                        matches!(
                                            *direction,
                                            MediaDirection::SendRecv | MediaDirection::RecvOnly
                                        )
                                    })
                                    .unwrap_or(true);
                                if !receive_enabled {
                                    debug!(
                                        "Dropping received RTP audio for dialog {} due to media direction",
                                        dialog_id
                                    );
                                    continue;
                                }
                                if collect_audio_quality {
                                    diagnostics::record_audio_rx_decoded_frame();
                                }

                                if skip_audio_frame_delivery {
                                    decoded_audio_frame_count += 1;
                                    record_transient_allocation(
                                        "media_core.audio.rx.decoded_without_frame_delivery",
                                        decoded_len * std::mem::size_of::<i16>(),
                                    );
                                    continue;
                                }

                                // Check for callback each time (it might be registered later).
                                // DashMap shard guard is held only across the synchronous
                                // try_send; we never await with a guard alive.
                                let sender = audio_frame_callbacks
                                    .get(&dialog_id)
                                    .map(|r| r.value().clone());
                                if let Some(sender) = sender {
                                    decoded_audio_frame_count += 1;
                                    let samples = decode_buffer[..decoded_len].to_vec();
                                    record_transient_allocation(
                                        "media_core.audio.rx.audio_frame.samples_vec",
                                        samples.capacity() * std::mem::size_of::<i16>(),
                                    );
                                    let audio_frame =
                                        AudioFrame::new(samples, 8000, 1, packet.header.timestamp);

                                    // Use try_send to avoid blocking the RTP event handler
                                    match sender.try_send(audio_frame) {
                                        Ok(_) => {
                                            if collect_audio_quality {
                                                let delivered_at = Instant::now();
                                                let delivery_gap =
                                                    last_delivered_at.map(|previous| {
                                                        delivered_at.duration_since(previous)
                                                    });
                                                diagnostics::record_audio_rx_delivered_frame(
                                                    delivery_gap,
                                                );
                                                last_delivered_at = Some(delivered_at);
                                            }
                                            if decoded_audio_frame_count % 10 == 0
                                                || decoded_audio_frame_count == 100
                                                || decoded_audio_frame_count == 101
                                            {
                                                info!(
                                                    "✅ Sent decoded audio frame #{} to callback for dialog {}",
                                                    decoded_audio_frame_count, dialog_id
                                                );
                                            }
                                        }
                                        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                                            warn!(
                                                "Audio frame buffer full for dialog {} at frame #{}, dropping frame",
                                                dialog_id, decoded_audio_frame_count
                                            );
                                        }
                                        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                                            error!(
                                                "❌ Audio frame channel closed for dialog {} at frame #{} - THIS IS THE 100-FRAME BUG!",
                                                dialog_id, decoded_audio_frame_count
                                            );
                                            break;
                                        }
                                    }
                                } else {
                                    if !logged_missing_audio_callback {
                                        info!(
                                            "⚠️ No audio frame callback registered yet for dialog {}",
                                            dialog_id
                                        );
                                        logged_missing_audio_callback = true;
                                    }
                                }
                            }
                            rtp_core::session::RtpSessionEvent::NewStreamDetected {
                                ssrc, ..
                            } => {
                                info!(
                                    "🎵 New RTP stream detected for dialog {}: SSRC={:08x}",
                                    dialog_id, ssrc
                                );
                            }
                            rtp_core::session::RtpSessionEvent::DtmfReceived {
                                event,
                                end_of_event,
                                duration,
                                ssrc: dtmf_ssrc,
                                ..
                            } => {
                                // Dedup is upstream now (Sprint 2.5 P4 —
                                // moved to UDP transport). We still gate
                                // on `end_of_event` because the sender
                                // emits start (E=0) + continuations
                                // (E=0) + 3× end (E=1); only the first
                                // E=1 reaches us, marking the tone
                                // boundary at which we fire the
                                // callback.
                                if !end_of_event {
                                    continue;
                                }
                                let digit = match event {
                                    0..=9 => (b'0' + event) as char,
                                    10 => '*',
                                    11 => '#',
                                    12..=15 => (b'A' + (event - 12)) as char,
                                    _ => '?',
                                };
                                // RFC 4733 telephone-event clock is
                                // 8 kHz by default; one timestamp tick
                                // is 1/8 ms, hence / 8.
                                let duration_ms = (duration as u32) / 8;
                                let notification = DtmfNotification {
                                    digit,
                                    duration_ms,
                                    ssrc: dtmf_ssrc,
                                };
                                let sender =
                                    dtmf_callbacks.get(&dialog_id).map(|r| r.value().clone());
                                if let Some(sender) = sender {
                                    match sender.try_send(notification) {
                                        Ok(_) => info!(
                                            "☎️  Delivered DTMF '{}' (duration={}ms) for dialog {}",
                                            digit, duration_ms, dialog_id
                                        ),
                                        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                                            warn!(
                                                "DTMF callback buffer full for dialog {}; dropping '{}'",
                                                dialog_id, digit
                                            );
                                        }
                                        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                                            debug!(
                                                "DTMF callback channel closed for dialog {}; '{}' dropped",
                                                dialog_id, digit
                                            );
                                        }
                                    }
                                } else {
                                    debug!(
                                        "No DTMF callback registered for dialog {}; '{}' dropped",
                                        dialog_id, digit
                                    );
                                }
                            }
                            _ => {
                                // Other events we don't need to handle
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        error!(
                            "❌ RTP event handler lagged {} events for dialog {} - PACKET LOSS!",
                            n, dialog_id
                        );
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        info!(
                            "RTP event channel closed for dialog {}, stopping handler",
                            dialog_id
                        );
                        break;
                    }
                }
            }

            info!("🛑 RTP event handler stopped for dialog: {}", dialog_id);
        });
    }
}

impl Default for MediaSessionController {
    fn default() -> Self {
        Self::new()
    }
}

// Implementation modules are in separate files
