//! Request and response routing
//!
//! This module handles routing of SIP messages to appropriate dialogs.

pub mod request_router;
pub mod response_router;
pub mod dialog_matcher;

// TODO: Re-export main types 