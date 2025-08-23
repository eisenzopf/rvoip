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

pub mod registrar;
pub mod presence;
pub mod api;
pub mod types;
pub mod error;
pub mod events;

// Re-exports for convenience
pub use api::RegistrarService;
pub use types::{
    UserRegistration, ContactInfo, Transport,
    PresenceState, PresenceStatus, BasicStatus, ExtendedStatus,
    Subscription, SubscriptionState,
};
pub use error::{RegistrarError, Result};
pub use events::{RegistrarEvent, PresenceEvent};

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