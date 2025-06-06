# Session-Core Directory Reorganization Plan

## 🎯 **Objective**

Reorganize session-core directory structure to create **consistent integration patterns** for external dependencies, specifically dialog-core and media-core integrations.

## 🔍 **Current State Analysis**

### **Current Structure Issues:**
```
src/
├── session/          # Basic session types (4 files)
├── manager/          # Mixed: SessionManager + dialog-core integration (5 files, 39KB)
├── coordination/     # Session primitives (4 files, 4KB)  
├── bridge/           # Conference bridging (3 files, 2KB)
├── media/            # ✅ Media-core integration (6 files, ~1.6KB) - GOOD
└── api/              # ✅ Public API + dialog setup - MIXED
```

### **Problem Identified:**
- **Media-core integration**: Clean, dedicated `/media` directory ✅
- **Dialog-core integration**: Scattered across `/manager` and `/api` ❌
- **Inconsistent pattern**: No parallel structure for external integrations

### **Dialog-Core Integration Currently Located In:**
1. **`manager/core.rs`** (26KB) - 15+ dialog-core references
   - Session coordination with dialog-core
   - All SIP operations via dialog-core unified API
   - Dialog event handling
2. **`api/builder.rs`** - Dialog-core setup and configuration
3. **`manager/events.rs`** - Some dialog event processing

## 🎯 **Target Structure**

### **Proposed Consistent Structure:**
```
src/
├── lib.rs
├── errors.rs
├── session/          # 📱 Basic session types & lifecycle
│   ├── session.rs    # (existing) 
│   ├── state.rs      # (existing)
│   ├── lifecycle.rs  # (existing)
│   ├── media.rs      # (existing stub)
│   └── mod.rs        # (existing)
├── dialog/           # 🗣️ Dialog-core integration (NEW - parallel to media/)
│   ├── manager.rs        # DialogManager (parallel to MediaManager)
│   ├── coordinator.rs    # SessionDialogCoordinator (parallel to SessionMediaCoordinator)
│   ├── config.rs         # DialogConfigConverter (parallel to MediaConfigConverter)
│   ├── bridge.rs         # DialogBridge (parallel to MediaBridge)
│   ├── types.rs          # Dialog types (parallel to MediaEngine/types)
│   ├── builder.rs        # Dialog setup (unique to dialog - media doesn't need this)
│   └── mod.rs            # exports and DialogError (parallel to MediaError)
├── media/            # 🎵 Media-core integration (keep as-is)
│   ├── mod.rs            # (existing)
│   ├── types.rs          # (existing)
│   ├── manager.rs        # (existing)
│   ├── coordinator.rs    # (existing)
│   ├── config.rs         # (existing)
│   └── bridge.rs         # (existing)
├── manager/          # 🎯 High-level orchestration (cleaned up)
│   ├── core.rs           # (simplified - dialog code removed)
│   ├── registry.rs       # (keep)
│   ├── cleanup.rs        # (keep)
│   └── mod.rs            # (updated)
├── coordination/     # 🤝 Session primitives (keep as-is)
│   ├── groups.rs         # (existing)
│   ├── priority.rs       # (existing)
│   ├── resources.rs      # (existing)
│   └── mod.rs            # (existing)
├── bridge/           # 🌉 Conference bridging (keep as-is)
│   ├── bridge.rs         # (existing)
│   ├── types.rs          # (existing)
│   └── mod.rs            # (existing)
└── api/              # 🌐 Public API (simplified)
    └── ... (dialog builder moved out)
```

## 🔧 **Detailed Migration Plan**

### **Phase 1: Create Dialog Integration Directory**

#### **1.1 Create `src/dialog/mod.rs`** (Parallel to `media/mod.rs`)
```rust
//! Session-Core Dialog Integration
//!
//! This module provides comprehensive dialog integration for session-core,
//! coordinating between session management and dialog-core SIP operations.
//!
//! Architecture (parallel to media/ module):
//! - `DialogManager`: Main interface for dialog operations (parallel to MediaManager)
//! - `SessionDialogCoordinator`: Automatic dialog lifecycle management (parallel to SessionMediaCoordinator)
//! - `DialogConfigConverter`: SIP ↔ session configuration conversion (parallel to MediaConfigConverter)
//! - `DialogBridge`: Event integration between dialog and session systems (parallel to MediaBridge)
//! - `types`: Dialog type definitions (parallel to MediaEngine/types)
//! - `builder`: Dialog setup and creation (unique to dialog - media doesn't need this)

pub mod manager;
pub mod coordinator;
pub mod config;
pub mod bridge;
pub mod types;
pub mod builder;

// Re-exports for convenience
pub use manager::DialogManager;
pub use coordinator::SessionDialogCoordinator;
pub use config::DialogConfigConverter;
pub use bridge::DialogBridge;
pub use types::*;
pub use builder::DialogBuilder;

/// Dialog integration result type
pub type DialogResult<T> = Result<T, DialogError>;

/// Dialog integration errors (parallel to MediaError)
#[derive(Debug, thiserror::Error)]
pub enum DialogError {
    #[error("Dialog session not found: {session_id}")]
    SessionNotFound { session_id: String },
    
    #[error("Dialog configuration error: {message}")]
    Configuration { message: String },
    
    #[error("SIP processing error: {message}")]
    SipProcessing { message: String },
    
    #[error("Dialog creation failed: {reason}")]
    DialogCreation { reason: String },
    
    #[error("Dialog-core error: {source}")]
    DialogCore { 
        #[from]
        source: Box<dyn std::error::Error + Send + Sync> 
    },
    
    #[error("Session coordination error: {message}")]
    Coordination { message: String },
}

impl From<DialogError> for crate::errors::SessionError {
    fn from(err: DialogError) -> Self {
        crate::errors::SessionError::DialogIntegration { 
            message: err.to_string() 
        }
    }
}
```

#### **1.2 Extract Dialog Manager: `manager/core.rs` → `dialog/manager.rs`** (Parallel to `media/manager.rs`)

**Content to extract from `manager/core.rs`:**
- Lines 16-17: UnifiedDialogApi integration
- Lines 147-150: Dialog creation and INVITE sending  
- Lines 189, 198, 211, 224, 237, 254, 295: All SIP operations (hold, resume, transfer, terminate, DTMF, update)
- Dialog-to-session mapping logic
- All `UnifiedDialogApi` usage and references

**Create as:** `src/dialog/manager.rs`
```rust
//! Dialog Manager (parallel to MediaManager)
//!
//! Main interface for dialog operations, providing session-level abstractions
//! over dialog-core UnifiedDialogApi functionality.

pub struct DialogManager {
    // Wrapper around UnifiedDialogApi with session-level interface
}
```

#### **1.3 Extract Session Coordination: `manager/core.rs` → `dialog/coordinator.rs`** (Parallel to `media/coordinator.rs`)

**Content to extract from `manager/core.rs`:**
- Lines 339+: Dialog event handling (`handle_session_coordination_event`)
- Session coordination event processing
- Dialog state change handling

**Create as:** `src/dialog/coordinator.rs`
```rust
//! Session Dialog Coordinator (parallel to SessionMediaCoordinator)
//!
//! Manages the coordination between session-core and dialog-core,
//! handling event bridging and lifecycle management.

pub struct SessionDialogCoordinator {
    // Coordinate between session events and dialog events
}
```

#### **1.4 Extract Dialog Builder: `api/builder.rs` → `dialog/builder.rs`** (Unique to Dialog)

**Content to extract from `api/builder.rs`:**
- Dialog-core UnifiedDialogApi setup code
- Dialog configuration logic
- Dialog manager creation

**Create as:** `src/dialog/builder.rs`

#### **1.5 Create Dialog Config: `dialog/config.rs`** (Parallel to `media/config.rs`)

**New file for SIP/dialog configuration conversion:**
```rust
//! Dialog Config Converter (parallel to MediaConfigConverter)
//!
//! Handles conversion between session-level configuration and 
//! dialog-core SIP configuration.

pub struct DialogConfigConverter {
    // Convert between session config and SIP/dialog config
}
```

#### **1.6 Create Dialog Bridge: `dialog/bridge.rs`** (Parallel to `media/bridge.rs`)

**New file for dialog-session event integration:**
```rust
//! Dialog Bridge (parallel to MediaBridge)
//!
//! Event integration between dialog-core and session systems.

pub struct DialogBridge {
    // Bridge dialog events to session events
}
```

#### **1.7 Create Dialog Types: `dialog/types.rs`** (Parallel to `media/types.rs`)

**New file for dialog type definitions:**
```rust
//! Dialog Types (parallel to MediaEngine/types)
//!
//! Type definitions for dialog integration.

// Dialog-related type definitions and traits
```

### **Phase 2: Update Manager Module**

#### **2.1 Simplify `manager/core.rs`** (Achieve Parallel Integration Levels)
- **Remove:** All dialog-core specific implementation code (UnifiedDialogApi usage)
- **Keep:** High-level session orchestration logic
- **Add:** Import and use `crate::dialog::DialogManager` (parallel to existing media integration)
- **Update:** Method implementations to delegate to DialogManager (same level as MediaManager)

**Current Integration Level:**
```rust
// Direct dialog-core integration - TOO LOW LEVEL for manager
use rvoip_dialog_core::{api::unified::UnifiedDialogApi, ...};
let _tx_key = self.dialog_api.send_bye(&dialog_id).await?;
```

**Target Integration Level (parallel to media):**
```rust
// High-level integration via DialogManager - SAME LEVEL as MediaManager
use crate::dialog::DialogManager;
use crate::media::MediaManager; 
// Both dialog and media managers at same abstraction level
self.dialog_manager.terminate_session(session_id).await?;
self.media_manager.stop_media(session_id).await?;
```

#### **2.2 Update `manager/mod.rs`**
```rust
//! Session Manager Module
//!
//! High-level session orchestration that coordinates dialog and media integration.

pub mod core;
pub mod registry;
pub mod cleanup;

// Re-export the main SessionManager
pub use core::SessionManager;
```

### **Phase 3: Update API Module**

#### **3.1 Simplify `api/builder.rs`**
- **Remove:** Dialog-core integration setup code
- **Add:** Use `crate::dialog::DialogBuilder`
- **Focus:** High-level session infrastructure setup only

### **Phase 4: Update Root Module**

#### **4.1 Update `src/lib.rs`**
```rust
pub mod api;
pub mod session;
pub mod dialog;        // NEW - dialog-core integration
pub mod media;         // EXISTING - media-core integration
pub mod manager;       // SIMPLIFIED - orchestration only
pub mod coordination;
pub mod bridge;

// Core error types
mod errors;
pub use errors::{SessionError, Result};

// Re-export the main API for convenience
pub use api::*;

// Re-export SessionManager for direct access
pub use manager::SessionManager;

// Prelude module for common imports
pub mod prelude {
    pub use crate::api::*;
    pub use crate::errors::{SessionError, Result};
    pub use crate::manager::events::{SessionEvent, SessionEventProcessor};
    pub use crate::manager::SessionManager;
    pub use crate::dialog::DialogManager;  // NEW
}
```

## 📋 **File Operations Summary**

### **Files to Create:**
1. `src/dialog/mod.rs` (parallel to `media/mod.rs`)
2. `src/dialog/manager.rs` (parallel to `media/manager.rs`, extracted from `manager/core.rs`)
3. `src/dialog/coordinator.rs` (parallel to `media/coordinator.rs`, extracted from `manager/core.rs`)
4. `src/dialog/config.rs` (parallel to `media/config.rs`, new)
5. `src/dialog/bridge.rs` (parallel to `media/bridge.rs`, new)
6. `src/dialog/types.rs` (parallel to `media/types.rs`, new)
7. `src/dialog/builder.rs` (unique to dialog, extracted from `api/builder.rs`)

### **Files to Modify:**
1. `src/lib.rs` - Add dialog module export
2. `src/manager/core.rs` - Remove dialog code, add dialog imports
3. `src/manager/mod.rs` - Update documentation
4. `src/api/builder.rs` - Remove dialog code, use dialog module
5. `Cargo.toml` - No changes needed

### **Files to Keep As-Is:**
- All files in `session/`, `media/`, `coordination/`, `bridge/`
- `errors.rs`
- All test files

## 🎯 **Benefits & Rationale**

### **1. Consistent Integration Pattern**
- **Before:** Media has dedicated directory, dialog scattered
- **After:** Both external integrations have parallel dedicated directories

### **2. Clear Separation of Concerns**
- **`/session`** = Basic session types and lifecycle
- **`/dialog`** = Dialog-core integration (SIP protocol coordination)
- **`/media`** = Media-core integration (media processing coordination)
- **`/manager`** = High-level orchestration using dialog + media
- **`/coordination`** = Session primitives (groups, priority, resources)
- **`/bridge`** = Conference functionality
- **`/api`** = Public interfaces

### **3. Improved Maintainability**
- Dialog-core changes only affect `/dialog` directory
- Media-core changes only affect `/media` directory
- Manager focuses on business logic, not integration details
- Easier to locate integration-specific code

### **4. Better Architecture**
- External dependencies clearly isolated
- Internal session logic separated from integration logic
- Consistent patterns for adding future integrations

## 🧪 **Testing Strategy**

### **After Each Phase:**
1. `cargo check -p rvoip-session-core`
2. Verify no compilation errors
3. Check that all imports resolve correctly

### **After Completion:**
1. `cargo test -p rvoip-session-core --lib`
2. Verify all 14 unit tests still pass
3. Ensure existing functionality works through new structure

### **Integration Testing:**
1. Test dialog operations work through new structure
2. Test media operations continue working
3. Test manager orchestration functions correctly

## ⏱️ **Estimated Timeline**

- **Phase 1**: ~2 hours (extract dialog code, create new files)
- **Phase 2**: ~1 hour (simplify manager, update imports)
- **Phase 3**: ~30 minutes (update API module)
- **Phase 4**: ~30 minutes (update root module and imports)

**Total Estimated Time**: ~4 hours

## ⚠️ **Risks & Mitigations**

### **Risk 1: Breaking Existing Functionality**
- **Mitigation:** Incremental approach with testing after each phase
- **Mitigation:** Keep all existing functionality, just reorganize location

### **Risk 2: Complex Import Dependencies**
- **Mitigation:** Update imports systematically in each phase
- **Mitigation:** Use re-exports to maintain API compatibility

### **Risk 3: Large File Movements**
- **Mitigation:** Extract code carefully, maintaining all functionality
- **Mitigation:** Git will track file movements and content changes

## 🔄 **Rollback Plan**

If issues arise:
1. **Phase-by-phase rollback:** Each phase is isolated
2. **Git revert:** Use git to revert specific commits
3. **Incremental fixing:** Address issues in small steps

## 🚀 **Success Criteria**

### **Functional Success:**
- [ ] All existing tests pass
- [ ] `cargo check` and `cargo test` succeed
- [ ] No breaking changes to public API
- [ ] All dialog operations work through new structure
- [ ] All media operations continue working

### **Structural Success:**
- [ ] Dialog-core integration isolated in `/dialog` directory
- [ ] Manager module simplified and focused on orchestration
- [ ] Consistent pattern between `/dialog` and `/media` directories
- [ ] Clear separation of concerns achieved

### **Maintainability Success:**
- [ ] Integration code easy to locate
- [ ] External dependency changes isolated to specific directories
- [ ] Manager focuses on business logic only
- [ ] Clean, understandable directory structure

---

## 📝 **Updated Based on Your Feedback**

**✅ Integration Levels:** Session manager will have **comparable levels** of dialog-core and media-core integration:
- `DialogManager` and `MediaManager` both provide high-level interfaces to session manager
- No direct `UnifiedDialogApi` calls in session manager (same as no direct media-core calls)

**✅ Parallel File Structure:** Dialog/ mirrors media/ structure exactly:
```
media/                     dialog/
├── manager.rs      ↔     ├── manager.rs        (parallel)
├── coordinator.rs  ↔     ├── coordinator.rs    (parallel)  
├── config.rs       ↔     ├── config.rs         (parallel)
├── bridge.rs       ↔     ├── bridge.rs         (parallel)
├── types.rs        ↔     ├── types.rs          (parallel)
├── mod.rs          ↔     ├── mod.rs            (parallel)
└── (none)                └── builder.rs        (unique to dialog)
```

**✅ PR Approach:** Can be done as one comprehensive refactor since you have no preference.

## 📊 **Implementation Status**

### **✅ Phase 1: COMPLETED** 
**Dialog Integration Directory Created** *(2024-06-06)*

All dialog module files have been successfully created with perfect parallel structure to media module:

- [x] `src/dialog/mod.rs` - Module root with exports and DialogError
- [x] `src/dialog/types.rs` - Dialog types and handles  
- [x] `src/dialog/config.rs` - DialogConfigConverter for session-to-dialog config
- [x] `src/dialog/bridge.rs` - DialogBridge for event integration
- [x] `src/dialog/coordinator.rs` - SessionDialogCoordinator for lifecycle management
- [x] `src/dialog/manager.rs` - DialogManager for session-level dialog operations
- [x] `src/dialog/builder.rs` - DialogBuilder for setup (unique to dialog)
- [x] `src/errors.rs` - Added DialogIntegration error variant
- [x] `src/lib.rs` - Added dialog module export and prelude
- [x] `src/api/types.rs` - Added CallState::Cancelled variant

**Phase 1 Results:**
- ✅ Library compiles successfully (`cargo check`)
- ✅ Perfect parallel structure achieved (media/ ↔ dialog/)
- ✅ DialogError integrates seamlessly with SessionError
- ✅ Comprehensive dialog event bridging implemented
- ✅ All dialog operations extracted (hold, resume, transfer, terminate, DTMF)
- ✅ Session coordination logic fully extracted from manager

### **🔄 Phase 2: PENDING**
**Update Manager Module** *(Next Step)*

- [ ] Simplify `manager/core.rs` - Remove dialog-core specific code
- [ ] Add DialogManager integration at same level as MediaManager  
- [ ] Update manager imports to use dialog module
- [ ] Achieve comparable integration levels

### **⏸️ Phase 3: PENDING**
**Update API Module**

- [ ] Simplify `api/builder.rs` - Remove dialog setup code
- [ ] Use DialogBuilder from dialog module

### **⏸️ Phase 4: PENDING** 
**Final Integration & Testing**

- [ ] Update all imports and references
- [ ] Run comprehensive tests
- [ ] Verify comparable abstraction levels

---

## 🎯 **Current Architecture Achieved**

### **Perfect Parallel Structure:**
```
✅ IMPLEMENTED:
media/                     dialog/
├── manager.rs      ↔     ├── manager.rs        ✅
├── coordinator.rs  ↔     ├── coordinator.rs    ✅  
├── config.rs       ↔     ├── config.rs         ✅
├── bridge.rs       ↔     ├── bridge.rs         ✅
├── types.rs        ↔     ├── types.rs          ✅
├── mod.rs          ↔     ├── mod.rs            ✅
└── (none)                └── builder.rs        ✅ (unique)
```

### **Next Target (Phase 2):**
```
🎯 MANAGER INTEGRATION LEVELS:
// Current (inconsistent):
self.media_manager.stop_media(session_id).await?;        // High-level ✅
self.dialog_api.send_bye(&dialog_id).await?;             // Low-level ❌

// Target (parallel):  
self.media_manager.stop_media(session_id).await?;        // High-level ✅
self.dialog_manager.terminate_session(session_id).await?; // High-level ✅
```

---

## 📝 **Phase 1 Implementation Notes**

### **Key Achievements:**
1. **Seamless Error Integration**: DialogError → SessionError conversion works perfectly
2. **Comprehensive Event Bridging**: All dialog coordination events mapped to session events
3. **Complete Code Extraction**: All dialog operations moved from manager to dialog module
4. **Type Safety**: All imports and dependencies resolved correctly
5. **Compilation Success**: No errors, only minor warnings in other crates

### **Technical Decisions:**
- Used `#[async_trait]` pattern for consistency with existing codebase
- Created separate coordinator for session-dialog event handling
- Implemented builder pattern for dialog API setup (unique to dialog needs)
- Added CallState::Cancelled for 487 SIP response handling
- Used dashmap for thread-safe dialog-to-session mapping

### **Lessons Learned:**
- Dialog-core API imports needed adjustment (`api::{CallHandle, DialogHandle}`)
- SIP types are `Request`/`Response` not `SipRequest`/`SipResponse`
- DialogManagerConfig uses builder pattern, not simple config struct
- Registration events needed string conversion for compatibility

---

## 🚀 **Ready for Phase 2**

**Current Status**: Phase 1 successfully completed with perfect parallel architecture established.

**Next Step**: Update manager module to use DialogManager at same abstraction level as MediaManager.

**Estimated Time for Phase 2**: ~1 hour

**Ready to proceed when requested.** 