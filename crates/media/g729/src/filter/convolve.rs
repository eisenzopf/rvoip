use crate::dsp::arith::extract_h;
use crate::dsp::arith32::l_mac;
use crate::dsp::shift::l_shl;
use crate::dsp::types::{DspContext, Word16, Word32};

/// Public function `convolve_with_ctx`.
pub fn convolve_with_ctx(
    ctx: &mut DspContext,
    x: &[Word16],
    h: &[Word16],
    y: &mut [Word16],
    l: usize,
) {
    for n in 0..l {
        let mut s = Word32(0);
        for i in 0..=n {
            s = l_mac(ctx, s, x[i], h[n - i]);
        }
        s = l_shl(ctx, s, 3);
        y[n] = extract_h(s);
    }
}

/// Public function `convolve`.
pub fn convolve(x: &[Word16], h: &[Word16], y: &mut [Word16], l: usize) {
    let mut ctx = DspContext::default();
    convolve_with_ctx(&mut ctx, x, h, y, l);
}

#[cfg(test)]
mod tests {
    use super::{Word16, convolve};

    #[test]
    fn convolve_impulse_identity() {
        let mut x = [Word16(0); 40];
        let mut h = [Word16(0); 40];
        x[0] = Word16(1000);
        h[0] = Word16(4096); // Q12 unity

        let mut y = [Word16(0); 40];
        convolve(&x, &h, &mut y, 40);
        assert_eq!(y[0].0, 1000);
    }
}
