#![allow(dead_code)]
//! Provenance: Annex B VAD/DTX/CNG behavior adapted from ITU G.729 Annex B reference routines.
//! Q-format: VAD features, SID parameters, and CNG gains use Q0/Q13/Q15 fixed-point domains.

//! Annex B VAD feature extraction and decision integration.

use super::{VadState, features_update::refine_marker};
use crate::constants::{M, NP};
use crate::dsp::arith::{add, extract_h, mult, sub};
use crate::dsp::arith32::{l_deposit_h, l_mac};
use crate::dsp::div::Log2;
use crate::dsp::oper32::{l_comp, mpy_32_16};
use crate::dsp::shift::{l_shl, shr};
use crate::dsp::types::{DspContext, Word16, Word32};
use crate::tables::sid::LBF_CORR;
use crate::tables::vad::{INIT_FRAME, NOISE, VOICE, ZC_END, ZC_START};

impl VadState {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn detect_from_analysis_impl(
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
        let mut ctx = DspContext::default();

        let mut exp = Word16(0);
        let mut frac = Word16(0);

        let mut acc0 = l_comp(Word16(r_h[0]), Word16(r_l[0]));
        Log2(acc0, &mut exp, &mut frac);
        acc0 = mpy_32_16(exp, frac, Word16(9864));
        let mut i = sub(&mut ctx, Word16(exp_r0), Word16(1)).0;
        i = sub(&mut ctx, Word16(i), Word16(1)).0;
        acc0 = l_mac(&mut ctx, acc0, Word16(9864), Word16(i));
        acc0 = l_shl(&mut ctx, acc0, 11);
        let mut energy = extract_h(acc0).0;
        energy = sub(&mut ctx, Word16(energy), Word16(4875)).0;

        acc0 = Word32(0);
        for idx in 1..=NP {
            acc0 = l_mac(&mut ctx, acc0, Word16(r_h[idx]), Word16(LBF_CORR[idx]));
        }
        acc0 = l_shl(&mut ctx, acc0, 1);
        acc0 = l_mac(&mut ctx, acc0, Word16(r_h[0]), Word16(LBF_CORR[0]));
        Log2(acc0, &mut exp, &mut frac);
        acc0 = mpy_32_16(exp, frac, Word16(9864));
        i = sub(&mut ctx, Word16(exp_r0), Word16(1)).0;
        i = sub(&mut ctx, Word16(i), Word16(1)).0;
        acc0 = l_mac(&mut ctx, acc0, Word16(9864), Word16(i));
        acc0 = l_shl(&mut ctx, acc0, 11);
        let mut energy_low = extract_h(acc0).0;
        energy_low = sub(&mut ctx, Word16(energy_low), Word16(4875)).0;

        acc0 = Word32(0);
        for idx in 0..M {
            let j = sub(&mut ctx, Word16(lsf[idx]), Word16(self.mean_lsf[idx])).0;
            acc0 = l_mac(&mut ctx, acc0, Word16(j), Word16(j));
        }
        let sd = extract_h(acc0).0;

        let mut zc = 0i16;
        for idx in (ZC_START + 1)..=ZC_END {
            if mult(&mut ctx, Word16(sigpp[idx - 1]), Word16(sigpp[idx])).0 < 0 {
                zc = add(&mut ctx, Word16(zc), Word16(410)).0;
            }
        }

        if sub(&mut ctx, Word16(frm_count), Word16(129)).0 < 0 {
            if sub(&mut ctx, Word16(energy), Word16(self.min)).0 < 0 {
                self.min = energy;
                self.prev_min = energy;
            }

            if (frm_count & 0x0007) == 0 {
                let fi = shr(&mut ctx, Word16(frm_count), 3).0;
                i = sub(&mut ctx, Word16(fi), Word16(1)).0;
                self.min_buffer[i as usize] = self.min;
                self.min = crate::dsp::types::MAX_16;
            }
        }

        if (frm_count & 0x0007) == 0 {
            self.prev_min = self.min_buffer[0];
            for idx in 1..16 {
                if sub(
                    &mut ctx,
                    Word16(self.min_buffer[idx]),
                    Word16(self.prev_min),
                )
                .0 < 0
                {
                    self.prev_min = self.min_buffer[idx];
                }
            }
        }

        if sub(&mut ctx, Word16(frm_count), Word16(129)).0 >= 0 {
            if ((frm_count & 0x0007) ^ 0x0001) == 0 {
                self.min = self.prev_min;
                self.next_min = crate::dsp::types::MAX_16;
            }
            if sub(&mut ctx, Word16(energy), Word16(self.min)).0 < 0 {
                self.min = energy;
            }
            if sub(&mut ctx, Word16(energy), Word16(self.next_min)).0 < 0 {
                self.next_min = energy;
            }

            if (frm_count & 0x0007) == 0 {
                self.min_buffer.copy_within(1..16, 0);
                self.min_buffer[15] = self.next_min;
                self.prev_min = self.min_buffer[0];
                for idx in 1..16 {
                    if sub(
                        &mut ctx,
                        Word16(self.min_buffer[idx]),
                        Word16(self.prev_min),
                    )
                    .0 < 0
                    {
                        self.prev_min = self.min_buffer[idx];
                    }
                }
            }
        }

        let mut marker: i16;
        if sub(&mut ctx, Word16(frm_count), Word16(INIT_FRAME)).0 <= 0 {
            if sub(&mut ctx, Word16(energy), Word16(3072)).0 < 0 {
                marker = NOISE;
                self.less_count = add(&mut ctx, Word16(self.less_count), Word16(1)).0;
            } else {
                marker = VOICE;
                acc0 = l_deposit_h(Word16(self.mean_e));
                acc0 = l_mac(&mut ctx, acc0, Word16(energy), Word16(1024));
                self.mean_e = extract_h(acc0).0;
                acc0 = l_deposit_h(Word16(self.mean_szc));
                acc0 = l_mac(&mut ctx, acc0, Word16(zc), Word16(1024));
                self.mean_szc = extract_h(acc0).0;
                for idx in 0..M {
                    acc0 = l_deposit_h(Word16(self.mean_lsf[idx]));
                    acc0 = l_mac(&mut ctx, acc0, Word16(lsf[idx]), Word16(1024));
                    self.mean_lsf[idx] = extract_h(acc0).0;
                }
            }
        } else {
            marker = NOISE;
        }

        marker = refine_marker(
            self,
            &mut ctx,
            rc,
            lsf,
            frm_count,
            prev_marker,
            pprev_marker,
            energy,
            energy_low,
            sd,
            zc,
            marker,
        );

        self.prev_energy = energy;
        marker
    }
}
