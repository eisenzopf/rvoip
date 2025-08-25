//! Provides timer functionalities for SIP transactions, including management of
//! timer lifecycles, standard timer configurations, and factories for creating timers.
//!
//! # Overview
//!
//! RFC 3261 defines various timers that are crucial for ensuring reliable message delivery,
//! proper timeout handling, and state management for SIP transactions. This module
//! implements these timers and provides a framework for managing their lifecycle.
//!
//! # Timer Component Relationships
//!
//! ```text
//! ┌─────────────────┐     creates    ┌───────────────┐     schedules    ┌───────────┐
//! │  TimerFactory   │───────────────▶│     Timer     │◀────────────────┤ Transaction│
//! └────────┬────────┘                └───────┬───────┘                  └───────────┘
//!          │                                 │                                ▲
//!          │ uses                   reported │                                │
//!          ▼                         by      │                                │
//! ┌─────────────────┐                        │         sends events           │
//! │  TimerManager   │────────────────────────┴────────────────────────────────┘
//! └─────────────────┘
//! ```
//!
//! # Key Components
//!
//! This module re-exports key components:
//! - [`Timer`]: Represents an active timer instance with its properties.
//! - [`TimerType`]: Enumerates specific SIP transaction timers (e.g., A, B, G) and generic types.
//! - [`TimerSettings`]: Configuration for standard timer durations (T1, T2, etc.).
//! - [`TimerManager`]: Manages the execution of timers and dispatches events.
//! - [`TimerFactory`]: Simplifies the creation of standard RFC 3261 timers.
//!
//! # SIP Timer Overview
//!
//! RFC 3261 defines several timer types for different transaction scenarios:
//!
//! ## Client Transaction Timers
//! - **Timer A** (INVITE): Controls request retransmissions 
//! - **Timer B** (INVITE): Transaction timeout
//! - **Timer D** (INVITE): Wait time for response retransmissions
//! - **Timer E** (non-INVITE): Controls request retransmissions
//! - **Timer F** (non-INVITE): Transaction timeout
//! - **Timer K** (non-INVITE): Wait time for response retransmissions
//!
//! ## Server Transaction Timers
//! - **Timer G** (INVITE): Controls response retransmissions
//! - **Timer H** (INVITE): Wait time for ACK
//! - **Timer I** (INVITE): Wait time in Confirmed state
//! - **Timer J** (non-INVITE): Wait time for request retransmissions
//!
//! # Example Usage
//!
//! ```rust,no_run
//! use std::sync::Arc;
//! use std::time::Duration;
//! use rvoip_dialog_core::transaction::timer::{TimerFactory, TimerManager, TimerSettings};
//! use rvoip_dialog_core::transaction::TransactionKey;
//! use rvoip_sip_core::Method;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create a transaction key
//! let tx_key = TransactionKey::new("z9hG4bK.123".to_string(), Method::Invite, false);
//!
//! // Create timer manager and factory
//! let timer_manager = Arc::new(TimerManager::new(None));
//! let timer_factory = TimerFactory::new(None, timer_manager.clone());
//!
//! // Schedule initial INVITE client timers (A and B)
//! timer_factory.schedule_invite_client_initial_timers(tx_key).await?;
//! # Ok(())
//! # }
//! ```

pub mod types;
pub mod manager;
pub mod factory;

// Re-export main items from submodules to make them accessible via `crate::timer::ItemName`
pub use types::{Timer, TimerType, TimerSettings};
pub use manager::TimerManager;
pub use factory::TimerFactory;

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use crate::transaction::TransactionKey; // For Timer::new_one_shot
    use rvoip_sip_core::Method; // For TransactionKey

    #[test]
    fn test_re_exports_exist_and_usable() {
        // Test TimerSettings re-export and usability
        let settings = TimerSettings::default();
        assert_eq!(settings.t1, Duration::from_millis(500));

        // Test TimerType re-export
        let timer_type_variant = TimerType::A;
        assert_eq!(timer_type_variant.to_string(), "A");

        // Test Timer struct re-export and basic usability
        let tx_key = TransactionKey::new("branch-mod-test".to_string(), Method::Invite, false);
        let timer_instance = Timer::new_one_shot("test", tx_key, Duration::from_secs(1));
        assert_eq!(timer_instance.name, "test");

        // TimerManager and TimerFactory can be constructed with defaults
        let _manager = TimerManager::default();
        let _factory = TimerFactory::default();
    }
} 