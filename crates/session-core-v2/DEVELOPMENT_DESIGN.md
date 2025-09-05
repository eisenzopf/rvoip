# Session-Core-V2 Development Design

## Overview
This document details the specific code changes needed to implement the architecture fix for session-core-v2, based on the successful patterns from the sip-client library.

## File Organization

### New Files to Create

#### Phase 1: Core Architecture Implementation
- `src/session_registry.rs` - Central ID mapping registry (~200 lines)
- `src/adapters/signaling_interceptor.rs` - SIP signaling event interceptor (~300 lines)
- `src/api/session_manager.rs` - Session lifecycle management (~300 lines)
- `src/api/call_controller.rs` - Call control operations (~400 lines)
- `src/api/conference_manager.rs` - Conference bridge management (~200 lines)

#### Phase 2: External Service Integration
- `src/api/registry_service.rs` - Registration/presence integration (~300 lines)
- `src/adapters/auth_adapter.rs` - Auth-core integration (~150 lines)
- `src/adapters/registrar_adapter.rs` - Registrar-core integration (~200 lines)

#### Phase 3: Plugin System (Can be implemented later)
- `src/adapters/registry.rs` - Adapter plugin registry (~250 lines)
- `src/api/adapter_manager.rs` - Adapter lifecycle management (~250 lines)

### Files to Modify

#### Major Refactoring
- `src/api/unified.rs` - Refactor from 580 lines to ~200 lines (thin orchestration layer)
- `src/api/simple.rs` - Complete the SimplePeer, Call, IncomingCall implementation (~400 lines)

#### Minor Updates
- `src/api/types.rs` - Add new types (CallDirection, TransferStatus, MediaDirection, etc.)
- `src/adapters/dialog_adapter.rs` - Add interceptor hook
- `src/adapters/session_event_handler.rs` - Modify to skip new calls
- `src/adapters/media_adapter.rs` - Add conference mixing support
- `src/state_machine/actions.rs` - Add new actions for hold/transfer/DTMF
- `src/state_table/state_table.yaml` - Add new states and transitions
- `src/api/mod.rs` - Export new modules and types
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

### Phase 1: Core Architecture Implementation

Create the modular architecture directly - no need to build in unified.rs first and then refactor.

#### Step 1: Create Core Infrastructure
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

#### Step 2: Create SignalingInterceptor

##### File: `src/adapters/signaling_interceptor.rs` (NEW)
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

#### Step 3: Create Modular Architecture

Instead of building everything in unified.rs and then refactoring, create the proper modular structure from the start.

##### File Structure

```
src/api/
├── mod.rs                    # Module exports
├── unified.rs                # Thin orchestration layer (~200 lines)
├── session_manager.rs        # Session lifecycle management (~300 lines)
├── call_controller.rs        # Call control operations (~400 lines)
├── registry_service.rs       # Registration/presence (~300 lines)
├── conference_manager.rs     # Conference operations (~200 lines)
├── adapter_manager.rs        # Adapter loading/management (~250 lines)
└── simple.rs                 # SimplePeer API (unchanged)
```

#### File: `src/api/unified.rs` (refactored to be thin orchestration layer)
```rust
use crate::api::{
    SessionManager, CallController, RegistryService, 
    ConferenceManager, AdapterManager
};

/// Unified coordinator - thin facade over specialized managers
pub struct UnifiedCoordinator {
    config: Config,
    session_manager: Arc<SessionManager>,
    call_controller: Arc<CallController>,
    registry_service: Option<Arc<RegistryService>>, // Optional
    conference_manager: Arc<ConferenceManager>,
    adapter_manager: Option<Arc<AdapterManager>>,    // Optional for Phase 2
}

impl UnifiedCoordinator {
    pub async fn new(config: Config) -> Result<Self> {
        // Initialize core components
        let session_registry = Arc::new(SessionRegistry::new());
        let session_store = Arc::new(SessionStore::new());
        let state_machine = Arc::new(StateMachine::new(&config.state_table_path)?);
        
        // Create specialized managers
        let session_manager = Arc::new(SessionManager::new(
            session_store.clone(),
            session_registry.clone(),
            state_machine.clone(),
        ));
        
        let call_controller = Arc::new(CallController::new(
            config.clone(),
            session_registry.clone(),
        ).await?);
        
        let conference_manager = Arc::new(ConferenceManager::new(
            call_controller.media_adapter.clone(),
        ));
        
        Ok(Self {
            config,
            session_manager,
            call_controller,
            registry_service: None,
            conference_manager,
            adapter_manager: None,
        })
    }
    
    pub async fn new_with_services(
        config: Config,
        auth_service: Option<Arc<dyn AuthClient>>,
        registrar_service: Option<Arc<RegistrarService>>,
    ) -> Result<Self> {
        let mut coordinator = Self::new(config).await?;
        
        // Add optional registry service
        if let (Some(auth), Some(registrar)) = (auth_service, registrar_service) {
            coordinator.registry_service = Some(Arc::new(
                RegistryService::new(auth, registrar).await?
            ));
        }
        
        Ok(coordinator)
    }
    
    // === Delegation Methods ===
    
    // Session operations delegate to SessionManager
    pub async fn create_session(&self, role: Role) -> Result<SessionId> {
        self.session_manager.create_session(role).await
    }
    
    pub async fn get_session(&self, id: &SessionId) -> Result<SessionState> {
        self.session_manager.get_session(id).await
    }
    
    // Call operations delegate to CallController
    pub async fn make_call(&self, session_id: &SessionId, target: &str) -> Result<()> {
        self.call_controller.make_call(session_id, target).await
    }
    
    pub async fn handle_incoming_invite(&self, invite: InviteDetails) -> Result<SessionId> {
        self.call_controller.handle_incoming_invite(invite).await
    }
    
    // Registration delegates to RegistryService
    pub async fn register(&self, registrar: &str, credentials: Credentials) -> Result<()> {
        self.registry_service
            .as_ref()
            .ok_or(SessionError::ServiceNotAvailable("registry"))?
            .register(registrar, credentials)
            .await
    }
    
    // Conference delegates to ConferenceManager
    pub async fn create_conference(&self) -> Result<Conference> {
        self.conference_manager.create().await
    }
}
```

#### File: `src/api/session_manager.rs` (NEW - manages session lifecycle)
```rust
/// Manages session lifecycle and state
pub struct SessionManager {
    store: Arc<SessionStore>,
    registry: Arc<SessionRegistry>,
    state_machine: Arc<StateMachine>,
    sessions: Arc<DashMap<SessionId, Arc<Mutex<SessionState>>>>,
}

impl SessionManager {
    pub fn new(
        store: Arc<SessionStore>,
        registry: Arc<SessionRegistry>,
        state_machine: Arc<StateMachine>,
    ) -> Self {
        Self {
            store,
            registry,
            state_machine,
            sessions: Arc::new(DashMap::new()),
        }
    }
    
    pub async fn create_session(&self, role: Role) -> Result<SessionId> {
        let session_id = SessionId::new();
        
        // Create session state
        let session = SessionState {
            session_id: session_id.clone(),
            role,
            state: CallState::Idle,
            created_at: Instant::now(),
            ..Default::default()
        };
        
        // Store in multiple places
        self.store.create_session(session_id.clone(), role, false).await?;
        self.sessions.insert(session_id.clone(), Arc::new(Mutex::new(session)));
        
        Ok(session_id)
    }
    
    pub async fn get_session(&self, id: &SessionId) -> Result<SessionState> {
        self.store.get_session(id).await
    }
    
    pub async fn update_session_state(&self, id: &SessionId, state: CallState) -> Result<()> {
        if let Some(session) = self.sessions.get(id) {
            let mut session = session.lock().await;
            let old_state = session.state;
            session.state = state;
            
            // Trigger state change events
            self.notify_state_change(id, old_state, state).await?;
        }
        Ok(())
    }
    
    pub async fn terminate_session(&self, id: &SessionId) -> Result<()> {
        // Clean up from all stores
        self.sessions.remove(id);
        self.store.delete_session(id).await?;
        self.registry.remove_session(id);
        Ok(())
    }
    
    pub async fn process_event(&self, session_id: &SessionId, event: EventType) -> Result<()> {
        self.state_machine.process_event(session_id, event).await
    }
}
```

#### File: `src/api/call_controller.rs` (NEW - handles call control)
```rust
/// Handles call control operations
pub struct CallController {
    pub dialog_adapter: Arc<DialogAdapter>,
    pub media_adapter: Arc<MediaAdapter>,
    signaling_interceptor: Arc<SignalingInterceptor>,
    session_registry: Arc<SessionRegistry>,
    incoming_tx: mpsc::Sender<IncomingCall>,
    incoming_rx: Arc<Mutex<mpsc::Receiver<IncomingCall>>>,
}

impl CallController {
    pub async fn new(
        config: Config,
        session_registry: Arc<SessionRegistry>,
    ) -> Result<Self> {
        // Initialize adapters
        let dialog_adapter = Arc::new(DialogAdapter::new(config.clone()).await?);
        let media_adapter = Arc::new(MediaAdapter::new(config.clone()).await?);
        
        // Create incoming call channel
        let (incoming_tx, incoming_rx) = mpsc::channel(100);
        
        // Create signaling interceptor
        let signaling_interceptor = Arc::new(SignalingInterceptor::new(
            incoming_tx.clone(),
            session_registry.clone(),
        ));
        
        // Wire up interceptor
        dialog_adapter.set_interceptor(signaling_interceptor.clone());
        
        Ok(Self {
            dialog_adapter,
            media_adapter,
            signaling_interceptor,
            session_registry,
            incoming_tx,
            incoming_rx: Arc::new(Mutex::new(incoming_rx)),
        })
    }
    
    pub async fn make_call(&self, session_id: &SessionId, target: &str) -> Result<()> {
        // Send INVITE via dialog adapter
        let dialog_id = self.dialog_adapter.send_invite(session_id, target).await?;
        
        // Register mapping
        self.session_registry.map_dialog(session_id.clone(), dialog_id);
        
        Ok(())
    }
    
    pub async fn handle_incoming_invite(&self, invite: InviteDetails) -> Result<SessionId> {
        // This is called by SignalingInterceptor
        let session_id = SessionId::new();
        
        // Map dialog to session
        self.session_registry.map_dialog(session_id.clone(), invite.dialog_id);
        
        // Send to incoming channel
        self.incoming_tx.send(IncomingCall {
            session_id: session_id.clone(),
            from: invite.from,
            sdp: invite.sdp,
        }).await?;
        
        Ok(session_id)
    }
    
    pub async fn accept_call(&self, session_id: &SessionId) -> Result<()> {
        self.dialog_adapter.send_response(session_id, 200, "OK").await?;
        self.media_adapter.start_media(session_id).await?;
        Ok(())
    }
    
    pub async fn reject_call(&self, session_id: &SessionId, reason: &str) -> Result<()> {
        self.dialog_adapter.send_response(session_id, 486, reason).await
    }
    
    pub async fn hangup(&self, session_id: &SessionId) -> Result<()> {
        self.dialog_adapter.send_bye(session_id).await?;
        self.media_adapter.stop_media(session_id).await?;
        Ok(())
    }
    
    pub async fn hold(&self, session_id: &SessionId) -> Result<()> {
        // Send re-INVITE with sendonly
        let sdp = self.media_adapter.create_hold_sdp(session_id).await?;
        self.dialog_adapter.send_reinvite(session_id, sdp).await
    }
    
    pub async fn get_incoming_call(&self) -> Option<IncomingCall> {
        self.incoming_rx.lock().await.recv().await
    }
}
```

#### File: `src/api/registry_service.rs` (NEW - registration/presence)
```rust
/// Handles registration and presence
pub struct RegistryService {
    auth_adapter: Arc<AuthAdapter>,
    registrar_adapter: Arc<RegistrarAdapter>,
    presence_subscriptions: Arc<DashMap<SessionId, Vec<Subscription>>>,
}

impl RegistryService {
    pub async fn new(
        auth_client: Arc<dyn AuthClient>,
        registrar_service: Arc<RegistrarService>,
    ) -> Result<Self> {
        let auth_adapter = Arc::new(AuthAdapter::new(auth_client));
        let registrar_adapter = Arc::new(RegistrarAdapter::new(registrar_service));
        
        Ok(Self {
            auth_adapter,
            registrar_adapter,
            presence_subscriptions: Arc::new(DashMap::new()),
        })
    }
    
    pub async fn register(&self, registrar: &str, credentials: Credentials) -> Result<()> {
        // Validate credentials
        self.auth_adapter.validate_credentials(&credentials).await?;
        
        // Register with registrar
        self.registrar_adapter.register(
            &credentials.username,
            registrar,
            3600, // 1 hour
        ).await
    }
    
    pub async fn unregister(&self) -> Result<()> {
        self.registrar_adapter.unregister().await
    }
    
    pub async fn subscribe_presence(&self, session_id: &SessionId, target: &str) -> Result<()> {
        let subscription = self.registrar_adapter
            .subscribe_presence(target)
            .await?;
        
        self.presence_subscriptions
            .entry(session_id.clone())
            .or_default()
            .push(subscription);
        
        Ok(())
    }
    
    pub async fn publish_presence(&self, uri: &str, status: PresenceStatus) -> Result<()> {
        self.registrar_adapter.publish_presence(uri, status).await
    }
    
    pub async fn park_call(&self, session_id: &SessionId) -> Result<String> {
        let park_slot = format!("park-{}", uuid::Uuid::new_v4());
        
        self.registrar_adapter.register_park_slot(
            &park_slot,
            session_id,
            300, // 5 min timeout
        ).await?;
        
        Ok(park_slot)
    }
    
    pub async fn retrieve_parked_call(&self, park_slot: &str) -> Result<SessionId> {
        self.registrar_adapter.retrieve_park_slot(park_slot).await
    }
}
```

#### File: `src/api/conference_manager.rs` (NEW - conference operations)
```rust
/// Manages conference bridges
pub struct ConferenceManager {
    conferences: Arc<DashMap<ConferenceId, ConferenceState>>,
    media_adapter: Arc<MediaAdapter>,
}

impl ConferenceManager {
    pub fn new(media_adapter: Arc<MediaAdapter>) -> Self {
        Self {
            conferences: Arc::new(DashMap::new()),
            media_adapter,
        }
    }
    
    pub async fn create(&self) -> Result<Conference> {
        let conf_id = ConferenceId::new();
        
        // Create audio mixer in media adapter
        let mixer = self.media_adapter.create_audio_mixer(&conf_id).await?;
        
        // Store conference state
        let state = ConferenceState {
            id: conf_id.clone(),
            created_at: Instant::now(),
            participants: Vec::new(),
            mixer_id: mixer.id(),
        };
        
        self.conferences.insert(conf_id.clone(), state);
        
        Ok(Conference {
            id: conf_id,
            manager: Arc::new(self.clone()),
        })
    }
    
    pub async fn add_participant(&self, conf_id: &ConferenceId, session_id: SessionId) -> Result<()> {
        // Get conference
        let mut conf = self.conferences.get_mut(conf_id)
            .ok_or(SessionError::ConferenceNotFound)?;
        
        // Redirect media to mixer
        self.media_adapter.redirect_to_mixer(&session_id, conf_id).await?;
        
        // Add to participants
        conf.participants.push(session_id);
        
        Ok(())
    }
    
    pub async fn remove_participant(&self, conf_id: &ConferenceId, session_id: &SessionId) -> Result<()> {
        // Get conference
        let mut conf = self.conferences.get_mut(conf_id)
            .ok_or(SessionError::ConferenceNotFound)?;
        
        // Remove from mixer
        self.media_adapter.remove_from_mixer(session_id, conf_id).await?;
        
        // Remove from participants
        conf.participants.retain(|id| id != session_id);
        
        // If empty, destroy conference
        if conf.participants.is_empty() {
            drop(conf); // Release lock
            self.destroy(conf_id).await?;
        }
        
        Ok(())
    }
    
    async fn destroy(&self, conf_id: &ConferenceId) -> Result<()> {
        self.conferences.remove(conf_id);
        self.media_adapter.destroy_mixer(conf_id).await
    }
}
```

### Phase 2: External Service Integration

After Phase 1 is working, add auth-core and registrar-core integration.

### Phase 3: Plugin System (Optional)

Can be added later without disrupting the core architecture.

## Benefits of This Architecture

1. **Manageable File Sizes**: Each file stays under 400 lines
2. **Single Responsibility**: Each module has one clear purpose
3. **Parallel Development**: Teams can work on different modules
4. **Easier Testing**: Each component can be tested in isolation
5. **Clear Dependencies**: UnifiedCoordinator just orchestrates

The UnifiedCoordinator becomes a thin facade that delegates to specialized managers, preventing the file from becoming a monolithic mess.

## Complete the Examples

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

## Call Control Functions for SIP-Client Support

The SimplePeer API needs comprehensive call control functions to support the sip-client crate. These functions should be added to the Call and SimplePeer structs to provide a complete telephony API.

### File: `src/api/simple.rs` (additional methods for Call struct)
```rust
impl Call {
    // === Hold/Resume ===
    /// Put the call on hold
    pub async fn hold(&self) -> Result<()> {
        self.session.send_reinvite_with_direction(MediaDirection::SendOnly).await
    }
    
    /// Resume a held call
    pub async fn resume(&self) -> Result<()> {
        self.session.send_reinvite_with_direction(MediaDirection::SendRecv).await
    }
    
    /// Check if call is on hold
    pub fn is_on_hold(&self) -> bool {
        self.session.media_direction() == MediaDirection::SendOnly
    }
    
    // === Transfer ===
    /// Blind transfer to another party
    pub async fn blind_transfer(&self, target: &str) -> Result<()> {
        self.session.send_refer(target).await
    }
    
    /// Attended transfer after consultation
    pub async fn attended_transfer(&self, other_call: &Call) -> Result<()> {
        self.session.send_refer_with_replaces(other_call.session.dialog_id()).await
    }
    
    /// Get transfer status
    pub async fn transfer_status(&self) -> TransferStatus {
        self.session.transfer_status().await
    }
    
    // === DTMF ===
    /// Send DTMF digit (0-9, *, #, A-D)
    pub async fn send_dtmf(&self, digit: char) -> Result<()> {
        self.session.send_dtmf(digit).await
    }
    
    /// Set callback for received DTMF
    pub fn on_dtmf_received<F>(&self, callback: F) 
    where 
        F: Fn(char) + Send + Sync + 'static 
    {
        self.session.set_dtmf_callback(callback);
    }
    
    // === Call Information ===
    /// Get remote party URI
    pub fn remote_uri(&self) -> &str {
        &self.session.remote_uri()
    }
    
    /// Get local URI  
    pub fn local_uri(&self) -> &str {
        &self.session.local_uri()
    }
    
    /// Get SIP Call-ID header
    pub fn call_id(&self) -> &str {
        &self.session.call_id()
    }
    
    /// Get call duration
    pub fn duration(&self) -> Duration {
        self.session.duration()
    }
    
    /// Get call direction
    pub fn direction(&self) -> CallDirection {
        self.direction
    }
    
    // === Media Control ===
    /// Mute microphone
    pub async fn mute_audio(&self) -> Result<()> {
        self.session.set_audio_mute(true).await
    }
    
    /// Unmute microphone
    pub async fn unmute_audio(&self) -> Result<()> {
        self.session.set_audio_mute(false).await
    }
    
    /// Check if muted
    pub fn is_muted(&self) -> bool {
        self.session.is_audio_muted()
    }
    
    /// Change audio device mid-call
    pub async fn set_audio_device(&self, device: AudioDevice) -> Result<()> {
        self.session.set_audio_device(device).await
    }
    
    // === Advanced Call States ===
    /// Wait for call to be answered (for outgoing calls)
    pub async fn wait_for_answer(&self) -> Result<()> {
        self.session.wait_for_state(CallState::Active).await
    }
    
    /// Check if call is ringing
    pub fn is_ringing(&self) -> bool {
        self.session.state() == CallState::Ringing
    }
    
    /// Check if call is answered
    pub fn is_answered(&self) -> bool {
        self.session.state() == CallState::Active
    }
    
    /// Cancel outgoing call before answer
    pub async fn cancel(&self) -> Result<()> {
        if self.direction == CallDirection::Outgoing && !self.is_answered() {
            self.session.send_cancel().await
        } else {
            Err(SessionError::InvalidState("Can only cancel unanswered outgoing calls"))
        }
    }
    
    // === Recording ===
    /// Start recording the call
    pub async fn start_recording(&self, path: &Path) -> Result<()> {
        self.session.start_recording(path).await
    }
    
    /// Stop recording
    pub async fn stop_recording(&self) -> Result<()> {
        self.session.stop_recording().await
    }
    
    /// Check if recording
    pub fn is_recording(&self) -> bool {
        self.session.is_recording()
    }
    
    // === Early Media ===
    /// Send early media (before answer)
    pub async fn send_early_media(&self, frame: AudioFrame) -> Result<()> {
        self.session.send_early_media(frame).await
    }
    
    /// Receive early media stream
    pub async fn recv_early_media(&self) -> Result<AudioStream> {
        let subscriber = self.session.subscribe_to_early_media().await?;
        Ok(AudioStream { subscriber })
    }
    
    // === Re-negotiation ===
    /// Renegotiate media parameters
    pub async fn renegotiate_media(&self, new_codecs: Vec<CodecInfo>) -> Result<()> {
        self.session.send_reinvite_with_codecs(new_codecs).await
    }
}
```

### File: `src/api/simple.rs` (additional methods for SimplePeer struct)
```rust
impl SimplePeer {
    // === Registration ===
    /// Register with SIP server
    pub async fn register(&self, registrar: &str, credentials: Credentials) -> Result<()> {
        self.coordinator.register(registrar, credentials).await
    }
    
    /// Unregister from SIP server
    pub async fn unregister(&self) -> Result<()> {
        self.coordinator.unregister().await
    }
    
    /// Check registration status
    pub fn is_registered(&self) -> bool {
        self.coordinator.is_registered()
    }
    
    // === Multiple Call Management ===
    /// Get list of active calls
    pub async fn active_calls(&self) -> Vec<Arc<Call>> {
        self.active_sessions.lock().await
            .iter()
            .filter_map(|session| {
                if session.is_active() {
                    Some(Arc::new(Call {
                        session: session.clone(),
                        direction: session.direction(),
                    }))
                } else {
                    None
                }
            })
            .collect()
    }
    
    /// Find call by Call-ID
    pub async fn find_call(&self, call_id: &str) -> Option<Arc<Call>> {
        self.active_sessions.lock().await
            .iter()
            .find(|s| s.call_id() == call_id)
            .map(|session| Arc::new(Call {
                session: session.clone(),
                direction: session.direction(),
            }))
    }
    
    /// Hangup all active calls
    pub async fn hangup_all(&self) -> Result<()> {
        let sessions = self.active_sessions.lock().await.clone();
        for session in sessions {
            session.hangup().await?;
        }
        Ok(())
    }
    
    // === Conference Support ===
    /// Create a local conference bridge
    pub async fn create_conference(&self) -> Result<Conference> {
        self.coordinator.create_conference().await
    }
    
    // === Event Callbacks ===
    /// Set callback for call state changes
    pub fn on_call_state_changed<F>(&self, callback: F)
    where
        F: Fn(CallId, CallState) + Send + Sync + 'static
    {
        self.coordinator.set_call_state_callback(callback);
    }
    
    /// Set callback for registration changes
    pub fn on_registration_changed<F>(&self, callback: F)
    where
        F: Fn(RegistrationState) + Send + Sync + 'static
    {
        self.coordinator.set_registration_callback(callback);
    }
}

/// Conference bridge for multi-party calls
pub struct Conference {
    id: ConferenceId,
    coordinator: Arc<UnifiedCoordinator>,
    participants: Arc<Mutex<Vec<Arc<Call>>>>,
}

impl Conference {
    /// Add a call to the conference
    pub async fn add_call(&self, call: Arc<Call>) -> Result<()> {
        self.coordinator.add_to_conference(self.id, call.session.clone()).await?;
        self.participants.lock().await.push(call);
        Ok(())
    }
    
    /// Remove a call from conference
    pub async fn remove_call(&self, call: &Call) -> Result<()> {
        self.coordinator.remove_from_conference(self.id, &call.session).await?;
        let mut participants = self.participants.lock().await;
        participants.retain(|c| c.call_id() != call.call_id());
        Ok(())
    }
    
    /// Get participant count
    pub async fn participant_count(&self) -> usize {
        self.participants.lock().await.len()
    }
}
```

### File: `src/api/types.rs` (additional types)
```rust
/// Transfer status
#[derive(Debug, Clone, PartialEq)]
pub enum TransferStatus {
    NotStarted,
    InProgress,
    Completed,
    Failed(String),
}

/// Media direction for hold/resume
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MediaDirection {
    SendRecv,   // Normal bidirectional
    SendOnly,   // On hold (we send hold music)
    RecvOnly,   // Reverse hold
    Inactive,   // No media
}

/// Registration state
#[derive(Debug, Clone, PartialEq)]
pub enum RegistrationState {
    NotRegistered,
    Registering,
    Registered,
    Failed(String),
}

/// SIP credentials
pub struct Credentials {
    pub username: String,
    pub password: String,
    pub realm: Option<String>,
}

/// Audio device info
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub device_type: AudioDeviceType,
}

/// Conference ID
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConferenceId(String);

/// Make CallDirection public
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CallDirection {
    Incoming,
    Outgoing,
}
```

### Implementation Architecture for Call Control Functions

The call control functions are distributed across multiple layers of the architecture. SimplePeer provides the clean API, but the actual implementation is spread across UnifiedSession, UnifiedCoordinator, state machine actions, and the adapters.

#### Layer 1: SimplePeer (API Surface Only)
```rust
// src/api/simple.rs - Just delegates to underlying layers
impl Call {
    pub async fn hold(&self) -> Result<()> {
        self.session.hold().await  // Delegates to UnifiedSession
    }
}
```

#### Layer 2: UnifiedSession (Session-Level Operations)
```rust
// src/api/unified.rs - Manages session-specific operations
impl UnifiedSession {
    // Hold/Resume - sends events to state machine
    pub async fn hold(&self) -> Result<()> {
        self.send_event(EventType::HoldCall).await
    }
    
    pub async fn resume(&self) -> Result<()> {
        self.send_event(EventType::ResumeCall).await
    }
    
    // DTMF - direct media adapter interaction
    pub async fn send_dtmf(&self, digit: char) -> Result<()> {
        self.coordinator.media_adapter.send_dtmf(&self.id, digit).await
    }
    
    // Transfer - initiates REFER through state machine
    pub async fn send_refer(&self, target: &str) -> Result<()> {
        self.send_event(EventType::TransferCall { target: target.to_string() }).await
    }
    
    // Recording - media adapter control
    pub async fn start_recording(&self, path: &Path) -> Result<()> {
        self.coordinator.media_adapter.start_recording(&self.id, path).await
    }
    
    // Information getters - query session store
    pub fn remote_uri(&self) -> String {
        let session = self.coordinator.store.get_session(&self.id).await?;
        session.remote_uri.unwrap_or_default()
    }
    
    pub fn call_id(&self) -> String {
        let session = self.coordinator.store.get_session(&self.id).await?;
        session.dialog_id.as_ref().map(|d| d.call_id()).unwrap_or_default()
    }
}
```

#### Layer 3: UnifiedCoordinator (Multi-Session Operations)
```rust
// src/api/unified.rs - Manages cross-session and system-wide operations
impl UnifiedCoordinator {
    // Registration - system-wide, not session specific
    pub async fn register(&self, registrar: &str, credentials: Credentials) -> Result<()> {
        self.dialog_adapter.send_register(registrar, credentials).await
    }
    
    // Conference - manages multiple sessions
    pub async fn create_conference(&self) -> Result<Conference> {
        let conf_id = ConferenceId::new();
        let mixer = self.media_adapter.create_audio_mixer(conf_id.clone()).await?;
        Ok(Conference {
            id: conf_id,
            coordinator: Arc::new(self),
            mixer,
            participants: Arc::new(Mutex::new(Vec::new())),
        })
    }
    
    pub async fn add_to_conference(&self, conf_id: ConferenceId, session: Arc<UnifiedSession>) -> Result<()> {
        // Redirect session's audio to conference mixer
        self.media_adapter.redirect_to_mixer(&session.id, &conf_id).await
    }
}
```

#### Layer 4: State Machine Actions (Protocol Coordination)
```rust
// src/state_machine/actions.rs - New actions for call control
pub enum Action {
    // Existing actions...
    
    // Hold/Resume actions
    SendReInviteWithDirection { direction: MediaDirection },
    UpdateMediaDirection { direction: MediaDirection },
    
    // Transfer actions
    SendRefer { target: String },
    SendReferWithReplaces { target: String, replaces: String },
    MonitorTransferProgress,
    
    // DTMF actions
    SendDtmfInfo { digit: char },
    SendDtmfRfc2833 { digit: char },
    
    // Registration actions
    SendRegister { registrar: String, credentials: Credentials },
    SendUnregister,
}

// State table additions for new transitions
// src/state_table/state_table.yaml
states:
  Active:
    transitions:
      - event: HoldCall
        next_state: OnHold
        actions:
          - SendReInviteWithDirection: { direction: SendOnly }
          - UpdateMediaDirection: { direction: SendOnly }
      
      - event: TransferCall
        next_state: Transferring
        actions:
          - SendRefer: { target: "${event.target}" }
          - MonitorTransferProgress
  
  OnHold:
    transitions:
      - event: ResumeCall
        next_state: Active
        actions:
          - SendReInviteWithDirection: { direction: SendRecv }
          - UpdateMediaDirection: { direction: SendRecv }
```

#### Layer 5: Adapter Implementations (Protocol/Media Details)
```rust
// src/adapters/dialog_adapter.rs - SIP protocol implementation
impl DialogAdapter {
    pub async fn send_reinvite_with_sdp(&self, session_id: &SessionId, sdp: String) -> Result<()> {
        // Get dialog from session
        let dialog_id = self.get_dialog_id(session_id)?;
        
        // Create re-INVITE with new SDP
        self.dialog_core.send_reinvite(dialog_id, sdp).await
    }
    
    pub async fn send_refer(&self, session_id: &SessionId, target: &str) -> Result<()> {
        let dialog_id = self.get_dialog_id(session_id)?;
        self.dialog_core.send_refer(dialog_id, target).await
    }
    
    pub async fn send_register(&self, registrar: &str, credentials: Credentials) -> Result<()> {
        // Registration is not session-specific
        self.dialog_core.register(registrar, credentials).await
    }
}

// src/adapters/media_adapter.rs - Media/RTP implementation
impl MediaAdapter {
    pub async fn send_dtmf(&self, session_id: &SessionId, digit: char) -> Result<()> {
        let media_session = self.get_media_session(session_id)?;
        media_session.send_dtmf_rfc2833(digit).await
    }
    
    pub async fn start_recording(&self, session_id: &SessionId, path: &Path) -> Result<()> {
        let media_session = self.get_media_session(session_id)?;
        media_session.start_recording(path).await
    }
    
    pub async fn create_audio_mixer(&self, conf_id: ConferenceId) -> Result<AudioMixer> {
        // Create N-way audio mixer for conference
        AudioMixer::new(conf_id)
    }
    
    pub async fn redirect_to_mixer(&self, session_id: &SessionId, conf_id: &ConferenceId) -> Result<()> {
        let media_session = self.get_media_session(session_id)?;
        let mixer = self.get_mixer(conf_id)?;
        
        // Redirect audio streams to/from mixer
        media_session.set_audio_sink(mixer.get_input_for(session_id));
        media_session.set_audio_source(mixer.get_output_for(session_id));
    }
}
```

### Data Flow Example: Hold Operation

```
User calls: call.hold()
    ↓
SimplePeer Call::hold()
    ↓
UnifiedSession::hold()
    ↓ (sends EventType::HoldCall)
StateMachine processes event
    ↓ (executes actions from state table)
Action: SendReInviteWithDirection(SendOnly)
    ↓
DialogAdapter::send_reinvite_with_sdp()
    ↓ (constructs SDP with a=sendonly)
dialog-core sends re-INVITE
    ↓
Action: UpdateMediaDirection(SendOnly)  
    ↓
MediaAdapter::set_direction(SendOnly)
    ↓
media-core stops receiving audio
```

### Implementation Priority for sip-client

**Phase 1 (Must Have):**
- Hold/Resume (state machine + dialog adapter)
- DTMF send/receive (media adapter)
- Mute/Unmute (media adapter only, no signaling)
- Call information getters (session store queries)
- Registration support (dialog adapter)

**Phase 2 (Should Have):**
- Blind transfer (state machine + dialog adapter)
- Call recording (media adapter)
- Multiple call management (coordinator level)
- Event callbacks (coordinator event bus)

**Phase 3 (Nice to Have):**
- Attended transfer (complex state machine)
- Conference support (media mixer + coordinator)
- Early media (state machine + media adapter)
- Media renegotiation (dialog + media coordination)

These additions ensure session-core-v2 provides all the call control functions needed by sip-client for a complete telephony solution, with clear separation of concerns across the architecture layers.

## Integration with Auth-Core and Registrar-Core

Session-core-v2 must integrate with existing auth-core and registrar-core libraries to provide complete SIP functionality including registration, presence, and authentication.

### Auth-Core Integration

#### File: `src/adapters/auth_adapter.rs` (NEW)
```rust
use rvoip_auth_core::{AuthError, types::{Token, Credentials as AuthCredentials}};
use rvoip_sip_core::auth::{DigestAuth, Challenge};

pub struct AuthAdapter {
    auth_client: Arc<dyn AuthClient>,
}

impl AuthAdapter {
    /// Validate incoming request authentication
    pub async fn validate_request(&self, auth_header: &str) -> Result<Token> {
        self.auth_client.validate_sip_auth(auth_header).await
    }
    
    /// Generate response to 401/407 challenge
    pub async fn respond_to_challenge(&self, challenge: Challenge, credentials: &Credentials) -> Result<String> {
        let auth_creds = AuthCredentials {
            username: credentials.username.clone(),
            password: credentials.password.clone(),
            realm: challenge.realm.clone(),
        };
        
        DigestAuth::compute_response(&challenge, &auth_creds)
    }
    
    /// Add authentication to outgoing request
    pub async fn add_auth_header(&self, request: &mut SipRequest, credentials: &Credentials) -> Result<()> {
        if let Some(auth_header) = self.generate_auth_header(credentials).await? {
            request.add_header("Authorization", auth_header);
        }
        Ok(())
    }
}
```

### Registrar-Core Integration

#### File: `src/adapters/registrar_adapter.rs` (NEW)
```rust
use rvoip_registrar_core::{
    RegistrarService, UserRegistration, ContactInfo,
    PresenceState, PresenceStatus, Subscription,
    events::{RegistrarEvent, PresenceEvent},
};

pub struct RegistrarAdapter {
    registrar_service: Arc<RegistrarService>,
    presence_subscriptions: Arc<DashMap<SessionId, Vec<Subscription>>>,
}

impl RegistrarAdapter {
    /// Handle REGISTER request
    pub async fn register(&self, uri: &str, contact: &str, expires: u32) -> Result<()> {
        let registration = UserRegistration {
            aor: uri.to_string(),
            contact: ContactInfo::from_str(contact)?,
            expires,
            q_value: 1.0,
        };
        
        self.registrar_service.register(registration).await
    }
    
    /// Handle SUBSCRIBE for presence
    pub async fn subscribe_presence(&self, session_id: &SessionId, target: &str) -> Result<()> {
        let subscription = self.registrar_service
            .subscribe_presence(target)
            .await?;
        
        self.presence_subscriptions
            .entry(session_id.clone())
            .or_default()
            .push(subscription);
        
        Ok(())
    }
    
    /// Handle PUBLISH for presence
    pub async fn publish_presence(&self, uri: &str, status: PresenceStatus) -> Result<()> {
        let state = PresenceState {
            basic: status.into(),
            note: None,
            activities: vec![],
        };
        
        self.registrar_service.publish_presence(uri, state).await
    }
    
    /// Handle presence NOTIFY events
    pub async fn on_presence_update(&self, event: PresenceEvent) -> Result<()> {
        // Route to appropriate session
        match event {
            PresenceEvent::StatusChanged { uri, status } => {
                // Find sessions subscribed to this URI
                for entry in self.presence_subscriptions.iter() {
                    let session_id = entry.key();
                    let subscriptions = entry.value();
                    
                    if subscriptions.iter().any(|s| s.target == uri) {
                        self.notify_session_presence(session_id, &uri, &status).await?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}
```

### Extended SimplePeer API for Registration/Presence

#### File: `src/api/simple.rs` (additional methods)
```rust
impl SimplePeer {
    // === Registration with Registrar ===
    /// Register with SIP registrar (with authentication)
    pub async fn register(&self, registrar: &str, credentials: Credentials) -> Result<()> {
        // Authenticate first
        self.coordinator.auth_adapter.validate_credentials(&credentials).await?;
        
        // Then register
        self.coordinator.registrar_adapter.register(
            &self.uri(),
            &self.contact_address(),
            3600, // 1 hour default
        ).await?;
        
        // Store registration state
        self.coordinator.set_registration_state(RegistrationState::Registered).await;
        Ok(())
    }
    
    // === Presence ===
    /// Subscribe to presence updates for a contact
    pub async fn subscribe_presence(&self, contact: &str) -> Result<()> {
        self.coordinator.registrar_adapter.subscribe_presence(
            &self.current_session_id(),
            contact
        ).await
    }
    
    /// Publish presence status
    pub async fn set_presence(&self, status: PresenceStatus) -> Result<()> {
        self.coordinator.registrar_adapter.publish_presence(
            &self.uri(),
            status
        ).await
    }
    
    /// Set callback for presence updates
    pub fn on_presence_update<F>(&self, callback: F)
    where
        F: Fn(&str, PresenceStatus) + Send + Sync + 'static
    {
        self.coordinator.set_presence_callback(callback);
    }
    
    // === Call Parking (via Registrar) ===
    /// Park a call
    pub async fn park_call(&self, call: &Call) -> Result<String> {
        // Register call with special parking AOR
        let park_slot = format!("park-{}", uuid::Uuid::new_v4());
        self.coordinator.registrar_adapter.register(
            &park_slot,
            &call.session.dialog_id()?,
            300, // 5 min timeout
        ).await?;
        
        // Put call on hold
        call.hold().await?;
        
        Ok(park_slot)
    }
    
    /// Retrieve parked call
    pub async fn retrieve_parked_call(&self, park_slot: &str) -> Result<Call> {
        // Lookup parked call from registrar
        let contact = self.coordinator.registrar_adapter
            .lookup(park_slot)
            .await?;
        
        // Create new session to retrieve
        let session = UnifiedSession::new(self.coordinator.clone(), Role::UAC).await?;
        session.retrieve_parked(contact).await?;
        
        Ok(Call {
            session: Arc::new(session),
            direction: CallDirection::Outgoing,
        })
    }
}
```

### State Machine Integration for Auth/Registration

#### File: `src/state_machine/actions.rs` (additional actions)
```rust
pub enum Action {
    // Existing actions...
    
    // Authentication actions
    RespondToAuthChallenge { credentials: Credentials },
    ValidateIncomingAuth,
    
    // Registration actions  
    SendRegisterWithAuth { registrar: String, credentials: Credentials },
    HandleRegisterResponse { status: u16 },
    RefreshRegistration,
    
    // Presence actions
    SendPresenceSubscribe { target: String },
    SendPresenceNotify { uri: String, status: PresenceStatus },
    PublishPresence { status: PresenceStatus },
}
```

### State Table Updates for Registration/Presence

```yaml
# src/state_table/state_table.yaml additions
states:
  Idle:
    transitions:
      - event: Register
        next_state: Registering
        actions:
          - SendRegisterWithAuth: { registrar: "${event.registrar}", credentials: "${event.credentials}" }
  
  Registering:
    transitions:
      - event: AuthChallenge
        next_state: Authenticating
        actions:
          - RespondToAuthChallenge: { credentials: "${session.credentials}" }
      
      - event: RegisterSuccess
        next_state: Registered
        actions:
          - HandleRegisterResponse: { status: 200 }
          - PublishPresence: { status: Available }
  
  Registered:
    transitions:
      - event: SubscribePresence
        next_state: Registered
        actions:
          - SendPresenceSubscribe: { target: "${event.target}" }
      
      - event: PresenceNotify
        next_state: Registered
        actions:
          - SendPresenceNotify: { uri: "${event.uri}", status: "${event.status}" }
```

### UnifiedCoordinator Updates

#### File: `src/api/unified.rs` (modifications)
```rust
impl UnifiedCoordinator {
    pub auth_adapter: Arc<AuthAdapter>,
    pub registrar_adapter: Arc<RegistrarAdapter>,
    
    pub async fn new_with_services(
        config: Config,
        auth_service: Arc<dyn AuthClient>,
        registrar_service: Arc<RegistrarService>,
    ) -> Result<Self> {
        // ... existing initialization ...
        
        let auth_adapter = Arc::new(AuthAdapter::new(auth_service));
        let registrar_adapter = Arc::new(RegistrarAdapter::new(registrar_service));
        
        // Setup event handlers for presence
        registrar_service.on_presence_event({
            let adapter = registrar_adapter.clone();
            move |event| {
                let adapter = adapter.clone();
                tokio::spawn(async move {
                    adapter.on_presence_update(event).await;
                });
            }
        });
        
        Ok(Self {
            // ... existing fields ...
            auth_adapter,
            registrar_adapter,
        })
    }
}
```

### Complete Feature Support

With auth-core and registrar-core integration, session-core-v2 now supports:

**Authentication:**
- Digest authentication for REGISTER/INVITE
- Token-based authentication
- Challenge/response handling

**Registration:**
- REGISTER with authentication
- Contact binding management
- Registration refresh

**Presence:**
- SUBSCRIBE/NOTIFY for presence
- PUBLISH presence state
- Buddy list management

**Call Parking:**
- Park calls using registrar
- Retrieve parked calls
- Parking timeout management

**MESSAGE Support:**
- Send/receive SIP MESSAGE
- Instant messaging between registered users

These additions ensure session-core-v2 provides all the call control functions needed by sip-client for a complete telephony solution, with clear separation of concerns across the architecture layers.

## Adapter Plugin System (Phase 2 - Can Be Added Later)

The adapter system provides extensibility without modifying core code. This can be implemented after Phase 1 is complete.

### File: `src/adapters/registry.rs` (NEW - Phase 2)
```rust
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use async_trait::async_trait;
use libloading::Library;

/// Base trait for all session-core adapters
#[async_trait]
pub trait SessionAdapter: Send + Sync {
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    async fn initialize(&mut self) -> Result<()>;
}

/// Adapter for call lifecycle events
#[async_trait]
pub trait CallEventAdapter: SessionAdapter {
    async fn on_call_state_change(&self,
        session_id: &SessionId,
        old_state: CallState,
        new_state: CallState
    ) -> Result<()>;
    
    async fn on_dtmf(&self, session_id: &SessionId, digit: char) -> Result<()>;
    
    async fn on_call_end(&self, session_id: &SessionId, cdr: CallDetail) -> Result<()>;
}

/// Adapter for custom state machine actions
#[async_trait]
pub trait StateActionAdapter: SessionAdapter {
    fn can_handle(&self, action: &str) -> bool;
    
    async fn execute(&self,
        action: &str,
        session: &SessionState,
        params: serde_json::Value
    ) -> Result<()>;
}

/// Registry that loads and manages adapters
pub struct AdapterRegistry {
    call_event_adapters: Vec<Arc<dyn CallEventAdapter>>,
    state_action_adapters: Vec<Arc<dyn StateActionAdapter>>,
    loaded_libraries: Vec<Library>,
}

impl AdapterRegistry {
    pub async fn load_from_directory(&mut self, dir: &Path) -> Result<()> {
        if !dir.exists() {
            return Ok(());
        }
        
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            
            match path.extension().and_then(|s| s.to_str()) {
                Some("so") | Some("dylib") | Some("dll") => {
                    self.load_native_adapter(&path).await?;
                }
                Some("toml") => {
                    self.load_from_manifest(&path).await?;
                }
                _ => {}
            }
        }
        Ok(())
    }
    
    unsafe fn load_native_adapter(&mut self, path: &Path) -> Result<()> {
        let lib = Library::new(path)?;
        
        // Try to load as CallEventAdapter
        if let Ok(create_fn) = lib.get::<fn() -> Box<dyn CallEventAdapter>>(b"create_call_event_adapter") {
            let adapter = Arc::from(create_fn());
            self.call_event_adapters.push(adapter);
        }
        
        // Try to load as StateActionAdapter
        if let Ok(create_fn) = lib.get::<fn() -> Box<dyn StateActionAdapter>>(b"create_state_action_adapter") {
            let adapter = Arc::from(create_fn());
            self.state_action_adapters.push(adapter);
        }
        
        self.loaded_libraries.push(lib);
        Ok(())
    }
}
```

### Example Adapter Implementation

#### File: `adapters/billing/src/lib.rs` (External crate)
```rust
use rvoip_session_core::adapters::{CallEventAdapter, SessionAdapter};
use async_trait::async_trait;

pub struct BillingAdapter {
    api_endpoint: String,
    api_key: String,
}

#[async_trait]
impl SessionAdapter for BillingAdapter {
    fn name(&self) -> &str { "billing" }
    fn version(&self) -> &str { "1.0.0" }
    
    async fn initialize(&mut self) -> Result<()> {
        // Connect to billing service
        Ok(())
    }
}

#[async_trait]
impl CallEventAdapter for BillingAdapter {
    async fn on_call_state_change(&self,
        session_id: &SessionId,
        old_state: CallState,
        new_state: CallState
    ) -> Result<()> {
        match new_state {
            CallState::Active => {
                // Start billing meter
                self.start_billing(session_id).await?;
            }
            CallState::Terminated => {
                // Stop billing and generate invoice
                self.stop_billing(session_id).await?;
            }
            _ => {}
        }
        Ok(())
    }
    
    async fn on_call_end(&self, session_id: &SessionId, cdr: CallDetail) -> Result<()> {
        // Send CDR to billing system
        self.send_cdr(cdr).await
    }
}

#[no_mangle]
pub extern "C" fn create_call_event_adapter() -> Box<dyn CallEventAdapter> {
    Box::new(BillingAdapter::default())
}
```

### Integration with UnifiedCoordinator

#### File: `src/api/unified.rs` (modifications for Phase 2)
```rust
impl UnifiedCoordinator {
    pub async fn new_with_adapters(config: Config) -> Result<Self> {
        let mut coordinator = Self::new(config).await?;
        
        // Load adapters if enabled
        if config.enable_adapters {
            let mut registry = AdapterRegistry::new();
            
            // Load from standard directories
            for dir in &[
                PathBuf::from("./adapters"),
                PathBuf::from("/usr/local/lib/rvoip/adapters"),
                dirs::config_dir().map(|d| d.join("rvoip/adapters")),
            ] {
                if let Some(dir) = dir {
                    registry.load_from_directory(&dir).await?;
                }
            }
            
            coordinator.adapter_registry = Some(Arc::new(registry));
        }
        
        Ok(coordinator)
    }
    
    /// Notify all call event adapters
    async fn notify_call_adapters(&self, event: CallEvent) -> Result<()> {
        if let Some(registry) = &self.adapter_registry {
            for adapter in registry.get_call_event_adapters() {
                // Run adapter in background, don't block call
                let adapter = adapter.clone();
                let event = event.clone();
                tokio::spawn(async move {
                    if let Err(e) = adapter.handle_event(event).await {
                        warn!("Adapter {} failed: {}", adapter.name(), e);
                    }
                });
            }
        }
        Ok(())
    }
}
```

### State Machine Integration for Custom Actions

#### File: `src/state_machine/executor.rs` (modifications for Phase 2)
```rust
impl StateMachineExecutor {
    async fn execute_action(&self, action: &Action, session: &mut SessionState) -> Result<()> {
        // Try built-in actions first
        if let Some(result) = self.execute_builtin_action(action, session).await? {
            return result;
        }
        
        // Try adapter actions
        if let Some(registry) = &self.adapter_registry {
            for adapter in registry.get_state_action_adapters() {
                if adapter.can_handle(&action.name) {
                    return adapter.execute(&action.name, session, action.params.clone()).await;
                }
            }
        }
        
        Err(SessionError::UnknownAction(action.name.clone()))
    }
}
```

### Configuration for Adapters

```toml
# ~/.config/rvoip/config.toml
[adapters]
enabled = true
directories = [
    "./adapters",
    "/usr/local/lib/rvoip/adapters"
]

# Adapter-specific config
[adapters.billing]
api_endpoint = "https://billing.example.com"
api_key = "secret-key"

[adapters.transcription]
provider = "whisper"
language = "en-US"
```

### Why This Can Be Added Later

1. **Core Functionality First**: The adapter system is not required for basic operation
2. **Clean Interfaces**: Adapter traits can be added without breaking existing code
3. **Optional Loading**: Adapters are loaded conditionally based on configuration
4. **Background Processing**: Adapters run async without blocking calls

This design ensures:
- Session-core stays focused on call control
- Media processing stays in media-core
- Third-party extensions don't compromise stability
- Can be implemented incrementally after Phase 1

These additions ensure session-core-v2 provides all the call control functions needed by sip-client for a complete telephony solution, with clear separation of concerns across the architecture layers.

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