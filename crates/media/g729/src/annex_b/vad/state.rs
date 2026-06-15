//! Provenance: Annex B VAD/DTX/CNG behavior adapted from ITU G.729 Annex B reference routines.
//! Q-format: VAD features, SID parameters, and CNG gains use Q0/Q13/Q15 fixed-point domains.

use crate::constants::{M, NP};
use crate::dsp::types::MAX_16;

/// Public struct `VadState`.
#[derive(Debug, Clone)]
pub struct VadState {
    pub(super) mean_lsf: [i16; M],
    pub(super) min_buffer: [i16; 16],
    pub(super) prev_min: i16,
    pub(super) next_min: i16,
    pub(super) min: i16,
    pub(super) mean_e: i16,
    pub(super) mean_se: i16,
    pub(super) mean_sle: i16,
    pub(super) mean_szc: i16,
    pub(super) prev_energy: i16,
    pub(super) count_sil: i16,
    pub(super) count_update: i16,
    pub(super) count_ext: i16,
    pub(super) flag: i16,
    pub(super) v_flag: i16,
    pub(super) less_count: i16,
}

impl Default for VadState {
    fn default() -> Self {
        Self {
            mean_lsf: [0; M],
            min_buffer: [0; 16],
            prev_min: 0,
            next_min: 0,
            min: MAX_16,
            mean_e: 0,
            mean_se: 0,
            mean_sle: 0,
            mean_szc: 0,
            prev_energy: 0,
            count_sil: 0,
            count_update: 0,
            count_ext: 0,
            flag: 1,
            v_flag: 0,
            less_count: 0,
        }
    }
}

impl VadState {
    /// Public function `detect`.
    pub fn detect(&mut self, pcm: &[i16; 80]) -> bool {
        self.detect_impl(pcm)
    }

    /// Public function `detect_from_analysis`.
    #[allow(clippy::too_many_arguments)]
    pub fn detect_from_analysis(
        &mut self,
        rc: i16,
        lsf: &[i16; M],
        r_h: &[i16; NP + 1],
        r_l: &[i16; NP + 1],
        exp_r0: i16,
        sigpp: &[i16],
        frm_count: i16,
        prev_marker: i16,
        pprev_marker: i16,
    ) -> i16 {
        self.detect_from_analysis_impl(
            rc,
            lsf,
            r_h,
            r_l,
            exp_r0,
            sigpp,
            frm_count,
            prev_marker,
            pprev_marker,
        )
    }
}
