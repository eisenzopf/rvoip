//! Simple API for SIP client

use crate::{
    error::{SipClientError, SipClientResult},
    events::{EventEmitter, EventStream, SipClientEvent},
    types::{Call, CallDirection, CallId, CallState, SipClientConfig},
};
use std::sync::Arc;
use parking_lot::RwLock;
use std::collections::HashMap;

/// Simple SIP client with easy-to-use API
pub struct SipClient {
    /// Internal state
    inner: Arc<SipClientInner>,
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
    /// Event emitter
    events: EventEmitter,
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
        
        let inner = Arc::new(SipClientInner {
            config,
            client,
            audio_manager,
            codec_registry,
            calls: Arc::new(RwLock::new(HashMap::new())),
            events: EventEmitter::default(),
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
    
    /// Subscribe to events
    pub fn events(&self) -> EventStream {
        self.inner.events.subscribe()
    }
    
    /// Get active calls
    pub fn active_calls(&self) -> Vec<Arc<Call>> {
        self.inner.calls.read().values().cloned().collect()
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
    
    async fn setup_audio_pipeline(&self, _call: &Arc<Call>) -> SipClientResult<()> {
        // TODO: Set up audio pipeline for the call
        // 1. Get audio devices
        // 2. Configure codec based on negotiation
        // 3. Create audio pipeline
        // 4. Connect to RTP session
        Ok(())
    }
    
    async fn cleanup_audio_pipeline(&self, _call_id: &CallId) -> SipClientResult<()> {
        // TODO: Clean up audio pipeline
        Ok(())
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