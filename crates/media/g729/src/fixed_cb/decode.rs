//! Provenance: Fixed codebook search/decode adapted from ITU G.729 ACELP algebraic codebook routines.
//! Q-format: Correlation buffers and pulse gains use Q13/Q15 with 32-bit fixed-point accumulators.

use crate::constants::L_SUBFR;
use crate::dsp::arith::add;
use crate::dsp::shift::{shl, shr};
use crate::dsp::types::{DspContext, Word16};

/// Public function `decod_acelp`.
pub fn decod_acelp(sign: Word16, index: Word16) -> [Word16; L_SUBFR] {
    let mut ctx = DspContext::default();
    let mut pos = [Word16(0); 4];

    let mut idx = index;

    let mut i = Word16(idx.0 & 7);
    let i5 = shl(&mut ctx, i, 2);
    pos[0] = add(&mut ctx, i, i5);

    idx = shr(&mut ctx, idx, 3);
    i = Word16(idx.0 & 7);
    let i5 = shl(&mut ctx, i, 2);
    i = add(&mut ctx, i, i5);
    pos[1] = add(&mut ctx, i, Word16(1));

    idx = shr(&mut ctx, idx, 3);
    i = Word16(idx.0 & 7);
    let i5 = shl(&mut ctx, i, 2);
    i = add(&mut ctx, i, i5);
    pos[2] = add(&mut ctx, i, Word16(2));

    idx = shr(&mut ctx, idx, 3);
    let j = Word16(idx.0 & 1);
    idx = shr(&mut ctx, idx, 1);
    i = Word16(idx.0 & 7);
    let i5 = shl(&mut ctx, i, 2);
    i = add(&mut ctx, i, i5);
    i = add(&mut ctx, i, Word16(3));
    pos[3] = add(&mut ctx, i, j);

    let mut cod = [Word16(0); L_SUBFR];
    let mut s = sign;

    for p in pos {
        let bit = Word16(s.0 & 1);
        s = shr(&mut ctx, s, 1);

        let index = p.0 as usize;
        if index < L_SUBFR {
            cod[index] = if bit.0 != 0 {
                Word16(8191)
            } else {
                Word16(-8192)
            };
        }
    }

    cod
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decod_acelp_has_4_pulses() {
        let cod = decod_acelp(Word16(0b1010), Word16(0x1234));
        let non_zero = cod.iter().filter(|c| c.0 != 0).count();
        assert_eq!(non_zero, 4);
    }
}
