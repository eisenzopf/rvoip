//! Audio Device Manager
//!
//! This module provides the AudioDeviceManager that coordinates audio devices,
//! manages playback/capture sessions, and bridges with session-core for audio streaming.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tokio::task::JoinHandle;

use crate::call::CallId;
use crate::audio::device::{AudioDevice, AudioDeviceInfo, AudioDirection, AudioFormat, AudioFrame, AudioError, AudioResult};
use crate::audio::platform::{create_platform_device, list_platform_devices, get_default_platform_device};
use rvoip_session_core::api::MediaControl;

/// Audio playback session
/// 
/// Manages audio playback for a specific call, bridging between session-core
/// and the audio device.
pub struct PlaybackSession {
    /// Call ID this session is associated with
    pub call_id: CallId,
    /// Audio device being used for playback
    pub device: Arc<dyn AudioDevice>,
    /// Audio format being used
    pub format: AudioFormat,
    /// Channel sender for audio frames to be played
    pub frame_sender: mpsc::Sender<AudioFrame>,
    /// Background task handle
    pub task_handle: JoinHandle<()>,
}

impl PlaybackSession {
    /// Create a new playback session
    pub fn new(
        call_id: CallId,
        device: Arc<dyn AudioDevice>,
        format: AudioFormat,
        frame_sender: mpsc::Sender<AudioFrame>,
        task_handle: JoinHandle<()>,
    ) -> Self {
        Self {
            call_id,
            device,
            format,
            frame_sender,
            task_handle,
        }
    }
    
    /// Send an audio frame for playback
    pub async fn play_frame(&self, frame: AudioFrame) -> AudioResult<()> {
        self.frame_sender.send(frame).await
            .map_err(|_| AudioError::IoError { 
                message: "Failed to send audio frame for playback".to_string() 
            })
    }
    
    /// Stop the playback session
    pub async fn stop(self) -> AudioResult<()> {
        // Stop the device first
        self.device.stop_playback().await?;
        
        // Cancel the background task
        self.task_handle.abort();
        
        Ok(())
    }
}

/// Audio capture session
/// 
/// Manages audio capture for a specific call, bridging between the audio device
/// and session-core.
pub struct CaptureSession {
    /// Call ID this session is associated with
    pub call_id: CallId,
    /// Audio device being used for capture
    pub device: Arc<dyn AudioDevice>,
    /// Audio format being used
    pub format: AudioFormat,
    /// Channel receiver for audio frames from the device
    pub frame_receiver: mpsc::Receiver<AudioFrame>,
    /// Background task handle
    pub task_handle: JoinHandle<()>,
}

impl CaptureSession {
    /// Create a new capture session
    pub fn new(
        call_id: CallId,
        device: Arc<dyn AudioDevice>,
        format: AudioFormat,
        frame_receiver: mpsc::Receiver<AudioFrame>,
        task_handle: JoinHandle<()>,
    ) -> Self {
        Self {
            call_id,
            device,
            format,
            frame_receiver,
            task_handle,
        }
    }
    
    /// Get the next captured audio frame
    pub async fn next_frame(&mut self) -> Option<AudioFrame> {
        self.frame_receiver.recv().await
    }
    
    /// Stop the capture session
    pub async fn stop(self) -> AudioResult<()> {
        // Stop the device first
        self.device.stop_capture().await?;
        
        // Cancel the background task
        self.task_handle.abort();
        
        Ok(())
    }
}

/// Audio Device Manager
/// 
/// Manages audio devices, coordinates playback/capture sessions, and integrates
/// with session-core for audio streaming.
pub struct AudioDeviceManager {
    /// Active playback sessions by call ID
    playback_sessions: RwLock<HashMap<CallId, PlaybackSession>>,
    /// Active capture sessions by call ID
    capture_sessions: RwLock<HashMap<CallId, CaptureSession>>,
    /// Session-core coordinator for audio streaming integration
    session_coordinator: Option<Arc<rvoip_session_core::coordinator::SessionCoordinator>>,
}

impl AudioDeviceManager {
    /// Create a new audio device manager
    pub async fn new() -> AudioResult<Self> {
        Ok(Self {
            playback_sessions: RwLock::new(HashMap::new()),
            capture_sessions: RwLock::new(HashMap::new()),
            session_coordinator: None,
        })
    }
    
    /// Set the session-core coordinator for audio streaming integration
    pub async fn set_session_coordinator(&mut self, coordinator: Arc<rvoip_session_core::coordinator::SessionCoordinator>) {
        self.session_coordinator = Some(coordinator);
    }
    
    /// Find a supported audio format for the given device
    async fn find_supported_format(&self, device: &Arc<dyn AudioDevice>) -> AudioResult<AudioFormat> {
        let info = device.info();
        
        // Try common formats in order of preference
        let test_formats = vec![
            AudioFormat::default_voip(),    // 8000 Hz
            AudioFormat::wideband_voip(),   // 16000 Hz  
            AudioFormat::new(44100, 1, 16, 20),  // 44.1 kHz
            AudioFormat::new(48000, 1, 16, 20),  // 48 kHz
            AudioFormat::new(44100, 2, 16, 20),  // 44.1 kHz stereo
            AudioFormat::new(48000, 2, 16, 20),  // 48 kHz stereo
        ];
        
        for format in test_formats {
            if device.supports_format(&format) {
                return Ok(format);
            }
        }
        
        // If none of the common formats work, create one from device capabilities
        let sample_rate = info.supported_sample_rates.first()
            .ok_or_else(|| AudioError::DeviceNotFound { device_id: info.id.clone() })?;
        let channels = info.supported_channels.first()
            .ok_or_else(|| AudioError::DeviceNotFound { device_id: info.id.clone() })?;
        
        Ok(AudioFormat::new(*sample_rate, *channels, 16, 20))
    }
    
    /// List available audio devices
    pub async fn list_devices(&self, direction: AudioDirection) -> AudioResult<Vec<AudioDeviceInfo>> {
        list_platform_devices(direction).await
    }
    
    /// Get the default audio device for the specified direction
    pub async fn get_default_device(&self, direction: AudioDirection) -> AudioResult<Arc<dyn AudioDevice>> {
        get_default_platform_device(direction).await
    }
    
    /// Create a specific audio device by ID
    pub async fn create_device(&self, device_id: &str) -> AudioResult<Arc<dyn AudioDevice>> {
        create_platform_device(device_id).await
    }
    
    /// Start audio playback for a call
    /// 
    /// This sets up the audio pipeline: session-core → AudioDeviceManager → audio device
    pub async fn start_playback(&self, call_id: &CallId, device: Arc<dyn AudioDevice>) -> AudioResult<()> {
        // Find a supported format
        let format = self.find_supported_format(&device).await?;
        
        // Start playback on the device
        let frame_sender = device.start_playback(format.clone()).await?;
        
        // Set up session-core integration if available
        if let Some(session_coordinator) = &self.session_coordinator {
            let session_id = rvoip_session_core::api::types::SessionId::new();
            
            // Subscribe to audio frames from session-core
            let audio_subscriber = session_coordinator.subscribe_to_audio_frames(&session_id).await
                .map_err(|e| AudioError::ConfigurationError { 
                    message: format!("Failed to subscribe to audio frames: {}", e) 
                })?;
            
            // Create bridge task to forward frames from session-core to device
            let frame_sender_clone = frame_sender.clone();
            let bridge_task = tokio::spawn(async move {
                loop {
                    // In a real implementation, this would receive frames from session-core
                    // For now, we'll just keep the task alive
                    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
                }
            });
            
            // Create playback session
            let session = PlaybackSession::new(
                call_id.clone(),
                device,
                format,
                frame_sender,
                bridge_task,
            );
            
            // Store the session
            let mut sessions = self.playback_sessions.write().await;
            sessions.insert(call_id.clone(), session);
        } else {
            // No session-core integration, create minimal session
            let task = tokio::spawn(async move {
                // Keep the session alive
                tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
            });
            
            let session = PlaybackSession::new(
                call_id.clone(),
                device,
                format,
                frame_sender,
                task,
            );
            
            let mut sessions = self.playback_sessions.write().await;
            sessions.insert(call_id.clone(), session);
        }
        
        Ok(())
    }
    
    /// Stop audio playback for a call
    pub async fn stop_playback(&self, call_id: &CallId) -> AudioResult<()> {
        let mut sessions = self.playback_sessions.write().await;
        if let Some(session) = sessions.remove(call_id) {
            session.stop().await?;
        }
        Ok(())
    }
    
    /// Start audio capture for a call
    /// 
    /// This sets up the audio pipeline: audio device → AudioDeviceManager → session-core
    pub async fn start_capture(&self, call_id: &CallId, device: Arc<dyn AudioDevice>) -> AudioResult<()> {
        // Find a supported format
        let format = self.find_supported_format(&device).await?;
        
        // Start capture on the device
        let frame_receiver = device.start_capture(format.clone()).await?;
        
        // Set up session-core integration if available
        if let Some(session_coordinator) = &self.session_coordinator {
            let session_id = rvoip_session_core::api::types::SessionId::new();
            
            // Create bridge task to forward frames from device to session-core
            let session_coordinator_clone = session_coordinator.clone();
            let bridge_task = tokio::spawn(async move {
                let mut rx = frame_receiver;
                while let Some(device_frame) = rx.recv().await {
                    // Convert device AudioFrame to session-core AudioFrame
                    let session_frame = device_frame.to_session_core();
                    
                    // Send to session-core (ignore errors as call may have ended)
                    let _ = session_coordinator_clone.send_audio_frame(&session_id, session_frame).await;
                }
            });
            
            // Create capture session with dummy receiver since frames are handled by bridge task
            let (dummy_tx, dummy_rx) = mpsc::channel(1);
            drop(dummy_tx); // Close immediately
            
            let session = CaptureSession::new(
                call_id.clone(),
                device,
                format,
                dummy_rx,
                bridge_task,
            );
            
            // Store the session
            let mut sessions = self.capture_sessions.write().await;
            sessions.insert(call_id.clone(), session);
        } else {
            // No session-core integration, create minimal session
            let task = tokio::spawn(async move {
                // Keep the session alive
                tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
            });
            
            let session = CaptureSession::new(
                call_id.clone(),
                device,
                format,
                frame_receiver,
                task,
            );
            
            let mut sessions = self.capture_sessions.write().await;
            sessions.insert(call_id.clone(), session);
        }
        
        Ok(())
    }
    
    /// Stop audio capture for a call
    pub async fn stop_capture(&self, call_id: &CallId) -> AudioResult<()> {
        let mut sessions = self.capture_sessions.write().await;
        if let Some(session) = sessions.remove(call_id) {
            session.stop().await?;
        }
        Ok(())
    }
    
    /// Check if playback is active for a call
    pub async fn is_playback_active(&self, call_id: &CallId) -> bool {
        let sessions = self.playback_sessions.read().await;
        sessions.contains_key(call_id)
    }
    
    /// Check if capture is active for a call
    pub async fn is_capture_active(&self, call_id: &CallId) -> bool {
        let sessions = self.capture_sessions.read().await;
        sessions.contains_key(call_id)
    }
    
    /// Get active playback sessions
    pub async fn get_active_playback_sessions(&self) -> Vec<CallId> {
        let sessions = self.playback_sessions.read().await;
        sessions.keys().cloned().collect()
    }
    
    /// Get active capture sessions
    pub async fn get_active_capture_sessions(&self) -> Vec<CallId> {
        let sessions = self.capture_sessions.read().await;
        sessions.keys().cloned().collect()
    }
    
    /// Stop all audio sessions
    pub async fn stop_all_sessions(&self) -> AudioResult<()> {
        // Stop all playback sessions
        let mut playback_sessions = self.playback_sessions.write().await;
        for (_, session) in playback_sessions.drain() {
            if let Err(e) = session.stop().await {
                eprintln!("Failed to stop playback session: {}", e);
            }
        }
        
        // Stop all capture sessions
        let mut capture_sessions = self.capture_sessions.write().await;
        for (_, session) in capture_sessions.drain() {
            if let Err(e) = session.stop().await {
                eprintln!("Failed to stop capture session: {}", e);
            }
        }
        
        Ok(())
    }
} 