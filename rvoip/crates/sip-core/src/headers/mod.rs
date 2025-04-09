//! Structured SIP header types.
//! 
//! This module contains structured representations of SIP headers,
//! allowing for more type-safe and ergonomic access to header values.

mod via;

pub use via::{Via, ViaParams}; 