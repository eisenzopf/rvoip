#![allow(clippy::field_reassign_with_default)]
#![allow(clippy::manual_clamp)]
#![allow(clippy::manual_memcpy)]
#![allow(clippy::needless_range_loop)]
//! Provenance: Frame encode/decode pipeline derived from ITU G.729 Annex A/B reference flow.
//! Q-format: Speech, excitation, and LPC paths follow Q0/Q12/Q13/Q15 fixed-point stages.

#[cfg(feature = "annex_b")]
use crate::annex_b::cng::CngState;
use crate::api::FrameType;
use crate::bitstream::itu_params::BITSNO;
use crate::codec::decode_sub::decod_ld8a;
use crate::codec::state::DecoderState;
use crate::constants::{BIT_0, BIT_1, L_FRAME, M, MP1, PRM_SIZE};
use crate::dsp::types::Word16;
use crate::pitch::check_parity_pitch;

#[inline(always)]
fn w(v: i16) -> Word16 {
    Word16(v)
}

fn bin2int(no_of_bits: i16, bits: &[u16], bit_offset: &mut usize) -> i16 {
    let mut value = 0i16;
    for _ in 0..no_of_bits {
        value <<= 1;
        let bit = bits.get(*bit_offset).copied().unwrap_or(BIT_0 as u16);
        if bit as i16 == BIT_1 {
            value = value.wrapping_add(1);
        }
        *bit_offset += 1;
    }
    value
}

fn bits2prm_ld8k(bits: &[u16; 80]) -> [i16; PRM_SIZE] {
    let mut prm = [0i16; PRM_SIZE];
    let mut off = 0usize;
    for i in 0..PRM_SIZE {
        prm[i] = bin2int(BITSNO[i] as i16, bits, &mut off);
    }
    prm
}

/// Public function `decode_speech_frame_words`.
pub fn decode_speech_frame_words(state: &mut DecoderState, words: &[u16]) -> [i16; L_FRAME] {
    let mut bits = [BIT_0 as u16; 80];
    let copy_len = bits.len().min(words.len());
    bits[..copy_len].copy_from_slice(&words[..copy_len]);

    let mut parm = [0i16; PRM_SIZE + 1];
    parm[0] = if bits.iter().any(|&w| w == 0) { 1 } else { 0 };
    let prm = bits2prm_ld8k(&bits);
    parm[1..].copy_from_slice(&prm);
    parm[4] = check_parity_pitch(w(parm[3]), w(parm[4])).0;

    let mut synth = [0i16; L_FRAME];
    let mut az_dec = [0i16; MP1 * 2];
    let mut t2 = [0i16; 2];
    decod_ld8a(state, &parm, &mut synth, &mut az_dec, &mut t2, None);

    state.synth_buf[M..M + L_FRAME].copy_from_slice(&synth);
    if state.post_filter_enabled {
        crate::postfilter::pipeline::post_filter(state, &az_dec, &t2, &mut synth, 1);
    }
    crate::postproc::post_process(state, &mut synth);
    state.synth_buf[M..M + L_FRAME].copy_from_slice(&synth);

    state.frame_index = state.frame_index.wrapping_add(1);
    let mut out = [0i16; L_FRAME];
    out.copy_from_slice(&state.synth_buf[M..M + L_FRAME]);
    out
}

/// Public function `decode_speech_frame`.
pub fn decode_speech_frame(state: &mut DecoderState, bits: &[u8; 10]) -> [i16; L_FRAME] {
    let mut words = [BIT_0 as u16; 80];
    for i in 0..80 {
        let bit = (bits[i / 8] >> (7 - (i % 8))) & 1;
        words[i] = if bit == 1 { BIT_1 as u16 } else { BIT_0 as u16 };
    }
    decode_speech_frame_words(state, &words)
}

/// Public function `decode_annex_b_frame_words`.
#[cfg(feature = "annex_b")]
pub fn decode_annex_b_frame_words(
    state: &mut DecoderState,
    cng: &mut CngState,
    frame_type: FrameType,
    words: &[u16],
    bfi_in: i16,
) -> [i16; L_FRAME] {
    crate::codec::decode_annexb::decode_annex_b_frame_words_impl(
        state, cng, frame_type, words, bfi_in,
    )
}

/// Public function `decode_annex_b_frame`.
#[cfg(feature = "annex_b")]
pub fn decode_annex_b_frame(
    state: &mut DecoderState,
    cng: &mut CngState,
    frame_type: FrameType,
    data: &[u8],
    bfi: i16,
) -> [i16; L_FRAME] {
    match frame_type {
        FrameType::Speech => {
            let mut words = [BIT_0 as u16; 80];
            for i in 0..80 {
                let bit = (data.get(i / 8).copied().unwrap_or(0) >> (7 - (i % 8))) & 1;
                words[i] = if bit == 1 { BIT_1 as u16 } else { BIT_0 as u16 };
            }
            decode_annex_b_frame_words(state, cng, frame_type, &words, bfi)
        }
        FrameType::Sid => {
            let mut words = [BIT_0 as u16; 16];
            for i in 0..16 {
                let bit = (data.get(i / 8).copied().unwrap_or(0) >> (7 - (i % 8))) & 1;
                words[i] = if bit == 1 { BIT_1 as u16 } else { BIT_0 as u16 };
            }
            decode_annex_b_frame_words(state, cng, frame_type, &words, bfi)
        }
        FrameType::NoData => decode_annex_b_frame_words(state, cng, frame_type, &[], bfi),
    }
}

/// Public function `decode_sid_frame`.
#[cfg(feature = "annex_b")]
pub fn decode_sid_frame(state: &mut DecoderState, cng: &mut CngState, sid: &[u8; 2]) -> [i16; 80] {
    decode_annex_b_frame(state, cng, FrameType::Sid, sid, 0)
}

/// Public function `decode_sid_frame`.
#[cfg(not(feature = "annex_b"))]
pub fn decode_sid_frame(_state: &mut DecoderState, _sid: &[u8; 2]) -> [i16; 80] {
    [0; 80]
}

/// Public function `decode_frame_typed`.
pub fn decode_frame_typed(
    state: &mut DecoderState,
    frame_type: FrameType,
    data: &[u8],
) -> [i16; 80] {
    match frame_type {
        FrameType::Speech => {
            let mut bits = [0u8; 10];
            let n = core::cmp::min(10, data.len());
            bits[..n].copy_from_slice(&data[..n]);
            decode_speech_frame(state, &bits)
        }
        FrameType::Sid => {
            #[cfg(feature = "annex_b")]
            {
                let mut sid = [0u8; 2];
                let n = core::cmp::min(2, data.len());
                sid[..n].copy_from_slice(&data[..n]);
                let mut cng = CngState::default();
                decode_sid_frame(state, &mut cng, &sid)
            }
            #[cfg(not(feature = "annex_b"))]
            {
                [0; 80]
            }
        }
        FrameType::NoData => {
            let mut out = [0i16; 80];
            for (i, s) in out.iter_mut().enumerate() {
                *s = ((i32::from(state.synth_buf[M + i]) * 3) / 4) as i16;
            }
            state.synth_buf[M..M + L_FRAME].copy_from_slice(&out);
            out
        }
    }
}
