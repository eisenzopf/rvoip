//! Provenance: LPC/LSP analysis derived from ITU G.729 LPC conversion and recursion routines.
//! Q-format: Correlations and predictor coefficients use Q12/Q13/Q15 fixed-point domains.

use crate::constants::NC;
use crate::dsp::arith::extract_h;
use crate::dsp::arith32::{l_mac, l_msu, l_mult};
use crate::dsp::oper32::l_extract;
use crate::dsp::oper32::mpy_32_16;
use crate::dsp::shift::l_shl;
use crate::dsp::types::{DspContext, Word16};

#[inline(always)]
fn w(v: i16) -> Word16 {
    Word16(v)
}

pub(crate) fn chebps_11(x: i16, f: &[i16; NC + 1]) -> i16 {
    let mut ctx = DspContext::default();

    let mut b2_h = 256i16;
    let mut b2_l = 0i16;

    let mut t0 = l_mult(&mut ctx, w(x), w(512));
    t0 = l_mac(&mut ctx, t0, w(f[1]), w(4096));
    let (mut b1_h, mut b1_l) = l_extract(t0);

    let mut i = 2usize;
    while i < NC {
        t0 = mpy_32_16(b1_h, b1_l, w(x));
        t0 = l_shl(&mut ctx, t0, 1);
        t0 = l_mac(&mut ctx, t0, w(b2_h), w(-32768));
        t0 = l_msu(&mut ctx, t0, w(b2_l), w(1));
        t0 = l_mac(&mut ctx, t0, w(f[i]), w(4096));
        let (b0_h, b0_l) = l_extract(t0);
        b2_h = b1_h.0;
        b2_l = b1_l.0;
        b1_h = b0_h;
        b1_l = b0_l;
        i += 1;
    }

    t0 = mpy_32_16(b1_h, b1_l, w(x));
    t0 = l_mac(&mut ctx, t0, w(b2_h), w(-32768));
    t0 = l_msu(&mut ctx, t0, w(b2_l), w(1));
    t0 = l_mac(&mut ctx, t0, w(f[NC]), w(2048));
    t0 = l_shl(&mut ctx, t0, 6);
    extract_h(t0).0
}

pub(crate) fn chebps_10(x: i16, f: &[i16; NC + 1]) -> i16 {
    let mut ctx = DspContext::default();

    let mut b2_h = 128i16;
    let mut b2_l = 0i16;

    let mut t0 = l_mult(&mut ctx, w(x), w(256));
    t0 = l_mac(&mut ctx, t0, w(f[1]), w(4096));
    let (mut b1_h, mut b1_l) = l_extract(t0);

    let mut i = 2usize;
    while i < NC {
        t0 = mpy_32_16(b1_h, b1_l, w(x));
        t0 = l_shl(&mut ctx, t0, 1);
        t0 = l_mac(&mut ctx, t0, w(b2_h), w(-32768));
        t0 = l_msu(&mut ctx, t0, w(b2_l), w(1));
        t0 = l_mac(&mut ctx, t0, w(f[i]), w(4096));
        let (b0_h, b0_l) = l_extract(t0);
        b2_h = b1_h.0;
        b2_l = b1_l.0;
        b1_h = b0_h;
        b1_l = b0_l;
        i += 1;
    }

    t0 = mpy_32_16(b1_h, b1_l, w(x));
    t0 = l_mac(&mut ctx, t0, w(b2_h), w(-32768));
    t0 = l_msu(&mut ctx, t0, w(b2_l), w(1));
    t0 = l_mac(&mut ctx, t0, w(f[NC]), w(2048));
    t0 = l_shl(&mut ctx, t0, 7);
    extract_h(t0).0
}
