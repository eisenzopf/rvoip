//! Provenance: LSP quantization/dequantization adapted from ITU G.729 LSP codebook routines.
//! Q-format: LSP/LSF vectors and prediction weights use Q13/Q15 fixed-point representation.

use crate::constants::{GAP3, L_LIMIT, M, M_LIMIT, NC};
use crate::dsp::arith::{add, sub};
use crate::dsp::arith32::{l_deposit_l, l_sub};
use crate::dsp::shift::shr;
use crate::dsp::types::{DspContext, Word16};

#[inline(always)]
fn w(v: i16) -> Word16 {
    Word16(v)
}

pub(crate) fn expand_1(buf: &mut [i16; M], gap: i16) {
    let mut ctx = DspContext::default();
    for j in 1..NC {
        let diff = sub(&mut ctx, w(buf[j - 1]), w(buf[j]));
        let sum = add(&mut ctx, diff, w(gap));
        let tmp = shr(&mut ctx, sum, 1);
        if tmp.0 > 0 {
            buf[j - 1] = sub(&mut ctx, w(buf[j - 1]), tmp).0;
            buf[j] = add(&mut ctx, w(buf[j]), tmp).0;
        }
    }
}

pub(crate) fn expand_2(buf: &mut [i16; M], gap: i16) {
    let mut ctx = DspContext::default();
    for j in NC..M {
        let diff = sub(&mut ctx, w(buf[j - 1]), w(buf[j]));
        let sum = add(&mut ctx, diff, w(gap));
        let tmp = shr(&mut ctx, sum, 1);
        if tmp.0 > 0 {
            buf[j - 1] = sub(&mut ctx, w(buf[j - 1]), tmp).0;
            buf[j] = add(&mut ctx, w(buf[j]), tmp).0;
        }
    }
}

pub(crate) fn expand_1_2(buf: &mut [i16; M], gap: i16) {
    let mut ctx = DspContext::default();
    for j in 1..M {
        let diff = sub(&mut ctx, w(buf[j - 1]), w(buf[j]));
        let sum = add(&mut ctx, diff, w(gap));
        let tmp = shr(&mut ctx, sum, 1);
        if tmp.0 > 0 {
            buf[j - 1] = sub(&mut ctx, w(buf[j - 1]), tmp).0;
            buf[j] = add(&mut ctx, w(buf[j]), tmp).0;
        }
    }
}

pub(crate) fn stabilize_encode(buf: &mut [i16; M]) {
    let mut ctx = DspContext::default();

    for j in 0..(M - 1) {
        let l_diff = l_sub(&mut ctx, l_deposit_l(w(buf[j + 1])), l_deposit_l(w(buf[j])));
        if l_diff.0 < 0 {
            buf.swap(j, j + 1);
        }
    }
    if sub(&mut ctx, w(buf[0]), w(L_LIMIT)).0 < 0 {
        buf[0] = L_LIMIT;
    }
    for j in 0..(M - 1) {
        let l_diff = l_sub(&mut ctx, l_deposit_l(w(buf[j + 1])), l_deposit_l(w(buf[j])));
        if l_sub(&mut ctx, l_diff, l_deposit_l(w(GAP3))).0 < 0 {
            buf[j + 1] = add(&mut ctx, w(buf[j]), w(GAP3)).0;
        }
    }
    if sub(&mut ctx, w(buf[M - 1]), w(M_LIMIT)).0 > 0 {
        buf[M - 1] = M_LIMIT;
    }
}
