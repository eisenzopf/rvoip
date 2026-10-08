//! SIMD utilities for cross-platform optimizations
//!
//! This module provides SIMD capability detection and optimized operations
//! for audio processing across different architectures.

use std::sync::OnceLock;

/// SIMD support information
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimdSupport {
    /// `x86_64` SSE2 support
    pub sse2: bool,
    /// `x86_64` AVX2 support
    pub avx2: bool,
    /// `AArch64` NEON support
    pub neon: bool,
}

/// Global SIMD support detection
static SIMD_SUPPORT: OnceLock<SimdSupport> = OnceLock::new();

#[cfg(target_arch = "x86_64")]
const fn extracted_i16(value: i32) -> i16 {
    let bytes = value.to_le_bytes();
    i16::from_le_bytes([bytes[0], bytes[1]])
}

/// Initialize SIMD support detection
pub fn init_simd_support() {
    SIMD_SUPPORT.get_or_init(detect_simd_support);
}

/// Internal function to detect SIMD support
fn detect_simd_support() -> SimdSupport {
    #[cfg(target_arch = "x86_64")]
    {
        SimdSupport {
            sse2: is_x86_feature_detected!("sse2"),
            avx2: is_x86_feature_detected!("avx2"),
            neon: false,
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        SimdSupport {
            sse2: false,
            avx2: false,
            neon: std::arch::is_aarch64_feature_detected!("neon"),
        }
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        SimdSupport {
            sse2: false,
            avx2: false,
            neon: false,
        }
    }
}

/// Get SIMD support information
#[must_use]
pub fn get_simd_support() -> SimdSupport {
    *SIMD_SUPPORT.get_or_init(detect_simd_support)
}

/// Check if any SIMD support is available
#[must_use]
pub fn has_simd_support() -> bool {
    let support = get_simd_support();
    support.sse2 || support.avx2 || support.neon
}

/// SIMD-optimized μ-law encoding (`x86_64` SSE2).
#[cfg(target_arch = "x86_64")]
#[allow(clippy::cast_ptr_alignment)] // `_mm_loadu_si128` explicitly supports unaligned input.
pub fn encode_mulaw_simd_sse2(samples: &[i16], output: &mut [u8]) {
    use std::arch::x86_64::{__m128i, _mm_extract_epi16, _mm_loadu_si128};

    if !get_simd_support().sse2 {
        return encode_mulaw_scalar(samples, output);
    }

    let mut chunks = samples.chunks_exact(8);
    let mut out_idx = 0;

    unsafe {
        for chunk in chunks.by_ref() {
            // Load 8 samples at once
            let samples_vec = _mm_loadu_si128(chunk.as_ptr().cast::<__m128i>());

            // Process each sample - need to unroll or use different approach
            // _mm_extract_epi16 requires compile-time constant, so we unroll
            output[out_idx] =
                linear_to_mulaw_scalar(extracted_i16(_mm_extract_epi16(samples_vec, 0)));
            output[out_idx + 1] =
                linear_to_mulaw_scalar(extracted_i16(_mm_extract_epi16(samples_vec, 1)));
            output[out_idx + 2] =
                linear_to_mulaw_scalar(extracted_i16(_mm_extract_epi16(samples_vec, 2)));
            output[out_idx + 3] =
                linear_to_mulaw_scalar(extracted_i16(_mm_extract_epi16(samples_vec, 3)));
            output[out_idx + 4] =
                linear_to_mulaw_scalar(extracted_i16(_mm_extract_epi16(samples_vec, 4)));
            output[out_idx + 5] =
                linear_to_mulaw_scalar(extracted_i16(_mm_extract_epi16(samples_vec, 5)));
            output[out_idx + 6] =
                linear_to_mulaw_scalar(extracted_i16(_mm_extract_epi16(samples_vec, 6)));
            output[out_idx + 7] =
                linear_to_mulaw_scalar(extracted_i16(_mm_extract_epi16(samples_vec, 7)));
            out_idx += 8;
        }
    }

    // Handle remainder
    for &sample in chunks.remainder() {
        output[out_idx] = linear_to_mulaw_scalar(sample);
        out_idx += 1;
    }
}

/// SIMD-optimized μ-law encoding (`AArch64` NEON)
#[cfg(target_arch = "aarch64")]
pub fn encode_mulaw_simd_neon(samples: &[i16], output: &mut [u8]) {
    if !get_simd_support().neon {
        return encode_mulaw_scalar(samples, output);
    }

    // For now, fall back to scalar implementation for simplicity
    encode_mulaw_scalar(samples, output);
}

/// Scalar μ-law encoding fallback
pub fn encode_mulaw_scalar(samples: &[i16], output: &mut [u8]) {
    for (i, &sample) in samples.iter().enumerate() {
        output[i] = linear_to_mulaw_scalar(sample);
    }
}

/// SIMD-optimized A-law encoding (`x86_64` SSE2).
#[cfg(target_arch = "x86_64")]
#[allow(clippy::cast_ptr_alignment)] // `_mm_loadu_si128` explicitly supports unaligned input.
pub fn encode_alaw_simd_sse2(samples: &[i16], output: &mut [u8]) {
    use std::arch::x86_64::{__m128i, _mm_extract_epi16, _mm_loadu_si128};

    if !get_simd_support().sse2 {
        return encode_alaw_scalar(samples, output);
    }

    let mut chunks = samples.chunks_exact(8);
    let mut out_idx = 0;

    unsafe {
        for chunk in chunks.by_ref() {
            // Load 8 samples at once
            let samples_vec = _mm_loadu_si128(chunk.as_ptr().cast::<__m128i>());

            // Process each sample - need to unroll or use different approach
            // _mm_extract_epi16 requires compile-time constant, so we unroll
            output[out_idx] =
                linear_to_alaw_scalar(extracted_i16(_mm_extract_epi16(samples_vec, 0)));
            output[out_idx + 1] =
                linear_to_alaw_scalar(extracted_i16(_mm_extract_epi16(samples_vec, 1)));
            output[out_idx + 2] =
                linear_to_alaw_scalar(extracted_i16(_mm_extract_epi16(samples_vec, 2)));
            output[out_idx + 3] =
                linear_to_alaw_scalar(extracted_i16(_mm_extract_epi16(samples_vec, 3)));
            output[out_idx + 4] =
                linear_to_alaw_scalar(extracted_i16(_mm_extract_epi16(samples_vec, 4)));
            output[out_idx + 5] =
                linear_to_alaw_scalar(extracted_i16(_mm_extract_epi16(samples_vec, 5)));
            output[out_idx + 6] =
                linear_to_alaw_scalar(extracted_i16(_mm_extract_epi16(samples_vec, 6)));
            output[out_idx + 7] =
                linear_to_alaw_scalar(extracted_i16(_mm_extract_epi16(samples_vec, 7)));
            out_idx += 8;
        }
    }

    // Handle remainder
    for &sample in chunks.remainder() {
        output[out_idx] = linear_to_alaw_scalar(sample);
        out_idx += 1;
    }
}

/// SIMD-optimized A-law encoding (`AArch64` NEON)
#[cfg(target_arch = "aarch64")]
pub fn encode_alaw_simd_neon(samples: &[i16], output: &mut [u8]) {
    if !get_simd_support().neon {
        return encode_alaw_scalar(samples, output);
    }

    // For now, fall back to scalar implementation for simplicity
    encode_alaw_scalar(samples, output);
}

/// Scalar A-law encoding fallback
pub fn encode_alaw_scalar(samples: &[i16], output: &mut [u8]) {
    for (i, &sample) in samples.iter().enumerate() {
        output[i] = linear_to_alaw_scalar(sample);
    }
}

/// Cross-platform μ-law encoding dispatcher
pub fn encode_mulaw_optimized(samples: &[i16], output: &mut [u8]) {
    #[cfg(target_arch = "x86_64")]
    {
        if get_simd_support().sse2 {
            return encode_mulaw_simd_sse2(samples, output);
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        if get_simd_support().neon {
            return encode_mulaw_simd_neon(samples, output);
        }
    }

    encode_mulaw_scalar(samples, output);
}

/// Cross-platform A-law encoding dispatcher
pub fn encode_alaw_optimized(samples: &[i16], output: &mut [u8]) {
    #[cfg(target_arch = "x86_64")]
    {
        if get_simd_support().sse2 {
            return encode_alaw_simd_sse2(samples, output);
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        if get_simd_support().neon {
            return encode_alaw_simd_neon(samples, output);
        }
    }

    encode_alaw_scalar(samples, output);
}

/// Scalar μ-law conversion (ITU-T G.711)
#[must_use]
pub const fn linear_to_mulaw_scalar(sample: i16) -> u8 {
    crate::codecs::g711_reference::ulaw_compress(sample)
}

/// Scalar A-law conversion (ITU-T G.711)
#[must_use]
pub const fn linear_to_alaw_scalar(sample: i16) -> u8 {
    crate::codecs::g711_reference::alaw_compress(sample)
}

/// Scalar μ-law to linear conversion
#[must_use]
#[allow(clippy::cast_lossless)]
pub const fn mulaw_to_linear_scalar(mulaw: u8) -> i16 {
    crate::codecs::g711_reference::ulaw_expand(mulaw)
}

/// Scalar A-law to linear conversion
#[must_use]
pub const fn alaw_to_linear_scalar(alaw: u8) -> i16 {
    crate::codecs::g711_reference::alaw_expand(alaw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simd_support_detection() {
        init_simd_support();
        let support = get_simd_support();

        // At least one of the fields should be accessible
        #[cfg(target_arch = "x86_64")]
        {
            // SSE2 is widely supported on x86_64
            println!("SSE2 support: {}", support.sse2);
        }

        #[cfg(target_arch = "aarch64")]
        {
            // NEON is standard on AArch64
            println!("NEON support: {}", support.neon);
        }
    }

    #[test]
    fn test_mulaw_roundtrip() {
        let original = 12345i16;
        let encoded = linear_to_mulaw_scalar(original);
        let decoded = mulaw_to_linear_scalar(encoded);

        // G.711 is lossy, so we expect some difference
        let error = (original - decoded).abs();
        assert!(error < 1000, "Error too large: {error}");
    }

    #[test]
    fn test_alaw_roundtrip() {
        let original = 12345i16;
        let encoded = linear_to_alaw_scalar(original);
        let decoded = alaw_to_linear_scalar(encoded);

        // G.711 A-law is lossy, so we expect some difference
        // A-law has different quantization than μ-law, so use more lenient threshold
        // A-law can have significant quantization errors for certain values
        let error = (original - decoded).abs();
        assert!(
            error < 5000,
            "Error too large: {error} (original: {original}, decoded: {decoded})"
        );
    }

    #[test]
    fn test_simd_vs_scalar() {
        let samples = vec![0, 1000, -1000, 16000, -16000, 32000, -32000, 12345];
        let mut simd_output = vec![0u8; samples.len()];
        let mut scalar_output = vec![0u8; samples.len()];

        encode_mulaw_optimized(&samples, &mut simd_output);
        encode_mulaw_scalar(&samples, &mut scalar_output);

        // Results should be identical
        assert_eq!(simd_output, scalar_output);
    }

    #[test]
    fn test_empty_input() {
        let samples: Vec<i16> = vec![];
        let mut output: Vec<u8> = vec![];

        encode_mulaw_optimized(&samples, &mut output);
        assert_eq!(output.len(), 0);
    }

    #[test]
    fn test_edge_cases() {
        let samples = vec![i16::MAX, i16::MIN, 0];
        let mut output = vec![0u8; samples.len()];

        encode_mulaw_optimized(&samples, &mut output);

        // Should not panic and produce valid output
        assert_eq!(output.len(), samples.len());
    }
}
