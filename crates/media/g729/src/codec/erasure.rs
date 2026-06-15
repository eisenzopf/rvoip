//! Decoder erasure concealment helpers.
//! Provenance: Frame encode/decode pipeline derived from ITU G.729 Annex A/B reference flow.
//! Q-format: Speech, excitation, and LPC paths follow Q0/Q12/Q13/Q15 fixed-point stages.

use crate::codec::state::DecoderState;
use crate::constants::{L_SUBFR, PIT_MAX, PIT_MIN};
use crate::dsp::types::Word16;
use crate::pitch::dec_lag3;

#[inline(always)]
fn w(v: i16) -> Word16 {
    Word16(v)
}

/// Decode pitch lag with bad-frame and parity-erasure handling.
pub(crate) fn decode_lag_or_erasure(
    state: &mut DecoderState,
    bfi: i16,
    sf: usize,
    index: i16,
    parity_err: Option<i16>,
    t0_var: &mut i16,
) -> (i16, i16) {
    if sf == 0 {
        let bad_pitch = if bfi != 0 || parity_err.unwrap_or(0) != 0 {
            1
        } else {
            0
        };
        if bad_pitch == 0 {
            let (t0, frac, old_t0) = dec_lag3(w(index), w(PIT_MIN), w(PIT_MAX), w(0), w(*t0_var));
            *t0_var = t0.0;
            state.old_t0 = old_t0.0;
            (t0.0, frac.0)
        } else {
            let t0 = *t0_var;
            state.old_t0 = (state.old_t0 + 1).min(PIT_MAX);
            (t0, 0)
        }
    } else if bfi == 0 {
        let (t0, frac, old_t0) = dec_lag3(
            w(index),
            w(PIT_MIN),
            w(PIT_MAX),
            w(L_SUBFR as i16),
            w(*t0_var),
        );
        *t0_var = t0.0;
        state.old_t0 = old_t0.0;
        (t0.0, frac.0)
    } else {
        let t0 = state.old_t0;
        *t0_var = t0;
        state.old_t0 = (state.old_t0 + 1).min(PIT_MAX);
        (t0, 0)
    }
}
