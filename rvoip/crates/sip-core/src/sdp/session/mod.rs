// Session module for SDP parsing
//
// This module handles all session-level SDP parsing, including:
// - Origin information (o=)
// - Connection data (c=)
// - Bandwidth information (b=)
// - Validation of hostnames, usernames, and addresses
// - Utility functions for session parsing

pub mod validation;
mod origin;
mod connection;
mod bandwidth;
mod utils;

// Re-export public API
pub use self::validation::*;
pub use self::origin::*;
pub use self::connection::*;
pub use self::bandwidth::*;
pub use self::utils::*; 