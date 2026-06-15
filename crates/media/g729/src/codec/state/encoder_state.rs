//! Provenance: Frame encode/decode pipeline derived from ITU G.729 Annex A/B reference flow.
//! Q-format: Speech, excitation, and LPC paths follow Q0/Q12/Q13/Q15 fixed-point stages.

use crate::constants::{
    L_FRAME, L_INTERPOL, L_NEXT, L_TOTAL, L_WINDOW, M, MA_NP, MP1, PIT_MAX, SHARPMIN,
};
use crate::tables::annexa::FREQ_PREV_RESET;

/// Public constant `OLD_WSP_LEN`.
pub const OLD_WSP_LEN: usize = L_FRAME + PIT_MAX as usize;
/// Public constant `OLD_EXC_LEN`.
pub const OLD_EXC_LEN: usize = L_FRAME + PIT_MAX as usize + L_INTERPOL;

/// Public constant `NEW_SPEECH_OFFSET`.
pub const NEW_SPEECH_OFFSET: usize = L_TOTAL - L_FRAME;
/// Public constant `SPEECH_OFFSET`.
pub const SPEECH_OFFSET: usize = L_TOTAL - L_FRAME - L_NEXT;
/// Public constant `P_WINDOW_OFFSET`.
pub const P_WINDOW_OFFSET: usize = L_TOTAL - L_WINDOW;
/// Public constant `WSP_OFFSET`.
pub const WSP_OFFSET: usize = PIT_MAX as usize;
/// Public constant `EXC_OFFSET`.
pub const EXC_OFFSET: usize = PIT_MAX as usize + L_INTERPOL;

/// Public struct `EncoderState`.
#[derive(Debug, Clone)]
pub struct EncoderState {
    pub old_speech: [i16; L_TOTAL],
    pub old_wsp: [i16; OLD_WSP_LEN],
    pub old_exc: [i16; OLD_EXC_LEN],

    pub lsp_old: [i16; M],
    pub lsp_old_q: [i16; M],
    pub freq_prev: [[i16; M]; MA_NP],

    pub mem_w0: [i16; M],
    pub mem_w: [i16; M],
    pub mem_zero: [i16; M],
    pub sharp: i16,

    pub past_qua_en: [i16; 4],
    pub l_exc_err: [i32; 4],

    pub old_a: [i16; MP1],
    pub old_rc: [i16; 2],

    pub pp_y2_hi: i16,
    pub pp_y2_lo: i16,
    pub pp_y1_hi: i16,
    pub pp_y1_lo: i16,
    pub pp_x0: i16,
    pub pp_x1: i16,

    pub past_vad: i16,
    pub ppast_vad: i16,
    pub seed: i16,
    pub frame: i16,
}

impl Default for EncoderState {
    fn default() -> Self {
        let lsp_init = [
            30000, 26000, 21000, 15000, 8000, 0, -8000, -15000, -21000, -26000,
        ];
        let mut freq_prev = [[0i16; M]; MA_NP];
        freq_prev.fill(FREQ_PREV_RESET);

        Self {
            old_speech: [0; L_TOTAL],
            old_wsp: [0; OLD_WSP_LEN],
            old_exc: [0; OLD_EXC_LEN],

            lsp_old: lsp_init,
            lsp_old_q: lsp_init,
            freq_prev,

            mem_w0: [0; M],
            mem_w: [0; M],
            mem_zero: [0; M],
            sharp: SHARPMIN,

            past_qua_en: [-14336; 4],
            l_exc_err: [0x0000_4000; 4],

            old_a: [4096, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            old_rc: [0, 0],

            pp_y2_hi: 0,
            pp_y2_lo: 0,
            pp_y1_hi: 0,
            pp_y1_lo: 0,
            pp_x0: 0,
            pp_x1: 0,

            past_vad: 1,
            ppast_vad: 1,
            seed: 11111,
            frame: 0,
        }
    }
}
