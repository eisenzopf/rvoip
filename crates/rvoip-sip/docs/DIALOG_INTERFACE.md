# Dialog Interface

This document describes the interface between session-core-v2 and dialog-core, clarifying the boundaries and communication patterns in the GlobalEventCoordinator-based architecture.

## Overview

The dialog interface operates on two primary communication channels:
1. **Direct Calls** - Synchronous method calls from session-core to dialog-core
2. **Events** - Asynchronous events from dialog-core to session-core via GlobalEventCoordinator

## Direct Calls (Session → Dialog)

These are methods that session-core calls directly on the DialogAdapter, which internally communicates with dialog-core:

### Core Operations

#### `send_invite_with_details(session_id, from, to, sdp_offer)`
- **Purpose**: Initiate an outbound call (UAC)
- **When Used**: MakeCall event in state machine
- **Parameters**:
  - `session_id`: Unique session identifier
  - `from`: Caller URI (e.g., "sip:alice@example.com")
  - `to`: Callee URI (e.g., "sip:bob@example.com")
  - `sdp_offer`: Optional SDP for media negotiation
- **Returns**: DialogId for tracking the dialog
- **Side Effects**: Creates dialog, sends INVITE, publishes DialogCreated event

#### `send_response(dialog_id, status_code, reason, sdp)`
- **Purpose**: Send SIP response (180 Ringing, 200 OK, etc.)
- **When Used**: AcceptCall, RejectCall events
- **Parameters**:
  - `dialog_id`: Dialog to respond to
  - `status_code`: SIP response code (180, 200, 486, etc.)
  - `reason`: Response reason phrase
  - `sdp`: Optional SDP answer (for 200 OK)
- **Returns**: Result<()>

#### `send_bye(dialog_id)`
- **Purpose**: Terminate an established call
- **When Used**: HangupCall event in Active state
- **Parameters**:
  - `dialog_id`: Dialog to terminate
- **Returns**: Result<()>
- **Side Effects**: Sends BYE, publishes CallTerminated event

#### `send_cancel(dialog_id)`
- **Purpose**: Cancel a call attempt
- **When Used**: HangupCall event in Initiating/Ringing states
- **Parameters**:
  - `dialog_id`: Dialog to cancel
- **Returns**: Result<()>

#### `send_ack(dialog_id)`
- **Purpose**: Acknowledge a 200 OK response
- **When Used**: After receiving 200 OK
- **Parameters**:
  - `dialog_id`: Dialog to acknowledge
- **Returns**: Result<()>

#### `send_update(dialog_id, sdp)`
- **Purpose**: Send re-INVITE for media update
- **When Used**: Media renegotiation scenarios
- **Parameters**:
  - `dialog_id`: Dialog to update
  - `sdp`: New SDP offer
- **Returns**: Result<()>

## Events (Dialog → Session)

These events are published by dialog-core through the GlobalEventCoordinator and handled by session-core's SessionCrossCrateEventHandler:

### Call Lifecycle Events

#### `DialogToSessionEvent::IncomingCall`
- **When**: New INVITE received
- **Fields**:
  - `session_id`: Generated session ID (from dialog perspective)
  - `call_id`: SIP Call-ID header
  - `from`: Caller URI
  - `to`: Callee URI
  - `sdp_offer`: Remote SDP offer
  - `headers`: Additional SIP headers
  - `transaction_id`: For sending responses
  - `source_addr`: Remote address
- **Expected Action**: Create session, send 180 Ringing

#### `DialogToSessionEvent::CallEstablished`
- **When**: Call successfully connected (ACK received)
- **Fields**:
  - `session_id`: Session identifier
  - `sdp_answer`: Negotiated SDP (if any)
- **Expected Action**: Transition to Active state

#### `DialogToSessionEvent::CallStateChanged`
- **When**: Dialog state changes (Ringing, etc.)
- **Fields**:
  - `session_id`: Session identifier
  - `new_state`: New call state
  - `reason`: Optional reason for change
- **Expected Action**: Update state machine

#### `DialogToSessionEvent::CallTerminated`
- **When**: Call ends (BYE received/sent)
- **Fields**:
  - `session_id`: Session identifier
  - `reason`: Termination reason
- **Expected Action**: Cleanup resources, transition to Terminated

### Dialog Management Events

#### `DialogToSessionEvent::DialogCreated`
- **When**: New dialog created (outbound calls)
- **Fields**:
  - `dialog_id`: Created dialog ID
  - `call_id`: SIP Call-ID
- **Expected Action**: Map dialog to session

#### `DialogToSessionEvent::DialogStateChanged`
- **When**: Dialog FSM state changes
- **Fields**:
  - `session_id`: Session identifier
  - `old_state`: Previous dialog state
  - `new_state`: New dialog state
- **Expected Action**: Log/track state change

#### `DialogToSessionEvent::DialogError`
- **When**: SIP protocol error occurs
- **Fields**:
  - `session_id`: Session identifier
  - `error`: Error description
  - `error_code`: Optional SIP error code
- **Expected Action**: Handle error, possibly terminate

### Advanced Call Features

#### `DialogToSessionEvent::ReinviteReceived`
- **When**: Re-INVITE received from remote
- **Fields**:
  - `session_id`: Session identifier
  - `sdp`: New SDP offer
- **Expected Action**: Renegotiate media

#### `DialogToSessionEvent::TransferRequested`
- **When**: REFER received for call transfer
- **Fields**:
  - `session_id`: Session identifier
  - `refer_to`: Transfer target
  - `transfer_type`: Blind/Attended
- **Expected Action**: Handle transfer logic

#### `DialogToSessionEvent::DtmfReceived`
- **When**: DTMF tones received (INFO/RFC2833)
- **Fields**:
  - `session_id`: Session identifier
  - `tones`: DTMF digits received
- **Expected Action**: Process DTMF input

## Important Design Principles

1. **No Direct Channel Communication**: All events flow through GlobalEventCoordinator
2. **Session-Core Drives**: Dialog-core is reactive; session-core initiates all actions
3. **Atomic Operations**: Each direct call is self-contained and atomic
4. **Event-Driven Responses**: Dialog-core never calls session-core directly
5. **Clear Ownership**: Session-core owns sessions, dialog-core owns dialogs

## Error Handling

- Direct calls return `Result<T>` for immediate errors
- Asynchronous errors are reported via `DialogError` events
- Network/protocol errors trigger appropriate state transitions

## Example Flow: Outbound Call

```
1. Session-Core: send_invite_with_details() → Dialog-Core
2. Dialog-Core: → DialogCreated event → Session-Core
3. Dialog-Core: (sends INVITE over network)
4. Dialog-Core: (receives 180 Ringing)
5. Dialog-Core: → CallStateChanged(Ringing) → Session-Core
6. Dialog-Core: (receives 200 OK)
7. Dialog-Core: → CallEstablished → Session-Core
8. Session-Core: send_ack() → Dialog-Core
```

## Example Flow: Inbound Call

```
1. Dialog-Core: (receives INVITE from network)
2. Dialog-Core: → IncomingCall event → Session-Core
3. Session-Core: send_response(180) → Dialog-Core
4. Session-Core: send_response(200) → Dialog-Core
5. Dialog-Core: (receives ACK)
6. Dialog-Core: → CallEstablished → Session-Core
```
