//! Executable G.711 conformance regressions.
//!
//! These tests use known ITU-T G.711 codepoints and exact companding
//! stability properties. They deliberately do not claim evidence from the
//! G.191 derivative files referenced by the historical test suite: those
//! files are not part of this repository and G.191 is not a G.711 wire-vector
//! format.

use crate::codecs::g711::{alaw_compress, alaw_expand, ulaw_compress, ulaw_expand};

const SWEEP_SOURCE: &[u8] = include_bytes!("test_data/sweep.src");

// (linear PCM, A-law, mu-law, expanded A-law, expanded mu-law)
const KNOWN_CODEPOINTS: &[(i16, u8, u8, i16, i16)] = &[
    (0, 0xd5, 0xff, 8, 0),
    (128, 0xdd, 0xef, 136, 132),
    (256, 0xc5, 0xe7, 264, 260),
    (512, 0xf5, 0xdb, 528, 524),
    (1024, 0xe5, 0xcd, 1056, 1052),
    (-128, 0x52, 0x6f, -120, -132),
    (-256, 0x5a, 0x67, -248, -260),
    (-512, 0x4a, 0x5b, -504, -524),
    (-1024, 0x7a, 0x4d, -1008, -1052),
];

fn sweep_samples() -> impl Iterator<Item = i16> {
    assert_eq!(
        SWEEP_SOURCE.len() % 2,
        0,
        "checked-in PCM sweep must contain complete big-endian i16 samples"
    );
    SWEEP_SOURCE
        .chunks_exact(2)
        .map(|bytes| i16::from_be_bytes([bytes[0], bytes[1]]))
}

fn assert_known_encodings() {
    for &(linear, expected_alaw, expected_ulaw, _, _) in KNOWN_CODEPOINTS {
        assert_eq!(
            alaw_compress(linear),
            expected_alaw,
            "A-law encoding mismatch for PCM {linear}"
        );
        assert_eq!(
            ulaw_compress(linear),
            expected_ulaw,
            "mu-law encoding mismatch for PCM {linear}"
        );
    }
}

fn assert_known_decodings() {
    for &(_, alaw, ulaw, expected_alaw, expected_ulaw) in KNOWN_CODEPOINTS {
        assert_eq!(
            alaw_expand(alaw),
            expected_alaw,
            "A-law decoding mismatch for codepoint {alaw:#04x}"
        );
        assert_eq!(
            ulaw_expand(ulaw),
            expected_ulaw,
            "mu-law decoding mismatch for codepoint {ulaw:#04x}"
        );
    }
}

fn assert_sweep_companding_stability() {
    for linear in sweep_samples() {
        let alaw = alaw_compress(linear);
        assert_eq!(
            alaw_compress(alaw_expand(alaw)),
            alaw,
            "A-law codepoint changed after expansion for PCM {linear}"
        );

        let ulaw = ulaw_compress(linear);
        let ulaw_linear = ulaw_expand(ulaw);
        assert_eq!(
            ulaw_expand(ulaw_compress(ulaw_linear)),
            ulaw_linear,
            "mu-law quantized value changed after recompression for PCM {linear}"
        );
    }
}

fn assert_exhaustive_decode_stability() {
    for codepoint in u8::MIN..=u8::MAX {
        let alaw_linear = alaw_expand(codepoint);
        assert_eq!(
            alaw_expand(alaw_compress(alaw_linear)),
            alaw_linear,
            "A-law decoded value changed for codepoint {codepoint:#04x}"
        );

        let ulaw_linear = ulaw_expand(codepoint);
        assert_eq!(
            ulaw_expand(ulaw_compress(ulaw_linear)),
            ulaw_linear,
            "mu-law decoded value changed for codepoint {codepoint:#04x}"
        );
    }
}

#[test]
fn test_alaw_encoding_compliance() {
    for &(linear, expected, _, _, _) in KNOWN_CODEPOINTS {
        assert_eq!(alaw_compress(linear), expected);
    }
}

#[test]
fn test_mulaw_encoding_compliance() {
    for &(linear, _, expected, _, _) in KNOWN_CODEPOINTS {
        assert_eq!(ulaw_compress(linear), expected);
    }
}

#[test]
fn test_alaw_decoding_compliance() {
    for &(_, encoded, _, expected, _) in KNOWN_CODEPOINTS {
        assert_eq!(alaw_expand(encoded), expected);
    }
}

#[test]
fn test_mulaw_decoding_compliance() {
    for &(_, _, encoded, _, expected) in KNOWN_CODEPOINTS {
        assert_eq!(ulaw_expand(encoded), expected);
    }
}

#[test]
fn test_g711_self_consistency() {
    assert_sweep_companding_stability();
}

#[test]
fn test_algorithm_correctness() {
    assert_known_encodings();
    assert_known_decodings();
}

#[test]
fn test_itu_compliance_summary() {
    assert_known_encodings();
    assert_known_decodings();
    assert_exhaustive_decode_stability();
    assert_sweep_companding_stability();
}
