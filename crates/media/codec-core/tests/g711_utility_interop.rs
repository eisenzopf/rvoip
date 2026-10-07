use codec_core::{
    codecs::g711,
    utils::{simd, tables},
};

#[test]
fn utility_g711_matches_independent_wire_vectors() {
    // Fixed G.711 wire values, independent of the implementation under test.
    for (pcm, mulaw, alaw) in [
        (0, 0xff, 0xd5),
        (1000, 0xce, 0xfa),
        (-1000, 0x4e, 0x7a),
        (i16::MAX, 0x80, 0xaa),
        (i16::MIN, 0x00, 0x2a),
    ] {
        assert_eq!(simd::linear_to_mulaw_scalar(pcm), mulaw);
        assert_eq!(simd::linear_to_alaw_scalar(pcm), alaw);
    }
    for (wire, pcm) in [(0xff, 0), (0x7f, 0), (0x00, -32124), (0x80, 32124)] {
        assert_eq!(simd::mulaw_to_linear_scalar(wire), pcm);
        assert_eq!(tables::decode_mulaw_table(wire), pcm);
    }
    for (wire, pcm) in [(0xd5, 8), (0x55, -8), (0xaa, 32256), (0x2a, -32256)] {
        assert_eq!(simd::alaw_to_linear_scalar(wire), pcm);
        assert_eq!(tables::decode_alaw_table(wire), pcm);
    }
}

#[test]
fn every_pcm_value_has_identical_scalar_table_and_dispatched_encoding() {
    let pcm: Vec<_> = (i16::MIN..=i16::MAX).collect();
    let expected_mu: Vec<_> = pcm.iter().copied().map(g711::ulaw_compress).collect();
    let expected_a: Vec<_> = pcm.iter().copied().map(g711::alaw_compress).collect();
    let mut output = vec![0; pcm.len()];
    for (encode, expected) in [
        (
            simd::encode_mulaw_scalar as fn(&[i16], &mut [u8]),
            &expected_mu,
        ),
        (simd::encode_mulaw_optimized, &expected_mu),
        (tables::encode_mulaw_batch, &expected_mu),
        (simd::encode_alaw_scalar, &expected_a),
        (simd::encode_alaw_optimized, &expected_a),
        (tables::encode_alaw_batch, &expected_a),
    ] {
        encode(&pcm, &mut output);
        assert_eq!(&output, expected);
    }
}

#[test]
fn every_wire_byte_decodes_identically_across_public_utilities() {
    for wire in u8::MIN..=u8::MAX {
        assert_eq!(simd::mulaw_to_linear_scalar(wire), g711::ulaw_expand(wire));
        assert_eq!(tables::decode_mulaw_table(wire), g711::ulaw_expand(wire));
        assert_eq!(simd::alaw_to_linear_scalar(wire), g711::alaw_expand(wire));
        assert_eq!(tables::decode_alaw_table(wire), g711::alaw_expand(wire));
    }
}
