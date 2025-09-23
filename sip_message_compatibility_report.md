# SIP Message Compatibility Report: dialog-core vs session-core-v2

## Executive Summary

This report analyzes the bidirectional SIP message support between `dialog-core` and `session-core-v2` crates. The analysis reveals that while there is substantial overlap in basic SIP message support, there are several gaps in both directions that need to be addressed for complete compatibility.

## SIP Messages Supported in dialog-core

### Core Protocol Handlers
The following SIP methods have dedicated handlers in dialog-core:

1. **INVITE** (`protocol/invite_handler.rs`)
   - Initial INVITE for dialog creation
   - Re-INVITE for session modification
   - Full early dialog support

2. **BYE** (`protocol/bye_handler.rs`)
   - Dialog termination
   - Mid-dialog BYE handling

3. **REGISTER** (`protocol/register_handler.rs`)
   - User registration
   - Registration refresh

4. **UPDATE** (`protocol/update_handler.rs`)
   - Session modification without re-INVITE
   - SDP negotiation

5. **CANCEL** (`protocol_handlers.rs`)
   - Transaction cancellation
   - Early dialog termination

6. **ACK** (`protocol_handlers.rs`)
   - INVITE transaction completion
   - Both 2xx and non-2xx ACK handling

7. **OPTIONS** (`protocol_handlers.rs`)
   - Capability discovery
   - Keep-alive mechanism

8. **INFO** (`protocol_handlers.rs`)
   - Application-level information exchange
   - Mid-dialog signaling

9. **REFER** (`protocol_handlers.rs`)
   - Call transfer (blind and attended)
   - Third-party call control

10. **SUBSCRIBE** (`protocol_handlers.rs`)
    - Event subscription
    - Presence and dialog state subscriptions

11. **NOTIFY** (`protocol_handlers.rs`)
    - Event notification
    - Subscription state updates

### Additional Method Support
dialog-core also has limited support for:

- **MESSAGE** - Code exists in transaction builders and quick dialog functions
- **PUBLISH** - Basic implementation in `presence/publish.rs`
- **PRACK** - Not implemented (no reliable provisional response support)

## SIP Message Handling in session-core-v2

### Event Types Mapped from Dialog Events
session-core-v2 handles the following dialog events through its event system:

1. **DialogInvite** → Mapped from INVITE
2. **Dialog180Ringing** → Mapped from 180 Ringing response
3. **Dialog183SessionProgress** → Mapped from 183 Session Progress
4. **Dialog200OK** → Mapped from 200 OK response
5. **DialogACK** → Mapped from ACK
6. **DialogBYE** → Mapped from BYE
7. **DialogCANCEL** → Mapped from CANCEL
8. **DialogREFER** → Mapped from REFER
9. **DialogReINVITE** → Mapped from re-INVITE
10. **Dialog4xxFailure** → Client error responses
11. **Dialog5xxFailure** → Server error responses
12. **Dialog6xxFailure** → Global failure responses

### Actions for Sending SIP Messages
session-core-v2 can send these SIP messages through actions:

1. **SendINVITE** - Initiate calls
2. **SendACK** - Acknowledge responses
3. **SendBYE** - Terminate calls
4. **SendCANCEL** - Cancel pending INVITEs
5. **SendReINVITE** - Modify sessions
6. **SendSIPResponse** - Send numeric responses
7. **SendREFER** - Transfer calls
8. **SendREFERWithReplaces** - Attended transfer

### State Table References
The state tables also reference but don't fully implement:

- **MESSAGE** support (instant messaging)
- **PUBLISH** support (presence)
- **PRACK** support (reliable provisional responses)
- **SUBSCRIBE/NOTIFY** (limited implementation)

## Gap Analysis

### Messages in dialog-core NOT properly handled in session-core-v2

1. **REGISTER**
   - dialog-core has full handler
   - session-core-v2 has no event mapping or state transitions

2. **UPDATE** 
   - dialog-core has dedicated handler
   - session-core-v2 has no DialogUPDATE event or SendUPDATE action

3. **OPTIONS**
   - dialog-core has handler with auto-response capability
   - session-core-v2 has no event mapping

4. **INFO**
   - dialog-core forwards to session layer
   - session-core-v2 has no DialogINFO event

5. **SUBSCRIBE/NOTIFY**
   - dialog-core has subscription manager
   - session-core-v2 has limited event mapping

### Messages referenced in session-core-v2 NOT fully supported in dialog-core

1. **MESSAGE** (Instant Messaging)
   - session-core-v2 state tables define SendMESSAGE action
   - dialog-core has partial support but no protocol handler

2. **PUBLISH** (Presence)
   - session-core-v2 state tables define SendPUBLISH action
   - dialog-core has basic implementation but no protocol handler

3. **PRACK** (Reliable Provisional Responses)
   - session-core-v2 state tables reference PRACK
   - dialog-core has no implementation

## Recommendations

### High Priority (Core Functionality)

1. **Add UPDATE support to session-core-v2**
   - Add `DialogUPDATE` event type
   - Add `SendUPDATE` action
   - Add state transitions for session modification

2. **Add REGISTER support to session-core-v2**
   - Add `DialogREGISTER` event type
   - Add registration state management
   - Add periodic re-registration logic

3. **Add OPTIONS support to session-core-v2**
   - Add `DialogOPTIONS` event type
   - Add capability response handling

4. **Add INFO support to session-core-v2**
   - Add `DialogINFO` event type
   - Add application-level info handling

### Medium Priority (Extended Features)

1. **Complete MESSAGE support in dialog-core**
   - Add protocol handler for MESSAGE
   - Implement instant messaging routing

2. **Complete PUBLISH support in dialog-core**
   - Add protocol handler for PUBLISH
   - Integrate with presence system

3. **Enhance SUBSCRIBE/NOTIFY in session-core-v2**
   - Add proper event mappings
   - Add subscription state management

### Low Priority (Advanced Features)

1. **Implement PRACK support**
   - Add to dialog-core protocol handlers
   - Add to session-core-v2 event system
   - Implement reliable provisional responses

## Implementation Notes

### For dialog-core additions:
- Follow existing protocol handler pattern
- Add to `protocol/` directory
- Update `handle_request()` in `manager/core.rs`
- Add to `MethodHandler` trait if needed

### For session-core-v2 additions:
- Add event types to `EventType` enum
- Add actions to `Action` enum
- Update state tables with transitions
- Add handlers in `session_event_handler.rs`
- Update `event_router.rs` for new actions

## Conclusion

While both crates support the core SIP methods needed for basic call functionality (INVITE, BYE, CANCEL, ACK), there are significant gaps in supporting the full SIP protocol suite. The most critical gaps are:

1. Missing UPDATE support in session-core-v2 (important for session modification)
2. Missing REGISTER support in session-core-v2 (critical for SIP registration)
3. Incomplete MESSAGE and PUBLISH support in both crates
4. No PRACK support for reliable provisional responses

Addressing these gaps will ensure full bidirectional compatibility between the dialog-core signaling layer and the session-core-v2 state management layer.
