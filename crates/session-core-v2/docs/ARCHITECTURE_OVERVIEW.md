# Session-Core-v2 Architecture Overview

## Table of Contents
1. [What is Session-Core-v2?](#what-is-session-core-v2)
2. [Core Architecture](#core-architecture)
3. [How Components Work Together](#how-components-work-together)
4. [Understanding States, Events, and Actions](#understanding-states-events-and-actions)
5. [Where Things Are Defined](#where-things-are-defined)
6. [Developer Guide](#developer-guide)
7. [Using Session-Core for Different Applications](#using-session-core-for-different-applications)
8. [Example: Call Flow Walkthrough](#example-call-flow-walkthrough)

## What is Session-Core-v2?

Session-Core-v2 is a **state machine-based SIP session management library** that provides a clean abstraction layer for building VoIP applications. It acts as the "brain" that coordinates between SIP signaling (dialog-core) and media handling (media-core) without needing to understand the low-level protocol details.

### Key Features
- **Protocol-Agnostic State Machine**: Core logic doesn't know about SIP or RTP specifics
- **Declarative Configuration**: State transitions defined in YAML, not code
- **Adapter Pattern**: Clean separation between protocols and business logic
- **Event-Driven Architecture**: Reactive design with clear event flows
- **Extensible**: Easy to add new states, events, and behaviors

### What Problems Does It Solve?
1. **Complexity Management**: SIP is complex; this library abstracts away the complexity
2. **State Coordination**: Manages the intricate dance between signaling and media
3. **Flexibility**: Change behavior by editing YAML files, not code
4. **Reusability**: Same state machine can power different types of VoIP applications

## Core Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                          External World                          │
│  (SIP Messages, RTP Packets, API Calls)                        │
└─────────────────┬───────────────────────┬──────────────────────┘
                  │                       │
┌─────────────────▼───────────┐ ┌────────▼──────────────┐
│      Dialog-Core            │ │    Media-Core         │
│  (SIP Protocol Handler)     │ │  (RTP/Audio Handler)  │
└─────────────────┬───────────┘ └────────┬──────────────┘
                  │                       │
┌─────────────────▼───────────────────────▼──────────────────────┐
│                          Adapters Layer                         │
│  ┌─────────────────┐  ┌──────────────┐  ┌────────────────┐   │
│  │ Dialog Adapter  │  │ Media Adapter│  │ Event Router   │   │
│  │                 │  │              │  │                │   │
│  └─────────┬───────┘  └──────┬───────┘  └────────┬───────┘   │
└────────────┼─────────────────┼───────────────────┼────────────┘
             │                 │                   │
             └─────────────────▼───────────────────┘
                              │
             ┌────────────────▼────────────────┐
             │        State Machine            │
             │   ┌─────────────────────────┐  │
             │   │   State Table (YAML)    │  │
             │   └─────────────────────────┘  │
             │   ┌─────────────────────────┐  │
             │   │      Executor           │  │
             │   └─────────────────────────┘  │
             │   ┌─────────────────────────┐  │
             │   │    Session Store        │  │
             │   └─────────────────────────┘  │
             └─────────────────────────────────┘
```

### Key Components

#### 1. **State Machine** (`state_machine/`)
The heart of the system that:
- Reads state transitions from YAML configuration
- Processes incoming events
- Executes guards (conditions) and actions
- Maintains session state
- Is completely protocol-agnostic

#### 2. **Adapters** (`adapters/`)
Translation layer between external protocols and the state machine:
- **DialogAdapter**: Translates SIP ↔ State Machine
- **MediaAdapter**: Translates RTP/Audio ↔ State Machine
- **EventRouter**: Routes events to correct handlers
- **SignalingInterceptor**: Creates sessions for incoming calls

#### 3. **State Table** (`state_tables/`)
Declarative YAML configuration that defines:
- All possible states (Idle, Ringing, Active, etc.)
- Valid transitions between states
- Guards (conditions) for transitions
- Actions to execute during transitions
- Event mappings

#### 4. **Session Store** (`session_store/`)
Manages runtime session data:
- Current state of each session
- Session metadata (URIs, SDP, etc.)
- Condition flags (DialogEstablished, MediaReady, etc.)
- Relationships between sessions

## How Components Work Together

> **Note**: For detailed event flow diagrams including incoming calls, outgoing calls, and bridge scenarios, see `EVENT_FLOW_DIAGRAMS.md`.

### Event Flow Example: Making a Call

1. **API Call**: Application calls `session.make_call("sip:bob@example.com")`
2. **Event Generation**: API generates `MakeCall` event
3. **State Machine Lookup**: Finds transition for (Idle + MakeCall + UAC role)
4. **Guard Check**: Verifies any conditions are met
5. **Actions Execute**:
   - `CreateDialog` → DialogAdapter prepares dialog
   - `CreateMediaSession` → MediaAdapter creates RTP session
   - `GenerateLocalSDP` → MediaAdapter generates SDP offer
   - `SendINVITE` → DialogAdapter sends SIP INVITE
6. **State Update**: Session moves from `Idle` → `Initiating`
7. **Response Handling**: When 200 OK arrives:
   - DialogAdapter translates to `Dialog200OK` event
   - State machine processes event
   - Actions: `SendACK`, `StartMediaFlow`
   - State: `Initiating` → `Active`

### The Adapter Pattern Advantage

```rust
// Without adapters (tightly coupled):
if sip_message.method == "INVITE" {
    create_rtp_session();
    generate_sdp();
    send_response(180);
}

// With adapters (loosely coupled):
state_machine.process_event("IncomingCall");
// State machine doesn't know this came from SIP!
```

## Understanding States, Events, and Actions

### States
States represent the current condition of a call session:

```yaml
states:
  - name: "Idle"
    description: "No active call"
  - name: "Ringing"
    description: "Call is ringing"
  - name: "Active"
    description: "Call established with media"
  - name: "OnHold"
    description: "Call is on hold"
  # ... more states
```

### Events
Events trigger state transitions:

```rust
// API Events (from application)
MakeCall              // User wants to make a call
AcceptCall           // User accepts incoming call
HangupCall          // User ends call

// Dialog Events (from SIP)
IncomingCall        // INVITE received
Dialog200OK        // 200 OK received
DialogBYE          // BYE received

// Media Events (from RTP)
MediaSessionReady  // RTP port allocated
MediaFlowStarted   // Audio flowing
MediaError         // RTP problem
```

### Actions
Actions are operations executed during transitions:

```rust
// Dialog Actions
CreateDialog        // Prepare SIP dialog
SendINVITE         // Send SIP INVITE
SendACK            // Acknowledge response
SendBYE            // End call

// Media Actions
CreateMediaSession  // Allocate RTP resources
GenerateLocalSDP   // Create SDP offer/answer
StartMediaFlow     // Begin audio streaming
StopMediaFlow      // Stop audio

// Control Actions
UpdateCallState    // Internal state update
PublishEvent       // Notify listeners
```

### Transitions
Transitions define how the system moves between states:

```yaml
transitions:
  - role: "UAC"                    # Who can do this
    state: "Idle"                  # Starting state
    event:
      type: "MakeCall"            # Triggering event
    guards:                       # Conditions (optional)
      - "HasValidTarget"
    actions:                      # What to do
      - type: "CreateDialog"
      - type: "SendINVITE"
    next_state: "Initiating"      # Where to go
    condition_updates:            # Update flags
      HasMediaSession: true
```

## Where Things Are Defined

### Directory Structure
```
session-core-v2/
├── src/
│   ├── state_machine/        # State machine implementation
│   │   ├── executor.rs      # Processes events through state table
│   │   ├── actions.rs       # Action implementations
│   │   └── guards.rs        # Guard condition checks
│   ├── adapters/            # Protocol adapters
│   │   ├── dialog_adapter.rs    # SIP adapter
│   │   ├── media_adapter.rs     # RTP adapter
│   │   └── event_router.rs      # Event distribution
│   ├── state_table/         # State table infrastructure
│   │   ├── types.rs         # Core types (Event, Action, etc.)
│   │   └── yaml_loader.rs   # YAML parser
│   ├── session_store/       # Session management
│   │   └── store.rs         # Session data storage
│   └── api/                 # Public API
│       └── unified.rs       # High-level interface
├── state_tables/            # YAML configurations
│   ├── default_state_table.yaml    # Standard SIP client
│   ├── gateway_states.yaml         # B2BUA/Gateway
│   └── sip_client_states.yaml      # Simple client
└── examples/                # Usage examples
    ├── peer_to_peer.rs     # P2P calling
    └── b2bua.rs            # Gateway example
```

### Finding Things

| What You're Looking For | Where to Find It |
|------------------------|------------------|
| Available states | `state_tables/*.yaml` → `states:` section |
| Available events | `src/state_table/types.rs` → `EventType` enum |
| Available actions | `src/state_table/types.rs` → `Action` enum |
| Action implementations | `src/state_machine/actions.rs` |
| State transitions | `state_tables/*.yaml` → `transitions:` section |
| Event-to-state mappings | `STATE_EVENT_ACTION_MAPPING.md` |
| Adapter implementations | `src/adapters/` |

## Developer Guide

### Adding a New State

1. **Define in YAML** (`state_tables/your_table.yaml`):
```yaml
states:
  - name: "YourNewState"
    description: "What this state represents"
```

2. **Add to Rust enum** (`src/types.rs`):
```rust
pub enum CallState {
    // ... existing states
    YourNewState,
}
```

3. **Define transitions** to/from your state:
```yaml
transitions:
  - role: "Both"
    state: "Active"
    event:
      type: "YourTriggerEvent"
    actions:
      - type: "YourAction"
    next_state: "YourNewState"
```

### Adding a New Event

1. **Define event type** (`src/state_table/types.rs`):
```rust
pub enum EventType {
    // ... existing events
    YourNewEvent,
}
```

2. **Add event handling** in appropriate adapter:
```rust
// In dialog_adapter.rs or media_adapter.rs
async fn handle_your_event(&self, session_id: &SessionId) -> Result<()> {
    // Convert external event to state machine event
    self.state_machine.process_event(session_id, EventType::YourNewEvent).await?;
}
```

3. **Define state transitions** for the event:
```yaml
transitions:
  - state: "SomeState"
    event:
      type: "YourNewEvent"
    actions:
      - type: "ReactToEvent"
    next_state: "NewState"
```

### Adding a New Action

1. **Define action type** (`src/state_table/types.rs`):
```rust
pub enum Action {
    // ... existing actions
    YourNewAction,
}
```

2. **Implement action** (`src/state_machine/actions.rs`):
```rust
pub async fn execute_action(
    action: &Action,
    session: &mut SessionState,
    dialog_adapter: &Arc<DialogAdapter>,
    media_adapter: &Arc<MediaAdapter>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match action {
        // ... existing actions
        Action::YourNewAction => {
            // Your implementation here
            info!("Executing YourNewAction for session {}", session.session_id);
            // Call adapter methods as needed
            dialog_adapter.do_something(&session.session_id).await?;
        }
    }
}
```

3. **Use in transitions**:
```yaml
actions:
  - type: "YourNewAction"
```

### Creating a New Adapter

1. **Create adapter file** (`src/adapters/your_adapter.rs`):
```rust
pub struct YourAdapter {
    store: Arc<SessionStore>,
    // Your protocol-specific dependencies
}

impl YourAdapter {
    pub fn new(store: Arc<SessionStore>) -> Self {
        Self { store }
    }
    
    // Convert protocol events to state machine events
    pub async fn handle_protocol_event(&self, event: ProtocolEvent) -> Result<()> {
        let session_id = self.find_session(&event)?;
        
        // Translate to state machine event
        match event.type {
            ProtocolEventType::Something => {
                self.state_machine.process_event(
                    &session_id, 
                    EventType::YourMappedEvent
                ).await?;
            }
        }
    }
    
    // Execute actions from state machine
    pub async fn execute_action(&self, session_id: &SessionId, action: &str) -> Result<()> {
        // Implement protocol-specific action
    }
}
```

2. **Wire into the system**:
- Add to `adapters/mod.rs`
- Initialize in `SessionCoordinator::new()`
- Route events through `EventRouter`

### Modifying State Table Behavior

The beauty of session-core-v2 is that most behavior changes don't require code changes:

1. **Change state transitions**: Edit YAML file
2. **Add guards/conditions**: Define in `conditions:` section
3. **Reorder actions**: Change order in `actions:` list
4. **Add failure handling**: Add transitions for error events

Example - Adding retry logic:
```yaml
transitions:
  # Normal flow
  - state: "Initiating"
    event:
      type: "DialogTimeout"
    guards:
      - condition: "RetryCount"
        operator: "LessThan"
        value: 3
    actions:
      - type: "IncrementRetry"
      - type: "SendINVITE"  # Retry
    next_state: "Initiating"  # Stay in same state
    
  # Max retries reached
  - state: "Initiating"
    event:
      type: "DialogTimeout"
    guards:
      - condition: "RetryCount"
        operator: "GreaterOrEqual"
        value: 3
    next_state: "Failed"
```

## Using Session-Core for Different Applications

### SIP Client
Basic softphone functionality:

```rust
use rvoip_session_core_v2::api::unified::{UnifiedSession, SessionCoordinator};

// Initialize
let coordinator = SessionCoordinator::new(config).await?;

// Make a call
let session = UnifiedSession::new(coordinator, Role::UAC).await?;
session.make_call("sip:bob@example.com").await?;

// Handle events
session.on_event(|event| {
    match event {
        SessionEvent::CallEstablished => println!("Connected!"),
        SessionEvent::CallTerminated { reason } => println!("Call ended: {}", reason),
        _ => {}
    }
}).await?;
```

**State Table**: Use `sip_client_states.yaml` for basic client features.

### Call Center (via call-engine)
Call center functionality requires multi-session coordination and agent state management, which is beyond the scope of session-core-v2. Instead, implement call centers using the `call-engine` crate:

**Architecture**:
- `call-engine` manages agents, queues, and routing logic
- `session-core-v2` handles individual SIP sessions (customer and agent)
- `call-engine` orchestrates multiple session-core-v2 instances

```rust
// In call-engine (pseudo-code)
// 1. Customer call arrives - create session in session-core-v2
let customer_session = session_core.create_session(incoming_call).await?;

// 2. Queue management in call-engine
queue_manager.add_call(customer_session.id, priority).await?;

// 3. Find agent (call-engine logic)
let agent = agent_manager.find_available_agent(skills).await?;

// 4. Create agent session via session-core-v2
let agent_session = session_core.make_call(agent.extension).await?;

// 5. Bridge sessions when both answer
session_core.bridge_sessions(customer_session.id, agent_session.id).await?;
```

**Note**: Session-core-v2 provides the SIP/media layer; call-engine adds the call center abstractions.

### SIP Gateway/B2BUA
Bridge between different networks or protocols:

```rust
// Incoming call from Network A
let inbound = coordinator.create_session_for_incoming(call_info).await?;

// Create outbound leg to Network B
let outbound = UnifiedSession::new(coordinator, Role::UAC).await?;
outbound.make_call(&translated_uri).await?;

// Bridge the calls
coordinator.bridge_sessions(&inbound, &outbound).await?;

// Handle transcoding if needed
if inbound.codec() != outbound.codec() {
    coordinator.enable_transcoding(&inbound, &outbound).await?;
}
```

**State Table**: Use `gateway_states.yaml` with bridge states.

### Key Differences by Use Case

| Feature | SIP Client | Gateway | Call Center (call-engine) |
|---------|------------|---------|---------------------------|
| States | Basic (Idle, Ringing, Active) | Bridge states | Uses session-core + agent states |
| Events | User-initiated | Protocol translation | Orchestrates session events |
| Actions | Simple call control | Transcoding, routing | Queue/agent management |
| Architecture | Single session | Dual sessions | Multi-session orchestration |

## Example: Call Flow Walkthrough

Let's trace a complete outbound call:

### 1. Application Makes Call
```rust
session.make_call("sip:bob@example.com").await?;
```

### 2. API Generates Event
```rust
// In unified.rs
self.state_machine.process_event(&self.id, EventType::MakeCall).await?;
```

### 3. State Machine Processes
```yaml
# From state table
- role: "UAC"
  state: "Idle"
  event:
    type: "MakeCall"
  actions:
    - type: "CreateDialog"
    - type: "CreateMediaSession"
    - type: "GenerateLocalSDP"
    - type: "SendINVITE"
  next_state: "Initiating"
```

### 4. Actions Execute
```rust
// In actions.rs
Action::CreateDialog => {
    // Prepare dialog info
}
Action::SendINVITE => {
    dialog_adapter.send_invite_with_details(...).await?;
}
```

### 5. SIP INVITE Sent
```
INVITE sip:bob@example.com SIP/2.0
From: <sip:alice@example.com>
To: <sip:bob@example.com>
...
```

### 6. Response Arrives
```
SIP/2.0 200 OK
...
```

### 7. Adapter Translates
```rust
// In dialog_adapter.rs
SessionCoordinationEvent::InviteResponse(response) => {
    if response.status_code() == 200 {
        state_machine.process_event(session_id, EventType::Dialog200OK).await?;
    }
}
```

### 8. State Updates
```yaml
- state: "Initiating"
  event:
    type: "Dialog200OK"
  actions:
    - type: "SendACK"
    - type: "StartMediaFlow"
  next_state: "Active"
```

### 9. Media Flows
- DialogAdapter sends ACK
- MediaAdapter starts RTP
- Audio flows bidirectionally
- Session is now Active

## Summary

Session-Core-v2 provides a powerful, flexible framework for building VoIP applications by:

1. **Separating Concerns**: Protocol details stay in adapters, business logic in state machine
2. **Declarative Configuration**: Behavior defined in YAML, not code
3. **Event-Driven Design**: Clear, traceable event flows
4. **Extensibility**: Easy to add new features without breaking existing code

The key to understanding session-core-v2 is recognizing that it's a **coordinator**, not an implementer. It orchestrates the dance between SIP signaling and media handling, but delegates the actual protocol work to specialized components.

For more details, see:
- `EVENT_FLOW_DIAGRAMS.md` - Detailed event flow diagrams and sequences
- `STATE_EVENT_ACTION_MAPPING.md` - Complete event/state reference
- `ADAPTER_ARCHITECTURE_EXPLANATION.md` - Deep dive on adapters
- `examples/` - Working code examples
- `state_tables/` - YAML configurations for different use cases
