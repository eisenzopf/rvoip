//! Advanced SIP client API for fine-grained control
//!
//! This module provides advanced APIs for applications that need:
//! - Custom audio pipeline configuration
//! - Direct access to audio frames
//! - Manual codec selection
//! - Advanced call control features

use crate::{
    error::{SipClientError, SipClientResult},
    events::{EventEmitter, SipClientEvent, EventStream},
    types::{Call, CallId, CallState, CallDirection, AudioQualityMetrics},
};
use parking_lot::RwLock as ParkingRwLock;
use rvoip_client_core::{
    Client as CoreClient,
    ClientBuilder as CoreClientBuilder,
    ClientEventHandler,
    error::ClientError,
};
use rvoip_audio_core::{
    AudioDeviceManager,
    pipeline::AudioPipelineBuilder,
    AudioFormat,
    AudioFrame,
    AudioDirection,
    AudioDeviceInfo,
};
use rvoip_session_core::{types::AudioFrame as SessionAudioFrame, SessionId};
use codec_core::CodecType;
use std::{
    sync::Arc,
    collections::HashMap,
};
use tokio::sync::{Mutex, RwLock, mpsc, oneshot};
use tokio_stream::{Stream, StreamExt};
use futures::stream::BoxStream;
use tracing::{debug, error, info, warn};

/// Advanced SIP client with full control over audio pipeline and media
pub struct AdvancedSipClient {
    /// The underlying client-core instance
    core_client: Arc<CoreClient>,
    
    /// Audio device manager
    audio_manager: Arc<AudioDeviceManager>,
    
    /// Event emitter for distributing events
    event_emitter: EventEmitter,
    
    /// Active calls with their audio pipeline state
    calls: Arc<RwLock<HashMap<CallId, AdvancedCallState>>>,
    
    /// Audio pipeline configuration
    pipeline_config: Arc<RwLock<AudioPipelineConfig>>,
    
    /// Media preferences
    media_prefs: Arc<RwLock<MediaPreferences>>,
    
    /// Client state
    started: Arc<Mutex<bool>>,
}

/// Advanced call state with audio pipeline access
struct AdvancedCallState {
    /// The call handle
    call: Arc<Call>,
    
    /// Core call ID
    core_call_id: rvoip_client_core::CallId,
    
    /// Audio frame channels for custom processing
    frame_channels: Option<FrameChannels>,
    
    /// Pipeline task handles
    pipeline_tasks: Vec<tokio::task::JoinHandle<()>>,
    
    /// Custom audio pipeline if configured
    custom_pipeline: Option<Arc<rvoip_audio_core::AudioPipeline>>,
}

/// Frame channels for bidirectional audio processing
struct FrameChannels {
    /// Channel for sending processed frames to network
    to_network_tx: mpsc::Sender<AudioFrame>,
    to_network_rx: Option<mpsc::Receiver<AudioFrame>>,
    
    /// Channel for receiving frames from network
    from_network_tx: mpsc::Sender<AudioFrame>,
    from_network_rx: Option<mpsc::Receiver<AudioFrame>>,
}

/// Configuration for custom audio pipelines
#[derive(Clone, Debug)]
pub struct AudioPipelineConfig {
    /// Input device ID (None for default)
    pub input_device: Option<String>,
    
    /// Output device ID (None for default)
    pub output_device: Option<String>,
    
    /// Enable echo cancellation
    pub echo_cancellation: bool,
    
    /// Enable noise suppression
    pub noise_suppression: bool,
    
    /// Enable automatic gain control
    pub auto_gain_control: bool,
    
    /// Custom pipeline components
    pub custom_processors: Vec<AudioProcessor>,
    
    /// Buffer size in frames
    pub buffer_size: usize,
    
    /// Enable direct frame access
    pub enable_frame_access: bool,
}

/// Custom audio processor trait
pub trait AudioProcessorTrait: Send + Sync {
    /// Process an audio frame
    fn process(&mut self, frame: &mut AudioFrame);
    
    /// Get processor name
    fn name(&self) -> &str;
}

/// Wrapper for custom audio processors
#[derive(Clone)]
pub struct AudioProcessor {
    processor: Arc<dyn AudioProcessorTrait>,
}

impl std::fmt::Debug for AudioProcessor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioProcessor")
            .field("name", &self.processor.name())
            .finish()
    }
}

/// Codec priority configuration
#[derive(Clone, Debug)]
pub struct CodecPriority {
    /// Codec name (e.g., "PCMU", "PCMA")
    pub name: String,
    
    /// Priority (higher is preferred)
    pub priority: u8,
}

/// Media preferences configuration
#[derive(Clone, Debug)]
pub struct MediaPreferences {
    /// Preferred codecs in priority order
    pub codecs: Vec<CodecPriority>,
    
    /// Custom SDP attributes
    pub sdp_attributes: HashMap<String, String>,
    
    /// Jitter buffer size in milliseconds
    pub jitter_buffer_ms: u32,
    
    /// Enable DTMF detection
    pub dtmf_detection: bool,
    
    /// Enable comfort noise generation
    pub comfort_noise: bool,
}

/// Audio stream for frame-level access
pub struct AudioStream {
    /// Call ID this stream belongs to
    call_id: CallId,
    
    /// Receiver for incoming frames from network
    from_network: mpsc::Receiver<AudioFrame>,
    
    /// Sender for processed frames to network
    to_network: mpsc::Sender<AudioFrame>,
}

/// Call statistics
#[derive(Debug, Clone)]
pub struct CallStatistics {
    /// Audio quality metrics
    pub audio_metrics: AudioQualityMetrics,
    
    /// Total packets sent
    pub packets_sent: u64,
    
    /// Total packets received
    pub packets_received: u64,
    
    /// Total bytes sent
    pub bytes_sent: u64,
    
    /// Total bytes received
    pub bytes_received: u64,
    
    /// Codec in use
    pub codec: String,
}

impl AudioStream {
    /// Send a processed audio frame to the network
    pub async fn send(&mut self, frame: AudioFrame) -> SipClientResult<()> {
        self.to_network.send(frame).await
            .map_err(|_| SipClientError::AudioPipelineError {
                operation: "send_frame".to_string(),
                details: "Audio pipeline closed".to_string(),
            })
    }
    
    /// Receive the next audio frame from the network
    pub async fn recv(&mut self) -> Option<AudioFrame> {
        self.from_network.recv().await
    }
}

impl Stream for AudioStream {
    type Item = AudioFrame;
    
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.from_network.poll_recv(cx)
    }
}

impl AdvancedSipClient {
    /// Create a new advanced SIP client with custom configuration
    pub async fn new(
        sip_uri: &str,
        pipeline_config: AudioPipelineConfig,
        media_prefs: MediaPreferences,
    ) -> SipClientResult<Self> {
        debug!("Creating advanced SIP client for {}", sip_uri);
        
        // Create audio device manager
        let audio_manager = AudioDeviceManager::new().await
            .map_err(|e| SipClientError::AudioPipelineError { 
                operation: "init".to_string(),
                details: e.to_string() 
            })?;
        
        // For now, just store the URI as a string
        // TODO: Add proper URI parsing when available
        
        // Create client-core instance
        let core_client = CoreClientBuilder::new()
            .build().await
            .map_err(|e| SipClientError::Internal { 
                message: format!("Failed to build client: {}", e) 
            })?;
        
        let event_emitter = EventEmitter::default();
        
        Ok(Self {
            core_client,
            audio_manager: Arc::new(audio_manager),
            event_emitter,
            calls: Arc::new(RwLock::new(HashMap::new())),
            pipeline_config: Arc::new(RwLock::new(pipeline_config)),
            media_prefs: Arc::new(RwLock::new(media_prefs)),
            started: Arc::new(Mutex::new(false)),
        })
    }
    
    /// Start the client
    pub async fn start(&self) -> SipClientResult<()> {
        let mut started = self.started.lock().await;
        if *started {
            return Err(SipClientError::InvalidState {
                message: "Client already started, expected stopped state for start operation".to_string(),
            });
        }
        
        info!("Starting advanced SIP client");
        
        // Set up event forwarding
        self.setup_event_forwarding().await?;
        
        // Start the core client
        self.core_client.start().await
            .map_err(|e| SipClientError::Internal { 
                message: format!("Failed to start client: {}", e) 
            })?;
        
        *started = true;
        self.event_emitter.emit(SipClientEvent::Started);
        
        Ok(())
    }
    
    /// Stop the client
    pub async fn stop(&self) -> SipClientResult<()> {
        let mut started = self.started.lock().await;
        if !*started {
            return Ok(());
        }
        
        info!("Stopping advanced SIP client");
        
        // Clean up all active calls
        let call_ids: Vec<CallId> = {
            let calls = self.calls.read().await;
            calls.keys().cloned().collect()
        };
        
        for call_id in call_ids {
            let _ = self.hangup(&call_id).await;
        }
        
        // Stop the core client
        self.core_client.stop().await
            .map_err(|e| SipClientError::Internal { 
                message: format!("Failed to stop client: {}", e) 
            })?;
        
        *started = false;
        self.event_emitter.emit(SipClientEvent::Stopped);
        
        Ok(())
    }
    
    /// Make an outgoing call with custom audio pipeline
    pub async fn call(&self, uri: &str) -> SipClientResult<Arc<Call>> {
        // Ensure client is started
        if !*self.started.lock().await {
            return Err(SipClientError::InvalidState {
                message: "Client is stopped, expected started state for call operation".to_string(),
            });
        }
        
        debug!("Making call to {}", uri);
        
        // For now, just use the URI as a string
        // TODO: Add proper URI parsing when available
        let target_uri = uri.to_string();
        
        // Make the call via client-core
        // TODO: Use actual client-core call API when available
        let core_call_id = CallId::new_v4();
        
        // Create our call wrapper
        let call_id = CallId::new_v4();
        let call = Arc::new(Call {
            id: call_id,
            state: Arc::new(ParkingRwLock::new(CallState::Initiating)),
            remote_uri: uri.to_string(),
            local_uri: "sip:user@example.com".to_string(), // TODO: Get from client config
            start_time: chrono::Utc::now(),
            connect_time: None,
            codec: None,
            direction: CallDirection::Outgoing,
        });
        
        // Create frame channels if frame access is enabled
        let frame_channels = if self.pipeline_config.read().await.enable_frame_access {
            let (to_net_tx, to_net_rx) = mpsc::channel(100);
            let (from_net_tx, from_net_rx) = mpsc::channel(100);
            
            Some(FrameChannels {
                to_network_tx: to_net_tx,
                to_network_rx: Some(to_net_rx),
                from_network_tx: from_net_tx,
                from_network_rx: Some(from_net_rx),
            })
        } else {
            None
        };
        
        // Store call state
        let mut calls = self.calls.write().await;
        calls.insert(call_id, AdvancedCallState {
            call: call.clone(),
            core_call_id,
            frame_channels,
            pipeline_tasks: Vec::new(),
            custom_pipeline: None,
        });
        
        // Set up custom audio pipeline if configured
        self.setup_custom_pipeline(&call_id).await?;
        
        // Emit event
        self.event_emitter.emit(SipClientEvent::OutgoingCall { 
            call: call.clone() 
        });
        
        Ok(call)
    }
    
    /// Get audio stream for a call (for frame-level processing)
    pub async fn get_audio_stream(&self, call_id: &CallId) -> SipClientResult<AudioStream> {
        let config = self.pipeline_config.read().await;
        if !config.enable_frame_access {
            return Err(SipClientError::Configuration {
                message: "Frame access not enabled in pipeline config".to_string(),
            });
        }
        
        let mut calls = self.calls.write().await;
        let call_state = calls.get_mut(call_id)
            .ok_or_else(|| SipClientError::CallNotFound { 
                call_id: call_id.to_string() 
            })?;
        
        let frame_channels = call_state.frame_channels.as_mut()
            .ok_or_else(|| SipClientError::AudioPipelineError {
                operation: "get_audio_stream".to_string(),
                details: "Frame channels not initialized".to_string(),
            })?;
        
        // Take the receivers (they can only be taken once)
        let from_network = frame_channels.from_network_rx.take()
            .ok_or_else(|| SipClientError::AudioPipelineError {
                operation: "get_audio_stream".to_string(),
                details: "Audio stream already taken".to_string(),
            })?;
        
        Ok(AudioStream {
            call_id: *call_id,
            from_network,
            to_network: frame_channels.to_network_tx.clone(),
        })
    }
    
    /// Update audio pipeline configuration
    pub async fn update_pipeline_config(&self, config: AudioPipelineConfig) -> SipClientResult<()> {
        let mut pipeline_config = self.pipeline_config.write().await;
        *pipeline_config = config;
        
        // Apply changes to active calls
        let calls = self.calls.read().await;
        for (call_id, _) in calls.iter() {
            self.reconfigure_pipeline(call_id).await?;
        }
        
        Ok(())
    }
    
    /// Update media preferences
    pub async fn update_media_preferences(&self, prefs: MediaPreferences) -> SipClientResult<()> {
        let mut media_prefs = self.media_prefs.write().await;
        *media_prefs = prefs;
        
        // In a real implementation, we would apply these preferences
        // to client-core for future calls
        
        Ok(())
    }
    
    /// Transfer a call to another party
    pub async fn transfer_call(
        &self, 
        call_id: &CallId, 
        target_uri: &str
    ) -> SipClientResult<()> {
        let calls = self.calls.read().await;
        let call_state = calls.get(call_id)
            .ok_or_else(|| SipClientError::CallNotFound { 
                call_id: call_id.to_string() 
            })?;
        
        // For now, just use the URI as a string
        // TODO: Add proper URI parsing when available
        
        // Perform transfer via client-core
        // TODO: client-core doesn't expose transfer API yet
        return Err(SipClientError::NotImplemented {
            feature: "call_transfer".to_string(),
        });
        
        self.event_emitter.emit(SipClientEvent::CallTransferred { 
            call: call_state.call.clone(),
            target: target_uri.to_string(),
        });
        
        Ok(())
    }
    
    /// Put a call on hold
    pub async fn hold_call(&self, call_id: &CallId) -> SipClientResult<()> {
        let calls = self.calls.read().await;
        let call_state = calls.get(call_id)
            .ok_or_else(|| SipClientError::CallNotFound { 
                call_id: call_id.to_string() 
            })?;
        
        // Hold via client-core
        // TODO: client-core doesn't expose hold API yet
        return Err(SipClientError::NotImplemented {
            feature: "call_hold".to_string(),
        });
        
        // Pause audio pipeline
        if let Some(pipeline) = &call_state.custom_pipeline {
            // In a real implementation, we would pause the pipeline
        }
        
        self.event_emitter.emit(SipClientEvent::CallOnHold { 
            call: call_state.call.clone() 
        });
        
        Ok(())
    }
    
    /// Resume a held call
    pub async fn resume_call(&self, call_id: &CallId) -> SipClientResult<()> {
        let calls = self.calls.read().await;
        let call_state = calls.get(call_id)
            .ok_or_else(|| SipClientError::CallNotFound { 
                call_id: call_id.to_string() 
            })?;
        
        // Resume via client-core
        // TODO: client-core doesn't expose resume API yet
        return Err(SipClientError::NotImplemented {
            feature: "call_resume".to_string(),
        });
        
        // Resume audio pipeline
        if let Some(pipeline) = &call_state.custom_pipeline {
            // In a real implementation, we would resume the pipeline
        }
        
        self.event_emitter.emit(SipClientEvent::CallResumed { 
            call: call_state.call.clone() 
        });
        
        Ok(())
    }
    
    /// Send DTMF digits
    pub async fn send_dtmf(&self, call_id: &CallId, digits: &str) -> SipClientResult<()> {
        let calls = self.calls.read().await;
        let call_state = calls.get(call_id)
            .ok_or_else(|| SipClientError::CallNotFound { 
                call_id: call_id.to_string() 
            })?;
        
        // Validate DTMF digits
        for digit in digits.chars() {
            match digit {
                '0'..='9' | '*' | '#' | 'A'..='D' => {},
                _ => return Err(SipClientError::Configuration {
                    message: format!("Invalid DTMF digit: {}", digit),
                }),
            }
        }
        
        // Send DTMF via client-core
        // TODO: client-core doesn't expose send_dtmf API yet
        return Err(SipClientError::NotImplemented {
            feature: "dtmf_send".to_string(),
        });
        
        self.event_emitter.emit(SipClientEvent::DtmfSent { 
            call: call_state.call.clone(),
            digits: digits.to_string(),
        });
        
        Ok(())
    }
    
    /// Get call statistics
    pub async fn get_call_statistics(&self, call_id: &CallId) -> SipClientResult<CallStatistics> {
        let calls = self.calls.read().await;
        let call_state = calls.get(call_id)
            .ok_or_else(|| SipClientError::CallNotFound { 
                call_id: call_id.to_string() 
            })?;
        
        // Get media statistics from client-core
        // TODO: Get actual statistics from client-core
        // For now, return mock statistics
        
        // Convert to our format
        Ok(CallStatistics {
            audio_metrics: AudioQualityMetrics {
                level: 0.0, // Would come from audio pipeline
                peak_level: 0.0,
                mos: 4.0, // Mock value
                packet_loss_percent: 0.0,
                jitter_ms: 0.0,
                rtt_ms: 0.0,
            },
            packets_sent: 0,
            packets_received: 0,
            bytes_sent: 0,
            bytes_received: 0,
            codec: "PCMU".to_string(),
        })
    }
    
    /// Answer an incoming call
    pub async fn answer(&self, call_id: &CallId) -> SipClientResult<()> {
        let calls = self.calls.read().await;
        let call_state = calls.get(call_id)
            .ok_or_else(|| SipClientError::CallNotFound { 
                call_id: call_id.to_string() 
            })?;
        
        // Answer via client-core
        // TODO: Use actual client-core answer API
        // For now, just update state
        *call_state.call.state.write() = CallState::Connected;
        
        // Set up custom audio pipeline
        drop(calls);
        self.setup_custom_pipeline(call_id).await?;
        
        Ok(())
    }
    
    /// Hangup a call
    pub async fn hangup(&self, call_id: &CallId) -> SipClientResult<()> {
        let mut calls = self.calls.write().await;
        if let Some(mut call_state) = calls.remove(call_id) {
            // Cancel all pipeline tasks
            for task in call_state.pipeline_tasks.drain(..) {
                task.abort();
            }
            
            // Hangup via client-core
            // TODO: Use actual client-core hangup API
            // For now, just update state
            *call_state.call.state.write() = CallState::Terminated;
            
            self.event_emitter.emit(SipClientEvent::CallEnded { 
                call: call_state.call 
            });
        }
        
        Ok(())
    }
    
    /// Get event stream
    pub fn events(&self) -> EventStream {
        self.event_emitter.subscribe()
    }
    
    /// Set up custom audio pipeline for a call
    async fn setup_custom_pipeline(&self, call_id: &CallId) -> SipClientResult<()> {
        let config = self.pipeline_config.read().await.clone();
        let mut calls = self.calls.write().await;
        
        let call_state = calls.get_mut(call_id)
            .ok_or_else(|| SipClientError::CallNotFound { 
                call_id: call_id.to_string() 
            })?;
        
        // Build custom audio pipeline
        let mut builder = AudioPipelineBuilder::new();
        
        // Configure audio format for VoIP
        builder = builder.input_format(AudioFormat::pcm_8khz_mono());
        
        // Apply settings - Note: audio-core doesn't expose these on builder yet
        // TODO: Add echo cancellation, noise suppression, AGC when available
        
        // Set devices if specified
        if let Some(input_device) = &config.input_device {
            // TODO: Set input device when API is available
        }
        
        if let Some(output_device) = &config.output_device {
            // TODO: Set output device when API is available
        }
        
        // Build the pipeline
        let pipeline = builder.build().await
            .map_err(|e| SipClientError::AudioPipelineError {
                operation: "build_pipeline".to_string(),
                details: e.to_string(),
            })?;
        
        call_state.custom_pipeline = Some(Arc::new(pipeline));
        
        // If frame access is enabled, set up frame routing
        if config.enable_frame_access && call_state.frame_channels.is_some() {
            // This would connect the custom pipeline to the frame channels
            // allowing direct frame access
        }
        
        Ok(())
    }
    
    /// Reconfigure pipeline for a call
    async fn reconfigure_pipeline(&self, call_id: &CallId) -> SipClientResult<()> {
        // In a real implementation, this would update the existing pipeline
        // with new configuration without dropping the call
        Ok(())
    }
    
    /// Set up event forwarding from client-core
    async fn setup_event_forwarding(&self) -> SipClientResult<()> {
        // Create event handler
        let emitter = self.event_emitter.clone();
        let calls = self.calls.clone();
        
        struct EventHandler {
            emitter: EventEmitter,
            calls: Arc<RwLock<HashMap<CallId, AdvancedCallState>>>,
        }
        
        // In a real implementation, we would register this handler with client-core
        // For now, this is a placeholder
        
        Ok(())
    }
}

impl AudioPipelineConfig {
    /// Create a custom pipeline configuration
    pub fn custom() -> Self {
        Self {
            input_device: None,
            output_device: None,
            echo_cancellation: true,
            noise_suppression: true,
            auto_gain_control: true,
            custom_processors: Vec::new(),
            buffer_size: 160, // Default 20ms at 8kHz
            enable_frame_access: false,
        }
    }
    
    /// Set input device
    pub fn input_device(mut self, device: impl Into<String>) -> Self {
        self.input_device = Some(device.into());
        self
    }
    
    /// Set output device
    pub fn output_device(mut self, device: impl Into<String>) -> Self {
        self.output_device = Some(device.into());
        self
    }
    
    /// Enable/disable echo cancellation
    pub fn echo_cancellation(mut self, enabled: bool) -> Self {
        self.echo_cancellation = enabled;
        self
    }
    
    /// Enable/disable noise suppression
    pub fn noise_suppression(mut self, enabled: bool) -> Self {
        self.noise_suppression = enabled;
        self
    }
    
    /// Enable/disable automatic gain control
    pub fn auto_gain_control(mut self, enabled: bool) -> Self {
        self.auto_gain_control = enabled;
        self
    }
    
    /// Add a custom audio processor
    pub fn add_processor(mut self, processor: Box<dyn AudioProcessorTrait>) -> Self {
        self.custom_processors.push(AudioProcessor { processor: Arc::from(processor) });
        self
    }
    
    /// Enable frame-level access
    pub fn enable_frame_access(mut self, enabled: bool) -> Self {
        self.enable_frame_access = enabled;
        self
    }
    
    /// Set buffer size
    pub fn buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = size;
        self
    }
}

impl CodecPriority {
    /// Create a new codec priority entry
    pub fn new(name: impl Into<String>, priority: u8) -> Self {
        Self {
            name: name.into(),
            priority,
        }
    }
}

impl Default for MediaPreferences {
    fn default() -> Self {
        Self {
            codecs: vec![
                CodecPriority::new("PCMU", 100),
                CodecPriority::new("PCMA", 90),
            ],
            sdp_attributes: HashMap::new(),
            jitter_buffer_ms: 100,
            dtmf_detection: false,
            comfort_noise: false,
        }
    }
}

impl MediaPreferences {
    /// Create new media preferences
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Set codec priorities
    pub fn codecs(mut self, codecs: Vec<CodecPriority>) -> Self {
        self.codecs = codecs;
        self
    }
    
    /// Add a custom SDP attribute
    pub fn add_sdp_attribute(mut self, key: String, value: String) -> Self {
        self.sdp_attributes.insert(key, value);
        self
    }
    
    /// Set jitter buffer size
    pub fn jitter_buffer_ms(mut self, ms: u32) -> Self {
        self.jitter_buffer_ms = ms;
        self
    }
    
    /// Enable DTMF detection
    pub fn dtmf_detection(mut self, enabled: bool) -> Self {
        self.dtmf_detection = enabled;
        self
    }
    
    /// Enable comfort noise
    pub fn comfort_noise(mut self, enabled: bool) -> Self {
        self.comfort_noise = enabled;
        self
    }
}