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
//!
//! See `MIGRATION_0_3.md` when migrating code that previously retrieved
//! plaintext credentials from `UserStore`.

pub mod api;
pub mod error;
pub mod events;
pub mod identity;
pub mod presence;
pub mod registrar;
pub mod types;

// Re-exports for convenience
pub use api::{RegistrarService, ServiceMode};
pub use error::{RegistrarError, Result};
pub use events::{PresenceEvent, RegistrarEvent};
pub use identity::{
    CredentialProvider, ExternalIdentity, IdentityProvider, IdentitySyncService,
    InMemoryIdentityProvider,
};
pub use registrar::{
    PlaintextCredentialUnavailable, Registrar, UserCredentialMetadata, UserCredentials, UserStore,
};
pub use types::{
    AddressOfRecord, BasicStatus, ContactInfo, ContactReachability, ExtendedStatus, PresenceState,
    PresenceStatus, RegistrarConfig, Subscription, SubscriptionState, Transport, UserRegistration,
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
