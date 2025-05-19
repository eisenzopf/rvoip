//! Video capture implementation
//!
//! This module provides the functionality for capturing video from cameras and other sources.

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tokio::time::sleep;
use tracing::{debug, error, info, warn};
use bytes::Bytes;

use crate::error::{Error, Result};
use crate::codec::video::{Resolution, VideoFrame, VideoFrameType};
use crate::engine::video::device::{VideoDevice, VideoDeviceManager};

/// Video capture configuration
#[derive(Debug, Clone)]
pub struct VideoCaptureConfig {
    /// Target resolution
    pub resolution: Resolution,
    
    /// Target frame rate
    pub frame_rate: f32,
    
    /// Enable hardware acceleration if available
    pub hw_acceleration: bool,
    
    /// Maximum bitrate in bits per second
    pub max_bitrate: Option<u32>,
    
    /// Low light enhancement
    pub low_light_enhancement: bool,
    
    /// Auto focus
    pub auto_focus: bool,
}

impl Default for VideoCaptureConfig {
    fn default() -> Self {
        Self {
            resolution: Resolution::new(1280, 720),
            frame_rate: 30.0,
            hw_acceleration: true,
            max_bitrate: None,
            low_light_enhancement: false,
            auto_focus: true,
        }
    }
}

/// Video source interface
pub trait VideoSource: Send + Sync {
    /// Start capturing
    fn start(&mut self) -> Result<()>;
    
    /// Stop capturing
    fn stop(&mut self) -> Result<()>;
    
    /// Check if capturing is active
    fn is_capturing(&self) -> bool;
    
    /// Get the current frame rate
    fn frame_rate(&self) -> f32;
    
    /// Get the current resolution
    fn resolution(&self) -> Resolution;
}

/// Video capture implementation
#[derive(Debug)]
pub struct VideoCapture {
    /// Configuration
    config: VideoCaptureConfig,
    
    /// Video device
    device: Arc<RwLock<Option<Arc<dyn VideoDevice + Send + Sync>>>>,
    
    /// Capture is running
    running: Arc<RwLock<bool>>,
    
    /// Frame sender
    frame_tx: mpsc::Sender<VideoFrame>,
    
    /// Frame receiver
    frame_rx: mpsc::Receiver<VideoFrame>,
    
    /// Capture thread
    capture_task: Option<tokio::task::JoinHandle<()>>,
}

impl VideoCapture {
    /// Create a new video capture instance
    pub fn new(config: VideoCaptureConfig) -> Self {
        let (tx, rx) = mpsc::channel(10);
        
        Self {
            config,
            device: Arc::new(RwLock::new(None)),
            running: Arc::new(RwLock::new(false)),
            frame_tx: tx,
            frame_rx: rx,
            capture_task: None,
        }
    }
    
    /// Create a video capture with default configuration
    pub fn default() -> Self {
        Self::new(VideoCaptureConfig::default())
    }
    
    /// Set video device
    pub async fn set_device(&self, device: Arc<dyn VideoDevice + Send + Sync>) -> Result<()> {
        let mut device_guard = self.device.write().await;
        
        // Close previous device if any
        if let Some(prev_device) = &mut *device_guard {
            if let Ok(mut prev_device) = Arc::get_mut(prev_device) {
                if prev_device.is_open() {
                    prev_device.close()?;
                }
            }
        }
        
        // Store new device
        *device_guard = Some(device);
        
        Ok(())
    }
    
    /// Start capturing
    pub async fn start(&mut self) -> Result<()> {
        let device_guard = self.device.read().await;
        let device = device_guard.as_ref().ok_or_else(|| {
            Error::InvalidState("No video device set".to_string())
        })?;
        
        // Check if already running
        if *self.running.read().await {
            return Ok(());
        }
        
        // Get a mutable reference if possible
        if let Ok(mut device_mut) = Arc::get_mut(device) {
            // Open device with configured resolution and frame rate
            device_mut.open(self.config.resolution, self.config.frame_rate)?;
        }
        
        // Mark as running
        *self.running.write().await = true;
        
        // Clone handles for capture task
        let running = self.running.clone();
        let frame_tx = self.frame_tx.clone();
        let config = self.config.clone();
        
        // Start capture task
        self.capture_task = Some(tokio::spawn(async move {
            let frame_interval = Duration::from_secs_f32(1.0 / config.frame_rate);
            
            // Dummy capture loop that just generates frames at the specified rate
            while *running.read().await {
                // Sleep for frame interval
                sleep(frame_interval).await;
                
                // Create dummy frame
                let frame = VideoFrame {
                    data: Bytes::from(vec![0u8; config.resolution.width as usize * config.resolution.height as usize * 3 / 2]),
                    resolution: config.resolution,
                    frame_type: VideoFrameType::Key,
                    timestamp_ms: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64,
                };
                
                // Send frame
                if frame_tx.send(frame).await.is_err() {
                    break;
                }
            }
        }));
        
        Ok(())
    }
    
    /// Stop capturing
    pub async fn stop(&mut self) -> Result<()> {
        // Check if running
        if !*self.running.read().await {
            return Ok(());
        }
        
        // Mark as not running
        *self.running.write().await = false;
        
        // Wait for capture task to finish
        if let Some(task) = self.capture_task.take() {
            let _ = task.await;
        }
        
        // Close device
        let device_guard = self.device.read().await;
        if let Some(device) = &*device_guard {
            if let Ok(mut device_mut) = Arc::get_mut(device) {
                if device_mut.is_open() {
                    device_mut.close()?;
                }
            }
        }
        
        Ok(())
    }
    
    /// Get the frame receiver
    pub fn frames(&self) -> mpsc::Receiver<VideoFrame> {
        self.frame_rx.clone()
    }
    
    /// Check if capturing is active
    pub async fn is_capturing(&self) -> bool {
        *self.running.read().await
    }
    
    /// Get the current configuration
    pub fn config(&self) -> &VideoCaptureConfig {
        &self.config
    }
    
    /// Update the configuration
    pub async fn set_config(&mut self, config: VideoCaptureConfig) -> Result<()> {
        let was_running = *self.running.read().await;
        
        // Stop if running
        if was_running {
            self.stop().await?;
        }
        
        // Update config
        self.config = config;
        
        // Restart if was running
        if was_running {
            self.start().await?;
        }
        
        Ok(())
    }
}

impl Drop for VideoCapture {
    fn drop(&mut self) {
        let running = self.running.clone();
        tokio::spawn(async move {
            *running.write().await = false;
        });
    }
} 