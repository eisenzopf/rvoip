//! Annex B DTX encoding stage.
//! Provenance: Annex B VAD/DTX/CNG behavior adapted from ITU G.729 Annex B reference routines.
//! Q-format: VAD features, SID parameters, and CNG gains use Q0/Q13/Q15 fixed-point domains.

use super::dtx_gain::update_cur_gain;
use super::{DtxState, FR_SID_MIN, FRAC_THRESH1, FRAC_THRESH2, NB_CURACF, NB_GAIN, w};
use crate::annex_b::cng::{calc_exc_rand, lsfq_noise, qua_sidgain};
use crate::codec::state::{ENC_EXC_OFFSET, EncoderState};
use crate::constants::{M, MP1};
use crate::dsp::arith::{abs_s, add, sub};
use crate::dsp::types::DspContext;
use crate::lp::{az_lsp::az_lsp, interp::int_qlpc};
use crate::tables::sid::TAB_SIDGAIN;

impl DtxState {
    pub(super) fn cod_cng_impl(
        &mut self,
        state: &mut EncoderState,
        past_vad: i16,
        aq: &mut [i16; MP1 * 2],
        ana: &mut [i16; 5],
    ) {
        let mut ctx = DspContext::default();

        for i in (1..NB_GAIN).rev() {
            self.ener[i] = self.ener[i - 1];
            self.sh_ener[i] = self.sh_ener[i - 1];
        }

        let mut cur_acf = [0i16; MP1];
        let mut sh_cur = 0i16;
        super::stationarity::calc_sum_acf_impl(
            &self.acf,
            &self.sh_acf,
            &mut cur_acf,
            &mut sh_cur,
            NB_CURACF,
        );
        self.sh_ener[0] = sh_cur;

        let mut cur_coeff = [0i16; MP1];
        if cur_acf[0] == 0 {
            self.ener[0] = 0;
        } else {
            let r_l = [0i16; MP1];
            let mut bid = [0i16; M];
            let mut err = 0i16;
            crate::lp::levinson::levinson_10(
                state,
                &cur_acf,
                &r_l,
                &mut cur_coeff,
                &mut bid,
                Some(&mut err),
            );
            self.ener[0] = err;
        }

        let mut cur_igain = 0i16;
        let mut energyq = 0i16;

        if past_vad != 0 {
            ana[0] = 2;
            self.count_fr0 = 0;
            self.nb_ener = 1;
            qua_sidgain(
                &self.ener,
                &self.sh_ener,
                self.nb_ener,
                &mut energyq,
                &mut cur_igain,
            );
        } else {
            self.nb_ener = add(&mut ctx, w(self.nb_ener), w(1)).0;
            if sub(&mut ctx, w(self.nb_ener), w(NB_GAIN as i16)).0 > 0 {
                self.nb_ener = NB_GAIN as i16;
            }

            qua_sidgain(
                &self.ener,
                &self.sh_ener,
                self.nb_ener,
                &mut energyq,
                &mut cur_igain,
            );

            if super::stationarity::cmp_filt_impl(
                &self.r_coeff,
                self.sh_r_coeff,
                &cur_acf,
                self.ener[0],
                FRAC_THRESH1,
            ) != 0
            {
                self.flag_chang = 1;
            }

            let diff_e = sub(&mut ctx, w(self.prev_energy), w(energyq)).0;
            let mut temp = abs_s(&mut ctx, w(diff_e)).0;
            temp = sub(&mut ctx, w(temp), w(2)).0;
            if temp > 0 {
                self.flag_chang = 1;
            }

            self.count_fr0 = add(&mut ctx, w(self.count_fr0), w(1)).0;
            if sub(&mut ctx, w(self.count_fr0), w(FR_SID_MIN)).0 < 0 {
                ana[0] = 0;
            } else {
                ana[0] = if self.flag_chang != 0 { 2 } else { 0 };
                self.count_fr0 = FR_SID_MIN;
            }
        }

        if ana[0] == 2 {
            self.count_fr0 = 0;
            self.flag_chang = 0;

            self.calc_pastfilt_impl(state);
            super::stationarity::calc_rcoeff_impl(
                &self.past_coeff,
                &mut self.r_coeff,
                &mut self.sh_r_coeff,
            );

            let lpc_coeff = if super::stationarity::cmp_filt_impl(
                &self.r_coeff,
                self.sh_r_coeff,
                &cur_acf,
                self.ener[0],
                FRAC_THRESH2,
            ) == 0
            {
                self.past_coeff
            } else {
                super::stationarity::calc_rcoeff_impl(
                    &cur_coeff,
                    &mut self.r_coeff,
                    &mut self.sh_r_coeff,
                );
                cur_coeff
            };

            let mut lsp_new = [0i16; M];
            az_lsp(&lpc_coeff, &mut lsp_new, &state.lsp_old_q);

            let mut sid_idx = [0i16; 3];
            lsfq_noise(
                &lsp_new,
                &mut self.lsp_sid_q,
                &mut state.freq_prev,
                &mut sid_idx,
            );
            ana[1] = sid_idx[0];
            ana[2] = sid_idx[1];
            ana[3] = sid_idx[2];

            self.prev_energy = energyq;
            ana[4] = cur_igain;
            self.sid_gain = TAB_SIDGAIN[cur_igain as usize];
        }

        self.cur_gain = update_cur_gain(&mut ctx, self.cur_gain, self.sid_gain, past_vad);

        calc_exc_rand(
            self.cur_gain,
            &mut state.old_exc,
            ENC_EXC_OFFSET,
            &mut state.seed,
            true,
            Some(&mut state.l_exc_err),
        );

        int_qlpc(&state.lsp_old_q, &self.lsp_sid_q, aq);
        state.lsp_old_q = self.lsp_sid_q;

        if self.fr_cur == 0 {
            self.update_sum_acf_impl();
        }
    }
}
