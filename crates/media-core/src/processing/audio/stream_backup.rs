//! Audio Stream Management for Conference Participants
//!
//! This module provides AudioStream management functionality for multi-party conference
//! audio mixing. It handles individual participant audio streams, synchronization,
//! format conversion, and health monitoring.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use crate::types::{AudioFrame, SampleRate};
use crate::types::conference::{ParticipantId, AudioStream, ConferenceError, ConferenceResult};
use crate::processing::format::FormatConverter;
use crate::processing::audio::VoiceActivityDetector;
use crate::error::Result;

/// Manager for audio streams from conference participants
pub struct AudioStreamManager {
    /// Active participant streams
    streams: Arc<Mutex<std::collections::HashMap<ParticipantId, ManagedAudioStream>>>,
    
    /// Target audio format for mixing
    target_sample_rate: u32,
    target_channels: u8,
    
    /// Format converter for stream adaptation
    format_converter: Arc<FormatConverter>,
    
    /// Voice activity detector for stream optimization
    vad: Arc<VoiceActivityDetector>,
    
    /// Configuration
    config: AudioStreamConfig,
}

/// Internal managed audio stream with buffering and processing
struct ManagedAudioStream {
    /// Stream metadata
    stream_info: AudioStream,
    
    /// Audio frame buffer for synchronization
    frame_buffer: VecDeque<AudioFrame>,
    
    /// Last activity timestamp
    last_activity: Instant,
    
    /// Synchronization state
    sync_state: StreamSyncState,
    
    /// Processing statistics
    stats: StreamStats,
}

/// Stream synchronization state
#[derive(Debug, Clone)]
struct StreamSyncState {
    /// Reference timestamp for synchronization
    reference_timestamp: Option<u32>,
    
    /// Clock drift compensation
    drift_samples: i32,
    
    /// Timestamp alignment offset
    timestamp_offset: u32,
    
    /// Synchronization quality score (0.0 to 1.0)
    sync_quality: f32,
}

/// Statistics for an individual audio stream
#[derive(Debug, Clone, Default)]
struct StreamStats {
    /// Total frames processed
    frames_processed: u64,
    
    /// Frames dropped due to format issues
    frames_dropped_format: u64,
    
    /// Frames dropped due to buffer overflow
    frames_dropped_overflow: u64,
    
    /// Average processing latency in microseconds
    avg_processing_latency_us: u64,
    
    /// Current buffer depth
    buffer_depth: usize,
    
    /// Voice activity percentage (0.0 to 1.0)
    voice_activity_ratio: f32,
}

/// Configuration for audio stream management
#[derive(Debug, Clone)]
pub struct AudioStreamConfig {
    /// Maximum buffer depth per stream (in frames)
    pub max_buffer_depth: usize,
    
    /// Stream timeout (mark as inactive if no frames received)
    pub stream_timeout: Duration,
    
    /// Enable format conversion for mismatched streams
    pub enable_format_conversion: bool,
    
    /// Enable voice activity detection
    pub enable_voice_activity_detection: bool,
    
    /// Synchronization tolerance in samples
    pub sync_tolerance_samples: u32,
    
    /// Quality thresholds
    pub min_sync_quality: f32,
    pub max_drift_samples: i32,
}

impl Default for AudioStreamConfig {
    fn default() -> Self {
        Self {
            max_buffer_depth: 10, // 200ms at 20ms frames
            stream_timeout: Duration::from_secs(5),
            enable_format_conversion: true,
            enable_voice_activity_detection: true,
            sync_tolerance_samples: 80, // 10ms at 8kHz
            min_sync_quality: 0.8,
            max_drift_samples: 160, // 20ms at 8kHz
        }
    }
}

impl AudioStreamManager {
    /// Create a new audio stream manager
    pub fn new(
        target_sample_rate: u32,
        target_channels: u8,
        config: AudioStreamConfig,
    ) -> Result<Self> {
        let format_converter = Arc::new(FormatConverter::new()?);
        let vad = Arc::new(VoiceActivityDetector::new()?);
        
        Ok(Self {
            streams: Arc::new(Mutex::new(std::collections::HashMap::new())),
            target_sample_rate,
            target_channels,
            format_converter,
            vad,
            config,
        })
    }
    
    /// Add a new participant audio stream
    pub fn add_stream(&self, stream_info: AudioStream) -> ConferenceResult<()> {
        let mut streams = self.streams.lock().map_err(|_| ConferenceError::MixingFailed {
            reason: "Failed to acquire stream lock".to_string(),
        })?;
        
        if streams.contains_key(&stream_info.participant_id) {
            return Err(ConferenceError::ParticipantAlreadyExists {
                participant_id: stream_info.participant_id.clone(),
            });
        }
        
        let managed_stream = ManagedAudioStream {
            stream_info,
            frame_buffer: VecDeque::new(),
            last_activity: Instant::now(),
            sync_state: StreamSyncState {
                reference_timestamp: None,
                drift_samples: 0,
                timestamp_offset: 0,
                sync_quality: 1.0,
            },
            stats: StreamStats::default(),
        };
        
        streams.insert(managed_stream.stream_info.participant_id.clone(), managed_stream);
        Ok(())
    }
    
    /// Remove a participant audio stream
    pub fn remove_stream(&self, participant_id: &ParticipantId) -> ConferenceResult<()> {
        let mut streams = self.streams.lock().map_err(|_| ConferenceError::MixingFailed {
            reason: "Failed to acquire stream lock".to_string(),
        })?;
        
        streams.remove(participant_id).ok_or_else(|| ConferenceError::ParticipantNotFound {
            participant_id: participant_id.clone(),
        })?;
        
        Ok(())
    }
    
    /// Process an incoming audio frame for a participant
    pub fn process_frame(
        &self,
        participant_id: &ParticipantId,
        mut frame: AudioFrame,
    ) -> ConferenceResult<()> {
        let mut streams = self.streams.lock().map_err(|_| ConferenceError::MixingFailed {
            reason: "Failed to acquire stream lock".to_string(),
        })?;
        
        let managed_stream = streams.get_mut(participant_id)
            .ok_or_else(|| ConferenceError::ParticipantNotFound {
                participant_id: participant_id.clone(),
            })?;
        
        let start_time = Instant::now();
        
        // Update stream activity
        managed_stream.last_activity = Instant::now();
        managed_stream.stream_info.update_frame_received();
        
        // Voice activity detection if enabled
        if self.config.enable_voice_activity_detection {
            let is_talking = self.vad.detect_voice_activity(&frame)
                .unwrap_or(false);
            managed_stream.stream_info.is_talking = is_talking;
            
            // Update voice activity ratio
            let total_frames = managed_stream.stats.frames_processed as f32;
            let talking_frames = total_frames * managed_stream.stats.voice_activity_ratio;
            let new_talking_frames = if is_talking { talking_frames + 1.0 } else { talking_frames };
            managed_stream.stats.voice_activity_ratio = new_talking_frames / (total_frames + 1.0);
        }
        
        // Format conversion if needed
        if self.config.enable_format_conversion {
            if frame.sample_rate != self.target_sample_rate || frame.channels != self.target_channels {
                frame = self.convert_frame_format(frame, participant_id)?;
            }
        }
        
        // Synchronization processing
        self.process_frame_synchronization(&mut frame, managed_stream)?;
        
        // Buffer management
        if managed_stream.frame_buffer.len() >= self.config.max_buffer_depth {
            // Drop oldest frame to prevent overflow
            managed_stream.frame_buffer.pop_front();
            managed_stream.stats.frames_dropped_overflow += 1;
            managed_stream.stream_info.update_frame_dropped();
        }
        
        // Add frame to buffer
        managed_stream.frame_buffer.push_back(frame);
        
        // Update statistics
        managed_stream.stats.frames_processed += 1;
        managed_stream.stats.buffer_depth = managed_stream.frame_buffer.len();
        
        let processing_time = start_time.elapsed().as_micros() as u64;
        managed_stream.stats.avg_processing_latency_us = 
            (managed_stream.stats.avg_processing_latency_us + processing_time) / 2;
        
        Ok(())
    }
    
    /// Get synchronized audio frames for all active participants
    pub fn get_synchronized_frames(&self) -> ConferenceResult<Vec<(ParticipantId, AudioFrame)>> {
        let mut streams = self.streams.lock().map_err(|_| ConferenceError::MixingFailed {
            reason: "Failed to acquire stream lock".to_string(),
        })?;
        
        let mut frames = Vec::new();
        let now = Instant::now();
        
        // Collect frames from active, healthy streams
        for (participant_id, managed_stream) in streams.iter_mut() {
            // Check if stream is healthy
            if !managed_stream.stream_info.is_healthy(self.config.stream_timeout) {
                continue;
            }
            
            // Skip muted participants
            if managed_stream.stream_info.is_muted {
                continue;
            }
            
            // Skip non-talking participants if VAD is enabled
            if self.config.enable_voice_activity_detection && !managed_stream.stream_info.is_talking {
                continue;
            }
            
            // Get frame from buffer
            if let Some(frame) = managed_stream.frame_buffer.pop_front() {
                frames.push((participant_id.clone(), frame));
                managed_stream.stats.buffer_depth = managed_stream.frame_buffer.len();
            }
        }
        
        Ok(frames)
    }
    
    /// Get stream statistics for monitoring
    pub fn get_stream_stats(&self, participant_id: &ParticipantId) -> ConferenceResult<StreamStats> {
        let streams = self.streams.lock().map_err(|_| ConferenceError::MixingFailed {
            reason: "Failed to acquire stream lock".to_string(),
        })?;
        
        let managed_stream = streams.get(participant_id)
            .ok_or_else(|| ConferenceError::ParticipantNotFound {
                participant_id: participant_id.clone(),
            })?;
        
        Ok(managed_stream.stats.clone())
    }
    
    /// Get list of active participants
    pub fn get_active_participants(&self) -> ConferenceResult<Vec<ParticipantId>> {
        let streams = self.streams.lock().map_err(|_| ConferenceError::MixingFailed {
            reason: "Failed to acquire stream lock".to_string(),
        })?;
        
        let now = Instant::now();
        let active_participants: Vec<ParticipantId> = streams
            .values()
            .filter(|stream| stream.stream_info.is_healthy(self.config.stream_timeout))
            .map(|stream| stream.stream_info.participant_id.clone())
            .collect();
        
        Ok(active_participants)
    }
    
    /// Convert audio frame format to target format
    fn convert_frame_format(
        &self,
        frame: AudioFrame,
        participant_id: &ParticipantId,
    ) -> ConferenceResult<AudioFrame> {
        self.format_converter.convert_frame(
            frame,
            self.target_sample_rate,
            self.target_channels,
        ).map_err(|e| ConferenceError::FrameProcessingError {
            reason: format!("Format conversion failed for participant {}: {}", participant_id, e),
        })
    }
    
    /// Process frame synchronization
    fn process_frame_synchronization(
        &self,
        frame: &mut AudioFrame,
        managed_stream: &mut ManagedAudioStream,
    ) -> ConferenceResult<()> {
        // Initialize reference timestamp if this is the first frame
        if managed_stream.sync_state.reference_timestamp.is_none() {
            managed_stream.sync_state.reference_timestamp = Some(frame.timestamp);
            managed_stream.sync_state.timestamp_offset = 0;
            return Ok(());
        }
        
        let reference_ts = managed_stream.sync_state.reference_timestamp.unwrap();
        let expected_ts = reference_ts.wrapping_add(managed_stream.sync_state.timestamp_offset);
        let drift = (frame.timestamp as i64) - (expected_ts as i64);
        
        // Update drift compensation
        managed_stream.sync_state.drift_samples = drift as i32;
        
        // Calculate synchronization quality
        let drift_ratio = (drift.abs() as f32) / (self.config.sync_tolerance_samples as f32);
        managed_stream.sync_state.sync_quality = (1.0 - drift_ratio).max(0.0);
        
        // Update timestamp offset for next frame
        managed_stream.sync_state.timestamp_offset = managed_stream.sync_state.timestamp_offset
            .wrapping_add(frame.samples.len() as u32 / frame.channels as u32);
        
        Ok(())
    }
    
    /// Clean up inactive streams
    pub fn cleanup_inactive_streams(&self) -> ConferenceResult<Vec<ParticipantId>> {
        let mut streams = self.streams.lock().map_err(|_| ConferenceError::MixingFailed {
            reason: "Failed to acquire stream lock".to_string(),
        })?;
        
        let timeout = self.config.stream_timeout;
        let mut removed_participants = Vec::new();
        
        streams.retain(|participant_id, managed_stream| {
            if !managed_stream.stream_info.is_healthy(timeout) {
                removed_participants.push(participant_id.clone());
                false
            } else {
                true
            }
        });
        
        Ok(removed_participants)
    }
} 