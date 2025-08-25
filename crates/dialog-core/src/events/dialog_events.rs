//! Dialog-specific events
//!
//! Internal events for dialog state changes and processing within dialog-core.

use serde::{Serialize, Deserialize};
use std::time::Duration;
use crate::dialog::{DialogId, DialogState, SubscriptionState};

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
    
    // ========== SUBSCRIPTION EVENTS (RFC 6665) ==========
    
    /// Subscription created
    SubscriptionCreated {
        dialog_id: DialogId,
        event_package: String,
        expires: Duration,
    },
    
    /// Subscription refreshed
    SubscriptionRefreshed {
        dialog_id: DialogId,
        new_expires: Duration,
    },
    
    /// Subscription terminated
    SubscriptionTerminated {
        dialog_id: DialogId,
        reason: Option<String>,
    },
    
    /// NOTIFY received for subscription
    NotifyReceived {
        dialog_id: DialogId,
        state: SubscriptionState,
        body: Option<Vec<u8>>,
    },
    
    /// Subscription refresh needed
    SubscriptionRefreshNeeded {
        dialog_id: DialogId,
        current_expires: Duration,
    },
    
    /// Subscription refresh failed
    SubscriptionRefreshFailed {
        dialog_id: DialogId,
        attempts: u32,
        error: String,
    },
} 