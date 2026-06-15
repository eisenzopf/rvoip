#![allow(dead_code)]
//! Provenance: Annex B VAD/DTX/CNG behavior adapted from ITU G.729 Annex B reference routines.
//! Q-format: VAD features, SID parameters, and CNG gains use Q0/Q13/Q15 fixed-point domains.

//! Annex B DTX stationarity and ACF processing.

pub(super) use super::stationarity_cmp::cmp_filt_impl;
use super::{DtxState, NB_CURACF, NB_SUMACF, SIZ_ACF, SIZ_SUMACF, w};
use crate::codec::state::EncoderState;
use crate::constants::{M, MP1};
use crate::dsp::arith::{add, extract_h, round, sub};
use crate::dsp::arith32::{l_add, l_deposit_l, l_mac};
use crate::dsp::shift::{l_shl, norm_l};
use crate::dsp::types::{DspContext, Word32};
use crate::lp::levinson::levinson_10;

impl DtxState {
    pub(super) fn update_cng_impl(&mut self, r_h: &[i16; MP1], exp_r: i16, vad: i16) {
        let mut ctx = DspContext::default();

        for i in (MP1..SIZ_ACF).rev() {
            self.acf[i] = self.acf[i - MP1];
        }
        for i in (1..NB_CURACF).rev() {
            self.sh_acf[i] = self.sh_acf[i - 1];
        }

        self.sh_acf[0] = negate_i16(add(&mut ctx, w(16), w(exp_r)).0);
        self.acf[..MP1].copy_from_slice(r_h);

        self.fr_cur = add(&mut ctx, w(self.fr_cur), w(1)).0;
        if sub(&mut ctx, w(self.fr_cur), w(NB_CURACF as i16)).0 == 0 {
            self.fr_cur = 0;
            if vad != 0 {
                self.update_sum_acf_impl();
            }
        }
    }

    pub(super) fn update_sum_acf_impl(&mut self) {
        for i in (MP1..SIZ_SUMACF).rev() {
            self.sum_acf[i] = self.sum_acf[i - MP1];
        }
        for i in (1..NB_SUMACF).rev() {
            self.sh_sum_acf[i] = self.sh_sum_acf[i - 1];
        }

        let mut tmp = [0i16; MP1];
        let mut sh = 0i16;
        calc_sum_acf_impl(&self.acf, &self.sh_acf, &mut tmp, &mut sh, NB_CURACF);
        self.sum_acf[..MP1].copy_from_slice(&tmp);
        self.sh_sum_acf[0] = sh;
    }

    pub(super) fn calc_pastfilt_impl(&mut self, state: &mut EncoderState) {
        let mut s_sum = [0i16; MP1];
        let mut sh = 0i16;
        calc_sum_acf_impl(
            &self.sum_acf,
            &self.sh_sum_acf,
            &mut s_sum,
            &mut sh,
            NB_SUMACF,
        );

        if s_sum[0] == 0 {
            self.past_coeff[0] = 4096;
            for i in 1..=M {
                self.past_coeff[i] = 0;
            }
            return;
        }

        let r_l = [0i16; MP1];
        let mut bid = [0i16; M];
        let mut err = 0i16;
        levinson_10(
            state,
            &s_sum,
            &r_l,
            &mut self.past_coeff,
            &mut bid,
            Some(&mut err),
        );
    }
}

pub(super) fn calc_sum_acf_impl(
    acf: &[i16],
    sh_acf: &[i16],
    sum: &mut [i16; MP1],
    sh_sum: &mut i16,
    nb: usize,
) {
    let mut ctx = DspContext::default();
    let mut l_tab = [Word32(0); MP1];

    let mut sh0 = sh_acf[0];
    for &s in sh_acf.iter().take(nb).skip(1) {
        if sub(&mut ctx, w(s), w(sh0)).0 < 0 {
            sh0 = s;
        }
    }
    sh0 = add(&mut ctx, w(sh0), w(14)).0;

    let mut ptr = 0usize;
    for &sh in sh_acf.iter().take(nb) {
        let temp = sub(&mut ctx, w(sh0), w(sh)).0;
        for lt in l_tab.iter_mut().take(MP1) {
            let mut l_temp = l_deposit_l(w(acf[ptr]));
            l_temp = l_shl(&mut ctx, l_temp, temp);
            *lt = l_add(&mut ctx, *lt, l_temp);
            ptr += 1;
        }
    }

    let temp = norm_l(l_tab[0]);
    for (i, s) in sum.iter_mut().enumerate().take(M + 1) {
        *s = extract_h(l_shl(&mut ctx, l_tab[i], temp)).0;
    }
    let t = sub(&mut ctx, w(temp), w(16)).0;
    *sh_sum = add(&mut ctx, w(sh0), w(t)).0;
}

pub(super) fn calc_rcoeff_impl(coeff: &[i16; MP1], rcoeff: &mut [i16; MP1], sh_rcoeff: &mut i16) {
    let mut ctx = DspContext::default();

    let mut l_acc = Word32(0);
    for &c in coeff.iter().take(M + 1) {
        l_acc = l_mac(&mut ctx, l_acc, w(c), w(c));
    }

    let sh1 = norm_l(l_acc);
    l_acc = l_shl(&mut ctx, l_acc, sh1);
    rcoeff[0] = round(&mut ctx, l_acc).0;

    for i in 1..=M {
        l_acc = Word32(0);
        for j in 0..=(M - i) {
            l_acc = l_mac(&mut ctx, l_acc, w(coeff[j]), w(coeff[j + i]));
        }
        l_acc = l_shl(&mut ctx, l_acc, sh1);
        rcoeff[i] = round(&mut ctx, l_acc).0;
    }

    *sh_rcoeff = sh1;
}

fn negate_i16(v: i16) -> i16 {
    if v == i16::MIN { i16::MAX } else { -v }
}
