//! Message validation for SIP methods
//!
//! This module provides validation functions for various SIP methods,
//! ensuring they comply with RFC requirements.

pub mod presence;

pub use presence::{
    validate_publish_request,
    validate_subscribe_request,
    validate_notify_request,
};