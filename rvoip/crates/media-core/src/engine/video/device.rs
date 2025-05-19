//! Video device management
//!
//! This module provides abstractions for managing video devices like cameras.

use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::{RwLock, mpsc};
use tracing::{debug, error, info, warn};

use crate::error::{Error, Result};
use crate::codec::video::Resolution;

/// Video device capabilities
#[derive(Debug, Clone)]
pub struct VideoDeviceCapabilities {
    /// Device name
    pub name: String,
    
    /// Device identifier
    pub id: String,
    
    /// Supported resolutions
    pub resolutions: Vec<Resolution>,
    
    /// Supported frame rates
    pub frame_rates: Vec<f32>,
    
    /// Whether the device supports hardware encoding
    pub hw_encoding: bool,
    
    /// Default resolution
    pub default_resolution: Resolution,
    
    /// Default frame rate
    pub default_frame_rate: f32,
}

/// Video device information
#[derive(Debug, Clone)]
pub struct VideoDeviceInfo {
    /// Device name
    pub name: String,
    
    /// Device identifier
    pub id: String,
    
    /// Whether this is the default device
    pub is_default: bool,
}

/// Video device event
#[derive(Debug, Clone)]
pub enum VideoDeviceEvent {
    /// Device added
    Added(VideoDeviceInfo),
    
    /// Device removed
    Removed(String),
    
    /// Default device changed
    DefaultChanged(String),
    
    /// Device error
    Error(String, String),
}

/// Video device interface
pub trait VideoDevice: Send + Sync {
    /// Get device identifier
    fn id(&self) -> &str;
    
    /// Get device name
    fn name(&self) -> &str;
    
    /// Get device capabilities
    fn capabilities(&self) -> Result<VideoDeviceCapabilities>;
    
    /// Open the device with given configuration
    fn open(&mut self, resolution: Resolution, frame_rate: f32) -> Result<()>;
    
    /// Close the device
    fn close(&mut self) -> Result<()>;
    
    /// Check if device is open
    fn is_open(&self) -> bool;
    
    /// Get current resolution
    fn current_resolution(&self) -> Result<Resolution>;
    
    /// Get current frame rate
    fn current_frame_rate(&self) -> Result<f32>;
}

/// Video device manager
#[derive(Debug)]
pub struct VideoDeviceManager {
    /// Available devices
    devices: RwLock<HashMap<String, Arc<dyn VideoDevice + Send + Sync>>>,
    
    /// Default device ID
    default_device_id: RwLock<Option<String>>,
    
    /// Event sender
    event_tx: mpsc::Sender<VideoDeviceEvent>,
    
    /// Event receiver
    event_rx: mpsc::Receiver<VideoDeviceEvent>,
}

impl VideoDeviceManager {
    /// Create a new video device manager
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(32);
        
        Self {
            devices: RwLock::new(HashMap::new()),
            default_device_id: RwLock::new(None),
            event_tx: tx,
            event_rx: rx,
        }
    }
    
    /// Initialize the device manager and discover devices
    pub async fn initialize(&self) -> Result<()> {
        debug!("Initializing video device manager (stub implementation)");
        
        // In a real implementation, we would:
        // 1. Enumerate available video devices on the system
        // 2. Create VideoDevice objects for each
        // 3. Set up hotplug monitoring
        
        // Stub just creates a dummy device
        let device_id = "dummy_camera_id".to_string();
        let device_name = "Dummy Camera".to_string();
        
        // Create dummy device info
        let device_info = VideoDeviceInfo {
            name: device_name.clone(),
            id: device_id.clone(),
            is_default: true,
        };
        
        // Create dummy device
        let device = DummyVideoDevice::new(device_id.clone(), device_name);
        
        // Add device to map
        self.devices.write().await.insert(device_id.clone(), Arc::new(device));
        
        // Set as default
        *self.default_device_id.write().await = Some(device_id.clone());
        
        // Send device added event
        let _ = self.event_tx.send(VideoDeviceEvent::Added(device_info)).await;
        
        Ok(())
    }
    
    /// Get a list of available video devices
    pub async fn get_devices(&self) -> Vec<VideoDeviceInfo> {
        let devices = self.devices.read().await;
        let default_id = self.default_device_id.read().await;
        
        devices.iter().map(|(id, device)| {
            VideoDeviceInfo {
                name: device.name().to_string(),
                id: id.clone(),
                is_default: Some(id.as_str()) == default_id.as_deref(),
            }
        }).collect()
    }
    
    /// Get default video device
    pub async fn get_default_device(&self) -> Option<Arc<dyn VideoDevice + Send + Sync>> {
        let devices = self.devices.read().await;
        let default_id = self.default_device_id.read().await;
        
        if let Some(id) = &*default_id {
            devices.get(id).cloned()
        } else {
            None
        }
    }
    
    /// Get a specific video device by ID
    pub async fn get_device(&self, id: &str) -> Option<Arc<dyn VideoDevice + Send + Sync>> {
        self.devices.read().await.get(id).cloned()
    }
    
    /// Set the default video device
    pub async fn set_default_device(&self, id: &str) -> Result<()> {
        // Check if device exists
        if !self.devices.read().await.contains_key(id) {
            return Err(Error::DeviceNotFound(format!("Video device not found: {}", id)));
        }
        
        // Set default device
        *self.default_device_id.write().await = Some(id.to_string());
        
        // Send event
        let _ = self.event_tx.send(VideoDeviceEvent::DefaultChanged(id.to_string())).await;
        
        Ok(())
    }
    
    /// Get the event receiver clone
    pub fn events(&self) -> mpsc::Receiver<VideoDeviceEvent> {
        self.event_rx.clone()
    }
}

impl Default for VideoDeviceManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Dummy video device implementation for stub/testing
#[derive(Debug)]
struct DummyVideoDevice {
    /// Device ID
    id: String,
    
    /// Device name
    name: String,
    
    /// Whether the device is open
    is_open: bool,
    
    /// Current resolution
    current_resolution: Resolution,
    
    /// Current frame rate
    current_frame_rate: f32,
}

impl DummyVideoDevice {
    /// Create a new dummy video device
    fn new(id: String, name: String) -> Self {
        Self {
            id,
            name,
            is_open: false,
            current_resolution: Resolution::new(1280, 720),
            current_frame_rate: 30.0,
        }
    }
}

impl VideoDevice for DummyVideoDevice {
    fn id(&self) -> &str {
        &self.id
    }
    
    fn name(&self) -> &str {
        &self.name
    }
    
    fn capabilities(&self) -> Result<VideoDeviceCapabilities> {
        Ok(VideoDeviceCapabilities {
            name: self.name.clone(),
            id: self.id.clone(),
            resolutions: vec![
                Resolution::new(320, 240),
                Resolution::new(640, 480),
                Resolution::new(1280, 720),
                Resolution::new(1920, 1080),
            ],
            frame_rates: vec![15.0, 30.0, 60.0],
            hw_encoding: false,
            default_resolution: Resolution::new(1280, 720),
            default_frame_rate: 30.0,
        })
    }
    
    fn open(&mut self, resolution: Resolution, frame_rate: f32) -> Result<()> {
        self.is_open = true;
        self.current_resolution = resolution;
        self.current_frame_rate = frame_rate;
        Ok(())
    }
    
    fn close(&mut self) -> Result<()> {
        self.is_open = false;
        Ok(())
    }
    
    fn is_open(&self) -> bool {
        self.is_open
    }
    
    fn current_resolution(&self) -> Result<Resolution> {
        if !self.is_open {
            return Err(Error::InvalidState("Device not open".to_string()));
        }
        Ok(self.current_resolution)
    }
    
    fn current_frame_rate(&self) -> Result<f32> {
        if !self.is_open {
            return Err(Error::InvalidState("Device not open".to_string()));
        }
        Ok(self.current_frame_rate)
    }
} 