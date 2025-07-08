//! Platform-specific audio device implementations
//!
//! This module provides platform-specific implementations of the AudioDevice trait.
//! It automatically selects the appropriate backend based on the target platform
//! and available features.

use std::sync::Arc;
use crate::audio::device::{AudioDevice, AudioDeviceInfo, AudioDirection, AudioError, AudioResult};

#[cfg(feature = "audio-cpal")]
pub mod cpal_impl;
pub mod mock_impl;

/// Platform configuration for audio devices
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioPlatformType {
    /// Mock implementation for testing
    Mock,
    /// CPAL-based implementation for real hardware
    #[cfg(feature = "audio-cpal")]
    Cpal,
}

impl Default for AudioPlatformType {
    fn default() -> Self {
        #[cfg(feature = "audio-cpal")]
        {
            Self::Cpal
        }
        #[cfg(not(feature = "audio-cpal"))]
        {
            Self::Mock
        }
    }
}

/// Create a platform-specific audio device
pub async fn create_platform_device(device_id: &str) -> AudioResult<Arc<dyn AudioDevice>> {
    // Try CPAL devices first
    #[cfg(feature = "audio-cpal")]
    {
        if device_id.starts_with("cpal-") {
            let platform = cpal_impl::create_cpal_platform();
            return Ok(platform.create_device(device_id)?);
        }
    }
    
    // Try mock devices (for testing or when device_id starts with "mock_")
    if device_id.starts_with("mock_") {
        return mock_impl::create_device(device_id).await;
    }
    
    // For unknown device IDs, try CPAL first if available
    #[cfg(feature = "audio-cpal")]
    {
        let platform = cpal_impl::create_cpal_platform();
        if let Ok(device) = platform.create_device(device_id) {
            return Ok(device);
        }
    }
    
    // Final fallback to mock implementation
    mock_impl::create_device(device_id).await
}

/// List available platform devices
pub async fn list_platform_devices(direction: AudioDirection) -> AudioResult<Vec<AudioDeviceInfo>> {
    let mut all_devices = Vec::new();
    
    // Add CPAL devices first (primary choice for real hardware)
    #[cfg(feature = "audio-cpal")]
    {
        let platform = cpal_impl::create_cpal_platform();
        match platform.list_devices(direction) {
            Ok(devices) => all_devices.extend(devices),
            Err(e) => {
                tracing::warn!("Failed to list CPAL devices: {}", e);
            }
        }
    }
    
    // Add mock devices as fallback (for testing when no real devices available)
    let mock_devices = mock_impl::list_devices(direction).await?;
    all_devices.extend(mock_devices);
    
    Ok(all_devices)
}

/// Get the default platform device
pub async fn get_default_platform_device(direction: AudioDirection) -> AudioResult<Arc<dyn AudioDevice>> {
    // Try CPAL devices first (primary choice for real hardware)
    #[cfg(feature = "audio-cpal")]
    {
        let platform = cpal_impl::create_cpal_platform();
        match platform.get_default_device(direction) {
            Ok(device) => return Ok(device),
            Err(e) => {
                tracing::warn!("Failed to get default CPAL device: {}, falling back to mock", e);
            }
        }
    }
    
    // Fallback to mock devices (for testing when no real devices available)
    mock_impl::get_default_device(direction).await
} 