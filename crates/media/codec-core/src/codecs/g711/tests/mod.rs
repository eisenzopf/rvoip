//! G.711 Test Module
//!
//! This module contains comprehensive tests for the G.711 codec implementation,
//! including unit tests, integration tests, and ITU-T compliance validation.

// Test vectors intentionally exercise lossy numeric conversions and print
// detailed diagnostics. Keep these test-only exceptions scoped to this module;
// production G.711 code remains under the full lint policy.
#![allow(
    clippy::cast_abs_to_unsigned,
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::checked_conversions,
    clippy::doc_markdown,
    clippy::float_cmp,
    clippy::needless_range_loop,
    clippy::suboptimal_flops,
    clippy::too_many_lines,
    clippy::uninlined_format_args,
    clippy::useless_vec
)]

pub mod algorithm_verification;
pub mod decoder_tests;
pub mod encoder_tests;
pub mod itu_test_standalone;
pub mod itu_validation_tests;
pub mod library_tests;
pub mod quick_itu_test;
pub mod tone_quality_tests;
pub mod wav_roundtrip_test;
