use crate::constants::{M, MP1};
use crate::dsp::arith::round;
use crate::dsp::arith32::{l_msu, l_mult};
use crate::dsp::shift::l_shl;
use crate::dsp::types::{DspContext, Word16};

/// Public function `syn_filt_with_ctx`.
pub fn syn_filt_with_ctx(
    ctx: &mut DspContext,
    a: &[Word16],
    x: &[Word16],
    y: &mut [Word16],
    lg: usize,
    mem: &mut [Word16],
    update: bool,
) {
    let mut tmp = [Word16(0); 200];
    let mut yy = 0usize;

    for &m in mem.iter().take(M) {
        tmp[yy] = m;
        yy += 1;
    }

    for i in 0..lg {
        let mut s = l_mult(ctx, x[i], a[0]);
        for j in 1..=M {
            s = l_msu(ctx, s, a[j], tmp[yy - j]);
        }
        s = l_shl(ctx, s, 3);
        tmp[yy] = round(ctx, s);
        yy += 1;
    }

    y[..lg].copy_from_slice(&tmp[M..M + lg]);

    if update {
        mem[..M].copy_from_slice(&y[lg - M..lg]);
    }
}

/// Public function `syn_filt`.
pub fn syn_filt(
    a: &[Word16],
    x: &[Word16],
    y: &mut [Word16],
    lg: usize,
    mem: &mut [Word16],
    update: bool,
) {
    let mut ctx = DspContext::default();
    syn_filt_with_ctx(&mut ctx, a, x, y, lg, mem, update);
}

pub(crate) fn syn_filt_i16(
    a: &[i16; MP1],
    x: &[i16],
    y: &mut [i16],
    lg: usize,
    mem: &mut [i16; M],
    update: bool,
) {
    let mut ctx = DspContext::default();
    let mut tmp = [0i16; 220];
    let mut yy = 0usize;

    for &m in mem.iter() {
        tmp[yy] = m;
        yy += 1;
    }

    for i in 0..lg {
        let mut s = l_mult(&mut ctx, Word16(x[i]), Word16(a[0]));
        for j in 1..=M {
            s = l_msu(&mut ctx, s, Word16(a[j]), Word16(tmp[yy - j]));
        }
        s = l_shl(&mut ctx, s, 3);
        tmp[yy] = round(&mut ctx, s).0;
        yy += 1;
    }

    y[..lg].copy_from_slice(&tmp[M..M + lg]);

    if update {
        mem.copy_from_slice(&y[lg - M..lg]);
    }
}
