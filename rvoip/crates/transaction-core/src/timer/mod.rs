//! Provides timer functionalities for SIP transactions, including management of
//! timer lifecycles, standard timer configurations, and factories for creating timers.
//!
//! This module re-exports key components:
//! - [`Timer`]: Represents an active timer instance with its properties.
//! - [`TimerType`]: Enumerates specific SIP transaction timers (e.g., A, B, G) and generic types.
//! - [`TimerSettings`]: Configuration for standard timer durations (T1, T2, etc.).
//! - [`TimerManager`]: Manages the execution of timers and dispatches events.
//! - [`TimerFactory`]: Simplifies the creation of standard RFC 3261 timers.

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