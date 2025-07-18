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
pub mod energy_preservation_test;
pub mod gain_debug_test;
pub mod debug_gain_computation_test;

// G.729 Annex A (reduced complexity) compliance tests
pub mod itu_annex_a_tests;

// G.729 Annex B (VAD/DTX/CNG) compliance tests  
pub mod itu_annex_b_tests;

// Comprehensive ITU integration tests (encoder, decoder, variants)
// TODO: Fix remaining import issues  
// pub mod itu_encoder_tests;
// pub mod itu_decoder_tests;
// pub mod itu_integration_tests; 