//! G.711 Audio Codec Reference Implementation
//!
//! This module contains the reference implementation of G.711 μ-law and A-law
//! encoding and decoding algorithms as specified in ITU-T Recommendation G.711.
//!
//! The implementation is based on the official ITU-T reference implementation
//! from the STL (Software Tools Library) and maintains bit-exact compatibility.
//!
//! ## Reference
//!
//! - ITU-T Recommendation G.711: "Pulse code modulation (PCM) of voice frequencies"
//! - ITU-T Software Tools Library (STL) - G.711 module
//!
//! ## Algorithm Details
//!
//! ### A-law Compression
//! - Uses 13 Most Significant Bits (MSBs) from input
//! - Produces 8 Least Significant Bits (LSBs) on output
//! - Applies 1's complement for negative values
//! - Toggles even bits (XOR with 0x55)
//!
//! ### μ-law Compression  
//! - Uses 14 Most Significant Bits (MSBs) from input
//! - Produces 8 Least Significant Bits (LSBs) on output
//! - Adds bias of 33 (0x21) for processing
//! - Applies 1's complement and inversion

/// A-law compression according to ITU-T G.711
///
/// Compresses a 16-bit linear PCM sample to 8-bit A-law encoding.
///
/// # Arguments
///
/// * `sample` - Input linear PCM sample (16-bit signed)
///
/// # Returns
///
/// A-law encoded sample (8-bit)
///
/// # Algorithm
///
/// Based on ITU-T G.711 specification and reference implementation.
pub fn alaw_compress(sample: i16) -> u8 {
    let mut ix = if sample < 0 {
        (((!sample) as u16) >> 4) as i16
    } else {
        sample >> 4
    };
    
    if ix > 15 {
        let mut iexp = 1;
        while ix > 16 + 15 {
            ix >>= 1;
            iexp += 1;
        }
        ix -= 16;
        ix += iexp << 4;
    }
    
    if sample >= 0 {
        ix |= 0x0080;
    }
    
    (ix ^ 0x0055) as u8
}

/// A-law expansion according to ITU-T G.711
///
/// Expands an 8-bit A-law encoded sample to 16-bit linear PCM.
///
/// # Arguments
///
/// * `compressed` - A-law encoded sample (8-bit)
///
/// # Returns
///
/// Linear PCM sample (16-bit signed)
///
/// # Algorithm
///
/// Based on ITU-T G.711 specification and reference implementation.
pub fn alaw_expand(compressed: u8) -> i16 {
    let mut ix = (compressed ^ 0x0055) as i16;
    
    ix &= 0x007F;
    let iexp = ix >> 4;
    let mut mant = ix & 0x000F;
    
    if iexp > 0 {
        mant = mant + 16;
    }
    
    mant = (mant << 4) + 0x0008;
    
    if iexp > 1 {
        mant = mant << (iexp - 1);
    }
    
    if compressed > 127 {
        mant
    } else {
        -mant
    }
}

/// μ-law compression according to ITU-T G.711
///
/// Compresses a 16-bit linear PCM sample to 8-bit μ-law encoding.
///
/// # Arguments
///
/// * `sample` - Input linear PCM sample (16-bit signed)
///
/// # Returns
///
/// μ-law encoded sample (8-bit)
///
/// # Algorithm
///
/// Based on ITU-T G.711 specification and reference implementation.
pub fn ulaw_compress(sample: i16) -> u8 {
    let absno = if sample < 0 {
        (((!sample) as u16) >> 2) as i16 + 33
    } else {
        (sample >> 2) + 33
    };
    
    let absno = if absno > 0x1FFF { 0x1FFF } else { absno };
    
    let mut i = absno >> 6;
    let mut segno = 1;
    while i != 0 {
        segno += 1;
        i >>= 1;
    }
    
    let high_nibble = 0x0008 - segno;
    let low_nibble = 0x000F - ((absno >> segno) & 0x000F);
    let mut result = (high_nibble << 4) | low_nibble;
    
    if sample >= 0 {
        result |= 0x0080;
    }
    
    result as u8
}

/// μ-law expansion according to ITU-T G.711
///
/// Expands an 8-bit μ-law encoded sample to 16-bit linear PCM.
///
/// # Arguments
///
/// * `compressed` - μ-law encoded sample (8-bit)
///
/// # Returns
///
/// Linear PCM sample (16-bit signed)
///
/// # Algorithm
///
/// Based on ITU-T G.711 specification and reference implementation.
pub fn ulaw_expand(compressed: u8) -> i16 {
    let sign = if compressed < 0x0080 { -1 } else { 1 };
    let mantissa = (!compressed) as i16;
    let exponent = (mantissa >> 4) & 0x0007;
    let segment = exponent + 1;
    let mantissa = mantissa & 0x000F;
    
    let step = 4 << segment;
    
    sign * (
        ((0x0080) << exponent) +
        step * mantissa +
        step / 2 -
        4 * 33
    )
}

/// Batch A-law compression
///
/// Compresses a slice of linear PCM samples to A-law encoding.
///
/// # Arguments
///
/// * `samples` - Input linear PCM samples
/// * `output` - Output buffer for A-law encoded samples
///
/// # Panics
///
/// Panics if the input and output slices have different lengths.
pub fn alaw_compress_batch(samples: &[i16], output: &mut [u8]) {
    assert_eq!(samples.len(), output.len(), "Input and output slices must have the same length");
    
    for (i, &sample) in samples.iter().enumerate() {
        output[i] = alaw_compress(sample);
    }
}

/// Batch A-law expansion
///
/// Expands a slice of A-law encoded samples to linear PCM.
///
/// # Arguments
///
/// * `encoded` - A-law encoded samples
/// * `output` - Output buffer for linear PCM samples
///
/// # Panics
///
/// Panics if the input and output slices have different lengths.
pub fn alaw_expand_batch(encoded: &[u8], output: &mut [i16]) {
    assert_eq!(encoded.len(), output.len(), "Input and output slices must have the same length");
    
    for (i, &encoded_sample) in encoded.iter().enumerate() {
        output[i] = alaw_expand(encoded_sample);
    }
}

/// Batch μ-law compression
///
/// Compresses a slice of linear PCM samples to μ-law encoding.
///
/// # Arguments
///
/// * `samples` - Input linear PCM samples
/// * `output` - Output buffer for μ-law encoded samples
///
/// # Panics
///
/// Panics if the input and output slices have different lengths.
pub fn ulaw_compress_batch(samples: &[i16], output: &mut [u8]) {
    assert_eq!(samples.len(), output.len(), "Input and output slices must have the same length");
    
    for (i, &sample) in samples.iter().enumerate() {
        output[i] = ulaw_compress(sample);
    }
}

/// Batch μ-law expansion
///
/// Expands a slice of μ-law encoded samples to linear PCM.
///
/// # Arguments
///
/// * `encoded` - μ-law encoded samples
/// * `output` - Output buffer for linear PCM samples
///
/// # Panics
///
/// Panics if the input and output slices have different lengths.
pub fn ulaw_expand_batch(encoded: &[u8], output: &mut [i16]) {
    assert_eq!(encoded.len(), output.len(), "Input and output slices must have the same length");
    
    for (i, &encoded_sample) in encoded.iter().enumerate() {
        output[i] = ulaw_expand(encoded_sample);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alaw_basic_round_trip() {
        let test_samples = vec![0i16, 100, -100, 1000, -1000, 10000, -10000];
        
        for sample in test_samples {
            let encoded = alaw_compress(sample);
            let decoded = alaw_expand(encoded);
            let error = (decoded - sample).abs();
            
            // A-law should have reasonable quantization error
            assert!(error < 2000, "A-law error too large for {}: {} (error: {})", sample, decoded, error);
        }
    }

    #[test]
    fn test_ulaw_basic_round_trip() {
        let test_samples = vec![0i16, 100, -100, 1000, -1000, 10000, -10000];
        
        for sample in test_samples {
            let encoded = ulaw_compress(sample);
            let decoded = ulaw_expand(encoded);
            let error = (decoded - sample).abs();
            
            // μ-law should have reasonable quantization error
            assert!(error < 2000, "μ-law error too large for {}: {} (error: {})", sample, decoded, error);
        }
    }

    #[test]
    fn test_alaw_batch_processing() {
        let samples = vec![0i16, 100, -100, 1000, -1000];
        let mut encoded = vec![0u8; samples.len()];
        let mut decoded = vec![0i16; samples.len()];
        
        alaw_compress_batch(&samples, &mut encoded);
        alaw_expand_batch(&encoded, &mut decoded);
        
        for (original, recovered) in samples.iter().zip(decoded.iter()) {
            let error = (recovered - original).abs();
            assert!(error < 2000, "A-law batch error too large: {} vs {}", original, recovered);
        }
    }

    #[test]
    fn test_ulaw_batch_processing() {
        let samples = vec![0i16, 100, -100, 1000, -1000];
        let mut encoded = vec![0u8; samples.len()];
        let mut decoded = vec![0i16; samples.len()];
        
        ulaw_compress_batch(&samples, &mut encoded);
        ulaw_expand_batch(&encoded, &mut decoded);
        
        for (original, recovered) in samples.iter().zip(decoded.iter()) {
            let error = (recovered - original).abs();
            assert!(error < 2000, "μ-law batch error too large: {} vs {}", original, recovered);
        }
    }

    #[test]
    fn test_boundary_values() {
        let boundary_samples = vec![-32768i16, -32767, -1, 0, 1, 32766, 32767];
        
        for sample in boundary_samples {
            // Test A-law doesn't panic on boundary values
            let alaw_encoded = alaw_compress(sample);
            let alaw_decoded = alaw_expand(alaw_encoded);
            
            // Test μ-law doesn't panic on boundary values
            let ulaw_encoded = ulaw_compress(sample);
            let ulaw_decoded = ulaw_expand(ulaw_encoded);
            
            // Basic sanity check - decoded values should be in reasonable range
            assert!(alaw_decoded >= -32768 && alaw_decoded <= 32767);
            assert!(ulaw_decoded >= -32768 && ulaw_decoded <= 32767);
        }
    }

    #[test]
    fn test_known_values() {
        // Test cases from ITU-T reference implementation
        assert_eq!(alaw_compress(0), 0xd5);
        assert_eq!(alaw_compress(128), 0xdd);
        assert_eq!(alaw_compress(1024), 0xe5);
        assert_eq!(alaw_compress(-128), 0x52);
        assert_eq!(alaw_compress(-1024), 0x7a);
        
        assert_eq!(ulaw_compress(0), 0xff);
        assert_eq!(ulaw_compress(128), 0xef);
        assert_eq!(ulaw_compress(1024), 0xcd);
        assert_eq!(ulaw_compress(-128), 0x6f);
        assert_eq!(ulaw_compress(-1024), 0x4d);
        
        assert_eq!(alaw_expand(0xd5), 8);
        assert_eq!(alaw_expand(0xdd), 136);
        assert_eq!(alaw_expand(0xe5), 1056);
        assert_eq!(alaw_expand(0x52), -120);
        assert_eq!(alaw_expand(0x7a), -1008);
        
        assert_eq!(ulaw_expand(0xff), 0);
        assert_eq!(ulaw_expand(0xef), 132);
        assert_eq!(ulaw_expand(0xcd), 1052);
        assert_eq!(ulaw_expand(0x6f), -132);
        assert_eq!(ulaw_expand(0x4d), -1052);
    }
} 