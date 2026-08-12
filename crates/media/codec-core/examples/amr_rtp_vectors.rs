//! Emit RTP packets carrying AMR payloads, for an independent dissector.
//!
//! # Why this exists
//!
//! The codec bits are bit-exact against 3GPP's own reference implementations,
//! and the *sorting* of those bits is checked against reference-produced
//! `.amr` files. But everything RFC 4867 adds on top of them for RTP — the CMR
//! nibble, the table-of-contents chain, octet-aligned padding, robust sorting,
//! the frame CRC — is verified here only by packing and then unpacking with
//! our own code.
//!
//! A round trip cannot catch a symmetric mistake. Put the CMR in the wrong
//! four bits and our depacker takes it out of the wrong four bits, the audio
//! is perfect, and no peer on earth can read the stream. That is precisely the
//! class of bug that two rvoip endpoints talking to each other cannot find.
//!
//! So this prints packets for Wireshark's AMR dissector, which is an
//! independent reading of the same RFC. `tools/verify-amr-rtp-framing.sh`
//! drives it and compares what tshark reports against the manifest below.
//!
//! Output is two interleaved streams on stdout:
//!   `# case <n> <variant> <framing> ft=<n> cmr=<n|none> ...`  — the manifest
//!   `<offset> <hex bytes...>`                                 — text2pcap input

use codec_core::codecs::amr::mode::{AmrFrameType, AmrMode, AmrVariant};
use codec_core::codecs::amr::payload::{
    AmrPacket, AmrPayloadCodec, AmrPayloadConfig, AmrPayloadFrame,
};
use codec_core::types::{CodecConfig, CodedFrame, FrameKind, VariableRateCodec};

/// One 20 ms frame of a deterministic tone at `rate`.
fn tone(rate: u32, index: usize) -> Vec<i16> {
    let samples = rate as usize / 50;
    (0..samples)
        .map(|i| {
            let t = ((index * samples + i) as f64) / f64::from(rate);
            (t * 440.0 * std::f64::consts::TAU).sin().mul_add(7000.0, 0.0) as i16
        })
        .collect()
}

/// Encode `count` consecutive real speech frames at `mode`.
///
/// Consecutive, from one encoder run, because a multi-frame payload carries a
/// frame *block* — 20 ms neighbours — and the dissector's length accounting
/// is only meaningfully exercised by genuine back-to-back frames.
fn speech_frames(variant: AmrVariant, mode: AmrMode, count: usize) -> Vec<Vec<u8>> {
    let mut config = match variant {
        AmrVariant::NarrowBand => CodecConfig::amr_nb(),
        AmrVariant::WideBand => CodecConfig::amr_wb(),
    };
    config.parameters.amr.mode_set = 1u16 << mode.index();
    let mut codec = codec_core::codecs::amr::AmrCodec::new(&config).expect("codec");
    let rate = match variant {
        AmrVariant::NarrowBand => 8_000,
        AmrVariant::WideBand => 16_000,
    };
    // A couple of frames in, so the encoder is past its cold start.
    for i in 0..3 {
        let _ = codec.encode_frame(&tone(rate, i));
    }
    (0..count)
        .map(|offset| {
            let coded: CodedFrame = codec.encode_frame(&tone(rate, 3 + offset)).expect("encodes");
            assert_eq!(coded.kind, FrameKind::Speech);
            coded.data
        })
        .collect()
}

/// A 12-byte RTP header plus payload, as text2pcap hex.
fn emit(case: usize, payload_type: u8, sequence: u16, payload: &[u8]) {
    let mut packet = Vec::with_capacity(12 + payload.len());
    packet.push(0x80); // V=2, no padding, no extension, CC=0
    packet.push(payload_type & 0x7F); // M=0
    packet.extend_from_slice(&sequence.to_be_bytes());
    packet.extend_from_slice(&(u32::from(sequence) * 160).to_be_bytes());
    packet.extend_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
    packet.extend_from_slice(payload);

    // text2pcap wants an offset then hex octets, one packet per block.
    for (row, chunk) in packet.chunks(16).enumerate() {
        let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
        println!("{:06x} {}", row * 16, hex.join(" "));
    }
    println!();
    let _ = case;
}

fn main() {
    // Wireshark's AMR framing and variant are *global* preferences, so one
    // capture can only carry one combination. The caller picks which.
    let which = std::env::args().nth(1).unwrap_or_else(|| "nb-oa".to_string());
    // A `-2f` suffix packs two frames per payload, exercising the F-bit chain
    // in the table of contents; the plain selections stay single-frame.
    let (base, frames_per_payload) = match which.strip_suffix("-2f") {
        Some(base) => (base, 2usize),
        None => (which.as_str(), 1),
    };
    let selected = match base {
        "nb-oa" => (AmrVariant::NarrowBand, true, 107u8),
        "nb-be" => (AmrVariant::NarrowBand, false, 106),
        "wb-oa" => (AmrVariant::WideBand, true, 105),
        "wb-be" => (AmrVariant::WideBand, false, 104),
        other => panic!(
            "unknown selection `{other}`; use nb-oa, nb-be, wb-oa or wb-be, optionally with -2f"
        ),
    };

    let mut case = 0usize;
    let mut sequence = 0u16;

    for (variant, octet_aligned, payload_type) in [selected] {
        let packer = AmrPayloadCodec::new(AmrPayloadConfig {
            variant,
            octet_aligned,
            crc: false,
            robust_sorting: false,
            interleaving: false,
        })
        .expect("packer");

        let modes: Vec<AmrMode> = AmrMode::all(variant);
        for mode in modes {
            let frames_data = speech_frames(variant, mode, frames_per_payload);
            // One with no mode request, one requesting the lowest mode, so the
            // CMR nibble is exercised at both its "absent" and a real value.
            for cmr in [None, Some(0u8)] {
                let frames: Vec<AmrPayloadFrame> = frames_data
                    .iter()
                    .map(|data| {
                        AmrPayloadFrame::new(AmrFrameType::Speech(mode), true, data.clone())
                            .expect("frame")
                    })
                    .collect();
                let packet = AmrPacket {
                    cmr,
                    interleaving: None,
                    frames,
                };
                let payload = packer.pack(&packet).expect("packs");
                case += 1;
                let ft_list = std::iter::repeat_n(mode.index().to_string(), frames_per_payload)
                    .collect::<Vec<_>>()
                    .join(",");
                eprintln!(
                    "# case {case} variant={} framing={} pt={payload_type} ft={ft_list} cmr={} bytes={}",
                    match variant {
                        AmrVariant::NarrowBand => "nb",
                        AmrVariant::WideBand => "wb",
                    },
                    if octet_aligned { "oa" } else { "be" },
                    cmr.map_or_else(|| "none".to_string(), |c| c.to_string()),
                    payload.len(),
                );
                emit(case, payload_type, sequence, &payload);
                sequence = sequence.wrapping_add(1);
            }
        }
    }
}
