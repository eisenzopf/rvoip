//! Simple peer API - The API that session-core-v2 should have had from the start
//!
//! This provides a clean, simple interface like the original session-core,
//! hiding the complexity of coordinators, sessions, and roles.

use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};
use crate::api::{
    unified::{UnifiedCoordinator, UnifiedSession, Config},
    session_manager::{SessionManager, SessionLifecycleEvent},
    call_controller::CallController,
};
use crate::state_table::types::{Role, SessionId, CallState};
use crate::errors::{Result, SessionError};
use rvoip_media_core::types::AudioFrame;

/// A simple SIP peer that can make and receive calls
/// 
/// This is the high-level API that users actually want.
pub struct SimplePeer {
    name: String,
    coordinator: Arc<UnifiedCoordinator>,
    session_manager: Arc<SessionManager>,
    call_controller: Arc<CallController>,
    active_sessions: Arc<RwLock<Vec<Arc<UnifiedSession>>>>,
    incoming_calls: Arc<Mutex<Vec<IncomingCall>>>,
    event_rx: Arc<Mutex<mpsc::Receiver<SessionLifecycleEvent>>>,
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
        
        // Create session manager with event channel
        let (event_tx, event_rx) = mpsc::channel(100);
        let (state_event_tx, _state_event_rx) = mpsc::channel(100);
        let session_manager = Arc::new(SessionManager::new(
            coordinator.session_registry(),
            coordinator.state_machine(),
            state_event_tx,
            event_tx,
        ));
        
        // Create call controller
        let (call_controller, incoming_call_tx) = CallController::new(
            session_manager.clone(),
            coordinator.session_registry(),
            coordinator.dialog_adapter(),
            coordinator.media_adapter(),
        );
        let call_controller = Arc::new(call_controller);
        
        // Start background task to handle incoming calls
        let incoming_calls = Arc::new(Mutex::new(Vec::new()));
        let incoming_calls_clone = incoming_calls.clone();
        let session_manager_clone = session_manager.clone();
        
        tokio::spawn(async move {
            // This task will monitor for incoming calls via SignalingInterceptor events
            // For now, this is a placeholder - actual implementation needs SignalingInterceptor integration
        });
        
        Ok(Self {
            name: name.to_string(),
            coordinator,
            session_manager,
            call_controller,
            active_sessions: Arc::new(RwLock::new(Vec::new())),
            incoming_calls,
            event_rx: Arc::new(Mutex::new(event_rx)),
        })
    }
    
    /// Make an outgoing call
    pub async fn call(&self, target: &str) -> Result<Call> {
        // Use CallController to make the call
        let from = format!("sip:{}@localhost", self.name);
        let session_id = self.call_controller.make_call(from, target.to_string()).await?;
        
        // Create a UAC session wrapper for the call
        let session = UnifiedSession::new(self.coordinator.clone(), Role::UAC).await?;
        let session = Arc::new(session);
        
        // Store the session
        self.active_sessions.write().await.push(session.clone());
        
        Ok(Call {
            session: session.clone(),
            direction: CallDirection::Outgoing,
        })
    }
    
    /// Check for incoming calls (non-blocking)
    pub async fn incoming_call(&self) -> Option<IncomingCall> {
        // Check if there are any queued incoming calls
        let mut calls = self.incoming_calls.lock().await;
        calls.pop()
    }
    
    /// Wait for an incoming call (blocking)
    pub async fn wait_for_call(&self) -> Result<IncomingCall> {
        loop {
            // Check for queued incoming calls
            if let Some(call) = self.incoming_call().await {
                return Ok(call);
            }
            
            // Wait for lifecycle events
            let mut rx = self.event_rx.lock().await;
            while let Some(event) = rx.recv().await {
                if let SessionLifecycleEvent::IncomingCall { session_id, from } = event {
                    // Create a UAS session for the incoming call
                    let session = UnifiedSession::new(self.coordinator.clone(), Role::UAS).await?;
                    let session = Arc::new(session);
                    
                    // Store the session
                    self.active_sessions.write().await.push(session.clone());
                    
                    return Ok(IncomingCall {
                        from,
                        session: session.clone(),
                        session_id,
                    });
                }
            }
        }
    }
}

/// Represents an incoming call that can be accepted or rejected
pub struct IncomingCall {
    from: String,
    session: Arc<UnifiedSession>,
    session_id: SessionId,
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