//! Provenance: Annex B DTX filter-comparison from ITU `DTX.C` (`cmp_filt`).
//! Q-format: reflection/ACF terms are Q15 with dynamic shifts to avoid overflow.

use super::w;
use crate::constants::{M, MP1};
use crate::dsp::arith::{add, mult_r, sub};
use crate::dsp::arith32::{l_add, l_deposit_l, l_mac, l_mult, l_sub};
use crate::dsp::shift::{l_shl, l_shr, shr};
use crate::dsp::types::DspContext;

pub(super) fn cmp_filt_impl(
    rcoeff: &[i16; MP1],
    sh_rcoeff: i16,
    acf: &[i16; MP1],
    alpha: i16,
    frac_thresh: i16,
) -> i16 {
    let mut ctx = DspContext::default();

    let mut sh = [0i16; 2];
    let mut ind = 1usize;
    let mut l_temp0;

    loop {
        ctx.overflow = false;
        let temp1 = shr(&mut ctx, w(rcoeff[0]), sh[0]).0;
        let temp2 = shr(&mut ctx, w(acf[0]), sh[1]).0;
        let prod0 = l_mult(&mut ctx, w(temp1), w(temp2));
        l_temp0 = l_shr(&mut ctx, prod0, 1);

        for i in 1..=M {
            let temp1 = shr(&mut ctx, w(rcoeff[i]), sh[0]).0;
            let temp2 = shr(&mut ctx, w(acf[i]), sh[1]).0;
            l_temp0 = l_mac(&mut ctx, l_temp0, w(temp1), w(temp2));
        }

        if !ctx.overflow {
            break;
        }
        sh[ind] = add(&mut ctx, w(sh[ind]), w(1)).0;
        ind = 1 - ind;
    }

    let temp1 = mult_r(&mut ctx, w(alpha), w(frac_thresh)).0;
    let mut l_temp1 = l_add(&mut ctx, l_deposit_l(w(temp1)), l_deposit_l(w(alpha)));
    let temp1 = add(&mut ctx, w(sh_rcoeff), w(9)).0;
    let temp2 = add(&mut ctx, w(sh[0]), w(sh[1])).0;
    let temp1 = sub(&mut ctx, w(temp1), w(temp2)).0;
    l_temp1 = l_shl(&mut ctx, l_temp1, temp1);

    let diff = l_sub(&mut ctx, l_temp0, l_temp1);
    if diff.0 > 0 { 1 } else { 0 }
}
