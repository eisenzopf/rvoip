# Unified API Design: One API for All Use Cases

## Core Insight

Every VoIP application - whether peer, server, or call center - is just orchestrating state transitions. The **role** (UAC/UAS/B2BUA) and **behavior** are determined by the state table, not the API.

## Use Cases Supported

### 1. Simple Peer (P2P Calling)
```rust
// Alice calls Bob directly
let session = UnifiedSession::new(coordinator.clone(), Role::UAC);
session.make_call("sip:bob@example.com").await?;

// Bob receives call
let session = UnifiedSession::new(coordinator.clone(), Role::UAS);
session.on_incoming_call(incoming_call).await?;
session.accept().await?;
```

### 2. Call Center Server
```rust
// Call center receives customer call
let customer_session = UnifiedSession::new(coordinator.clone(), Role::UAS);
customer_session.on_incoming_call(customer_call).await?;

// Find available agent
let agent = agent_queue.get_next_available().await?;

// Create outbound leg to agent
let agent_session = UnifiedSession::new(coordinator.clone(), Role::UAC);
agent_session.make_call(&agent.sip_uri).await?;

// Bridge the calls when agent answers
coordinator.bridge_sessions(customer_session.id(), agent_session.id()).await?;
```

### 3. B2BUA (Back-to-Back User Agent)
```rust
// B2BUA acts as both UAS and UAC
let inbound = UnifiedSession::new(coordinator.clone(), Role::UAS);
inbound.on_incoming_call(incoming).await?;

// Apply business logic (routing, authentication, etc.)
let destination = routing_engine.get_destination(&incoming).await?;

let outbound = UnifiedSession::new(coordinator.clone(), Role::UAC);
outbound.make_call(&destination).await?;

// Bridge when ready
coordinator.bridge_sessions(inbound.id(), outbound.id()).await?;
```

### 4. Conference Server
```rust
// Conference server handling multiple participants
struct ConferenceRoom {
    room_id: String,
    participants: Vec<UnifiedSession>,
    mixer: AudioMixer,
}

impl ConferenceRoom {
    async fn add_participant(&mut self, call: IncomingCall) -> Result<()> {
        // Each participant is just a session
        let session = UnifiedSession::new(self.coordinator.clone(), Role::UAS);
        session.on_incoming_call(call).await?;
        session.accept().await?;
        
        // Add to conference mixer
        self.mixer.add_stream(session.id()).await?;
        self.participants.push(session);
        Ok(())
    }
}
```

### 5. IVR System
```rust
// IVR with menu navigation
let session = UnifiedSession::new(coordinator.clone(), Role::UAS);
session.on_incoming_call(call).await?;
session.accept().await?;

// Play menu
session.play_audio("press-1-for-sales.wav").await?;

// Handle DTMF
session.on_dtmf(|digit| async move {
    match digit {
        '1' => transfer_to_sales(&session).await,
        '2' => transfer_to_support(&session).await,
        _ => play_invalid_option(&session).await,
    }
}).await?;
```

### 6. SIP Proxy/Registrar
```rust
// Even a SIP proxy can use the same API
struct SipProxy {
    coordinator: Arc<SessionCoordinator>,
    registrations: HashMap<String, Location>,
}

impl SipProxy {
    async fn handle_invite(&self, invite: SipMessage) -> Result<()> {
        // Proxy doesn't establish media, just forwards
        let session = UnifiedSession::new(self.coordinator.clone(), Role::Proxy);
        
        // Lookup destination
        let destination = self.registrations.get(&invite.to)?;
        
        // Forward without establishing dialog
        session.forward_to(destination).await?;
    }
}
```

## The Unified API

```rust
pub struct UnifiedSession {
    id: SessionId,
    coordinator: Arc<SessionCoordinator>,
    role: Role,
    event_tx: mpsc::Sender<EventType>,
}

impl UnifiedSession {
    /// Create a new session with specified role
    pub fn new(coordinator: Arc<SessionCoordinator>, role: Role) -> Self {
        let id = SessionId::new();
        let session_state = SessionState::new(id.clone(), role);
        coordinator.store.create_session(session_state).await?;
        
        Self { id, coordinator, role }
    }
    
    // ===== Core Operations (work for any role) =====
    
    /// Make an outbound call (UAC role)
    pub async fn make_call(&self, target: &str) -> Result<()> {
        self.send_event(EventType::MakeCall { target: target.to_string() }).await
    }
    
    /// Handle incoming call (UAS role)
    pub async fn on_incoming_call(&self, call: IncomingCall) -> Result<()> {
        self.send_event(EventType::IncomingCall { call }).await
    }
    
    /// Accept the call
    pub async fn accept(&self) -> Result<()> {
        self.send_event(EventType::AcceptCall).await
    }
    
    /// Reject the call
    pub async fn reject(&self, reason: &str) -> Result<()> {
        self.send_event(EventType::RejectCall { reason: reason.to_string() }).await
    }
    
    /// Hangup the call
    pub async fn hangup(&self) -> Result<()> {
        self.send_event(EventType::HangupCall).await
    }
    
    // ===== Advanced Operations =====
    
    /// Put call on hold
    pub async fn hold(&self) -> Result<()> {
        self.send_event(EventType::HoldCall).await
    }
    
    /// Resume from hold
    pub async fn resume(&self) -> Result<()> {
        self.send_event(EventType::ResumeCall).await
    }
    
    /// Transfer call (blind or attended)
    pub async fn transfer(&self, target: &str, attended: bool) -> Result<()> {
        if attended {
            self.send_event(EventType::AttendedTransfer { target: target.to_string() }).await
        } else {
            self.send_event(EventType::BlindTransfer { target: target.to_string() }).await
        }
    }
    
    // ===== Media Operations =====
    
    /// Play audio file
    pub async fn play_audio(&self, file: &str) -> Result<()> {
        self.send_event(EventType::PlayAudio { file: file.to_string() }).await
    }
    
    /// Start recording
    pub async fn start_recording(&self) -> Result<()> {
        self.send_event(EventType::StartRecording).await
    }
    
    /// Send DTMF
    pub async fn send_dtmf(&self, digits: &str) -> Result<()> {
        self.send_event(EventType::SendDTMF { digits: digits.to_string() }).await
    }
    
    // ===== Event Handling =====
    
    /// Subscribe to session events
    pub async fn on_event<F>(&self, callback: F) -> Result<()> 
    where
        F: Fn(SessionEvent) -> Future<Output = ()> + Send + 'static
    {
        self.coordinator.subscribe_to_session(self.id.clone(), callback).await
    }
    
    /// Get current state
    pub async fn state(&self) -> Result<CallState> {
        self.coordinator.get_session_state(&self.id).await
    }
    
    // ===== Internal =====
    
    fn send_event(&self, event: EventType) -> Result<()> {
        self.coordinator.process_event(&self.id, event).await
    }
}
```

## Role-Specific Behaviors via State Table

The beauty is that role-specific behavior is defined in the state table, not the API:

### UAC State Table Entries
```rust
// UAC can make calls
(Role::UAC, CallState::Idle, EventType::MakeCall) -> CallState::Initiating

// UAC handles responses
(Role::UAC, CallState::Initiating, EventType::Dialog180Ringing) -> CallState::Ringing
(Role::UAC, CallState::Ringing, EventType::Dialog200OK) -> CallState::Active
```

### UAS State Table Entries
```rust
// UAS receives calls
(Role::UAS, CallState::Idle, EventType::IncomingCall) -> CallState::Initiating

// UAS can accept/reject
(Role::UAS, CallState::Initiating, EventType::AcceptCall) -> CallState::Active
(Role::UAS, CallState::Initiating, EventType::RejectCall) -> CallState::Terminated
```

### B2BUA State Table Entries
```rust
// B2BUA can do both UAC and UAS operations
(Role::B2BUA, CallState::Idle, EventType::IncomingCall) -> CallState::Initiating
(Role::B2BUA, CallState::Idle, EventType::MakeCall) -> CallState::Initiating

// B2BUA-specific: can bridge sessions
(Role::B2BUA, CallState::Active, EventType::BridgeSessions) -> CallState::Bridged
```

### Call Center Specific States
```rust
// Call center specific states
pub enum CallState {
    // ... standard states ...
    
    // Call center states
    Queued,           // Call waiting in queue
    ConnectingToAgent, // Finding available agent
    AgentRinging,     // Agent phone ringing
    OnHoldByAgent,    // Agent put customer on hold
    PostCallWork,     // Agent wrapping up
}

// Call center transitions
(Role::CallCenter, CallState::Active, EventType::QueueCall) -> CallState::Queued
(Role::CallCenter, CallState::Queued, EventType::AgentAvailable) -> CallState::ConnectingToAgent
(Role::CallCenter, CallState::ConnectingToAgent, EventType::AgentAnswered) -> CallState::Active
```

## Why This Works

1. **Separation of Concerns**
   - API provides the interface
   - State table defines the behavior
   - Role determines which transitions are valid

2. **Flexibility**
   - Add new roles without changing API
   - Add new states for specific use cases
   - Compose complex behaviors from simple transitions

3. **Consistency**
   - Same API for all applications
   - Same event flow
   - Same error handling

4. **Scalability**
   - Call center with 1000 agents? Same API
   - Simple peer app? Same API
   - Complex B2BUA? Same API

## Examples by Use Case

### Peer Application
```rust
// Simple peer - 50 lines of code
async fn main() {
    let coordinator = SessionCoordinator::new(config).await?;
    
    // Make a call
    let session = UnifiedSession::new(coordinator, Role::UAC);
    session.make_call("sip:friend@example.com").await?;
    
    // Wait for answer
    session.on_event(|event| async {
        if let SessionEvent::CallEstablished = event {
            println!("Call connected!");
        }
    }).await?;
}
```

### Call Center Server
```rust
// Call center - still simple!
struct CallCenter {
    coordinator: Arc<SessionCoordinator>,
    agent_queue: AgentQueue,
    active_calls: HashMap<SessionId, CallInfo>,
}

impl CallCenter {
    async fn handle_customer_call(&self, call: IncomingCall) -> Result<()> {
        // Customer leg
        let customer = UnifiedSession::new(self.coordinator.clone(), Role::UAS);
        customer.on_incoming_call(call).await?;
        
        // Queue if no agents available
        if self.agent_queue.is_empty() {
            customer.play_audio("all-agents-busy.wav").await?;
            customer.send_event(EventType::QueueCall).await?;
            return Ok(());
        }
        
        // Get next agent
        let agent = self.agent_queue.pop().await?;
        
        // Agent leg
        let agent_session = UnifiedSession::new(self.coordinator.clone(), Role::UAC);
        agent_session.make_call(&agent.sip_uri).await?;
        
        // When agent answers, bridge
        agent_session.on_event(|event| async {
            if let SessionEvent::CallEstablished = event {
                self.coordinator.bridge_sessions(
                    customer.id(), 
                    agent_session.id()
                ).await?;
            }
        }).await?;
        
        Ok(())
    }
}
```

## Conclusion

The Unified API works for **everything** because:

1. **It's just state transitions** - Whether you're a peer, server, or call center, you're just moving through states
2. **Role is configuration** - The state table determines what transitions are valid for each role
3. **Behavior is declarative** - Complex behaviors emerge from simple state transitions
4. **No special cases** - A call center is just a B2BUA with queue states

This dramatically simplifies the codebase while supporting all use cases!