//! Audio generation and transmission functionality
//!
//! This module provides audio generation capabilities for testing and
//! audio transmission management for RTP sessions with support for multiple audio sources.

use bytes::Bytes;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{interval_at, Instant as TokioInstant, MissedTickBehavior};
use tracing::{debug, error, info, warn};

use crate::diagnostics;
use rvoip_rtp_core::{RtpSendHandle, RtpSession};

const AUDIO_TX_PHASE_MULTIPLIER: u64 = 0x9E37_79B9_7F4A_7C15;

static AUDIO_TX_START_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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
}

impl AudioGenerator {
    /// Create a new audio generator with tone generation
    pub fn new(sample_rate: u32, frequency: f64, amplitude: f64) -> Self {
        Self {
            sample_rate,
            phase: 0.0,
            source: AudioSource::Tone {
                frequency,
                amplitude,
            },
            sample_position: 0,
        }
    }

    /// Create a new audio generator with custom audio source
    pub fn new_with_source(sample_rate: u32, source: AudioSource) -> Self {
        Self {
            sample_rate,
            phase: 0.0,
            source,
            sample_position: 0,
        }
    }

    /// Generate audio samples for PCMU (G.711 μ-law) encoding
    pub fn generate_pcmu_samples(&mut self, num_samples: usize) -> Vec<u8> {
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
        self.source = source;
        self.sample_position = 0; // Reset position for custom samples
    }
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
    /// Whether transmission is active
    is_active: Arc<AtomicBool>,
    /// Background transmission task.
    tx_task: Option<JoinHandle<()>>,
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
        let collect_diagnostics = diagnostics::enabled();
        let mut interval_timer =
            interval_at(TokioInstant::now() + initial_delay, self.config.interval);
        interval_timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let samples_per_packet = self.config.samples_per_packet;

        self.tx_task = Some(super::spawn_memory_tracked(
            "media_core.audio_transmitter_task",
            async move {
                let mut last_tick_at: Option<TokioInstant> = None;
                let mut tick_gap_count = 0_u64;
                let mut tick_gap_total = Duration::ZERO;
                let mut tick_gap_max = Duration::ZERO;
                let mut send_count = 0_u64;
                let mut send_failures = 0_u64;
                let mut send_total = Duration::ZERO;
                let mut send_max = Duration::ZERO;
                let mut next_timestamp = 0_u32;

                while is_active.load(Ordering::Acquire) {
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

                    // Generate audio samples
                    let audio_samples = {
                        let mut generator = audio_generator.lock().await;
                        generator.generate_pcmu_samples(samples_per_packet)
                    };
                    record_transient_allocation(
                        "media_core.audio.tx.payload_vec",
                        audio_samples.capacity(),
                    );

                    // Send RTP packet (only if not in pass-through mode)
                    if !matches!(audio_samples.as_slice(), [0x7F, ..] if audio_samples.iter().all(|&x| x == 0x7F))
                    {
                        let current_timestamp = next_timestamp;
                        next_timestamp =
                            next_timestamp.wrapping_add(samples_per_packet as u32);

                        // Fast path: send through the lock-free handle —
                        // no `session.lock().await` per frame.
                        let send_started = collect_diagnostics.then(Instant::now);
                        let send_result = if let Some(handle) = &send_handle {
                            handle
                                .send_packet(current_timestamp, Bytes::from(audio_samples), false)
                                .await
                        } else {
                            // Fallback: session lock per frame. Only reached
                            // if the session was missing a scheduler when
                            // we tried to build the handle.
                            let session = rtp_session.lock().await;
                            session
                                .send_packet(current_timestamp, Bytes::from(audio_samples), false)
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
                        } else {
                            debug!(
                                "📡 Sent RTP audio packet (timestamp: {}, {} samples)",
                                current_timestamp, samples_per_packet
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
    use super::{audio_tx_start_delay_for_sequence, AudioGenerator, AudioSource};
    use std::time::Duration;

    #[test]
    fn generate_pcmu_samples_returns_requested_length() {
        let mut generator = AudioGenerator::new(8000, 440.0, 0.5);
        let output = generator.generate_pcmu_samples(160);

        assert_eq!(output.len(), 160);
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
}
