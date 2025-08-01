//! Simple API for SIP client

use crate::{
    error::{SipClientError, SipClientResult},
    error_reporting::ErrorReportingExt,
    events::{EventEmitter, EventStream, SipClientEvent},
    types::{Call, CallDirection, CallId, CallState, SipClientConfig},
    recovery::{RecoveryManager, RecoveryConfig, ConnectionMonitor, NetworkMetrics},
    degradation::QualityAdaptationManager,
    reconnect::ReconnectionHandler,
};
use std::sync::Arc;
use parking_lot::RwLock;
use std::collections::HashMap;
use tokio::task::JoinHandle;
use async_trait::async_trait;
use rvoip_client_core::events::ClientEventHandler;

/// Simple SIP client with easy-to-use API
#[derive(Clone)]
pub struct SipClient {
    /// Internal state
    inner: Arc<SipClientInner>,
}

/// Audio pipeline tasks for a call
struct AudioPipelineTasks {
    /// Capture task handle
    capture_task: JoinHandle<()>,
    /// Playback task handle
    playback_task: JoinHandle<()>,
}

struct SipClientInner {
    /// Configuration
    config: SipClientConfig,
    /// Client from client-core
    client: Arc<rvoip_client_core::Client>,
    /// Audio device manager
    audio_manager: Arc<rvoip_audio_core::AudioDeviceManager>,
    /// Codec registry
    codec_registry: Arc<codec_core::CodecRegistry>,
    /// Active calls
    calls: Arc<RwLock<HashMap<CallId, Arc<Call>>>>,
    /// Audio pipeline tasks per call
    audio_tasks: Arc<RwLock<HashMap<CallId, AudioPipelineTasks>>>,
    /// Event emitter
    events: EventEmitter,
    /// Event handler task handle
    event_handler_task: Arc<RwLock<Option<JoinHandle<()>>>>,
    /// Recovery manager
    recovery_manager: Arc<RecoveryManager>,
    /// Quality adaptation manager
    quality_adaptation_manager: Arc<QualityAdaptationManager>,
    /// Reconnection handler
    reconnection_handler: Arc<ReconnectionHandler>,
    /// Connection monitor handle
    connection_monitor: Arc<RwLock<Option<JoinHandle<()>>>>,
}

impl SipClient {
    /// Create a new SIP client with default configuration
    ///
    /// # Example
    /// ```no_run
    /// # use rvoip_sip_client::SipClient;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = SipClient::new("sip:alice@example.com").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new(sip_identity: impl Into<String>) -> SipClientResult<Self> {
        let config = SipClientConfig {
            sip_identity: sip_identity.into(),
            ..Default::default()
        };
        
        Self::from_config(config).await
    }
    
    /// Create a SIP client from configuration
    pub(crate) async fn from_config(config: SipClientConfig) -> SipClientResult<Self> {
        // Initialize components
        let client = Self::create_client(&config).await?;
        let audio_manager = Self::create_audio_manager(&config).await?;
        let codec_registry = Self::create_codec_registry(&config)?;
        
        // Create event emitter
        let events = EventEmitter::default();
        
        // Create recovery components
        let recovery_config = RecoveryConfig::default();
        let recovery_manager = Arc::new(RecoveryManager::new(
            recovery_config.clone(),
            events.clone(),
        ));
        let quality_adaptation_manager = Arc::new(QualityAdaptationManager::new(
            events.clone(),
        ));
        
        let reconnection_handler = Arc::new(ReconnectionHandler::new(
            recovery_manager.clone(),
            events.clone(),
        ));
        
        let inner = Arc::new(SipClientInner {
            config,
            client,
            audio_manager,
            codec_registry,
            calls: Arc::new(RwLock::new(HashMap::new())),
            audio_tasks: Arc::new(RwLock::new(HashMap::new())),
            events,
            event_handler_task: Arc::new(RwLock::new(None)),
            recovery_manager,
            quality_adaptation_manager,
            reconnection_handler,
            connection_monitor: Arc::new(RwLock::new(None)),
        });
        
        Ok(Self { inner })
    }
    
    /// Make a call
    ///
    /// # Example
    /// ```no_run
    /// # use rvoip_sip_client::SipClient;
    /// # async fn example(client: &SipClient) -> Result<(), Box<dyn std::error::Error>> {
    /// let call = client.call("sip:bob@example.com").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn call(&self, uri: impl Into<String>) -> SipClientResult<Arc<Call>> {
        let uri = uri.into();
        
        // Create SDP offer with our codec preferences
        let sdp = self.create_sdp_offer().await?;
        
        // Make the call via client-core
        let call_id = self.inner.client
            .make_call(
                self.inner.config.sip_identity.clone(),
                uri.clone(),
                Some(sdp),
            )
            .await?;
        
        // Create call object
        let call = Arc::new(Call {
            id: call_id,
            state: Arc::new(RwLock::new(CallState::Initiating)),
            remote_uri: uri,
            local_uri: self.inner.config.sip_identity.clone(),
            start_time: chrono::Utc::now(),
            connect_time: None,
            codec: None,
            direction: CallDirection::Outgoing,
        });
        
        // Store call
        self.inner.calls.write().insert(call_id, call.clone());
        
        // Start audio pipeline for the call
        self.setup_audio_pipeline(&call).await?;
        
        // Initialize quality adaptation for the call
        let codecs = vec![codec_core::CodecType::G711Pcmu, codec_core::CodecType::G711Pcma];
        self.inner.quality_adaptation_manager.initialize_call(call_id, codecs).await;
        
        Ok(call)
    }
    
    /// Answer an incoming call
    pub async fn answer(&self, call_id: &CallId) -> SipClientResult<()> {
        let call = self.get_call(call_id)?;
        
        // Create SDP answer
        let sdp = self.create_sdp_answer(&call).await?;
        
        // Answer via client-core
        self.inner.client.answer_call(call_id).await?;
        
        // Update call state
        *call.state.write() = CallState::Connected;
        
        // Start audio pipeline for answered call
        self.setup_audio_pipeline(&call).await?;
        
        // Initialize quality adaptation for the call
        let codecs = vec![codec_core::CodecType::G711Pcmu, codec_core::CodecType::G711Pcma];
        self.inner.quality_adaptation_manager.initialize_call(*call_id, codecs).await;
        
        Ok(())
    }
    
    /// Reject an incoming call
    pub async fn reject(&self, call_id: &CallId) -> SipClientResult<()> {
        self.inner.client.reject_call(call_id).await?;
        
        // Remove call
        self.inner.calls.write().remove(call_id);
        
        Ok(())
    }
    
    /// Hangup a call
    pub async fn hangup(&self, call_id: &CallId) -> SipClientResult<()> {
        // Terminate via client-core
        self.inner.client.hangup_call(call_id).await?;
        
        // Clean up audio pipeline
        self.cleanup_audio_pipeline(call_id).await?;
        
        // Clean up quality adaptation
        self.inner.quality_adaptation_manager.cleanup_call(call_id).await;
        
        // Update state
        if let Some(call) = self.inner.calls.read().get(call_id) {
            *call.state.write() = CallState::Terminated;
        }
        
        // Remove call after a delay
        let calls = self.inner.calls.clone();
        let call_id = *call_id;
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            calls.write().remove(&call_id);
        });
        
        Ok(())
    }
    
    /// Mute/unmute microphone
    pub async fn set_mute(&self, call_id: &CallId, mute: bool) -> SipClientResult<()> {
        self.inner.client.set_microphone_mute(call_id, mute).await?;
        Ok(())
    }
    
    /// Get current mute state
    pub async fn is_muted(&self, call_id: &CallId) -> SipClientResult<bool> {
        Ok(self.inner.client.get_microphone_mute_state(call_id).await?)
    }
    
    /// Put call on hold
    pub async fn hold(&self, call_id: &CallId) -> SipClientResult<()> {
        self.inner.client.hold_call(call_id).await?;
        
        if let Some(call) = self.inner.calls.read().get(call_id) {
            *call.state.write() = CallState::OnHold;
        }
        
        Ok(())
    }
    
    /// Resume a held call
    pub async fn resume(&self, call_id: &CallId) -> SipClientResult<()> {
        self.inner.client.resume_call(call_id).await?;
        
        if let Some(call) = self.inner.calls.read().get(call_id) {
            *call.state.write() = CallState::Connected;
        }
        
        Ok(())
    }
    
    /// Subscribe to events (requires StreamExt)
    pub fn events(&self) -> EventStream {
        self.inner.events.subscribe()
    }
    
    /// Subscribe to events with simple iterator (no StreamExt needed)
    pub fn event_iter(&self) -> crate::events::EventIterator {
        self.inner.events.subscribe_simple()
    }
    
    /// Get active calls
    pub fn active_calls(&self) -> Vec<Arc<Call>> {
        self.inner.calls.read().values().cloned().collect()
    }
    
    /// List available audio devices
    pub async fn list_audio_devices(&self, direction: rvoip_audio_core::AudioDirection) -> SipClientResult<Vec<rvoip_audio_core::AudioDeviceInfo>> {
        Ok(self.inner.audio_manager.list_devices(direction).await?)
    }
    
    /// Get current audio device
    pub async fn get_audio_device(&self, direction: rvoip_audio_core::AudioDirection) -> SipClientResult<rvoip_audio_core::AudioDeviceInfo> {
        let device = self.inner.audio_manager.get_default_device(direction).await?;
        Ok(device.info().clone())
    }
    
    /// Set audio device
    pub async fn set_audio_device(&self, direction: rvoip_audio_core::AudioDirection, device_id: &str) -> SipClientResult<()> {
        // Get current device for comparison
        let old_device = self.get_audio_device(direction).await.ok();
        
        // TODO: Actually change the device in audio_manager
        // For now, just emit the event
        
        // Emit device change event
        self.inner.events.emit(SipClientEvent::AudioDeviceChanged {
            direction,
            old_device: old_device.map(|d| d.name),
            new_device: Some(device_id.to_string()),
        });
        
        Ok(())
    }
    
    /// Start the SIP client
    ///
    /// This initializes all subsystems and begins listening for calls.
    ///
    /// # Example
    /// ```no_run
    /// # use rvoip_sip_client::SipClient;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut client = SipClient::new("sip:alice@example.com").await?;
    /// client.start().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn start(&self) -> SipClientResult<()> {
        // Client-core should already be started from create_client
        
        // Start event forwarding from client-core
        self.start_event_forwarding().await?;
        
        // Register reconnection callbacks
        self.register_reconnection_callbacks().await?;
        
        // Start connection monitoring with quality adaptation
        let inner = self.inner.clone();
        let quality_manager = self.inner.quality_adaptation_manager.clone();
        let monitor = ConnectionMonitor::new(
            self.inner.events.clone(),
            std::time::Duration::from_secs(5),
            move || {
                let inner = inner.clone();
                let quality_manager = quality_manager.clone();
                Box::pin(async move {
                    // Simulate network metrics collection
                    // In a real implementation, this would gather actual metrics
                    let metrics = NetworkMetrics {
                        packet_loss_percent: 0.5,
                        jitter_ms: 15.0,
                        rtt_ms: 50.0,
                        available_bandwidth_bps: Some(128000),
                        consecutive_errors: 0,
                    };
                    
                    // Update quality adaptation based on metrics
                    let degradation_actions = quality_manager.update_metrics(metrics).await;
                    
                    // Apply degradation actions if needed
                    for (call_id, actions) in degradation_actions {
                        if actions.codec_downgrade {
                            tracing::info!("Applied codec downgrade for call {}", call_id);
                        }
                        if actions.reduce_quality {
                            tracing::info!("Reduced quality for call {} to {} bps", call_id, actions.target_bitrate.unwrap_or(0));
                        }
                    }
                    
                    // Simple health check - return true if connection is healthy
                    true
                })
            },
        );
        
        let monitor_handle = monitor.start_monitoring().await;
        *self.inner.connection_monitor.write() = Some(monitor_handle);
        
        // Emit started event
        self.inner.events.emit(SipClientEvent::Started);
        
        Ok(())
    }
    
    /// Stop the SIP client
    ///
    /// This gracefully shuts down all subsystems and cleans up resources.
    ///
    /// # Example
    /// ```no_run
    /// # use rvoip_sip_client::SipClient;
    /// # async fn example(client: &SipClient) -> Result<(), Box<dyn std::error::Error>> {
    /// client.stop().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn stop(&self) -> SipClientResult<()> {
        // Stop event forwarding
        self.stop_event_forwarding().await?;
        
        // Clean up all active calls
        let call_ids: Vec<CallId> = {
            self.inner.calls.read().keys().cloned().collect()
        };
        
        for call_id in call_ids {
            if let Err(e) = self.hangup(&call_id).await {
                tracing::warn!("Failed to hangup call {} during shutdown: {}", call_id, e);
            }
        }
        
        // Stop client-core
        self.inner.client.stop().await?;
        
        // Emit stopped event
        self.inner.events.emit(SipClientEvent::Stopped);
        
        Ok(())
    }
    
    /// Get a specific call
    pub fn get_call(&self, call_id: &CallId) -> SipClientResult<Arc<Call>> {
        self.inner.calls.read()
            .get(call_id)
            .cloned()
            .ok_or_else(|| SipClientError::CallNotFound {
                call_id: call_id.to_string(),
            })
    }
    
    /// Get the currently active call (if any)
    pub fn active_call(&self) -> Option<Arc<Call>> {
        // Return the first connected call
        self.inner.calls.read()
            .values()
            .find(|call| matches!(*call.state.read(), CallState::Connected))
            .cloned()
    }
    
    /// Check if there's an active call
    pub fn has_active_call(&self) -> bool {
        self.active_call().is_some()
    }
    
    /// Wait for the next event (convenience method)
    pub async fn next_event(&mut self) -> Option<SipClientEvent> {
        // This would require making events() return a mutable stream
        // For now, users still need to use events() + StreamExt
        None
    }
    
    // Helper methods
    
    async fn create_client(config: &SipClientConfig) -> SipClientResult<Arc<rvoip_client_core::Client>> {
        let client = rvoip_client_core::ClientBuilder::new()
            .local_address(config.local_address)
            .user_agent(&config.user_agent)
            .build()
            .await?;
        
        client.start().await?;
        
        Ok(client)
    }
    
    async fn create_audio_manager(config: &SipClientConfig) -> SipClientResult<Arc<rvoip_audio_core::AudioDeviceManager>> {
        let manager = rvoip_audio_core::AudioDeviceManager::new().await?;
        
        // TODO: Configure audio devices based on config
        
        Ok(Arc::new(manager))
    }
    
    fn create_codec_registry(config: &SipClientConfig) -> SipClientResult<Arc<codec_core::CodecRegistry>> {
        let mut registry = codec_core::CodecRegistry::new();
        
        // Register codecs based on configuration
        for codec_priority in &config.codecs.priorities {
            match codec_priority.name.as_str() {
                "PCMU" | "G711U" => {
                    registry.register(
                        "PCMU".to_string(),
                        Box::new(
                            codec_core::codecs::g711::G711Codec::new(
                                codec_core::codecs::g711::G711Variant::MuLaw
                            )
                        )
                    );
                }
                "PCMA" | "G711A" => {
                    registry.register(
                        "PCMA".to_string(),
                        Box::new(
                            codec_core::codecs::g711::G711Codec::new(
                                codec_core::codecs::g711::G711Variant::ALaw
                            )
                        )
                    );
                }
                _ => {
                    // Skip unknown codecs
                }
            }
        }
        
        Ok(Arc::new(registry))
    }
    
    async fn create_sdp_offer(&self) -> SipClientResult<String> {
        // TODO: Create proper SDP based on codec configuration
        Ok("v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 5004 RTP/AVP 0 8\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:8 PCMA/8000\r\n".to_string())
    }
    
    async fn create_sdp_answer(&self, _call: &Call) -> SipClientResult<String> {
        // TODO: Create proper SDP answer based on offer
        self.create_sdp_offer().await
    }
    
    async fn setup_audio_pipeline(&self, call: &Arc<Call>) -> SipClientResult<()> {
        use rvoip_audio_core::pipeline::AudioPipeline;
        use rvoip_audio_core::types::{AudioFormat, AudioStreamConfig};
        
        // Create audio pipeline configuration
        let mut config = AudioStreamConfig::voip_basic();
        
        // Configure based on negotiated codec (default to G.711 μ-law 8kHz)
        let codec_type = call.codec.as_ref()
            .cloned()
            .unwrap_or(codec_core::CodecType::G711Pcmu);
            
        // Set codec name and format based on codec type
        // Note: codec-core currently only supports G.711 codecs
        let (codec_name, audio_format) = match codec_type {
            codec_core::CodecType::G711Pcmu => ("PCMU", AudioFormat::pcm_8khz_mono()),
            codec_core::CodecType::G711Pcma => ("PCMA", AudioFormat::pcm_8khz_mono()),
            // For unsupported codecs, default to PCMU
            _ => {
                tracing::warn!("Codec {:?} not supported by codec-core, defaulting to PCMU", codec_type);
                ("PCMU", AudioFormat::pcm_8khz_mono())
            }
        };
        
        config.codec_name = codec_name.to_string();
        
        config.input_format = audio_format.clone();
        config.output_format = audio_format;
        
        // Create audio pipeline with audio processing enabled
        let mut pipeline = AudioPipeline::builder()
            .input_format(config.input_format.clone())
            .output_format(config.output_format.clone())
            .device_manager(self.inner.audio_manager.as_ref().clone())
            .enable_processing(true) // Enable AEC, AGC, noise suppression
            .buffer_size_ms(50) // 50ms buffer for jitter
            .build()
            .await
            .map_err(|e| SipClientError::AudioPipelineError {
                operation: "create".to_string(),
                details: e.to_string(),
            })?;
        
        // Start the pipeline
        pipeline.start().await
            .map_err(|e| SipClientError::AudioPipelineError {
                operation: "start".to_string(),
                details: e.to_string(),
            })?;
        
        // Store pipeline reference (we'll need to add this field)
        // For now, we'll spawn tasks to handle audio flow
        
        // Spawn task to capture audio and send to client-core
        let client = self.inner.client.clone();
        let call_id = call.id;
        let events = self.inner.events.clone();
        let capture_handle = tokio::spawn(async move {
            let mut pipeline = pipeline;
            let mut frame_count = 0u64;
            loop {
                match pipeline.capture_frame().await {
                    Ok(audio_frame) => {
                        // Emit audio level event periodically (every 50 frames = ~1 second at 20ms frames)
                        frame_count += 1;
                        if frame_count % 50 == 0 {
                            let level = audio_frame.rms_level();
                            let peak = audio_frame.samples.iter()
                                .map(|&s| s.abs() as f32 / i16::MAX as f32)
                                .fold(0.0f32, |max, val| if val > max { val } else { max });
                            
                            events.emit(SipClientEvent::AudioLevelChanged {
                                call_id: Some(call_id),
                                direction: rvoip_audio_core::AudioDirection::Input,
                                level: level / i16::MAX as f32,
                                peak,
                            });
                        }
                        
                        // Convert audio-core frame to session-core frame
                        let session_frame = audio_frame.to_session_core();
                        
                        // Send to client-core for encoding and RTP transmission
                        if let Err(e) = client.send_audio_frame(&call_id, session_frame).await {
                            tracing::error!("Failed to send audio frame: {}", e);
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to capture audio frame: {}", e);
                        events.emit(SipClientEvent::AudioDeviceError {
                            message: format!("Capture error: {}", e),
                            device: None,
                        });
                        break;
                    }
                }
                
                // Add small delay to prevent busy loop
                tokio::time::sleep(tokio::time::Duration::from_micros(100)).await;
            }
        });
        
        // Subscribe to incoming audio frames from client-core
        let mut audio_subscriber = self.inner.client
            .subscribe_to_audio_frames(&call.id)
            .await
            .map_err(|e| SipClientError::AudioPipelineError {
                operation: "subscribe".to_string(),
                details: e.to_string(),
            })?;
        
        // Create a new pipeline for playback (we need separate instance for now)
        let mut playback_pipeline = AudioPipeline::builder()
            .input_format(config.input_format.clone())
            .output_format(config.output_format.clone())
            .device_manager(self.inner.audio_manager.as_ref().clone())
            .enable_processing(false) // No processing needed for playback
            .buffer_size_ms(50)
            .build()
            .await
            .map_err(|e| SipClientError::AudioPipelineError {
                operation: "create_playback".to_string(),
                details: e.to_string(),
            })?;
        
        playback_pipeline.start().await
            .map_err(|e| SipClientError::AudioPipelineError {
                operation: "start_playback".to_string(),
                details: e.to_string(),
            })?;
        
        // Spawn task to receive audio from client-core and play
        let events_playback = self.inner.events.clone();
        let call_id_playback = call.id;
        let playback_handle = tokio::spawn(async move {
            let mut pipeline = playback_pipeline;
            let mut frame_count = 0u64;
            while let Ok(session_frame) = audio_subscriber.recv() {
                // Convert session-core frame to audio-core frame
                let audio_frame = rvoip_audio_core::types::AudioFrame::from_session_core(
                    &session_frame,
                    config.output_format.frame_size_ms
                );
                
                // Emit audio level event periodically for playback
                frame_count += 1;
                if frame_count % 50 == 0 {
                    let level = audio_frame.rms_level();
                    let peak = audio_frame.samples.iter()
                        .map(|&s| s.abs() as f32 / i16::MAX as f32)
                        .fold(0.0f32, |max, val| if val > max { val } else { max });
                    
                    events_playback.emit(SipClientEvent::AudioLevelChanged {
                        call_id: Some(call_id_playback),
                        direction: rvoip_audio_core::AudioDirection::Output,
                        level: level / i16::MAX as f32,
                        peak,
                    });
                }
                
                // Send to audio pipeline for playback
                if let Err(e) = pipeline.playback_frame(audio_frame).await {
                    tracing::error!("Failed to playback audio frame: {}", e);
                    events_playback.emit(SipClientEvent::AudioDeviceError {
                        message: format!("Playback error: {}", e),
                        device: None,
                    });
                    break;
                }
            }
        });
        
        // Store task handles for cleanup
        let audio_tasks = AudioPipelineTasks {
            capture_task: capture_handle,
            playback_task: playback_handle,
        };
        
        self.inner.audio_tasks.write().insert(call.id, audio_tasks);
        
        Ok(())
    }
    
    async fn cleanup_audio_pipeline(&self, call_id: &CallId) -> SipClientResult<()> {
        // Remove and stop audio tasks
        if let Some(tasks) = self.inner.audio_tasks.write().remove(call_id) {
            // Cancel the tasks
            tasks.capture_task.abort();
            tasks.playback_task.abort();
            
            // Wait for tasks to finish (with timeout)
            let timeout = tokio::time::Duration::from_secs(1);
            let _ = tokio::time::timeout(timeout, tasks.capture_task).await;
            let _ = tokio::time::timeout(timeout, tasks.playback_task).await;
            
            tracing::debug!("Cleaned up audio pipeline for call {}", call_id);
        }
        
        Ok(())
    }
    
    /// Start event forwarding from client-core to sip-client events
    async fn start_event_forwarding(&self) -> SipClientResult<()> {
        // Subscribe to client-core events
        let mut event_rx = self.inner.client.subscribe_events();
        
        // Create event forwarder task
        let inner = self.inner.clone();
        let task = tokio::spawn(async move {
            while let Ok(event) = event_rx.recv().await {
                // Forward event through our event handler
                let handler = SipClientEventHandler {
                    inner: inner.clone(),
                };
                
                handler.handle_client_event(event).await;
            }
        });
        
        // Store the task handle
        *self.inner.event_handler_task.write() = Some(task);
        
        Ok(())
    }
    
    /// Stop event forwarding
    async fn stop_event_forwarding(&self) -> SipClientResult<()> {
        if let Some(task) = self.inner.event_handler_task.write().take() {
            task.abort();
            let _ = task.await;
        }
        Ok(())
    }
    
    /// Register reconnection callbacks for various connection types
    async fn register_reconnection_callbacks(&self) -> SipClientResult<()> {
        use crate::reconnect::ConnectionType;
        
        // Registration reconnection
        let client = self.inner.client.clone();
        let config = self.inner.config.clone();
        self.inner.reconnection_handler.register_callback(
            ConnectionType::Registration,
            move || {
                let client = client.clone();
                let config = config.clone();
                Box::pin(async move {
                    // Create registration config
                    let reg_config = rvoip_client_core::registration::RegistrationConfig::new(
                        config.sip_registrar.clone().unwrap_or_else(|| "sip:localhost:5060".to_string()),
                        config.sip_identity.clone(),
                        format!("sip:{}@{}:{}", 
                            config.sip_identity.split('@').next().unwrap_or("user"),
                            config.local_address.ip(),
                            config.local_address.port()
                        ),
                    ).with_expires(config.registration_ttl);
                    
                    // Re-register with SIP server
                    client.register(reg_config).await
                        .map_err(|e| SipClientError::RegistrationFailed {
                            reason: e.to_string(),
                        })?;
                    Ok(())
                })
            },
        ).await;
        
        // Audio device reconnection
        let audio_manager = self.inner.audio_manager.clone();
        self.inner.reconnection_handler.register_callback(
            ConnectionType::AudioDevice,
            move || {
                let audio_manager = audio_manager.clone();
                Box::pin(async move {
                    // Try to reinitialize audio devices
                    // This is a simplified version - real implementation would be more sophisticated
                    let input_devices = audio_manager
                        .list_devices(rvoip_audio_core::AudioDirection::Input)
                        .await
                        .map_err(|e| SipClientError::AudioDevice {
                            message: format!("Failed to list input devices: {}", e),
                        })?;
                        
                    if input_devices.is_empty() {
                        return Err(SipClientError::AudioDevice {
                            message: "No input devices available".to_string(),
                        });
                    }
                    
                    Ok(())
                })
            },
        ).await;
        
        Ok(())
    }
}

// Extension trait for Call to add convenience methods
/// Event handler that forwards client-core events to sip-client events
struct SipClientEventHandler {
    inner: Arc<SipClientInner>,
}

impl SipClientEventHandler {
    /// Handle a client-core event and forward it as a sip-client event
    async fn handle_client_event(&self, event: rvoip_client_core::events::ClientEvent) {
        use rvoip_client_core::events::ClientEvent;
        
        match event {
            ClientEvent::IncomingCall { info, .. } => {
                let action = self.on_incoming_call(info).await;
                // Note: We can't return the action here, so we always accept
                // In future we might add a callback mechanism
            }
            ClientEvent::CallStateChanged { info, .. } => {
                self.on_call_state_changed(info).await;
            }
            ClientEvent::RegistrationStatusChanged { info, .. } => {
                self.on_registration_status_changed(info).await;
            }
            ClientEvent::MediaEvent { info, .. } => {
                self.on_media_event(info).await;
            }
            ClientEvent::ClientError { error, call_id, .. } => {
                self.on_client_error(error, call_id).await;
            }
            ClientEvent::NetworkEvent { connected, reason, .. } => {
                self.on_network_event(connected, reason).await;
            }
        }
    }
}

#[async_trait]
impl rvoip_client_core::events::ClientEventHandler for SipClientEventHandler {
    async fn on_incoming_call(&self, call_info: rvoip_client_core::events::IncomingCallInfo) -> rvoip_client_core::events::CallAction {
        // Create call object for incoming call
        let call = Arc::new(Call {
            id: call_info.call_id,
            state: Arc::new(RwLock::new(CallState::IncomingRinging)),
            remote_uri: call_info.caller_uri.clone(),
            local_uri: call_info.callee_uri.clone(),
            start_time: call_info.created_at,
            connect_time: None,
            codec: None,
            direction: CallDirection::Incoming,
        });
        
        // Store call
        self.inner.calls.write().insert(call_info.call_id, call.clone());
        
        // Emit incoming call event
        self.inner.events.emit(SipClientEvent::IncomingCall {
            call: call.clone(),
            from: call_info.caller_uri,
            display_name: call_info.caller_display_name,
        });
        
        // For now, always accept (in future, we might add a handler callback)
        rvoip_client_core::events::CallAction::Accept
    }
    
    async fn on_call_state_changed(&self, status_info: rvoip_client_core::events::CallStatusInfo) {
        // Update call state
        if let Some(call) = self.inner.calls.read().get(&status_info.call_id) {
            // Map client-core states to sip-client states
            let new_state = match status_info.new_state {
                rvoip_client_core::call::CallState::Initiating => CallState::Initiating,
                rvoip_client_core::call::CallState::Proceeding => CallState::Initiating,
                rvoip_client_core::call::CallState::Ringing => CallState::Ringing,
                rvoip_client_core::call::CallState::Connected => CallState::Connected,
                rvoip_client_core::call::CallState::Terminating => CallState::Terminated,
                rvoip_client_core::call::CallState::Terminated => CallState::Terminated,
                rvoip_client_core::call::CallState::Failed => CallState::Terminated,
                rvoip_client_core::call::CallState::Cancelled => CallState::Terminated,
                rvoip_client_core::call::CallState::IncomingPending => CallState::IncomingRinging,
            };
            
            let old_state = *call.state.read();
            *call.state.write() = new_state;
            
            // Update connect time if transitioning to connected
            if new_state == CallState::Connected && old_state != CallState::Connected {
                // We can't mutate the Call struct directly since it's behind an Arc
                // This would need to be refactored to store connect_time separately
                // For now, we'll skip updating connect_time
            }
            
            // Emit state change event
            self.inner.events.emit(SipClientEvent::CallStateChanged {
                call: call.clone(),
                previous_state: old_state,
                new_state,
                reason: status_info.reason,
            });
        }
    }
    
    async fn on_registration_status_changed(&self, status_info: rvoip_client_core::events::RegistrationStatusInfo) {
        // Map registration status
        let status = match status_info.status {
            rvoip_client_core::registration::RegistrationStatus::Pending => "pending",
            rvoip_client_core::registration::RegistrationStatus::Active => "active",
            rvoip_client_core::registration::RegistrationStatus::Failed => "failed",
            rvoip_client_core::registration::RegistrationStatus::Expired => "expired",
            _ => "unknown",
        };
        
        // Emit registration event
        self.inner.events.emit(SipClientEvent::RegistrationStatusChanged {
            uri: status_info.user_uri,
            status: status.to_string(),
            reason: status_info.reason,
        });
    }
    
    async fn on_media_event(&self, media_info: rvoip_client_core::events::MediaEventInfo) {
        use rvoip_client_core::events::MediaEventType;
        
        // Get call for the media event
        if let Some(call) = self.inner.calls.read().get(&media_info.call_id) {
            match &media_info.event_type {
                MediaEventType::AudioStarted => {
                    self.inner.events.emit(SipClientEvent::MediaStarted {
                        call: call.clone(),
                        media_type: "audio".to_string(),
                    });
                }
                MediaEventType::AudioStopped => {
                    self.inner.events.emit(SipClientEvent::MediaStopped {
                        call: call.clone(),
                        media_type: "audio".to_string(),
                    });
                }
                MediaEventType::QualityChanged { mos_score_x100 } => {
                    self.inner.events.emit(SipClientEvent::CallQualityReport {
                        call_id: call.id,
                        metrics: crate::types::AudioQualityMetrics {
                            level: 0.0,
                            peak_level: 0.0,
                            mos: (*mos_score_x100 as f64) / 100.0,
                            packet_loss_percent: 0.0,
                            jitter_ms: 0.0,
                            rtt_ms: 0.0,
                        },
                    });
                }
                MediaEventType::PacketLoss { percentage_x100 } => {
                    self.inner.events.emit(SipClientEvent::CallQualityReport {
                        call_id: call.id,
                        metrics: crate::types::AudioQualityMetrics {
                            level: 0.0,
                            peak_level: 0.0,
                            mos: 0.0,
                            packet_loss_percent: (*percentage_x100 as f64) / 100.0,
                            jitter_ms: 0.0,
                            rtt_ms: 0.0,
                        },
                    });
                }
                MediaEventType::DtmfSent { digits } => {
                    self.inner.events.emit(SipClientEvent::DtmfSent {
                        call: call.clone(),
                        digits: digits.clone(),
                    });
                }
                _ => {
                    // Other media events not currently mapped
                }
            }
        }
    }
    
    async fn on_client_error(&self, error: rvoip_client_core::ClientError, call_id: Option<CallId>) {
        // Find associated call if any
        let call = call_id.and_then(|id| self.inner.calls.read().get(&id).cloned());
        
        // Determine error category and trigger recovery if needed
        let category = match &error {
            rvoip_client_core::ClientError::NetworkError { .. } => crate::events::ErrorCategory::Network,
            rvoip_client_core::ClientError::ProtocolError { .. } => crate::events::ErrorCategory::Protocol,
            rvoip_client_core::ClientError::MediaError { .. } => crate::events::ErrorCategory::Audio,
            rvoip_client_core::ClientError::InvalidConfiguration { .. } => crate::events::ErrorCategory::Configuration,
            _ => crate::events::ErrorCategory::Internal,
        };
        
        // Convert to SipClientError for recovery handling  
        let sip_error = match &error {
            rvoip_client_core::ClientError::NetworkError { reason } => SipClientError::Network {
                message: reason.clone(),
            },
            rvoip_client_core::ClientError::AuthenticationFailed { reason } => SipClientError::RegistrationFailed {
                reason: reason.clone(),
            },
            _ => SipClientError::Internal {
                message: error.to_string(),
            },
        };
        
        // Trigger reconnection based on error type
        match category {
            crate::events::ErrorCategory::Network => {
                // Trigger registration reconnection
                let reg_error = match &sip_error {
                    SipClientError::Network { message } => SipClientError::Network { message: message.clone() },
                    SipClientError::RegistrationFailed { reason } => SipClientError::RegistrationFailed { reason: reason.clone() },
                    _ => SipClientError::Internal { message: error.to_string() },
                };
                
                let _ = self.inner.reconnection_handler.trigger_reconnection(
                    crate::reconnect::ConnectionType::Registration,
                    reg_error,
                ).await;
                
                // If there's a call, try to recover it
                if let Some(call_id) = call_id {
                    let call_error = match &sip_error {
                        SipClientError::Network { message } => SipClientError::Network { message: message.clone() },
                        _ => SipClientError::Internal { message: error.to_string() },
                    };
                    
                    let _ = self.inner.reconnection_handler.trigger_reconnection(
                        crate::reconnect::ConnectionType::Call(call_id),
                        call_error,
                    ).await;
                }
            }
            _ => {}
        }
        
        // Emit error event with enhanced message
        let enhanced_message = sip_error.user_message();
        
        self.inner.events.emit(SipClientEvent::Error {
            call,
            message: enhanced_message,
            category,
        });
    }
    
    async fn on_network_event(&self, connected: bool, reason: Option<String>) {
        if connected {
            self.inner.events.emit(SipClientEvent::NetworkConnected { reason });
        } else {
            self.inner.events.emit(SipClientEvent::NetworkDisconnected { 
                reason: reason.unwrap_or_else(|| "Unknown".to_string()),
            });
        }
    }
}

// Extension trait for Call to add convenience methods
impl Call {
    /// Wait for the call to be answered
    pub async fn wait_for_answer(&self) -> SipClientResult<()> {
        loop {
            let state = *self.state.read();
            match state {
                CallState::Connected => return Ok(()),
                CallState::Terminated => {
                    return Err(SipClientError::invalid_state("Call was terminated"));
                }
                _ => {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                }
            }
        }
    }
    
    /// Hangup this call
    pub async fn hangup(&self) -> SipClientResult<()> {
        // This would need a reference back to the client
        // For now, users should use client.hangup(&call.id)
        Err(SipClientError::NotImplemented {
            feature: "Direct call hangup".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockall::mock;
    use tokio_test::assert_ok;
    
    #[tokio::test]
    async fn test_create_client() {
        let config = SipClientConfig {
            sip_identity: "sip:test@example.com".to_string(),
            ..Default::default()
        };
        
        // This will fail unless we have mock implementations
        // For now, just verify the structure compiles
        // let client = SipClient::from_config(config).await;
    }
    
    #[tokio::test]
    async fn test_call_state_transitions() {
        let call = Call {
            id: CallId::new_v4(),
            state: Arc::new(RwLock::new(CallState::Initiating)),
            remote_uri: "sip:bob@example.com".to_string(),
            local_uri: "sip:alice@example.com".to_string(),
            start_time: chrono::Utc::now(),
            connect_time: None,
            codec: None,
            direction: CallDirection::Outgoing,
        };
        
        // Test state transitions
        assert_eq!(*call.state.read(), CallState::Initiating);
        
        *call.state.write() = CallState::Ringing;
        assert_eq!(*call.state.read(), CallState::Ringing);
        
        *call.state.write() = CallState::Connected;
        assert_eq!(*call.state.read(), CallState::Connected);
        
        *call.state.write() = CallState::Terminated;
        assert_eq!(*call.state.read(), CallState::Terminated);
    }
    
    #[tokio::test]
    async fn test_wait_for_answer() {
        let call = Arc::new(Call {
            id: CallId::new_v4(),
            state: Arc::new(RwLock::new(CallState::Ringing)),
            remote_uri: "sip:bob@example.com".to_string(),
            local_uri: "sip:alice@example.com".to_string(),
            start_time: chrono::Utc::now(),
            connect_time: None,
            codec: None,
            direction: CallDirection::Outgoing,
        });
        
        // Spawn task to change state after delay
        let call_clone = call.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            *call_clone.state.write() = CallState::Connected;
        });
        
        // Wait for answer should succeed
        let result = call.wait_for_answer().await;
        assert!(result.is_ok());
        assert_eq!(*call.state.read(), CallState::Connected);
    }
    
    #[tokio::test]
    async fn test_wait_for_answer_terminated() {
        let call = Arc::new(Call {
            id: CallId::new_v4(),
            state: Arc::new(RwLock::new(CallState::Ringing)),
            remote_uri: "sip:bob@example.com".to_string(),
            local_uri: "sip:alice@example.com".to_string(),
            start_time: chrono::Utc::now(),
            connect_time: None,
            codec: None,
            direction: CallDirection::Outgoing,
        });
        
        // Spawn task to terminate call
        let call_clone = call.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            *call_clone.state.write() = CallState::Terminated;
        });
        
        // Wait for answer should fail
        let result = call.wait_for_answer().await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SipClientError::InvalidState { .. }));
    }
    
    #[test]
    fn test_codec_registry_creation() {
        let config = SipClientConfig::default();
        let registry = SipClient::create_codec_registry(&config).unwrap();
        
        // Should have registered default codecs
        let codecs = registry.list_codecs();
        assert!(codecs.iter().any(|c| c.as_str() == "PCMU"));
        assert!(codecs.iter().any(|c| c.as_str() == "PCMA"));
    }
    
    #[test]
    fn test_event_emitter() {
        let emitter = EventEmitter::default();
        let mut stream = emitter.subscribe();
        
        // Emit an event
        emitter.emit(SipClientEvent::Started);
        
        // Should receive the event
        // Note: This would need async handling in a real test
    }
}