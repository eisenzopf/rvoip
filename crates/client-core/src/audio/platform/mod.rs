//! Platform-specific audio device implementations
//!
//! This module provides platform-specific implementations of the AudioDevice trait.
//! It automatically selects the appropriate backend based on the target platform
//! and available features.

use std::sync::Arc;
use crate::audio::device::{AudioDevice, AudioDeviceInfo, AudioDirection, AudioError, AudioResult};

#[cfg(feature = "cpal")]
pub mod cpal_impl;
pub mod mock_impl;

/// Create a platform-specific audio device
pub async fn create_platform_device(device_id: &str) -> AudioResult<Arc<dyn AudioDevice>> {
    #[cfg(feature = "cpal")]
    {
        cpal_impl::create_device(device_id).await
    }
    #[cfg(not(feature = "cpal"))]
    {
        mock_impl::create_device(device_id).await
    }
}

/// List available platform devices
pub async fn list_platform_devices(direction: AudioDirection) -> AudioResult<Vec<AudioDeviceInfo>> {
    #[cfg(feature = "cpal")]
    {
        cpal_impl::list_devices(direction).await
    }
    #[cfg(not(feature = "cpal"))]
    {
        mock_impl::list_devices(direction).await
    }
}

/// Get the default platform device
pub async fn get_default_platform_device(direction: AudioDirection) -> AudioResult<Arc<dyn AudioDevice>> {
    #[cfg(feature = "cpal")]
    {
        cpal_impl::get_default_device(direction).await
    }
    #[cfg(not(feature = "cpal"))]
    {
        mock_impl::get_default_device(direction).await
    }
} 