//! Gain modules.
//! Provenance: Gain prediction/quantization adapted from ITU G.729 gain predictor and quantizer flow.
//! Q-format: Pitch/code gains and predictors use Q14/Q15 arithmetic with 32-bit intermediates.

/// Public module `decode`.
pub mod decode;
/// Public module `predict`.
pub mod predict;
pub(crate) mod presel;
/// Public module `quantize`.
pub mod quantize;
pub(crate) mod quantize_prepare;
/// Public module `taming`.
pub mod taming;
