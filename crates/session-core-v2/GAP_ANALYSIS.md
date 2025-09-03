# Gap Analysis: session-core vs session-core-v2

## Executive Summary

The original session-core has 112 source files across 17 directories with complex interdependencies. The new session-core-v2 can be dramatically simplified to just **the state table + unified API layer**, eliminating most complexity while maintaining all functionality.

## Current Architecture Overview

### session-core (Original) - 112 Files
```
src/
├── api/           (41 files) - Public API surface
├── auth/          (4 files)  - Authentication (JWT, OAuth)
├── bridge/        (3 files)  - Call bridging
├── conference/    (8 files)  - Conference calling
├── coordination/  (4 files)  - Resource coordination
├── coordinator/   (13 files) - Core orchestration ⚡
├── dialog/        (8 files)  - Dialog-core integration
├── events/        (6 files)  - Event processing
├── manager/       (6 files)  - Session management
├── media/         (9 files)  - Media-core integration
├── planes/        (0 files)  - Control/media plane separation
├── sdp/           (3 files)  - SDP handling
└── session/       (5 files)  - Session state
```

### session-core-v2 (New) - Target: ~20 Files
```
src/
├── api/           (5-10 files) - Simplified unified API
├── state_table/   (4 files)    - Core state machine ⚡
├── state_machine/ (4 files)    - Execution engine
├── session_store/ (3 files)    - Runtime state
├── adapters/      (2 files)    - Dialog/media adapters
└── lib.rs
```

## Detailed Component Analysis

### 1. ✅ COMPLETED - Core Coordination (coordinator/)

**Original**: 13 files, 2,220+ lines of imperative logic
- `event_handler.rs` (1,479 lines) - Complex event processing
- `coordinator.rs` (741 lines) - Orchestration logic
- Multiple auxiliary files for specific features

**New**: State table + executor (979 lines total)
- State tables define all transitions declaratively
- Generic executor processes any event
- **Status: ✅ COMPLETE - 56% reduction**

### 2. 🔄 NEEDS SIMPLIFICATION - API Layer (api/)

**Original**: 41 files with overlapping responsibilities
```
Key Components:
- types.rs          - Core types (SessionId, CallState, etc.)
- control.rs        - Session lifecycle control
- media.rs          - Media control interface
- call.rs           - SimpleCall implementation
- peer.rs           - SimplePeer implementation
- b2bua.rs          - B2BUA implementation
- client.rs         - SIP client functionality
- server.rs         - Server management
- bridge.rs         - Bridging API
- handlers.rs       - Event handlers
- notifications.rs  - Callback interfaces
- builder.rs        - Configuration builders
+ 28 more files...
```

**Proposed Simplification**:
```rust
// Consolidate into 5-6 files:
api/
├── mod.rs          - Public exports
├── types.rs        - All types (merge from 8+ files)
├── session.rs      - Unified session API (merge control + call + peer)
├── media.rs        - Media control (simplified)
├── config.rs       - All configuration (merge builder + server_types)
└── handlers.rs     - Event callbacks (merge handlers + notifications)
```

### 3. 🚫 ELIMINATE - Conference System (conference/)

**Original**: 8 files, ~1,500 lines
- Full conference room management
- Participant tracking
- Media mixing coordination

**Recommendation**: **ELIMINATE from v2**
- Conference calling is a specialized feature
- Should be a separate crate that uses session-core-v2
- Removing saves ~1,500 lines

### 4. 🚫 ELIMINATE - Authentication (auth/)

**Original**: 4 files (JWT, OAuth, types)

**Recommendation**: **ELIMINATE from v2**
- Authentication belongs in a higher layer
- Should be handled by the application using session-core-v2
- Removing saves ~400 lines

### 5. 🚫 ELIMINATE - Bridge Operations (bridge/)

**Original**: 3 files for call bridging

**Recommendation**: **MOVE TO STATE TABLE**
- Bridging is just another state transition
- Add bridge states to CallState enum
- Add bridge actions to state table
- Eliminates 3 files (~300 lines)

### 6. 🔄 SIMPLIFY - Dialog Integration (dialog/)

**Original**: 8 files for dialog-core integration

**New**: Already simplified to 1 adapter file
- `adapters/dialog_adapter.rs` - Thin adapter layer
- **Status: ✅ Simplified from 8 files to 1**

### 7. 🔄 SIMPLIFY - Media Integration (media/)

**Original**: 9 files including stats, config, types

**New**: Already simplified to 1 adapter file
- `adapters/media_adapter.rs` - Thin adapter layer
- Media statistics can be queried through adapter
- **Status: ✅ Simplified from 9 files to 1**

### 8. 🚫 ELIMINATE - Coordination Layer (coordination/)

**Original**: 4 files (groups, priority, resources)

**Recommendation**: **ELIMINATE**
- Resource coordination is handled by state machine
- Priority handled by event queue
- Groups not needed in simplified design
- Saves ~500 lines

### 9. 🔄 MERGE - Manager Layer (manager/)

**Original**: 6 files including events, registry, cleanup

**New**: Merge into state machine
- Events → Already in state_table/types.rs
- Registry → session_store handles this
- Cleanup → Part of state transitions
- **Can eliminate entire directory**

### 10. 🚫 ELIMINATE - SDP Module (sdp/)

**Original**: 3 files for SDP handling

**Recommendation**: **USE EXTERNAL CRATE**
- SDP parsing/generation should use rvoip-sdp-core
- No need for session-specific SDP logic
- Saves ~400 lines

### 11. 🔄 SIMPLIFY - Session Module (session/)

**Original**: 5 files for session management

**New**: Already merged into session_store
- All session state in `session_store/state.rs`
- **Status: ✅ Merged**

### 12. 🚫 ELIMINATE - Events Module (events/)

**Original**: 6 files for event processing

**New**: Events are part of state table
- All events defined in `state_table/types.rs`
- Event processing is the state machine
- **Can eliminate entire directory**

## Simplification Opportunities

### 1. Unified Session API
Instead of SimpleCall, SimplePeer, SimpleB2BUA:
```rust
pub struct UnifiedSession {
    id: SessionId,
    coordinator: Arc<SessionCoordinator>,
}

impl UnifiedSession {
    // All call operations through state machine
    pub async fn make_call(&self, target: &str) -> Result<()> {
        self.send_event(EventType::MakeCall)
    }
    
    pub async fn accept(&self) -> Result<()> {
        self.send_event(EventType::AcceptCall)
    }
    
    // Works for any role (UAC, UAS, B2BUA)
    fn send_event(&self, event: EventType) -> Result<()> {
        self.coordinator.process_event(&self.id, event)
    }
}
```

### 2. Configuration Consolidation
Merge all builders and configs:
```rust
pub struct Config {
    pub sip_port: u16,
    pub media_ports: (u16, u16),
    pub bind_addr: SocketAddr,
    // All other config in one place
}
```

### 3. Handler Simplification
One trait for all callbacks:
```rust
pub trait EventHandler {
    async fn on_event(&self, event: SessionEvent);
}
```

## Implementation Plan

### Phase 1: Core Completion ✅
- [x] State table implementation
- [x] State machine executor
- [x] Session store
- [x] Basic adapters

### Phase 2: API Simplification (Next)
1. **Create unified session API** (200 lines)
   - Merge SimpleCall, SimplePeer, SimpleB2BUA
   - All operations through state machine

2. **Consolidate types** (300 lines)
   - Merge 8+ type files into one
   - Remove duplicates

3. **Simplify configuration** (100 lines)
   - One Config struct
   - One builder

4. **Unify handlers** (100 lines)
   - One EventHandler trait
   - Remove complex callback chains

### Phase 3: Feature Addition (As Needed)
- Add bridge states to state table (20 lines)
- Add transfer states to state table (20 lines)
- Add hold/resume states (already done)

## Line Count Projection

### Current session-core
```
coordinator/    2,220 lines
api/           ~8,000 lines
conference/    ~1,500 lines
auth/           ~400 lines
bridge/         ~300 lines
dialog/         ~800 lines
media/          ~900 lines
coordination/   ~500 lines
manager/        ~600 lines
sdp/            ~400 lines
session/        ~500 lines
events/         ~600 lines
----------------------------
TOTAL:        ~16,720 lines
```

### Projected session-core-v2
```
state_table/     590 lines (done)
state_machine/   400 lines (done)
session_store/   300 lines (done)
adapters/        300 lines (done)
api/             700 lines (simplified from 8,000)
----------------------------
TOTAL:         2,290 lines (86% reduction!)
```

## Key Simplifications

1. **Everything is a state transition**
   - Bridges, transfers, conferences → just states
   - No special handling needed

2. **One session type**
   - UnifiedSession works for all scenarios
   - Role determined by state table

3. **No feature-specific modules**
   - Conference → separate crate if needed
   - Auth → application layer
   - Bridge → state table entries

4. **Minimal API surface**
   - 5-6 files instead of 41
   - Clear separation of concerns
   - Everything flows through state machine

## Benefits of Simplification

1. **Maintainability**
   - 86% less code to maintain
   - All logic in state table
   - No scattered conditions

2. **Correctness**
   - Impossible to have inconsistent state
   - All transitions validated
   - Race conditions eliminated

3. **Extensibility**
   - New features = new table entries
   - No code changes to core
   - Easy to test

4. **Performance**
   - Less code = smaller binary
   - Direct table lookups
   - No complex inheritance

## Recommendation

**Proceed with radical simplification:**

1. Keep only essential API functionality
2. Everything else through state table
3. Complex features (conference, auth) as separate crates
4. Target: Under 2,500 lines total

This makes session-core-v2 a **pure state-driven coordination layer** with minimal API surface - exactly what it should be!