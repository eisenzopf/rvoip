//! Provenance: Fixed codebook search/decode adapted from ITU G.729 ACELP algebraic codebook routines.
//! Q-format: Correlation buffers and pulse gains use Q13/Q15 with 32-bit fixed-point accumulators.

pub(crate) mod build_code;
pub(crate) mod cor_h;
mod cor_h_cross_a;
mod cor_h_cross_b;
pub(crate) mod correlation;
pub(crate) mod d4i;
mod d4i_finalize;
mod d4i_prepare;
mod d4i_state;
mod d4i_variant_a;
mod d4i_variant_b;
mod decode;
pub(crate) mod search;
pub(crate) mod search_indices;

/// Public re-export.
pub use decode::decod_acelp;
