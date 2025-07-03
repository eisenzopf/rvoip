//! Core Audio Mixer for Multi-Party Conference Calls
//!
//! This module provides the AudioMixer component that handles real-time mixing
//! of multiple audio streams for conference calls. Each participant receives
//! a mix of all other participants (N-1 mixing).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use crate::types::{AudioFrame, SampleRate};
use crate::types::conference::{
    ParticipantId, AudioStream, ConferenceMixingConfig, MixingQuality, 
    MixedAudioOutput, ConferenceMixingStats, ConferenceError, ConferenceResult,
    ConferenceMixingEvent
};
use crate::processing::audio::{AudioStreamManager, AudioStreamConfig, VoiceActivityDetector, VadConfig};
use crate::processing::format::FormatConverter;
use crate::error::Result;

/// Core audio mixer for multi-party conference calls
pub struct AudioMixer {
    /// Stream manager for participant audio streams
    stream_manager: Arc<AudioStreamManager>,
    
    /// Configuration for mixing behavior
    config: ConferenceMixingConfig,
    
    /// Current mixing statistics
    stats: Arc<Mutex<ConferenceMixingStats>>,
    
    /// Mixed output cache (participant_id -> mixed_frame)
    output_cache: Arc<Mutex<HashMap<ParticipantId, Arc<AudioFrame>>>>,
    
    /// Memory pool for audio frames to reduce allocations
    frame_pool: Arc<Mutex<Vec<AudioFrame>>>,
    
    /// Format converter for audio processing
    format_converter: Arc<Mutex<FormatConverter>>,
    
    /// Voice activity detector for selective mixing
    vad: Arc<Mutex<VoiceActivityDetector>>,
    
    /// Event sender for conference monitoring
    event_sender: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedSender<ConferenceMixingEvent>>>>,
}

/// Audio mixing algorithms implementation
struct MixingAlgorithms;

impl AudioMixer {
    /// Create a new audio mixer with the given configuration
    pub async fn new(config: ConferenceMixingConfig) -> ConferenceResult<Self> {
        // Create audio stream manager with appropriate configuration
        let stream_config = AudioStreamConfig {
            max_buffer_depth: 5, // Smaller buffer for real-time mixing
            stream_timeout: std::time::Duration::from_secs(10),
            enable_format_conversion: true,
            enable_voice_activity_detection: config.enable_voice_activity_mixing,
            sync_tolerance_samples: config.output_samples_per_frame / 4, // 25% tolerance
            min_sync_quality: 0.7,
            max_drift_samples: config.output_samples_per_frame as i32,
        };
        
        let stream_manager = Arc::new(AudioStreamManager::new(
            config.output_sample_rate,
            config.output_channels,
            stream_config,
        )?);
        
        let format_converter = Arc::new(Mutex::new(FormatConverter::new()));
        let vad_config = crate::processing::audio::VadConfig::default();
        let vad = VoiceActivityDetector::new(vad_config)
            .map_err(|e| ConferenceError::MixingFailed {
                reason: format!("Failed to create VAD: {}", e),
            })?;
        let vad = Arc::new(Mutex::new(vad));
        
        Ok(Self {
            stream_manager,
            config,
            stats: Arc::new(Mutex::new(ConferenceMixingStats::default())),
            output_cache: Arc::new(Mutex::new(HashMap::new())),
            frame_pool: Arc::new(Mutex::new(Vec::new())),
            format_converter,
            vad,
            event_sender: Arc::new(Mutex::new(None)),
        })
    }
    
    /// Set event sender for conference monitoring
    pub async fn set_event_sender(&self, sender: tokio::sync::mpsc::UnboundedSender<ConferenceMixingEvent>) {
        let mut event_sender = self.event_sender.lock().await; {
            *event_sender = Some(sender);
        }
    }
    
    /// Add a participant audio stream to the conference
    pub async fn add_audio_stream(&self, id: ParticipantId, stream: AudioStream) -> ConferenceResult<()> {
        // Check conference capacity
        let active_participants = self.stream_manager.get_active_participants()?;
        if active_participants.len() >= self.config.max_participants {
            return Err(ConferenceError::ConferenceAtCapacity { 
                max: self.config.max_participants 
            });
        }
        
        // Add stream to manager
        self.stream_manager.add_stream(stream)?;
        
        // Update statistics
        let mut stats = self.stats.lock().await;
        stats.active_participants = active_participants.len() + 1;
        
        // Send event
        self.send_event(ConferenceMixingEvent::ParticipantAdded {
            participant_id: id.clone(),
            participant_count: active_participants.len() + 1,
        }).await;
        
        Ok(())
    }
    
    /// Remove a participant audio stream from the conference
    pub async fn remove_audio_stream(&self, id: &ParticipantId) -> ConferenceResult<()> {
        // Remove stream from manager
        self.stream_manager.remove_stream(id)?;
        
        // Clear cached output for this participant
        let mut cache = self.output_cache.lock().await;
        cache.remove(id);
        
        // Update statistics
        let active_participants = self.stream_manager.get_active_participants()?;
        let mut stats = self.stats.lock().await;
        stats.active_participants = active_participants.len();
        
        // Send event
        self.send_event(ConferenceMixingEvent::ParticipantRemoved {
            participant_id: id.clone(),
            participant_count: active_participants.len(),
        }).await;
        
        Ok(())
    }
    
    /// Process audio frame from a participant
    pub async fn process_audio_frame(&self, participant_id: &ParticipantId, frame: AudioFrame) -> ConferenceResult<()> {
        self.stream_manager.process_frame(participant_id, frame)
    }
    
    /// Get mixed audio for a specific participant (everyone except themselves)
    pub async fn get_mixed_audio(&self, participant_id: &ParticipantId) -> ConferenceResult<Option<AudioFrame>> {
        // Check cache first
        let cache = self.output_cache.lock().await; {
            if let Some(cached_frame) = cache.get(participant_id) {
                return Ok(Some((**cached_frame).clone()));
            }
        }
        
        // No cached output, need to generate mix
        Ok(None)
    }
    
    /// Mix audio from all participants and produce outputs for each
    /// This is the core N-way mixing function: N inputs → N outputs (N-1 mixing)
    pub async fn mix_participants(&self, inputs: &[AudioFrame]) -> ConferenceResult<HashMap<ParticipantId, AudioFrame>> {
        let start_time = Instant::now();
        
        // Get synchronized frames from all participants
        let participant_frames = self.stream_manager.get_synchronized_frames()?;
        
        let mut mixed_outputs = HashMap::new();
        
        if participant_frames.len() >= 2 {
            // We have enough frames for actual mixing
            // For each participant, create a mix of all OTHER participants
            for (target_participant, _) in &participant_frames {
                let mixed_frame = self.create_mixed_frame_for_participant(
                    target_participant,
                    &participant_frames,
                )?;
                
                if let Some(frame) = mixed_frame {
                    mixed_outputs.insert(target_participant.clone(), frame);
                }
            }
        }
        
        // Always update statistics (even for attempted mixes)
        self.update_mixing_stats(start_time, participant_frames.len(), &mixed_outputs).await?;
        
        // Cache outputs (if any)
        if !mixed_outputs.is_empty() {
            let mut cache = self.output_cache.lock().await;
            cache.clear();
            for (participant_id, frame) in &mixed_outputs {
                cache.insert(participant_id.clone(), Arc::new(frame.clone()));
            }
        }
        
        Ok(mixed_outputs)
    }
    
    /// Create mixed audio frame for a specific participant (exclude their own audio)
    fn create_mixed_frame_for_participant(
        &self,
        target_participant: &ParticipantId,
        all_frames: &[(ParticipantId, AudioFrame)],
    ) -> ConferenceResult<Option<AudioFrame>> {
        // Filter out the target participant's own audio
        let other_frames: Vec<&AudioFrame> = all_frames
            .iter()
            .filter(|(id, _)| id != target_participant)
            .map(|(_, frame)| frame)
            .collect();
        
        if other_frames.is_empty() {
            return Ok(None);
        }
        
        // Mix the audio frames using the configured algorithm
        let mixed_frame = match self.config.mixing_quality {
            MixingQuality::Fast => MixingAlgorithms::fast_mix(&other_frames, &self.config)?,
            MixingQuality::Balanced => MixingAlgorithms::balanced_mix(&other_frames, &self.config)?,
            MixingQuality::High => MixingAlgorithms::high_quality_mix(&other_frames, &self.config)?,
        };
        
        Ok(Some(mixed_frame))
    }
    
    /// Update mixing statistics
    async fn update_mixing_stats(
        &self,
        start_time: Instant,
        participant_count: usize,
        mixed_outputs: &HashMap<ParticipantId, AudioFrame>,
    ) -> ConferenceResult<()> {
        let mixing_latency = start_time.elapsed().as_micros() as u64;
        
        // Get actual active participant count (not just frame count)
        let actual_active_count = self.stream_manager.get_active_participants()?.len();
        
        let mut stats = self.stats.lock().await; {
            stats.total_mixes += 1;
            stats.active_participants = actual_active_count; // Use actual count, not frame count
            stats.avg_mixing_latency_us = 
                (stats.avg_mixing_latency_us + mixing_latency) / 2;
            
            // Estimate CPU usage based on mixing latency
            let frame_duration_us = (self.config.output_samples_per_frame as f64 / 
                                   self.config.output_sample_rate as f64) * 1_000_000.0;
            stats.cpu_usage = (mixing_latency as f32) / (frame_duration_us as f32);
            
            // Update memory usage estimate
            stats.memory_usage_bytes = mixed_outputs.len() * 
                (self.config.output_samples_per_frame as usize * 2); // i16 samples
        }
        
        // Check for performance warnings
        if mixing_latency > 5000 { // 5ms threshold
            self.send_event(ConferenceMixingEvent::PerformanceWarning {
                latency_us: mixing_latency,
                cpu_usage: (mixing_latency as f32) / 20000.0, // 20ms frame
                reason: "High mixing latency detected".to_string(),
            }).await;
        }
        
        Ok(())
    }
    
    /// Get current mixing statistics
    pub async fn get_mixing_stats(&self) -> ConferenceResult<ConferenceMixingStats> {
        let stats = self.stats.lock().await;
        Ok(stats.clone())
    }
    
    /// Get list of active participants
    pub async fn get_active_participants(&self) -> ConferenceResult<Vec<ParticipantId>> {
        self.stream_manager.get_active_participants()
    }
    
    /// Clean up inactive participants
    pub async fn cleanup_inactive_participants(&self) -> ConferenceResult<Vec<ParticipantId>> {
        let removed = self.stream_manager.cleanup_inactive_streams()?;
        
        // Clear cache for removed participants
        let mut cache = self.output_cache.lock().await;
        for participant_id in &removed {
            cache.remove(participant_id);
        }
        
        // Update stats
        let active_count = self.stream_manager.get_active_participants()?.len();
        let mut stats = self.stats.lock().await;
        stats.active_participants = active_count;
        
        // Send events for removed participants
        for participant_id in &removed {
            self.send_event(ConferenceMixingEvent::ParticipantRemoved {
                participant_id: participant_id.clone(),
                participant_count: active_count,
            }).await;
        }
        
        Ok(removed)
    }
    
    /// Send a conference mixing event
    async fn send_event(&self, event: ConferenceMixingEvent) {
        let event_sender = self.event_sender.lock().await; {
            if let Some(sender) = event_sender.as_ref() {
                let _ = sender.send(event);
            }
        }
    }
    
    /// Flush pending events (ensure they are delivered)
    /// This is primarily for testing to ensure events are processed synchronously
    pub async fn flush_events(&self) {
        // Add sufficient delay for async event processing in tests
        // This ensures events are delivered through the channel before continuing
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }
}

impl MixingAlgorithms {
    /// Fast mixing algorithm - simple additive mixing
    fn fast_mix(frames: &[&AudioFrame], config: &ConferenceMixingConfig) -> ConferenceResult<AudioFrame> {
        if frames.is_empty() {
            return Err(ConferenceError::MixingFailed {
                reason: "No frames to mix".to_string(),
            });
        }
        
        let first_frame = frames[0];
        let samples_per_channel = first_frame.samples_per_channel();
        let mut mixed_samples = vec![0i32; first_frame.samples.len()];
        
        // Simple additive mixing
        for frame in frames {
            for (i, &sample) in frame.samples.iter().enumerate() {
                mixed_samples[i] += sample as i32;
            }
        }
        
        // Apply overflow protection if enabled
        let final_samples: Vec<i16> = if config.overflow_protection {
            let participant_count = frames.len() as i32;
            mixed_samples.iter()
                .map(|&sample| {
                    let normalized = sample / participant_count;
                    normalized.clamp(-32768, 32767) as i16
                })
                .collect()
        } else {
            mixed_samples.iter()
                .map(|&sample| sample.clamp(-32768, 32767) as i16)
                .collect()
        };
        
        Ok(AudioFrame::new(
            final_samples,
            config.output_sample_rate,
            config.output_channels,
            first_frame.timestamp,
        ))
    }
    
    /// Balanced mixing algorithm - additive with AGC
    fn balanced_mix(frames: &[&AudioFrame], config: &ConferenceMixingConfig) -> ConferenceResult<AudioFrame> {
        let mut mixed_frame = Self::fast_mix(frames, config)?;
        
        // Apply automatic gain control if enabled
        if config.enable_automatic_gain_control {
            Self::apply_agc(&mut mixed_frame)?;
        }
        
        Ok(mixed_frame)
    }
    
    /// High quality mixing algorithm - balanced with noise reduction
    fn high_quality_mix(frames: &[&AudioFrame], config: &ConferenceMixingConfig) -> ConferenceResult<AudioFrame> {
        let mut mixed_frame = Self::balanced_mix(frames, config)?;
        
        // Apply noise reduction if enabled
        if config.enable_noise_reduction {
            Self::apply_noise_reduction(&mut mixed_frame)?;
        }
        
        Ok(mixed_frame)
    }
    
    /// Apply automatic gain control to mixed audio
    fn apply_agc(frame: &mut AudioFrame) -> ConferenceResult<()> {
        // Calculate RMS level
        let rms = Self::calculate_rms(&frame.samples);
        let target_level = 8000.0; // Target RMS level for -20dB
        
        if rms > 0.0 {
            let gain = target_level / rms;
            let gain = gain.clamp(0.1, 4.0); // Limit gain range
            
            for sample in &mut frame.samples {
                *sample = ((*sample as f32) * gain).clamp(-32768.0, 32767.0) as i16;
            }
        }
        
        Ok(())
    }
    
    /// Apply basic noise reduction
    fn apply_noise_reduction(frame: &mut AudioFrame) -> ConferenceResult<()> {
        // Simple noise gate - mute samples below threshold
        let threshold = 500; // Noise gate threshold
        
        for sample in &mut frame.samples {
            if sample.abs() < threshold {
                *sample = 0;
            }
        }
        
        Ok(())
    }
    
    /// Calculate RMS (Root Mean Square) of audio samples
    fn calculate_rms(samples: &[i16]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        
        let sum_squares: f64 = samples.iter()
            .map(|&sample| (sample as f64).powi(2))
            .sum();
        
        (sum_squares / samples.len() as f64).sqrt() as f32
    }
}
