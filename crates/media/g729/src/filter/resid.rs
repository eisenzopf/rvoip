use crate::constants::{M, MP1};
use crate::dsp::arith::round;
use crate::dsp::arith32::{l_mac, l_mult};
use crate::dsp::shift::l_shl;
use crate::dsp::types::{DspContext, Word16};

/// Public function `residu_with_ctx`.
pub fn residu_with_ctx(
    ctx: &mut DspContext,
    a: &[Word16],
    x_with_hist: &[Word16],
    y: &mut [Word16],
    lg: usize,
) {
    for i in 0..lg {
        let cur = i + M;
        let mut s = l_mult(ctx, x_with_hist[cur], a[0]);
        for j in 1..=M {
            s = l_mac(ctx, s, a[j], x_with_hist[cur - j]);
        }
        s = l_shl(ctx, s, 3);
        y[i] = round(ctx, s);
    }
}

/// Public function `residu`.
pub fn residu(a: &[Word16], x_with_hist: &[Word16], y: &mut [Word16], lg: usize) {
    let mut ctx = DspContext::default();
    residu_with_ctx(&mut ctx, a, x_with_hist, y, lg);
}

pub(crate) fn residu_i16(a: &[i16; MP1], x: &[i16], x_start: usize, y: &mut [i16], lg: usize) {
    let mut ctx = DspContext::default();
    for i in 0..lg {
        let xi = x_start + i;
        let mut s = l_mult(&mut ctx, Word16(x[xi]), Word16(a[0]));
        for j in 1..=M {
            s = l_mac(&mut ctx, s, Word16(a[j]), Word16(x[xi - j]));
        }
        s = l_shl(&mut ctx, s, 3);
        y[i] = round(&mut ctx, s).0;
    }
}

#[cfg(test)]
mod tests {
    use super::{Word16, residu};
    use crate::constants::M;

    #[test]
    fn residu_zero_history_runs() {
        let a = [Word16(4096); M + 1];
        let x = [Word16(0); M + 40];
        let mut y = [Word16(0); 40];
        residu(&a, &x, &mut y, 40);
        assert_eq!(y[0].0, 0);
    }
}
