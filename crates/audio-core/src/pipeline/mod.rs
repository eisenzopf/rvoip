//! Audio pipeline
//!
//! This module provides high-level audio streaming pipelines that integrate
//! device management, format conversion, and codec processing.

use crate::types::{AudioFormat, AudioFrame, AudioStreamConfig, AudioCodec};
use crate::device::{AudioDevice, AudioDeviceManager};
use crate::types::AudioDirection;
use crate::format::{FormatConverter, AudioFrameBuffer};
use crate::error::{AudioError, AudioResult};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tokio::time::{Duration, Interval, interval};
use std::collections::HashMap;

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

    /// Set codec
    pub fn codec(mut self, codec: AudioCodec) -> Self {
        self.config.codec = codec;
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

        // Start input processing task
        if let Some(ref input_device) = self.input_device {
            let device = input_device.clone();
            let tx = self.input_frame_tx.clone();
            let format = self.config.input_format.clone();
            let state = self.state.clone();

            tokio::spawn(async move {
                Self::input_capture_task(device, tx, format, state).await;
            });
        }

        // Start output processing task
        if let Some(ref output_device) = self.output_device {
            let device = output_device.clone();
            let mut rx = self.output_frame_rx.take().ok_or_else(|| AudioError::PipelineError {
                stage: "start".to_string(),
                reason: "Output receiver already taken".to_string(),
            })?;
            let state = self.state.clone();

            tokio::spawn(async move {
                Self::output_playback_task(device, rx, state).await;
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
    pub async fn set_codec(&mut self, codec: AudioCodec) -> AudioResult<()> {
        self.config.codec = codec;
        // TODO: Reconfigure codec processing
        Ok(())
    }

    /// Input capture task (simulated for now)
    async fn input_capture_task(
        _device: Arc<dyn AudioDevice>,
        tx: mpsc::Sender<AudioFrame>,
        format: AudioFormat,
        state: Arc<RwLock<PipelineState>>,
    ) {
        let mut interval = interval(Duration::from_millis(format.frame_size_ms as u64));
        let mut timestamp = 0u32;

        loop {
            // Check if we should stop
            {
                let current_state = state.read().await;
                if *current_state == PipelineState::Stopping || *current_state == PipelineState::Stopped {
                    break;
                }
            }

            interval.tick().await;

            // Simulate capturing audio (in real implementation, this would read from device)
            let samples = vec![0i16; format.samples_per_frame()]; // Silent frame for simulation
            let frame = AudioFrame::new(samples, format.clone(), timestamp);

            if tx.send(frame).await.is_err() {
                break; // Channel closed
            }

            timestamp = timestamp.wrapping_add(format.samples_per_frame() as u32);
        }
    }

    /// Output playback task (simulated for now)
    async fn output_playback_task(
        _device: Arc<dyn AudioDevice>,
        mut rx: mpsc::Receiver<AudioFrame>,
        state: Arc<RwLock<PipelineState>>,
    ) {
        loop {
            // Check if we should stop
            {
                let current_state = state.read().await;
                if *current_state == PipelineState::Stopping || *current_state == PipelineState::Stopped {
                    break;
                }
            }

            match rx.recv().await {
                Some(_frame) => {
                    // Simulate playing audio (in real implementation, this would write to device)
                    // For now, just consume the frame
                }
                None => break, // Channel closed
            }
        }
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
            .codec(config.codec.clone())
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