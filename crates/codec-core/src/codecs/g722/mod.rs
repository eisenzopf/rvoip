//! G.722 Wideband Audio Codec Implementation
//!
//! This module implements the G.722 codec according to ITU-T Recommendation G.722.
//! It provides a clean, reference-compliant implementation with proper QMF filters
//! and ADPCM encoding/decoding.
//!
//! # Architecture
//!
//! The implementation is split into several modules:
//! - `codec`: High-level codec implementation
//! - `qmf`: QMF analysis and synthesis filters
//! - `adpcm`: ADPCM encoding/decoding algorithms
//! - `tables`: Quantization tables and constants
//! - `state`: State management structures
//! - `reference`: ITU-T reference functions for compliance
//!
//! # Reference
//!
//! Based on ITU-T G.722 Annex E (Release 3.00, 2014-11) reference implementation.

pub mod codec;
pub mod qmf;
pub mod adpcm;
pub mod tables;
pub mod state;
pub mod reference;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod itu_reference_tests;

// Re-export the main codec struct
pub use codec::G722Codec;

// Re-export key types
pub use state::{G722State, AdpcmState};

// Re-export ITU-T reference functions for compliance testing
pub use reference::*; 