//! Deterministic G.711 tone quality tests.
//!
//! These tests validate the codec path used by rvoip-sip through media-core:
//! `G711Codec::new(...)`, then `AudioCodec::encode` and `AudioCodec::decode`.

use crate::codecs::g711::{G711Codec, G711Variant};
use crate::types::AudioCodec;

const SAMPLE_RATE_HZ: u32 = 8_000;
const TONE_HZ: f64 = 1_000.0;
const DURATION_SECONDS: usize = 10;
const AMPLITUDE: f64 = 10_000.0;
const CHANNELS: u8 = 1;
const EXPECTED_BITRATE_BPS: usize = 64_000;

#[derive(Debug)]
struct ToneMetrics {
    snr_db: f64,
    original_rms: f64,
    decoded_rms: f64,
    original_peak: u16,
    decoded_peak: u16,
    dc_offset: f64,
    tone_dominance_ratio: f64,
    window_count: usize,
    weakest_window_tone_dominance_ratio: f64,
}

#[test]
fn test_g711_10_second_tone_roundtrip_within_parameters() {
    let original = generate_tone();

    assert_eq!(
        original.len(),
        SAMPLE_RATE_HZ as usize * DURATION_SECONDS,
        "10 seconds at 8 kHz must produce 80,000 samples"
    );

    verify_variant(G711Variant::MuLaw, "PCMU", Some(0), &original);
    verify_variant(G711Variant::ALaw, "PCMA", Some(8), &original);
}

fn verify_variant(
    variant: G711Variant,
    expected_name: &str,
    expected_payload_type: Option<u8>,
    original: &[i16],
) {
    let mut codec = G711Codec::new(variant);
    let info = codec.info();

    assert_eq!(info.name, expected_name);
    assert_eq!(info.sample_rate, SAMPLE_RATE_HZ);
    assert_eq!(info.channels, CHANNELS);
    assert_eq!(info.bitrate as usize, EXPECTED_BITRATE_BPS);
    assert_eq!(info.payload_type, expected_payload_type);

    let encoded = codec
        .encode(original)
        .unwrap_or_else(|err| panic!("{} encode failed: {}", expected_name, err));
    assert_eq!(
        encoded.len(),
        original.len(),
        "{} encoded length must be one byte per input sample",
        expected_name
    );

    let effective_bitrate = (encoded.len() * 8) / DURATION_SECONDS;
    assert_eq!(
        effective_bitrate, EXPECTED_BITRATE_BPS,
        "{} effective bitrate must be 64 kbps",
        expected_name
    );

    let decoded = codec
        .decode(&encoded)
        .unwrap_or_else(|err| panic!("{} decode failed: {}", expected_name, err));
    assert_eq!(
        decoded.len(),
        original.len(),
        "{} decoded sample count must match input sample count",
        expected_name
    );

    let metrics = analyze_tone(original, &decoded);
    println!(
        "{} 10s tone: SNR={:.2} dB, RMS {:.2}->{:.2}, peak {}->{}, DC={:.2}, tone dominance={:.2}x, weakest 1s dominance={:.2}x",
        expected_name,
        metrics.snr_db,
        metrics.original_rms,
        metrics.decoded_rms,
        metrics.original_peak,
        metrics.decoded_peak,
        metrics.dc_offset,
        metrics.tone_dominance_ratio,
        metrics.weakest_window_tone_dominance_ratio
    );

    assert!(
        metrics.snr_db >= 30.0,
        "{} SNR too low for a 1 kHz G.711 tone: {:.2} dB",
        expected_name,
        metrics.snr_db
    );

    assert!(
        within_relative_tolerance(metrics.decoded_rms, metrics.original_rms, 0.10),
        "{} decoded RMS out of range: original {:.2}, decoded {:.2}",
        expected_name,
        metrics.original_rms,
        metrics.decoded_rms
    );

    assert!(
        within_relative_tolerance(
            f64::from(metrics.decoded_peak),
            f64::from(metrics.original_peak),
            0.15
        ),
        "{} decoded peak out of range: original {}, decoded {}",
        expected_name,
        metrics.original_peak,
        metrics.decoded_peak
    );

    assert!(
        metrics.decoded_peak <= i16::MAX as u16,
        "{} decoded peak must remain inside i16 range",
        expected_name
    );

    assert!(
        metrics.dc_offset.abs() < 100.0,
        "{} decoded tone has unexpected DC offset: {:.2}",
        expected_name,
        metrics.dc_offset
    );

    assert!(
        metrics.tone_dominance_ratio >= 20.0,
        "{} decoded 1 kHz tone is not dominant enough: {:.2}x",
        expected_name,
        metrics.tone_dominance_ratio
    );

    assert_eq!(
        metrics.window_count, DURATION_SECONDS,
        "{} should analyze one 1-second window for each generated second",
        expected_name
    );

    assert!(
        metrics.weakest_window_tone_dominance_ratio >= 20.0,
        "{} decoded 1 kHz tone is not present throughout the full 10 seconds: weakest 1s window {:.2}x",
        expected_name,
        metrics.weakest_window_tone_dominance_ratio
    );
}

fn generate_tone() -> Vec<i16> {
    let sample_count = SAMPLE_RATE_HZ as usize * DURATION_SECONDS;
    (0..sample_count)
        .map(|sample_index| {
            let t = sample_index as f64 / f64::from(SAMPLE_RATE_HZ);
            let sample = (2.0 * std::f64::consts::PI * TONE_HZ * t).sin() * AMPLITUDE;
            sample.round() as i16
        })
        .collect()
}

fn analyze_tone(original: &[i16], decoded: &[i16]) -> ToneMetrics {
    assert_eq!(original.len(), decoded.len());

    let signal_power: f64 = original
        .iter()
        .map(|&sample| f64::from(sample).powi(2))
        .sum();
    let noise_power: f64 = original
        .iter()
        .zip(decoded)
        .map(|(&original_sample, &decoded_sample)| {
            (f64::from(original_sample) - f64::from(decoded_sample)).powi(2)
        })
        .sum();
    let snr_db = if noise_power == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (signal_power / noise_power).log10()
    };

    let original_rms = rms(original);
    let decoded_rms = rms(decoded);
    let original_peak = peak_abs(original);
    let decoded_peak = peak_abs(decoded);
    let dc_offset =
        decoded.iter().map(|&sample| f64::from(sample)).sum::<f64>() / decoded.len() as f64;
    let tone_mag = goertzel_magnitude(decoded, TONE_HZ);
    let lower_adjacent_mag = goertzel_magnitude(decoded, TONE_HZ - 250.0);
    let upper_adjacent_mag = goertzel_magnitude(decoded, TONE_HZ + 250.0);
    let strongest_adjacent = lower_adjacent_mag.max(upper_adjacent_mag).max(1.0);
    let window_dominance_ratios = one_second_tone_dominance_ratios(decoded);

    ToneMetrics {
        snr_db,
        original_rms,
        decoded_rms,
        original_peak,
        decoded_peak,
        dc_offset,
        tone_dominance_ratio: tone_mag / strongest_adjacent,
        window_count: window_dominance_ratios.len(),
        weakest_window_tone_dominance_ratio: window_dominance_ratios
            .into_iter()
            .fold(f64::INFINITY, f64::min),
    }
}

fn rms(samples: &[i16]) -> f64 {
    let power = samples
        .iter()
        .map(|&sample| f64::from(sample).powi(2))
        .sum::<f64>()
        / samples.len() as f64;
    power.sqrt()
}

fn peak_abs(samples: &[i16]) -> u16 {
    samples
        .iter()
        .map(|&sample| sample.unsigned_abs())
        .max()
        .unwrap_or(0)
}

fn within_relative_tolerance(actual: f64, expected: f64, tolerance: f64) -> bool {
    let delta = (actual - expected).abs();
    delta <= expected.abs() * tolerance
}

fn goertzel_magnitude(samples: &[i16], target_hz: f64) -> f64 {
    let n = samples.len() as f64;
    let k = (0.5 + (n * target_hz) / f64::from(SAMPLE_RATE_HZ)).floor();
    let omega = (2.0 * std::f64::consts::PI * k) / n;
    let coeff = 2.0 * omega.cos();
    let mut q1 = 0.0;
    let mut q2 = 0.0;

    for &sample in samples {
        let q0 = coeff * q1 - q2 + f64::from(sample);
        q2 = q1;
        q1 = q0;
    }

    (q1 * q1 + q2 * q2 - q1 * q2 * coeff).sqrt()
}

fn one_second_tone_dominance_ratios(samples: &[i16]) -> Vec<f64> {
    samples
        .chunks_exact(SAMPLE_RATE_HZ as usize)
        .map(|window| {
            let tone_mag = goertzel_magnitude(window, TONE_HZ);
            let lower_adjacent_mag = goertzel_magnitude(window, TONE_HZ - 250.0);
            let upper_adjacent_mag = goertzel_magnitude(window, TONE_HZ + 250.0);
            let strongest_adjacent = lower_adjacent_mag.max(upper_adjacent_mag).max(1.0);
            tone_mag / strongest_adjacent
        })
        .collect()
}
