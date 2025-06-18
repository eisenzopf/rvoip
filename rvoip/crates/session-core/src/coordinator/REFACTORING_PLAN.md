# SessionCoordinator Refactoring Plan

## Overview

The current `mod.rs` file contains 1174 lines and handles too many responsibilities. This refactoring will break it into 6 focused modules, each with a clear single responsibility.

## Current State

- **File**: `mod.rs`
- **Lines**: 1174
- **Responsibilities**: 
  - Core coordinator structure and initialization
  - Event handling and processing
  - Session management operations
  - Bridge/conference management
  - SIP client implementation
  - Media coordination

## Target Architecture

```
coordinator/
├── mod.rs                  # Module exports only (~50 lines)
├── coordinator.rs          # Core structure (~200 lines)
├── event_handler.rs        # Event processing (~250 lines)
├── session_ops.rs          # Session operations (~150 lines)
├── bridge_ops.rs           # Bridge management (~200 lines)
├── sip_client.rs          # SIP client implementation (~300 lines)
└── REFACTORING_PLAN.md    # This document
```

## Detailed Module Breakdown

### 1. `mod.rs` - Module Root
**Purpose**: Public API and module organization  
**Size**: ~50 lines

**Contents**:
```rust
// Module declarations
mod coordinator;
mod event_handler;
mod session_ops;
mod bridge_ops;
mod sip_client;

// Re-exports
pub use coordinator::SessionCoordinator;
// Any other public types that need to be exposed
```

### 2. `coordinator.rs` - Core Structure
**Purpose**: Core SessionCoordinator struct and initialization  
**Size**: ~200 lines

**Contents**:
- `SessionCoordinator` struct definition with all fields
- `impl SessionCoordinator`:
  - `new()` - Create and initialize the system
  - `initialize()` - Initialize all subsystems
  - `start()` - Start all subsystems
  - `stop()` - Stop all subsystems
  - `get_bound_address()` - Get the bound socket address
  - Private helpers:
    - `start_media_session()`
    - `stop_media_session()`
- `Debug` implementation for SessionCoordinator

**Key Imports**:
```rust
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use crate::api::*;
use crate::errors::{Result, SessionError};
use crate::manager::*;
use crate::dialog::*;
use crate::media::*;
use crate::conference::*;
```

### 3. `event_handler.rs` - Event Processing
**Purpose**: All event handling logic  
**Size**: ~250 lines

**Contents**:
- `impl SessionCoordinator` for event handling:
  - `run_event_loop()` - Main event processing loop
  - `handle_event()` - Event dispatcher
  - Event-specific handlers:
    - `handle_session_created()`
    - `handle_state_changed()`
    - `handle_session_terminated()`
    - `handle_media_event()`
    - `handle_sdp_event()`
    - `handle_registration_request()`

**Key Dependencies**:
- Access to coordinator fields via `self`
- SessionEvent enum and related types
- Logging/tracing

### 4. `session_ops.rs` - Session Operations
**Purpose**: Session management operations  
**Size**: ~150 lines

**Contents**:
- `impl SessionCoordinator` for session operations:
  - `create_outgoing_call()` - Create new outgoing call
  - `terminate_session()` - End a session
  - `send_dtmf()` - Send DTMF tones
  - `generate_sdp_offer()` - Generate SDP for a session
  - `create_outgoing_session()` - Pre-allocate session
  - `find_session()` - Look up session by ID
  - `list_active_sessions()` - Get all active sessions
  - `get_stats()` - Get session statistics

**Key Dependencies**:
- SessionRegistry interactions
- DialogManager interactions
- MediaManager for SDP generation

### 5. `bridge_ops.rs` - Bridge Management
**Purpose**: Bridge/conference operations  
**Size**: ~200 lines

**Contents**:
- `impl SessionCoordinator` for bridge operations:
  - `create_bridge()` - Create empty bridge
  - `bridge_sessions()` - Bridge two sessions
  - `destroy_bridge()` - Terminate a bridge
  - `add_session_to_bridge()` - Add session to existing bridge
  - `remove_session_from_bridge()` - Remove from bridge
  - `get_session_bridge()` - Find bridge for session
  - `get_bridge_info()` - Get bridge details
  - `list_bridges()` - List all active bridges
  - `subscribe_to_bridge_events()` - Event subscription
  - Private: `emit_bridge_event()` - Broadcast events

**Key Dependencies**:
- ConferenceManager for bridge implementation
- Bridge event types and subscribers

### 6. `sip_client.rs` - SIP Client Implementation
**Purpose**: SipClient trait implementation  
**Size**: ~300 lines

**Contents**:
- `use async_trait::async_trait;`
- `impl SipClient for Arc<SessionCoordinator>`:
  - `register()` - SIP REGISTER
  - `send_options()` - SIP OPTIONS
  - `send_message()` - SIP MESSAGE
  - `subscribe()` - SIP SUBSCRIBE (stub)
  - `send_raw_request()` - Send arbitrary SIP request
- Related helper from session_ops:
  - `send_sip_response()` - Send SIP response

**Key Dependencies**:
- rvoip_sip_core for request building
- DialogCoordinator for sending requests
- URI resolution utilities

## Benefits

1. **Separation of Concerns**: Each module has a single, clear responsibility
2. **Improved Maintainability**: Easier to find and modify specific functionality
3. **Better Testing**: Can unit test each module independently
4. **Reduced Complexity**: Smaller files are easier to understand
5. **Parallel Development**: Multiple developers can work on different aspects
6. **Cleaner Dependencies**: Each module only imports what it needs

## Implementation Steps

1. **Create new files**: Create empty files for each module
2. **Move coordinator core**: Extract struct and initialization to `coordinator.rs`
3. **Move event handling**: Extract all event processing to `event_handler.rs`
4. **Move session ops**: Extract session management to `session_ops.rs`
5. **Move bridge ops**: Extract bridge functionality to `bridge_ops.rs`
6. **Move SIP client**: Extract SipClient impl to `sip_client.rs`
7. **Update mod.rs**: Replace with module declarations and exports
8. **Test**: Ensure all tests still pass
9. **Update imports**: Fix any import issues in dependent modules

## Considerations

- **Visibility**: Some methods may need to be `pub(super)` for cross-module access
- **Shared State**: All modules access SessionCoordinator fields via `self`
- **Circular Dependencies**: Avoid by keeping clear module boundaries
- **Documentation**: Update module-level documentation for each file

## Success Criteria

- [ ] All functionality preserved
- [ ] All tests passing
- [ ] No public API changes
- [ ] Each file under 300 lines
- [ ] Clear module boundaries
- [ ] Improved code organization 