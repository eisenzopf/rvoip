//! Provenance: Annex B DTX/CNG gain smoothing from ITU `DTX.C`.
//! Q-format: SID and current gains are Q0 indices converted via Q15 smoothing factors.

use super::w;
use crate::dsp::arith::{add, mult_r};
use crate::dsp::types::DspContext;
use crate::tables::sid::{A_GAIN0, A_GAIN1};

pub(super) fn update_cur_gain(
    ctx: &mut DspContext,
    cur_gain: i16,
    sid_gain: i16,
    past_vad: i16,
) -> i16 {
    if past_vad != 0 {
        sid_gain
    } else {
        let mut out = mult_r(ctx, w(cur_gain), w(A_GAIN0)).0;
        let sid_part = mult_r(ctx, w(sid_gain), w(A_GAIN1));
        out = add(ctx, w(out), sid_part).0;
        out
    }
}
