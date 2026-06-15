//! Provenance: Pitch search and lag coding adapted from ITU G.729 open/closed-loop pitch routines.
//! Q-format: Lag correlations and adaptive codebook gains use Q0/Q15 accumulator scaling.

use crate::dsp::arith::add;
use crate::dsp::shift::shr;
use crate::dsp::types::{DspContext, Word16};

/// Public function `parity_pitch`.
pub fn parity_pitch(pitch_index: Word16) -> Word16 {
    let mut ctx = DspContext::default();
    let mut temp = shr(&mut ctx, pitch_index, 1);

    let mut sum = Word16(1);
    for _ in 0..=5 {
        temp = shr(&mut ctx, temp, 1);
        let bit = Word16(temp.0 & 1);
        sum = add(&mut ctx, sum, bit);
    }

    Word16(sum.0 & 1)
}

/// Public function `check_parity_pitch`.
pub fn check_parity_pitch(pitch_index: Word16, parity: Word16) -> Word16 {
    let mut ctx = DspContext::default();
    let mut temp = shr(&mut ctx, pitch_index, 1);

    let mut sum = Word16(1);
    for _ in 0..=5 {
        temp = shr(&mut ctx, temp, 1);
        let bit = Word16(temp.0 & 1);
        sum = add(&mut ctx, sum, bit);
    }

    sum = add(&mut ctx, sum, parity);
    Word16(sum.0 & 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parity_roundtrip_consistency() {
        for idx in [Word16(0), Word16(1), Word16(255), Word16(112), Word16(197)] {
            let p = parity_pitch(idx);
            assert_eq!(check_parity_pitch(idx, p).0, 0);
            assert_eq!(check_parity_pitch(idx, Word16(p.0 ^ 1)).0, 1);
        }
    }
}
