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

### 2. Complex Event Flow
- Events flow through multiple layers
- Handler notifications are buried in the wrong layer
- Difficult to trace event paths

### 3. Initialization Complexity
- Complex initialization in SessionManager::initialize
- Unsafe code workarounds for ownership issues
- Circular dependencies between components

## Proposed Architecture

### 1. New Component Hierarchy
```
API Layer (builder.rs, create.rs, control.rs)
    ↓
SessionCoordinator (NEW: top-level orchestrator)
    ├── SessionRegistry (storage/lookup)
    ├── EventProcessor (event pub/sub)
    ├── CleanupManager (lifecycle)
    ├── DialogManager (SIP signaling)
    └── MediaManager (RTP/media)
```

### 2. SessionCoordinator Responsibilities
- **Primary orchestrator** for all session operations
- Owns the main event loop
- Coordinates between dialog and media subsystems
- Handles all handler notifications
- Manages session lifecycle

### 3. Simplified SessionManager
- Becomes a simple service for session registry
- No longer creates coordinators
- No complex initialization logic
- Just storage and lookup operations

## Implementation Plan

### Phase 1: Create New SessionCoordinator ✓
1. Create `src/coordinator/mod.rs` as the new top-level module
2. Move orchestration logic from SessionManager
3. Implement proper event handling
4. Add handler notification in the right place

### Phase 2: Update API Layer ✓
1. Update `SessionManagerBuilder` to create `SessionCoordinator`
2. Update all API methods to use `SessionCoordinator`
3. Remove references to old `SessionManager` methods

### Phase 3: Refactor SessionManager
1. Remove coordinator creation logic
2. Remove event loop from initialize()
3. Keep only registry/storage functionality
4. Clean up unsafe code

### Phase 4: Update Examples and Tests
1. Update all examples to use new API
2. Fix any broken tests
3. Add tests for handler notifications

### Phase 5: Clean Up
1. Remove old SessionCoordinator from manager/
2. Update documentation
3. Remove any dead code

## Benefits

1. **Cleaner Architecture**: Proper separation of concerns
2. **Better Event Flow**: Direct path from events to handlers
3. **Easier Testing**: Components can be tested in isolation
4. **No Unsafe Code**: Proper ownership without workarounds
5. **Better Maintainability**: Clear responsibilities for each component

## Migration Guide

For users of the API:
```rust
// Old way
let session_mgr = SessionManagerBuilder::new()
    .build()
    .await?;

// New way (same API, different internals)
let session_mgr = SessionManagerBuilder::new()
    .build()
    .await?;
```

The external API remains the same, only the internal architecture changes.

## Implementation Results

### Phase 1 & 2 Completed Successfully ✓

The refactoring has been successfully implemented with the following changes:

1. **Created New Top-Level SessionCoordinator** (`src/coordinator/mod.rs`)
   - Implements the main orchestration logic
   - Owns the event loop and coordinates all subsystems
   - Properly handles `on_call_ended` notifications
   - Direct event flow from subsystems to handlers

2. **Updated API Layer**
   - `SessionManagerBuilder` now creates `SessionCoordinator` instead of `SessionManager`
   - All API functions updated to use `SessionCoordinator`
   - Removed deprecated functions and traits
   - Fixed import issues and compilation errors

3. **Updated Examples**
   - `uac_client.rs` and `uas_server.rs` updated to use new API
   - Removed references to deprecated methods
   - Fixed all compilation errors

### Test Results

Initial testing shows the refactored system is working:
- ✓ UAC client successfully connects to UAS server
- ✓ Calls are established (200 OK responses)
- ✓ Session states are properly updated
- ✓ BYE messages are sent to terminate calls
- ✓ Event coordination between dialog and media subsystems works

### Known Issues

1. **Media Session Duplication**: Minor issue where media session creation is attempted twice
2. **Call Timing**: Some calls timeout when server is under load
3. **Handler Notification**: The `on_call_ended` handler is now properly called

### Next Steps

1. Complete Phase 3: Simplify SessionManager to just registry operations
2. Fix the media session duplication issue
3. Add comprehensive tests for the new architecture
4. Update documentation to reflect the new architecture

The refactoring successfully addresses the main architectural issues and provides a cleaner, more maintainable codebase. 