//! Provenance: Annex B random codebook parameter generation from ITU `CALCEXC.C`.
//! Q-format: lag/fraction and gain indices are integer-domain control parameters.

use super::excitation_helpers::random_itu;
use super::w;
use crate::dsp::arith::{add, sub};
use crate::dsp::shift::{shl, shr};
use crate::dsp::types::DspContext;

pub(super) struct ExcitationParams {
    pub(super) pos: [i16; 4],
    pub(super) sign: [i16; 4],
    pub(super) t0: i16,
    pub(super) frac: i16,
    pub(super) gp: i16,
    pub(super) gp2: i16,
}

pub(super) fn sample_excitation_params(seed: &mut i16) -> ExcitationParams {
    let mut ctx = DspContext::default();
    let mut pos = [0i16; 4];
    let mut sign = [0i16; 4];

    let mut temp1 = random_itu(seed);
    let mut frac = sub(&mut ctx, w(temp1 & 0x0003), w(1)).0;
    if sub(&mut ctx, w(frac), w(2)).0 == 0 {
        frac = 0;
    }
    temp1 = shr(&mut ctx, w(temp1), 2).0;
    let t0 = add(&mut ctx, w(temp1 & 0x003f), w(40)).0;
    temp1 = shr(&mut ctx, w(temp1), 6).0;

    let mut temp2 = temp1 & 0x0007;
    let t5 = shl(&mut ctx, w(temp2), 2);
    pos[0] = add(&mut ctx, t5, w(temp2)).0;
    temp1 = shr(&mut ctx, w(temp1), 3).0;
    sign[0] = temp1 & 0x0001;

    temp1 = shr(&mut ctx, w(temp1), 1).0;
    temp2 = temp1 & 0x0007;
    let t5 = shl(&mut ctx, w(temp2), 2);
    temp2 = add(&mut ctx, t5, w(temp2)).0;
    pos[1] = add(&mut ctx, w(temp2), w(1)).0;
    temp1 = shr(&mut ctx, w(temp1), 3).0;
    sign[1] = temp1 & 0x0001;

    temp1 = random_itu(seed);
    temp2 = temp1 & 0x0007;
    let t5 = shl(&mut ctx, w(temp2), 2);
    temp2 = add(&mut ctx, t5, w(temp2)).0;
    pos[2] = add(&mut ctx, w(temp2), w(2)).0;
    temp1 = shr(&mut ctx, w(temp1), 3).0;
    sign[2] = temp1 & 0x0001;

    temp1 = shr(&mut ctx, w(temp1), 1).0;
    temp2 = temp1 & 0x000f;
    pos[3] = add(&mut ctx, w(temp2 & 1), w(3)).0;
    temp2 = shr(&mut ctx, w(temp2), 1).0 & 0x0007;
    let t5 = shl(&mut ctx, w(temp2), 2);
    temp2 = add(&mut ctx, t5, w(temp2)).0;
    pos[3] = add(&mut ctx, w(pos[3]), w(temp2)).0;
    temp1 = shr(&mut ctx, w(temp1), 4).0;
    sign[3] = temp1 & 0x0001;

    let gp = random_itu(seed) & 0x1fff;
    let gp2 = shl(&mut ctx, w(gp), 1).0;

    ExcitationParams {
        pos,
        sign,
        t0,
        frac,
        gp,
        gp2,
    }
}
