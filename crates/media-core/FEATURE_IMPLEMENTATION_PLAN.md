# Media-Core Feature Implementation Plan

## Executive Summary

This document outlines the implementation plan for missing media capabilities in media-core that are required to support B2BUA use cases including call centers, IVR systems, call recording, and conferencing.

**Timeline**: 8 weeks
**Priority**: High (blocking B2BUA production use cases)
**Dependencies**: None (can proceed in parallel with b2bua-core development)

## Feature Overview

### Critical Features (Weeks 1-4)
1. **Recording System**: File-based audio recording with multiple formats
2. **Announcement Engine**: Audio playback and queue management
3. **DTMF Framework**: Collection, detection, and generation

### Advanced Features (Weeks 5-8)
4. **Conference Enhancement**: Participant management and mixing matrix
5. **IVR Support**: Prompt interruption and audio scheduling
6. **WebRTC Integration**: ICE/STUN for browser clients (future)

## Week 1-2: Recording System Implementation

### 1.1 Recording Backend Architecture

**File: `/crates/media-core/src/recording/mod.rs`**
```rust
//! Audio recording subsystem for media-core
//!
//! Provides file-based recording with support for multiple formats,
//! concurrent recordings, and compliance features.

pub mod backend;
pub mod formats;
pub mod manager;
pub mod buffer;

pub use backend::{RecordingBackend, RecordingHandle};
pub use formats::{WavRecorder, Mp3Recorder, OpusRecorder};
pub use manager::{RecordingManager, RecordingSession};

use crate::error::Result;
use crate::types::MediaSessionId;
```

### 1.2 Recording Backend Trait

**File: `/crates/media-core/src/recording/backend.rs`**
```rust
//! Recording backend trait and common functionality

use async_trait::async_trait;
use std::path::PathBuf;
use crate::error::Result;
use crate::types::{AudioFrame, AudioFormat};

/// Handle for an active recording
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct RecordingHandle {
    pub id: uuid::Uuid,
    pub path: PathBuf,
    pub format: AudioFormat,
    pub started_at: std::time::Instant,
}

/// Recording configuration
#[derive(Debug, Clone)]
pub struct RecordingConfig {
    /// Output format
    pub format: AudioFormat,
    /// Number of channels (1 = mono, 2 = stereo)
    pub channels: u8,
    /// Sample rate in Hz
    pub sample_rate: u32,
    /// Output file path
    pub output_path: PathBuf,
    /// Maximum file size in bytes (0 = unlimited)
    pub max_size_bytes: u64,
    /// Maximum duration in seconds (0 = unlimited)
    pub max_duration_secs: u32,
}

/// Recording information after completion
#[derive(Debug, Clone)]
pub struct RecordingInfo {
    /// File path
    pub path: PathBuf,
    /// File size in bytes
    pub size_bytes: u64,
    /// Duration in seconds
    pub duration_secs: f32,
    /// Number of samples written
    pub samples_written: u64,
    /// Any errors encountered
    pub errors: Vec<String>,
}

/// Recording backend trait
#[async_trait]
pub trait RecordingBackend: Send + Sync {
    /// Start a new recording
    async fn start_recording(&self, config: RecordingConfig) -> Result<RecordingHandle>;

    /// Write audio samples to recording
    async fn write_audio(&self, handle: &RecordingHandle, frame: &AudioFrame) -> Result<()>;

    /// Stop recording and finalize file
    async fn stop_recording(&self, handle: RecordingHandle) -> Result<RecordingInfo>;

    /// Pause recording temporarily
    async fn pause_recording(&self, handle: &RecordingHandle) -> Result<()>;

    /// Resume paused recording
    async fn resume_recording(&self, handle: &RecordingHandle) -> Result<()>;

    /// Get current recording statistics
    async fn get_stats(&self, handle: &RecordingHandle) -> Result<RecordingInfo>;
}
```

### 1.3 WAV Recorder Implementation

**File: `/crates/media-core/src/recording/formats/wav.rs`**
```rust
//! WAV format recording implementation

use std::fs::File;
use std::io::{BufWriter, Write, Seek, SeekFrom};
use std::path::PathBuf;
use std::collections::HashMap;
use tokio::sync::RwLock;
use async_trait::async_trait;
use byteorder::{LittleEndian, WriteBytesExt};

use crate::recording::{
    RecordingBackend, RecordingHandle, RecordingConfig, RecordingInfo
};
use crate::types::AudioFrame;
use crate::error::{Result, Error};

/// WAV file recorder
pub struct WavRecorder {
    /// Active recordings
    active: Arc<RwLock<HashMap<RecordingHandle, WavSession>>>,
}

/// Active WAV recording session
struct WavSession {
    /// File writer
    writer: BufWriter<File>,
    /// Recording configuration
    config: RecordingConfig,
    /// Samples written
    samples_written: u64,
    /// Data chunk position for size update
    data_chunk_pos: u64,
    /// Recording state
    state: RecordingState,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum RecordingState {
    Recording,
    Paused,
    Stopped,
}

impl WavRecorder {
    /// Create new WAV recorder
    pub fn new() -> Self {
        Self {
            active: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Write WAV header
    fn write_header(
        writer: &mut BufWriter<File>,
        channels: u16,
        sample_rate: u32,
        bits_per_sample: u16,
    ) -> Result<u64> {
        // RIFF header
        writer.write_all(b"RIFF")?;
        writer.write_u32::<LittleEndian>(0)?; // File size - 8 (will update later)
        writer.write_all(b"WAVE")?;

        // Format chunk
        writer.write_all(b"fmt ")?;
        writer.write_u32::<LittleEndian>(16)?; // Chunk size
        writer.write_u16::<LittleEndian>(1)?;  // PCM format
        writer.write_u16::<LittleEndian>(channels)?;
        writer.write_u32::<LittleEndian>(sample_rate)?;
        writer.write_u32::<LittleEndian>(
            sample_rate * channels as u32 * (bits_per_sample / 8) as u32
        )?; // Byte rate
        writer.write_u16::<LittleEndian>(
            channels * (bits_per_sample / 8)
        )?; // Block align
        writer.write_u16::<LittleEndian>(bits_per_sample)?;

        // Data chunk header
        writer.write_all(b"data")?;
        let data_chunk_pos = writer.stream_position()?;
        writer.write_u32::<LittleEndian>(0)?; // Data size (will update later)

        Ok(data_chunk_pos)
    }

    /// Update WAV header with final sizes
    fn update_header(
        writer: &mut BufWriter<File>,
        data_chunk_pos: u64,
        data_size: u32,
    ) -> Result<()> {
        let current_pos = writer.stream_position()?;

        // Update RIFF chunk size
        writer.seek(SeekFrom::Start(4))?;
        writer.write_u32::<LittleEndian>(data_size + 36)?;

        // Update data chunk size
        writer.seek(SeekFrom::Start(data_chunk_pos))?;
        writer.write_u32::<LittleEndian>(data_size)?;

        // Restore position
        writer.seek(SeekFrom::Start(current_pos))?;

        Ok(())
    }
}

#[async_trait]
impl RecordingBackend for WavRecorder {
    async fn start_recording(&self, config: RecordingConfig) -> Result<RecordingHandle> {
        // Create output file
        let file = File::create(&config.output_path)
            .map_err(|e| Error::io(format!("Failed to create WAV file: {}", e)))?;

        let mut writer = BufWriter::new(file);

        // Write WAV header
        let data_chunk_pos = Self::write_header(
            &mut writer,
            config.channels as u16,
            config.sample_rate,
            16, // 16-bit samples
        )?;

        // Create handle
        let handle = RecordingHandle {
            id: uuid::Uuid::new_v4(),
            path: config.output_path.clone(),
            format: AudioFormat::Wav,
            started_at: std::time::Instant::now(),
        };

        // Create session
        let session = WavSession {
            writer,
            config,
            samples_written: 0,
            data_chunk_pos,
            state: RecordingState::Recording,
        };

        // Store session
        self.active.write().await.insert(handle.clone(), session);

        Ok(handle)
    }

    async fn write_audio(&self, handle: &RecordingHandle, frame: &AudioFrame) -> Result<()> {
        let mut sessions = self.active.write().await;
        let session = sessions.get_mut(handle)
            .ok_or_else(|| Error::recording("Recording session not found"))?;

        if session.state != RecordingState::Recording {
            return Ok(()); // Skip if paused or stopped
        }

        // Check limits
        if session.config.max_duration_secs > 0 {
            let elapsed = handle.started_at.elapsed().as_secs();
            if elapsed >= session.config.max_duration_secs as u64 {
                session.state = RecordingState::Stopped;
                return Ok(());
            }
        }

        // Write samples as 16-bit little-endian
        for sample in &frame.samples {
            session.writer.write_i16::<LittleEndian>(*sample)?;
            session.samples_written += 1;
        }

        // Check size limit
        if session.config.max_size_bytes > 0 {
            let size = session.samples_written * 2; // 2 bytes per sample
            if size >= session.config.max_size_bytes {
                session.state = RecordingState::Stopped;
            }
        }

        Ok(())
    }

    async fn stop_recording(&self, handle: RecordingHandle) -> Result<RecordingInfo> {
        let mut sessions = self.active.write().await;
        let mut session = sessions.remove(&handle)
            .ok_or_else(|| Error::recording("Recording session not found"))?;

        // Update WAV header with final size
        let data_size = (session.samples_written * 2) as u32;
        Self::update_header(&mut session.writer, session.data_chunk_pos, data_size)?;

        // Flush and close
        session.writer.flush()?;

        // Calculate duration
        let duration_secs = session.samples_written as f32
            / (session.config.sample_rate as f32 * session.config.channels as f32);

        Ok(RecordingInfo {
            path: handle.path,
            size_bytes: (data_size + 44) as u64, // Include header
            duration_secs,
            samples_written: session.samples_written,
            errors: Vec::new(),
        })
    }

    async fn pause_recording(&self, handle: &RecordingHandle) -> Result<()> {
        let mut sessions = self.active.write().await;
        if let Some(session) = sessions.get_mut(handle) {
            session.state = RecordingState::Paused;
        }
        Ok(())
    }

    async fn resume_recording(&self, handle: &RecordingHandle) -> Result<()> {
        let mut sessions = self.active.write().await;
        if let Some(session) = sessions.get_mut(handle) {
            if session.state == RecordingState::Paused {
                session.state = RecordingState::Recording;
            }
        }
        Ok(())
    }

    async fn get_stats(&self, handle: &RecordingHandle) -> Result<RecordingInfo> {
        let sessions = self.active.read().await;
        let session = sessions.get(handle)
            .ok_or_else(|| Error::recording("Recording session not found"))?;

        let duration_secs = session.samples_written as f32
            / (session.config.sample_rate as f32 * session.config.channels as f32);

        Ok(RecordingInfo {
            path: handle.path.clone(),
            size_bytes: (session.samples_written * 2 + 44) as u64,
            duration_secs,
            samples_written: session.samples_written,
            errors: Vec::new(),
        })
    }
}
```

### 1.4 Recording Manager

**File: `/crates/media-core/src/recording/manager.rs`**
```rust
//! High-level recording management

use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::RwLock;

use crate::recording::{
    RecordingBackend, RecordingHandle, RecordingConfig, RecordingInfo,
    WavRecorder, Mp3Recorder, OpusRecorder,
};
use crate::types::{MediaSessionId, AudioFormat, AudioFrame};
use crate::error::{Result, Error};

/// Recording session information
#[derive(Debug, Clone)]
pub struct RecordingSession {
    /// Recording handle
    pub handle: RecordingHandle,
    /// Associated media session
    pub media_session_id: MediaSessionId,
    /// Recording metadata
    pub metadata: HashMap<String, String>,
}

/// Recording manager for coordinating multiple recordings
pub struct RecordingManager {
    /// WAV recorder
    wav_recorder: Arc<WavRecorder>,
    /// MP3 recorder
    mp3_recorder: Arc<Mp3Recorder>,
    /// Opus recorder
    opus_recorder: Arc<OpusRecorder>,
    /// Active recording sessions
    sessions: Arc<RwLock<HashMap<MediaSessionId, Vec<RecordingSession>>>>,
    /// Default output directory
    output_dir: PathBuf,
}

impl RecordingManager {
    /// Create new recording manager
    pub fn new(output_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&output_dir)
            .map_err(|e| Error::io(format!("Failed to create recording directory: {}", e)))?;

        Ok(Self {
            wav_recorder: Arc::new(WavRecorder::new()),
            mp3_recorder: Arc::new(Mp3Recorder::new()),
            opus_recorder: Arc::new(OpusRecorder::new()),
            sessions: Arc::new(RwLock::new(HashMap::new())),
            output_dir,
        })
    }

    /// Start recording for a media session
    pub async fn start_recording(
        &self,
        media_session_id: MediaSessionId,
        format: AudioFormat,
        metadata: HashMap<String, String>,
    ) -> Result<RecordingHandle> {
        // Generate filename with timestamp
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let filename = format!("recording_{}_{}.{}",
            media_session_id,
            timestamp,
            format.extension()
        );
        let path = self.output_dir.join(filename);

        // Create recording configuration
        let config = RecordingConfig {
            format,
            channels: 1, // Mono for telephony
            sample_rate: 8000, // Standard telephony rate
            output_path: path,
            max_size_bytes: 1024 * 1024 * 1024, // 1GB limit
            max_duration_secs: 3600 * 4, // 4 hour limit
        };

        // Start recording with appropriate backend
        let handle = match format {
            AudioFormat::Wav => {
                self.wav_recorder.start_recording(config).await?
            }
            AudioFormat::Mp3 => {
                self.mp3_recorder.start_recording(config).await?
            }
            AudioFormat::Opus => {
                self.opus_recorder.start_recording(config).await?
            }
            _ => return Err(Error::unsupported_format(format)),
        };

        // Create session
        let session = RecordingSession {
            handle: handle.clone(),
            media_session_id: media_session_id.clone(),
            metadata,
        };

        // Store session
        self.sessions.write().await
            .entry(media_session_id)
            .or_insert_with(Vec::new)
            .push(session);

        Ok(handle)
    }

    /// Write audio to all recordings for a media session
    pub async fn write_audio(
        &self,
        media_session_id: &MediaSessionId,
        frame: &AudioFrame,
    ) -> Result<()> {
        let sessions = self.sessions.read().await;

        if let Some(recordings) = sessions.get(media_session_id) {
            for session in recordings {
                // Route to appropriate backend
                match session.handle.format {
                    AudioFormat::Wav => {
                        self.wav_recorder.write_audio(&session.handle, frame).await?;
                    }
                    AudioFormat::Mp3 => {
                        self.mp3_recorder.write_audio(&session.handle, frame).await?;
                    }
                    AudioFormat::Opus => {
                        self.opus_recorder.write_audio(&session.handle, frame).await?;
                    }
                    _ => {}
                }
            }
        }

        Ok(())
    }

    /// Stop all recordings for a media session
    pub async fn stop_recordings(
        &self,
        media_session_id: &MediaSessionId,
    ) -> Result<Vec<RecordingInfo>> {
        let mut sessions = self.sessions.write().await;
        let recordings = sessions.remove(media_session_id).unwrap_or_default();

        let mut results = Vec::new();

        for session in recordings {
            let info = match session.handle.format {
                AudioFormat::Wav => {
                    self.wav_recorder.stop_recording(session.handle).await?
                }
                AudioFormat::Mp3 => {
                    self.mp3_recorder.stop_recording(session.handle).await?
                }
                AudioFormat::Opus => {
                    self.opus_recorder.stop_recording(session.handle).await?
                }
                _ => continue,
            };

            results.push(info);
        }

        Ok(results)
    }
}
```

## Week 2-3: Announcement System Implementation

### 2.1 Announcement Engine Architecture

**File: `/crates/media-core/src/announcement/mod.rs`**
```rust
//! Announcement and prompt playback system
//!
//! Provides audio file playback, queueing, and IVR prompt management

pub mod source;
pub mod queue;
pub mod player;
pub mod cache;

pub use source::{AnnouncementSource, FileSource, MemorySource};
pub use queue::{AnnouncementQueue, QueueMode};
pub use player::{AnnouncementPlayer, PlayerState};
pub use cache::{PromptCache, CacheEntry};
```

### 2.2 Announcement Source Trait

**File: `/crates/media-core/src/announcement/source.rs`**
```rust
//! Announcement audio sources

use async_trait::async_trait;
use std::time::Duration;
use crate::types::AudioFrame;
use crate::error::Result;

/// Source of announcement audio
#[async_trait]
pub trait AnnouncementSource: Send + Sync {
    /// Get audio frame at specified position
    async fn get_audio(&self, position: Duration) -> Result<Option<AudioFrame>>;

    /// Total duration of the announcement
    fn duration(&self) -> Duration;

    /// Whether this announcement can be interrupted
    fn is_interruptible(&self) -> bool;

    /// Reset to beginning
    async fn reset(&mut self) -> Result<()>;

    /// Source identifier for caching
    fn source_id(&self) -> String;
}

/// File-based announcement source
pub struct FileSource {
    /// File path
    path: PathBuf,
    /// Cached audio data
    audio_data: Vec<i16>,
    /// Sample rate
    sample_rate: u32,
    /// Current position in samples
    position: usize,
    /// Interruptible flag
    interruptible: bool,
}

impl FileSource {
    /// Load announcement from file
    pub async fn load(path: PathBuf, interruptible: bool) -> Result<Self> {
        // Load audio file (WAV, MP3, etc.)
        let audio_data = load_audio_file(&path).await?;

        Ok(Self {
            path,
            audio_data,
            sample_rate: 8000,
            position: 0,
            interruptible,
        })
    }
}

#[async_trait]
impl AnnouncementSource for FileSource {
    async fn get_audio(&self, position: Duration) -> Result<Option<AudioFrame>> {
        let sample_position = (position.as_secs_f32() * self.sample_rate as f32) as usize;

        if sample_position >= self.audio_data.len() {
            return Ok(None); // End of audio
        }

        // Get next frame (20ms at 8kHz = 160 samples)
        let frame_size = 160;
        let end = std::cmp::min(sample_position + frame_size, self.audio_data.len());
        let samples = self.audio_data[sample_position..end].to_vec();

        if samples.is_empty() {
            return Ok(None);
        }

        // Pad with silence if needed
        let mut frame_samples = samples;
        frame_samples.resize(frame_size, 0);

        Ok(Some(AudioFrame {
            samples: frame_samples,
            sample_rate: self.sample_rate,
            channels: 1,
            duration: Duration::from_millis(20),
            timestamp: 0,
        }))
    }

    fn duration(&self) -> Duration {
        Duration::from_secs_f32(self.audio_data.len() as f32 / self.sample_rate as f32)
    }

    fn is_interruptible(&self) -> bool {
        self.interruptible
    }

    async fn reset(&mut self) -> Result<()> {
        self.position = 0;
        Ok(())
    }

    fn source_id(&self) -> String {
        format!("file:{}", self.path.display())
    }
}
```

### 2.3 Announcement Queue

**File: `/crates/media-core/src/announcement/queue.rs`**
```rust
//! Announcement queue management

use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::announcement::AnnouncementSource;
use crate::types::AudioFrame;
use crate::error::Result;

/// Queue mode for announcements
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QueueMode {
    /// Play announcements sequentially
    Sequential,
    /// Replace current with new
    Replace,
    /// Mix with current
    Mix,
}

/// Active announcement being played
struct ActiveAnnouncement {
    source: Arc<dyn AnnouncementSource>,
    position: Duration,
    started_at: Instant,
}

/// Announcement queue for managing playback
pub struct AnnouncementQueue {
    /// Queued announcements
    queue: Arc<RwLock<VecDeque<Arc<dyn AnnouncementSource>>>>,
    /// Currently playing announcement
    current: Arc<RwLock<Option<ActiveAnnouncement>>>,
    /// Queue mode
    mode: QueueMode,
    /// Playback state
    state: Arc<RwLock<PlaybackState>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum PlaybackState {
    Idle,
    Playing,
    Paused,
}

impl AnnouncementQueue {
    /// Create new announcement queue
    pub fn new(mode: QueueMode) -> Self {
        Self {
            queue: Arc::new(RwLock::new(VecDeque::new())),
            current: Arc::new(RwLock::new(None)),
            mode,
            state: Arc::new(RwLock::new(PlaybackState::Idle)),
        }
    }

    /// Add announcement to queue
    pub async fn enqueue(&self, source: Arc<dyn AnnouncementSource>) -> Result<()> {
        match self.mode {
            QueueMode::Sequential => {
                self.queue.write().await.push_back(source);
                self.try_start_next().await?;
            }
            QueueMode::Replace => {
                // Stop current and replace
                self.stop_current().await?;
                self.queue.write().await.clear();
                self.queue.write().await.push_back(source);
                self.try_start_next().await?;
            }
            QueueMode::Mix => {
                // Add to queue for mixing
                self.queue.write().await.push_back(source);
            }
        }

        Ok(())
    }

    /// Get next audio frame
    pub async fn get_next_frame(&self) -> Result<Option<AudioFrame>> {
        let state = *self.state.read().await;

        if state != PlaybackState::Playing {
            return Ok(None);
        }

        let mut current = self.current.write().await;

        if let Some(ref mut active) = *current {
            // Get audio at current position
            let frame = active.source.get_audio(active.position).await?;

            if let Some(ref f) = frame {
                // Advance position
                active.position += f.duration;
                Ok(Some(f.clone()))
            } else {
                // Announcement finished
                drop(current);
                self.announcement_finished().await?;
                Ok(None)
            }
        } else {
            // Try to start next announcement
            drop(current);
            self.try_start_next().await?;

            // Retry getting frame
            self.get_next_frame().await
        }
    }

    /// Handle DTMF interruption
    pub async fn handle_dtmf(&self, digit: char) -> Result<bool> {
        let current = self.current.read().await;

        if let Some(ref active) = *current {
            if active.source.is_interruptible() {
                // Stop current announcement
                drop(current);
                self.stop_current().await?;
                return Ok(true); // Interrupted
            }
        }

        Ok(false) // Not interrupted
    }

    /// Try to start next announcement in queue
    async fn try_start_next(&self) -> Result<()> {
        let current = self.current.read().await;

        if current.is_some() {
            return Ok(()); // Already playing
        }

        drop(current);

        let mut queue = self.queue.write().await;

        if let Some(source) = queue.pop_front() {
            let active = ActiveAnnouncement {
                source,
                position: Duration::ZERO,
                started_at: Instant::now(),
            };

            *self.current.write().await = Some(active);
            *self.state.write().await = PlaybackState::Playing;
        } else {
            *self.state.write().await = PlaybackState::Idle;
        }

        Ok(())
    }

    /// Stop current announcement
    async fn stop_current(&self) -> Result<()> {
        *self.current.write().await = None;

        if self.queue.read().await.is_empty() {
            *self.state.write().await = PlaybackState::Idle;
        }

        Ok(())
    }

    /// Handle announcement finished
    async fn announcement_finished(&self) -> Result<()> {
        self.stop_current().await?;
        self.try_start_next().await
    }
}
```

## Week 3: DTMF Framework Implementation

### 3.1 DTMF Detection and Collection

**File: `/crates/media-core/src/dtmf/mod.rs`**
```rust
//! DTMF detection, generation, and collection framework

pub mod detector;
pub mod generator;
pub mod collector;
pub mod rfc2833;

pub use detector::{DtmfDetector, DetectionMode};
pub use generator::{DtmfGenerator, Tone};
pub use collector::{DtmfCollector, CollectionConfig};
pub use rfc2833::{Rfc2833Decoder, Rfc2833Encoder};

/// DTMF event
#[derive(Debug, Clone)]
pub struct DtmfEvent {
    /// DTMF digit (0-9, *, #, A-D)
    pub digit: char,
    /// Duration in milliseconds
    pub duration_ms: u32,
    /// Volume level (0-63 for RFC2833)
    pub volume: u8,
    /// Detection method
    pub source: DtmfSource,
    /// Timestamp
    pub timestamp: std::time::Instant,
}

/// DTMF detection source
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DtmfSource {
    /// RFC 2833 RTP events
    Rfc2833,
    /// SIP INFO messages
    SipInfo,
    /// In-band audio detection
    InBand,
}
```

### 3.2 DTMF Collector

**File: `/crates/media-core/src/dtmf/collector.rs`**
```rust
//! DTMF digit collection for IVR systems

use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use crate::dtmf::DtmfEvent;
use crate::error::Result;

/// DTMF collection configuration
#[derive(Debug, Clone)]
pub struct CollectionConfig {
    /// Maximum digits to collect
    pub max_digits: usize,
    /// Minimum digits required
    pub min_digits: usize,
    /// Overall timeout
    pub timeout: Duration,
    /// Inter-digit timeout
    pub inter_digit_timeout: Duration,
    /// Terminator digit (e.g., '#')
    pub terminator: Option<char>,
    /// Clear on timeout
    pub clear_on_timeout: bool,
}

impl Default for CollectionConfig {
    fn default() -> Self {
        Self {
            max_digits: 10,
            min_digits: 1,
            timeout: Duration::from_secs(30),
            inter_digit_timeout: Duration::from_secs(5),
            terminator: Some('#'),
            clear_on_timeout: true,
        }
    }
}

/// DTMF digit collector
pub struct DtmfCollector {
    /// Collection configuration
    config: CollectionConfig,
    /// Collected digits
    buffer: Vec<char>,
    /// Start time
    started_at: Option<Instant>,
    /// Last digit time
    last_digit_at: Option<Instant>,
    /// Event sender
    event_tx: mpsc::UnboundedSender<CollectorEvent>,
}

/// Collector events
#[derive(Debug, Clone)]
pub enum CollectorEvent {
    /// Digit collected
    DigitCollected { digit: char, total: usize },
    /// Collection completed
    CollectionComplete { digits: String },
    /// Collection timeout
    Timeout { partial: String },
    /// Collection cancelled
    Cancelled,
}

impl DtmfCollector {
    /// Create new DTMF collector
    pub fn new(config: CollectionConfig) -> (Self, mpsc::UnboundedReceiver<CollectorEvent>) {
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        let collector = Self {
            config,
            buffer: Vec::new(),
            started_at: None,
            last_digit_at: None,
            event_tx,
        };

        (collector, event_rx)
    }

    /// Process DTMF event
    pub async fn process_dtmf(&mut self, event: DtmfEvent) -> Result<()> {
        let now = Instant::now();

        // Start timer on first digit
        if self.started_at.is_none() {
            self.started_at = Some(now);
        }

        // Check overall timeout
        if let Some(start) = self.started_at {
            if now.duration_since(start) > self.config.timeout {
                return self.handle_timeout().await;
            }
        }

        // Check inter-digit timeout
        if let Some(last) = self.last_digit_at {
            if now.duration_since(last) > self.config.inter_digit_timeout {
                return self.handle_timeout().await;
            }
        }

        // Add digit to buffer
        self.buffer.push(event.digit);
        self.last_digit_at = Some(now);

        // Send digit collected event
        let _ = self.event_tx.send(CollectorEvent::DigitCollected {
            digit: event.digit,
            total: self.buffer.len(),
        });

        // Check terminator
        if let Some(term) = self.config.terminator {
            if event.digit == term && self.buffer.len() >= self.config.min_digits {
                return self.complete_collection().await;
            }
        }

        // Check max digits
        if self.buffer.len() >= self.config.max_digits {
            return self.complete_collection().await;
        }

        Ok(())
    }

    /// Complete collection
    async fn complete_collection(&mut self) -> Result<()> {
        let digits: String = self.buffer.iter()
            .filter(|&&c| self.config.terminator != Some(c))
            .collect();

        let _ = self.event_tx.send(CollectorEvent::CollectionComplete {
            digits: digits.clone(),
        });

        self.reset();
        Ok(())
    }

    /// Handle timeout
    async fn handle_timeout(&mut self) -> Result<()> {
        let partial: String = self.buffer.iter().collect();

        let _ = self.event_tx.send(CollectorEvent::Timeout {
            partial: partial.clone(),
        });

        if self.config.clear_on_timeout {
            self.reset();
        }

        Ok(())
    }

    /// Reset collector
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.started_at = None;
        self.last_digit_at = None;
    }

    /// Get current buffer
    pub fn get_buffer(&self) -> String {
        self.buffer.iter().collect()
    }
}
```

### 3.3 RFC 2833 DTMF Support

**File: `/crates/media-core/src/dtmf/rfc2833.rs`**
```rust
//! RFC 2833 DTMF RTP events

use crate::dtmf::{DtmfEvent, DtmfSource};
use crate::error::Result;

/// RFC 2833 telephone event payload
#[derive(Debug, Clone)]
pub struct TelephoneEvent {
    /// Event code (0-15 for DTMF)
    pub event: u8,
    /// End of event flag
    pub end: bool,
    /// Volume (0-63)
    pub volume: u8,
    /// Duration in timestamp units
    pub duration: u16,
}

/// RFC 2833 DTMF decoder
pub struct Rfc2833Decoder {
    /// Current event being decoded
    current_event: Option<TelephoneEvent>,
    /// Sample rate for timestamp conversion
    sample_rate: u32,
}

impl Rfc2833Decoder {
    /// Create new RFC 2833 decoder
    pub fn new(sample_rate: u32) -> Self {
        Self {
            current_event: None,
            sample_rate,
        }
    }

    /// Decode RTP packet as telephone event
    pub fn decode(&mut self, payload: &[u8]) -> Result<Option<DtmfEvent>> {
        if payload.len() < 4 {
            return Ok(None);
        }

        let event = payload[0];
        let end = (payload[1] & 0x80) != 0;
        let volume = payload[1] & 0x3F;
        let duration = u16::from_be_bytes([payload[2], payload[3]]);

        // Map event code to DTMF digit
        let digit = match event {
            0..=9 => (b'0' + event) as char,
            10 => '*',
            11 => '#',
            12 => 'A',
            13 => 'B',
            14 => 'C',
            15 => 'D',
            _ => return Ok(None), // Not a DTMF event
        };

        let tel_event = TelephoneEvent {
            event,
            end,
            volume,
            duration,
        };

        // Check if this is end of event
        if end {
            if let Some(ref current) = self.current_event {
                if current.event == event {
                    // Event ended
                    let duration_ms = (duration as u64 * 1000) / self.sample_rate as u64;

                    let dtmf_event = DtmfEvent {
                        digit,
                        duration_ms: duration_ms as u32,
                        volume,
                        source: DtmfSource::Rfc2833,
                        timestamp: std::time::Instant::now(),
                    };

                    self.current_event = None;
                    return Ok(Some(dtmf_event));
                }
            }
        } else {
            // Ongoing event
            self.current_event = Some(tel_event);
        }

        Ok(None)
    }
}

/// RFC 2833 DTMF encoder
pub struct Rfc2833Encoder {
    /// Sample rate for timestamp conversion
    sample_rate: u32,
    /// Packet timestamp increment
    timestamp_increment: u32,
}

impl Rfc2833Encoder {
    /// Create new RFC 2833 encoder
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            timestamp_increment: sample_rate / 50, // 20ms packets
        }
    }

    /// Encode DTMF digit as RFC 2833 packets
    pub fn encode(&self, digit: char, duration_ms: u32, volume: u8) -> Result<Vec<Vec<u8>>> {
        // Map digit to event code
        let event = match digit {
            '0'..='9' => digit as u8 - b'0',
            '*' => 10,
            '#' => 11,
            'A' => 12,
            'B' => 13,
            'C' => 14,
            'D' => 15,
            _ => return Err(Error::invalid_dtmf(digit)),
        };

        let mut packets = Vec::new();
        let duration_samples = (duration_ms * self.sample_rate / 1000) as u16;
        let packet_count = (duration_ms / 20).max(3); // At least 3 packets

        // Generate packets for event duration
        for i in 0..packet_count {
            let end = i >= packet_count - 3; // Last 3 packets have end flag
            let duration = ((i + 1) * self.timestamp_increment) as u16;

            let mut payload = vec![0u8; 4];
            payload[0] = event;
            payload[1] = if end { 0x80 | volume } else { volume };
            payload[2] = (duration >> 8) as u8;
            payload[3] = duration as u8;

            packets.push(payload);
        }

        Ok(packets)
    }
}
```

## Week 4: Conference Enhancement

### 4.1 Conference Mixing Matrix

**File: `/crates/media-core/src/conference/mixer.rs`**
```rust
//! Conference audio mixing matrix

use std::collections::HashMap;
use crate::types::{AudioFrame, ParticipantId};
use crate::error::Result;

/// Audio mixing matrix for conferences
pub struct MixingMatrix {
    /// Participant audio buffers
    buffers: HashMap<ParticipantId, AudioBuffer>,
    /// Mixing configuration per participant
    mix_config: HashMap<ParticipantId, MixConfig>,
    /// Global conference settings
    settings: ConferenceSettings,
}

/// Participant mix configuration
#[derive(Debug, Clone)]
pub struct MixConfig {
    /// Output gain (0.0 to 2.0)
    pub output_gain: f32,
    /// Input gain (0.0 to 2.0)
    pub input_gain: f32,
    /// Muted flag
    pub is_muted: bool,
    /// Solo flag (only hear this participant)
    pub is_solo: bool,
    /// Excluded participants (don't mix these)
    pub exclude: Vec<ParticipantId>,
}

impl MixingMatrix {
    /// Create N-1 mix for participant (everyone except themselves)
    pub fn create_mix_for(
        &self,
        participant_id: &ParticipantId,
    ) -> Result<AudioFrame> {
        let config = self.mix_config.get(participant_id)
            .ok_or_else(|| Error::participant_not_found())?;

        if config.is_muted {
            // Return silence
            return Ok(AudioFrame::silence());
        }

        let mut mixed_samples = vec![0i32; 160]; // 20ms at 8kHz
        let mut mix_count = 0;

        // Mix other participants
        for (other_id, buffer) in &self.buffers {
            if other_id == participant_id {
                continue; // Don't mix self
            }

            if config.exclude.contains(other_id) {
                continue; // Excluded participant
            }

            let other_config = self.mix_config.get(other_id)
                .ok_or_else(|| Error::participant_not_found())?;

            if other_config.is_muted {
                continue; // Skip muted participants
            }

            if config.is_solo && !other_config.is_solo {
                continue; // Solo mode - only mix other solo participants
            }

            // Get audio from participant buffer
            if let Some(frame) = buffer.get_latest_frame() {
                for (i, &sample) in frame.samples.iter().enumerate() {
                    let scaled = (sample as f32 * other_config.output_gain) as i32;
                    mixed_samples[i] = mixed_samples[i].saturating_add(scaled);
                }
                mix_count += 1;
            }
        }

        // Normalize mixed audio
        let divisor = mix_count.max(1) as f32;
        let output_samples: Vec<i16> = mixed_samples.iter()
            .map(|&s| {
                let normalized = (s as f32 / divisor * config.output_gain) as i32;
                normalized.clamp(i16::MIN as i32, i16::MAX as i32) as i16
            })
            .collect();

        Ok(AudioFrame {
            samples: output_samples,
            sample_rate: 8000,
            channels: 1,
            duration: Duration::from_millis(20),
            timestamp: 0,
        })
    }

    /// Add participant audio to buffer
    pub fn add_participant_audio(
        &mut self,
        participant_id: ParticipantId,
        frame: AudioFrame,
    ) -> Result<()> {
        self.buffers
            .entry(participant_id)
            .or_insert_with(AudioBuffer::new)
            .add_frame(frame);

        Ok(())
    }

    /// Update participant configuration
    pub fn update_config(
        &mut self,
        participant_id: &ParticipantId,
        config: MixConfig,
    ) -> Result<()> {
        self.mix_config.insert(participant_id.clone(), config);
        Ok(())
    }
}
```

## Implementation Timeline

| Week | Component | Status | Dependencies |
|------|-----------|--------|--------------|
| 1 | Recording Backend | 🔴 Not Started | None |
| 1 | WAV Recorder | 🔴 Not Started | Recording Backend |
| 2 | Recording Manager | 🔴 Not Started | Recorders |
| 2 | Announcement Source | 🔴 Not Started | None |
| 3 | Announcement Queue | 🔴 Not Started | Source |
| 3 | DTMF Collector | 🔴 Not Started | None |
| 3 | RFC 2833 Support | 🔴 Not Started | DTMF Framework |
| 4 | Conference Mixer | 🔴 Not Started | None |

## Testing Strategy

### Unit Tests
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_wav_recording() {
        let recorder = WavRecorder::new();
        let config = RecordingConfig {
            format: AudioFormat::Wav,
            channels: 1,
            sample_rate: 8000,
            output_path: "/tmp/test.wav".into(),
            max_size_bytes: 0,
            max_duration_secs: 0,
        };

        let handle = recorder.start_recording(config).await.unwrap();

        // Write test frames
        for _ in 0..100 {
            let frame = AudioFrame::tone(440.0, 8000); // 440Hz tone
            recorder.write_audio(&handle, &frame).await.unwrap();
        }

        let info = recorder.stop_recording(handle).await.unwrap();
        assert_eq!(info.samples_written, 16000); // 100 frames * 160 samples
    }

    #[tokio::test]
    async fn test_announcement_queue() {
        let queue = AnnouncementQueue::new(QueueMode::Sequential);

        let source1 = Arc::new(FileSource::load("prompt1.wav", true).await.unwrap());
        let source2 = Arc::new(FileSource::load("prompt2.wav", false).await.unwrap());

        queue.enqueue(source1).await.unwrap();
        queue.enqueue(source2).await.unwrap();

        // Should play sequentially
        let frame1 = queue.get_next_frame().await.unwrap();
        assert!(frame1.is_some());

        // Test interruption
        let interrupted = queue.handle_dtmf('1').await.unwrap();
        assert!(interrupted); // First was interruptible

        // Second should not be interruptible
        let frame2 = queue.get_next_frame().await.unwrap();
        let interrupted = queue.handle_dtmf('2').await.unwrap();
        assert!(!interrupted);
    }

    #[test]
    fn test_dtmf_collection() {
        let config = CollectionConfig {
            max_digits: 4,
            min_digits: 1,
            terminator: Some('#'),
            ..Default::default()
        };

        let (mut collector, mut rx) = DtmfCollector::new(config);

        // Collect digits
        collector.process_dtmf(DtmfEvent {
            digit: '1',
            ..Default::default()
        }).await.unwrap();

        collector.process_dtmf(DtmfEvent {
            digit: '2',
            ..Default::default()
        }).await.unwrap();

        collector.process_dtmf(DtmfEvent {
            digit: '#',
            ..Default::default()
        }).await.unwrap();

        // Should complete with "12"
        if let Some(CollectorEvent::CollectionComplete { digits }) = rx.recv().await {
            assert_eq!(digits, "12");
        }
    }

    #[test]
    fn test_rfc2833_encoding() {
        let encoder = Rfc2833Encoder::new(8000);
        let packets = encoder.encode('5', 100, 10).unwrap();

        assert!(packets.len() >= 5); // 100ms = 5+ packets at 20ms each

        let decoder = Rfc2833Decoder::new(8000);
        for packet in packets {
            if let Some(event) = decoder.decode(&packet).unwrap() {
                assert_eq!(event.digit, '5');
                assert_eq!(event.volume, 10);
            }
        }
    }
}
```

## Integration with B2BUA

### Usage Example
```rust
use rvoip_media_core::recording::RecordingManager;
use rvoip_media_core::announcement::AnnouncementPlayer;
use rvoip_media_core::dtmf::DtmfCollector;

// In b2bua-core
impl MediaProcessor for B2buaMediaProcessor {
    async fn start_recording(
        &self,
        bridge_id: &MediaBridgeId,
        config: RecordingConfig,
    ) -> Result<RecordingId> {
        // Use media-core recording manager
        let handle = self.recording_manager
            .start_recording(bridge_id.into(), config.format, metadata)
            .await?;

        Ok(RecordingId(handle.id.to_string()))
    }
}
```

## Performance Considerations

### Memory Usage
- Recording buffer: 1MB per active recording
- Announcement cache: 100MB for prompt storage
- DTMF buffer: 1KB per collector
- Conference buffer: 10KB per participant

### CPU Usage
- WAV recording: <1% per stream
- MP3 encoding: 5% per stream
- DTMF detection: <1% per stream
- Conference mixing: 2% per 10 participants

### Optimization Opportunities
1. Zero-copy audio paths where possible
2. SIMD for mixing operations
3. Lock-free buffers for real-time audio
4. Prompt pre-loading and caching
5. Efficient file I/O with async operations

## Risk Mitigation

| Risk | Impact | Mitigation |
|------|--------|------------|
| File I/O blocking audio | High | Use dedicated thread pool |
| Memory leaks in long recordings | Medium | Implement size limits and rotation |
| DTMF detection accuracy | Medium | Multiple detection algorithms |
| Conference scaling | High | Optimize mixing algorithms |

## Conclusion

This implementation plan provides all missing media capabilities required for B2BUA production use cases:

1. **Recording**: Complete file-based recording system
2. **Announcements**: Full IVR prompt playback
3. **DTMF**: Collection and detection framework
4. **Conference**: Enhanced mixing and participant management

The modular design allows incremental implementation while maintaining compatibility with the b2bua-core trait interfaces. All components are designed for high performance and production reliability.