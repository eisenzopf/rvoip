//! SIP Protocol Handlers
//!
//! This module contains handlers for specific SIP methods and protocol operations.
//! Each handler module implements the logic for a specific SIP method according
//! to the relevant RFC specifications.
//!
//! ## Handler Modules
//!
//! - [`invite_handler`]: INVITE requests (RFC 3261 Section 14) - dialog creation and session establishment
//! - [`bye_handler`]: BYE requests (RFC 3261 Section 15) - dialog termination
//! - [`response_handler`]: SIP responses (RFC 3261) - response processing and dialog state transitions
//! - [`update_handler`]: UPDATE requests (RFC 3311) - session modification within dialogs
//! - [`register_handler`]: REGISTER requests (RFC 3261 Section 10) - endpoint registration
//!
//! ## Usage Pattern
//!
//! Each handler provides:
//! - A trait defining the handler interface
//! - Implementation of the trait for DialogManager
//! - Helper methods for specific processing logic

pub mod invite_handler;
pub mod bye_handler;
pub mod register_handler;
pub mod update_handler;
pub mod response_handler;

// Re-export handler traits for external use
pub use invite_handler::InviteHandler;
pub use bye_handler::ByeHandler;
pub use response_handler::ResponseHandler;
pub use update_handler::UpdateHandler;
pub use register_handler::RegisterHandler; 