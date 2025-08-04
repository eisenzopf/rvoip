//! Audio pipeline
//!
//! This module provides high-level audio streaming pipelines that integrate
//! device management, format conversion, and codec processing.

use crate::types::{AudioFormat, AudioFrame, AudioStreamConfig};
use crate::device::{AudioDevice, AudioDeviceManager};
use crate::types::AudioDirection;
use crate::format::{FormatConverter, AudioFrameBuffer};
use crate::error::{AudioError, AudioResult};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock, oneshot};
use tokio::time::{Duration, Interval, interval};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// Audio pipeline for streaming between devices and RTP
pub struct AudioPipeline {
    /// Pipeline configuration
    config: AudioStreamConfig,
    /// Input device (microphone)
    input_device: Option<Arc<dyn AudioDevice>>,
    /// Output device (speakers)
    output_device: Option<Arc<dyn AudioDevice>>,
    /// Format converter for input
    input_converter: Option<FormatConverter>,
    /// Format converter for output
    output_converter: Option<FormatConverter>,
    /// Audio frame buffer for input
    input_buffer: AudioFrameBuffer,
    /// Audio frame buffer for output
    output_buffer: AudioFrameBuffer,
    /// Pipeline state
    state: Arc<RwLock<PipelineState>>,
    /// Input frame sender
    input_frame_tx: mpsc::Sender<AudioFrame>,
    /// Input frame receiver
    input_frame_rx: Option<mpsc::Receiver<AudioFrame>>,
    /// Output frame sender
    output_frame_tx: mpsc::Sender<AudioFrame>,
    /// Output frame receiver  
    output_frame_rx: Option<mpsc::Receiver<AudioFrame>>,
    /// Number of frames sent for playback
    frames_sent: Arc<AtomicU64>,
    /// Number of frames actually played
    frames_played: Arc<AtomicU64>,
}

/// Pipeline operational state
#[derive(Debug, Clone, PartialEq)]
pub enum PipelineState {
    /// Pipeline is stopped
    Stopped,
    /// Pipeline is starting up
    Starting,
    /// Pipeline is running
    Running,
    /// Pipeline is stopping
    Stopping,
    /// Pipeline encountered an error
    Error(String),
}

/// Audio pipeline builder for configuration
pub struct AudioPipelineBuilder {
    config: AudioStreamConfig,
    input_device: Option<Arc<dyn AudioDevice>>,
    output_device: Option<Arc<dyn AudioDevice>>,
    device_manager: Option<AudioDeviceManager>,
}

impl AudioPipelineBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            config: AudioStreamConfig::voip_basic(),
            input_device: None,
            output_device: None,
            device_manager: None,
        }
    }

    /// Set input format
    pub fn input_format(mut self, format: AudioFormat) -> Self {
        self.config.input_format = format;
        self
    }

    /// Set output format
    pub fn output_format(mut self, format: AudioFormat) -> Self {
        self.config.output_format = format;
        self
    }

    /// Set codec by name
    pub fn codec_name(mut self, codec_name: impl Into<String>) -> Self {
        self.config.codec_name = codec_name.into();
        self
    }

    /// Set input device
    pub fn input_device(mut self, device: Arc<dyn AudioDevice>) -> Self {
        self.input_device = Some(device);
        self
    }

    /// Set output device
    pub fn output_device(mut self, device: Arc<dyn AudioDevice>) -> Self {
        self.output_device = Some(device);
        self
    }

    /// Set device manager for automatic device selection
    pub fn device_manager(mut self, manager: AudioDeviceManager) -> Self {
        self.device_manager = Some(manager);
        self
    }

    /// Enable audio processing features
    pub fn enable_processing(mut self, enable: bool) -> Self {
        self.config.enable_aec = enable;
        self.config.enable_agc = enable;
        self.config.enable_noise_suppression = enable;
        self
    }

    /// Enable echo cancellation
    pub fn enable_aec(mut self, enable: bool) -> Self {
        self.config.enable_aec = enable;
        self
    }

    /// Enable automatic gain control
    pub fn enable_agc(mut self, enable: bool) -> Self {
        self.config.enable_agc = enable;
        self
    }

    /// Set buffer size
    pub fn buffer_size_ms(mut self, size_ms: u32) -> Self {
        self.config.buffer_size_ms = size_ms;
        self
    }

    /// Build the pipeline
    pub async fn build(mut self) -> AudioResult<AudioPipeline> {
        // Auto-select devices if not provided
        if self.input_device.is_none() || self.output_device.is_none() {
            if let Some(ref manager) = self.device_manager {
                if self.input_device.is_none() {
                    self.input_device = Some(manager.get_default_device(AudioDirection::Input).await?);
                }
                if self.output_device.is_none() {
                    self.output_device = Some(manager.get_default_device(AudioDirection::Output).await?);
                }
            }
        }

        let input_device = self.input_device.ok_or_else(|| AudioError::ConfigurationError {
            component: "pipeline".to_string(),
            reason: "No input device specified".to_string(),
        })?;

        let output_device = self.output_device.ok_or_else(|| AudioError::ConfigurationError {
            component: "pipeline".to_string(),
            reason: "No output device specified".to_string(),
        })?;

        // Create format converters if needed
        let input_converter = if input_device.info().best_voip_format().is_compatible_with(&self.config.input_format) {
            None
        } else {
            Some(FormatConverter::new(
                input_device.info().best_voip_format(),
                self.config.input_format.clone(),
            )?)
        };

        let output_converter = if self.config.output_format.is_compatible_with(&output_device.info().best_voip_format()) {
            None
        } else {
            Some(FormatConverter::new(
                self.config.output_format.clone(),
                output_device.info().best_voip_format(),
            )?)
        };

        // Create audio buffers
        let buffer_frames = (self.config.buffer_size_ms / self.config.input_format.frame_size_ms) as usize;
        let input_buffer = AudioFrameBuffer::new(buffer_frames, self.config.input_format.clone());
        let output_buffer = AudioFrameBuffer::new(buffer_frames, self.config.output_format.clone());

        // Create channels for audio streaming
        let (input_frame_tx, input_frame_rx) = mpsc::channel(buffer_frames);
        let (output_frame_tx, output_frame_rx) = mpsc::channel(buffer_frames);

        Ok(AudioPipeline {
            config: self.config,
            input_device: Some(input_device),
            output_device: Some(output_device),
            input_converter,
            output_converter,
            input_buffer,
            output_buffer,
            state: Arc::new(RwLock::new(PipelineState::Stopped)),
            input_frame_tx,
            input_frame_rx: Some(input_frame_rx),
            output_frame_tx,
            output_frame_rx: Some(output_frame_rx),
            frames_sent: Arc::new(AtomicU64::new(0)),
            frames_played: Arc::new(AtomicU64::new(0)),
        })
    }
}

impl AudioPipeline {
    /// Create a pipeline builder
    pub fn builder() -> AudioPipelineBuilder {
        AudioPipelineBuilder::new()
    }

    /// Start the audio pipeline
    pub async fn start(&mut self) -> AudioResult<()> {
        let mut state = self.state.write().await;
        
        match *state {
            PipelineState::Running => return Ok(()),
            PipelineState::Starting => {
                return Err(AudioError::PipelineError {
                    stage: "start".to_string(),
                    reason: "Pipeline is already starting".to_string(),
                });
            }
            _ => {}
        }

        *state = PipelineState::Starting;
        drop(state);

        // Reset frame counters
        self.frames_sent.store(0, Ordering::SeqCst);
        self.frames_played.store(0, Ordering::SeqCst);

        // Start input processing task
        if let Some(ref input_device) = self.input_device {
            let device = input_device.clone();
            let tx = self.input_frame_tx.clone();
            let format = self.config.input_format.clone();
            let state = self.state.clone();

            tokio::task::spawn_blocking(move || {
                let rt = tokio::runtime::Handle::current();
                rt.block_on(async move {
                    Self::input_capture_task(device, tx, format, state).await;
                });
            });
        }

        // Start output processing task
        if let Some(ref output_device) = self.output_device {
            let device = output_device.clone();
            let rx = self.output_frame_rx.take().ok_or_else(|| AudioError::PipelineError {
                stage: "start".to_string(),
                reason: "Output receiver already taken".to_string(),
            })?;
            let state = self.state.clone();
            let frames_played = self.frames_played.clone();

            tokio::task::spawn_blocking(move || {
                let rt = tokio::runtime::Handle::current();
                rt.block_on(async move {
                    Self::output_playback_task(device, rx, state, frames_played).await;
                });
            });
        }

        let mut state = self.state.write().await;
        *state = PipelineState::Running;

        Ok(())
    }

    /// Stop the audio pipeline
    pub async fn stop(&mut self) -> AudioResult<()> {
        let mut state = self.state.write().await;
        *state = PipelineState::Stopping;
        drop(state);

        // TODO: Signal tasks to stop and wait for them

        let mut state = self.state.write().await;
        *state = PipelineState::Stopped;

        Ok(())
    }

    /// Capture an audio frame from the input device
    pub async fn capture_frame(&mut self) -> AudioResult<AudioFrame> {
        if let Some(mut rx) = self.input_frame_rx.take() {
            match rx.recv().await {
                Some(frame) => {
                    self.input_frame_rx = Some(rx);
                    Ok(frame)
                }
                None => Err(AudioError::PipelineError {
                    stage: "capture".to_string(),
                    reason: "Input channel closed".to_string(),
                }),
            }
        } else {
            Err(AudioError::PipelineError {
                stage: "capture".to_string(),
                reason: "Input receiver not available".to_string(),
            })
        }
    }
    
    /// Play an audio frame to the output device
    pub async fn play_frame(&mut self, frame: AudioFrame) -> AudioResult<()> {
        self.output_frame_tx.send(frame).await
            .map_err(|_| AudioError::PipelineError {
                stage: "playback".to_string(),
                reason: "Output channel closed".to_string(),
            })?;
        self.frames_sent.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    
    /// Wait for all queued frames to finish playing
    /// 
    /// Note: This method tracks when frames start playing and waits for the audio
    /// duration plus a buffer. Due to CPAL's internal buffering, we cannot track
    /// exactly when samples finish playing. A fully accurate implementation would
    /// require low-level audio driver integration.
    pub async fn wait_for_playback_complete(&self) -> AudioResult<()> {
        let sent = self.frames_sent.load(Ordering::SeqCst);
        if sent == 0 {
            return Ok(()); // Nothing to wait for
        }
        
        // Record when we started waiting
        let wait_start = tokio::time::Instant::now();
        
        // First, wait for the first frame to start playing
        let mut attempts = 0;
        let max_attempts = 500; // 5 seconds with 10ms intervals
        
        while self.frames_played.load(Ordering::SeqCst) == 0 {
            if attempts >= max_attempts {
                return Err(AudioError::PipelineError {
                    stage: "wait_playback".to_string(),
                    reason: "Timeout waiting for playback to start".to_string(),
                });
            }
            
            tokio::time::sleep(Duration::from_millis(10)).await;
            attempts += 1;
        }
        
        // Calculate total audio duration from when first frame started playing
        let frame_duration_ms = self.config.output_format.frame_size_ms as u64;
        let total_audio_duration_ms = sent * frame_duration_ms;
        
        // Add buffer time for CPAL's internal buffering and OS latency
        let buffer_time_ms = 500;
        let total_wait_from_start = Duration::from_millis(total_audio_duration_ms + buffer_time_ms);
        
        // Calculate remaining wait time
        let elapsed = wait_start.elapsed();
        if elapsed < total_wait_from_start {
            let remaining = total_wait_from_start - elapsed;
            eprintln!("⏱️ Waiting {:.1}s for audio to finish playing ({} frames)", 
                remaining.as_secs_f32(), sent);
            tokio::time::sleep(remaining).await;
        }
        
        Ok(())
    }

    /// Send an audio frame to the output device for playback
    pub async fn playback_frame(&mut self, frame: AudioFrame) -> AudioResult<()> {
        self.output_frame_tx.send(frame).await.map_err(|_| AudioError::PipelineError {
            stage: "playback".to_string(),
            reason: "Output channel closed".to_string(),
        })
    }

    /// Get current pipeline state
    pub async fn get_state(&self) -> PipelineState {
        self.state.read().await.clone()
    }

    /// Get pipeline configuration
    pub fn get_config(&self) -> &AudioStreamConfig {
        &self.config
    }

    /// Update pipeline configuration
    pub async fn update_config(&mut self, config: AudioStreamConfig) -> AudioResult<()> {
        let state = self.get_state().await;
        if state == PipelineState::Running {
            return Err(AudioError::PipelineError {
                stage: "config_update".to_string(),
                reason: "Cannot update config while pipeline is running".to_string(),
            });
        }

        self.config = config;
        Ok(())
    }

    /// Set codec for the pipeline
    pub async fn set_codec_name(&mut self, codec_name: impl Into<String>) -> AudioResult<()> {
        self.config.codec_name = codec_name.into();
        // TODO: Reconfigure codec processing
        Ok(())
    }

    /// Input capture task with real audio device support
    async fn input_capture_task(
        device: Arc<dyn AudioDevice>,
        tx: mpsc::Sender<AudioFrame>,
        format: AudioFormat,
        state: Arc<RwLock<PipelineState>>,
    ) {
        // Check if this is a test audio device
        #[cfg(feature = "test-audio")]
        {
            if let Some(test_device) = device.as_any().downcast_ref::<crate::device::test_audio::TestAudioDevice>() {
                eprintln!("🧪 Starting TEST audio capture from: {}", device.info().name);
                
                // Ensure device is started
                test_device.start();
                
                let mut frame_count = 0u64;
                
                loop {
                    // Check if we should stop
                    {
                        let current_state = state.read().await;
                        if *current_state == PipelineState::Stopping || *current_state == PipelineState::Stopped {
                            break;
                        }
                    }
                    
                    // Try to read a frame from the test device
                    // This will wait for proper timing and return silence if no data
                    if let Some(frame) = test_device.read_frame().await {
                        frame_count += 1;
                        if frame_count <= 5 || frame_count % 100 == 0 {
                            eprintln!("🧪 Read test audio frame #{}: {} samples", frame_count, frame.samples.len());
                        }
                        
                        if tx.send(frame).await.is_err() {
                            break; // Pipeline channel closed
                        }
                    } else {
                        // Device stopped or error
                        break;
                    }
                }
                
                // Stop the device
                test_device.stop();
                eprintln!("🛑 Test audio capture stopped after {} frames", frame_count);
                return;
            }
        }
        
        #[cfg(feature = "device-cpal")]
        {
            // Try to use real CPAL audio capture
            if let Some(cpal_device) = device.as_any().downcast_ref::<crate::device::cpal_backend::CpalAudioDevice>() {
                eprintln!("🎤 Starting REAL audio capture from: {}", device.info().name);
                
                // Create a channel for the CPAL stream to send frames
                let (stream_tx, mut stream_rx) = mpsc::channel::<AudioFrame>(100);
                
                // Create the CPAL capture stream
                match crate::device::cpal_stream::create_capture_stream(
                    cpal_device.cpal_device(),
                    format.clone(),
                    stream_tx,
                ) {
                    Ok(stream) => {
                        eprintln!("✅ CPAL audio capture stream started successfully!");
                        
                        // Keep stream alive in a separate variable
                        let _stream = stream;
                        
                        // Keep the stream alive and forward frames
                        let mut frame_count = 0u64;
                        loop {
                            // Check if we should stop
                            {
                                let current_state = state.read().await;
                                if *current_state == PipelineState::Stopping || *current_state == PipelineState::Stopped {
                                    break;
                                }
                            }
                            
                            // Forward frames from CPAL stream to pipeline
                            match stream_rx.recv().await {
                                Some(frame) => {
                                    frame_count += 1;
                                    if frame_count <= 5 || frame_count % 100 == 0 {
                                        eprintln!("🎤 Captured real audio frame #{}", frame_count);
                                    }
                                    
                                    if tx.send(frame).await.is_err() {
                                        break; // Pipeline channel closed
                                    }
                                }
                                None => {
                                    eprintln!("⚠️ CPAL stream channel closed");
                                    break;
                                }
                            }
                        }
                        
                        // Stream will be dropped here, stopping capture
                        eprintln!("🛑 Real audio capture stopped after {} frames", frame_count);
                        return;
                    }
                    Err(e) => {
                        eprintln!("❌ Failed to create CPAL capture stream: {}", e);
                        eprintln!("⚠️ Falling back to test tone generation");
                    }
                }
            }
        }
        
        // Fallback: Generate test tone if CPAL is not available or fails
        eprintln!("⚠️ Using test tone generation (no real audio capture)");
        
        let mut interval = interval(Duration::from_millis(format.frame_size_ms as u64));
        let mut timestamp = 0u32;
        let mut frame_count = 0u64;

        loop {
            // Check if we should stop
            {
                let current_state = state.read().await;
                if *current_state == PipelineState::Stopping || *current_state == PipelineState::Stopped {
                    break;
                }
            }

            interval.tick().await;

            // Generate a test tone
            let samples = if frame_count < 100 {
                // Generate a 440Hz test tone for first 100 frames (2 seconds at 20ms frames)
                let samples_per_frame = format.samples_per_frame();
                let mut samples = Vec::with_capacity(samples_per_frame);
                for i in 0..samples_per_frame {
                    let t = (timestamp + i as u32) as f32 / format.sample_rate as f32;
                    let sample = (t * 440.0 * 2.0 * std::f32::consts::PI).sin() * 0.3 * i16::MAX as f32;
                    samples.push(sample as i16);
                }
                samples
            } else {
                // Silent frames after the test tone
                vec![0i16; format.samples_per_frame()]
            };
            
            let frame = AudioFrame::new(samples, format.clone(), timestamp);

            if frame_count < 5 {
                eprintln!("📡 Generated test tone frame #{}", frame_count);
            }

            if tx.send(frame).await.is_err() {
                break; // Channel closed
            }

            timestamp = timestamp.wrapping_add(format.samples_per_frame() as u32);
            frame_count += 1;
        }
        
        eprintln!("🛑 Test tone generation stopped after {} frames", frame_count);
    }

    /// Output playback task with real audio device support
    async fn output_playback_task(
        device: Arc<dyn AudioDevice>,
        mut rx: mpsc::Receiver<AudioFrame>,
        state: Arc<RwLock<PipelineState>>,
        frames_played: Arc<AtomicU64>,
    ) {
        // Check if this is a test audio device
        #[cfg(feature = "test-audio")]
        {
            if let Some(test_device) = device.as_any().downcast_ref::<crate::device::test_audio::TestAudioDevice>() {
                eprintln!("🧪 Starting TEST audio playback to: {}", device.info().name);
                
                // Ensure device is started
                test_device.start();
                
                let mut frame_count = 0u64;
                let frame_timeout = Duration::from_millis(100); // Timeout for receiving frames
                
                loop {
                    // Check if we should stop
                    {
                        let current_state = state.read().await;
                        if *current_state == PipelineState::Stopping || *current_state == PipelineState::Stopped {
                            break;
                        }
                    }
                    
                    // Receive frame from pipeline with timeout
                    match tokio::time::timeout(frame_timeout, rx.recv()).await {
                        Ok(Some(frame)) => {
                            frame_count += 1;
                            if frame_count <= 5 || frame_count % 100 == 0 {
                                eprintln!("🧪 Writing test audio frame #{}: {} samples", frame_count, frame.samples.len());
                            }
                            
                            // Write to test device
                            if let Err(e) = test_device.write_frame(frame).await {
                                eprintln!("❌ Failed to write to test device: {}", e);
                            }
                            
                            frames_played.fetch_add(1, Ordering::SeqCst);
                        }
                        Ok(None) => {
                            // Channel closed
                            break;
                        }
                        Err(_) => {
                            // Timeout - continue to check if we should stop
                            continue;
                        }
                    }
                }
                
                // Stop the device
                test_device.stop();
                eprintln!("🛑 Test audio playback stopped after {} frames", frame_count);
                return;
            }
        }
        
        #[cfg(feature = "device-cpal")]
        {
            // Try to use real CPAL audio playback
            if let Some(cpal_device) = device.as_any().downcast_ref::<crate::device::cpal_backend::CpalAudioDevice>() {
                eprintln!("🔊 Starting REAL audio playback to: {}", device.info().name);
                
                // Get the format from the first frame
                if let Some(first_frame) = rx.recv().await {
                    let format = first_frame.format.clone();
                    
                    // Create a channel to send frames to CPAL stream
                    let (stream_tx, stream_rx) = mpsc::channel::<AudioFrame>(100);
                    
                    // Send the first frame
                    let _ = stream_tx.send(first_frame).await;
                    frames_played.fetch_add(1, Ordering::SeqCst);
                    
                    // Create the CPAL playback stream
                    match crate::device::cpal_stream::create_playback_stream(
                        cpal_device.cpal_device(),
                        format,
                        stream_rx,
                    ) {
                        Ok(stream) => {
                            eprintln!("✅ CPAL audio playback stream started successfully!");
                            
                            let mut frame_count = 1u64; // Already sent first frame
                            
                            // Forward frames to CPAL stream
                            loop {
                                // Check if we should stop
                                {
                                    let current_state = state.read().await;
                                    if *current_state == PipelineState::Stopping || *current_state == PipelineState::Stopped {
                                        break;
                                    }
                                }
                                
                                match rx.recv().await {
                                    Some(frame) => {
                                        frame_count += 1;
                                        
                                        if frame_count <= 5 || frame_count % 100 == 0 {
                                            let rms = frame.rms_level();
                                            eprintln!("🔊 Playing real audio frame #{}: RMS: {:.3}", 
                                                frame_count, 
                                                rms / i16::MAX as f32
                                            );
                                        }
                                        
                                        // Send to CPAL stream
                                        if stream_tx.send(frame).await.is_err() {
                                            eprintln!("⚠️ CPAL playback channel full or closed");
                                            break;
                                        }
                                        
                                        // Increment frames played counter
                                        frames_played.fetch_add(1, Ordering::SeqCst);
                                    }
                                    None => {
                                        eprintln!("🔊 Pipeline playback channel closed");
                                        break;
                                    }
                                }
                            }
                            
                            // Drop the stream to stop playback
                            drop(stream);
                            eprintln!("🛑 Real audio playback stopped after {} frames", frame_count);
                            return;
                        }
                        Err(e) => {
                            eprintln!("❌ Failed to create CPAL playback stream: {}", e);
                            eprintln!("⚠️ Falling back to discarding audio frames");
                        }
                    }
                } else {
                    eprintln!("⚠️ No audio frames received for playback");
                    return;
                }
            }
        }
        
        // Fallback: Just discard frames if CPAL is not available
        eprintln!("⚠️ Discarding audio frames (no real playback)");
        
        let mut frame_count = 0u64;
        
        loop {
            // Check if we should stop
            {
                let current_state = state.read().await;
                if *current_state == PipelineState::Stopping || *current_state == PipelineState::Stopped {
                    break;
                }
            }

            match rx.recv().await {
                Some(frame) => {
                    frame_count += 1;
                    
                    if frame_count <= 5 || frame_count % 100 == 0 {
                        let rms = frame.rms_level();
                        eprintln!("🔊 Discarded audio frame #{}: {} samples, RMS: {:.3}", 
                            frame_count, 
                            frame.samples.len(),
                            rms / i16::MAX as f32
                        );
                    }
                    
                    // Even in fallback mode, mark frame as played
                    frames_played.fetch_add(1, Ordering::SeqCst);
                }
                None => {
                    eprintln!("🔊 Playback channel closed");
                    break;
                }
            }
        }
        
        eprintln!("🛑 Frame discard stopped after {} frames", frame_count);
    }

    /// Get pipeline statistics
    pub async fn get_stats(&self) -> PipelineStats {
        PipelineStats {
            state: self.get_state().await,
            config: self.config.clone(),
            input_buffer_stats: self.input_buffer.get_stats(),
            output_buffer_stats: self.output_buffer.get_stats(),
            input_converter_active: self.input_converter.is_some(),
            output_converter_active: self.output_converter.is_some(),
        }
    }
}

/// Pipeline statistics
#[derive(Debug, Clone)]
pub struct PipelineStats {
    /// Current pipeline state
    pub state: PipelineState,
    /// Pipeline configuration
    pub config: AudioStreamConfig,
    /// Input buffer statistics
    pub input_buffer_stats: crate::format::AudioFrameBufferStats,
    /// Output buffer statistics
    pub output_buffer_stats: crate::format::AudioFrameBufferStats,
    /// Whether input converter is active
    pub input_converter_active: bool,
    /// Whether output converter is active
    pub output_converter_active: bool,
}

/// Pipeline manager for handling multiple pipelines
pub struct PipelineManager {
    /// Active pipelines
    pipelines: HashMap<String, AudioPipeline>,
    /// Device manager
    device_manager: AudioDeviceManager,
}

impl PipelineManager {
    /// Create a new pipeline manager
    pub async fn new() -> AudioResult<Self> {
        let device_manager = AudioDeviceManager::new().await?;
        
        Ok(Self {
            pipelines: HashMap::new(),
            device_manager,
        })
    }

    /// Create a new pipeline with given ID
    pub async fn create_pipeline(
        &mut self,
        id: String,
        config: AudioStreamConfig,
    ) -> AudioResult<()> {
        let pipeline = AudioPipeline::builder()
            .input_format(config.input_format.clone())
            .output_format(config.output_format.clone())
            .codec_name(config.codec_name.clone())
            .device_manager(self.device_manager.clone())
            .build()
            .await?;

        self.pipelines.insert(id, pipeline);
        Ok(())
    }

    /// Start a pipeline
    pub async fn start_pipeline(&mut self, id: &str) -> AudioResult<()> {
        let pipeline = self.pipelines.get_mut(id).ok_or_else(|| AudioError::PipelineError {
            stage: "start".to_string(),
            reason: format!("Pipeline '{}' not found", id),
        })?;

        pipeline.start().await
    }

    /// Stop a pipeline
    pub async fn stop_pipeline(&mut self, id: &str) -> AudioResult<()> {
        let pipeline = self.pipelines.get_mut(id).ok_or_else(|| AudioError::PipelineError {
            stage: "stop".to_string(),
            reason: format!("Pipeline '{}' not found", id),
        })?;

        pipeline.stop().await
    }

    /// Remove a pipeline
    pub async fn remove_pipeline(&mut self, id: &str) -> AudioResult<()> {
        if let Some(mut pipeline) = self.pipelines.remove(id) {
            pipeline.stop().await?;
        }
        Ok(())
    }

    /// Get pipeline statistics
    pub async fn get_pipeline_stats(&self, id: &str) -> AudioResult<PipelineStats> {
        let pipeline = self.pipelines.get(id).ok_or_else(|| AudioError::PipelineError {
            stage: "stats".to_string(),
            reason: format!("Pipeline '{}' not found", id),
        })?;

        Ok(pipeline.get_stats().await)
    }

    /// List all pipeline IDs
    pub fn list_pipelines(&self) -> Vec<String> {
        self.pipelines.keys().cloned().collect()
    }
}

impl Default for AudioPipelineBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::AudioDeviceManager;

    #[tokio::test]
    async fn test_pipeline_builder() {
        let input_format = AudioFormat::pcm_8khz_mono();
        let output_format = AudioFormat::pcm_16khz_mono();
        let device_manager = AudioDeviceManager::new().await.unwrap();

        let pipeline = AudioPipeline::builder()
            .input_format(input_format)
            .output_format(output_format)
            .device_manager(device_manager)
            .build()
            .await;

        assert!(pipeline.is_ok());
    }

    #[tokio::test]
    async fn test_pipeline_with_device_manager() {
        let device_manager = AudioDeviceManager::new().await.unwrap();
        
        let pipeline = AudioPipeline::builder()
            .input_format(AudioFormat::pcm_8khz_mono())
            .output_format(AudioFormat::pcm_16khz_mono())
            .device_manager(device_manager)
            .build()
            .await;

        assert!(pipeline.is_ok());
    }

    #[tokio::test]
    async fn test_pipeline_start_stop() {
        let device_manager = AudioDeviceManager::new().await.unwrap();
        
        let mut pipeline = AudioPipeline::builder()
            .input_format(AudioFormat::pcm_8khz_mono())
            .output_format(AudioFormat::pcm_8khz_mono())
            .device_manager(device_manager)
            .build()
            .await
            .unwrap();

        // Initially stopped
        assert_eq!(pipeline.get_state().await, PipelineState::Stopped);

        // Start pipeline
        let start_result = pipeline.start().await;
        assert!(start_result.is_ok());

        // Should be running
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(pipeline.get_state().await, PipelineState::Running);

        // Stop pipeline
        let stop_result = pipeline.stop().await;
        assert!(stop_result.is_ok());
        assert_eq!(pipeline.get_state().await, PipelineState::Stopped);
    }

    #[tokio::test]
    async fn test_pipeline_manager() {
        let mut manager = PipelineManager::new().await.unwrap();
        
        let config = AudioStreamConfig::voip_basic();
        
        // Create pipeline
        let create_result = manager.create_pipeline("test_pipeline".to_string(), config).await;
        assert!(create_result.is_ok());
        
        // List pipelines
        let pipelines = manager.list_pipelines();
        assert!(pipelines.contains(&"test_pipeline".to_string()));
        
        // Start pipeline
        let start_result = manager.start_pipeline("test_pipeline").await;
        assert!(start_result.is_ok());
        
        // Stop pipeline
        let stop_result = manager.stop_pipeline("test_pipeline").await;
        assert!(stop_result.is_ok());
        
        // Remove pipeline
        let remove_result = manager.remove_pipeline("test_pipeline").await;
        assert!(remove_result.is_ok());
        
        // Should be empty now
        assert!(manager.list_pipelines().is_empty());
    }

    #[tokio::test]
    async fn test_pipeline_configuration() {
        let device_manager = AudioDeviceManager::new().await.unwrap();
        let mut pipeline = AudioPipeline::builder()
            .input_format(AudioFormat::pcm_8khz_mono())
            .output_format(AudioFormat::pcm_16khz_mono())
            .enable_processing(true)
            .buffer_size_ms(50)
            .device_manager(device_manager)
            .build()
            .await
            .unwrap();

        let config = pipeline.get_config();
        assert!(config.enable_aec);
        assert!(config.enable_agc);
        assert!(config.enable_noise_suppression);
        assert_eq!(config.buffer_size_ms, 50);

        // Update configuration
        let new_config = AudioStreamConfig::voip_high_quality();
        let update_result = pipeline.update_config(new_config).await;
        assert!(update_result.is_ok());
    }
} 