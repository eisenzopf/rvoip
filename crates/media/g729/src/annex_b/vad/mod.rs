#![allow(clippy::needless_range_loop)]
//! Provenance: Annex B VAD/DTX/CNG behavior adapted from ITU G.729 Annex B reference routines.
//! Q-format: VAD features, SID parameters, and CNG gains use Q0/Q13/Q15 fixed-point domains.

/// Public module `decision`.
pub mod decision;
/// Public module `detect`.
pub mod detect;
/// Public module `features`.
pub mod features;
mod features_update;
/// Public module `state`.
pub mod state;
/// Public re-export.
pub use state::VadState;

#[inline(always)]
#[allow(dead_code)]
pub(crate) fn make_dec(dsle: i16, dse: i16, sd: i16, dszc: i16) -> i16 {
    decision::make_dec_impl(dsle, dse, sd, dszc)
}
