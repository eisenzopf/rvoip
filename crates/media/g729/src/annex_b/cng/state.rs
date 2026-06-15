//! Annex B CNG state and lifecycle helpers.
//! Provenance: Annex B VAD/DTX/CNG behavior adapted from ITU G.729 Annex B reference routines.
//! Q-format: VAD features, SID parameters, and CNG gains use Q0/Q13/Q15 fixed-point domains.

use super::{calc_exc_rand, w};
use crate::constants::{L_FRAME, M, PIT_MAX};
use crate::dsp::arith::{round, sub};
use crate::dsp::arith32::l_mac;
use crate::dsp::shift::{l_shl, norm_l};
use crate::dsp::types::{DspContext, Word32};
use crate::tables::sid::TAB_SIDGAIN;

/// Public struct `CngState`.
#[derive(Debug, Clone)]
pub struct CngState {
    pub cur_gain: i16,
    pub sid_gain: i16,
    pub lsp_sid: [i16; M],
    pub seed: i16,
    pub past_ftyp: i16,
    pub sid_sav: i16,
    pub sh_sid_sav: i16,
}

impl Default for CngState {
    fn default() -> Self {
        Self {
            cur_gain: 0,
            sid_gain: TAB_SIDGAIN[0],
            lsp_sid: [
                31441, 27566, 21458, 13612, 4663, -4663, -13612, -21458, -27566, -31441,
            ],
            seed: 11111,
            past_ftyp: 1,
            sid_sav: 0,
            sh_sid_sav: 1,
        }
    }
}

impl CngState {
    /// Public function `reset`.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Public function `set_gain`.
    pub fn set_gain(&mut self, gain: i16) {
        self.sid_gain = gain.max(2);
        self.cur_gain = self.sid_gain;
    }

    /// Public function `generate_frame`.
    pub fn generate_frame(&mut self) -> [i16; L_FRAME] {
        let mut old_exc = [0i16; L_FRAME + PIT_MAX as usize + crate::constants::L_INTERPOL];
        let exc_index = PIT_MAX as usize + crate::constants::L_INTERPOL;
        calc_exc_rand(
            self.cur_gain,
            &mut old_exc,
            exc_index,
            &mut self.seed,
            false,
            None,
        );
        let mut out = [0i16; L_FRAME];
        out.copy_from_slice(&old_exc[exc_index..exc_index + L_FRAME]);
        out
    }

    /// Public function `update_sid_energy`.
    pub fn update_sid_energy(&mut self, exc: &[i16; L_FRAME], bfi: i16) {
        if bfi != 0 {
            return;
        }
        let mut ctx = DspContext::default();
        let mut l_acc = Word32(0);
        for &e in exc {
            l_acc = l_mac(&mut ctx, l_acc, w(e), w(e));
        }
        let n = norm_l(l_acc);
        let l_acc_n = l_shl(&mut ctx, l_acc, n);
        self.sid_sav = round(&mut ctx, l_acc_n).0;
        self.sh_sid_sav = sub(&mut ctx, w(16), w(n)).0;
    }
}
