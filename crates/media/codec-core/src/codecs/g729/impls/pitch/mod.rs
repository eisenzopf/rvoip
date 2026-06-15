//! Provenance: Pitch search and lag coding adapted from ITU G.729 open/closed-loop pitch routines.
//! Q-format: Lag correlations and adaptive codebook gains use Q0/Q15 accumulator scaling.

pub(crate) mod closed_loop;
mod lag_decode;
pub(crate) mod lag_encode;
pub(crate) mod open_loop;
mod open_loop_search;
mod parity;
pub(crate) mod pred_lt3;

/// Public re-export.
pub use lag_decode::dec_lag3;
/// Public re-export.
pub use parity::{check_parity_pitch, parity_pitch};
