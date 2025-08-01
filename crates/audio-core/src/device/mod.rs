//! Audio device management
//!
//! This module provides audio device discovery, management, and control.

#[cfg(feature = "device-cpal")]
pub(crate) mod cpal_backend;

#[cfg(feature = "device-cpal")]
pub(crate) mod cpal_stream;

/// Audio device trait for platform abstraction
pub trait AudioDevice: Send + Sync + std::fmt::Debug {
    /// Get device information
    fn info(&self) -> &crate::types::AudioDeviceInfo;
    
    /// Check if the device supports the given format
    fn supports_format(&self, format: &crate::types::AudioFormat) -> bool {
        self.info().supports_format(format)
    }
    
    /// Get as Any for downcasting
    fn as_any(&self) -> &dyn std::any::Any {
        // Default implementation that panics - concrete types should override
        panic!("AudioDevice::as_any not implemented")
    }
}

/// Audio device manager for discovering and managing audio devices
#[derive(Clone)]
pub struct AudioDeviceManager {
    // Placeholder implementation
}

impl AudioDeviceManager {
    /// Create a new audio device manager
    pub async fn new() -> crate::error::AudioResult<Self> {
        Ok(Self {})
    }

    /// List available audio devices
    pub async fn list_devices(&self, direction: crate::types::AudioDirection) -> crate::error::AudioResult<Vec<crate::types::AudioDeviceInfo>> {
        #[cfg(feature = "device-cpal")]
        {
            cpal_backend::list_cpal_devices(direction)
        }
        
        #[cfg(not(feature = "device-cpal"))]
        {
            // Fallback to mock devices if CPAL is not enabled
            let devices = match direction {
                crate::types::AudioDirection::Input => {
                    vec![
                        {
                            let mut info = crate::types::AudioDeviceInfo::new("default-mic", "Default Microphone", direction);
                            info.is_default = true;
                            info
                        },
                        crate::types::AudioDeviceInfo::new("builtin-mic", "Built-in Microphone", direction),
                    ]
                }
                crate::types::AudioDirection::Output => {
                    vec![
                        {
                            let mut info = crate::types::AudioDeviceInfo::new("default-speaker", "Default Speaker", direction);
                            info.is_default = true;
                            info
                        },
                        crate::types::AudioDeviceInfo::new("builtin-speaker", "Built-in Output", direction),
                    ]
                }
            };
            
            Ok(devices)
        }
    }

    /// Get the default device for the specified direction
    pub async fn get_default_device(&self, direction: crate::types::AudioDirection) -> crate::error::AudioResult<std::sync::Arc<dyn AudioDevice>> {
        #[cfg(feature = "device-cpal")]
        {
            cpal_backend::get_default_cpal_device(direction)
                .map(|device| device as std::sync::Arc<dyn AudioDevice>)
        }
        
        #[cfg(not(feature = "device-cpal"))]
        {
            // Create a mock device for fallback
            let device_info = crate::types::AudioDeviceInfo::new(
                "default",
                "Default Device",
                direction,
            );
            Ok(std::sync::Arc::new(MockAudioDevice { info: device_info }))
        }
    }
    
    /// Get a specific device by ID
    pub async fn get_device_by_id(&self, id: &str, direction: crate::types::AudioDirection) -> crate::error::AudioResult<std::sync::Arc<dyn AudioDevice>> {
        #[cfg(feature = "device-cpal")]
        {
            cpal_backend::get_cpal_device_by_id(id, direction)
                .map(|device| device as std::sync::Arc<dyn AudioDevice>)
        }
        
        #[cfg(not(feature = "device-cpal"))]
        {
            // For mock, just return a device with the requested ID
            let device_info = crate::types::AudioDeviceInfo::new(
                id,
                id,
                direction,
            );
            Ok(std::sync::Arc::new(MockAudioDevice { info: device_info }))
        }
    }
}

/// Mock audio device for testing
#[derive(Debug)]
struct MockAudioDevice {
    info: crate::types::AudioDeviceInfo,
}

impl AudioDevice for MockAudioDevice {
    fn info(&self) -> &crate::types::AudioDeviceInfo {
        &self.info
    }
    
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
} 