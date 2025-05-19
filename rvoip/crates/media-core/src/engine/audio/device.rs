use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use thiserror::Error;
use tracing::{debug, info, warn};

use crate::codec::audio::common::AudioFormat;
use crate::error::Result;

/// Audio device type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioDeviceType {
    /// Input device (microphone)
    Input,
    /// Output device (speaker)
    Output,
    /// Both input and output
    Both,
}

/// Audio device information
#[derive(Clone)]
pub struct AudioDeviceInfo {
    /// Device ID
    pub id: String,
    /// Device name
    pub name: String,
    /// Device type
    pub device_type: AudioDeviceType,
    /// Default device flag
    pub is_default: bool,
    /// Available sample rates
    pub sample_rates: Vec<u32>,
    /// Available channel counts
    pub channels: Vec<u8>,
}

impl fmt::Debug for AudioDeviceInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AudioDeviceInfo")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("type", &self.device_type)
            .field("default", &self.is_default)
            .field("sample_rates", &self.sample_rates)
            .field("channels", &self.channels)
            .finish()
    }
}

/// Audio device handle
pub struct AudioDevice {
    /// Device information
    info: AudioDeviceInfo,
    /// Current format
    format: AudioFormat,
    /// Native device handle
    #[allow(dead_code)]
    handle: usize,
    /// Buffer size in frames
    buffer_size: usize,
    /// Device status
    status: AudioDeviceStatus,
}

/// Audio device status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioDeviceStatus {
    /// Device is closed
    Closed,
    /// Device is open but not active
    Open,
    /// Device is active and streaming
    Active,
    /// Device has an error
    Error,
}

/// Errors specific to audio devices
#[derive(Error, Debug)]
pub enum AudioDeviceError {
    /// Device not found
    #[error("Audio device not found: {0}")]
    DeviceNotFound(String),
    
    /// Device already in use
    #[error("Audio device already in use: {0}")]
    DeviceInUse(String),
    
    /// Unsupported format
    #[error("Unsupported audio format: {0}")]
    UnsupportedFormat(String),
    
    /// Device error
    #[error("Audio device error: {0}")]
    DeviceError(String),
}

impl AudioDevice {
    /// Create a new audio device
    pub fn new(info: AudioDeviceInfo, format: AudioFormat, buffer_size: usize) -> Self {
        Self {
            info,
            format,
            handle: 0,
            buffer_size,
            status: AudioDeviceStatus::Closed,
        }
    }
    
    /// Open the audio device
    pub fn open(&mut self) -> Result<()> {
        // In a real implementation, this would initialize the native audio API
        // For this stub, we just update the status
        debug!("Opening audio device: {}", self.info.name);
        self.status = AudioDeviceStatus::Open;
        Ok(())
    }
    
    /// Start the audio device
    pub fn start(&mut self) -> Result<()> {
        if self.status != AudioDeviceStatus::Open {
            return Err(AudioDeviceError::DeviceError(
                format!("Cannot start device in state: {:?}", self.status)
            ).into());
        }
        
        // In a real implementation, this would start the audio stream
        info!("Starting audio device: {}", self.info.name);
        self.status = AudioDeviceStatus::Active;
        Ok(())
    }
    
    /// Stop the audio device
    pub fn stop(&mut self) -> Result<()> {
        if self.status != AudioDeviceStatus::Active {
            return Ok(());
        }
        
        // In a real implementation, this would stop the audio stream
        debug!("Stopping audio device: {}", self.info.name);
        self.status = AudioDeviceStatus::Open;
        Ok(())
    }
    
    /// Close the audio device
    pub fn close(&mut self) -> Result<()> {
        if self.status == AudioDeviceStatus::Closed {
            return Ok(());
        }
        
        // Stop first if active
        if self.status == AudioDeviceStatus::Active {
            self.stop()?;
        }
        
        // In a real implementation, this would close the native device
        debug!("Closing audio device: {}", self.info.name);
        self.status = AudioDeviceStatus::Closed;
        Ok(())
    }
    
    /// Get the device information
    pub fn info(&self) -> &AudioDeviceInfo {
        &self.info
    }
    
    /// Get the current audio format
    pub fn format(&self) -> AudioFormat {
        self.format
    }
    
    /// Get the buffer size in frames
    pub fn buffer_size(&self) -> usize {
        self.buffer_size
    }
    
    /// Get the device status
    pub fn status(&self) -> AudioDeviceStatus {
        self.status
    }
    
    /// Set the audio format
    pub fn set_format(&mut self, format: AudioFormat) -> Result<()> {
        if self.status != AudioDeviceStatus::Closed {
            return Err(AudioDeviceError::DeviceError(
                "Cannot change format while device is open".to_string()
            ).into());
        }
        
        // Check if format is supported
        let sample_rate_supported = self.info.sample_rates.contains(&format.sample_rate.as_hz());
        let channels_supported = self.info.channels.contains(&format.channels.channel_count());
        
        if !sample_rate_supported || !channels_supported {
            return Err(AudioDeviceError::UnsupportedFormat(
                format!("{:?}", format)
            ).into());
        }
        
        self.format = format;
        Ok(())
    }
    
    /// Set the buffer size in frames
    pub fn set_buffer_size(&mut self, buffer_size: usize) -> Result<()> {
        if self.status != AudioDeviceStatus::Closed {
            return Err(AudioDeviceError::DeviceError(
                "Cannot change buffer size while device is open".to_string()
            ).into());
        }
        
        self.buffer_size = buffer_size;
        Ok(())
    }
    
    /// Get the latency of the device
    pub fn latency(&self) -> Duration {
        // In a real implementation, this would query the native device
        // For this stub, we calculate based on buffer size
        let frames = self.buffer_size as u64;
        let sample_rate = self.format.sample_rate.as_hz() as u64;
        Duration::from_millis(frames * 1000 / sample_rate)
    }
}

impl Drop for AudioDevice {
    fn drop(&mut self) {
        if self.status != AudioDeviceStatus::Closed {
            let _ = self.close();
        }
    }
}

/// Audio device manager
pub struct AudioDeviceManager {
    /// Available devices
    devices: Vec<AudioDeviceInfo>,
    /// Refresh interval
    refresh_interval: Duration,
    /// Last refresh time
    #[allow(dead_code)]
    last_refresh: std::time::Instant,
}

impl AudioDeviceManager {
    /// Create a new audio device manager
    pub fn new() -> Self {
        let devices = Vec::new();
        
        let mut manager = Self {
            devices,
            refresh_interval: Duration::from_secs(5),
            last_refresh: std::time::Instant::now(),
        };
        
        // Initial device scan
        let _ = manager.refresh_devices();
        
        manager
    }
    
    /// Refresh the list of available devices
    pub fn refresh_devices(&mut self) -> Result<()> {
        // In a real implementation, this would query the system for devices
        // For this stub, we create some dummy devices
        
        self.devices.clear();
        
        // Add default input device
        self.devices.push(AudioDeviceInfo {
            id: "default_input".to_string(),
            name: "Default Microphone".to_string(),
            device_type: AudioDeviceType::Input,
            is_default: true,
            sample_rates: vec![8000, 16000, 44100, 48000],
            channels: vec![1, 2],
        });
        
        // Add default output device
        self.devices.push(AudioDeviceInfo {
            id: "default_output".to_string(),
            name: "Default Speaker".to_string(),
            device_type: AudioDeviceType::Output,
            is_default: true,
            sample_rates: vec![8000, 16000, 44100, 48000],
            channels: vec![1, 2],
        });
        
        // Add some additional devices
        self.devices.push(AudioDeviceInfo {
            id: "headset".to_string(),
            name: "Headset".to_string(),
            device_type: AudioDeviceType::Both,
            is_default: false,
            sample_rates: vec![16000, 44100, 48000],
            channels: vec![1, 2],
        });
        
        self.last_refresh = std::time::Instant::now();
        debug!("Refreshed audio devices, found {} devices", self.devices.len());
        
        Ok(())
    }
    
    /// Get all available devices
    pub fn devices(&self) -> &[AudioDeviceInfo] {
        &self.devices
    }
    
    /// Get input devices
    pub fn input_devices(&self) -> Vec<&AudioDeviceInfo> {
        self.devices.iter()
            .filter(|d| matches!(d.device_type, AudioDeviceType::Input | AudioDeviceType::Both))
            .collect()
    }
    
    /// Get output devices
    pub fn output_devices(&self) -> Vec<&AudioDeviceInfo> {
        self.devices.iter()
            .filter(|d| matches!(d.device_type, AudioDeviceType::Output | AudioDeviceType::Both))
            .collect()
    }
    
    /// Get default input device
    pub fn default_input_device(&self) -> Option<&AudioDeviceInfo> {
        self.input_devices()
            .into_iter()
            .find(|d| d.is_default)
    }
    
    /// Get default output device
    pub fn default_output_device(&self) -> Option<&AudioDeviceInfo> {
        self.output_devices()
            .into_iter()
            .find(|d| d.is_default)
    }
    
    /// Get device by ID
    pub fn get_device(&self, id: &str) -> Option<&AudioDeviceInfo> {
        self.devices.iter().find(|d| d.id == id)
    }
    
    /// Open an input device
    pub fn open_input(&self, device_id: Option<&str>, format: AudioFormat, buffer_size: usize) -> Result<AudioDevice> {
        let device_info = match device_id {
            Some(id) => self.get_device(id)
                .ok_or_else(|| AudioDeviceError::DeviceNotFound(id.to_string()))?,
            None => self.default_input_device()
                .ok_or_else(|| AudioDeviceError::DeviceNotFound("default input".to_string()))?,
        };
        
        // Check device type
        if !matches!(device_info.device_type, AudioDeviceType::Input | AudioDeviceType::Both) {
            return Err(AudioDeviceError::DeviceError(
                format!("Device {} is not an input device", device_info.name)
            ).into());
        }
        
        // Create and open the device
        let mut device = AudioDevice::new(device_info.clone(), format, buffer_size);
        device.open()?;
        
        Ok(device)
    }
    
    /// Open an output device
    pub fn open_output(&self, device_id: Option<&str>, format: AudioFormat, buffer_size: usize) -> Result<AudioDevice> {
        let device_info = match device_id {
            Some(id) => self.get_device(id)
                .ok_or_else(|| AudioDeviceError::DeviceNotFound(id.to_string()))?,
            None => self.default_output_device()
                .ok_or_else(|| AudioDeviceError::DeviceNotFound("default output".to_string()))?,
        };
        
        // Check device type
        if !matches!(device_info.device_type, AudioDeviceType::Output | AudioDeviceType::Both) {
            return Err(AudioDeviceError::DeviceError(
                format!("Device {} is not an output device", device_info.name)
            ).into());
        }
        
        // Create and open the device
        let mut device = AudioDevice::new(device_info.clone(), format, buffer_size);
        device.open()?;
        
        Ok(device)
    }
    
    /// Set the device refresh interval
    pub fn set_refresh_interval(&mut self, interval: Duration) {
        self.refresh_interval = interval;
    }
    
    /// Get a shared instance of the device manager
    pub fn instance() -> Arc<Mutex<Self>> {
        static mut INSTANCE: Option<Arc<Mutex<AudioDeviceManager>>> = None;
        static INIT: std::sync::Once = std::sync::Once::new();
        
        unsafe {
            INIT.call_once(|| {
                INSTANCE = Some(Arc::new(Mutex::new(AudioDeviceManager::new())));
            });
            
            INSTANCE.clone().unwrap()
        }
    }
} 