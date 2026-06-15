//! Annex B comfort-noise decoding helpers.
//! Provenance: Annex B VAD/DTX/CNG behavior adapted from ITU G.729 Annex B reference routines.
//! Q-format: VAD features, SID parameters, and CNG gains use Q0/Q13/Q15 fixed-point domains.

use super::{CngState, calc_exc_rand, qua_sidgain, sid_lsfq_decode, w};
use crate::constants::{M, MA_NP};
use crate::dsp::arith::{add, mult_r, sub};
use crate::dsp::types::DspContext;
use crate::tables::sid::{A_GAIN0, A_GAIN1, TAB_SIDGAIN};

impl CngState {
    /// Public function `dec_cng`.
    pub fn dec_cng(
        &mut self,
        parm: &[i16; 5],
        old_exc: &mut [i16],
        exc_index: usize,
        _lsp_old: &mut [i16; M],
        freq_prev: &mut [[i16; M]; MA_NP],
    ) {
        let mut ctx = DspContext::default();
        let dif = sub(&mut ctx, w(self.past_ftyp), w(1)).0;

        if parm[0] != 0 {
            self.sid_gain = TAB_SIDGAIN[parm[4] as usize];
            sid_lsfq_decode(&parm[1..4], &mut self.lsp_sid, freq_prev);
        } else if dif == 0 {
            let mut tmp_ener = 0i16;
            let mut ind = 0i16;
            let e = [self.sid_sav, 0];
            let sh = [self.sh_sid_sav, 0];
            qua_sidgain(&e, &sh, 0, &mut tmp_ener, &mut ind);
            self.sid_gain = TAB_SIDGAIN[ind as usize];
        }

        if dif == 0 {
            self.cur_gain = self.sid_gain;
        } else {
            self.cur_gain = mult_r(&mut ctx, w(self.cur_gain), w(A_GAIN0)).0;
            let sid_part = mult_r(&mut ctx, w(self.sid_gain), w(A_GAIN1));
            self.cur_gain = add(&mut ctx, w(self.cur_gain), sid_part).0;
        }

        calc_exc_rand(
            self.cur_gain,
            old_exc,
            exc_index,
            &mut self.seed,
            false,
            None,
        );

        self.past_ftyp = parm[0];
    }
}
