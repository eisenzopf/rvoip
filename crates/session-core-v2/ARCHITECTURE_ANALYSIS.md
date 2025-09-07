# Session-Core-V2 Architecture Analysis

## Current Event Flow Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                          INCOMING CALL FLOW (CURRENT)                        │
└─────────────────────────────────────────────────────────────────────────────┘

1. SIP INVITE arrives at network
                │
                ▼
2. dialog-core TransportManager receives packet
                │
                ▼
3. dialog-core TransactionManager processes
                │
                ▼
4. dialog-core publishes SessionCoordinationEvent::IncomingCall
                │
                ▼
5. DialogAdapter::handle_session_event() receives event
                │
                ├──► Creates NEW SessionId (❌ PROBLEM: Too early!)
                ├──► Stores dialog_id -> session_id mapping
                ├──► Stores incoming request for later
                └──► Publishes DialogToSessionEvent::IncomingCall via GlobalEventCoordinator
                            │
                            ▼
6. Two handlers receive this event:
   
   a) SessionCrossCrateEventHandler (ACTIVE PATH)
                │
                ├──► Parses event string (❌ PROBLEM: String parsing!)
                ├──► Extracts session_id, from
                ├──► Converts to EventType::IncomingCall
                └──► Sends to StateMachine.process_event()
                            │
                            ▼
      StateMachine processes event:
                ├──► Updates session state
                ├──► Executes actions
                └──► Event dies here (❌ PROBLEM: No user notification!)
   
   b) CallControllerEventHandler (BROKEN PATH)
                │
                ├──► Would call CallController.handle_incoming_invite()
                ├──► But doesn't have dialog_id (❌ PROBLEM: Missing data!)
                └──► Can't work properly

7. Test code waits forever:
   SessionManager.incoming_calls() -> Creates disconnected channel (❌ PROBLEM!)
```

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                          OUTGOING CALL FLOW (CURRENT)                        │
└─────────────────────────────────────────────────────────────────────────────┘

1. User calls SessionManager.make_call() or CallController.make_call()
                │
                ▼
2. Creates SessionId and SessionState
                │
                ▼
3. Sends EventType::MakeCall to StateMachine
                │
                ▼
4. StateMachine looks up transition in StateTable
                │
                ▼
5. Executes Action::SendINVITE
                │
                ▼
6. Action calls DialogAdapter.send_invite()
                │
                ▼
7. DialogAdapter:
   ├──► Creates UAC dialog in dialog-core
   ├──► Stores session_id -> dialog_id mapping
   └──► Sends SIP INVITE via dialog-core
                │
                ▼
8. Response arrives from network
                │
                ▼
9. dialog-core publishes SessionCoordinationEvent::ResponseReceived
                │
                ▼
10. DialogAdapter.handle_session_event():
    ├──► Looks up session_id from dialog_id
    ├──► Converts to EventType (Dialog180Ringing, Dialog200OK, etc.)
    └──► Publishes via GlobalEventCoordinator
                │
                ▼
11. SessionCrossCrateEventHandler:
    ├──► Receives event
    ├──► Sends to StateMachine
    └──► StateMachine transitions state
```

## State Machine Action Execution

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        STATE MACHINE ACTION FLOW                             │
└─────────────────────────────────────────────────────────────────────────────┘

StateMachine.process_event(session_id, event)
                │
                ▼
1. Load SessionState from SessionStore
                │
                ▼
2. Build StateKey{role, state, event}
                │
                ▼
3. Lookup transition in StateTable
                │
                ▼
4. Evaluate Guards (if any)
                │
                ▼
5. Execute Actions sequentially:
   
   ┌──────────────────────────────────────┐
   │         DIALOG ACTIONS               │
   ├──────────────────────────────────────┤
   │ SendINVITE:                          │
   │   └─> DialogAdapter.send_invite()   │
   │                                      │
   │ Send200OK:                           │
   │   └─> DialogAdapter.send_response() │
   │                                      │
   │ SendACK:                             │
   │   └─> DialogAdapter.send_ack()      │
   │                                      │
   │ SendBYE:                             │
   │   └─> DialogAdapter.send_bye()      │
   └──────────────────────────────────────┘
   
   ┌──────────────────────────────────────┐
   │         MEDIA ACTIONS                │
   ├──────────────────────────────────────┤
   │ CreateMediaSession:                  │
   │   └─> MediaAdapter.create_session() │
   │                                      │
   │ StartMedia:                          │
   │   └─> MediaAdapter.start_stream()   │
   │                                      │
   │ StopMedia:                           │
   │   └─> MediaAdapter.stop_stream()    │
   │                                      │
   │ UpdateMedia:                         │
   │   └─> MediaAdapter.update_codec()   │
   └──────────────────────────────────────┘
   
6. Update SessionState with new state
                │
                ▼
7. Store in SessionStore
                │
                ▼
8. Publish state change event
```

## Component Interaction Map

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                          COMPONENT DEPENDENCIES                              │
└─────────────────────────────────────────────────────────────────────────────┘

                           UnifiedCoordinator
                                  │
                ┌─────────────────┼─────────────────┐
                │                 │                 │
                ▼                 ▼                 ▼
        SessionManager     CallController    ConferenceManager
                │                 │                 │
                │                 │                 │
                └────────┬────────┴────────┬────────┘
                         │                 │
                         ▼                 ▼
                  SessionRegistry    SignalingInterceptor
                         │                 │
                         │                 │ (❌ UNUSED!)
                ┌────────┴────────┬────────┴────────┐
                │                 │                 │
                ▼                 ▼                 ▼
         StateMachine      DialogAdapter      MediaAdapter
                │                 │                 │
                │                 │                 │
                ▼                 ▼                 ▼
         SessionStore       dialog-core       media-core
                            (External)         (External)

                           GlobalEventCoordinator
                                  │
                ┌─────────────────┼─────────────────┐
                │                 │                 │
                ▼                 ▼                 ▼
    SessionCrossCrateHandler  CallControllerHandler  (Others)
         (ACTIVE)              (BROKEN)
```

## Problems Identified

### 1. Multiple Parallel Paths
```
Problem: Two event handlers trying to process the same events
- SessionCrossCrateEventHandler -> StateMachine (works but no user notification)
- CallControllerEventHandler -> CallController (broken, missing data)

Impact: Confusion about which path is authoritative
```

### 2. Session Creation Too Early
```
Problem: DialogAdapter creates SessionId immediately on IncomingCall
- Should wait for application to accept/reject
- SignalingInterceptor never gets to make decision
- CallController is bypassed entirely

Impact: No call screening, no application control
```

### 3. Channel Type Mismatch
```
Problem: Using mpsc channels where broadcast is needed
- SessionManager.incoming_calls() creates new channel each time
- Multiple listeners can't receive same events
- Tests create disconnected receivers

Impact: Tests hang forever waiting for events
```

### 4. String-Based Event Parsing
```
Problem: SessionCrossCrateEventHandler parses debug strings
- Fragile and error-prone
- Loses type safety
- Can't extract all needed fields

Impact: Data loss, brittle code
```

### 5. Circular Dependencies
```
Problem: Components need bidirectional references
- DialogAdapter needs CallController (doesn't have it)
- CallController has DialogAdapter (works)
- Can't wire properly without complex initialization

Impact: Can't route events properly
```

## Simplified Architecture Proposal

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        SIMPLIFIED EVENT FLOW                                 │
└─────────────────────────────────────────────────────────────────────────────┘

                        External Events (SIP, RTP)
                                  │
                                  ▼
                        GlobalEventCoordinator
                                  │
                    ┌─────────────┴─────────────┐
                    │                           │
                    ▼                           ▼
            Signaling Events              Media Events
                    │                           │
                    ▼                           ▼
            CallController                MediaController
            (Single entry point)          (Single entry point)
                    │                           │
                    ├──► SignalingInterceptor   │
                    │    (Pre-process)          │
                    │                           │
                    └─────────┬─────────────────┘
                              │
                              ▼
                        StateMachine
                    (Central coordinator)
                              │
                    ┌─────────┴─────────┐
                    │                   │
                    ▼                   ▼
            Dialog Actions         Media Actions
                    │                   │
                    ▼                   ▼
            dialog-core            media-core
```

### Key Simplifications:

1. **Single Entry Point for Events**
   - All dialog events go through CallController first
   - All media events go through MediaController first
   - No parallel competing paths

2. **Broadcast Channels Everywhere**
   - Replace all mpsc with broadcast
   - Multiple listeners supported naturally
   - No disconnected channels

3. **Late Session Creation**
   - Don't create SessionId until call is accepted
   - SignalingInterceptor makes the decision
   - Application has control

4. **Type-Safe Events**
   - No string parsing
   - Proper enums with all data
   - Compile-time safety

5. **Clear Ownership**
   - CallController owns signaling flow
   - MediaController owns media flow
   - StateMachine owns state transitions
   - No circular dependencies

## Optimization Opportunities

### 1. Remove Redundant Handlers
```
Current: 
- SessionCrossCrateEventHandler
- CallControllerEventHandler
- Both trying to handle same events

Optimized:
- Single handler per event type
- Clear routing rules
```

### 2. Eliminate String Parsing
```
Current:
- Parse debug strings to extract event data
- Fragile regex/string operations

Optimized:
- Strongly typed events
- Direct field access
```

### 3. Reduce Mapping Layers
```
Current:
- SessionId -> DialogId mapping
- DialogId -> SessionId mapping
- CallId -> SessionId mapping
- Multiple DashMaps

Optimized:
- Single source of truth
- SessionRegistry holds all mappings
```

### 4. Consolidate Event Types
```
Current:
- SessionEvent
- EventType
- DialogToSessionEvent
- SessionCoordinationEvent
- StateMachineEvent
- Multiple overlapping types

Optimized:
- Single Event enum
- Clear variants for each source
```

### 5. Simplify Component Creation
```
Current:
- Complex initialization order
- Circular dependency management
- Late binding of references

Optimized:
- Linear initialization
- No circular deps
- All wiring at creation time
```

## Recommended Changes

### Phase 1: Fix Immediate Issues (Minimal Changes)
1. Change SessionManager.incoming_calls() to use broadcast channel
2. Have DialogAdapter send to this broadcast channel
3. Remove broken CallControllerEventHandler
4. Tests use broadcast receiver

### Phase 2: Architectural Cleanup
1. Make CallController the single entry point for signaling
2. Move session creation to SignalingInterceptor
3. Remove redundant event handlers
4. Consolidate event types

### Phase 3: Optimization
1. Replace string parsing with proper types
2. Consolidate mapping layers
3. Simplify initialization
4. Add proper error handling

## State Machine Role

The StateMachine should be the central coordinator that:
1. Receives events from Controllers
2. Looks up transitions in StateTable
3. Evaluates guards
4. Executes actions via Adapters
5. Updates state
6. Publishes state changes

It should NOT:
- Create sessions
- Handle raw network events
- Make policy decisions
- Manage channels

## Event Flow Summary

### Incoming Call (Simplified)
```
1. SIP INVITE -> dialog-core
2. dialog-core -> GlobalEventCoordinator
3. GlobalEventCoordinator -> CallController
4. CallController -> SignalingInterceptor (decision point)
5. If accepted:
   - Create SessionId
   - Send to broadcast channel
   - Application receives via CallController.get_incoming_call()
6. Application accepts/rejects
7. CallController -> StateMachine
8. StateMachine executes appropriate actions
```

### Outgoing Call (Already Clean)
```
1. Application -> CallController.make_call()
2. CallController creates session
3. CallController -> StateMachine
4. StateMachine executes SendINVITE action
5. Action -> DialogAdapter -> dialog-core
6. Responses flow back through same path
```

## Conclusion

The architecture is overly complex with multiple parallel paths trying to accomplish the same thing. The core issue is that DialogAdapter and CallController are both trying to manage incoming calls, leading to disconnected flows.

The fix is straightforward:
1. Use broadcast channels (as you specified)
2. Make CallController the authoritative entry point
3. Remove redundant handlers
4. Simplify event types

This will make the library much easier to understand and maintain.