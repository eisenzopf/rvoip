use std::path::Path;

#[cfg(feature = "annex_b")]
use g729::annex_b::cng::CngState;
use g729::api::FrameType;
use g729::bitstream::itu_serial::{BIT_0, BIT_1, SYNC_WORD, parse_stream};
#[cfg(feature = "annex_b")]
use g729::codec::decode::decode_annex_b_frame_words;
use g729::codec::decode::{decode_frame_typed, decode_speech_frame_words};
use g729::codec::state::DecoderState;

pub fn read_serial_frames(path: &Path) -> Vec<(u16, Vec<u16>)> {
    let raw =
        std::fs::read(path).unwrap_or_else(|e| panic!("failed to read {}: {}", path.display(), e));
    parse_stream(&raw)
}

pub fn decode_annex_a_bit_file(path: &Path) -> Vec<i16> {
    let frames = read_serial_frames(path);
    let mut state = DecoderState::default();
    let mut out = Vec::new();

    for (_sync, words) in frames {
        let frame = if words.len() == 80 {
            decode_speech_frame_words(&mut state, &words)
        } else {
            decode_frame_typed(&mut state, FrameType::NoData, &[])
        };
        out.extend_from_slice(&frame);
    }

    out
}

#[cfg(feature = "annex_b")]
pub fn decode_annex_b_bit_file(path: &Path) -> Vec<i16> {
    let frames = read_serial_frames(path);
    let mut state = DecoderState::default();
    let mut cng = CngState::default();
    let mut out = Vec::new();

    for (sync, words) in frames {
        let frame_type = match words.len() {
            80 => FrameType::Speech,
            16 | 15 => FrameType::Sid,
            0 => FrameType::NoData,
            _ => FrameType::NoData,
        };

        let bfi = if words.is_empty() {
            if sync == SYNC_WORD { 0 } else { 1 }
        } else if words.contains(&0) {
            1
        } else {
            0
        };

        let frame = decode_annex_b_frame_words(&mut state, &mut cng, frame_type, &words, bfi);
        out.extend_from_slice(&frame);
    }

    out
}

pub fn speech_words_from_payload(payload: &[u8; 10]) -> Vec<u16> {
    let mut bits = vec![BIT_0; 80];
    for i in 0..80 {
        let bit = (payload[i / 8] >> (7 - (i % 8))) & 1;
        bits[i] = if bit == 1 { BIT_1 } else { BIT_0 };
    }
    bits
}

pub fn sid_words_from_payload(payload: &[u8; 10]) -> Vec<u16> {
    let mut bits = vec![BIT_0; 16];
    for i in 0..16 {
        let bit = (payload[i / 8] >> (7 - (i % 8))) & 1;
        bits[i] = if bit == 1 { BIT_1 } else { BIT_0 };
    }
    bits
}

pub fn compare_word_frames(
    reference: &[(u16, Vec<u16>)],
    test: &[Vec<u16>],
    name: &str,
) -> (usize, usize, Option<usize>) {
    let len = reference.len().min(test.len());
    let mut matches = 0usize;
    let mut first_diverge = None;

    for i in 0..len {
        if reference[i].1 == test[i] {
            matches += 1;
        } else if first_diverge.is_none() {
            first_diverge = Some(i);
            eprintln!(
                "{}: frame {} differs (ref_bits={}, test_bits={})",
                name,
                i,
                reference[i].1.len(),
                test[i].len()
            );
        }
    }

    if reference.len() != test.len() {
        eprintln!(
            "{}: frame count mismatch ref={} test={}",
            name,
            reference.len(),
            test.len()
        );
    }

    (matches, len, first_diverge)
}
