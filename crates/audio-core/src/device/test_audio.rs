//! Test audio device implementation for integration testing
//!
//! This module provides memory-based audio devices that can be used
//! to test applications without requiring actual audio hardware.

use crate::{
    error::{AudioError, AudioResult},
    types::{AudioDeviceInfo, AudioDirection, AudioFormat, AudioFrame},
};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant, sleep};

/// Shared audio buffers for test devices
#[derive(Debug, Clone)]
pub struct TestAudioBuffers {
    /// Input buffer (simulates microphone data)
    pub input_buffer: Arc<Mutex<VecDeque<AudioFrame>>>,
    /// Output buffer (captures speaker data)
    pub output_buffer: Arc<Mutex<VecDeque<AudioFrame>>>,
}

impl TestAudioBuffers {
    /// Create new test audio buffers
    pub fn new() -> Self {
        Self {
            input_buffer: Arc::new(Mutex::new(VecDeque::new())),
            output_buffer: Arc::new(Mutex::new(VecDeque::new())),
        }
    }
}

impl Default for TestAudioBuffers {
    fn default() -> Self {
        Self::new()
    }
}

/// Frame scheduler for maintaining proper audio timing
#[derive(Debug)]
struct FrameScheduler {
    frame_duration: Duration,
    last_frame_time: Mutex<Option<Instant>>,
    frame_counter: Mutex<u64>,
}

impl FrameScheduler {
    fn new(frame_duration_ms: u64) -> Self {
        Self {
            frame_duration: Duration::from_millis(frame_duration_ms),
            last_frame_time: Mutex::new(None),
            frame_counter: Mutex::new(0),
        }
    }
    
    async fn wait_for_next_frame(&self) {
        let mut last_time = self.last_frame_time.lock().await;
        let now = Instant::now();
        
        if let Some(last) = *last_time {
            let elapsed = now.duration_since(last);
            if elapsed < self.frame_duration {
                sleep(self.frame_duration - elapsed).await;
            }
        }
        
        *last_time = Some(Instant::now());
        let mut counter = self.frame_counter.lock().await;
        *counter += 1;
    }
    
    async fn reset(&self) {
        *self.last_frame_time.lock().await = None;
        *self.frame_counter.lock().await = 0;
    }
}

/// Test audio device that reads/writes to memory buffers
#[derive(Clone)]
pub struct TestAudioDevice {
    info: AudioDeviceInfo,
    buffers: TestAudioBuffers,
    scheduler: Arc<FrameScheduler>,
    is_running: Arc<AtomicBool>,
    /// Default format for generating silence frames
    default_format: AudioFormat,
}

impl TestAudioDevice {
    /// Create a test input device (microphone)
    pub fn new_input(name: &str, buffers: TestAudioBuffers) -> Self {
        let mut info = AudioDeviceInfo::new(
            format!("test-input-{}", name),
            format!("Test Input {}", name),
            AudioDirection::Input,
        );
        info.is_default = true;
        
        Self {
            info,
            buffers,
            scheduler: Arc::new(FrameScheduler::new(20)), // 20ms frames
            is_running: Arc::new(AtomicBool::new(true)),
            default_format: AudioFormat::pcm_8khz_mono(),
        }
    }
    
    /// Create a test output device (speaker)
    pub fn new_output(name: &str, buffers: TestAudioBuffers) -> Self {
        let mut info = AudioDeviceInfo::new(
            format!("test-output-{}", name),
            format!("Test Output {}", name),
            AudioDirection::Output,
        );
        info.is_default = true;
        
        Self {
            info,
            buffers,
            scheduler: Arc::new(FrameScheduler::new(20)), // 20ms frames
            is_running: Arc::new(AtomicBool::new(true)),
            default_format: AudioFormat::pcm_8khz_mono(),
        }
    }
    
    /// Start the device (for pipeline integration)
    pub fn start(&self) {
        self.is_running.store(true, Ordering::SeqCst);
    }
    
    /// Stop the device
    pub fn stop(&self) {
        self.is_running.store(false, Ordering::SeqCst);
    }
    
    /// Check if device is running
    pub fn is_running(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }
    
    /// Generate a silence frame
    fn generate_silence_frame(&self) -> AudioFrame {
        let samples_per_frame = (self.default_format.sample_rate * 20 / 1000) as usize;
        AudioFrame::new(
            vec![0i16; samples_per_frame],
            self.default_format.clone(),
            0,
        )
    }
    
    /// Read an audio frame from the input buffer (continuous mode)
    pub async fn read_frame(&self) -> Option<AudioFrame> {
        if self.info.direction != AudioDirection::Input {
            return None;
        }
        
        if !self.is_running() {
            return None;
        }
        
        // Wait for the next frame time
        self.scheduler.wait_for_next_frame().await;
        
        // Try to get a frame from the buffer
        let frame = self.buffers.input_buffer.lock().await.pop_front();
        
        // If no frame available, generate silence to maintain continuous stream
        Some(frame.unwrap_or_else(|| self.generate_silence_frame()))
    }
    
    /// Write an audio frame to the output buffer
    pub async fn write_frame(&self, frame: AudioFrame) -> AudioResult<()> {
        if self.info.direction == AudioDirection::Output {
            self.buffers.output_buffer.lock().await.push_back(frame);
            Ok(())
        } else {
            Err(AudioError::DeviceError {
                device: self.info.name.clone(),
                operation: "write_frame".to_string(),
                reason: "Not an output device".to_string(),
            })
        }
    }
    
    /// Push frames to the input buffer (for testing)
    pub async fn push_input_frame(&self, frame: AudioFrame) -> AudioResult<()> {
        if self.info.direction == AudioDirection::Input {
            self.buffers.input_buffer.lock().await.push_back(frame);
            Ok(())
        } else {
            Err(AudioError::DeviceError {
                device: self.info.name.clone(),
                operation: "push_input_frame".to_string(),
                reason: "Not an input device".to_string(),
            })
        }
    }
    
    /// Pop frames from the output buffer (for testing)
    pub async fn pop_output_frame(&self) -> Option<AudioFrame> {
        if self.info.direction == AudioDirection::Output {
            self.buffers.output_buffer.lock().await.pop_front()
        } else {
            None
        }
    }
}

impl std::fmt::Debug for TestAudioDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestAudioDevice")
            .field("info", &self.info)
            .field("is_running", &self.is_running())
            .finish()
    }
}

impl super::AudioDevice for TestAudioDevice {
    fn info(&self) -> &AudioDeviceInfo {
        &self.info
    }
    
    fn supports_format(&self, format: &AudioFormat) -> bool {
        // Support common VoIP formats
        (format.sample_rate == 8000 || format.sample_rate == 16000 || 
         format.sample_rate == 44100 || format.sample_rate == 48000) &&
        (format.channels == 1 || format.channels == 2) &&
        format.bits_per_sample == 16
    }
    
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Test audio device provider for AudioDeviceManager
pub struct TestAudioProvider {
    buffers: TestAudioBuffers,
    device_name: String,
}

impl TestAudioProvider {
    /// Create a new test audio provider
    pub fn new(buffers: TestAudioBuffers, device_name: String) -> Self {
        Self {
            buffers,
            device_name,
        }
    }
    
    /// Get the test input device
    pub fn get_input_device(&self) -> Arc<dyn super::AudioDevice> {
        Arc::new(TestAudioDevice::new_input(&self.device_name, self.buffers.clone()))
    }
    
    /// Get the test output device
    pub fn get_output_device(&self) -> Arc<dyn super::AudioDevice> {
        Arc::new(TestAudioDevice::new_output(&self.device_name, self.buffers.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::AudioFormat;
    
    #[tokio::test]
    async fn test_audio_device_creation() {
        let buffers = TestAudioBuffers::new();
        
        let input_device = TestAudioDevice::new_input("test", buffers.clone());
        assert_eq!(input_device.info().direction, AudioDirection::Input);
        assert!(input_device.info().is_default);
        
        let output_device = TestAudioDevice::new_output("test", buffers);
        assert_eq!(output_device.info().direction, AudioDirection::Output);
        assert!(output_device.info().is_default);
    }
    
    #[tokio::test]
    async fn test_audio_frame_flow() {
        let buffers = TestAudioBuffers::new();
        let input_device = TestAudioDevice::new_input("test", buffers.clone());
        let output_device = TestAudioDevice::new_output("test", buffers);
        
        // Create a test frame
        let frame = AudioFrame::new(
            vec![1, 2, 3, 4],
            AudioFormat::pcm_8khz_mono(),
            0,
        );
        
        // Push to input
        input_device.push_input_frame(frame.clone()).await.unwrap();
        
        // Read from input
        let read_frame = input_device.read_frame().await.unwrap();
        assert_eq!(read_frame.samples, frame.samples);
        
        // Write to output
        output_device.write_frame(frame.clone()).await.unwrap();
        
        // Pop from output
        let output_frame = output_device.pop_output_frame().await.unwrap();
        assert_eq!(output_frame.samples, frame.samples);
    }
}