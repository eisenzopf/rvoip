//! Provenance: extracted from ITU `DEC_LD8A.C` decode frame path helpers.
//! Q-format: excitation energy paths operate in Q0/Q15 with fixed-point accumulators.

use crate::codec::state::{DecoderState, EXC_OFFSET};
use crate::constants::{L_FRAME, L_INTERPOL, PIT_MAX};
use crate::dsp::arith::{extract_l, round, sub};
use crate::dsp::arith32::{l_add, l_mac, l_mult};
use crate::dsp::shift::{l_shl, l_shr, norm_l};
use crate::dsp::types::{DspContext, Word16, Word32};

#[inline(always)]
fn w(v: i16) -> Word16 {
    Word16(v)
}

/// Compute SID reference energy for good speech frames.
pub(crate) fn save_sid_energy_if_good_frame(
    state: &DecoderState,
    bfi: i16,
    sid_energy_out: Option<(&mut i16, &mut i16)>,
) {
    if bfi != 0 {
        return;
    }

    let mut ctx = DspContext::default();
    let mut l_temp = Word32(0);
    for i in 0..L_FRAME {
        let e = state.old_exc[EXC_OFFSET + i];
        l_temp = l_mac(&mut ctx, l_temp, w(e), w(e));
    }
    let mut sh_sid_sav = norm_l(l_temp);
    let l_temp_n = l_shl(&mut ctx, l_temp, sh_sid_sav);
    let sid_sav = round(&mut ctx, l_temp_n).0;
    sh_sid_sav = sub(&mut ctx, w(16), w(sh_sid_sav)).0;
    if let Some((sid_sav_out, sh_sid_sav_out)) = sid_energy_out {
        *sid_sav_out = sid_sav;
        *sh_sid_sav_out = sh_sid_sav;
    }
}

/// Shift excitation history to prepare the next frame.
pub(crate) fn slide_excitation_history(state: &mut DecoderState) {
    for i in 0..(PIT_MAX as usize + L_INTERPOL) {
        state.old_exc[i] = state.old_exc[i + L_FRAME];
    }
}

pub(super) fn random_itu(seed: &mut i16) -> i16 {
    let mut ctx = DspContext::default();
    let l = l_mult(&mut ctx, w(*seed), w(31821));
    let l = l_shr(&mut ctx, l, 1);
    let l = l_add(&mut ctx, l, Word32(13849));
    *seed = extract_l(l).0;
    *seed
}
