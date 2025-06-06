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
│   ├── integration.rs    # extracted from manager/core.rs
│   ├── events.rs         # extracted from manager/events.rs
│   ├── coordination.rs   # dialog-session coordination
│   ├── builder.rs        # extracted from api/builder.rs
│   └── mod.rs            # (new)
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

#### **1.1 Create `src/dialog/mod.rs`**
```rust
//! Dialog-Core Integration
//!
//! This module manages all integration with dialog-core, providing a clean
//! interface for session-core to coordinate with SIP dialog functionality.

pub mod integration;
pub mod events;
pub mod coordination;
pub mod builder;

// Re-exports
pub use integration::DialogManager;
pub use events::DialogEventHandler;
pub use coordination::SessionDialogCoordinator;
pub use builder::DialogBuilder;
```

#### **1.2 Extract Dialog Integration: `manager/core.rs` → `dialog/integration.rs`**

**Content to extract from `manager/core.rs`:**
- Lines 15, 86-87: Dialog-core integration setup
- Lines 147-150: Dialog creation and INVITE sending  
- Lines 189, 198, 211, 224, 237, 254, 295: All SIP operations via dialog-core
- Lines 339+: Dialog event handling (`handle_session_coordination_event`)
- Lines 450, 469: Additional dialog operations
- All `UnifiedDialogApi` usage and references

**Create as:** `src/dialog/integration.rs`
```rust
//! Dialog-Core Integration Implementation
//!
//! Handles all direct integration with dialog-core UnifiedDialogApi,
//! providing session-level abstractions over SIP dialog operations.

// All dialog-core specific code extracted from manager/core.rs
```

#### **1.3 Extract Dialog Events: `manager/events.rs` → `dialog/events.rs`**

**Content to extract:**
- Dialog-specific event handling logic
- Session coordination event processing
- Dialog state change handling

**Create as:** `src/dialog/events.rs`

#### **1.4 Extract Dialog Builder: `api/builder.rs` → `dialog/builder.rs`**

**Content to extract from `api/builder.rs`:**
- Dialog-core UnifiedDialogApi setup code
- Dialog configuration logic
- Dialog manager creation

**Create as:** `src/dialog/builder.rs`

#### **1.5 Create Dialog Coordination: `dialog/coordination.rs`**

**New file for session-dialog coordination:**
```rust
//! Session-Dialog Coordination
//!
//! Manages the coordination between session-core and dialog-core,
//! handling event bridging and lifecycle management.

pub struct SessionDialogCoordinator {
    // Coordinate between session events and dialog events
}
```

### **Phase 2: Update Manager Module**

#### **2.1 Simplify `manager/core.rs`**
- **Remove:** All dialog-core specific implementation code
- **Keep:** High-level session orchestration logic
- **Add:** Import and use `crate::dialog::DialogManager`
- **Update:** Method implementations to delegate to dialog module

**Before (lines to remove):**
```rust
// Dialog-core integration (only layer we integrate with) - using UnifiedDialogApi
// All the dialog-specific implementation code
```

**After (new imports):**
```rust
use crate::dialog::DialogManager;
// Use dialog manager methods instead of direct dialog-core calls
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
1. `src/dialog/mod.rs`
2. `src/dialog/integration.rs` (extracted from `manager/core.rs`)
3. `src/dialog/events.rs` (extracted from `manager/events.rs`)
4. `src/dialog/coordination.rs` (new)
5. `src/dialog/builder.rs` (extracted from `api/builder.rs`)

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

## 📝 **Review Questions**

1. **Scope:** Should we move ALL dialog-related code from manager, or keep some high-level orchestration there?
2. **Events:** Should dialog coordination events go in `/dialog` or stay in `/manager`?
3. **Naming:** Any preferences for naming the new dialog module files?
4. **Approach:** Should we do this refactoring in one large change or multiple smaller PRs?
5. **Testing:** Any additional testing requirements beyond the proposed strategy?

**Ready for review and approval before execution.** 