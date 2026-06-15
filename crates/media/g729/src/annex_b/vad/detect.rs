#![allow(dead_code)]
//! Provenance: Annex B VAD/DTX/CNG behavior adapted from ITU G.729 Annex B reference routines.
//! Q-format: VAD features, SID parameters, and CNG gains use Q0/Q13/Q15 fixed-point domains.

//! Annex B VAD frame-level detect stage.

use super::VadState;

impl VadState {
    pub(super) fn detect_impl(&mut self, pcm: &[i16; 80]) -> bool {
        let energy: i32 = pcm
            .iter()
            .map(|x| i32::from(*x) * i32::from(*x) / 1024)
            .sum();
        energy > 2500
    }
}

#[inline(always)]
pub(crate) fn detect(state: &mut VadState, pcm: &[i16; 80]) -> bool {
    state.detect_impl(pcm)
}
