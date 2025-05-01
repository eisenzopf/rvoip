// Headers module
//
// This module contains all the SIP header implementations.
// Each header type is implemented in its own file.

use crate::error::{Error, Result};
use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeader, TypedHeaderTrait};

// Re-export header types
// These will be expanded as we implement each header file
pub mod common;

// Re-export common types for convenience
pub use common::*; 