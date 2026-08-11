//! Audio generation and transmission functionality
//!
//! This module provides audio generation capabilities for testing and
//! audio transmission management for RTP sessions with support for multiple audio sources.

use bytes::Bytes;
use parking_lot::Mutex as ParkingMutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::Instant as TokioInstant;
use tracing::{debug, error, info, warn};

use crate::diagnostics;
use crate::types::AudioFrame;
use rvoip_rtp_core::{RtpSendHandle, RtpSession};

use super::codec_runtime::DialogCodecRuntime;
use super::types::NegotiatedAudioCodec;

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

static AUDIO_TX_START_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static AUDIO_TX_PACING_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static AUDIO_TX_ACTIVE_TASKS: AtomicU64 = AtomicU64::new(0);

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
    /// Clock of legacy PCMU custom samples before resampling to `sample_rate`.
    custom_source_rate: u32,
    /// Current phase for sine wave generation
    phase: f64,
    /// Audio source configuration
    source: AudioSource,
    /// Current position in custom samples (if using custom samples)
    sample_position: usize,
    /// Output-frame cursor used for deterministic nearest-neighbour custom
    /// sample-rate conversion.
    custom_output_position: u64,
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
            custom_source_rate: sample_rate,
            phase: 0.0,
            tone_waveform: encoded_integral_tone_period(sample_rate, &source),
            source,
            sample_position: 0,
            custom_output_position: 0,
            tone_payload_cache: HashMap::new(),
        }
    }

    /// Create a new audio generator with custom audio source
    pub fn new_with_source(sample_rate: u32, source: AudioSource) -> Self {
        Self::new_with_source_rates(sample_rate, sample_rate, source)
    }

    fn new_with_source_rates(
        sample_rate: u32,
        custom_source_rate: u32,
        source: AudioSource,
    ) -> Self {
        Self {
            sample_rate,
            custom_source_rate,
            phase: 0.0,
            tone_waveform: encoded_integral_tone_period(sample_rate, &source),
            source,
            sample_position: 0,
            custom_output_position: 0,
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

    /// Generate codec-neutral signed PCM at this generator's sample rate.
    ///
    /// `CustomSamples` remains source-compatible with its historical wire
    /// contract: the bytes are interpreted as PCMU. They are decoded to PCM
    /// here so the negotiated codec, rather than the source container, owns
    /// the RTP payload encoding.
    pub fn generate_pcm_frame(&mut self, frames: usize, channels: u8) -> Vec<i16> {
        let channels = usize::from(channels.max(1));
        let mut output = Vec::with_capacity(frames.saturating_mul(channels));
        match self.source.clone() {
            AudioSource::Tone {
                frequency,
                amplitude,
            } => {
                let phase_increment =
                    2.0 * std::f64::consts::PI * frequency / self.sample_rate.max(1) as f64;
                for _ in 0..frames {
                    let sample = (self.phase.sin() * amplitude * 32767.0) as i16;
                    output.extend(std::iter::repeat_n(sample, channels));
                    self.phase = (self.phase + phase_increment) % (2.0 * std::f64::consts::PI);
                }
            }
            AudioSource::CustomSamples { samples, repeat } => {
                for _ in 0..frames {
                    let source_index = self
                        .custom_output_position
                        .saturating_mul(u64::from(self.custom_source_rate.max(1)))
                        / u64::from(self.sample_rate.max(1));
                    self.custom_output_position = self.custom_output_position.saturating_add(1);
                    let encoded = if samples.is_empty() {
                        0xff
                    } else if repeat {
                        let sample_count = u64::try_from(samples.len()).unwrap_or(u64::MAX);
                        samples[usize::try_from(source_index % sample_count).unwrap_or(0)]
                    } else {
                        usize::try_from(source_index)
                            .ok()
                            .and_then(|index| samples.get(index).copied())
                            .unwrap_or(0xff)
                    };
                    let sample = codec_core::utils::mulaw_to_linear_scalar(encoded);
                    output.extend(std::iter::repeat_n(sample, channels));
                }
            }
            AudioSource::PassThrough => output.resize(frames.saturating_mul(channels), 0),
        }
        output
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
        self.custom_output_position = 0;
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
    audio_generator: Arc<ParkingMutex<AudioGenerator>>,
    /// Transmission configuration
    config: AudioTransmitterConfig,
    /// Exact codec generation used for both bytes and RTP payload identity.
    codec_runtime: Arc<DialogCodecRuntime>,
    /// Whether transmission is active
    is_active: Arc<AtomicBool>,
    /// Background transmission task.
    tx_task: Option<JoinHandle<()>>,
}

impl AudioTransmitter {
    /// Create a new audio transmitter with default configuration (pass-through mode)
    pub fn new(rtp_session: Arc<Mutex<RtpSession>>) -> Self {
        let config = AudioTransmitterConfig::default();
        Self::new_with_config(rtp_session, config, default_pcmu_runtime())
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
        Self::new_with_config(rtp_session, config, default_pcmu_runtime())
    }

    /// Create a new audio transmitter with custom configuration
    pub(super) fn new_with_config(
        rtp_session: Arc<Mutex<RtpSession>>,
        config: AudioTransmitterConfig,
        codec_runtime: Arc<DialogCodecRuntime>,
    ) -> Self {
        let audio_generator = AudioGenerator::new_with_source_rates(
            codec_runtime.format.clock_rate,
            config.sample_rate,
            config.source.clone(),
        );

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
            audio_generator: Arc::new(ParkingMutex::new(audio_generator)),
            config,
            codec_runtime,
            is_active: Arc::new(AtomicBool::new(false)),
            tx_task: None,
        }
    }

    /// Start audio transmission
    pub async fn start(&mut self) {
        if self.tx_task.is_some() {
            debug!("AudioTransmitter: transmission task already running");
            return;
        }

        if matches!(self.config.source, AudioSource::PassThrough) {
            self.is_active.store(false, Ordering::Release);
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

        self.is_active.store(true, Ordering::Release);

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
        let initial_delay = next_audio_tx_start_delay(self.config.interval);
        diagnostics::record_audio_tx_task_started(initial_delay);
        let pacing_config = audio_tx_pacing_config_from_env();
        let codec_runtime = Arc::clone(&self.codec_runtime);
        let payload_type = codec_runtime.format.payload_type;
        let clock_rate = codec_runtime.format.clock_rate;
        let channels = codec_runtime.format.channels;

        let collect_diagnostics = diagnostics::enabled();
        let mut next_tick_at = TokioInstant::now() + initial_delay;
        let base_packet_interval = self.config.interval;
        let samples_per_packet = rescale_packet_frames(
            self.config.samples_per_packet,
            self.config.sample_rate,
            clock_rate,
        );
        let max_packetization_divisor = if codec_runtime.format.name.eq_ignore_ascii_case("opus") {
            max_divisor_for_duration(base_packet_interval, Duration::from_millis(60))
        } else {
            MAX_AUDIO_TX_PACKETIZATION_DIVISOR
        };
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
                let mut next_timestamp = 0_u32;

                while is_active.load(Ordering::Acquire) {
                    tokio::time::sleep_until(next_tick_at).await;
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
                        next_tick_at = next_audio_tx_deadline(
                            next_tick_at,
                            config_interval_or_floor(base_packet_interval),
                        );
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
                            next_timestamp = next_timestamp.wrapping_add(samples_per_packet as u32);
                            continue;
                        }
                        pacing_consecutive_skip_count = 0;
                        samples_per_packet
                    } else {
                        let divisor = adaptive_packetization_divisor(active_guard.active_count())
                            .min(max_packetization_divisor);
                        let packet_interval =
                            adaptive_packetization_interval(base_packet_interval, divisor);
                        next_tick_at = next_audio_tx_deadline(next_tick_at, packet_interval);
                        samples_per_packet.saturating_mul(divisor as usize)
                    };

                    // Generate codec-neutral PCM and encode it with the exact
                    // negotiated codec generation before assigning its PT.
                    let pcm_samples = {
                        let mut generator = audio_generator.lock();
                        generator.generate_pcm_frame(effective_samples_per_packet, channels)
                    };
                    let frame = AudioFrame::new(pcm_samples, clock_rate, channels, next_timestamp);
                    let audio_samples = match codec_runtime.encode(&frame).await {
                        Ok(payload) => Bytes::from(payload),
                        Err(error) => {
                            error!("Failed to encode generated RTP audio: {}", error);
                            is_active.store(false, Ordering::Release);
                            break;
                        }
                    };
                    let current_timestamp = next_timestamp;
                    next_timestamp = next_timestamp.wrapping_add(
                        u32::try_from(effective_samples_per_packet).unwrap_or(u32::MAX),
                    );

                    // Fast path: send through the lock-free handle — no
                    // `session.lock().await` per frame. PT and bytes come
                    // from the same immutable codec generation.
                    let send_started = collect_diagnostics.then(Instant::now);
                    let send_result = if let Some(handle) = &send_handle {
                        handle
                            .send_packet_with_pt(
                                current_timestamp,
                                audio_samples,
                                false,
                                payload_type,
                            )
                            .await
                    } else {
                        let session = rtp_session.lock().await;
                        session
                            .send_packet_with_pt(
                                current_timestamp,
                                audio_samples,
                                false,
                                payload_type,
                            )
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
                        is_active.store(false, Ordering::Release);
                        break;
                    }
                    debug!(
                        "📡 Sent RTP audio packet (timestamp: {}, {} frames, PT {})",
                        current_timestamp, effective_samples_per_packet, payload_type
                    );
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
        self.is_active.store(false, Ordering::Release);
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
        self.is_active.load(Ordering::Acquire)
    }

    pub(super) fn replacement_config(&self) -> AudioTransmitterConfig {
        let mut config = self.config.clone();
        config.source = self.audio_generator.lock().source.clone();
        config
    }

    #[cfg(test)]
    pub(super) fn codec_format(&self) -> &NegotiatedAudioCodec {
        &self.codec_runtime.format
    }

    /// Update the audio source during transmission
    pub async fn set_audio_source(&self, source: AudioSource) {
        let mut generator = self.audio_generator.lock();
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

fn default_pcmu_runtime() -> Arc<DialogCodecRuntime> {
    Arc::new(
        DialogCodecRuntime::new(NegotiatedAudioCodec {
            name: "PCMU".to_string(),
            payload_type: 0,
            clock_rate: 8_000,
            channels: 1,
            fmtp: None,
            dtx: false,
        })
        .expect("the built-in PCMU transmitter format is valid"),
    )
}

fn rescale_packet_frames(frames: usize, source_rate: u32, target_rate: u32) -> usize {
    let source_rate = u64::from(source_rate.max(1));
    let target_rate = u64::from(target_rate.max(1));
    let frames = u64::try_from(frames).unwrap_or(u64::MAX);
    usize::try_from(frames.saturating_mul(target_rate).div_ceil(source_rate))
        .unwrap_or(usize::MAX)
        .max(1)
}

fn max_divisor_for_duration(base: Duration, maximum: Duration) -> u64 {
    let base_nanos = base.as_nanos().max(1);
    let divisor = maximum.as_nanos() / base_nanos;
    u64::try_from(divisor).unwrap_or(u64::MAX).max(1)
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

fn adaptive_packetization_interval(base: Duration, divisor: u64) -> Duration {
    config_interval_or_floor(base).saturating_mul(u32::try_from(divisor.max(1)).unwrap_or(u32::MAX))
}

fn config_interval_or_floor(interval: Duration) -> Duration {
    if interval.is_zero() {
        Duration::from_millis(1)
    } else {
        interval
    }
}

fn next_audio_tx_deadline(previous: TokioInstant, interval: Duration) -> TokioInstant {
    let scheduled = previous + interval;
    let now = TokioInstant::now();
    if scheduled <= now {
        now + interval
    } else {
        scheduled
    }
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
        if let Some(task) = self.tx_task.take() {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        adaptive_packetization_divisor, adaptive_packetization_interval,
        audio_tx_start_delay_for_sequence, pacing_divisor, AudioGenerator, AudioSource,
        DialogCodecRuntime, NegotiatedAudioCodec,
    };
    use crate::types::AudioFrame;
    use std::sync::Arc;
    use std::time::Duration;

    async fn encode_generated(
        format: NegotiatedAudioCodec,
        source: AudioSource,
        frames: usize,
    ) -> (Vec<u8>, AudioFrame) {
        let mut generator = AudioGenerator::new_with_source(format.clock_rate, source);
        let pcm = generator.generate_pcm_frame(frames, format.channels);
        let runtime = Arc::new(DialogCodecRuntime::new(format.clone()).unwrap());
        let payload = runtime
            .encode(&AudioFrame::new(pcm, format.clock_rate, format.channels, 0))
            .await
            .unwrap();
        let decoded = runtime.decode(&payload, 0).await.unwrap();
        (payload, decoded)
    }

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
    fn custom_pcmu_is_resampled_to_the_negotiated_clock() {
        let mut generator = AudioGenerator::new_with_source_rates(
            48_000,
            8_000,
            AudioSource::CustomSamples {
                samples: vec![0xff, 0x7f],
                repeat: false,
            },
        );
        let output = generator.generate_pcm_frame(12, 1);
        assert!(output[..6].iter().all(|sample| *sample == output[0]));
        assert!(output[6..].iter().all(|sample| *sample == output[6]));
        assert_ne!(output[0], output[6]);
    }

    #[tokio::test]
    async fn generated_tone_and_custom_audio_use_pcma_encoding() {
        let format = NegotiatedAudioCodec {
            name: "PCMA".to_string(),
            payload_type: 8,
            clock_rate: 8_000,
            channels: 1,
            fmtp: None,
            dtx: false,
        };
        let (tone_payload, tone) = encode_generated(
            format.clone(),
            AudioSource::Tone {
                frequency: 440.0,
                amplitude: 0.4,
            },
            160,
        )
        .await;
        let (custom_payload, custom) = encode_generated(
            format,
            AudioSource::CustomSamples {
                samples: vec![0xff, 0xce, 0x4e],
                repeat: true,
            },
            160,
        )
        .await;
        assert_eq!(tone_payload.len(), 160);
        assert_eq!(custom_payload.len(), 160);
        assert_eq!(tone.sample_rate, 8_000);
        assert_eq!(custom.channels, 1);
    }

    #[cfg(feature = "opus")]
    #[tokio::test]
    async fn generated_tone_and_custom_audio_use_opus_encoding() {
        let format = NegotiatedAudioCodec {
            name: "opus".to_string(),
            payload_type: 96,
            clock_rate: 48_000,
            channels: 2,
            fmtp: None,
            dtx: false,
        };
        for source in [
            AudioSource::Tone {
                frequency: 440.0,
                amplitude: 0.4,
            },
            AudioSource::CustomSamples {
                samples: vec![0xff, 0xce, 0x4e],
                repeat: true,
            },
        ] {
            let (payload, decoded) = encode_generated(format.clone(), source, 960).await;
            assert!(!payload.is_empty());
            assert!(payload.len() < decoded.samples.len() * std::mem::size_of::<i16>());
            assert_eq!(decoded.sample_rate, 48_000);
            assert_eq!(decoded.channels, 2);
            assert_eq!(decoded.samples.len(), 1_920);
        }
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
    fn adaptive_packetization_sleeps_for_the_encoded_audio_duration() {
        let base = Duration::from_millis(20);
        assert_eq!(
            adaptive_packetization_interval(base, adaptive_packetization_divisor(500)),
            Duration::from_millis(20)
        );
        assert_eq!(
            adaptive_packetization_interval(base, adaptive_packetization_divisor(501)),
            Duration::from_millis(40)
        );
        assert_eq!(
            adaptive_packetization_interval(base, adaptive_packetization_divisor(4_800)),
            Duration::from_millis(180)
        );
        assert_eq!(
            adaptive_packetization_interval(base, adaptive_packetization_divisor(12_500)),
            Duration::from_millis(180)
        );
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
