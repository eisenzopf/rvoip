//! # Registrar Core
//!
//! A high-performance SIP Registrar and Presence Server for the rvoip ecosystem.
//!
//! This crate provides:
//! - User registration management (REGISTER)
//! - Location services (contact bindings)  
//! - Presence state management (PUBLISH)
//! - Subscription handling (SUBSCRIBE/NOTIFY)
//! - Automatic buddy lists for registered users

pub mod api;
pub mod error;
pub mod events;
pub mod presence;
pub mod registrar;
pub mod types;

// Re-exports for convenience
pub use api::RegistrarService;
pub use error::{RegistrarError, Result};
pub use events::{PresenceEvent, RegistrarEvent};
pub use registrar::{UserCredentials, UserStore};
pub use types::{
    BasicStatus, ContactInfo, ExtendedStatus, PresenceState, PresenceStatus, Subscription,
    SubscriptionState, Transport, UserRegistration,
};

// Version information
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version() {
        assert!(!VERSION.is_empty());
    }
}
