use crate::dsp::arith::{mult, sub};
use crate::dsp::types::{DspContext, Word16};

#[inline(always)]
fn w(v: i16) -> Word16 {
    Word16(v)
}

/// Public function `preemphasis_with_mem`.
pub fn preemphasis_with_mem(signal: &mut [i16], coeff: i16, mem: &mut i16) {
    if signal.is_empty() {
        return;
    }

    let mut ctx = DspContext::default();
    let temp = signal[signal.len() - 1];
    for idx in (1..signal.len()).rev() {
        let m = mult(&mut ctx, w(coeff), w(signal[idx - 1]));
        signal[idx] = sub(&mut ctx, w(signal[idx]), m).0;
    }
    let m0 = mult(&mut ctx, w(coeff), w(*mem));
    signal[0] = sub(&mut ctx, w(signal[0]), m0).0;
    *mem = temp;
}
