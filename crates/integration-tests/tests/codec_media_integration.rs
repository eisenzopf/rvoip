//! Cross-crate integration tests for codec + media pipeline.
//!
//! Tests the codec-core G.711 codecs (PCMU and PCMA) for encode/decode round-trip
//! fidelity, verifying that audio samples survive compression and expansion with
//! acceptable quality.

use codec_core::codecs::g711::{G711Codec, G711Variant};

/// Number of PCM samples per 20ms frame at 8kHz.
const SAMPLES_PER_FRAME: usize = 160;

/// Generate a sine wave tone as PCM i16 samples.
fn generate_sine_tone(frequency_hz: f32, sample_rate: u32, num_samples: usize) -> Vec<i16> {
    let mut samples = Vec::with_capacity(num_samples);
    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;
        let value = (2.0 * std::f32::consts::PI * frequency_hz * t).sin();
        samples.push((value * 16000.0) as i16);
    }
    samples
}

// =============================================================================
// Test 1: PCMU (mu-law) round-trip preserves audio fidelity
// =============================================================================

#[test]
fn test_pcmu_round_trip_fidelity() {
    let codec = G711Codec::new(G711Variant::MuLaw);

    // Generate a 440 Hz tone (one frame = 20ms at 8kHz = 160 samples)
    let input = generate_sine_tone(440.0, 8000, SAMPLES_PER_FRAME);
    assert_eq!(input.len(), SAMPLES_PER_FRAME);

    // Encode (compress)
    let encoded = codec.compress(&input).expect("PCMU encode should succeed");
    assert_eq!(
        encoded.len(),
        SAMPLES_PER_FRAME,
        "G.711 produces 1 byte per sample"
    );

    // Decode (expand)
    let decoded = codec.expand(&encoded).expect("PCMU decode should succeed");
    assert_eq!(
        decoded.len(),
        SAMPLES_PER_FRAME,
        "Decoded output should have same sample count as input"
    );

    // Verify fidelity: compute SNR
    let snr = compute_snr(&input, &decoded);
    assert!(
        snr > 30.0,
        "PCMU round-trip SNR should be > 30 dB, got {:.1} dB",
        snr
    );
}

// =============================================================================
// Test 2: PCMA (A-law) round-trip preserves audio fidelity
// =============================================================================

#[test]
fn test_pcma_round_trip_fidelity() {
    let codec = G711Codec::new(G711Variant::ALaw);

    let input = generate_sine_tone(1000.0, 8000, SAMPLES_PER_FRAME);

    let encoded = codec.compress(&input).expect("PCMA encode should succeed");
    assert_eq!(encoded.len(), SAMPLES_PER_FRAME);

    let decoded = codec.expand(&encoded).expect("PCMA decode should succeed");
    assert_eq!(decoded.len(), SAMPLES_PER_FRAME);

    let snr = compute_snr(&input, &decoded);
    assert!(
        snr > 30.0,
        "PCMA round-trip SNR should be > 30 dB, got {:.1} dB",
        snr
    );
}

// =============================================================================
// Test 3: Multiple frames encode/decode consistently
// =============================================================================

#[test]
fn test_multi_frame_codec_consistency() {
    let codec_mu = G711Codec::new(G711Variant::MuLaw);
    let codec_a = G711Codec::new(G711Variant::ALaw);

    // Generate 500ms of audio = 25 frames at 20ms each
    let total_samples = 8000 / 2; // 4000 samples = 500ms
    let input = generate_sine_tone(300.0, 8000, total_samples);

    // Process in 20ms frames
    let frames: Vec<&[i16]> = input.chunks(SAMPLES_PER_FRAME).collect();
    assert!(frames.len() >= 20, "Should have at least 20 frames for 500ms");

    for (i, frame) in frames.iter().enumerate() {
        if frame.len() < SAMPLES_PER_FRAME {
            continue; // Skip incomplete last frame
        }

        // PCMU round-trip
        let mu_encoded = codec_mu
            .compress(frame)
            .unwrap_or_else(|e| panic!("PCMU encode frame {} failed: {}", i, e));
        let mu_decoded = codec_mu
            .expand(&mu_encoded)
            .unwrap_or_else(|e| panic!("PCMU decode frame {} failed: {}", i, e));
        assert_eq!(mu_decoded.len(), SAMPLES_PER_FRAME);

        // PCMA round-trip
        let a_encoded = codec_a
            .compress(frame)
            .unwrap_or_else(|e| panic!("PCMA encode frame {} failed: {}", i, e));
        let a_decoded = codec_a
            .expand(&a_encoded)
            .unwrap_or_else(|e| panic!("PCMA decode frame {} failed: {}", i, e));
        assert_eq!(a_decoded.len(), SAMPLES_PER_FRAME);
    }
}

// =============================================================================
// Test 4: Silence encodes/decodes correctly
// =============================================================================

#[test]
fn test_silence_round_trip() {
    let codec_mu = G711Codec::new(G711Variant::MuLaw);
    let codec_a = G711Codec::new(G711Variant::ALaw);

    // Silence = all zeros
    let silence = vec![0i16; SAMPLES_PER_FRAME];

    // PCMU
    let mu_encoded = codec_mu.compress(&silence).expect("PCMU encode silence");
    let mu_decoded = codec_mu.expand(&mu_encoded).expect("PCMU decode silence");
    // Decoded silence should be very close to zero (G.711 may introduce tiny bias)
    let max_deviation: i16 = mu_decoded.iter().map(|s| s.abs()).max().unwrap_or(0);
    assert!(
        max_deviation < 10,
        "PCMU silence deviation should be < 10, got {}",
        max_deviation
    );

    // PCMA
    let a_encoded = codec_a.compress(&silence).expect("PCMA encode silence");
    let a_decoded = codec_a.expand(&a_encoded).expect("PCMA decode silence");
    let max_deviation: i16 = a_decoded.iter().map(|s| s.abs()).max().unwrap_or(0);
    assert!(
        max_deviation < 10,
        "PCMA silence deviation should be < 10, got {}",
        max_deviation
    );
}

// =============================================================================
// Test 5: PCMU and PCMA produce different encoded output for same input
// =============================================================================

#[test]
fn test_pcmu_pcma_produce_different_encodings() {
    let codec_mu = G711Codec::new(G711Variant::MuLaw);
    let codec_a = G711Codec::new(G711Variant::ALaw);

    let input = generate_sine_tone(500.0, 8000, SAMPLES_PER_FRAME);

    let mu_encoded = codec_mu.compress(&input).expect("PCMU encode");
    let a_encoded = codec_a.compress(&input).expect("PCMA encode");

    // Both should produce same-length output
    assert_eq!(mu_encoded.len(), a_encoded.len());

    // But the encoded bytes should differ (mu-law vs A-law use different companding)
    let differences: usize = mu_encoded
        .iter()
        .zip(a_encoded.iter())
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        differences > 0,
        "PCMU and PCMA should produce different encoded bytes for the same input"
    );
}

// =============================================================================
// Helper: compute Signal-to-Noise Ratio in dB
// =============================================================================

fn compute_snr(original: &[i16], reconstructed: &[i16]) -> f64 {
    assert_eq!(original.len(), reconstructed.len());

    let signal_power: f64 = original.iter().map(|&s| (s as f64) * (s as f64)).sum();
    let noise_power: f64 = original
        .iter()
        .zip(reconstructed.iter())
        .map(|(&o, &r)| {
            let diff = (o as f64) - (r as f64);
            diff * diff
        })
        .sum();

    if noise_power < 1e-10 {
        return 100.0; // Perfect reconstruction
    }

    10.0 * (signal_power / noise_power).log10()
}
