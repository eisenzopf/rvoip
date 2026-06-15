//! Decoder post-processing high-pass filter.

use crate::codecs::g729::impls::codec::state::DecoderState;
use crate::codecs::g729::impls::constants::L_FRAME;
use crate::codecs::g729::impls::dsp::arith::round;
use crate::codecs::g729::impls::dsp::arith32::{l_add, l_mac};
use crate::codecs::g729::impls::dsp::oper32::{l_extract, mpy_32_16};
use crate::codecs::g729::impls::dsp::shift::l_shl;
use crate::codecs::g729::impls::dsp::types::{DspContext, Word16};
use crate::codecs::g729::impls::tables::annexa::{A100, B100};

pub(crate) fn post_process(state: &mut DecoderState, signal: &mut [i16; L_FRAME]) {
    let mut ctx = DspContext::default();
    for s in signal.iter_mut() {
        let x2 = state.pp_x1;
        state.pp_x1 = state.pp_x0;
        state.pp_x0 = *s;

        let mut l_tmp = mpy_32_16(
            Word16(state.pp_y1_hi),
            Word16(state.pp_y1_lo),
            Word16(A100[1]),
        );
        l_tmp = l_add(
            &mut ctx,
            l_tmp,
            mpy_32_16(
                Word16(state.pp_y2_hi),
                Word16(state.pp_y2_lo),
                Word16(A100[2]),
            ),
        );
        l_tmp = l_mac(&mut ctx, l_tmp, Word16(state.pp_x0), Word16(B100[0]));
        l_tmp = l_mac(&mut ctx, l_tmp, Word16(state.pp_x1), Word16(B100[1]));
        l_tmp = l_mac(&mut ctx, l_tmp, Word16(x2), Word16(B100[2]));
        l_tmp = l_shl(&mut ctx, l_tmp, 2);

        let t = l_shl(&mut ctx, l_tmp, 1);
        *s = round(&mut ctx, t).0;

        state.pp_y2_hi = state.pp_y1_hi;
        state.pp_y2_lo = state.pp_y1_lo;
        let (hi, lo) = l_extract(l_tmp);
        state.pp_y1_hi = hi.0;
        state.pp_y1_lo = lo.0;
    }
}
