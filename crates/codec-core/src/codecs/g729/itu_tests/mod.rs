//! G.729 ITU-T Compliance Test Suite (Simplified)
//!
//! This module contains ITU-T G.729 compliance tests that work with the current implementation.

#[allow(missing_docs)]

// ITU test data parsing utilities and compliance framework
pub mod itu_test_utils;

// Basic ITU compliance tests that work with current implementation
pub mod basic_itu_test;
pub mod quality_evaluation_tests;
pub mod synthesis_debug_test; 