//! Adaptive codebook interpolation helpers.
//! Provenance: Pitch search and lag coding adapted from ITU G.729 open/closed-loop pitch routines.
//! Q-format: Lag correlations and adaptive codebook gains use Q0/Q15 accumulator scaling.

use crate::codec::state::{ENC_OLD_EXC_LEN, OLD_EXC_LEN};
use crate::constants::{L_INTER10, L_SUBFR, UP_SAMP};
use crate::dsp::arith::{add, negate, round, sub};
use crate::dsp::arith32::l_mac;
use crate::dsp::types::{DspContext, Word16, Word32};
use crate::tables::annexa::INTER_3L;

fn pred_lt_3_impl(old_exc: &mut [i16], exc_idx: usize, t0: i16, frac_in: i16) {
    let mut ctx = DspContext::default();
    let mut x0 = exc_idx.saturating_sub(t0 as usize);
    let mut frac = negate(&mut ctx, Word16(frac_in)).0;
    if frac < 0 {
        frac = add(&mut ctx, Word16(frac), Word16(UP_SAMP as i16)).0;
        x0 = x0.saturating_sub(1);
    }

    for j in 0..L_SUBFR {
        let x1 = x0;
        x0 += 1;
        let x2 = x0;
        let c1 = frac as usize;
        let c2 = sub(&mut ctx, Word16(UP_SAMP as i16), Word16(frac)).0 as usize;

        let mut s = Word32(0);
        let mut k = 0usize;
        for i in 0..L_INTER10 {
            s = l_mac(
                &mut ctx,
                s,
                Word16(old_exc[x1 - i]),
                Word16(INTER_3L[c1 + k]),
            );
            s = l_mac(
                &mut ctx,
                s,
                Word16(old_exc[x2 + i]),
                Word16(INTER_3L[c2 + k]),
            );
            k += UP_SAMP;
        }
        old_exc[exc_idx + j] = round(&mut ctx, s).0;
    }
}

pub(crate) fn interpolate_excitation_encode(
    old_exc: &mut [i16; ENC_OLD_EXC_LEN],
    exc_idx: usize,
    t0: i16,
    frac_in: i16,
) {
    pred_lt_3_impl(old_exc, exc_idx, t0, frac_in);
}

pub(crate) fn interpolate_excitation_decode(
    old_exc: &mut [i16; OLD_EXC_LEN],
    exc_idx: usize,
    t0: i16,
    frac_in: i16,
) {
    pred_lt_3_impl(old_exc, exc_idx, t0, frac_in);
}
