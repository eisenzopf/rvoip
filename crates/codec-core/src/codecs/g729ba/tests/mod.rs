//! G.729BA Test Module
//!
//! This module contains comprehensive tests for the G.729BA codec implementation,
//! including unit tests and ITU compliance tests using official test vectors.

pub mod unit_tests;
pub mod itu_compliance_tests;
pub mod test_utils;

// Re-export test functions for easy access
pub use unit_tests::*;
pub use itu_compliance_tests::*;
pub use test_utils::*; 