//! Simple peer API - The API that session-core-v2 should have had from the start
//!
//! This provides a clean, simple interface like the original session-core,
//! hiding the complexity of coordinators, sessions, and roles.

use std::sync::Arc;
use std::net::IpAddr;
use tokio::sync::{mpsc, Mutex};
use crate::api::unified::{UnifiedCoordinator, UnifiedSession, Config};
use crate::state_table::types::{Role, SessionId, CallState};
use crate::errors::{Result, SessionError};
use rvoip_media_core::types::AudioFrame;

/// A simple SIP peer that can make and receive calls
/// 
/// This is the high-level API that users actually want.
pub struct SimplePeer {
    name: String,
    coordinator: Arc<UnifiedCoordinator>,
    active_sessions: Arc<Mutex<Vec<Arc<UnifiedSession>>>>,
}

impl SimplePeer {
    /// Create a new peer with sensible defaults
    pub async fn new(name: &str) -> Result<Self> {
        Self::with_port(name, 5060).await
    }
    
    /// Create a new peer with a specific port
    pub async fn with_port(name: &str, port: u16) -> Result<Self> {
        let config = Config {
            sip_port: port,
            media_port_start: port + 1000,
            media_port_end: port + 2000,
            local_ip: "127.0.0.1".parse().unwrap(),
            bind_addr: format!("127.0.0.1:{}", port).parse()
                .map_err(|_| SessionError::ConfigError("Invalid address".to_string()))?,
            state_table_path: None, // Use default state table
        };
        
        let coordinator = UnifiedCoordinator::new(config).await?;
        
        Ok(Self {
            name: name.to_string(),
            coordinator: coordinator.clone(),
            active_sessions: Arc::new(Mutex::new(Vec::new())),
        })
    }
    
    /// Make an outgoing call
    pub async fn call(&self, target: &str) -> Result<Call> {
        // Create a UAC session for the outgoing call
        let session = UnifiedSession::new(self.coordinator.clone(), Role::UAC).await?;
        let session = Arc::new(session);
        
        // Store the session
        self.active_sessions.lock().await.push(session.clone());
        
        // Make the call
        session.make_call(target).await?;
        
        Ok(Call {
            session: session.clone(),
            direction: CallDirection::Outgoing,
        })
    }
    
    /// Check for incoming calls (non-blocking)
    pub async fn incoming_call(&self) -> Option<IncomingCall> {
        // TODO: This needs proper implementation with coordinator support
        // For now, return None
        None
    }
    
    /// Wait for an incoming call (blocking)
    pub async fn wait_for_call(&self) -> Result<IncomingCall> {
        // Create a UAS session to wait for calls
        let session = UnifiedSession::new(self.coordinator.clone(), Role::UAS).await?;
        let session = Arc::new(session);
        
        // Store the session
        self.active_sessions.lock().await.push(session.clone());
        
        // TODO: This needs proper implementation
        // For now, create a mock incoming call
        Ok(IncomingCall {
            from: "unknown".to_string(),
            session: session.clone(),
        })
    }
}

/// Represents an incoming call that can be accepted or rejected
pub struct IncomingCall {
    from: String,
    session: Arc<UnifiedSession>,
}

impl IncomingCall {
    /// Accept the incoming call
    pub async fn accept(self) -> Result<Call> {
        self.session.accept().await?;
        
        Ok(Call {
            session: self.session,
            direction: CallDirection::Incoming,
        })
    }
    
    /// Reject the incoming call
    pub async fn reject(self) -> Result<()> {
        self.session.reject("Busy").await
    }
}

/// Direction of the call
#[derive(Debug, Clone, Copy)]
enum CallDirection {
    Incoming,
    Outgoing,
}

/// Represents an active call with simple audio I/O
pub struct Call {
    session: Arc<UnifiedSession>,
    direction: CallDirection,
}

impl Call {
    /// Send an audio frame
    pub async fn send_audio(&self, frame: AudioFrame) -> Result<()> {
        self.session.send_audio_frame(frame).await
    }
    
    /// Receive audio frames
    pub async fn audio_stream(&self) -> Result<AudioStream> {
        let subscriber = self.session.subscribe_to_audio_frames().await?;
        Ok(AudioStream { subscriber })
    }
    
    /// Check if call is active
    pub async fn is_active(&self) -> Result<bool> {
        Ok(self.session.state().await? == CallState::Active)
    }
    
    /// Hang up the call
    pub async fn hangup(self) -> Result<()> {
        self.session.hangup().await
    }
}

/// Stream of audio frames from the remote peer
pub struct AudioStream {
    subscriber: crate::adapters::media_adapter::AudioFrameSubscriber,
}

impl AudioStream {
    /// Receive the next audio frame
    pub async fn recv(&mut self) -> Option<AudioFrame> {
        self.subscriber.recv().await
    }
}