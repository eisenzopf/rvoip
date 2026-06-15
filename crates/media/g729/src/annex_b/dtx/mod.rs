//! Provenance: Annex B VAD/DTX/CNG behavior adapted from ITU G.729 Annex B reference routines.
//! Q-format: VAD features, SID parameters, and CNG gains use Q0/Q13/Q15 fixed-point domains.

mod dtx_gain;
/// Public module `encode`.
pub mod encode;
/// Public module `state`.
pub mod state;
/// Public module `stationarity`.
pub mod stationarity;
mod stationarity_cmp;

/// Public re-export.
pub use state::DtxState;

use crate::constants::MP1;
use crate::dsp::types::Word16;

pub(super) const NB_SUMACF: usize = 3;
pub(super) const NB_CURACF: usize = 2;
pub(super) const NB_GAIN: usize = 2;
pub(super) const FR_SID_MIN: i16 = 3;
pub(super) const FRAC_THRESH1: i16 = 4855;
pub(super) const FRAC_THRESH2: i16 = 3161;
pub(super) const SIZ_SUMACF: usize = NB_SUMACF * MP1;
pub(super) const SIZ_ACF: usize = NB_CURACF * MP1;

#[inline(always)]
pub(super) fn w(v: i16) -> Word16 {
    Word16(v)
}
