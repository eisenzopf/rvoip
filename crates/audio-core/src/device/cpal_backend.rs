//! CPAL backend for real audio device access

use crate::error::{AudioError, AudioResult};
use crate::types::{AudioDeviceInfo, AudioDirection, AudioFormat};
use super::AudioDevice;
use std::sync::Arc;

#[cfg(feature = "device-cpal")]
use cpal::traits::{DeviceTrait, HostTrait};

/// CPAL-based audio device implementation
pub struct CpalAudioDevice {
    info: AudioDeviceInfo,
    #[cfg(feature = "device-cpal")]
    device: cpal::Device,
}

impl std::fmt::Debug for CpalAudioDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CpalAudioDevice")
            .field("info", &self.info)
            .field("device", &"<cpal::Device>")
            .finish()
    }
}

impl AudioDevice for CpalAudioDevice {
    fn info(&self) -> &AudioDeviceInfo {
        &self.info
    }
    
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(feature = "device-cpal")]
impl CpalAudioDevice {
    /// Get the underlying CPAL device
    pub fn cpal_device(&self) -> &cpal::Device {
        &self.device
    }
}

/// Get available audio devices using CPAL
#[cfg(feature = "device-cpal")]
pub fn list_cpal_devices(direction: AudioDirection) -> AudioResult<Vec<AudioDeviceInfo>> {
    let host = cpal::default_host();
    let mut devices = Vec::new();
    
    let device_iter: Box<dyn Iterator<Item = cpal::Device>> = match direction {
        AudioDirection::Input => Box::new(host.input_devices().map_err(|e| {
            AudioError::DeviceError {
                device: "input enumeration".to_string(),
                operation: "list".to_string(),
                reason: e.to_string(),
            }
        })?),
        AudioDirection::Output => Box::new(host.output_devices().map_err(|e| {
            AudioError::DeviceError {
                device: "output enumeration".to_string(),
                operation: "list".to_string(),
                reason: e.to_string(),
            }
        })?),
    };
    
    // Get default device name for comparison
    let default_device = match direction {
        AudioDirection::Input => host.default_input_device(),
        AudioDirection::Output => host.default_output_device(),
    };
    let default_name = default_device
        .as_ref()
        .and_then(|d| d.name().ok());
    
    for device in device_iter {
        let name = device.name().unwrap_or_else(|_| "Unknown Device".to_string());
        let id = name.clone(); // Use name as ID for simplicity
        
        // Get supported configs
        let mut supported_sample_rates = Vec::new();
        let mut supported_channels = Vec::new();
        
        match direction {
            AudioDirection::Input => {
                if let Ok(configs) = device.supported_input_configs() {
                    for config in configs {
                        let sample_rate_range = config.min_sample_rate().0..=config.max_sample_rate().0;
                        
                        // Add common sample rates that fall within the range
                        for &rate in &[8000, 16000, 44100, 48000] {
                            if sample_rate_range.contains(&rate) && !supported_sample_rates.contains(&rate) {
                                supported_sample_rates.push(rate);
                            }
                        }
                        
                        let channels = config.channels() as u16;
                        if !supported_channels.contains(&channels) {
                            supported_channels.push(channels);
                        }
                    }
                }
            }
            AudioDirection::Output => {
                if let Ok(configs) = device.supported_output_configs() {
                    for config in configs {
                        let sample_rate_range = config.min_sample_rate().0..=config.max_sample_rate().0;
                        
                        // Add common sample rates that fall within the range
                        for &rate in &[8000, 16000, 44100, 48000] {
                            if sample_rate_range.contains(&rate) && !supported_sample_rates.contains(&rate) {
                                supported_sample_rates.push(rate);
                            }
                        }
                        
                        let channels = config.channels() as u16;
                        if !supported_channels.contains(&channels) {
                            supported_channels.push(channels);
                        }
                    }
                }
            }
        }
        
        // Ensure we have at least some defaults
        if supported_sample_rates.is_empty() {
            supported_sample_rates = vec![48000];
        }
        if supported_channels.is_empty() {
            supported_channels = vec![2];
        }
        
        let is_default = default_name.as_ref() == Some(&name);
        
        let device_info = AudioDeviceInfo {
            id: id.clone(),
            name: name.clone(),
            direction,
            is_default,
            supported_sample_rates,
            supported_channels,
            supported_bit_depths: vec![16], // CPAL typically uses f32, but we'll convert to i16
        };
        
        devices.push(device_info);
    }
    
    Ok(devices)
}

/// Get the default CPAL device
#[cfg(feature = "device-cpal")]
pub fn get_default_cpal_device(direction: AudioDirection) -> AudioResult<Arc<CpalAudioDevice>> {
    let host = cpal::default_host();
    
    let device = match direction {
        AudioDirection::Input => host.default_input_device(),
        AudioDirection::Output => host.default_output_device(),
    }
    .ok_or_else(|| AudioError::DeviceError {
        device: "default".to_string(),
        operation: "get".to_string(),
        reason: format!("No default {:?} device found", direction),
    })?;
    
    let name = device.name().unwrap_or_else(|_| "Default Device".to_string());
    
    // Get device capabilities
    let mut supported_sample_rates = Vec::new();
    let mut supported_channels = Vec::new();
    
    match direction {
        AudioDirection::Input => {
            if let Ok(configs) = device.supported_input_configs() {
                for config in configs {
                    let sample_rate_range = config.min_sample_rate().0..=config.max_sample_rate().0;
                    
                    // Add common sample rates that fall within the range
                    for &rate in &[8000, 16000, 44100, 48000] {
                        if sample_rate_range.contains(&rate) && !supported_sample_rates.contains(&rate) {
                            supported_sample_rates.push(rate);
                        }
                    }
                    
                    let channels = config.channels() as u16;
                    if !supported_channels.contains(&channels) {
                        supported_channels.push(channels);
                    }
                }
            }
        }
        AudioDirection::Output => {
            if let Ok(configs) = device.supported_output_configs() {
                for config in configs {
                    let sample_rate_range = config.min_sample_rate().0..=config.max_sample_rate().0;
                    
                    // Add common sample rates that fall within the range
                    for &rate in &[8000, 16000, 44100, 48000] {
                        if sample_rate_range.contains(&rate) && !supported_sample_rates.contains(&rate) {
                            supported_sample_rates.push(rate);
                        }
                    }
                    
                    let channels = config.channels() as u16;
                    if !supported_channels.contains(&channels) {
                        supported_channels.push(channels);
                    }
                }
            }
        }
    }
    
    // Ensure we have at least some defaults
    if supported_sample_rates.is_empty() {
        supported_sample_rates = vec![48000];
    }
    if supported_channels.is_empty() {
        supported_channels = vec![2];
    }
    
    let device_info = AudioDeviceInfo {
        id: name.clone(),
        name,
        direction,
        is_default: true,
        supported_sample_rates,
        supported_channels,
        supported_bit_depths: vec![16],
    };
    
    Ok(Arc::new(CpalAudioDevice {
        info: device_info,
        device,
    }))
}

/// Get a specific CPAL device by ID (name)
#[cfg(feature = "device-cpal")]
pub fn get_cpal_device_by_id(id: &str, direction: AudioDirection) -> AudioResult<Arc<CpalAudioDevice>> {
    let host = cpal::default_host();
    
    let device_iter: Box<dyn Iterator<Item = cpal::Device>> = match direction {
        AudioDirection::Input => Box::new(host.input_devices().map_err(|e| {
            AudioError::DeviceError {
                device: id.to_string(),
                operation: "enumerate".to_string(),
                reason: e.to_string(),
            }
        })?),
        AudioDirection::Output => Box::new(host.output_devices().map_err(|e| {
            AudioError::DeviceError {
                device: id.to_string(),
                operation: "enumerate".to_string(),
                reason: e.to_string(),
            }
        })?),
    };
    
    for device in device_iter {
        if let Ok(name) = device.name() {
            if name == id {
                // Found the device
                let mut supported_sample_rates = Vec::new();
                let mut supported_channels = Vec::new();
                
                match direction {
                    AudioDirection::Input => {
                        if let Ok(configs) = device.supported_input_configs() {
                            for config in configs {
                                let sample_rate_range = config.min_sample_rate().0..=config.max_sample_rate().0;
                                
                                // Add common sample rates that fall within the range
                                for &rate in &[8000, 16000, 44100, 48000] {
                                    if sample_rate_range.contains(&rate) && !supported_sample_rates.contains(&rate) {
                                        supported_sample_rates.push(rate);
                                    }
                                }
                                
                                let channels = config.channels() as u16;
                                if !supported_channels.contains(&channels) {
                                    supported_channels.push(channels);
                                }
                            }
                        }
                    }
                    AudioDirection::Output => {
                        if let Ok(configs) = device.supported_output_configs() {
                            for config in configs {
                                let sample_rate_range = config.min_sample_rate().0..=config.max_sample_rate().0;
                                
                                // Add common sample rates that fall within the range
                                for &rate in &[8000, 16000, 44100, 48000] {
                                    if sample_rate_range.contains(&rate) && !supported_sample_rates.contains(&rate) {
                                        supported_sample_rates.push(rate);
                                    }
                                }
                                
                                let channels = config.channels() as u16;
                                if !supported_channels.contains(&channels) {
                                    supported_channels.push(channels);
                                }
                            }
                        }
                    }
                }
                
                // Ensure we have at least some defaults
                if supported_sample_rates.is_empty() {
                    supported_sample_rates = vec![48000];
                }
                if supported_channels.is_empty() {
                    supported_channels = vec![2];
                }
                
                let device_info = AudioDeviceInfo {
                    id: name.clone(),
                    name,
                    direction,
                    is_default: false,
                    supported_sample_rates,
                    supported_channels,
                    supported_bit_depths: vec![16],
                };
                
                return Ok(Arc::new(CpalAudioDevice {
                    info: device_info,
                    device,
                }));
            }
        }
    }
    
    Err(AudioError::DeviceError {
        device: id.to_string(),
        operation: "get".to_string(),
        reason: "Device not found".to_string(),
    })
}