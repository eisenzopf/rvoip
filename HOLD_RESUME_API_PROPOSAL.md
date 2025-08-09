# Hold/Resume API Design Proposal

## Problem Statement

The current implementation has a mismatch between the public API and internal implementation:

1. **Public API (`CallSession`)**: Contains only basic call info (id, from, to, state, started_at)
2. **Internal `Session` struct**: Contains additional fields including `local_sdp` and `remote_sdp`
3. **Issue**: The `DialogManager` needs access to SDP data for hold/resume operations, but it only has access to the public `SessionRegistry` which stores `CallSession` objects

## Current Architecture

```
Public API Layer:
- SessionControl (trait)
- CallSession (public type)
- SessionRegistry (stores CallSession)

Internal Implementation:
- SessionCoordinator (manages multiple concurrent sessions)
- Session (internal type with SDP fields)
- DialogManager (needs SDP access)
```

## Chosen Solution: Internal Session Registry

We will implement an internal registry that stores full `Session` objects while maintaining the clean public API.

### Architecture Overview

```rust
// New file: coordinator/registry.rs
pub struct InternalSessionRegistry {
    // Stores full Session objects with all internal data
    sessions: Arc<RwLock<HashMap<SessionId, Session>>>,
    stats: Arc<RwLock<SessionRegistryStats>>,
}

impl InternalSessionRegistry {
    // Internal: Store/retrieve full Session
    pub async fn register_session(&self, session: Session) -> Result<()>
    pub async fn get_session(&self, id: &SessionId) -> Result<Option<Session>>
    
    // Public API: Convert to CallSession
    pub async fn get_public_session(&self, id: &SessionId) -> Result<Option<CallSession>> {
        self.get_session(id).await
            .map(|opt| opt.map(|s| s.into_call_session()))
    }
    
    // Update session state
    pub async fn update_session_state(&self, id: &SessionId, state: CallState) -> Result<()>
    
    // Update SDP data
    pub async fn update_session_sdp(
        &self, 
        id: &SessionId, 
        local_sdp: Option<String>,
        remote_sdp: Option<String>
    ) -> Result<()>
}
```

### Key Benefits

1. **Single Source of Truth**: One `Session` object per call contains all data
2. **Clean API Separation**: Public API only sees `CallSession`, internal components access full `Session`
3. **Multi-Session Support**: Properly handles multiple concurrent sessions
4. **Type Safety**: Clear distinction between internal and public types

## Implementation Plan

### Phase 1: Create Internal Registry
- [ ] Create `coordinator/registry.rs` with `InternalSessionRegistry`
- [ ] Implement conversion methods between `Session` and `CallSession`
- [ ] Add SDP update methods

### Phase 2: Update SessionCoordinator
- [ ] Replace `Arc<SessionRegistry>` with `Arc<InternalSessionRegistry>`
- [ ] Update all registry access to use new methods
- [ ] Ensure public API methods use `get_public_session()`

### Phase 3: Update DialogManager
- [ ] Pass `InternalSessionRegistry` to DialogManager
- [ ] Update `get_current_sdp()` to use `registry.get_session()` for full access
- [ ] Update `update_session_sdp()` to use new registry methods

### Phase 4: Fix Compilation Issues
- [ ] Update all components that access the registry
- [ ] Fix any type mismatches between `Session` and `CallSession`
- [ ] Ensure tests work with the new structure

### Phase 5: Testing
- [ ] Run existing hold/resume tests
- [ ] Verify multi-session scenarios work correctly
- [ ] Test SDP persistence across hold/resume cycles

## Migration Notes

### What Changes
- `SessionCoordinator` uses `InternalSessionRegistry` instead of `SessionRegistry`
- Internal components work with `Session` objects
- Registry provides conversion to `CallSession` for public API

### What Stays the Same
- Public API (`SessionControl`, `CallSession`)
- Test interfaces
- External callers see no changes

## Example Usage

```rust
// Internal usage (in DialogManager)
let session = self.registry.get_session(session_id).await?;
let sdp = session.local_sdp; // Full access to Session fields

// Public API usage (in SessionControl impl)
let call_session = self.registry.get_public_session(session_id).await?;
// Returns CallSession without SDP fields
```

## Status

- **Decision**: Approved
- **Priority**: High (blocking hold/resume tests)
- **Estimated Effort**: 2-3 hours
- **Next Steps**: Begin Phase 1 implementation