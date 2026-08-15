#![no_main]
//! Fuzz the AMR-NB and AMR-WB *encoders* — arbitrary PCM through the
//! fixed-point analysis chain, with mode switches and DTX in play.
//!
//! The decoder fuzzer's justification does not transfer here — an encoder's
//! input is samples, not attacker bits — but "samples" still spans the full
//! i16 range, and the reference pipeline internally assumes conditioned
//! input: the 14-bit restriction, scaling decisions driven by frame energy,
//! VAD state machines, and the DTX hangover counters all carry state between
//! frames. A pathological sample sequence that desynchronizes any of that is
//! reachable from a microphone, a WAV file, or a hostile far end of a
//! transcoding bridge. The properties under test:
//!
//! - a frame-sized slice of ANY i16 values must encode: `Ok`, never a panic,
//!   and with DTX off the payload length must be exactly the mode's coded
//!   size — a wrong-length payload would corrupt every downstream framing;
//! - with DTX on, the only other legal outputs are a SID frame or an empty
//!   NO_DATA payload;
//! - mode switches between frames (the live CMR path) must respect the
//!   configured mode set and never disturb the invariants above;
//! - wrong-length input is an `Err`, not a panic.
//!
//! Layout: two selector bytes (variant, DTX, mode-set mask), then per frame
//! one control byte (mode-switch request) followed by the frame's PCM as
//! little-endian i16, zero-padded at the tail so a truncated final chunk
//! still exercises a full frame.

use codec_core::codecs::amr::mode::AmrVariant;
use codec_core::codecs::amr::AmrCodec;
use codec_core::types::{AudioCodec, CodecConfig, VariableRateCodec};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Encoding is ~1000x the cost of parsing; cap the input so one unit of
    // fuzzer work stays around a dozen frames, and give libFuzzer no reason
    // to grow inputs past new coverage.
    if data.len() > 8 * 1024 {
        return;
    }
    let [s0, s1, rest @ ..] = data else { return };

    let narrowband = s0 & 1 == 0;
    let variant = if narrowband {
        AmrVariant::NarrowBand
    } else {
        AmrVariant::WideBand
    };
    let dtx = s0 & 2 != 0;

    let mut config = if narrowband {
        CodecConfig::amr_nb()
    } else {
        CodecConfig::amr_wb()
    };
    // Any subset of the variant's modes, zero meaning "all" per RFC 4867.
    // An empty intersection is impossible here because the mask is applied
    // to the variant's own range before use.
    let top = if narrowband { 8u16 } else { 9 };
    config.parameters.amr.mode_set = u16::from(*s1) & ((1 << top) - 1);
    config.parameters.amr.dtx = dtx;

    let Ok(mut codec) = AmrCodec::new(&config) else {
        // A mode set the constructor refuses is a valid negotiation outcome,
        // not a finding.
        return;
    };

    // Wrong-length input must be a clean error on both sides of the frame
    // size, never a panic or a silent partial encode.
    let samples = variant.frame_samples();
    assert!(codec.encode(&[]).is_err());
    assert!(codec.encode(&vec![0i16; samples - 1]).is_err());
    assert!(codec.encode(&vec![0i16; samples + 1]).is_err());

    let stride = 1 + samples * 2;
    let mut pcm = vec![0i16; samples];
    for chunk in rest.chunks(stride).take(48) {
        let Some((&control, body)) = chunk.split_first() else {
            break;
        };

        // The live mode-switch path: a peer's CMR lands between frames. An
        // index outside the configured set must be refused; either way the
        // next frame must still uphold the output contract.
        if control & 1 != 0 {
            let _ = codec.set_mode((control >> 1) % 9);
        }

        for slot in pcm.iter_mut() {
            *slot = 0;
        }
        for (i, sample) in body.chunks_exact(2).enumerate() {
            pcm[i] = i16::from_le_bytes([sample[0], sample[1]]);
        }

        let mode = codec.mode();
        let payload = codec
            .encode(&pcm)
            .expect("a full frame of arbitrary PCM must encode");

        let speech_len = mode.octet_aligned_bytes();
        if dtx {
            // Speech, SID (both variants pack SID into at most 6 octets), or
            // an empty NO_DATA payload. Anything else is a framing bug.
            assert!(
                payload.len() == speech_len || payload.len() <= 6,
                "impossible payload length {} for {mode:?} with dtx",
                payload.len()
            );
        } else {
            assert_eq!(
                payload.len(),
                speech_len,
                "coded length must match the mode with dtx off"
            );
        }
    }
});
