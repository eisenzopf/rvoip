//! Round-trip integration tests: encode then decode and verify output quality.

#![cfg(all(feature = "std", feature = "itu_serial"))]

use g729::constants::PRM_SIZE;
use g729::{FRAME_SAMPLES, FrameType, G729Config, G729Decoder, G729Encoder};

fn round_trip(input: &[i16], annex_b: bool) -> Vec<i16> {
    let mut encoder = G729Encoder::new(G729Config { annex_b });
    let mut decoder = G729Decoder::new(G729Config { annex_b });
    let mut output = Vec::with_capacity(input.len());

    let num_frames = input.len() / FRAME_SAMPLES;
    for frame_idx in 0..num_frames {
        let start = frame_idx * FRAME_SAMPLES;
        let mut frame_in = [0i16; FRAME_SAMPLES];
        frame_in.copy_from_slice(&input[start..start + FRAME_SAMPLES]);

        let mut ana = [0i16; PRM_SIZE + 1];
        let (frame_type, _) = encoder
            .encode_parm(&frame_in, &mut ana)
            .expect("encode_parm should succeed with fixed-size buffers");

        let mut parm = [0i16; PRM_SIZE + 2 + 4];
        match frame_type {
            FrameType::Speech => {
                parm[0] = 0;
                parm[1] = 1;
                parm[2..2 + PRM_SIZE].copy_from_slice(&ana[1..1 + PRM_SIZE]);
            }
            FrameType::Sid => {
                parm[0] = 0;
                parm[1] = 2;
                parm[2..6].copy_from_slice(&ana[1..5]);
            }
            FrameType::NoData => {
                parm[0] = 1;
                parm[1] = 0;
            }
        }

        let mut frame_out = [0i16; FRAME_SAMPLES];
        decoder
            .decode_parm(&mut parm, &mut frame_out)
            .expect("decode_parm should succeed with fixed-size buffers");
        output.extend_from_slice(&frame_out);
    }

    output
}

fn generate_sine(freq_hz: f64, num_samples: usize, amplitude: i16) -> Vec<i16> {
    (0..num_samples)
        .map(|i| {
            let t = i as f64 / 8000.0;
            (amplitude as f64 * (2.0 * std::f64::consts::PI * freq_hz * t).sin()) as i16
        })
        .collect()
}

#[test]
fn round_trip_silence() {
    let silence = vec![0i16; FRAME_SAMPLES * 20];
    let output = round_trip(&silence, false);
    assert_eq!(output.len(), silence.len());

    let late_energy: f64 = output[FRAME_SAMPLES * 5..]
        .iter()
        .map(|&s| (s as f64).powi(2))
        .sum::<f64>()
        / (output.len() - FRAME_SAMPLES * 5) as f64;
    assert!(late_energy.sqrt() < 500.0);
}

#[test]
fn round_trip_sine_300hz() {
    let input = generate_sine(300.0, FRAME_SAMPLES * 50, 10_000);
    let output = round_trip(&input, false);

    let skip = FRAME_SAMPLES * 5;
    let input_slice = &input[skip..];
    let output_slice = &output[skip..];

    let mean_in = input_slice.iter().map(|&x| x as f64).sum::<f64>() / input_slice.len() as f64;
    let mean_out = output_slice.iter().map(|&x| x as f64).sum::<f64>() / output_slice.len() as f64;

    let mut cov = 0.0;
    let mut var_in = 0.0;
    let mut var_out = 0.0;
    for i in 0..input_slice.len() {
        let a = input_slice[i] as f64 - mean_in;
        let b = output_slice[i] as f64 - mean_out;
        cov += a * b;
        var_in += a * a;
        var_out += b * b;
    }

    let corr = cov / (var_in.sqrt() * var_out.sqrt());
    assert!(corr.abs() > 0.8, "correlation too low: {corr:.4}");
}

#[cfg(feature = "annex_b")]
#[test]
fn round_trip_annex_b_speech_then_silence() {
    let mut input = generate_sine(300.0, FRAME_SAMPLES * 30, 10_000);
    input.extend_from_slice(&vec![0i16; FRAME_SAMPLES * 30]);
    let output = round_trip(&input, true);
    assert_eq!(output.len(), input.len());
}

#[test]
fn tandem_encoding() {
    let input = generate_sine(300.0, FRAME_SAMPLES * 50, 10_000);
    let pass1 = round_trip(&input, false);
    let pass2 = round_trip(&pass1, false);
    let pass3 = round_trip(&pass2, false);

    let skip = FRAME_SAMPLES * 5;
    for (label, output) in [("pass1", &pass1), ("pass2", &pass2), ("pass3", &pass3)] {
        let energy = output[skip..]
            .iter()
            .map(|&s| (s as f64).powi(2))
            .sum::<f64>()
            / (output.len() - skip) as f64;
        assert!(energy.sqrt() > 100.0, "{label} degraded to near-silence");
    }
}

#[test]
fn long_duration_session() {
    let mut encoder = G729Encoder::new(G729Config { annex_b: false });
    let mut decoder = G729Decoder::new(G729Config { annex_b: false });

    let mut bits = [0u8; 10];
    let mut out = [0i16; FRAME_SAMPLES];

    for frame_idx in 0..75_000usize {
        let mut pcm = [0i16; FRAME_SAMPLES];
        match frame_idx % 300 {
            0..150 => {
                for (i, sample) in pcm.iter_mut().enumerate() {
                    let t = (frame_idx * FRAME_SAMPLES + i) as f64 / 8000.0;
                    *sample = (8000.0 * (2.0 * std::f64::consts::PI * 300.0 * t).sin()) as i16;
                }
            }
            150..250 => {}
            _ => {
                let seed = (frame_idx * 7 + 13) as u32;
                for (i, sample) in pcm.iter_mut().enumerate() {
                    *sample =
                        (((seed.wrapping_mul(i as u32 + 1).wrapping_add(37)) % 201) as i16) - 100;
                }
            }
        }

        let frame_type = encoder.encode(&pcm, &mut bits);
        assert_eq!(frame_type, FrameType::Speech);
        decoder.decode(&bits, &mut out);

        if frame_idx % 10_000 == 9_999 {
            let max_abs = out.iter().map(|&s| (s as i32).abs()).max().unwrap_or(0);
            assert!(max_abs <= 32767);
        }
    }
}
