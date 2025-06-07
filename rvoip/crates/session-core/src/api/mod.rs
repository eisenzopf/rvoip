//! Developer-facing API for RVOIP Session Core
//!
//! This module provides a clean, simple API for creating and managing SIP sessions.
//! It abstracts away the complexity of the underlying SIP protocol while providing
//! all the functionality needed for building SIP applications.

pub mod builder;
pub mod create;
pub mod control;
pub mod handlers;
pub mod types;
pub mod examples;

// Re-export all public APIs
pub use builder::*;
pub use create::*;
pub use control::*;
pub use handlers::*;
pub use types::*;

// Re-export core types that examples need
pub use crate::manager::SessionManager;
pub use crate::errors::{Result, SessionError}; 