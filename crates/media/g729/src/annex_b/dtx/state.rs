//! Annex B DTX state and public entrypoints.
//! Provenance: Annex B VAD/DTX/CNG behavior adapted from ITU G.729 Annex B reference routines.
//! Q-format: VAD features, SID parameters, and CNG gains use Q0/Q13/Q15 fixed-point domains.

use super::{NB_CURACF, NB_GAIN, NB_SUMACF, SIZ_ACF, SIZ_SUMACF};
use crate::api::FrameType;
use crate::codec::state::EncoderState;
use crate::constants::{M, MP1};

/// Public struct `DtxState`.
#[derive(Debug, Clone)]
pub struct DtxState {
    pub(super) lsp_sid_q: [i16; M],
    pub(super) past_coeff: [i16; MP1],
    pub(super) r_coeff: [i16; MP1],
    pub(super) sh_r_coeff: i16,

    pub(super) acf: [i16; SIZ_ACF],
    pub(super) sh_acf: [i16; NB_CURACF],
    pub(super) sum_acf: [i16; SIZ_SUMACF],
    pub(super) sh_sum_acf: [i16; NB_SUMACF],

    pub(super) ener: [i16; NB_GAIN],
    pub(super) sh_ener: [i16; NB_GAIN],

    pub(super) fr_cur: i16,
    pub(super) cur_gain: i16,
    pub(super) nb_ener: i16,
    pub(super) sid_gain: i16,
    pub(super) flag_chang: i16,
    pub(super) prev_energy: i16,
    pub(super) count_fr0: i16,
}

impl Default for DtxState {
    fn default() -> Self {
        Self {
            lsp_sid_q: [0; M],
            past_coeff: [0; MP1],
            r_coeff: [0; MP1],
            sh_r_coeff: 0,

            acf: [0; SIZ_ACF],
            sh_acf: [40; NB_CURACF],
            sum_acf: [0; SIZ_SUMACF],
            sh_sum_acf: [40; NB_SUMACF],

            ener: [0; NB_GAIN],
            sh_ener: [40; NB_GAIN],

            fr_cur: 0,
            cur_gain: 0,
            nb_ener: 0,
            sid_gain: 0,
            flag_chang: 0,
            prev_energy: 0,
            count_fr0: 0,
        }
    }
}

impl DtxState {
    /// Public function `next_frame_type`.
    pub fn next_frame_type(&mut self, vad_voice: bool) -> FrameType {
        if vad_voice {
            FrameType::Speech
        } else if self.count_fr0 == 0 {
            FrameType::Sid
        } else {
            FrameType::NoData
        }
    }

    /// Public function `update_cng`.
    pub fn update_cng(&mut self, r_h: &[i16; MP1], exp_r: i16, vad: i16) {
        self.update_cng_impl(r_h, exp_r, vad);
    }

    /// Public function `cod_cng`.
    pub fn cod_cng(
        &mut self,
        state: &mut EncoderState,
        past_vad: i16,
        aq: &mut [i16; MP1 * 2],
        ana: &mut [i16; 5],
    ) {
        self.cod_cng_impl(state, past_vad, aq, ana);
    }
}
