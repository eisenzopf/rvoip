//! Simple UAC Client API - The easiest way to make SIP calls
//! 
//! This module provides a simplified interface for making SIP calls with
//! automatic audio channel setup and minimal configuration.

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{RwLock, mpsc};
use uuid::Uuid;
use crate::api::builder::SessionManagerConfig;
use crate::api::common::{setup_audio_channels, parse_call_target, operations, Protocol};
use crate::api::control::SessionControl;
use crate::api::media::MediaControl;
use crate::api::types::{SessionId, CallState, AudioFrame};
use crate::coordinator::SessionCoordinator;
use crate::errors::{Result, SessionError};

// ============================================================================
// CLIENT BUILDER
// ============================================================================

/// Builder for creating a SimpleUacClient
pub struct SimpleUacClientBuilder {
    identity: String,
    local_addr: String,
    port: u16,
}

impl SimpleUacClientBuilder {
    fn new(identity: String) -> Self {
        Self {
            identity,
            local_addr: "127.0.0.1".to_string(),
            port: 5060,
        }
    }
    
    /// Set the local IP address (defaults to 127.0.0.1)
    pub fn local_addr(mut self, addr: &str) -> Self {
        self.local_addr = addr.to_string();
        self
    }
    
    /// Set the local port (defaults to 5060)
    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }
}

// Implement Future for the builder to support .await
impl std::future::IntoFuture for SimpleUacClientBuilder {
    type Output = Result<SimpleUacClient>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send>>;
    
    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let local_bind_addr = format!("{}:{}", self.local_addr, self.port)
                .parse()
                .map_err(|_| SessionError::ConfigError("Invalid local address".to_string()))?;
            
            let config = SessionManagerConfig {
                sip_port: self.port,
                local_address: format!("sip:{}@{}:{}", self.identity, self.local_addr, self.port),
                local_bind_addr,
                media_port_start: 10000,
                media_port_end: 20000,
                enable_stun: false,
                stun_server: None,
                enable_sip_client: true,
                media_config: Default::default(),
            };
            
            let coordinator = SessionCoordinator::new(config, None).await?;
            coordinator.start().await?;
            
            Ok(SimpleUacClient {
                coordinator,
                identity: self.identity,
                registered_to: Arc::new(RwLock::new(None)),
            })
        })
    }
}

// ============================================================================
// SIMPLE UAC CLIENT
// ============================================================================

/// Simple UAC Client for making SIP calls
pub struct SimpleUacClient {
    coordinator: Arc<SessionCoordinator>,
    identity: String,
    registered_to: Arc<RwLock<Option<String>>>,
}

impl SimpleUacClient {
    /// Create a new UAC client with the given identity
    /// 
    /// # Example
    /// ```rust
    /// use rvoip_session_core::api::uac::SimpleUacClient;
    /// 
    /// let builder = SimpleUacClient::new("alice");
    /// // The builder will need to be awaited to create the client
    /// ```
    pub fn new(identity: &str) -> SimpleUacClientBuilder {
        SimpleUacClientBuilder::new(identity.to_string())
    }
    
    /// Register with a SIP registrar to receive calls
    pub async fn register(&self, registrar: &str) -> Result<()> {
        // TODO: Implement actual registration when available in SessionControl
        *self.registered_to.write().await = Some(registrar.to_string());
        tracing::info!("Registered {} to {}", self.identity, registrar);
        Ok(())
    }
    
    /// Unregister from the SIP registrar
    pub async fn unregister(&self) -> Result<()> {
        if let Some(registrar) = self.registered_to.write().await.take() {
            tracing::info!("Unregistered {} from {}", self.identity, registrar);
        }
        Ok(())
    }
    
    /// Make a call to the specified target
    /// 
    /// # Example
    /// ```rust,ignore
    /// let call = client.call("bob@example.com").await?;
    /// let call = client.call("bob@example.com").port(5070).await?;
    /// let call = client.call("tel:+14155551234").await?;
    /// ```
    pub fn call(&self, target: &str) -> CallBuilder {
        CallBuilder::new(self, target)
    }
    
    /// Shutdown the client and cleanup resources
    pub async fn shutdown(self) -> Result<()> {
        self.unregister().await?;
        self.coordinator.stop().await?;
        Ok(())
    }
}

// ============================================================================
// CALL BUILDER
// ============================================================================

/// Builder for creating calls with optional parameters
pub struct CallBuilder<'a> {
    client: &'a SimpleUacClient,
    target: String,
    port: u16,
    call_id: Option<String>,
}

impl<'a> CallBuilder<'a> {
    fn new(client: &'a SimpleUacClient, target: &str) -> Self {
        Self {
            client,
            target: target.to_string(),
            port: 5060,
            call_id: None,
        }
    }
    
    /// Set the target port (defaults to 5060)
    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }
    
    /// Set a custom call ID (auto-generated if not set)
    pub fn call_id(mut self, id: &str) -> Self {
        self.call_id = Some(id.to_string());
        self
    }
}

// Implement Future for the call builder to support .await
impl<'a> std::future::IntoFuture for CallBuilder<'a> {
    type Output = Result<SimpleCall>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;
    
    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            // Parse the target and determine protocol
            let (uri, protocol) = parse_call_target(&self.target, self.port);
            
            // Generate call ID if not provided
            let call_id = self.call_id.unwrap_or_else(|| {
                format!("call-{}", Uuid::new_v4())
            });
            
            // Create the call using SessionControl
            let from_uri = format!("sip:{}@{}", 
                self.client.identity,
                self.client.coordinator.config().local_bind_addr
            );
            
            // Prepare and initiate the call
            let prepared = SessionControl::prepare_outgoing_call(
                &self.client.coordinator,
                &from_uri,
                &uri,
            ).await?;
            
            let session = SessionControl::initiate_prepared_call(
                &self.client.coordinator,
                &prepared,
            ).await?;
            
            // Set up audio channels automatically
            let (audio_tx, audio_rx) = setup_audio_channels(
                &self.client.coordinator,
                &session.id,
            ).await?;
            
            // Create the SimpleCall
            Ok(SimpleCall {
                session_id: session.id,
                coordinator: self.client.coordinator.clone(),
                audio_tx: Some(audio_tx),
                audio_rx: Some(audio_rx),
                remote_uri: uri,
                start_time: Instant::now(),
                state: Arc::new(RwLock::new(CallState::Active)),
            })
        })
    }
}

// ============================================================================
// SIMPLE CALL
// ============================================================================

/// A simple call handle with all operations
pub struct SimpleCall {
    session_id: SessionId,
    coordinator: Arc<SessionCoordinator>,
    audio_tx: Option<mpsc::Sender<AudioFrame>>,
    audio_rx: Option<mpsc::Receiver<AudioFrame>>,
    remote_uri: String,
    start_time: Instant,
    state: Arc<RwLock<CallState>>,
}

impl SimpleCall {
    /// Get the audio channels for this call
    /// 
    /// Returns (tx, rx) where:
    /// - tx: Send audio to remote party
    /// - rx: Receive audio from remote party
    /// 
    /// Note: This consumes the channels, can only be called once
    pub fn audio_channels(&mut self) -> (mpsc::Sender<AudioFrame>, mpsc::Receiver<AudioFrame>) {
        let tx = self.audio_tx.take().expect("Audio channels already taken");
        let rx = self.audio_rx.take().expect("Audio channels already taken");
        (tx, rx)
    }
    
    /// Put the call on hold
    pub async fn hold(&self) -> Result<()> {
        operations::hold(&self.coordinator, &self.session_id).await?;
        *self.state.write().await = CallState::OnHold;
        Ok(())
    }
    
    /// Resume the call from hold
    pub async fn unhold(&self) -> Result<()> {
        operations::unhold(&self.coordinator, &self.session_id).await?;
        *self.state.write().await = CallState::Active;
        Ok(())
    }
    
    /// Mute audio transmission (stop sending audio)
    pub async fn mute(&self) -> Result<()> {
        operations::mute(&self.coordinator, &self.session_id).await
    }
    
    /// Unmute audio transmission (resume sending audio)
    pub async fn unmute(&self) -> Result<()> {
        operations::unmute(&self.coordinator, &self.session_id).await
    }
    
    /// Send DTMF digits
    pub async fn send_dtmf(&self, digits: &str) -> Result<()> {
        operations::send_dtmf(&self.coordinator, &self.session_id, digits).await
    }
    
    /// Transfer this call to another target
    pub fn transfer(&self, target: &str) -> TransferBuilder {
        TransferBuilder::new(self, target)
    }
    
    /// Bridge this call with another call (3-way conference)
    pub async fn bridge(&self, other: SimpleCall) -> Result<()> {
        operations::bridge(&self.coordinator, &self.session_id, &other.session_id).await
    }
    
    /// Get the call ID
    pub fn id(&self) -> &str {
        self.session_id.as_str()
    }
    
    /// Get the remote party URI
    pub fn remote_uri(&self) -> &str {
        &self.remote_uri
    }
    
    /// Get the call duration
    pub fn duration(&self) -> Duration {
        self.start_time.elapsed()
    }
    
    /// Get the call state
    pub async fn state(&self) -> CallState {
        self.state.read().await.clone()
    }
    
    /// Get the packet loss rate (0.0 to 1.0)
    pub async fn packet_loss_rate(&self) -> f32 {
        operations::get_packet_loss_rate(&self.coordinator, &self.session_id)
            .await
            .unwrap_or(0.0)
    }
    
    /// Hang up the call
    pub async fn hangup(self) -> Result<()> {
        *self.state.write().await = CallState::Terminated;
        operations::hangup(&self.coordinator, &self.session_id).await
    }
}

// ============================================================================
// TRANSFER BUILDER
// ============================================================================

/// Builder for transfer operations
pub struct TransferBuilder<'a> {
    call: &'a SimpleCall,
    target: String,
    attended_call: Option<SimpleCall>,
}

impl<'a> TransferBuilder<'a> {
    fn new(call: &'a SimpleCall, target: &str) -> Self {
        Self {
            call,
            target: target.to_string(),
            attended_call: None,
        }
    }
    
    /// Make this an attended transfer (consult before transferring)
    pub fn attended(mut self, call: SimpleCall) -> Self {
        self.attended_call = Some(call);
        self
    }
}

// Implement Future for transfer builder
impl<'a> std::future::IntoFuture for TransferBuilder<'a> {
    type Output = Result<()>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;
    
    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            if let Some(attended) = self.attended_call {
                // Attended transfer - transfer the call to the consultation call
                // TODO: Implement attended transfer when available
                tracing::warn!("Attended transfer not yet implemented");
                attended.hangup().await?;
                operations::transfer(&self.call.coordinator, &self.call.session_id, &self.target).await
            } else {
                // Blind transfer
                operations::transfer(&self.call.coordinator, &self.call.session_id, &self.target).await
            }
        })
    }
}