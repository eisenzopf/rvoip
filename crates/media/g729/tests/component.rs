use g729::{CodecError, DecoderConfig, EncoderConfig, FrameType, G729Decoder, G729Encoder};

#[test]
fn component_encode_decode_smoke() {
    let mut enc = G729Encoder::new(EncoderConfig::default());
    let mut dec = G729Decoder::new(DecoderConfig::default());

    let mut pcm = [0i16; 80];
    for (i, s) in pcm.iter_mut().enumerate() {
        *s = ((i as i16) - 40) * 100;
    }

    let mut bits = [0u8; 10];
    let _frame_type = enc.encode(&pcm, &mut bits);

    let mut out = [0i16; 80];
    dec.decode(&bits, &mut out);

    assert!(out.iter().any(|x| *x != 0));
}

#[test]
fn component_decode_erasure_path_smoke() {
    let mut dec = G729Decoder::new(DecoderConfig::default());
    let mut out = [0i16; 80];

    dec.decode_erasure(&mut out);
    dec.decode_erasure(&mut out);

    assert_eq!(out.len(), 80);
}

#[test]
fn component_decoder_post_filter_flag_is_effective() {
    let mut enc = G729Encoder::new(EncoderConfig::default());
    let mut pcm = [0i16; 80];
    for (i, s) in pcm.iter_mut().enumerate() {
        *s = ((i as i16) - 20) * 123;
    }

    let mut bits = [0u8; 10];
    let _ = enc.encode(&pcm, &mut bits);

    let mut dec_pf = G729Decoder::new(DecoderConfig {
        annex_b: false,
        post_filter: true,
        max_consecutive_erasures: None,
    });
    let mut dec_no_pf = G729Decoder::new(DecoderConfig {
        annex_b: false,
        post_filter: false,
        max_consecutive_erasures: None,
    });

    let mut out_pf = [0i16; 80];
    let mut out_no_pf = [0i16; 80];
    dec_pf.decode(&bits, &mut out_pf);
    dec_no_pf.decode(&bits, &mut out_no_pf);

    assert_ne!(out_pf, out_no_pf);
}

#[cfg(feature = "annex_b")]
#[test]
fn component_annex_b_sid_nodata_path() {
    let mut enc = G729Encoder::new(EncoderConfig { annex_b: true });
    let mut bits = [0u8; 10];
    let pcm = [0i16; 80];

    let mut saw_non_speech = false;
    for _ in 0..12 {
        let t = enc.encode(&pcm, &mut bits);
        if !matches!(t, g729::FrameType::Speech) {
            saw_non_speech = true;
            break;
        }
    }

    assert!(saw_non_speech);
}

#[test]
fn component_decoder_erasure_muting_limit_is_applied() {
    let mut enc = G729Encoder::new(EncoderConfig { annex_b: false });
    let mut dec = G729Decoder::new(DecoderConfig {
        annex_b: false,
        post_filter: true,
        max_consecutive_erasures: Some(0),
    });

    let mut pcm = [0i16; 80];
    for (i, s) in pcm.iter_mut().enumerate() {
        *s = ((i as i16) - 32) * 137;
    }

    let mut bits = [0u8; 10];
    let _ = enc.encode(&pcm, &mut bits);

    let mut out = [0i16; 80];
    dec.decode(&bits, &mut out);
    assert!(out.iter().any(|&s| s != 0));

    dec.decode_erasure(&mut out);
    assert!(out.iter().all(|&s| s == 0));
}

#[test]
fn component_decoder_decode_treats_invalid_length_as_nodata() {
    let mut tolerant = G729Decoder::new(DecoderConfig::default());
    let mut explicit = G729Decoder::new(DecoderConfig::default());

    let invalid = [1u8, 2u8, 3u8];
    let mut out_tolerant = [0i16; 80];
    let mut out_explicit = [0i16; 80];

    tolerant.decode(&invalid, &mut out_tolerant);
    explicit.decode_with_type(&invalid, FrameType::NoData, &mut out_explicit);

    assert_eq!(out_tolerant, out_explicit);
}

#[test]
fn component_decoder_decode_frame_rejects_invalid_length() {
    let mut dec = G729Decoder::new(DecoderConfig::default());
    let err = dec
        .decode_frame(&[1u8, 2u8, 3u8])
        .expect_err("decode_frame should reject invalid payload lengths");

    assert_eq!(
        err,
        CodecError::InvalidBitstreamLength {
            expected: &[0, 2, 10],
            got: 3,
        }
    );
}

#[test]
fn component_encoder_encode_frame_rejects_invalid_pcm_length() {
    let mut enc = G729Encoder::new(EncoderConfig::default());
    let short = [0i16; 79];
    let err = enc
        .encode_frame(&short)
        .expect_err("encode_frame should reject non-80-sample input");

    assert_eq!(
        err,
        CodecError::InvalidPcmLength {
            expected: 80,
            got: 79,
        }
    );
}

#[cfg(all(feature = "annex_b", feature = "itu_serial"))]
#[test]
fn component_decode_parm_annex_b_nodata_good_bfi_matches_reference() {
    use g729::G729Config;
    use g729::annex_b::cng::CngState;
    use g729::bitstream::itu_params::pack_sid_params;
    use g729::codec::decode::decode_annex_b_frame;
    use g729::codec::state::DecoderState;
    use g729::constants::PRM_SIZE;
    use g729::dsp::Word16;

    let sid_params = [Word16(1), Word16(12), Word16(7), Word16(18)];
    let sid_bits = pack_sid_params(&sid_params);

    let mut ref_state = DecoderState::default();
    let mut ref_cng = CngState::default();
    let _ = decode_annex_b_frame(&mut ref_state, &mut ref_cng, FrameType::Sid, &sid_bits, 0);
    let expected = decode_annex_b_frame(&mut ref_state, &mut ref_cng, FrameType::NoData, &[], 0);

    let mut dec = G729Decoder::new(G729Config { annex_b: true });
    let mut sid_parm = [0i16; PRM_SIZE + 2 + 4];
    sid_parm[0] = 0;
    sid_parm[1] = 2;
    for i in 0..4 {
        sid_parm[i + 2] = sid_params[i].0;
    }
    let mut warmup = [0i16; 80];
    dec.decode_parm(&mut sid_parm, &mut warmup)
        .expect("decode_parm should accept a valid SID frame");

    let mut nodata_parm = [0i16; PRM_SIZE + 2 + 4];
    nodata_parm[0] = 0;
    nodata_parm[1] = 0;
    let mut actual = [0i16; 80];
    dec.decode_parm(&mut nodata_parm, &mut actual)
        .expect("decode_parm should accept a valid no-data frame");

    assert_eq!(actual, expected);
}

#[cfg(all(feature = "annex_b", feature = "itu_serial"))]
#[test]
fn component_decode_parm_annex_b_bad_frame_bfi_matches_reference() {
    use g729::G729Config;
    use g729::annex_b::cng::CngState;
    use g729::bitstream::itu_params::{pack_sid_params, pack_speech_params};
    use g729::codec::decode::decode_annex_b_frame;
    use g729::codec::state::DecoderState;
    use g729::constants::PRM_SIZE;
    use g729::dsp::Word16;

    let speech_params = [
        Word16(0),
        Word16(120),
        Word16(210),
        Word16(1),
        Word16(6200),
        Word16(15),
        Word16(100),
        Word16(19),
        Word16(5300),
        Word16(8),
        Word16(110),
    ];
    let speech_bits = pack_speech_params(&speech_params);

    let sid_params = [Word16(1), Word16(12), Word16(7), Word16(18)];
    let sid_bits = pack_sid_params(&sid_params);

    let mut ref_state = DecoderState::default();
    let mut ref_cng = CngState::default();
    let _ = decode_annex_b_frame(
        &mut ref_state,
        &mut ref_cng,
        FrameType::Speech,
        &speech_bits,
        0,
    );
    let expected_speech_bad = decode_annex_b_frame(
        &mut ref_state,
        &mut ref_cng,
        FrameType::Speech,
        &speech_bits,
        1,
    );
    let expected_sid_bad =
        decode_annex_b_frame(&mut ref_state, &mut ref_cng, FrameType::Sid, &sid_bits, 1);

    let mut dec = G729Decoder::new(G729Config { annex_b: true });
    let mut speech_good = [0i16; PRM_SIZE + 2 + 4];
    speech_good[0] = 0;
    speech_good[1] = 1;
    for i in 0..11 {
        speech_good[i + 2] = speech_params[i].0;
    }
    let mut speech_bad = speech_good;
    speech_bad[0] = 1;

    let mut sid_bad = [0i16; PRM_SIZE + 2 + 4];
    sid_bad[0] = 1;
    sid_bad[1] = 2;
    for i in 0..4 {
        sid_bad[i + 2] = sid_params[i].0;
    }

    let mut warmup = [0i16; 80];
    dec.decode_parm(&mut speech_good, &mut warmup)
        .expect("decode_parm should accept a valid speech frame");

    let mut actual_speech_bad = [0i16; 80];
    dec.decode_parm(&mut speech_bad, &mut actual_speech_bad)
        .expect("decode_parm should accept a bad speech frame");
    assert_eq!(actual_speech_bad, expected_speech_bad);

    let mut actual_sid_bad = [0i16; 80];
    dec.decode_parm(&mut sid_bad, &mut actual_sid_bad)
        .expect("decode_parm should accept a bad SID frame");
    assert_eq!(actual_sid_bad, expected_sid_bad);
}
