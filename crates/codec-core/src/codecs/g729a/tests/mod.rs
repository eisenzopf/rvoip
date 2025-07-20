//! G.729A Test Module
//!
//! This module contains comprehensive tests for the G.729A codec implementation,
//! including unit tests, integration tests, and ITU compliance tests using 
//! official test vectors.

pub mod test_utils;
pub mod unit_tests;
pub mod integration_tests; 
pub mod itu_compliance_tests;
pub mod performance_tests;

// Re-export for convenience
pub use test_utils::*;
pub use unit_tests::*;
pub use integration_tests::*;
pub use itu_compliance_tests::*;
pub use performance_tests::*; 