//! G.711 Test Module
//!
//! This module contains comprehensive tests for the G.711 codec implementation,
//! including unit tests, integration tests, and ITU-T compliance validation.

pub mod algorithm_verification;
pub mod decoder_tests;
pub mod encoder_tests;
pub mod itu_test_standalone;
pub mod itu_validation_tests;
pub mod library_tests;
pub mod quick_itu_test;
pub mod wav_roundtrip_test;
