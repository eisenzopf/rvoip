//! Dialog-specific events
//!
//! Internal events for dialog state changes and processing within dialog-core.

use serde::{Serialize, Deserialize};
use crate::dialog::{DialogId, DialogState};

/// Internal dialog events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DialogEvent {
    /// Dialog state changed
    StateChanged {
        dialog_id: DialogId,
        old_state: DialogState,
        new_state: DialogState,
    },
    
    /// Dialog created
    Created {
        dialog_id: DialogId,
    },
    
    /// Dialog terminated
    Terminated {
        dialog_id: DialogId,
        reason: String,
    },
    
    /// Dialog entered recovery mode
    RecoveryStarted {
        dialog_id: DialogId,
        reason: String,
    },
    
    /// Dialog recovery completed
    RecoveryCompleted {
        dialog_id: DialogId,
    },
    
    // ========== GRACEFUL SHUTDOWN EVENTS ==========
    
    /// Shutdown request received from session layer
    ShutdownRequested,
    
    /// Dialog manager is ready for shutdown
    ShutdownReady,
    
    /// Dialog manager should shutdown now
    ShutdownNow,
    
    /// Dialog manager shutdown complete
    ShutdownComplete,
} 