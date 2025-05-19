//! Video rendering implementation
//!
//! This module provides functionality for rendering video frames to displays or other output devices.

use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};

use crate::error::{Error, Result};
use crate::codec::video::{Resolution, VideoFrame};

/// Video render configuration
#[derive(Debug, Clone)]
pub struct VideoRenderConfig {
    /// Target resolution
    pub resolution: Resolution,
    
    /// Scaling quality (0.0-1.0, where 1.0 is highest quality)
    pub scaling_quality: f32,
    
    /// Enable hardware acceleration if available
    pub hw_acceleration: bool,
    
    /// Enable deinterlacing
    pub deinterlace: bool,
    
    /// Fullscreen mode
    pub fullscreen: bool,
    
    /// Display index (0 = primary display)
    pub display_index: u32,
}

impl Default for VideoRenderConfig {
    fn default() -> Self {
        Self {
            resolution: Resolution::new(1280, 720),
            scaling_quality: 0.8,
            hw_acceleration: true,
            deinterlace: false,
            fullscreen: false,
            display_index: 0,
        }
    }
}

/// Video sink interface
pub trait VideoSink: Send + Sync {
    /// Process a video frame
    fn process_frame(&mut self, frame: &VideoFrame) -> Result<()>;
    
    /// Get the render resolution
    fn resolution(&self) -> Resolution;
    
    /// Reset the sink
    fn reset(&mut self) -> Result<()>;
}

/// Video renderer implementation
#[derive(Debug)]
pub struct VideoRenderer {
    /// Configuration
    config: VideoRenderConfig,
    
    /// Renderer is running
    running: Arc<RwLock<bool>>,
    
    /// Frame receiver
    frame_rx: mpsc::Receiver<VideoFrame>,
    
    /// Frame sender
    frame_tx: mpsc::Sender<VideoFrame>,
    
    /// Render task
    render_task: Option<tokio::task::JoinHandle<()>>,
}

impl VideoRenderer {
    /// Create a new video renderer
    pub fn new(config: VideoRenderConfig) -> Self {
        let (tx, rx) = mpsc::channel(10);
        
        Self {
            config,
            running: Arc::new(RwLock::new(false)),
            frame_rx: rx,
            frame_tx: tx,
            render_task: None,
        }
    }
    
    /// Create a video renderer with default configuration
    pub fn default() -> Self {
        Self::new(VideoRenderConfig::default())
    }
    
    /// Start the renderer
    pub async fn start(&mut self) -> Result<()> {
        // Check if already running
        if *self.running.read().await {
            return Ok(());
        }
        
        // Mark as running
        *self.running.write().await = true;
        
        // Clone handles for render task
        let running = self.running.clone();
        let mut frame_rx = self.frame_rx.clone();
        let config = self.config.clone();
        
        // Start render task
        self.render_task = Some(tokio::spawn(async move {
            debug!("Video renderer started with resolution: {}x{}", 
                   config.resolution.width, config.resolution.height);
            
            // Dummy render loop that just processes frames
            while *running.read().await {
                // Wait for frame
                if let Some(frame) = frame_rx.recv().await {
                    // In a real implementation, we would render the frame to a display
                    // For the stub, just log info about it
                    debug!("Rendered frame: {}x{}, type: {:?}, timestamp: {}ms", 
                          frame.resolution.width, frame.resolution.height, 
                          frame.frame_type, frame.timestamp_ms);
                }
            }
            
            debug!("Video renderer stopped");
        }));
        
        Ok(())
    }
    
    /// Stop the renderer
    pub async fn stop(&mut self) -> Result<()> {
        // Check if running
        if !*self.running.read().await {
            return Ok(());
        }
        
        // Mark as not running
        *self.running.write().await = false;
        
        // Wait for render task to finish
        if let Some(task) = self.render_task.take() {
            let _ = task.await;
        }
        
        Ok(())
    }
    
    /// Send a frame to the renderer
    pub async fn render_frame(&self, frame: VideoFrame) -> Result<()> {
        // Check if running
        if !*self.running.read().await {
            return Err(Error::InvalidState("Renderer not running".to_string()));
        }
        
        // Send frame
        self.frame_tx.send(frame).await
            .map_err(|_| Error::ChannelSendError("Failed to send frame to renderer".to_string()))?;
        
        Ok(())
    }
    
    /// Check if rendering is active
    pub async fn is_running(&self) -> bool {
        *self.running.read().await
    }
    
    /// Get the current configuration
    pub fn config(&self) -> &VideoRenderConfig {
        &self.config
    }
    
    /// Update the configuration
    pub async fn set_config(&mut self, config: VideoRenderConfig) -> Result<()> {
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

impl Drop for VideoRenderer {
    fn drop(&mut self) {
        let running = self.running.clone();
        tokio::spawn(async move {
            *running.write().await = false;
        });
    }
}

/// Simple screen renderer implementation
#[derive(Debug)]
pub struct ScreenRenderer {
    /// Configuration
    config: VideoRenderConfig,
    
    /// Last frame timestamp
    last_frame_ts: u64,
    
    /// Frames processed
    frames_processed: u64,
}

impl ScreenRenderer {
    /// Create a new screen renderer
    pub fn new(config: VideoRenderConfig) -> Self {
        Self {
            config,
            last_frame_ts: 0,
            frames_processed: 0,
        }
    }
    
    /// Get frames processed
    pub fn frames_processed(&self) -> u64 {
        self.frames_processed
    }
}

impl VideoSink for ScreenRenderer {
    fn process_frame(&mut self, frame: &VideoFrame) -> Result<()> {
        // In a real implementation, we would render the frame to a display
        // For the stub, just increment counters
        self.last_frame_ts = frame.timestamp_ms;
        self.frames_processed += 1;
        
        Ok(())
    }
    
    fn resolution(&self) -> Resolution {
        self.config.resolution
    }
    
    fn reset(&mut self) -> Result<()> {
        self.last_frame_ts = 0;
        self.frames_processed = 0;
        Ok(())
    }
} 