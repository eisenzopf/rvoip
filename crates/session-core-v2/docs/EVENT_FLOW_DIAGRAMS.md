# Session-Core-v2 Event Flow Diagrams

This document provides detailed diagrams and explanations of how events flow through the session-core-v2 system, from external protocols through the global event system to state machine actions.

## Table of Contents
1. [Event System Overview](#event-system-overview)
2. [Key Components](#key-components)
3. [Incoming Call Flow](#incoming-call-flow)
4. [Outgoing Call Flow](#outgoing-call-flow)
5. [Bridge Call Flow](#bridge-call-flow)
6. [Event Types and Routing](#event-types-and-routing)
7. [Common Event Patterns](#common-event-patterns)

## Event System Overview

Session-core-v2 uses a layered event system that separates protocol-specific events from business logic:

```
┌─────────────────────────────────────────────────────────────────────────┐
│                              External World                              │
│                    (SIP Messages, RTP Packets, API Calls)               │
└─────────────────────────────────┬───────────────────────────────────────┘
                                  │
┌─────────────────────────────────▼───────────────────────────────────────┐
│                        Global Event Coordinator                          │
│                        (infra-common event bus)                         │
│  • Singleton instance for monolithic deployments                        │
│  • Routes CrossCrateEvents between components                           │
│  • Type-safe event contracts                                            │
└─────────────────────────────────┬───────────────────────────────────────┘
                                  │
                   ┌──────────────┴──────────────┐
                   │                             │
         ┌─────────▼─────────┐         ┌────────▼────────┐
         │   Dialog Events   │         │  Media Events   │
         │ DialogToSession   │         │ MediaToSession  │
         └─────────┬─────────┘         └────────┬────────┘
                   │                             │
┌──────────────────▼─────────────────────────────▼────────────────────────┐
│                           Event Router                                   │
│  • Routes events to appropriate handlers                                │
│  • Manages adapter coordination                                         │
│  • No direct channel communication                                      │
└─────────────────────────────────┬───────────────────────────────────────┘
                                  │
┌─────────────────────────────────▼───────────────────────────────────────┐
│                      Session Event Handler                               │
│  • Converts protocol events to state machine events                     │
│  • Creates sessions for incoming calls                                  │
│  • Manages session-dialog mappings                                      │
└─────────────────────────────────┬───────────────────────────────────────┘
                                  │
┌─────────────────────────────────▼───────────────────────────────────────┐
│                          State Machine                                   │
│  • Processes events against state table                                 │
│  • Executes guards and actions                                          │
│  • Updates session state                                                │
└─────────────────────────────────────────────────────────────────────────┘
```

## Key Components

### 1. **Global Event Coordinator** (infra-common)
- Singleton event bus for the entire application
- Publishes and subscribes to `CrossCrateEvent` types
- Handles event serialization for distributed deployments (future)

### 2. **Event Types**
```rust
// Cross-crate events (infra-common)
pub enum RvoipCrossCrateEvent {
    DialogToSession(DialogToSessionEvent),
    MediaToSession(MediaToSessionEvent),
    SessionToDialog(SessionToDialogEvent),
    SessionToMedia(SessionToMediaEvent),
}

// Dialog → Session events
pub enum DialogToSessionEvent {
    IncomingCall { from, to, call_id, dialog_id, sdp },
    CallProgress { dialog_id, status_code },
    CallAnswered { dialog_id, sdp },
    CallTerminated { dialog_id, reason },
}

// Media → Session events  
pub enum MediaToSessionEvent {
    MediaSessionReady { session_id, local_addr },
    MediaFlowEstablished { session_id, remote_addr },
    MediaError { session_id, error },
}
```

### 3. **Event Router** (`event_router.rs`)
- Central hub for all event handling in session-core-v2
- NO scattered event handlers or channels
- Routes events to state machine and executes resulting actions

### 4. **Session Event Handler** (`session_event_handler.rs`)
- Implements `CrossCrateEventHandler` trait
- Converts protocol events to state machine `EventType`
- Creates sessions for incoming calls (ONLY place besides API)

## Incoming Call Flow

### Overview
When a SIP INVITE arrives, it flows through multiple layers before reaching the state machine:

```
┌──────────────────────────────────────────────────────────────────────────┐
│                              SIP INVITE                                   │
│                     "INVITE sip:bob@example.com"                        │
└─────────────────────────────────┬────────────────────────────────────────┘
                                  │
                                  ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                           Dialog-Core                                     │
│  1. Receives and parses SIP INVITE                                      │
│  2. Creates dialog state                                                 │
│  3. Generates dialog_id (UUID)                                          │
│  4. Publishes DialogToSessionEvent::IncomingCall                        │
└─────────────────────────────────┬────────────────────────────────────────┘
                                  │
                                  ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                     Global Event Coordinator                              │
│  5. Receives CrossCrateEvent                                            │
│  6. Routes to registered handlers                                        │
│  7. Finds SessionEventHandler registered for DialogToSession            │
└─────────────────────────────────┬────────────────────────────────────────┘
                                  │
                                  ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                      Session Event Handler                                │
│  8. Handles DialogToSessionEvent::IncomingCall                          │
│  9. Creates new session_id and UAS session in store                     │
│  10. Maps dialog_id ↔ session_id in registry                           │
│  11. Stores incoming call info                                          │
│  12. Converts to EventType::IncomingCall                               │
└─────────────────────────────────┬────────────────────────────────────────┘
                                  │
                                  ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                          State Machine                                    │
│  13. Looks up transition: (Idle, IncomingCall, UAS)                    │
│  14. Executes actions:                                                  │
│      - CreateMediaSession                                               │
│      - StoreRemoteSDP                                                   │
│      - SendSIPResponse(180, "Ringing")                                 │
│  15. Updates state: Idle → Ringing                                     │
└─────────────────────────────────┬────────────────────────────────────────┘
                                  │
                                  ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                          Action Execution                                 │
│  16. CreateMediaSession → MediaAdapter allocates RTP port               │
│  17. SendSIPResponse → DialogAdapter sends "180 Ringing"               │
│  18. Updates session conditions (HasRemoteSDP=true)                     │
└──────────────────────────────────────────────────────────────────────────┘
```

### Detailed Sequence

```
External       Dialog-Core    GlobalCoordinator    SessionHandler    StateMachine    DialogAdapter
    │               │                 │                  │                │               │
    │   INVITE      │                 │                  │                │               │
    ├──────────────>│                 │                  │                │               │
    │               │                 │                  │                │               │
    │               │ Create Dialog   │                  │                │               │
    │               ├─────────────────│                  │                │               │
    │               │                 │                  │                │               │
    │               │ Publish Event   │                  │                │               │
    │               ├────────────────>│                  │                │               │
    │               │                 │                  │                │               │
    │               │                 │ Route to Handler │                │               │
    │               │                 ├─────────────────>│                │               │
    │               │                 │                  │                │               │
    │               │                 │                  │ Create Session │               │
    │               │                 │                  ├───────────────>│               │
    │               │                 │                  │                │               │
    │               │                 │                  │ Process Event  │               │
    │               │                 │                  ├───────────────>│               │
    │               │                 │                  │                │               │
    │               │                 │                  │                │ Send 180      │
    │               │                 │                  │                ├──────────────>│
    │               │                 │                  │                │               │
    │  180 Ringing  │                 │                  │                │               │
    │<──────────────┼─────────────────┼──────────────────┼────────────────┼───────────────│
    │               │                 │                  │                │               │
```

### Key Points
1. **Session Creation**: SessionEventHandler is the ONLY place that creates sessions for incoming calls
2. **ID Mapping**: Dialog ID from dialog-core is mapped to session ID in session-core-v2
3. **State Transition**: New session starts in Idle state and transitions to Ringing
4. **No Channels**: All communication via global event bus, no direct channels

## Outgoing Call Flow

### Overview
When the application initiates an outgoing call:

```
┌──────────────────────────────────────────────────────────────────────────┐
│                         API Call (Application)                            │
│              session.make_call("sip:bob@example.com")                   │
└─────────────────────────────────┬────────────────────────────────────────┘
                                  │
                                  ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                      Unified Session API                                  │
│  1. Creates UAC session in store                                        │
│  2. Sets local/remote URIs                                              │
│  3. Generates EventType::MakeCall                                       │
└─────────────────────────────────┬────────────────────────────────────────┘
                                  │
                                  ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                          State Machine                                    │
│  4. Looks up transition: (Idle, MakeCall, UAC)                         │
│  5. Executes actions:                                                   │
│      - CreateDialog                                                     │
│      - CreateMediaSession                                               │
│      - GenerateLocalSDP                                                 │
│      - SendINVITE                                                      │
│  6. Updates state: Idle → Initiating                                   │
└─────────────────────────────────┬────────────────────────────────────────┘
                                  │
                                  ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                      Action Execution (Event Router)                      │
│  7. CreateMediaSession → MediaAdapter allocates RTP session             │
│  8. GenerateLocalSDP → MediaAdapter creates SDP offer                   │
│  9. SendINVITE → DialogAdapter creates dialog & sends INVITE           │
│  10. Publishes SessionToDialogEvent::SendInvite                        │
└─────────────────────────────────┬────────────────────────────────────────┘
                                  │
                                  ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                           Dialog-Core                                     │
│  11. Receives SendInvite event                                          │
│  12. Creates outbound dialog                                            │
│  13. Sends SIP INVITE with SDP                                         │
│  14. Waits for response...                                              │
└─────────────────────────────────┬────────────────────────────────────────┘
                                  │
                                  ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                     Response Processing (200 OK)                          │
│  15. Dialog-core receives 200 OK                                        │
│  16. Publishes DialogToSessionEvent::CallAnswered                      │
│  17. SessionHandler converts to EventType::Dialog200OK                  │
│  18. State Machine: Initiating → Active                                │
│  19. Actions: SendACK, StartMediaFlow                                  │
└──────────────────────────────────────────────────────────────────────────┘
```

### Detailed Sequence

```
Application    UnifiedAPI    StateMachine    MediaAdapter    DialogAdapter    Dialog-Core
     │             │              │               │               │                │
     │ make_call() │              │               │               │                │
     ├────────────>│              │               │               │                │
     │             │              │               │               │                │
     │             │ Create       │               │               │                │
     │             │ Session      │               │               │                │
     │             ├─────────────>│               │               │                │
     │             │              │               │               │                │
     │             │              │ CreateMedia   │               │                │
     │             │              ├──────────────>│               │                │
     │             │              │               │               │                │
     │             │              │ Generate SDP  │               │                │
     │             │              ├──────────────>│               │                │
     │             │              │               │               │                │
     │             │              │               │  SendINVITE   │                │
     │             │              ├───────────────┼──────────────>│                │
     │             │              │               │               │                │
     │             │              │               │               │ Create Dialog  │
     │             │              │               │               ├───────────────>│
     │             │              │               │               │                │
     │             │              │               │               │   SIP INVITE   │
     │             │              │               │               │───────────────>│
     │             │              │               │               │                │
     │             │              │               │               │   200 OK       │
     │             │              │               │               │<───────────────│
     │             │              │               │               │                │
     │             │              │ Process 200OK │               │                │
     │             │              │<──────────────┼───────────────┼────────────────│
     │             │              │               │               │                │
     │             │              │               │               │ Send ACK       │
     │             │              ├───────────────┼──────────────>│───────────────>│
     │             │              │               │               │                │
     │             │              │ Start Media   │               │                │
     │             │              ├──────────────>│               │                │
     │             │              │               │               │                │
```

### Key Points
1. **Session Creation**: API creates session directly (not via event handler)
2. **Dialog Creation**: DialogAdapter creates dialog when sending INVITE
3. **SDP Flow**: Media adapter generates SDP before INVITE is sent
4. **State Progression**: Idle → Initiating → Active

## Bridge Call Flow

### Overview
Bridging connects two established calls (e.g., for transfers or B2BUA):

```
┌──────────────────────────────────────────────────────────────────────────┐
│                    Two Active Sessions (A and B)                         │
│                 Both in Active state with media flowing                  │
└─────────────────────────────────┬────────────────────────────────────────┘
                                  │
                                  ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                      API Bridge Request                                   │
│           coordinator.bridge_sessions(session_a, session_b)              │
└─────────────────────────────────┬────────────────────────────────────────┘
                                  │
                                  ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                   State Machine (Session A)                               │
│  1. Process EventType::InitiateBridge                                   │
│  2. Transition: Active → BridgeInitiating                              │
│  3. Actions:                                                            │
│      - CreateBridge(session_b)                                          │
│      - SetupMediaBridge                                                 │
└─────────────────────────────────┬────────────────────────────────────────┘
                                  │
                                  ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                      Media Bridge Setup                                   │
│  4. MediaAdapter creates bridge between RTP sessions                    │
│  5. Redirects media flows:                                              │
│      - Session A RTP → Session B                                        │
│      - Session B RTP → Session A                                        │
│  6. Publishes MediaToSessionEvent::BridgeEstablished                   │
└─────────────────────────────────┬────────────────────────────────────────┘
                                  │
                                  ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                   State Update (Both Sessions)                            │
│  7. Session A: BridgeInitiating → BridgeActive                         │
│  8. Session B: Active → BridgeActive                                   │
│  9. Both sessions now bridged with direct media flow                   │
└──────────────────────────────────────────────────────────────────────────┘
```

### Bridge Architecture

```
Before Bridge:
┌─────────┐      SIP       ┌─────────┐      SIP       ┌─────────┐
│ User A  │◄─────────────►│ Session │◄─────────────►│ User B  │
└────┬────┘               │    A    │                └────┬────┘
     │                    └─────────┘                     │
     │         RTP             │              RTP         │
     └─────────────────────────┴──────────────────────────┘

After Bridge:
┌─────────┐      SIP       ┌─────────┐      Bridge    ┌─────────┐
│ User A  │◄─────────────►│ Session │◄═══════════►│ Session │
└────┬────┘               │    A    │              │    B    │
     │                    └────┬────┘              └────┬────┘
     │         RTP             │                        │
     └─────────────────────────┴────────────────────────┘
                          (Direct media flow)
```

### Detailed Sequence

```
Coordinator    SessionA    MediaAdapter    SessionB    EventBus
     │            │             │             │           │
     │ bridge()   │             │             │           │
     ├───────────>│             │             │           │
     │            │             │             │           │
     │            │ InitBridge  │             │           │
     │            ├────────────>│             │           │
     │            │             │             │           │
     │            │             │ Setup Bridge│           │
     │            │             ├────────────>│           │
     │            │             │             │           │
     │            │             │ Redirect    │           │
     │            │             │ Media       │           │
     │            │             ├─────────────┤           │
     │            │             │             │           │
     │            │             │      Publish Event      │
     │            │             ├─────────────────────────>
     │            │             │             │           │
     │            │ BridgeActive│             │           │
     │            │<────────────┼─────────────┼───────────│
     │            │             │             │           │
     │            │             │             │BridgeActive│
     │            │             │             │<──────────│
     │            │             │             │           │
```

### Key Points
1. **Both Sessions Required**: Both must be in Active state
2. **Media Redirection**: RTP flows directly between endpoints
3. **State Synchronization**: Both sessions transition to BridgeActive
4. **Transparent to Users**: No SIP signaling changes needed

## Event Types and Routing

### Event Flow Layers

1. **Protocol Events** (SIP/RTP specific)
   - Raw protocol messages
   - Handled by dialog-core/media-core

2. **Cross-Crate Events** (Standardized)
   - Type-safe contracts between crates
   - Routed via GlobalEventCoordinator
   - Examples: `DialogToSessionEvent`, `MediaToSessionEvent`

3. **State Machine Events** (Business logic)
   - Protocol-agnostic event types
   - Processed by state machine
   - Examples: `MakeCall`, `AcceptCall`, `HangupCall`

### Event Routing Matrix

| Source | Event Type | Handler | Destination |
|--------|-----------|---------|-------------|
| Dialog-Core | IncomingCall | SessionEventHandler | State Machine |
| Dialog-Core | CallProgress | SessionEventHandler | State Machine |
| Media-Core | MediaReady | SessionEventHandler | State Machine |
| API | MakeCall | State Machine | Dialog/Media Adapters |
| State Machine | SendINVITE | DialogAdapter | Dialog-Core |
| State Machine | StartMedia | MediaAdapter | Media-Core |

### Event Publishing Flow

```rust
// 1. Dialog-core publishes event
global_coordinator.publish(Arc::new(
    RvoipCrossCrateEvent::DialogToSession(
        DialogToSessionEvent::IncomingCall { ... }
    )
)).await?;

// 2. Global coordinator routes to handlers
for handler in registered_handlers {
    handler.handle(event.clone()).await?;
}

// 3. SessionEventHandler processes
match event {
    DialogToSessionEvent::IncomingCall { ... } => {
        // Create session
        // Convert to state machine event
        // Process through state machine
    }
}

// 4. State machine executes actions
for action in transition.actions {
    event_router.execute_action(session_id, action).await?;
}
```

## Common Event Patterns

### 1. **Request-Response Pattern**
```
External → Protocol Layer → CrossCrateEvent → State Machine → Action → Protocol Layer → External
```

Example: INVITE → IncomingCall → Ringing state → Send 180 → 180 Ringing

### 2. **Notification Pattern**
```
Protocol Layer → CrossCrateEvent → State Machine → State Update → Event Publication
```

Example: RTP established → MediaReady → Update conditions → Publish MediaEstablished

### 3. **Command Pattern**
```
API → State Machine → Action → Protocol Adapter → CrossCrateEvent → Protocol Layer
```

Example: make_call() → MakeCall event → SendINVITE action → Dialog-core → SIP INVITE

### 4. **Bridging Pattern**
```
API → State Machine A → Media Adapter → State Machine B → Synchronized State
```

Example: bridge() → InitiateBridge → SetupMediaBridge → Both sessions BridgeActive

## Best Practices

1. **Event Handler Isolation**: Only SessionEventHandler creates sessions for incoming calls
2. **No Direct Channels**: All cross-crate communication via GlobalEventCoordinator
3. **State Machine Authority**: All state changes go through state machine
4. **Action Routing**: EventRouter executes all actions from state transitions
5. **Type Safety**: Use strongly-typed CrossCrateEvent enums
6. **Single Responsibility**: Each adapter handles one protocol
7. **Traceable Flow**: Events can be traced from source to destination

## Debugging Event Flows

To trace an event through the system:

1. **Enable debug logging**:
   ```
   RUST_LOG=rvoip_session_core_v2=debug,rvoip_infra_common=debug
   ```

2. **Key log points**:
   - "Publishing event type: X" - GlobalEventCoordinator
   - "Handling DialogToSession event: X" - SessionEventHandler  
   - "Routing event X for session Y" - EventRouter
   - "Processing event X in state Y" - State Machine
   - "Executing action X" - Action execution

3. **Event correlation**:
   - Session ID links all related events
   - Dialog ID maps to Session ID
   - Call-ID for SIP correlation

## Summary

The session-core-v2 event system provides:
- **Clean separation** between protocols and business logic
- **Centralized routing** through GlobalEventCoordinator
- **Type-safe events** with clear contracts
- **Traceable flows** from external input to external output
- **Extensible design** for new protocols and events

All events flow through well-defined paths with clear responsibilities at each layer, making the system predictable, debuggable, and maintainable.
