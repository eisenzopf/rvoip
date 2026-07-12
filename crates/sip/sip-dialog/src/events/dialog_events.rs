//! Dialog-specific events
//!
//! Internal events for dialog state changes and processing within dialog-core.

use crate::dialog::{DialogId, DialogState, SubscriptionState};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Duration;

/// Internal dialog events
#[derive(Clone, Serialize, Deserialize)]
pub enum DialogEvent {
    /// Dialog state changed
    StateChanged {
        dialog_id: DialogId,
        old_state: DialogState,
        new_state: DialogState,
    },

    /// Dialog created
    Created { dialog_id: DialogId },

    /// Dialog terminated
    Terminated { dialog_id: DialogId, reason: String },

    /// Dialog entered recovery mode
    RecoveryStarted { dialog_id: DialogId, reason: String },

    /// Dialog recovery completed
    RecoveryCompleted { dialog_id: DialogId },

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

impl fmt::Debug for DialogEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StateChanged {
                dialog_id,
                old_state,
                new_state,
            } => formatter
                .debug_struct("StateChanged")
                .field("dialog_id", dialog_id)
                .field("old_state", old_state)
                .field("new_state", new_state)
                .finish(),
            Self::Created { dialog_id } => formatter
                .debug_struct("Created")
                .field("dialog_id", dialog_id)
                .finish(),
            Self::Terminated { dialog_id, reason } => formatter
                .debug_struct("Terminated")
                .field("dialog_id", dialog_id)
                .field("reason_len", &reason.len())
                .finish(),
            Self::RecoveryStarted { dialog_id, reason } => formatter
                .debug_struct("RecoveryStarted")
                .field("dialog_id", dialog_id)
                .field("reason_len", &reason.len())
                .finish(),
            Self::RecoveryCompleted { dialog_id } => formatter
                .debug_struct("RecoveryCompleted")
                .field("dialog_id", dialog_id)
                .finish(),
            Self::ShutdownRequested => formatter.write_str("ShutdownRequested"),
            Self::ShutdownReady => formatter.write_str("ShutdownReady"),
            Self::ShutdownNow => formatter.write_str("ShutdownNow"),
            Self::ShutdownComplete => formatter.write_str("ShutdownComplete"),
            Self::SubscriptionCreated {
                dialog_id,
                event_package,
                expires,
            } => formatter
                .debug_struct("SubscriptionCreated")
                .field("dialog_id", dialog_id)
                .field("event_package_len", &event_package.len())
                .field("expires", expires)
                .finish(),
            Self::SubscriptionRefreshed {
                dialog_id,
                new_expires,
            } => formatter
                .debug_struct("SubscriptionRefreshed")
                .field("dialog_id", dialog_id)
                .field("new_expires", new_expires)
                .finish(),
            Self::SubscriptionTerminated { dialog_id, reason } => formatter
                .debug_struct("SubscriptionTerminated")
                .field("dialog_id", dialog_id)
                .field("reason_present", &reason.is_some())
                .field("reason_len", &reason.as_ref().map(String::len))
                .finish(),
            Self::NotifyReceived {
                dialog_id,
                state: _,
                body,
            } => formatter
                .debug_struct("NotifyReceived")
                .field("dialog_id", dialog_id)
                .field("subscription_state_present", &true)
                .field("body_len", &body.as_ref().map(Vec::len))
                .finish(),
            Self::SubscriptionRefreshNeeded {
                dialog_id,
                current_expires,
            } => formatter
                .debug_struct("SubscriptionRefreshNeeded")
                .field("dialog_id", dialog_id)
                .field("current_expires", current_expires)
                .finish(),
            Self::SubscriptionRefreshFailed {
                dialog_id,
                attempts,
                error,
            } => formatter
                .debug_struct("SubscriptionRefreshFailed")
                .field("dialog_id", dialog_id)
                .field("attempts", attempts)
                .field("error_len", &error.len())
                .finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialog_event_debug_does_not_reflect_payloads() {
        const SECRET: &str = "dialog-event-debug-secret-canary";
        let event = DialogEvent::SubscriptionRefreshFailed {
            dialog_id: DialogId::new(),
            attempts: 3,
            error: SECRET.to_string(),
        };
        let debug = format!("{event:?}");

        assert!(!debug.contains(SECRET));
        assert!(debug.contains("error_len"));
        assert!(debug.contains("attempts: 3"));
    }
}
