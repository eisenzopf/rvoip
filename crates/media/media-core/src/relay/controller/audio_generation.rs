//! Audio generation and transmission functionality
//!
//! This module provides audio generation capabilities for testing and
//! audio transmission management for RTP sessions with support for multiple audio sources.

use bytes::Bytes;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::{interval, interval_at, Instant as TokioInstant, MissedTickBehavior};
use tracing::{debug, error, info, warn};

use crate::diagnostics;
use rvoip_rtp_core::{RtpSendHandle, RtpSession};

const AUDIO_TX_PHASE_MULTIPLIER: u64 = 0x9E37_79B9_7F4A_7C15;
#[cfg(any(feature = "perf-diagnostics", test))]
const DEFAULT_AUDIO_TX_PACING_TARGET_ACTIVE: u64 = 3_000;
// RFC 3551 §4.2 says RTP audio receivers should accept 0..=200 ms packets.
// Generated-audio streams retain the ordinary 20 ms packetization below
// this density and coalesce complete adjacent blocks above it. The divisor
// cap bounds PCMU packets at 180 ms: 1,440 payload bytes and 1,480 bytes with
// IPv4/UDP/RTP headers, preserving continuity and ordinary 1,500-byte MTUs.
const DEFAULT_AUDIO_TX_PACKETIZATION_TARGET_ACTIVE: u64 = 500;
const MAX_AUDIO_TX_PACKETIZATION_DIVISOR: u64 = 9;
const DEFAULT_SHARED_AUDIO_TX_BATCH_SIZE: usize = 256;
const SHARED_AUDIO_TX_TICK: Duration = Duration::from_millis(1);

static AUDIO_TX_START_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static AUDIO_TX_PACING_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static AUDIO_TX_ACTIVE_TASKS: AtomicU64 = AtomicU64::new(0);
static SHARED_AUDIO_TX_SCHEDULER: OnceLock<SharedAudioTxScheduler> = OnceLock::new();

#[derive(Debug, Clone, Copy)]
struct AudioTxPacingConfig {
    target_active: u64,
}

struct AudioTxActiveGuard {
    enabled: bool,
}

impl AudioTxActiveGuard {
    fn new(enabled: bool) -> Self {
        if enabled {
            AUDIO_TX_ACTIVE_TASKS.fetch_add(1, Ordering::Relaxed);
        }
        Self { enabled }
    }

    fn active_count(&self) -> u64 {
        if self.enabled {
            AUDIO_TX_ACTIVE_TASKS.load(Ordering::Relaxed).max(1)
        } else {
            0
        }
    }
}

impl Drop for AudioTxActiveGuard {
    fn drop(&mut self) {
        if self.enabled {
            AUDIO_TX_ACTIVE_TASKS.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

struct SharedAudioTxScheduler {
    slots: Arc<StdMutex<Vec<SharedAudioTxSlot>>>,
    started: AtomicBool,
}

struct SharedAudioTxSlot {
    entry: Arc<SharedAudioTxEntry>,
    next_due: TokioInstant,
}

struct SharedAudioTxEntry {
    rtp_session: Arc<Mutex<RtpSession>>,
    send_handle: Option<RtpSendHandle>,
    audio_generator: Arc<Mutex<AudioGenerator>>,
    timestamp: Arc<Mutex<u32>>,
    is_active: Arc<RwLock<bool>>,
    active: AtomicBool,
    interval: Duration,
    samples_per_packet: usize,
    pacing_config: Option<AudioTxPacingConfig>,
    pacing_sequence: u64,
    pacing_tick: AtomicU64,
    pacing_consecutive_skips: AtomicU64,
    packetization_pending_ticks: AtomicU64,
}

struct SharedAudioTxRegistration {
    entry: Arc<SharedAudioTxEntry>,
}

impl Drop for SharedAudioTxRegistration {
    fn drop(&mut self) {
        self.entry.active.store(false, Ordering::Release);
    }
}

impl SharedAudioTxScheduler {
    fn new() -> Self {
        Self {
            slots: Arc::new(StdMutex::new(Vec::new())),
            started: AtomicBool::new(false),
        }
    }

    fn register(
        &self,
        entry: Arc<SharedAudioTxEntry>,
        initial_delay: Duration,
    ) -> SharedAudioTxRegistration {
        self.ensure_started();
        let next_due = TokioInstant::now() + initial_delay;
        {
            let mut slots = self
                .slots
                .lock()
                .expect("shared audio TX scheduler poisoned");
            slots.push(SharedAudioTxSlot {
                entry: entry.clone(),
                next_due,
            });
        }
        SharedAudioTxRegistration { entry }
    }

    fn ensure_started(&self) {
        if self.started.swap(true, Ordering::AcqRel) {
            return;
        }

        let slots = self.slots.clone();
        let batch_size = shared_audio_tx_batch_size_from_env();
        super::spawn_memory_tracked("media_core.shared_audio_tx_scheduler", async move {
            run_shared_audio_tx_scheduler(slots, batch_size).await;
        });
    }
}

async fn run_shared_audio_tx_scheduler(
    slots: Arc<StdMutex<Vec<SharedAudioTxSlot>>>,
    batch_size: usize,
) {
    let mut tick = interval(SHARED_AUDIO_TX_TICK);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut due_entries: Vec<Arc<SharedAudioTxEntry>> = Vec::new();

    loop {
        tick.tick().await;
        due_entries.clear();

        let now = TokioInstant::now();
        let active_count = {
            let mut slots = slots.lock().expect("shared audio TX scheduler poisoned");
            slots.retain(|slot| slot.entry.active.load(Ordering::Acquire));
            let active_count = slots.len() as u64;

            for slot in slots.iter_mut() {
                if slot.next_due <= now {
                    due_entries.push(slot.entry.clone());
                    while slot.next_due <= now {
                        slot.next_due += slot.entry.interval;
                    }
                }
            }

            active_count
        };

        if due_entries.is_empty() {
            continue;
        }

        let mut sent_count = 0_u64;
        let mut skip_count = 0_u64;
        let mut fail_count = 0_u64;
        let mut pacing_evaluated_count = 0_u64;
        let mut pacing_consecutive_skip_max = 0_u64;
        let mut pacing_divisor_max = 1_u64;

        for (index, entry) in due_entries.iter().enumerate() {
            match entry.send_due(active_count).await {
                SharedAudioTxOutcome::Sent { pacing_evaluated } => {
                    sent_count = sent_count.saturating_add(1);
                    if pacing_evaluated {
                        pacing_evaluated_count = pacing_evaluated_count.saturating_add(1);
                    }
                }
                SharedAudioTxOutcome::Skipped {
                    divisor,
                    consecutive_skips,
                } => {
                    skip_count = skip_count.saturating_add(1);
                    pacing_evaluated_count = pacing_evaluated_count.saturating_add(1);
                    pacing_divisor_max = pacing_divisor_max.max(divisor);
                    pacing_consecutive_skip_max =
                        pacing_consecutive_skip_max.max(consecutive_skips);
                }
                SharedAudioTxOutcome::Deferred => {}
                SharedAudioTxOutcome::Inactive => {}
                SharedAudioTxOutcome::Failed => {
                    fail_count = fail_count.saturating_add(1);
                }
            }

            if (index + 1) % batch_size == 0 {
                tokio::task::yield_now().await;
            }
        }

        diagnostics::record_audio_tx_shared_batch(
            due_entries.len() as u64,
            sent_count,
            skip_count,
            fail_count,
            active_count,
        );
        if pacing_evaluated_count > 0 {
            diagnostics::record_audio_tx_pacing_batch(
                pacing_evaluated_count,
                skip_count,
                active_count,
                pacing_divisor_max,
                pacing_consecutive_skip_max,
            );
        }
    }
}

enum SharedAudioTxOutcome {
    Sent {
        pacing_evaluated: bool,
    },
    Skipped {
        divisor: u64,
        consecutive_skips: u64,
    },
    Deferred,
    Inactive,
    Failed,
}

impl SharedAudioTxEntry {
    async fn send_due(&self, active_count: u64) -> SharedAudioTxOutcome {
        if !self.active.load(Ordering::Acquire) {
            return SharedAudioTxOutcome::Inactive;
        }

        let effective_samples_per_packet = if let Some(pacing) = self.pacing_config {
            let divisor = pacing_divisor(active_count.max(1), pacing.target_active);
            let pacing_tick = self.pacing_tick.fetch_add(1, Ordering::Relaxed);
            let should_skip =
                divisor > 1 && pacing_tick.wrapping_add(self.pacing_sequence) % divisor != 0;
            if should_skip {
                let consecutive_skips = self
                    .pacing_consecutive_skips
                    .fetch_add(1, Ordering::Relaxed)
                    .saturating_add(1);
                advance_rtp_timestamp(&self.timestamp, self.samples_per_packet).await;
                return SharedAudioTxOutcome::Skipped {
                    divisor,
                    consecutive_skips,
                };
            }
            self.pacing_consecutive_skips.store(0, Ordering::Relaxed);
            self.samples_per_packet
        } else {
            let pending_ticks = self
                .packetization_pending_ticks
                .fetch_add(1, Ordering::Relaxed)
                .saturating_add(1);
            let divisor = adaptive_packetization_divisor(active_count.max(1));
            if pending_ticks < divisor {
                return SharedAudioTxOutcome::Deferred;
            }
            self.packetization_pending_ticks.store(0, Ordering::Relaxed);
            self.samples_per_packet
                .saturating_mul(pending_ticks as usize)
        };

        let audio_samples = {
            let mut generator = self.audio_generator.lock().await;
            generator.generate_pcmu_payload(effective_samples_per_packet)
        };

        if matches!(audio_samples.as_ref(), [0x7F, ..] if audio_samples.iter().all(|&x| x == 0x7F))
        {
            return SharedAudioTxOutcome::Inactive;
        }

        if !self.active.load(Ordering::Acquire) {
            return SharedAudioTxOutcome::Inactive;
        }

        let current_timestamp =
            advance_rtp_timestamp(&self.timestamp, effective_samples_per_packet).await;

        if !self.active.load(Ordering::Acquire) {
            return SharedAudioTxOutcome::Inactive;
        }

        let send_result = if let Some(handle) = &self.send_handle {
            handle
                .send_packet(current_timestamp, audio_samples, false)
                .await
        } else {
            let session = self.rtp_session.lock().await;
            session
                .send_packet(current_timestamp, audio_samples, false)
                .await
        };

        match send_result {
            Ok(()) => SharedAudioTxOutcome::Sent {
                pacing_evaluated: self.pacing_config.is_some(),
            },
            Err(e) => {
                if !self.active.load(Ordering::Acquire) || !*self.is_active.read().await {
                    debug!("Shared RTP audio send skipped after stop: {}", e);
                    return SharedAudioTxOutcome::Inactive;
                }
                error!("Failed to send shared RTP audio packet: {}", e);
                self.active.store(false, Ordering::Release);
                *self.is_active.write().await = false;
                SharedAudioTxOutcome::Failed
            }
        }
    }
}

fn shared_audio_tx_scheduler() -> &'static SharedAudioTxScheduler {
    SHARED_AUDIO_TX_SCHEDULER.get_or_init(SharedAudioTxScheduler::new)
}

#[cfg(feature = "memory-diagnostics")]
fn record_transient_allocation(kind: &'static str, bytes: usize) {
    rvoip_infra_common::memory_diagnostics::record_transient_allocation(kind, bytes as u64);
}

#[cfg(not(feature = "memory-diagnostics"))]
fn record_transient_allocation(_: &'static str, _: usize) {}

/// Audio source types supported by the audio transmitter
#[derive(Debug, Clone)]
pub enum AudioSource {
    /// Generate a sine wave tone
    Tone { frequency: f64, amplitude: f64 },
    /// Use custom audio samples (repeating)
    CustomSamples { samples: Vec<u8>, repeat: bool },
    /// Pass-through mode (no audio generation)
    PassThrough,
}

/// Audio generator for creating test tones and audio streams
pub struct AudioGenerator {
    /// Sample rate (Hz)
    sample_rate: u32,
    /// Current phase for sine wave generation
    phase: f64,
    /// Audio source configuration
    source: AudioSource,
    /// Current position in custom samples (if using custom samples)
    sample_position: usize,
    /// One exact encoded period for integral-frequency tones.
    tone_waveform: Option<Vec<u8>>,
    /// Immutable packet payloads keyed by (packet length, waveform offset).
    tone_payload_cache: HashMap<(usize, usize), Bytes>,
}

impl AudioGenerator {
    /// Create a new audio generator with tone generation
    pub fn new(sample_rate: u32, frequency: f64, amplitude: f64) -> Self {
        let source = AudioSource::Tone {
            frequency,
            amplitude,
        };
        Self {
            sample_rate,
            phase: 0.0,
            tone_waveform: encoded_integral_tone_period(sample_rate, &source),
            source,
            sample_position: 0,
            tone_payload_cache: HashMap::new(),
        }
    }

    /// Create a new audio generator with custom audio source
    pub fn new_with_source(sample_rate: u32, source: AudioSource) -> Self {
        Self {
            sample_rate,
            phase: 0.0,
            tone_waveform: encoded_integral_tone_period(sample_rate, &source),
            source,
            sample_position: 0,
            tone_payload_cache: HashMap::new(),
        }
    }

    /// Generate one immutable PCMU payload for the RTP send path.
    ///
    /// Integral-frequency tones have an exact sample period at an integral
    /// sample rate. Cache each packet phase once and share its backing storage
    /// thereafter. Other sources retain the ordinary generator semantics.
    pub fn generate_pcmu_payload(&mut self, num_samples: usize) -> Bytes {
        if let Some(waveform) = self.tone_waveform.as_ref() {
            if waveform.is_empty() || num_samples == 0 {
                return Bytes::new();
            }

            let start = self.sample_position % waveform.len();
            let cache_key = (num_samples, start);
            let payload = if let Some(payload) = self.tone_payload_cache.get(&cache_key) {
                payload.clone()
            } else {
                let mut samples = Vec::with_capacity(num_samples);
                let mut remaining = num_samples;
                let mut offset = start;
                while remaining > 0 {
                    let copy_len = remaining.min(waveform.len() - offset);
                    samples.extend_from_slice(&waveform[offset..offset + copy_len]);
                    remaining -= copy_len;
                    offset = 0;
                }
                record_transient_allocation("media_core.audio.tx.payload_vec", samples.capacity());
                let payload = Bytes::from(samples);
                self.tone_payload_cache.insert(cache_key, payload.clone());
                payload
            };
            self.sample_position = start.wrapping_add(num_samples) % waveform.len();
            return payload;
        }

        let samples = self.generate_pcmu_samples_uncached(num_samples);
        record_transient_allocation("media_core.audio.tx.payload_vec", samples.capacity());
        Bytes::from(samples)
    }

    /// Generate audio samples for PCMU (G.711 μ-law) encoding
    pub fn generate_pcmu_samples(&mut self, num_samples: usize) -> Vec<u8> {
        self.generate_pcmu_payload(num_samples).to_vec()
    }

    fn generate_pcmu_samples_uncached(&mut self, num_samples: usize) -> Vec<u8> {
        match self.source.clone() {
            AudioSource::Tone {
                frequency,
                amplitude,
            } => self.generate_tone_samples(num_samples, frequency, amplitude),
            AudioSource::CustomSamples { samples, repeat } => {
                self.generate_custom_samples(num_samples, &samples, repeat)
            }
            AudioSource::PassThrough => {
                // Return silence for pass-through mode
                vec![0x7F; num_samples] // μ-law silence
            }
        }
    }

    /// Generate tone samples
    fn generate_tone_samples(
        &mut self,
        num_samples: usize,
        frequency: f64,
        amplitude: f64,
    ) -> Vec<u8> {
        let mut samples = Vec::with_capacity(num_samples);
        let phase_increment = 2.0 * std::f64::consts::PI * frequency / self.sample_rate as f64;

        for _ in 0..num_samples {
            // Generate sine wave sample
            let sample = (self.phase.sin() * amplitude * 32767.0) as i16;

            // Convert to μ-law
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

    /// Generate samples from custom audio data
    fn generate_custom_samples(
        &mut self,
        num_samples: usize,
        samples: &[u8],
        repeat: bool,
    ) -> Vec<u8> {
        let mut result = Vec::with_capacity(num_samples);

        if samples.is_empty() {
            // Return silence if no custom samples
            return vec![0x7F; num_samples]; // μ-law silence
        }

        for _ in 0..num_samples {
            if self.sample_position >= samples.len() {
                if repeat {
                    self.sample_position = 0;
                } else {
                    // End of samples, return silence
                    result.push(0x7F); // μ-law silence
                    continue;
                }
            }

            result.push(samples[self.sample_position]);
            self.sample_position += 1;
        }

        result
    }

    /// Convert linear PCM to μ-law (G.711)
    pub fn linear_to_ulaw(pcm: i16) -> u8 {
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

    /// Convert PCM samples to μ-law
    pub fn pcm_to_mulaw(pcm_samples: &[i16]) -> Vec<u8> {
        pcm_samples
            .iter()
            .map(|&sample| Self::linear_to_ulaw(sample))
            .collect()
    }

    /// Update the audio source
    pub fn set_source(&mut self, source: AudioSource) {
        self.tone_waveform = encoded_integral_tone_period(self.sample_rate, &source);
        self.tone_payload_cache.clear();
        self.source = source;
        self.phase = 0.0;
        self.sample_position = 0; // Reset position for custom samples
    }
}

fn encoded_integral_tone_period(sample_rate: u32, source: &AudioSource) -> Option<Vec<u8>> {
    let AudioSource::Tone {
        frequency,
        amplitude,
    } = source
    else {
        return None;
    };
    if sample_rate == 0
        || !frequency.is_finite()
        || !amplitude.is_finite()
        || *frequency < 0.0
        || frequency.fract() != 0.0
        || *frequency > u32::MAX as f64
    {
        return None;
    }

    let frequency_hz = *frequency as u32;
    let period_samples = if frequency_hz == 0 {
        1
    } else {
        sample_rate / greatest_common_divisor(sample_rate, frequency_hz)
    } as usize;
    let phase_increment = 2.0 * std::f64::consts::PI * *frequency / sample_rate as f64;
    let mut waveform = Vec::with_capacity(period_samples);
    for sample_index in 0..period_samples {
        let phase = sample_index as f64 * phase_increment;
        let sample = (phase.sin() * *amplitude * 32767.0) as i16;
        waveform.push(AudioGenerator::linear_to_ulaw(sample));
    }
    Some(waveform)
}

fn greatest_common_divisor(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left.max(1)
}

/// Audio transmission configuration
#[derive(Debug, Clone)]
pub struct AudioTransmitterConfig {
    /// Audio source type
    pub source: AudioSource,
    /// Transmission interval (default: 20ms)
    pub interval: Duration,
    /// Samples per packet (default: 160 for 20ms at 8kHz)
    pub samples_per_packet: usize,
    /// Sample rate (default: 8000 Hz)
    pub sample_rate: u32,
}

impl Default for AudioTransmitterConfig {
    fn default() -> Self {
        Self {
            source: AudioSource::PassThrough, // Default to pass-through mode
            interval: Duration::from_millis(20),
            samples_per_packet: 160,
            sample_rate: 8000,
        }
    }
}

/// Audio transmission task for RTP sessions.
///
/// Phase C16: the per-frame send path no longer locks the session's
/// outer `Mutex<RtpSession>`. At construction we snapshot a
/// lock-free [`RtpSendHandle`] (shares the scheduler's sequence
/// atomic), and the spawned TX loop uses that handle directly. This
/// removes the per-20ms `session.lock().await` on the dominant audio
/// path — relevant for multi-call setups where the RTCP scheduler,
/// bridge forwarders, or controller methods contend for the same
/// per-session mutex.
pub struct AudioTransmitter {
    /// RTP session for transmission. Kept for fallback construction
    /// of a fresh send handle if the cached one is invalidated;
    /// **not** locked on the TX hot path.
    rtp_session: Arc<Mutex<RtpSession>>,
    /// Lock-free send path snapshot. `None` only if the session had
    /// no scheduler when we built the transmitter (current sessions
    /// always do — see `RtpSession::new`).
    send_handle: Option<RtpSendHandle>,
    /// Audio generator
    audio_generator: Arc<Mutex<AudioGenerator>>,
    /// Transmission configuration
    config: AudioTransmitterConfig,
    /// Current RTP timestamp
    timestamp: Arc<Mutex<u32>>,
    /// Whether transmission is active
    is_active: Arc<RwLock<bool>>,
    /// Background transmission task.
    tx_task: Option<JoinHandle<()>>,
    /// Registration in the optional shared audio TX scheduler.
    shared_tx_registration: Option<SharedAudioTxRegistration>,
}

impl AudioTransmitter {
    /// Create a new audio transmitter with default configuration (pass-through mode)
    pub fn new(rtp_session: Arc<Mutex<RtpSession>>) -> Self {
        let config = AudioTransmitterConfig::default();
        Self::new_with_config(rtp_session, config)
    }

    /// Create a new audio transmitter with tone generation (for backward compatibility)
    pub fn new_with_tone(rtp_session: Arc<Mutex<RtpSession>>) -> Self {
        let config = AudioTransmitterConfig {
            source: AudioSource::Tone {
                frequency: 440.0,
                amplitude: 0.5,
            },
            ..Default::default()
        };
        Self::new_with_config(rtp_session, config)
    }

    /// Create a new audio transmitter with custom configuration
    pub fn new_with_config(
        rtp_session: Arc<Mutex<RtpSession>>,
        config: AudioTransmitterConfig,
    ) -> Self {
        let audio_generator =
            AudioGenerator::new_with_source(config.sample_rate, config.source.clone());

        // Build the lock-free send handle once. This briefly locks the
        // session at construction time only; the TX hot path never
        // re-locks.
        let send_handle = rtp_session.try_lock().ok().and_then(|s| s.send_handle());
        if send_handle.is_none() {
            warn!(
                "AudioTransmitter: could not snapshot send_handle at construction \
                 (session may be missing scheduler or already locked). Will fall back \
                 to per-frame session lock until refreshed."
            );
        }

        Self {
            rtp_session,
            send_handle,
            audio_generator: Arc::new(Mutex::new(audio_generator)),
            config,
            timestamp: Arc::new(Mutex::new(0)),
            is_active: Arc::new(RwLock::new(false)),
            tx_task: None,
            shared_tx_registration: None,
        }
    }

    /// Start audio transmission
    pub async fn start(&mut self) {
        if self.tx_task.is_some() || self.shared_tx_registration.is_some() {
            debug!("AudioTransmitter: transmission task already running");
            return;
        }

        if matches!(self.config.source, AudioSource::PassThrough) {
            *self.is_active.write().await = false;
            debug!("AudioTransmitter: pass-through source has no background TX task");
            return;
        }

        // If we never managed to snapshot a send handle (e.g. the
        // session was locked at construction), try again now — the
        // construction-time lock contention is gone by definition.
        if self.send_handle.is_none() {
            let session = self.rtp_session.lock().await;
            self.send_handle = session.send_handle();
        }

        *self.is_active.write().await = true;

        let source_desc = match &self.config.source {
            AudioSource::Tone {
                frequency,
                amplitude,
            } => {
                format!("{}Hz tone (amplitude: {})", frequency, amplitude)
            }
            AudioSource::CustomSamples { samples, repeat } => {
                format!(
                    "custom audio ({} samples, repeat: {})",
                    samples.len(),
                    repeat
                )
            }
            AudioSource::PassThrough => "pass-through mode".to_string(),
        };

        info!(
            "🎵 Started audio transmission ({}, {}ms packets)",
            source_desc,
            self.config.interval.as_millis()
        );

        let rtp_session = self.rtp_session.clone();
        let send_handle = self.send_handle.clone();
        let is_active = self.is_active.clone();
        let audio_generator = self.audio_generator.clone();
        let timestamp = self.timestamp.clone();
        let initial_delay = next_audio_tx_start_delay(self.config.interval);
        diagnostics::record_audio_tx_task_started(initial_delay);
        let pacing_config = audio_tx_pacing_config_from_env();

        if shared_audio_tx_scheduler_enabled() {
            let pacing_sequence = pacing_config
                .map(|_| AUDIO_TX_PACING_SEQUENCE.fetch_add(1, Ordering::Relaxed))
                .unwrap_or(0);
            let entry = Arc::new(SharedAudioTxEntry {
                rtp_session,
                send_handle,
                audio_generator,
                timestamp,
                is_active,
                active: AtomicBool::new(true),
                interval: self.config.interval,
                samples_per_packet: self.config.samples_per_packet,
                pacing_config,
                pacing_sequence,
                pacing_tick: AtomicU64::new(0),
                pacing_consecutive_skips: AtomicU64::new(0),
                packetization_pending_ticks: AtomicU64::new(0),
            });
            self.shared_tx_registration =
                Some(shared_audio_tx_scheduler().register(entry, initial_delay));
            info!("🎵 Started shared audio transmission");
            return;
        }

        let collect_diagnostics = diagnostics::enabled();
        let mut interval_timer =
            interval_at(TokioInstant::now() + initial_delay, self.config.interval);
        interval_timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let samples_per_packet = self.config.samples_per_packet;
        let pacing_sequence = pacing_config
            .map(|_| AUDIO_TX_PACING_SEQUENCE.fetch_add(1, Ordering::Relaxed))
            .unwrap_or(0);

        self.tx_task = Some(super::spawn_memory_tracked(
            "media_core.audio_transmitter_task",
            async move {
                let active_guard = AudioTxActiveGuard::new(true);
                let mut last_tick_at: Option<TokioInstant> = None;
                let mut tick_gap_count = 0_u64;
                let mut tick_gap_total = Duration::ZERO;
                let mut tick_gap_max = Duration::ZERO;
                let mut send_count = 0_u64;
                let mut send_failures = 0_u64;
                let mut send_total = Duration::ZERO;
                let mut send_max = Duration::ZERO;
                let mut pacing_tick = 0_u64;
                let mut pacing_evaluated_count = 0_u64;
                let mut pacing_skip_count = 0_u64;
                let mut pacing_active_max = 0_u64;
                let mut pacing_divisor_max = 1_u64;
                let mut pacing_consecutive_skip_count = 0_u64;
                let mut pacing_consecutive_skip_max = 0_u64;
                let mut packetization_pending_ticks = 0_u64;

                while *is_active.read().await {
                    interval_timer.tick().await;
                    if collect_diagnostics {
                        let tick_at = TokioInstant::now();
                        if let Some(previous) = last_tick_at {
                            let gap = tick_at.duration_since(previous);
                            tick_gap_count = tick_gap_count.saturating_add(1);
                            tick_gap_total = tick_gap_total.saturating_add(gap);
                            tick_gap_max = tick_gap_max.max(gap);
                        }
                        last_tick_at = Some(tick_at);
                    }

                    let effective_samples_per_packet = if let Some(pacing) = pacing_config {
                        pacing_evaluated_count = pacing_evaluated_count.saturating_add(1);
                        let active_count = active_guard.active_count();
                        pacing_active_max = pacing_active_max.max(active_count);
                        let divisor = pacing_divisor(active_count, pacing.target_active);
                        pacing_divisor_max = pacing_divisor_max.max(divisor);
                        let should_skip =
                            divisor > 1 && pacing_tick.wrapping_add(pacing_sequence) % divisor != 0;
                        pacing_tick = pacing_tick.wrapping_add(1);

                        if should_skip {
                            pacing_skip_count = pacing_skip_count.saturating_add(1);
                            pacing_consecutive_skip_count =
                                pacing_consecutive_skip_count.saturating_add(1);
                            pacing_consecutive_skip_max =
                                pacing_consecutive_skip_max.max(pacing_consecutive_skip_count);
                            advance_rtp_timestamp(&timestamp, samples_per_packet).await;
                            continue;
                        }
                        pacing_consecutive_skip_count = 0;
                        samples_per_packet
                    } else {
                        packetization_pending_ticks = packetization_pending_ticks.saturating_add(1);
                        let divisor = adaptive_packetization_divisor(active_guard.active_count());
                        if packetization_pending_ticks < divisor {
                            continue;
                        }
                        let effective_samples =
                            samples_per_packet.saturating_mul(packetization_pending_ticks as usize);
                        packetization_pending_ticks = 0;
                        effective_samples
                    };

                    // Generate audio samples
                    let audio_samples = {
                        let mut generator = audio_generator.lock().await;
                        generator.generate_pcmu_payload(effective_samples_per_packet)
                    };

                    // Send RTP packet (only if not in pass-through mode)
                    if !matches!(audio_samples.as_ref(), [0x7F, ..] if audio_samples.iter().all(|&x| x == 0x7F))
                    {
                        let current_timestamp = {
                            advance_rtp_timestamp(&timestamp, effective_samples_per_packet).await
                        };

                        // Fast path: send through the lock-free handle —
                        // no `session.lock().await` per frame.
                        let send_started = collect_diagnostics.then(Instant::now);
                        let send_result = if let Some(handle) = &send_handle {
                            handle
                                .send_packet(current_timestamp, audio_samples, false)
                                .await
                        } else {
                            // Fallback: session lock per frame. Only reached
                            // if the session was missing a scheduler when
                            // we tried to build the handle.
                            let session = rtp_session.lock().await;
                            session
                                .send_packet(current_timestamp, audio_samples, false)
                                .await
                        };
                        if let Some(send_started) = send_started {
                            let send_elapsed = send_started.elapsed();
                            send_count = send_count.saturating_add(1);
                            if send_result.is_err() {
                                send_failures = send_failures.saturating_add(1);
                            }
                            send_total = send_total.saturating_add(send_elapsed);
                            send_max = send_max.max(send_elapsed);
                        }

                        if let Err(e) = send_result {
                            error!("Failed to send RTP audio packet: {}", e);
                            *is_active.write().await = false;
                            break;
                        } else {
                            debug!(
                                "📡 Sent RTP audio packet (timestamp: {}, {} samples)",
                                current_timestamp, effective_samples_per_packet
                            );
                        }
                    }
                }

                if collect_diagnostics {
                    diagnostics::record_audio_tx_tick_gap_batch(
                        tick_gap_count,
                        tick_gap_total,
                        tick_gap_max,
                    );
                    diagnostics::record_audio_tx_send_batch(
                        send_count,
                        send_failures,
                        send_total,
                        send_max,
                    );
                }
                if pacing_config.is_some() {
                    diagnostics::record_audio_tx_pacing_batch(
                        pacing_evaluated_count,
                        pacing_skip_count,
                        pacing_active_max,
                        pacing_divisor_max,
                        pacing_consecutive_skip_max,
                    );
                }

                info!("🛑 Stopped audio transmission");
            },
        ));
    }

    /// Stop audio transmission
    pub async fn stop(mut self) {
        *self.is_active.write().await = false;
        self.shared_tx_registration.take();
        if let Some(task) = self.tx_task.take() {
            let mut task = task;
            tokio::select! {
                _ = &mut task => {}
                _ = tokio::time::sleep(self.config.interval.saturating_mul(2)) => {
                    task.abort();
                    let _ = task.await;
                }
            }
        }
        info!("🛑 Stopping audio transmission");
    }

    /// Check if transmission is active
    pub async fn is_active(&self) -> bool {
        *self.is_active.read().await
    }

    /// Update the audio source during transmission
    pub async fn set_audio_source(&self, source: AudioSource) {
        let mut generator = self.audio_generator.lock().await;
        generator.set_source(source);
        info!("🔄 Updated audio source");
    }

    /// Set custom audio samples for transmission
    pub async fn set_custom_audio(&self, samples: Vec<u8>, repeat: bool) {
        let source = AudioSource::CustomSamples { samples, repeat };
        self.set_audio_source(source).await;
    }

    /// Set tone generation parameters
    pub async fn set_tone(&self, frequency: f64, amplitude: f64) {
        let source = AudioSource::Tone {
            frequency,
            amplitude,
        };
        self.set_audio_source(source).await;
    }

    /// Enable pass-through mode (no audio generation)
    pub async fn set_pass_through(&self) {
        let source = AudioSource::PassThrough;
        self.set_audio_source(source).await;
    }
}

fn next_audio_tx_start_delay(interval: Duration) -> Duration {
    let interval_nanos = interval.as_nanos().min(u128::from(u64::MAX)) as u64;
    if interval_nanos == 0 {
        return Duration::ZERO;
    }

    let sequence = AUDIO_TX_START_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let offset = sequence.wrapping_mul(AUDIO_TX_PHASE_MULTIPLIER) % interval_nanos;
    Duration::from_nanos(offset)
}

async fn advance_rtp_timestamp(timestamp: &Arc<Mutex<u32>>, samples_per_packet: usize) -> u32 {
    let mut ts = timestamp.lock().await;
    let current = *ts;
    *ts = ts.wrapping_add(samples_per_packet as u32);
    current
}

#[cfg(any(feature = "perf-diagnostics", test))]
fn audio_tx_pacing_config_from_env() -> Option<AudioTxPacingConfig> {
    if !env_flag_enabled("RVOIP_MEDIA_AUDIO_TX_PACING") {
        return None;
    }

    let target_active = std::env::var("RVOIP_MEDIA_AUDIO_TX_PACING_TARGET_ACTIVE")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_AUDIO_TX_PACING_TARGET_ACTIVE);

    Some(AudioTxPacingConfig { target_active })
}

#[cfg(not(any(feature = "perf-diagnostics", test)))]
fn audio_tx_pacing_config_from_env() -> Option<AudioTxPacingConfig> {
    None
}

#[cfg(any(feature = "perf-diagnostics", test))]
fn shared_audio_tx_scheduler_enabled() -> bool {
    env_flag_enabled("RVOIP_MEDIA_AUDIO_TX_SHARED_SCHEDULER")
}

#[cfg(not(any(feature = "perf-diagnostics", test)))]
fn shared_audio_tx_scheduler_enabled() -> bool {
    false
}

#[cfg(any(feature = "perf-diagnostics", test))]
fn shared_audio_tx_batch_size_from_env() -> usize {
    std::env::var("RVOIP_MEDIA_AUDIO_TX_SHARED_BATCH_SIZE")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_SHARED_AUDIO_TX_BATCH_SIZE)
}

#[cfg(not(any(feature = "perf-diagnostics", test)))]
fn shared_audio_tx_batch_size_from_env() -> usize {
    DEFAULT_SHARED_AUDIO_TX_BATCH_SIZE
}

#[cfg(any(feature = "perf-diagnostics", test))]
fn env_flag_enabled(name: &str) -> bool {
    std::env::var(name)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn pacing_divisor(active_count: u64, target_active: u64) -> u64 {
    if target_active == 0 || active_count <= target_active {
        1
    } else {
        active_count
            .saturating_add(target_active - 1)
            .saturating_div(target_active)
            .max(1)
    }
}

fn adaptive_packetization_divisor(active_count: u64) -> u64 {
    pacing_divisor(active_count, DEFAULT_AUDIO_TX_PACKETIZATION_TARGET_ACTIVE)
        .min(MAX_AUDIO_TX_PACKETIZATION_DIVISOR)
}

#[cfg(test)]
fn audio_tx_start_delay_for_sequence(sequence: u64, interval: Duration) -> Duration {
    let interval_nanos = interval.as_nanos().min(u128::from(u64::MAX)) as u64;
    if interval_nanos == 0 {
        return Duration::ZERO;
    }
    Duration::from_nanos(sequence.wrapping_mul(AUDIO_TX_PHASE_MULTIPLIER) % interval_nanos)
}

impl Drop for AudioTransmitter {
    fn drop(&mut self) {
        self.shared_tx_registration.take();
        if let Some(task) = self.tx_task.take() {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        adaptive_packetization_divisor, audio_tx_start_delay_for_sequence, pacing_divisor,
        AudioGenerator, AudioSource,
    };
    use std::time::Duration;

    #[test]
    fn generate_pcmu_samples_returns_requested_length() {
        let mut generator = AudioGenerator::new(8000, 440.0, 0.5);
        let output = generator.generate_pcmu_samples(160);

        assert_eq!(output.len(), 160);
    }

    #[test]
    fn integral_tone_payload_cache_preserves_continuous_wire_samples() {
        let mut cached = AudioGenerator::new(8000, 440.0, 0.5);
        let mut baseline = AudioGenerator::new(8000, 440.0, 0.5);
        baseline.tone_waveform = None;

        for _ in 0..10 {
            assert_eq!(
                cached.generate_pcmu_payload(160).as_ref(),
                baseline.generate_pcmu_samples_uncached(160).as_slice()
            );
        }
    }

    #[test]
    fn integral_tone_reuses_immutable_packet_payloads() {
        let mut generator = AudioGenerator::new(8000, 440.0, 0.5);
        let first = generator.generate_pcmu_payload(160);
        for _ in 0..4 {
            generator.generate_pcmu_payload(160);
        }
        let repeated = generator.generate_pcmu_payload(160);

        assert_eq!(first, repeated);
        assert_eq!(first.as_ptr(), repeated.as_ptr());
        assert_eq!(generator.tone_payload_cache.len(), 5);
    }

    #[test]
    fn generate_custom_samples_repeats() {
        let mut generator = AudioGenerator::new_with_source(
            8000,
            AudioSource::CustomSamples {
                samples: vec![1, 2, 3],
                repeat: true,
            },
        );
        let output = generator.generate_pcmu_samples(5);

        assert_eq!(output, vec![1, 2, 3, 1, 2]);
    }

    #[test]
    fn audio_tx_start_delay_spreads_sequential_transmitters() {
        let interval = Duration::from_millis(20);
        let first = audio_tx_start_delay_for_sequence(0, interval);
        let second = audio_tx_start_delay_for_sequence(1, interval);
        let third = audio_tx_start_delay_for_sequence(2, interval);

        assert_eq!(first, Duration::ZERO);
        assert!(second < interval);
        assert!(third < interval);
        assert_ne!(second, first);
        assert_ne!(third, second);
    }

    #[test]
    fn pacing_divisor_scales_after_target_active() {
        assert_eq!(pacing_divisor(2_999, 3_000), 1);
        assert_eq!(pacing_divisor(3_000, 3_000), 1);
        assert_eq!(pacing_divisor(3_001, 3_000), 2);
        assert_eq!(pacing_divisor(6_000, 3_000), 2);
        assert_eq!(pacing_divisor(6_001, 3_000), 3);
        assert_eq!(pacing_divisor(6_001, 0), 1);
    }

    #[test]
    fn adaptive_packetization_only_coalesces_above_target() {
        assert_eq!(adaptive_packetization_divisor(499), 1);
        assert_eq!(adaptive_packetization_divisor(500), 1);
        assert_eq!(adaptive_packetization_divisor(501), 2);
        assert_eq!(adaptive_packetization_divisor(2_001), 5);
        assert_eq!(adaptive_packetization_divisor(4_800), 9);
        assert_eq!(adaptive_packetization_divisor(u64::MAX), 9);
    }

    #[test]
    fn coalesced_tone_packets_preserve_continuous_audio() {
        let mut ordinary = AudioGenerator::new(8000, 440.0, 0.5);
        let mut coalesced = AudioGenerator::new(8000, 440.0, 0.5);
        let ordinary_audio = [
            ordinary.generate_pcmu_payload(160),
            ordinary.generate_pcmu_payload(160),
            ordinary.generate_pcmu_payload(160),
            ordinary.generate_pcmu_payload(160),
        ]
        .concat();
        let coalesced_audio = [
            coalesced.generate_pcmu_payload(320),
            coalesced.generate_pcmu_payload(320),
        ]
        .concat();

        assert_eq!(coalesced_audio, ordinary_audio);
        let sequence_numbers = [41_u16, 42_u16];
        let timestamps = [10_000_u32, 10_320_u32];
        assert_eq!(sequence_numbers[1], sequence_numbers[0].wrapping_add(1));
        assert_eq!(timestamps[1], timestamps[0].wrapping_add(320));
    }
}
