# Session-Core-V2 Architecture Fix

## Executive Summary

The session-core-v2 library has a good state machine design but inverted architecture that makes incoming calls impossible and the API complex. This document outlines the architectural changes needed to fix these issues while preserving the state machine benefits.

## The Fundamental Problem

The state machine was designed to manage state transitions for sessions, but we made it the entry point for ALL events. This creates an impossible situation:

1. State machine receives event (e.g., IncomingCall)
2. State machine looks up session to process event
3. Session doesn't exist because the call just arrived
4. Event is dropped, call fails

This is why the examples resort to SIMULATING incoming calls - there's literally no way for real incoming calls to work.

## Current Architecture (BROKEN)

```
┌─────────────────────────────────────────────────────────┐
│                   Current Flow (BROKEN)                  │
└─────────────────────────────────────────────────────────┘

Network Layer:
┌──────────────┐
│ SIP Transport│──────► Incoming INVITE arrives
└──────────────┘
        │
        ▼
┌──────────────┐
│DialogAdapter │──────► Creates DialogToSessionEvent
└──────────────┘
        │
        ▼
┌──────────────┐
│Event Router  │──────► Routes to SessionEventHandler  
└──────────────┘
        │
        ▼
┌──────────────┐
│State Machine │──────► Looks for session...
└──────────────┘
        │
        ▼
    ❌ FAILS - No session exists for new call!
```

### Why This Architecture Fails

1. **No Session Factory**: Nothing creates sessions when calls arrive
2. **Wrong Entry Point**: State machine is first receiver but can't create sessions
3. **Circular Dependency**: Need session to process event, need event to create session
4. **No User Notification**: Even if we could create session, no way to tell user

## Current Code That Shows The Problem

In `src/adapters/session_event_handler.rs`:
```rust
// This handler receives events but REQUIRES session to exist
async fn handle(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()> {
    // Extract session_id from event
    if let Some(session_id) = self.extract_session_id(&event_str) {
        // Process through state machine
        self.state_machine.process_event(&SessionId(session_id), event_type).await
        //                                ^^^^^^^^^^^^^^^^^^^^^^
        //                                MUST ALREADY EXIST!
    }
}
```

In `src/state_machine/executor.rs`:
```rust
pub async fn process_event(&self, session_id: &SessionId, event: EventType) {
    // Get existing session from store
    let session = self.store.get_session(session_id).await?;
    //            ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    //            FAILS if session doesn't exist!
    
    // Process transitions...
}
```

## Proposed Architecture (FIXED)

```
┌─────────────────────────────────────────────────────────┐
│                  Fixed Architecture                       │
└─────────────────────────────────────────────────────────┘

         User Code
         ┌────────┐
         │SimplePeer│◄──── "alice.call('bob')" 
         └────────┘       "alice.get_incoming_call()"
              │
              ▼
    ┌──────────────────┐
    │UnifiedCoordinator│◄──── Session Manager
    └──────────────────┘      Creates/destroys sessions
              │
              ▼
    ┌──────────────────┐
    │  State Machine   │◄──── State Transitions
    └──────────────────┘      Only for EXISTING sessions
              │
              ▼
    ┌──────────────────┐
    │    Adapters      │◄──── Protocol Implementation
    └──────────────────┘      Dialog, Media, etc.
```

### Incoming Call Flow (FIXED)

```
┌─────────────────────────────────────────────────────────┐
│              Incoming Call Flow (WORKING)                │
└─────────────────────────────────────────────────────────┘

1. INVITE Arrives:
┌──────────────┐
│ SIP Transport│──────► "INVITE sip:bob@127.0.0.1"
└──────────────┘
        │
        ▼
2. Coordinator Intercepts:
┌──────────────────┐
│UnifiedCoordinator│──────► "Is this for existing session?"
└──────────────────┘        "No - it's a new call!"
        │
        ▼
3. Create UAS Session:
┌──────────────────┐
│ Session Factory  │──────► Creates SessionId: "abc123"
└──────────────────┘        Role: UAS
        │                   Stores in SessionStore
        ▼
4. Notify User:
┌──────────────────┐
│ Incoming Channel │──────► SimplePeer receives IncomingCall
└──────────────────┘        User can accept/reject
        │
        ▼
5. Route to State Machine:
┌──────────────────┐
│  State Machine   │──────► NOW session exists!
└──────────────────┘        Process IncomingCall event
        │                   Transition: Idle -> Proceeding
        ▼
6. Execute Actions:
┌──────────────────┐
│  DialogAdapter   │──────► Send "180 Ringing"
└──────────────────┘
```

### Outgoing Call Flow (Already Working)

```
┌─────────────────────────────────────────────────────────┐
│               Outgoing Call Flow                         │
└─────────────────────────────────────────────────────────┘

1. User Makes Call:
┌──────────────┐
│  SimplePeer  │──────► alice.call("bob@127.0.0.1")
└──────────────┘
        │
        ▼
2. Coordinator Creates UAC Session:
┌──────────────────┐
│UnifiedCoordinator│──────► Creates SessionId: "xyz789"
└──────────────────┘        Role: UAC
        │
        ▼
3. State Machine Processes:
┌──────────────────┐
│  State Machine   │──────► Process MakeCall event
└──────────────────┘        Transition: Idle -> Calling
        │
        ▼
4. Execute Action:
┌──────────────────┐
│  DialogAdapter   │──────► Send INVITE
└──────────────────┘
```

## Key Architectural Changes

### 1. Extensible Signaling Interception Layer

```
┌──────────────────────────────────────────────────────────┐
│         Extensible Signaling Interception (NEW)          │
└──────────────────────────────────────────────────────────┘

BEFORE (Broken):
Dialog-Core ──► DialogAdapter ──► EventRouter ──► StateMachine
                                                   ↑
                                                   └─ Requires session!

AFTER (Fixed + Extensible):
                          ┌─► Custom Handler (call-engine)
                          │         ↓
Dialog-Core ──► DialogAdapter ──► SignalingInterceptor ──► StateMachine
                                   ↑                        ↑
                                   │                        └─ Session exists!
                                   └─ Default: Creates sessions
                                      Custom: Routing decisions
```

**Key Design**: SignalingInterceptor supports pluggable handlers:
- **SimplePeer**: Uses default handler (auto-accept, create sessions)
- **Call-Engine**: Injects custom handler for routing/queuing
- **Media Server**: Custom handler for IVR/recording decisions
- **Call Center**: Check agent availability before accepting

### 2. Session Factory Pattern

```rust
// NEW: Coordinator creates sessions automatically
impl UnifiedCoordinator {
    async fn handle_transport_event(&self, event: TransportEvent) {
        match event {
            TransportEvent::IncomingInvite { from, to, dialog_id, sdp } => {
                // 1. Create new UAS session
                let session_id = SessionId::new();
                let session = SessionState {
                    session_id: session_id.clone(),
                    role: Role::UAS,
                    remote_uri: Some(from.clone()),
                    remote_sdp: sdp,
                    // ... other fields
                };
                
                // 2. Store in SessionStore
                self.store.create_session(session).await?;
                
                // 3. Map dialog to session
                self.dialog_to_session.insert(dialog_id, session_id.clone());
                
                // 4. Notify user via channel
                self.incoming_tx.send(IncomingCall {
                    session_id,
                    from,
                }).await?;
                
                // 5. NOW route to state machine (session exists!)
                self.state_machine.process_event(
                    &session_id,
                    EventType::IncomingCall { from, sdp }
                ).await?;
            }
            TransportEvent::Response { dialog_id, response } => {
                // For existing sessions, just route
                if let Some(session_id) = self.dialog_to_session.get(&dialog_id) {
                    self.state_machine.process_event(session_id, ...).await?;
                }
            }
        }
    }
}
```

### 3. Component Responsibilities

```
┌──────────────────────────────────────────────────────────┐
│              Component Responsibilities                    │
└──────────────────────────────────────────────────────────┘

SimplePeer (User API)
├─ call(target) -> Call
├─ get_incoming_call() -> Option<IncomingCall>
├─ Simple audio send/recv
└─ Hides ALL complexity

UnifiedCoordinator (Session Manager)
├─ Hosts SignalingInterceptor with handlers
├─ Creates sessions for new calls (default)
├─ Maintains SessionRegistry
├─ Provides incoming call channel
├─ Routes events to state machine
└─ Manages session lifecycle

SignalingInterceptor (Extensible)
├─ Default handler for SimplePeer
├─ Custom handler support for call-engine
├─ Decides: Accept/Reject/Defer/Custom
├─ Creates sessions or defers to custom logic
└─ Enables complex routing scenarios

SessionRegistry (ID Mapping) - NEW
├─ Maps SessionId ↔ DialogId (bidirectional)
├─ Maps SessionId ↔ MediaSessionId (bidirectional) 
├─ Thread-safe concurrent access
├─ Lookup by any ID type
└─ Cleanup on session termination

StateMachine (State Engine)
├─ Processes events for EXISTING sessions
├─ Executes state transitions
├─ Triggers actions via adapters
├─ Interprets state table
└─ Pure business logic

Adapters (Protocol Layer)
├─ DialogAdapter: SIP protocol
├─ MediaAdapter: RTP/audio handling
├─ Execute actions from state machine
└─ Generate events for state machine
```

### 4. SessionRegistry - Critical Missing Component

The current architecture is missing a centralized registry to map between different ID types. Events from dialog-core use DialogId, media-core uses MediaSessionId, but the state machine uses SessionId.

```
┌──────────────────────────────────────────────────────────┐
│                  SessionRegistry                          │
└──────────────────────────────────────────────────────────┘

Purpose: Central ID mapping for event routing

Mappings:
    SessionId ──┬──► DialogId
                └──► MediaSessionId
                
    DialogId ────► SessionId
    MediaSessionId ──► SessionId

Usage Flow:
1. Dialog event arrives with DialogId
2. Registry looks up SessionId 
3. Event routed to correct session
4. State machine processes with SessionId

Implementation:
- Use DashMap for concurrent access
- Bidirectional mappings
- Auto-cleanup on session end
```

## API Layers and When to Use Each

### SimplePeer API (High-Level)
**Use for**: Basic SIP clients, softphones, simple peer-to-peer calls
- Auto-accepts incoming calls
- Simple call/hangup/audio methods
- Uses DefaultSignalingHandler internally
- No custom routing or control

### UnifiedCoordinator API (Low-Level) 
**Use for**: Call centers, media servers, complex routing scenarios
- Full control over signaling decisions
- Inject custom SignalingHandler
- Access to session registry and state machine
- Defer calls, custom routing, queue management

```
┌─────────────────────────────────────────────────────────┐
│                   API Layer Choice                       │
└─────────────────────────────────────────────────────────┘

Simple Softphone?          ──► Use SimplePeer
                                (DefaultSignalingHandler)

Call Center/ACD?           ──► Use UnifiedCoordinator  
                                (Custom SignalingHandler)

Media Server/IVR?          ──► Use UnifiedCoordinator
                                (Custom SignalingHandler)

Basic Peer-to-Peer?        ──► Use SimplePeer
                                (DefaultSignalingHandler)
```

## Extensibility for Higher-Level Applications

The SignalingInterceptor's handler pattern enables different use cases:

### SimplePeer (Default Handler)
```rust
// Automatic session creation for simple use cases
let peer = SimplePeer::new("alice", 5060).await?;
// Uses DefaultSignalingHandler internally - accepts all calls
```

### Call-Engine (Using UnifiedCoordinator with Custom Handler)
```rust
// Call-engine uses UnifiedCoordinator directly, NOT SimplePeer
struct CallEngineHandler {
    routing_rules: RoutingEngine,
    agent_pool: AgentPool,
}

impl SignalingHandler for CallEngineHandler {
    async fn on_incoming_invite(&self, invite: InviteDetails) -> SignalingDecision {
        // Complex routing logic
        if let Some(agent) = self.agent_pool.find_available() {
            SignalingDecision::Custom(Box::new(move || {
                // Queue for agent
                // Create session with metadata
                // Send to specific queue
            }))
        } else {
            SignalingDecision::Reject(486) // Busy
        }
    }
}

// Call-engine creates UnifiedCoordinator with custom handler
let handler = Arc::new(CallEngineHandler::new());
let coordinator = UnifiedCoordinator::with_handler(config, handler).await?;
// Now call-engine has full control over signaling and routing
```

### Media Server (IVR Handler)
```rust
struct IvrHandler {
    menu_system: IvrMenuSystem,
}

impl SignalingHandler for IvrHandler {
    async fn on_incoming_invite(&self, invite: InviteDetails) -> SignalingDecision {
        // Defer to IVR logic
        SignalingDecision::Defer
    }
}
```

This design allows session-core-v2 to be:
- **Simple by default** (SimplePeer just works)
- **Extensible when needed** (call-engine can customize everything)
- **Backwards compatible** (old patterns still work)

## Why We Need This Architecture

### For SIP Clients
Users building softphones need:
- Simple call/answer API
- Audio in/out
- No knowledge of SIP internals

### For Call Centers  
Call routing systems need:
- Handle thousands of concurrent calls
- Route based on rules
- Bridge calls together
- Transfer capabilities

### For Media Servers
Media processing needs:
- Record calls
- Play announcements
- Mix audio streams
- Conference bridges

The current architecture can't support ANY of these because it can't receive calls!

## Comparison with Original Session-Core

### Original session-core (WORKING)
```rust
// Simple, intuitive API
let peer = SimplePeer::new("alice", 5060).await?;

// Incoming calls just work
peer.on_incoming_call(|call| {
    println!("Call from {}", call.from());
    call.accept()
});

// Outgoing calls are simple
let call = peer.call("bob@server.com").await?;
call.send_audio(frame).await?;
```

### Current session-core-v2 (BROKEN)
```rust
// Complex, confusing API
let config = Config { /* 6 fields */ };
let coordinator = UnifiedCoordinator::new(config).await?;

// Must create session BEFORE call exists (makes no sense!)
let session = UnifiedSession::new(coordinator, Role::UAS).await?;

// No way to receive real calls
// Examples literally SIMULATE incoming calls!
```

### After Fix (WORKING)
```rust
// Back to simple API
let peer = SimplePeer::new("alice", 5060).await?;

// Incoming calls work again
if let Some(incoming) = peer.get_incoming_call().await {
    let call = incoming.accept().await?;
    
    // Send audio
    call.send_audio(frame).await?;
    
    // Receive audio
    if let Some(frame) = call.recv_audio().await {
        // Process received audio frame
    }
}

// Outgoing calls stay simple
let call = peer.call("bob@server.com").await?;

// Bidirectional audio
call.send_audio(outgoing_frame).await?;
let incoming_frame = call.recv_audio().await?;
```

## State Machine's Role

The state machine is GOOD - it provides:
- Clear state transitions (Idle -> Ringing -> Active -> Terminated)
- Coordinated actions (send SIP, start media, etc.)
- Extensibility via state table

But it should be an implementation detail, not the API!

```
User sees:              State Machine does:
call.accept()    ──►    Idle -> Proceeding -> Active
                        Send 180, Send 200, Start Media
                        
call.hangup()    ──►    Active -> Terminating -> Terminated  
                        Send BYE, Stop Media, Cleanup
```

## Implementation Priority

### Phase 1: Make Incoming Calls Work (CRITICAL)
Without this, the library is useless for real applications.

### Phase 2: Simple API (HIGH)
Without this, developers will avoid the library.

### Phase 3: Advanced Features (MEDIUM)
Transfer, conference, etc. can come after basics work.

## Success Criteria

1. **The Peer Audio Example Works End-to-End**
   - Alice calls Bob using real SIP
   - Bob receives the call (not simulated!)
   - Both exchange real RTP audio
   - Both save valid .wav files

2. **API Simplicity Test**
   - Making a call: < 5 lines of code
   - Receiving a call: < 5 lines of code
   - No need to understand UAC/UAS/sessions

3. **Architecture Clarity**
   - Each component has one clear job
   - State machine only handles existing sessions
   - Coordinator manages session lifecycle

## Implementation Layers for Call Control

The architecture supports comprehensive call control through clear separation of concerns:

```
┌──────────────────────────────────────────────────────────┐
│           Call Control Implementation Layers              │
└──────────────────────────────────────────────────────────┘

Layer 1: SimplePeer (API Surface)
├─ call.hold() / call.resume()
├─ call.send_dtmf()
├─ call.transfer()
└─ Just delegates to UnifiedSession

Layer 2: UnifiedSession (Session Operations)
├─ Sends events to state machine (hold, transfer)
├─ Direct adapter calls (DTMF, recording)
├─ Queries session store (getters)
└─ Per-session operations only

Layer 3: UnifiedCoordinator (System Operations)  
├─ Registration (system-wide)
├─ Conference management (multi-session)
├─ Session lifecycle (create/destroy)
└─ Cross-session coordination

Layer 4: State Machine (Business Logic)
├─ State transitions (Active -> OnHold)
├─ Action sequencing (re-INVITE then update media)
├─ Protocol coordination (SIP + RTP)
└─ Driven by state table

Layer 5: Adapters (Protocol Implementation)
├─ DialogAdapter: SIP signaling (INVITE, REFER, REGISTER)
├─ MediaAdapter: RTP/audio (DTMF, recording, mixing)
├─ Execute actions from state machine
└─ Generate events for state machine
```

### Example: Conference Call Implementation

```
User Level:
    conference = peer.create_conference()
    conference.add_call(call1)
    conference.add_call(call2)

Implementation:
    SimplePeer::create_conference()
        ↓
    UnifiedCoordinator::create_conference()
        ├─ Creates ConferenceId
        ├─ MediaAdapter::create_audio_mixer()
        └─ Returns Conference handle
    
    Conference::add_call(call)
        ↓
    UnifiedCoordinator::add_to_conference(conf_id, session)
        ├─ MediaAdapter::redirect_to_mixer(session_id, conf_id)
        │   ├─ Get media session for call
        │   ├─ Get mixer for conference  
        │   ├─ Connect audio sink to mixer input
        │   └─ Connect audio source from mixer output
        └─ Update conference participant list

Audio Flow:
    Call1 RTP ──► MediaSession1 ──► Mixer ──► Mixed Audio ──► All Participants
    Call2 RTP ──► MediaSession2 ──┘
```

This layered architecture ensures:
- Simple API remains simple (SimplePeer/Call)
- Complex operations are properly decomposed
- State machine handles protocol coordination
- Adapters handle protocol details
- Clear separation between session and system operations

## Integration with Core Services

Session-core-v2 integrates with existing RVoIP core services for complete functionality:

```
┌──────────────────────────────────────────────────────────┐
│              Core Service Integration                      │
└──────────────────────────────────────────────────────────┘

                    SimplePeer
                        │
                        ▼
                UnifiedCoordinator
                   /    │    \
                  /     │     \
                 ▼      ▼      ▼
          AuthAdapter  RegistrarAdapter  DialogAdapter
                │            │                │
                ▼            ▼                ▼
          auth-core    registrar-core    dialog-core
                             │
                             ├─ Registration (REGISTER)
                             ├─ Presence (SUBSCRIBE/NOTIFY)  
                             ├─ Location Service
                             └─ Call Parking
```

### Auth-Core Integration
- Digest authentication for SIP requests
- Token validation
- Challenge/response handling
- Credential management

### Registrar-Core Integration
- User registration with authentication
- Contact binding management
- Presence state (PUBLISH/SUBSCRIBE/NOTIFY)
- Buddy list management
- Call parking via special AORs

### Complete SIP Feature Set
With these integrations, session-core-v2 supports:
- **Basic Calls**: INVITE, BYE, CANCEL
- **Call Control**: Hold, Transfer, DTMF
- **Registration**: REGISTER with auth
- **Presence**: Full presence model
- **Messaging**: SIP MESSAGE
- **Parking**: Park/retrieve calls
- **Conference**: Local mixing

This makes session-core-v2 feature-complete for sip-client requirements.

## Adapter Plugin System

Session-core-v2 provides an extensible adapter system similar to FreeSWITCH/Asterisk modules, allowing developers to extend functionality without modifying core code.

```
┌──────────────────────────────────────────────────────────┐
│                Adapter Plugin Architecture                │
└──────────────────────────────────────────────────────────┘

Session-Core Adapter Types:              Media-Core Adapter Types:
├─ Call Event Adapters                   ├─ Codec Adapters (G.711, Opus)
├─ State Action Adapters                  ├─ DSP Adapters (Echo cancel)
├─ Analytics Adapters                     └─ Format Adapters (WAV, MP3)
├─ Storage Adapters                       
└─ Integration Adapters                   

            SimplePeer/Call
                  │
                  ▼
          UnifiedCoordinator
                  │
         ┌────────┴────────┐
         │                 │
    AdapterRegistry   MediaAdapter ──► media-core
         │                              │
         ▼                              ▼
    Session Adapters              Media Adapters
    - Transcription               - Custom Codecs
    - CDR/Billing                 - Audio Effects  
    - CRM Integration             - Noise Reduction
    - Analytics                  - Format Conversion
```

### Adapter Surfaces at Session-Core Level

```rust
// Session-level adapters (NOT media processing)
pub trait CallEventAdapter: Send + Sync {
    /// Called when call state changes
    async fn on_call_state_change(&self, 
        session_id: &SessionId, 
        old_state: CallState, 
        new_state: CallState
    ) -> Result<()>;
    
    /// Called when DTMF received
    async fn on_dtmf(&self, session_id: &SessionId, digit: char) -> Result<()>;
    
    /// Called when call ends with CDR
    async fn on_call_end(&self, session_id: &SessionId, cdr: CallDetail) -> Result<()>;
}

pub trait StateActionAdapter: Send + Sync {
    /// Check if this adapter handles the action
    fn can_handle(&self, action: &str) -> bool;
    
    /// Execute custom state machine action
    async fn execute(&self, 
        action: &str, 
        session: &SessionState, 
        params: serde_json::Value
    ) -> Result<()>;
}

pub trait StorageAdapter: Send + Sync {
    /// Store call recording metadata (NOT the audio)
    async fn store_recording_metadata(&self, 
        session_id: &SessionId, 
        path: &str, 
        duration: Duration
    ) -> Result<String>;
    
    /// Store CDR
    async fn store_cdr(&self, cdr: CallDetail) -> Result<()>;
}
```

### How Codec Adapter Would Actually Work

Since codecs are media-core's responsibility, session-core would coordinate but not process:

```rust
// WRONG: Session-core doesn't handle codecs directly
// pub trait CodecAdapter { 
//     fn encode(&self, pcm: &[i16]) -> Vec<u8>; 
// }

// CORRECT: Session-core coordinates codec negotiation
pub trait CodecNegotiationAdapter: Send + Sync {
    /// Influence SDP codec priorities during negotiation
    async fn adjust_codec_priorities(&self, 
        offered: Vec<CodecInfo>
    ) -> Vec<CodecInfo>;
    
    /// Notify media-core to load custom codec
    async fn on_codec_selected(&self, 
        session_id: &SessionId, 
        codec: &CodecInfo
    ) -> Result<()> {
        // Tell media-core to load the codec module
        self.notify_media_core(codec).await
    }
}
```

### Real Example: Transcription Adapter

```rust
// adapters/transcription/src/lib.rs
pub struct TranscriptionAdapter {
    api_endpoint: String,
    sessions: Arc<DashMap<SessionId, TranscriptionSession>>,
}

#[async_trait]
impl CallEventAdapter for TranscriptionAdapter {
    async fn on_call_state_change(&self, 
        session_id: &SessionId, 
        old: CallState, 
        new: CallState
    ) -> Result<()> {
        match new {
            CallState::Active => {
                // Start transcription session
                let session = TranscriptionSession::new();
                self.sessions.insert(session_id.clone(), session);
                
                // Register with media-core to receive audio
                media_core::register_tap(session_id, "transcription").await?;
            }
            CallState::Terminated => {
                // Finalize and store transcription
                if let Some(session) = self.sessions.remove(session_id) {
                    let transcript = session.finalize().await?;
                    self.store_transcript(session_id, transcript).await?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
```

### Adapter Loading Strategy

```rust
impl UnifiedCoordinator {
    pub async fn new_with_adapters(config: Config) -> Result<Self> {
        let mut registry = AdapterRegistry::new()?;
        
        // Load from directories
        registry.scan_directories(&[
            "./adapters",
            "/usr/lib/rvoip/adapters",
        ]).await?;
        
        // Register adapters by type
        for adapter in registry.get_call_event_adapters() {
            self.event_bus.register_adapter(adapter);
        }
        
        for adapter in registry.get_state_action_adapters() {
            self.state_machine.register_adapter(adapter);
        }
        
        Ok(self)
    }
}
```

### Implementation Timeline

**Phase 1 (Core)**: Build without adapters
**Phase 2 (Extensibility)**: Add adapter system
- Can be added after core is working
- Won't break existing API
- Allows third-party extensions

This design ensures:
- Session-core handles call logic, not media processing
- Media-core handles codecs, DSP, formats
- Clear boundaries between layers
- Extensible without core modifications

## Conclusion

The state table design is good. The state machine is good. But we put it in the wrong place architecturally. By adding a transport interception layer at the coordinator level and properly layering the implementation of call control functions, we can fix the incoming call problem while keeping all the benefits of the state-driven design and providing a complete telephony API.