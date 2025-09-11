# Event Handler Audit Results

This document lists the violations found during Phase 7 audit of event handlers.

## Principle

Event handlers should ONLY trigger state transitions via `state_machine.process_event()`. All business logic, side effects, and actions should be performed by the state machine actions, not in the event handlers.

## Violations Found

### 1. `handle_dialog_created` 
**Location**: session_event_handler.rs:230-273

**Current Behavior**:
- Directly maps dialog to session via `dialog_adapter.map_dialog()` 
- Publishes `StoreDialogMapping` event
- Makes decisions about whether event is "ours"

**Fix Required**:
- Should only call `state_machine.process_event()` with a new `EventType::DialogCreated`
- State machine action should handle the mapping

### 2. `handle_incoming_call`
**Location**: session_event_handler.rs:276-360

**Current Behavior**:
- Creates new session directly via `state_machine.store.create_session()`
- Maps dialog via `registry.map_dialog()`
- Sends `IncomingCallInfo` via channel
- Stores transaction info in dialog adapter

**Fix Required**:
- Should only call `state_machine.process_event()` with `EventType::IncomingCall`
- State machine action should create session and handle all setup

### 3. `handle_call_established`
**Location**: session_event_handler.rs:361-391

**Current Behavior**:
- Directly updates session store with remote SDP
- Makes decisions about event processing order

**Fix Required**:
- Should pass SDP in the event data to state machine
- State machine action should store the SDP

## Compliant Handlers

The following handlers correctly only trigger state transitions:
- `handle_call_state_changed`
- `handle_call_terminated`
- `handle_dialog_error`
- `handle_media_stream_started`
- `handle_media_stream_stopped`
- `handle_media_flow_established`
- `handle_media_error`
- All new Phase 3 event handlers

## Recommendations

1. Add missing `EventType::DialogCreated` variant
2. Move session creation logic to state machine actions
3. Ensure all data needed by actions is passed through event parameters
4. Remove all direct adapter/store access from event handlers
