//! Provenance: LPC/LSP analysis derived from ITU G.729 LPC conversion and recursion routines.
//! Q-format: Correlations and predictor coefficients use Q12/Q13/Q15 fixed-point domains.

use super::chebyshev::{chebps_10, chebps_11};
use crate::constants::{M, MP1, NC};
use crate::dsp::arith::{abs_s, add, extract_h, negate, sub};
use crate::dsp::arith32::{l_mac, l_msu, l_mult};
use crate::dsp::div::div_s;
use crate::dsp::shift::{l_shr, norm_s, shl, shr};
use crate::dsp::types::{DspContext, Word16};
use crate::tables::annexa::GRID;

#[inline(always)]
fn w(v: i16) -> Word16 {
    Word16(v)
}

pub(crate) fn az_lsp(a: &[i16; MP1], lsp: &mut [i16; M], old_lsp: &[i16; M]) {
    let mut ctx = DspContext::default();

    let mut f1 = [0i16; NC + 1];
    let mut f2 = [0i16; NC + 1];

    let mut ovf_coef = false;
    f1[0] = 2048;
    f2[0] = 2048;

    for i in 0..NC {
        ctx.overflow = false;
        let mut t0 = l_mult(&mut ctx, w(a[i + 1]), w(16384));
        t0 = l_mac(&mut ctx, t0, w(a[M - i]), w(16384));
        let x = extract_h(t0).0;
        if ctx.overflow {
            ovf_coef = true;
        }

        ctx.overflow = false;
        f1[i + 1] = sub(&mut ctx, w(x), w(f1[i])).0;
        if ctx.overflow {
            ovf_coef = true;
        }

        ctx.overflow = false;
        let mut t0 = l_mult(&mut ctx, w(a[i + 1]), w(16384));
        t0 = l_msu(&mut ctx, t0, w(a[M - i]), w(16384));
        let x2 = extract_h(t0).0;
        if ctx.overflow {
            ovf_coef = true;
        }

        ctx.overflow = false;
        f2[i + 1] = add(&mut ctx, w(x2), w(f2[i])).0;
        if ctx.overflow {
            ovf_coef = true;
        }
    }

    if ovf_coef {
        f1[0] = 1024;
        f2[0] = 1024;

        for i in 0..NC {
            let mut t0 = l_mult(&mut ctx, w(a[i + 1]), w(8192));
            t0 = l_mac(&mut ctx, t0, w(a[M - i]), w(8192));
            let x = extract_h(t0).0;
            f1[i + 1] = sub(&mut ctx, w(x), w(f1[i])).0;

            let mut t0 = l_mult(&mut ctx, w(a[i + 1]), w(8192));
            t0 = l_msu(&mut ctx, t0, w(a[M - i]), w(8192));
            let x2 = extract_h(t0).0;
            f2[i + 1] = add(&mut ctx, w(x2), w(f2[i])).0;
        }
    }

    let mut nf = 0usize;
    let mut ip = 0usize;
    let mut coef_is_f1 = true;

    let mut xlow = GRID[0];
    let mut ylow = if ovf_coef {
        chebps_10(xlow, &f1)
    } else {
        chebps_11(xlow, &f1)
    };

    let mut j = 0usize;
    while nf < M && j < 50 {
        j += 1;
        let mut xhigh = xlow;
        let mut yhigh = ylow;
        xlow = GRID[j];

        ylow = if coef_is_f1 {
            if ovf_coef {
                chebps_10(xlow, &f1)
            } else {
                chebps_11(xlow, &f1)
            }
        } else if ovf_coef {
            chebps_10(xlow, &f2)
        } else {
            chebps_11(xlow, &f2)
        };

        let l_temp = l_mult(&mut ctx, w(ylow), w(yhigh));
        if l_temp.0 <= 0 {
            for _ in 0..2 {
                let xl = shr(&mut ctx, w(xlow), 1).0;
                let xh = shr(&mut ctx, w(xhigh), 1).0;
                let xmid = add(&mut ctx, w(xl), w(xh)).0;

                let ymid = if coef_is_f1 {
                    if ovf_coef {
                        chebps_10(xmid, &f1)
                    } else {
                        chebps_11(xmid, &f1)
                    }
                } else if ovf_coef {
                    chebps_10(xmid, &f2)
                } else {
                    chebps_11(xmid, &f2)
                };

                let l_temp2 = l_mult(&mut ctx, w(ylow), w(ymid));
                if l_temp2.0 <= 0 {
                    yhigh = ymid;
                    xhigh = xmid;
                } else {
                    ylow = ymid;
                    xlow = xmid;
                }
            }

            let x = sub(&mut ctx, w(xhigh), w(xlow)).0;
            let mut y = sub(&mut ctx, w(yhigh), w(ylow)).0;
            let xint = if y == 0 {
                xlow
            } else {
                let sign = y;
                y = abs_s(&mut ctx, w(y)).0;
                let exp = norm_s(w(y));
                y = shl(&mut ctx, w(y), exp).0;
                y = div_s(w(16383), w(y)).0;
                let mut t0 = l_mult(&mut ctx, w(x), w(y));
                let sh = sub(&mut ctx, w(20), w(exp)).0;
                t0 = l_shr(&mut ctx, t0, sh);
                let mut yy = crate::dsp::arith::extract_l(t0).0;
                if sign < 0 {
                    yy = negate(&mut ctx, w(yy)).0;
                }
                let mut t0 = l_mult(&mut ctx, w(ylow), w(yy));
                t0 = l_shr(&mut ctx, t0, 11);
                sub(&mut ctx, w(xlow), crate::dsp::arith::extract_l(t0)).0
            };

            lsp[nf] = xint;
            xlow = xint;
            nf += 1;

            ip ^= 1;
            coef_is_f1 = ip == 0;
            ylow = if coef_is_f1 {
                if ovf_coef {
                    chebps_10(xlow, &f1)
                } else {
                    chebps_11(xlow, &f1)
                }
            } else if ovf_coef {
                chebps_10(xlow, &f2)
            } else {
                chebps_11(xlow, &f2)
            };
        }
    }

    if nf < M {
        lsp.copy_from_slice(old_lsp);
    }
}
