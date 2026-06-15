#![allow(clippy::collapsible_if)]
#![allow(clippy::needless_range_loop)]
#![allow(clippy::too_many_arguments)]
//! Provenance: Annex B VAD/DTX/CNG behavior adapted from ITU G.729 Annex B reference routines.
//! Q-format: VAD features, SID parameters, and CNG gains use Q0/Q13/Q15 fixed-point domains.

/// Public module `decode`.
pub mod decode;
/// Public module `excitation`.
pub mod excitation;
mod excitation_helpers;
mod excitation_params;
mod excitation_subframe;
/// Public module `sid`.
pub mod sid;
mod sid_lsp;
mod sid_lsp_prev;
mod sid_quant;
mod sid_search;
/// Public module `state`.
pub mod state;

/// Public re-export.
pub use excitation::calc_exc_rand;
/// Public re-export.
pub use sid::{lsfq_noise, qua_sidgain, sid_lsfq_decode};
/// Public re-export.
pub use state::CngState;

use crate::dsp::types::Word16;

#[inline(always)]
pub(super) fn w(v: i16) -> Word16 {
    Word16(v)
}
