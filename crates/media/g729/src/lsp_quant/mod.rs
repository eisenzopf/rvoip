//! LSP quantization modules.
//! Provenance: LSP quantization/dequantization adapted from ITU G.729 LSP codebook routines.
//! Q-format: LSP/LSF vectors and prediction weights use Q13/Q15 fixed-point representation.

/// Public module `decode`.
pub mod decode;
/// Public module `encode`.
pub mod encode;
/// Public module `helpers`.
pub mod helpers;
/// Public module `prev`.
pub mod prev;
/// Public module `stability`.
pub mod stability;
