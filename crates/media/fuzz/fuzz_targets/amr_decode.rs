#![no_main]
//! Fuzz the AMR-NB and AMR-WB decoders — the DSP behind the depacketizer.
//!
//! `amr_unpack` covers the RFC 4867 parser; this covers what happens after it,
//! which until now nothing reached: a frame's coded bits go straight into a
//! fixed-point kernel full of table lookups indexed by fields the sender
//! chose.
//!
//! The property is that no input panics, hangs or allocates unboundedly. A
//! decode failure is a correct outcome — the decoders reject a payload that is
//! too short for its rate — but a decode *success* on nonsense is also correct,
//! because a corrupted frame is indistinguishable from a valid one at this
//! layer. That is exactly why the kernel has to survive arbitrary bits rather
//! than trust them.
//!
//! # Why this is worth fuzzing rather than reasoning about
//!
//! Several parameter fields have a legal range narrower than their bit width.
//! At 10.2 kbit/s the pulse positions are packed as 125x8 and 25x4
//! combinations into 10- and 7-bit fields, so a value above 999 or 99 makes the
//! reference's own position decode return an index past the end of a 40-sample
//! codevector — and the C writes there. Building the vector generator hit that
//! overrun on its own stack before the bound was applied. An encoder never
//! emits those values; a corrupted packet does.
//!
//! The first input byte selects the variant and the mode, since neither is
//! derivable from the coded bits and a fuzzer cannot discover them.

use libfuzzer_sys::fuzz_target;
use codec_core::codecs::amr::{AmrMode, AmrVariant};
use codec_core::codecs::amr::nb::decoder::{Decoder as NbDecoder, FrameState};
use codec_core::codecs::amr::wb::decoder::Decoder as WbDecoder;

fuzz_target!(|data: &[u8]| {
    // A single AMR frame is at most 60 bytes; a few frames' worth is plenty,
    // and larger inputs only slow the fuzzer without reaching new code.
    if data.len() > 1024 {
        return;
    }
    let Some((&selector, payload)) = data.split_first() else {
        return;
    };

    let narrowband = selector & 1 == 0;
    let variant = if narrowband {
        AmrVariant::NarrowBand
    } else {
        AmrVariant::WideBand
    };
    // Modes 0..=8 for wideband, 0..=7 for narrowband; anything else is not a
    // speech mode and the depacketizer would have rejected it already.
    let Ok(mode) = AmrMode::new(variant, (selector >> 1) % 9) else {
        return;
    };

    // Feed the payload as a run of frames rather than one, so the decoder's
    // carried state — filter memories, the adaptive codebook history, the
    // predicted gain — is exercised on adversarial input rather than only on
    // its reset values. State that only misbehaves on the third frame is
    // exactly what a single-frame fuzzer misses.
    let frame_len = mode.octet_aligned_bytes();
    if narrowband {
        let mut decoder = NbDecoder::new();
        for (i, chunk) in payload.chunks(frame_len).enumerate() {
            // Alternate the quality bit so concealment is fuzzed too: it runs
            // only when something has already gone wrong, which makes it the
            // path least likely to be exercised by accident.
            let state = if i % 3 == 2 {
                FrameState::Bad
            } else {
                FrameState::Good
            };
            match state {
                FrameState::Good => {
                    let _ = decoder.decode(mode.index(), chunk);
                }
                _ => {
                    if let Some(params) =
                        codec_core::codecs::amr::nb::bitstream::parse(mode.index(), chunk)
                    {
                        let _ = decoder.decode_parameters(mode.index(), &params, state);
                    }
                }
            }
        }
    } else {
        let mut decoder = WbDecoder::new();
        for chunk in payload.chunks(frame_len) {
            let _ = decoder.decode(mode, chunk);
        }
    }
});
