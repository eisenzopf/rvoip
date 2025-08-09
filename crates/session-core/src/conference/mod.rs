//! Conference Module for Session-Core
//!
//! Provides multi-session coordination for conference scenarios.
//! This module bridges the gap between individual session management
//! and conference room coordination.

pub mod types;
pub mod participant;
pub mod room;
pub mod manager;
pub mod api;
// pub mod coordinator; // Disabled - uses old SessionManager
pub mod events;

// Re-export main types and interfaces
pub use types::*;
pub use participant::ConferenceParticipant;
pub use room::ConferenceRoom;
pub use manager::ConferenceManager;
pub use api::ConferenceApi;
// pub use coordinator::ConferenceCoordinator; // Disabled - uses old SessionManager
pub use events::*;

// Convenience imports for users
pub mod prelude {
    pub use super::{
        ConferenceId,
        ConferenceConfig,
        ConferenceManager,
        ConferenceRoom,
        ConferenceParticipant,
        ConferenceApi,
        ConferenceEvent,
    };
    pub use crate::api::types::SessionId;
} 