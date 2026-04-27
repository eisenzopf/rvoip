// Session module for SDP parsing
//
// This module handles all session-level SDP parsing, including:
// - Origin information (o=)
// - Connection data (c=)
// - Bandwidth information (b=)
// - Validation of hostnames, usernames, and addresses
// - Utility functions for session parsing

mod bandwidth;
mod connection;
mod origin;
mod utils;
pub mod validation;

// Re-export public API
pub use self::bandwidth::*;
pub use self::connection::*;
pub use self::origin::*;
pub use self::utils::*;
pub use self::validation::*;
