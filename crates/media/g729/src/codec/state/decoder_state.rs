//! Provenance: Frame encode/decode pipeline derived from ITU G.729 Annex A/B reference flow.
//! Q-format: Speech, excitation, and LPC paths follow Q0/Q12/Q13/Q15 fixed-point stages.

use crate::constants::{L_FRAME, L_INTERPOL, L_SUBFR, M, MA_NP, PIT_MAX, SHARPMIN};

/// Public constant `OLD_EXC_LEN`.
pub const OLD_EXC_LEN: usize = L_FRAME + PIT_MAX as usize + L_INTERPOL;
/// Public constant `EXC_OFFSET`.
pub const EXC_OFFSET: usize = PIT_MAX as usize + L_INTERPOL;
/// Public constant `SYNTH_BUF_LEN`.
pub const SYNTH_BUF_LEN: usize = L_FRAME + M;
/// Public constant `RES2_BUF_LEN`.
pub const RES2_BUF_LEN: usize = PIT_MAX as usize + L_SUBFR;
/// Public constant `FREQ_PREV_RESET`.
pub const FREQ_PREV_RESET: [i16; M] = [
    2339, 4679, 7018, 9358, 11698, 14037, 16377, 18717, 21056, 23396,
];

/// Public struct `DecoderState`.
#[derive(Debug, Clone)]
pub struct DecoderState {
    pub old_exc: [i16; OLD_EXC_LEN],
    pub mem_syn: [i16; M],
    pub lsp_old: [i16; M],
    pub sharp: i16,
    pub old_t0: i16,
    pub gain_code: i16,
    pub gain_pitch: i16,

    pub freq_prev: [[i16; M]; MA_NP],
    pub prev_ma: i16,
    pub prev_lsp: [i16; M],
    pub bad_lsf: i16,

    pub past_qua_en: [i16; 4],
    pub rand_seed: i16,

    pub synth_buf: [i16; SYNTH_BUF_LEN],

    pub res2_buf: [i16; RES2_BUF_LEN],
    pub scal_res2_buf: [i16; RES2_BUF_LEN],
    pub mem_syn_pst: [i16; M],
    pub mem_pre: i16,
    pub past_gain: i16,

    pub pp_y2_hi: i16,
    pub pp_y2_lo: i16,
    pub pp_y1_hi: i16,
    pub pp_y1_lo: i16,
    pub pp_x0: i16,
    pub pp_x1: i16,

    pub post_filter_enabled: bool,
    pub frame_index: u32,
}

impl Default for DecoderState {
    fn default() -> Self {
        let mut freq_prev = [[0i16; M]; MA_NP];
        freq_prev.fill(FREQ_PREV_RESET);

        Self {
            old_exc: [0; OLD_EXC_LEN],
            mem_syn: [0; M],
            lsp_old: [
                30000, 26000, 21000, 15000, 8000, 0, -8000, -15000, -21000, -26000,
            ],
            sharp: SHARPMIN,
            old_t0: 60,
            gain_code: 0,
            gain_pitch: 0,

            freq_prev,
            prev_ma: 0,
            prev_lsp: FREQ_PREV_RESET,
            bad_lsf: 0,

            past_qua_en: [-14336; 4],
            rand_seed: 21845,

            synth_buf: [0; SYNTH_BUF_LEN],

            res2_buf: [0; RES2_BUF_LEN],
            scal_res2_buf: [0; RES2_BUF_LEN],
            mem_syn_pst: [0; M],
            mem_pre: 0,
            past_gain: 4096,

            pp_y2_hi: 0,
            pp_y2_lo: 0,
            pp_y1_hi: 0,
            pp_y1_lo: 0,
            pp_x0: 0,
            pp_x1: 0,

            post_filter_enabled: true,
            frame_index: 0,
        }
    }
}
