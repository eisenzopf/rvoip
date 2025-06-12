# SessionCoordinator Architecture Refactoring Plan

## Executive Summary

The current architecture has an inverted dependency structure where `SessionManager` creates and manages `SessionCoordinator`, when it should be the opposite. This document outlines a plan to refactor the architecture to follow proper dependency inversion principles and create a cleaner, more maintainable system.

## Current Architecture Issues

### 1. Inverted Dependencies
```
Current (Wrong):
SessionManager (core.rs) 
    → creates SessionCoordinator
    → creates DialogCoordinator & MediaCoordinator
    → manages event loops
    → handles initialization

Should Be:
SessionCoordinator
    → uses SessionManager (for registry/storage)
    → coordinates DialogManager & MediaManager
    → owns the event loop
    → handles all orchestration
```

### 2. Event Flow Problems
- Events flow through multiple layers unnecessarily
- Handler notifications are buried in coordinator but coordinator is created by manager
- Complex event bridging in `SessionManager::initialize()`
- Duplicate event handling logic

### 3. Initialization Complexity
- SessionManager has 160+ lines of initialization logic that belongs in coordinator
- Unsafe workarounds due to Arc<Self> limitations
- Multiple spawned tasks that should be consolidated

### 4. API Surface Confusion
- API calls go to SessionManager which delegates to DialogManager
- Should go to SessionCoordinator which orchestrates everything

## Proposed Architecture

### Layer 1: API Layer (Unchanged)
- `SessionManagerBuilder` - Builds the system
- Simple API functions (make_call, hold_call, etc.)
- CallHandler trait for user callbacks

### Layer 2: Orchestration Layer (New Primary Layer)
```rust
pub struct SessionCoordinator {
    // Core services
    registry: Arc<SessionRegistry>,
    event_processor: Arc<SessionEventProcessor>,
    cleanup_manager: Arc<CleanupManager>,
    
    // Subsystem managers
    dialog_manager: Arc<DialogManager>,
    media_manager: Arc<MediaManager>,
    
    // User handler
    handler: Option<Arc<dyn CallHandler>>,
    
    // Configuration
    config: SessionConfig,
}

impl SessionCoordinator {
    /// Create and initialize the entire system
    pub async fn new(config: SessionConfig, handler: Option<Arc<dyn CallHandler>>) -> Result<Arc<Self>>;
    
    /// Start all subsystems
    pub async fn start(&self) -> Result<()>;
    
    /// Stop all subsystems
    pub async fn stop(&self) -> Result<()>;
    
    // All public API methods that were in SessionManager
    pub async fn create_outgoing_call(...) -> Result<CallSession>;
    pub async fn terminate_session(...) -> Result<()>;
    // ... etc
}
```

### Layer 3: Service Layer
```rust
// SessionManager becomes a simple service for session registry
pub struct SessionManager {
    registry: Arc<SessionRegistry>,
}

impl SessionManager {
    pub async fn register_session(&self, session: CallSession) -> Result<()>;
    pub async fn get_session(&self, id: &SessionId) -> Result<Option<CallSession>>;
    pub async fn update_session_state(&self, id: &SessionId, state: CallState) -> Result<()>;
    pub async fn remove_session(&self, id: &SessionId) -> Result<()>;
}
```

### Layer 4: Subsystem Layer (Unchanged)
- DialogManager - SIP protocol handling
- MediaManager - RTP/Media handling
- Already properly structured

## Refactoring Steps

### Phase 1: Create New SessionCoordinator Structure
1. Create `src/coordinator/mod.rs` with new structure
2. Move orchestration logic from `SessionManager::initialize()` 
3. Implement proper event loop in coordinator
4. Add all public API methods

### Phase 2: Simplify SessionManager
1. Rename current `SessionManager` to `SessionRegistry` 
2. Create new simple `SessionManager` as a service
3. Remove all orchestration logic
4. Keep only session storage/retrieval functions

### Phase 3: Update Event Flow
1. Single event loop in SessionCoordinator
2. Direct handler notifications
3. Remove complex event bridging
4. Cleaner event types

### Phase 4: Update API Layer
1. Update `SessionManagerBuilder` to build `SessionCoordinator`
2. Update all API functions to use coordinator
3. Update examples to use new structure

### Phase 5: Testing & Migration
1. Update all tests
2. Update examples (uac_client.rs, uas_server.rs)
3. Ensure backward compatibility where possible

## Benefits

1. **Cleaner Architecture**: Proper dependency direction
2. **Simpler Event Flow**: Direct path from events to handlers
3. **Better Testability**: Each layer can be tested independently
4. **Easier Maintenance**: Clear separation of concerns
5. **Fix Handler Issues**: Direct access to handler from coordinator

## Migration Strategy

### For Library Users (Examples)
```rust
// Old way
let session_mgr = SessionManagerBuilder::new()
    .with_handler(handler)
    .build()
    .await?;

// New way (same API, different internals)
let session_mgr = SessionManagerBuilder::new()
    .with_handler(handler)
    .build()
    .await?;
```

The external API remains the same, only internals change.

### For Internal Code
- All internal references to SessionManager for orchestration change to SessionCoordinator
- SessionManager becomes a simple service used by SessionCoordinator

## Implementation Timeline

1. **Day 1**: Create new coordinator structure, move initialization logic
2. **Day 2**: Refactor SessionManager to simple service
3. **Day 3**: Update event flow and handler notifications
4. **Day 4**: Update API layer and tests
5. **Day 5**: Update examples and documentation

## Risks & Mitigations

### Risk 1: Breaking Changes
- **Mitigation**: Keep external API identical, only change internals

### Risk 2: Complex Migration
- **Mitigation**: Do refactoring in small, testable steps

### Risk 3: Hidden Dependencies
- **Mitigation**: Comprehensive testing at each phase

## Success Criteria

1. ✅ Handler notifications work properly (on_call_ended is called)
2. ✅ Cleaner, more understandable code structure
3. ✅ All tests pass
4. ✅ Examples work without modification
5. ✅ Better separation of concerns

## Code Examples

### New SessionCoordinator Event Loop
```rust
impl SessionCoordinator {
    async fn run_event_loop(&self, mut events_rx: mpsc::Receiver<SessionEvent>) {
        while let Some(event) = events_rx.recv().await {
            match event {
                SessionEvent::SessionTerminated { session_id, reason } => {
                    // Direct handler notification
                    if let Some(handler) = &self.handler {
                        if let Some(session) = self.registry.get_session(&session_id).await? {
                            handler.on_call_ended(session, &reason).await;
                        }
                    }
                    // Cleanup
                    self.cleanup_session(&session_id).await;
                }
                // ... other events
            }
        }
    }
}
```

### Simplified API Method
```rust
impl SessionCoordinator {
    pub async fn terminate_session(&self, session_id: &SessionId) -> Result<()> {
        // Direct orchestration
        self.dialog_manager.send_bye(session_id).await?;
        self.media_manager.stop_media(session_id).await?;
        self.registry.remove_session(session_id).await?;
        
        // Send event for handler notification
        self.event_tx.send(SessionEvent::SessionTerminated {
            session_id: session_id.clone(),
            reason: "User terminated".to_string(),
        }).await?;
        
        Ok(())
    }
}
```

## Conclusion

This refactoring will create a cleaner, more maintainable architecture that follows proper design principles. The SessionCoordinator will truly be the coordinator of the system, with clear dependencies and responsibilities. Most importantly, it will fix the current handler notification issues and make the system easier to understand and extend. 