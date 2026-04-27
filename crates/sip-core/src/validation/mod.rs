//! Message validation for SIP methods
//!
//! This module provides validation functions for various SIP methods,
//! ensuring they comply with RFC requirements.

#[cfg(any(test, feature = "generated-validation"))]
pub mod generated;
pub mod presence;
pub mod wire;

pub use presence::{validate_notify_request, validate_publish_request, validate_subscribe_request};

#[cfg(any(test, feature = "generated-validation"))]
pub use generated::{
    validate_generated_message, validate_generated_request, validate_generated_response,
};
pub use wire::{validate_content_length, validate_wire_request, validate_wire_response};
