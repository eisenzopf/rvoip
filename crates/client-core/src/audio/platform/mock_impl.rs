//! Mock audio device implementation for testing
//!
//! This module provides mock audio devices that simulate real hardware
//! for testing and development purposes.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use tokio::time::{interval, Duration};

use crate::audio::device::{
    AudioDevice, AudioDeviceInfo, AudioDirection, AudioFormat, AudioFrame, AudioError, AudioResult
};

/// Mock audio device implementation
#[derive(Debug)]
pub struct MockAudioDevice {
    info: AudioDeviceInfo,
    is_active: AtomicBool,
    current_format: parking_lot::Mutex<Option<AudioFormat>>,
}

impl MockAudioDevice {
    /// Create a new mock audio device
    pub fn new(info: AudioDeviceInfo) -> Self {
        Self {
            info,
            is_active: AtomicBool::new(false),
            current_format: parking_lot::Mutex::new(None),
        }
    }
    
    /// Create a mock microphone device
    pub fn mock_microphone() -> Self {
        let info = AudioDeviceInfo::new(
            "mock_microphone".to_string(),
            "Mock Microphone".to_string(),
            AudioDirection::Input,
        );
        Self::new(info)
    }
    
    /// Create a mock speaker device
    pub fn mock_speaker() -> Self {
        let info = AudioDeviceInfo::new(
            "mock_speaker".to_string(),
            "Mock Speaker".to_string(),
            AudioDirection::Output,
        );
        Self::new(info)
    }
}

#[async_trait::async_trait]
impl AudioDevice for MockAudioDevice {
    fn info(&self) -> &AudioDeviceInfo {
        &self.info
    }
    
    async fn start_capture(&self, format: AudioFormat) -> AudioResult<mpsc::Receiver<AudioFrame>> {
        if self.info.direction != AudioDirection::Input {
            return Err(AudioError::ConfigurationError {
                message: "Cannot start capture on output device".to_string(),
            });
        }
        
        if !self.supports_format(&format) {
            return Err(AudioError::FormatNotSupported {
                format: format.clone(),
                device_id: self.info.id.clone(),
            });
        }
        
        self.is_active.store(true, Ordering::Relaxed);
        *self.current_format.lock() = Some(format.clone());
        
        let (tx, rx) = mpsc::channel(10);
        
        // Generate mock audio frames
        let samples_per_frame = format.samples_per_frame();
        let frame_duration = Duration::from_millis(format.frame_size_ms as u64);
        
        tokio::spawn(async move {
            let mut interval = interval(frame_duration);
            let mut timestamp = 0u64;
            
            loop {
                interval.tick().await;
                
                // Generate sine wave at 440Hz for mock audio
                let mut samples = Vec::with_capacity(samples_per_frame);
                for i in 0..samples_per_frame {
                    let t = (timestamp as f64 / 1000.0) + (i as f64 / format.sample_rate as f64);
                    let sample = (440.0 * 2.0 * std::f64::consts::PI * t).sin();
                    samples.push((sample * 16384.0) as i16); // Scale to 16-bit range
                }
                
                let frame = AudioFrame::new(samples, format.clone(), timestamp);
                
                if tx.send(frame).await.is_err() {
                    break; // Receiver dropped
                }
                
                timestamp += format.frame_size_ms as u64;
            }
        });
        
        Ok(rx)
    }
    
    async fn stop_capture(&self) -> AudioResult<()> {
        self.is_active.store(false, Ordering::Relaxed);
        *self.current_format.lock() = None;
        Ok(())
    }
    
    async fn start_playback(&self, format: AudioFormat) -> AudioResult<mpsc::Sender<AudioFrame>> {
        if self.info.direction != AudioDirection::Output {
            return Err(AudioError::ConfigurationError {
                message: "Cannot start playback on input device".to_string(),
            });
        }
        
        if !self.supports_format(&format) {
            return Err(AudioError::FormatNotSupported {
                format: format.clone(),
                device_id: self.info.id.clone(),
            });
        }
        
        self.is_active.store(true, Ordering::Relaxed);
        *self.current_format.lock() = Some(format.clone());
        
        let (tx, mut rx) = mpsc::channel::<AudioFrame>(10);
        
        // Consume audio frames (mock playback)
        tokio::spawn(async move {
            while let Some(frame) = rx.recv().await {
                // Mock playback - just log the frame
                println!("MockAudioDevice: Playing frame with {} samples at {}Hz", 
                    frame.samples.len(), frame.format.sample_rate);
                
                // Simulate playback delay
                tokio::time::sleep(Duration::from_millis(frame.format.frame_size_ms as u64)).await;
            }
        });
        
        Ok(tx)
    }
    
    async fn stop_playback(&self) -> AudioResult<()> {
        self.is_active.store(false, Ordering::Relaxed);
        *self.current_format.lock() = None;
        Ok(())
    }
    
    fn is_active(&self) -> bool {
        self.is_active.load(Ordering::Relaxed)
    }
    
    fn current_format(&self) -> Option<AudioFormat> {
        self.current_format.lock().clone()
    }
}

/// Create a mock audio device by ID
pub async fn create_device(device_id: &str) -> AudioResult<Arc<dyn AudioDevice>> {
    let device = match device_id {
        "mock_microphone" => MockAudioDevice::mock_microphone(),
        "mock_speaker" => MockAudioDevice::mock_speaker(),
        _ => return Err(AudioError::DeviceNotFound {
            device_id: device_id.to_string(),
        }),
    };
    
    Ok(Arc::new(device))
}

/// List available mock devices
pub async fn list_devices(direction: AudioDirection) -> AudioResult<Vec<AudioDeviceInfo>> {
    let mut devices = Vec::new();
    
    match direction {
        AudioDirection::Input => {
            let mut info = AudioDeviceInfo::new(
                "mock_microphone".to_string(),
                "Mock Microphone".to_string(),
                AudioDirection::Input,
            );
            info.is_default = true;
            devices.push(info);
        }
        AudioDirection::Output => {
            let mut info = AudioDeviceInfo::new(
                "mock_speaker".to_string(),
                "Mock Speaker".to_string(),
                AudioDirection::Output,
            );
            info.is_default = true;
            devices.push(info);
        }
    }
    
    Ok(devices)
}

/// Get the default mock device
pub async fn get_default_device(direction: AudioDirection) -> AudioResult<Arc<dyn AudioDevice>> {
    match direction {
        AudioDirection::Input => create_device("mock_microphone").await,
        AudioDirection::Output => create_device("mock_speaker").await,
    }
} 