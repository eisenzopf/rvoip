//! Test audio backend for integration testing
//! 
//! This module provides memory-based audio devices that can be used
//! to test two SIP clients on the same machine without requiring
//! actual audio hardware.

use std::collections::{VecDeque, HashMap};
use std::sync::Arc;
use tokio::sync::Mutex;
use rvoip_audio_core::{
    AudioDevice, AudioDeviceInfo, AudioDirection, AudioFormat, 
    AudioFrame, AudioError, AudioResult
};

/// Shared audio buffers between test clients
#[derive(Debug)]
pub struct TestAudioBuffers {
    /// Audio from client A to client B
    pub a_to_b: Arc<Mutex<VecDeque<AudioFrame>>>,
    /// Audio from client B to client A  
    pub b_to_a: Arc<Mutex<VecDeque<AudioFrame>>>,
}

impl TestAudioBuffers {
    /// Create new shared audio buffers
    pub fn new() -> Self {
        Self {
            a_to_b: Arc::new(Mutex::new(VecDeque::new())),
            b_to_a: Arc::new(Mutex::new(VecDeque::new())),
        }
    }
}

/// Test audio device that reads/writes to memory buffers
pub struct TestAudioDevice {
    info: AudioDeviceInfo,
    /// Buffer to read from (for input devices)
    read_buffer: Option<Arc<Mutex<VecDeque<AudioFrame>>>>,
    /// Buffer to write to (for output devices)
    write_buffer: Option<Arc<Mutex<VecDeque<AudioFrame>>>>,
}

impl TestAudioDevice {
    /// Create a test input device (microphone)
    pub fn input(name: &str, read_from: Arc<Mutex<VecDeque<AudioFrame>>>) -> Self {
        let mut info = AudioDeviceInfo::new(
            format!("test-input-{}", name),
            format!("Test Input {}", name),
            AudioDirection::Input,
        );
        info.is_default = true;
        
        Self {
            info,
            read_buffer: Some(read_from),
            write_buffer: None,
        }
    }
    
    /// Create a test output device (speaker)
    pub fn output(name: &str, write_to: Arc<Mutex<VecDeque<AudioFrame>>>) -> Self {
        let mut info = AudioDeviceInfo::new(
            format!("test-output-{}", name),
            format!("Test Output {}", name),
            AudioDirection::Output,
        );
        info.is_default = true;
        
        Self {
            info,
            read_buffer: None,
            write_buffer: Some(write_to),
        }
    }
    
    /// Read audio frame from input buffer
    pub async fn read_frame(&self) -> Option<AudioFrame> {
        if let Some(buffer) = &self.read_buffer {
            buffer.lock().await.pop_front()
        } else {
            None
        }
    }
    
    /// Write audio frame to output buffer
    pub async fn write_frame(&self, frame: AudioFrame) -> AudioResult<()> {
        if let Some(buffer) = &self.write_buffer {
            buffer.lock().await.push_back(frame);
            Ok(())
        } else {
            Err(AudioError::DeviceError {
                device: "test".to_string(),
                operation: "write_frame".to_string(),
                reason: "Not an output device".to_string(),
            })
        }
    }
}

impl std::fmt::Debug for TestAudioDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestAudioDevice")
            .field("info", &self.info)
            .finish()
    }
}

impl AudioDevice for TestAudioDevice {
    fn info(&self) -> &AudioDeviceInfo {
        &self.info
    }
    
    fn supports_format(&self, format: &AudioFormat) -> bool {
        // Support standard VoIP formats
        format.sample_rate == 8000 || format.sample_rate == 16000
    }
    
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Test audio device manager that creates memory-based devices
pub struct TestAudioDeviceManager {
    buffers: TestAudioBuffers,
    client_name: String,
}

impl TestAudioDeviceManager {
    /// Create a new test audio device manager for a specific client
    pub fn new(buffers: TestAudioBuffers, client_name: String) -> Self {
        Self {
            buffers,
            client_name,
        }
    }
    
    /// Get the appropriate input device for this client
    pub fn get_input_device(&self) -> Arc<dyn AudioDevice> {
        let read_buffer = match self.client_name.as_str() {
            "A" => self.buffers.b_to_a.clone(), // Client A reads from B's output
            "B" => self.buffers.a_to_b.clone(), // Client B reads from A's output
            _ => panic!("Invalid client name"),
        };
        
        Arc::new(TestAudioDevice::input(&self.client_name, read_buffer))
    }
    
    /// Get the appropriate output device for this client
    pub fn get_output_device(&self) -> Arc<dyn AudioDevice> {
        let write_buffer = match self.client_name.as_str() {
            "A" => self.buffers.a_to_b.clone(), // Client A writes to A->B buffer
            "B" => self.buffers.b_to_a.clone(), // Client B writes to B->A buffer
            _ => panic!("Invalid client name"),
        };
        
        Arc::new(TestAudioDevice::output(&self.client_name, write_buffer))
    }
}

/// Audio pipeline for test mode that directly processes frames
pub struct TestAudioPipeline {
    device_manager: TestAudioDeviceManager,
    format: AudioFormat,
}

impl TestAudioPipeline {
    /// Create a new test audio pipeline
    pub fn new(device_manager: TestAudioDeviceManager, format: AudioFormat) -> Self {
        Self {
            device_manager,
            format,
        }
    }
    
    /// Process an input frame (from microphone to network)
    pub async fn process_input_frame(&self) -> Option<AudioFrame> {
        let input_device = self.device_manager.get_input_device();
        if let Some(device) = input_device.as_any().downcast_ref::<TestAudioDevice>() {
            device.read_frame().await
        } else {
            None
        }
    }
    
    /// Process an output frame (from network to speaker)
    pub async fn process_output_frame(&self, frame: AudioFrame) -> AudioResult<()> {
        let output_device = self.device_manager.get_output_device();
        if let Some(device) = output_device.as_any().downcast_ref::<TestAudioDevice>() {
            device.write_frame(frame).await
        } else {
            Err(AudioError::DeviceError {
                device: "test".to_string(),
                operation: "write_frame".to_string(),
                reason: "Invalid device type".to_string(),
            })
        }
    }
}