// Headers module
//
// This module contains all the SIP header implementations.
// Each header type is implemented in its own file.

use crate::error::{Error, Result};
use crate::types::header::{Header, TypedHeader, TypedHeaderTrait};

// Define sub-modules
pub mod header_name;
pub mod header_value;
pub mod common;
pub mod header_access;

// Tests
#[cfg(test)]
mod tests;

// Re-export common types for convenience
pub use common::*;
pub use header_access::*;
pub use header_name::HeaderName;
pub use header_value::HeaderValue; 