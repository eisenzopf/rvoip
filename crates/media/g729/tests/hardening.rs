use core::mem::size_of;

use g729::codec::state::{DecoderState, EncoderState};
use g729::{DecoderConfig, EncoderConfig, FrameType, G729Decoder, G729Encoder};

#[test]
fn send_bounds_are_satisfied() {
    fn assert_send<T: Send>() {}
    assert_send::<G729Encoder>();
    assert_send::<G729Decoder>();
}

#[test]
fn size_assertions_hold() {
    assert!(size_of::<EncoderState>() < 8 * 1024);
    assert!(size_of::<DecoderState>() < 4 * 1024);
}

fn gen_frame(seed: &mut u32, frame_idx: usize) -> [i16; 80] {
    let mut frame = [0i16; 80];
    if frame_idx % 50 < 8 {
        // Periodic silence windows to exercise VAD/DTX transitions.
        return frame;
    }

    for s in &mut frame {
        *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let v = ((*seed >> 16) as i16) >> 1;
        *s = v;
    }
    frame
}

#[test]
fn determinism_repeated_encode_sequence() {
    let mut enc_a = G729Encoder::new(EncoderConfig { annex_b: true });
    let mut enc_b = G729Encoder::new(EncoderConfig { annex_b: true });
    let mut seed_a = 1u32;
    let mut seed_b = 1u32;

    for i in 0..240 {
        let pcm_a = gen_frame(&mut seed_a, i);
        let pcm_b = gen_frame(&mut seed_b, i);
        assert_eq!(pcm_a, pcm_b);

        let mut bits_a = [0u8; 10];
        let mut bits_b = [0u8; 10];
        let ty_a = enc_a.encode(&pcm_a, &mut bits_a);
        let ty_b = enc_b.encode(&pcm_b, &mut bits_b);

        assert_eq!(ty_a, ty_b);
        assert_eq!(bits_a, bits_b);
    }
}

#[test]
fn erasure_burst_handling_smoke() {
    let mut enc = G729Encoder::new(EncoderConfig { annex_b: false });
    let mut dec = G729Decoder::new(DecoderConfig {
        annex_b: false,
        post_filter: true,
        max_consecutive_erasures: None,
    });

    // Prime decoder state with voiced frames.
    let mut seed = 99u32;
    for i in 0..8 {
        let pcm = gen_frame(&mut seed, 100 + i);
        let mut bits = [0u8; 10];
        let ty = enc.encode(&pcm, &mut bits);
        assert!(matches!(ty, FrameType::Speech));
        let mut out = [0i16; 80];
        dec.decode(&bits, &mut out);
    }

    for _ in 0..16 {
        let mut out = [0i16; 80];
        dec.decode_erasure(&mut out);
    }

    // Decoder should recover and continue producing valid samples after a burst.
    let pcm = gen_frame(&mut seed, 509);
    let mut bits = [0u8; 10];
    let ty = enc.encode(&pcm, &mut bits);
    assert!(matches!(ty, FrameType::Speech));
    let mut recovered = [0i16; 80];
    dec.decode(&bits, &mut recovered);
    assert!(recovered.iter().any(|&s| s != 0));
}

#[test]
fn long_session_encode_decode_stability() {
    let mut enc = G729Encoder::new(EncoderConfig { annex_b: true });
    let mut dec = G729Decoder::new(DecoderConfig {
        annex_b: true,
        post_filter: true,
        max_consecutive_erasures: None,
    });
    let mut seed = 7u32;
    let mut checksum = 0i64;

    for i in 0..1200 {
        let pcm = gen_frame(&mut seed, i);
        let mut bits = [0u8; 10];
        let ty = enc.encode(&pcm, &mut bits);

        let mut out = [0i16; 80];
        match ty {
            FrameType::Speech => dec.decode_with_type(&bits, FrameType::Speech, &mut out),
            FrameType::Sid => dec.decode_with_type(&bits[..2], FrameType::Sid, &mut out),
            FrameType::NoData => dec.decode_with_type(&[], FrameType::NoData, &mut out),
        }

        checksum += i64::from(out[0]) + i64::from(out[39]) + i64::from(out[79]);
    }

    assert_ne!(checksum, 0);
}

#[test]
fn tandem_encode_decode_smoke() {
    let mut enc_1 = G729Encoder::new(EncoderConfig { annex_b: false });
    let mut dec_1 = G729Decoder::new(DecoderConfig {
        annex_b: false,
        post_filter: true,
        max_consecutive_erasures: None,
    });
    let mut enc_2 = G729Encoder::new(EncoderConfig { annex_b: false });
    let mut dec_2 = G729Decoder::new(DecoderConfig {
        annex_b: false,
        post_filter: true,
        max_consecutive_erasures: None,
    });

    let mut seed = 123u32;
    let mut cur = gen_frame(&mut seed, 0);

    for i in 0..120 {
        if i > 0 {
            cur = gen_frame(&mut seed, i);
        }

        let mut bits_1 = [0u8; 10];
        let mut bits_2 = [0u8; 10];
        let t1 = enc_1.encode(&cur, &mut bits_1);
        assert!(matches!(t1, FrameType::Speech));

        let mut d1 = [0i16; 80];
        dec_1.decode(&bits_1, &mut d1);

        let t2 = enc_2.encode(&d1, &mut bits_2);
        assert!(matches!(t2, FrameType::Speech));

        let mut d2 = [0i16; 80];
        dec_2.decode(&bits_2, &mut d2);
        cur = d2;
    }

    assert!(cur.iter().any(|&s| s != 0));
}
