# Session-Core-V2 Development Design

## Overview
This document details the specific code changes needed to implement the architecture fix for session-core-v2, based on the successful patterns from the sip-client library.

## File Organization

### New Files to Create
- `src/adapters/signaling_interceptor.rs` - SIP signaling event interceptor
- `src/session_registry.rs` - Central ID mapping registry

### Files to Modify
- `src/api/simple.rs` - Complete the SimplePeer, Call, IncomingCall implementation
- `src/api/types.rs` - Add CallId, CallDirection, CallState types
- `src/api/unified.rs` - Add factory methods and incoming call support
- `src/adapters/dialog_adapter.rs` - Add interceptor hook
- `src/adapters/session_event_handler.rs` - Modify to skip new calls
- `src/api/mod.rs` - Export SimplePeer and related types
- `src/lib.rs` - Public API exports

## Complete SimplePeer API

### File: `src/api/simple.rs` (MODIFY existing file)
```rust
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};
use std::collections::HashMap;
use crate::api::types::{CallId, CallDirection, CallState};
use crate::api::unified::{UnifiedCoordinator, UnifiedSession};

/// Simple SIP peer that matches the sip-client ease of use
pub struct SimplePeer {
    /// Peer name/identity
    name: String,
    
    /// SIP URI (e.g., "sip:alice@127.0.0.1:5060")
    sip_uri: String,
    
    /// Coordinator handles all the complexity
    coordinator: Arc<UnifiedCoordinator>,
    
    /// Channel for incoming calls
    incoming_rx: Mutex<mpsc::Receiver<IncomingCall>>,
    
    /// Active calls
    active_calls: Arc<RwLock<HashMap<CallId, Arc<Call>>>>,
}

impl SimplePeer {
    /// Create new peer with just a name and port
    /// Example: SimplePeer::new("alice", 5060)
    pub async fn new(name: &str, port: u16) -> Result<Self> {
        let sip_uri = format!("sip:{}@127.0.0.1:{}", name, port);
        Self::with_uri(&sip_uri).await
    }
    
    /// Create with full SIP URI
    /// Example: SimplePeer::with_uri("sip:alice@example.com:5060")
    pub async fn with_uri(sip_uri: &str) -> Result<Self> {
        // Parse URI to get port
        let port = extract_port_from_uri(sip_uri).unwrap_or(5060);
        
        // Create config
        let config = Config {
            sip_port: port,
            media_port_start: port + 1000,
            media_port_end: port + 2000,
            local_ip: "127.0.0.1".parse().unwrap(),
            bind_addr: format!("0.0.0.0:{}", port).parse()?,
            state_table_path: None, // Use default
        };
        
        // Create coordinator with incoming call handler
        let (coordinator, incoming_rx) = UnifiedCoordinator::new_with_handler(config).await?;
        
        Ok(Self {
            name: extract_name_from_uri(sip_uri),
            sip_uri: sip_uri.to_string(),
            coordinator: Arc::new(coordinator),
            incoming_rx: Mutex::new(incoming_rx),
            active_calls: Arc::new(RwLock::new(HashMap::new())),
        })
    }
    
    /// Make an outgoing call
    /// Example: peer.call("sip:bob@127.0.0.1:5061")
    pub async fn call(&self, target: &str) -> Result<Arc<Call>> {
        // Coordinator creates UAC session internally
        let session = self.coordinator.create_outgoing_call(target).await?;
        
        // Wrap in Call object
        let call = Arc::new(Call::new(session, CallDirection::Outgoing));
        
        // Track active call
        self.active_calls.write().await.insert(call.id(), call.clone());
        
        Ok(call)
    }
    
    /// Check for incoming call (non-blocking)
    /// Returns None if no calls pending
    pub async fn try_incoming(&self) -> Option<IncomingCall> {
        self.incoming_rx.lock().await.try_recv().ok()
    }
    
    /// Wait for incoming call (blocking)
    pub async fn wait_for_incoming(&self) -> Result<IncomingCall> {
        self.incoming_rx.lock().await.recv()
            .await
            .ok_or_else(|| SessionError::ChannelClosed("Incoming call channel closed".into()))
    }
    
    /// Get active calls
    pub async fn active_calls(&self) -> Vec<Arc<Call>> {
        self.active_calls.read().await.values().cloned().collect()
    }
}
```

### File: `src/api/simple.rs` (continued)
```rust
/// Active call handle
pub struct Call {
    /// Internal session (hidden complexity)
    session: Arc<UnifiedSession>,
    
    /// Call direction
    direction: CallDirection,
    
    /// Call ID
    id: CallId,
    
    /// Audio receiver (simplified)
    audio_rx: Arc<Mutex<mpsc::Receiver<AudioFrame>>>,
}

impl Call {
    /// Send audio frame
    pub async fn send_audio(&self, frame: AudioFrame) -> Result<()> {
        self.session.send_audio(frame).await
    }
    
    /// Receive audio frame (blocking)
    pub async fn recv_audio(&self) -> Option<AudioFrame> {
        self.audio_rx.lock().await.recv().await
    }
    
    /// Try to receive audio (non-blocking)
    pub async fn try_recv_audio(&self) -> Option<AudioFrame> {
        self.audio_rx.lock().await.try_recv().ok()
    }
    
    /// Check if call is active
    pub async fn is_active(&self) -> bool {
        self.session.state().await == CallState::Active
    }
    
    /// Wait for call to be answered (for outgoing calls)
    pub async fn wait_for_answer(&self) -> Result<()> {
        // Poll state until Active or Failed
        loop {
            match self.session.state().await {
                CallState::Active => return Ok(()),
                CallState::Failed | CallState::Terminated => {
                    return Err(SessionError::CallFailed("Call was not answered".into()))
                }
                _ => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
    }
    
    /// Hang up the call
    pub async fn hangup(self) -> Result<()> {
        self.session.hangup().await
    }
    
    /// Get call ID
    pub fn id(&self) -> CallId {
        self.id.clone()
    }
    
    /// Get remote party
    pub fn remote_party(&self) -> &str {
        self.session.remote_uri()
    }
}
```

### File: `src/api/simple.rs` (continued)
```rust
/// Incoming call that needs accept/reject decision
pub struct IncomingCall {
    /// Who's calling
    from: String,
    
    /// Pre-created session (by coordinator)
    session: Arc<UnifiedSession>,
    
    /// SDP offer from caller
    sdp: Option<String>,
}

impl IncomingCall {
    /// Get caller identity
    pub fn from(&self) -> &str {
        &self.from
    }
    
    /// Accept the call
    pub async fn accept(self) -> Result<Arc<Call>> {
        // Tell state machine to accept
        self.session.accept().await?;
        
        // Create Call object
        let call = Arc::new(Call::new(self.session, CallDirection::Incoming));
        
        Ok(call)
    }
    
    /// Reject the call with reason
    pub async fn reject(self, reason: &str) -> Result<()> {
        self.session.reject(reason).await
    }
    
    /// Reject with default reason
    pub async fn reject_busy(self) -> Result<()> {
        self.reject("Busy").await
    }
}
```

### File: `src/api/types.rs` (MODIFY existing file)
```rust
//! Common types used by the simple API

use std::fmt;

/// Unique call identifier
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct CallId(pub String);

impl CallId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

impl fmt::Display for CallId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Call direction
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallDirection {
    Incoming,
    Outgoing,
}

/// Call state (simplified from internal state machine)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallState {
    Idle,
    Calling,
    Ringing,
    Active,
    Terminating,
    Terminated,
    Failed,
}
```

### File: `src/session_registry.rs` (NEW)
```rust
//! Central registry for mapping between SessionId, DialogId, and MediaSessionId

use dashmap::DashMap;
use std::sync::Arc;
use crate::state_table::types::SessionId;

/// Dialog ID from dialog-core
pub type DialogId = String;

/// Media session ID from media-core  
pub type MediaSessionId = String;

/// Thread-safe registry for ID mappings
#[derive(Clone)]
pub struct SessionRegistry {
    /// SessionId -> DialogId mapping
    session_to_dialog: Arc<DashMap<SessionId, DialogId>>,
    
    /// DialogId -> SessionId mapping
    dialog_to_session: Arc<DashMap<DialogId, SessionId>>,
    
    /// SessionId -> MediaSessionId mapping
    session_to_media: Arc<DashMap<SessionId, MediaSessionId>>,
    
    /// MediaSessionId -> SessionId mapping
    media_to_session: Arc<DashMap<MediaSessionId, SessionId>>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self {
            session_to_dialog: Arc::new(DashMap::new()),
            dialog_to_session: Arc::new(DashMap::new()),
            session_to_media: Arc::new(DashMap::new()),
            media_to_session: Arc::new(DashMap::new()),
        }
    }
    
    /// Register a dialog ID for a session
    pub fn register_dialog(&self, session_id: SessionId, dialog_id: DialogId) {
        self.session_to_dialog.insert(session_id.clone(), dialog_id.clone());
        self.dialog_to_session.insert(dialog_id, session_id);
    }
    
    /// Register a media session ID for a session
    pub fn register_media(&self, session_id: SessionId, media_id: MediaSessionId) {
        self.session_to_media.insert(session_id.clone(), media_id.clone());
        self.media_to_session.insert(media_id, session_id);
    }
    
    /// Look up SessionId by DialogId
    pub fn get_session_by_dialog(&self, dialog_id: &DialogId) -> Option<SessionId> {
        self.dialog_to_session.get(dialog_id).map(|e| e.clone())
    }
    
    /// Look up SessionId by MediaSessionId
    pub fn get_session_by_media(&self, media_id: &MediaSessionId) -> Option<SessionId> {
        self.media_to_session.get(media_id).map(|e| e.clone())
    }
    
    /// Look up DialogId by SessionId
    pub fn get_dialog_by_session(&self, session_id: &SessionId) -> Option<DialogId> {
        self.session_to_dialog.get(session_id).map(|e| e.clone())
    }
    
    /// Look up MediaSessionId by SessionId
    pub fn get_media_by_session(&self, session_id: &SessionId) -> Option<MediaSessionId> {
        self.session_to_media.get(session_id).map(|e| e.clone())
    }
    
    /// Clean up all mappings for a session
    pub fn remove_session(&self, session_id: &SessionId) {
        // Remove dialog mappings
        if let Some((_, dialog_id)) = self.session_to_dialog.remove(session_id) {
            self.dialog_to_session.remove(&dialog_id);
        }
        
        // Remove media mappings
        if let Some((_, media_id)) = self.session_to_media.remove(session_id) {
            self.media_to_session.remove(&media_id);
        }
    }
}
```

## Modified Structures

### File: `src/api/unified.rs`
```rust
use crate::session_registry::{SessionRegistry, DialogId, MediaSessionId};

pub struct UnifiedCoordinator {
    // Existing fields
    pub store: Arc<SessionStore>,
    pub event_router: Arc<EventRouter>,
    pub media_adapter: Arc<MediaAdapter>,
    dialog_adapter: Arc<DialogAdapter>,
    state_machine: Arc<StateMachineExecutor>,
    
    // NEW FIELDS
    session_registry: Arc<SessionRegistry>,  // NEW: Central ID registry
    incoming_tx: mpsc::Sender<IncomingCall>,
    incoming_rx: Arc<Mutex<mpsc::Receiver<IncomingCall>>>,
    signaling_interceptor: Arc<SignalingInterceptor>, // NEW
}

impl UnifiedCoordinator {
    // MODIFIED: Now sets up transport interception and registry
    pub async fn new(config: Config) -> Result<Arc<Self>> {
        // ... existing setup ...
        
        // NEW: Create session registry
        let session_registry = Arc::new(SessionRegistry::new());
        
        // ... rest of initialization
    }
    
    // NEW: Get incoming call channel
    pub fn incoming_call_receiver(&self) -> mpsc::Receiver<IncomingCall>
    
    // NEW: Internal session factory
    async fn create_session_for_incoming(&self, from: String, dialog_id: DialogId) -> Result<SessionId> {
        let session_id = SessionId::new();
        
        // Register in registry
        self.session_registry.register_dialog(session_id.clone(), dialog_id);
        
        // ... rest of session creation
    }
}
```

### File: `src/adapters/signaling_interceptor.rs` (NEW)
```rust
pub struct SignalingInterceptor {
    coordinator: Weak<UnifiedCoordinator>,
    state_machine: Arc<StateMachineExecutor>,
}

impl SignalingInterceptor {
    // Intercepts all SIP signaling events before state machine
    pub async fn handle_signaling_event(&self, event: SipSignalingEvent) -> Result<()> {
        match event {
            SipSignalingEvent::IncomingInvite(invite) => {
                // Create UAS session
                let session_id = self.create_uas_session(invite).await?;
                // Notify user
                self.notify_incoming_call(session_id, invite.from).await?;
                // Then route to state machine
                self.route_to_state_machine(session_id, EventType::IncomingCall).await?;
            }
            SipSignalingEvent::Response(dialog_id, response) => {
                // Find existing session
                if let Some(session_id) = self.find_session(dialog_id) {
                    self.route_to_state_machine(session_id, convert_response(response)).await?;
                }
            }
        }
    }
}
```

## Modified Functions

### File: `src/adapters/dialog_adapter.rs`
```rust
impl DialogAdapter {
    // NEW: Accept event handler for transport events
    pub fn set_signaling_interceptor(&self, interceptor: Arc<SignalingInterceptor>)
    
    // MODIFIED: Routes events through handler instead of direct to state machine
    async fn handle_incoming_event(&self, event: DialogEvent)
}
```

### File: `src/api/unified.rs` (modifications to UnifiedSession)
```rust
impl UnifiedSession {
    // REMOVE: Don't expose role in constructor
    // OLD: pub async fn new(coordinator: Arc<UnifiedCoordinator>, role: Role) -> Result<Self>
    
    // NEW: Internal use only, created by coordinator
    pub(crate) async fn new_internal(
        coordinator: Arc<UnifiedCoordinator>, 
        role: Role,
        session_id: SessionId
    ) -> Result<Self>
    
    // SIMPLIFIED: Audio methods return frames directly
    pub async fn recv_audio(&self) -> Option<AudioFrame>
}
```

## Real-World Usage Examples

### Example 1: Simple Softphone with Bidirectional Audio
```rust
#[tokio::main]
async fn main() -> Result<()> {
    // One-line setup
    let peer = SimplePeer::new("alice", 5060).await?;
    
    // Handle incoming calls in background
    let peer_clone = peer.clone();
    tokio::spawn(async move {
        while let Ok(incoming) = peer_clone.wait_for_incoming().await {
            println!("Call from {}", incoming.from());
            let call = incoming.accept().await?;
            
            // Bidirectional audio for incoming call
            tokio::spawn(async move {
                loop {
                    // Send audio to caller
                    let outgoing = capture_from_mic();
                    call.send_audio(outgoing).await?;
                    
                    // Receive audio from caller
                    if let Some(incoming) = call.try_recv_audio().await {
                        play_to_speaker(incoming);
                    }
                    
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            });
        }
    });
    
    // Make outgoing call
    let call = peer.call("sip:bob@server.com").await?;
    call.wait_for_answer().await?;
    
    // Bidirectional audio loop
    loop {
        // Capture and send
        let frame = capture_from_mic();
        call.send_audio(frame).await?;
        
        // Receive and play
        if let Some(frame) = call.try_recv_audio().await {
            play_to_speaker(frame);
        }
        
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
```

### How Audio Reception Works

#### SimplePeer Level
```rust
impl Call {
    /// Blocking receive - waits for next frame
    pub async fn recv_audio(&self) -> Option<AudioFrame> {
        self.audio_rx.lock().await.recv().await
    }
    
    /// Non-blocking receive - returns immediately
    pub async fn try_recv_audio(&self) -> Option<AudioFrame> {
        self.audio_rx.lock().await.try_recv().ok()
    }
}
```

#### UnifiedCoordinator Level (for advanced users)
```rust
// Get audio subscriber for direct access
let mut audio_rx = session.subscribe_to_audio().await?;

// Process incoming audio frames
while let Some(frame) = audio_rx.recv().await {
    // Custom audio processing
    process_audio(frame);
}
```

The audio flows from:
1. RTP packets arrive at MediaAdapter
2. MediaAdapter decodes and sends to session's audio channel
3. SimplePeer's Call object exposes simple recv methods
4. UnifiedCoordinator users can access the raw subscriber

### Example 2: Call Center Server
```rust
// Multiple concurrent calls
let peer = SimplePeer::new("callcenter", 5060).await?;

// Route incoming calls to agents
while let Ok(incoming) = peer.wait_for_incoming().await {
    tokio::spawn(async move {
        // Find available agent
        let agent = find_available_agent().await;
        
        // Accept and bridge
        let customer_call = incoming.accept().await?;
        let agent_call = peer.call(&agent.uri).await?;
        
        // Bridge audio between calls
        bridge_calls(customer_call, agent_call).await;
    });
}
```

### Example 3: Call-Engine Integration
```rust
// Call-engine provides its own handler for complex routing
struct CallEngineSignalingHandler {
    queue_manager: Arc<QueueManager>,
    routing_engine: Arc<RoutingEngine>,
}

impl SignalingHandler for CallEngineSignalingHandler {
    async fn on_incoming_invite(&self, invite: InviteDetails) -> SignalingDecision {
        // Check routing rules
        match self.routing_engine.route(&invite.from, &invite.to).await {
            Route::Queue(queue_id) => {
                // Custom: add to queue, don't create session yet
                SignalingDecision::Custom(Box::new(move || {
                    self.queue_manager.enqueue(queue_id, invite);
                }))
            }
            Route::Reject(reason) => {
                SignalingDecision::Reject(reason.to_sip_code())
            }
            Route::DirectAgent(agent_uri) => {
                // Accept and let state machine handle
                SignalingDecision::Accept
            }
        }
    }
}

// In call-engine initialization
let handler = Arc::new(CallEngineSignalingHandler::new());
let coordinator = UnifiedCoordinator::with_handler(config, handler).await?;
```

### Example 4: Media Server
```rust
// Play announcement to callers
let peer = SimplePeer::new("ivr", 5060).await?;

while let Ok(incoming) = peer.wait_for_incoming().await {
    let call = incoming.accept().await?;
    
    // Play welcome message
    let wav_frames = load_wav("welcome.wav");
    for frame in wav_frames {
        call.send_audio(frame).await?;
    }
    
    // Record caller's message
    let mut recording = Vec::new();
    while let Some(frame) = call.recv_audio().await {
        recording.push(frame);
    }
    save_wav("message.wav", recording);
}
```

## Implementation Steps

### File: `src/api/mod.rs` (modifications)
```rust
//! Session Core V2 API modules

pub mod unified;
pub mod simple;  // Already exists
pub mod types;   // Already exists

// Re-export for convenience
pub use simple::{SimplePeer, Call, IncomingCall};
pub use types::{CallId, CallDirection, CallState};
pub use unified::{UnifiedCoordinator, UnifiedSession};  // Keep for compatibility
```

### File: `src/lib.rs` (modifications)
```rust
//! Session Core V2 - State-driven SIP session management

// ... existing exports ...

// NEW: Simple API as the primary interface
pub use api::simple::{SimplePeer, Call, IncomingCall};
pub use api::types::{CallId, CallDirection, CallState};

// Deprecated but kept for compatibility
#[deprecated(note = "Use SimplePeer instead")]
pub use api::unified::{UnifiedCoordinator, UnifiedSession};
```

### Phase 1: Signaling Interception (CRITICAL)

#### File: `src/adapters/signaling_interceptor.rs` (NEW)
```rust
/// Extensible handler for signaling decisions
#[async_trait]
pub trait SignalingHandler: Send + Sync {
    /// Handle incoming INVITE
    async fn on_incoming_invite(&self, invite: InviteDetails) -> SignalingDecision;
    
    /// Handle SIP response
    async fn on_response(&self, dialog_id: DialogId, response: SipResponse) -> SignalingDecision;
}

/// Decision from handler
pub enum SignalingDecision {
    Accept,              // Create session and proceed
    Reject(u16),         // Send SIP error code
    Defer,               // Let higher layer handle
    Custom(Box<dyn FnOnce() + Send>), // Custom action
}

/// Default handler for SimplePeer
struct DefaultSignalingHandler;

impl SignalingHandler for DefaultSignalingHandler {
    async fn on_incoming_invite(&self, invite: InviteDetails) -> SignalingDecision {
        SignalingDecision::Accept // Auto-accept all calls
    }
    
    async fn on_response(&self, _: DialogId, _: SipResponse) -> SignalingDecision {
        SignalingDecision::Accept // Process normally
    }
}

pub struct SignalingInterceptor {
    coordinator: Weak<UnifiedCoordinator>,
    incoming_tx: mpsc::Sender<IncomingCall>,
    session_registry: Arc<SessionRegistry>,
    handler: Arc<dyn SignalingHandler>,  // Extensible handler
}

impl SignalingInterceptor {
    pub fn new_with_handler(handler: Arc<dyn SignalingHandler>) -> Self {
        Self { 
            // ... other fields
            handler,
        }
    }
    
    pub async fn handle_signaling_event(&self, event: DialogEvent) -> Result<()> {
        match event {
            DialogEvent::IncomingInvite { from, to, dialog_id, sdp } => {
                // Ask handler what to do
                let invite = InviteDetails { from, to, dialog_id, sdp };
                let decision = self.handler.on_incoming_invite(invite).await;
                
                match decision {
                    SignalingDecision::Accept => {
                        // Default behavior: create session
                        let coordinator = self.coordinator.upgrade().unwrap();
                        let session = coordinator.create_uas_session(from, dialog_id.clone(), sdp).await?;
                        
                        // Notify user via channel
                        self.incoming_tx.send(IncomingCall { 
                            from, 
                            session,
                            sdp 
                        }).await?;
                        
                        // Route to state machine
                        coordinator.route_to_state_machine(session.id, EventType::IncomingCall).await?;
                    }
                    SignalingDecision::Reject(code) => {
                        // Send SIP rejection
                        self.send_reject(dialog_id, code).await?;
                    }
                    SignalingDecision::Defer => {
                        // Let higher layer (call-engine) handle it
                        return Ok(());
                    }
                    SignalingDecision::Custom(action) => {
                        // Execute custom logic
                        action();
                    }
                }
            }
            DialogEvent::Response { dialog_id, response } => {
                // Use registry to find session
                if let Some(session_id) = self.session_registry.get_session_by_dialog(&dialog_id) {
                    self.route_to_state_machine(session_id, convert_response(response)).await?;
                }
            }
            _ => {
                // Other events need registry lookup too
                self.route_existing_event(event).await?;
            }
        }
        Ok(())
    }
}
```rust

#### File: `src/adapters/dialog_adapter.rs` (modifications)
impl DialogAdapter {
    pub fn set_interceptor(&self, interceptor: Arc<SignalingInterceptor>) {
        self.interceptor.set(Some(interceptor));
    }
}
```

### Phase 2: Session Factory

#### File: `src/api/unified.rs` (add factory methods)
```rust
impl UnifiedCoordinator {
    /// Create session for incoming call
    async fn create_uas_session(
        &self, 
        from: String,
        dialog_id: DialogId,
        sdp: Option<String>
    ) -> Result<Arc<UnifiedSession>> {
        let session_id = SessionId::new();
        
        // Create session in store
        self.store.create_session(session_id.clone(), Role::UAS, false).await?;
        
        // Map dialog to session
        self.dialog_to_session.insert(dialog_id, session_id.clone());
        
        // Create session object
        let session = UnifiedSession::new_internal(self.clone(), Role::UAS, session_id).await?;
        
        // Store remote SDP if provided
        if let Some(sdp_data) = sdp {
            session.set_remote_sdp(sdp_data).await?;
        }
        
        Ok(Arc::new(session))
    }
    
    /// Create session for outgoing call
    pub async fn create_outgoing_call(&self, target: &str) -> Result<Arc<UnifiedSession>> {
        let session = UnifiedSession::new(self.clone(), Role::UAC).await?;
        session.make_call(target).await?;
        Ok(Arc::new(session))
    }
}
```

### Phase 3: Complete the Examples

#### File: `crates/session-core-v2/examples/api_peer_audio/peer1.rs`
```rust
use rvoip_session_core_v2::SimplePeer;  // Use new simple API

#[tokio::main]
async fn main() -> Result<()> {
    // Create Alice
    let alice = SimplePeer::new("alice", 5060).await?;
    println!("Alice listening on port 5060");
    
    // Wait a bit for Bob to start
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // Call Bob
    println!("Calling Bob...");
    let call = alice.call("sip:bob@127.0.0.1:5061").await?;
    
    // Wait for answer
    call.wait_for_answer().await?;
    println!("Call established!");
    
    // Exchange audio
    let wav_writer = WavWriter::new("output/peer1_audio.wav");
    for i in 0..1000 {
        // Send audio
        let frame = generate_tone(440.0, i); // A note
        call.send_audio(frame).await?;
        
        // Receive and save
        if let Some(frame) = call.try_recv_audio().await {
            wav_writer.write(frame);
        }
        
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    
    // Hangup
    call.hangup().await?;
    println!("Call ended");
    
    Ok(())
}
```

#### File: `crates/session-core-v2/examples/api_peer_audio/peer2.rs`
```rust
use rvoip_session_core_v2::SimplePeer;  // Use new simple API

#[tokio::main]
async fn main() -> Result<()> {
    // Create Bob
    let bob = SimplePeer::new("bob", 5061).await?;
    println!("Bob listening on port 5061");
    
    // Wait for incoming call
    println!("Waiting for incoming call...");
    let incoming = bob.wait_for_incoming().await?;
    println!("Incoming call from {}", incoming.from());
    
    // Accept the call
    let call = incoming.accept().await?;
    println!("Call accepted!");
    
    // Exchange audio
    let wav_writer = WavWriter::new("output/peer2_audio.wav");
    for i in 0..1000 {
        // Send audio
        let frame = generate_tone(880.0, i); // A5 note
        call.send_audio(frame).await?;
        
        // Receive and save
        if let Some(frame) = call.try_recv_audio().await {
            wav_writer.write(frame);
        }
        
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    
    // Call will be terminated by Alice
    println!("Call ended");
    
    Ok(())
}
```

### File: `src/adapters/session_event_handler.rs` (modifications)
```rust
impl SessionCrossCrateEventHandler {
    session_registry: Arc<SessionRegistry>,  // Add registry access
    
    async fn handle(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()> {
        // NEW: Use registry to check if session exists
        match event.event_type() {
            "dialog_to_session" => {
                // Extract dialog_id from event
                let event_str = format!("{:?}", event);
                if let Some(dialog_id) = self.extract_dialog_id(&event_str) {
                    // Check registry for existing session
                    if let Some(session_id) = self.session_registry.get_session_by_dialog(&dialog_id) {
                        // Process for existing session
                        self.state_machine.process_event(&session_id, event_type).await?;
                    } else if event_str.contains("IncomingCall") {
                        // New incoming call - let coordinator handle via interceptor
                        debug!("New incoming call for dialog {} - coordinator will handle", dialog_id);
                        return Ok(());
                    }
                }
            }
            "media_to_session" => {
                // Extract media_id and use registry
                if let Some(media_id) = self.extract_media_id(&event_str) {
                    if let Some(session_id) = self.session_registry.get_session_by_media(&media_id) {
                        self.state_machine.process_event(&session_id, event_type).await?;
                    }
                }
            }
            // ... rest unchanged
        }
    }
}
```

## Testing Strategy

### File: `crates/session-core-v2/tests/simple_api_test.rs` (NEW)
```rust
#[tokio::test]
async fn test_real_sip_call() {
    // Start both peers
    let alice = SimplePeer::new("alice", 5060).await.unwrap();
    let bob = SimplePeer::new("bob", 5061).await.unwrap();
    
    // Bob waits for call in background
    let bob_handle = tokio::spawn(async move {
        let incoming = bob.wait_for_incoming().await.unwrap();
        assert_eq!(incoming.from(), "sip:alice@127.0.0.1:5060");
        let call = incoming.accept().await.unwrap();
        
        // Exchange some audio
        for _ in 0..10 {
            call.send_audio(test_frame()).await.unwrap();
            call.recv_audio().await.unwrap();
        }
    });
    
    // Alice makes call
    let call = alice.call("sip:bob@127.0.0.1:5061").await.unwrap();
    call.wait_for_answer().await.unwrap();
    
    // Exchange audio
    for _ in 0..10 {
        call.send_audio(test_frame()).await.unwrap();
        call.recv_audio().await.unwrap();
    }
    
    // Clean up
    call.hangup().await.unwrap();
    bob_handle.await.unwrap();
}
```

## Success Metrics

1. ✅ **Incoming calls work**: Bob receives real SIP INVITE, not simulation
2. ✅ **Simple API**: < 10 lines for basic call
3. ✅ **Real audio exchange**: Both peers save valid .wav files
4. ✅ **State machine preserved**: All transitions still work
5. ✅ **Production ready**: Can build real applications

## Timeline

- **Day 1**: Signaling interception + Session factory
- **Day 2**: SimplePeer API implementation
- **Day 3**: Update examples + Testing
- **Total**: 3 days (compressed from 7)