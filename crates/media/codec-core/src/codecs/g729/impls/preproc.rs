//! Encoder high-pass pre-processing filter.

use crate::codecs::g729::impls::codec::state::EncoderState;
use crate::codecs::g729::impls::constants::L_FRAME;
use crate::codecs::g729::impls::dsp::arith32::{l_add, l_mac};
use crate::codecs::g729::impls::dsp::oper32::{l_extract, mpy_32_16};
use crate::codecs::g729::impls::dsp::shift::l_shl;
use crate::codecs::g729::impls::dsp::types::{DspContext, Word16};
use crate::codecs::g729::impls::tables::annexa::{A140, B140};

pub(crate) fn pre_process_frame(state: &mut EncoderState, signal: &mut [i16; L_FRAME]) {
    let mut ctx = DspContext::default();

    for s in signal.iter_mut() {
        let x2 = state.pp_x1;
        state.pp_x1 = state.pp_x0;
        state.pp_x0 = *s;

        let mut l_tmp = mpy_32_16(
            Word16(state.pp_y1_hi),
            Word16(state.pp_y1_lo),
            Word16(A140[1]),
        );
        l_tmp = l_add(
            &mut ctx,
            l_tmp,
            mpy_32_16(
                Word16(state.pp_y2_hi),
                Word16(state.pp_y2_lo),
                Word16(A140[2]),
            ),
        );
        l_tmp = l_mac(&mut ctx, l_tmp, Word16(state.pp_x0), Word16(B140[0]));
        l_tmp = l_mac(&mut ctx, l_tmp, Word16(state.pp_x1), Word16(B140[1]));
        l_tmp = l_mac(&mut ctx, l_tmp, Word16(x2), Word16(B140[2]));
        l_tmp = l_shl(&mut ctx, l_tmp, 3);
        *s = crate::codecs::g729::impls::dsp::arith::round(&mut ctx, l_tmp).0;

        state.pp_y2_hi = state.pp_y1_hi;
        state.pp_y2_lo = state.pp_y1_lo;
        let (hi, lo) = l_extract(l_tmp);
        state.pp_y1_hi = hi.0;
        state.pp_y1_lo = lo.0;
    }
}
