//! G.711 Lookup Tables
//!
//! This module contains pre-computed lookup tables for G.711 μ-law and A-law
//! encoding and decoding.
//!
//! ## Performance
//!
//! Using lookup tables provides O(1) conversion time compared to the algorithmic
//! approach, significantly improving performance for real-time applications.
//!
//! ## Memory Usage
//!
//! - μ-law encode table: 65536 bytes (64KB)
//! - μ-law decode table: 512 bytes (256 samples × 2 bytes)
//! - A-law encode table: 65536 bytes (64KB)
//! - A-law decode table: 512 bytes (256 samples × 2 bytes)
//! - Total: ~130KB of lookup tables

use crate::codecs::g711::reference::*;
use std::sync::LazyLock;

/// Pre-computed μ-law encoding table (16-bit linear → 8-bit μ-law)
///
/// This table covers the full 16-bit signed input range (-32768 to 32767).
/// Generated using the ITU-T reference implementation.
static MULAW_ENCODE_TABLE: LazyLock<[u8; 65536]> = LazyLock::new(|| {
    let mut table = [0u8; 65536];
    
    // Generate table for all possible 16-bit input values
    for i in 0..65536 {
        let sample = (i as u16).wrapping_sub(32768) as i16; // Convert to signed -32768..32767
        table[i] = ulaw_compress(sample);
    }
    
    table
});

/// Pre-computed μ-law decoding table (8-bit μ-law → 16-bit linear)
///
/// This table covers all possible 8-bit μ-law encoded values (0 to 255).
static MULAW_DECODE_TABLE: LazyLock<[i16; 256]> = LazyLock::new(|| {
    let mut table = [0i16; 256];
    
    // Generate table for all possible 8-bit μ-law values
    for i in 0..256 {
        table[i] = ulaw_expand(i as u8);
    }
    
    table
});

/// Pre-computed A-law encoding table (16-bit linear → 8-bit A-law)
///
/// This table covers the full 16-bit signed input range (-32768 to 32767).
static ALAW_ENCODE_TABLE: LazyLock<[u8; 65536]> = LazyLock::new(|| {
    let mut table = [0u8; 65536];
    
    // Generate table for all possible 16-bit input values
    for i in 0..65536 {
        let sample = (i as u16).wrapping_sub(32768) as i16; // Convert to signed -32768..32767
        table[i] = alaw_compress(sample);
    }
    
    table
});

/// Pre-computed A-law decoding table (8-bit A-law → 16-bit linear)
///
/// This table covers all possible 8-bit A-law encoded values (0 to 255).
static ALAW_DECODE_TABLE: LazyLock<[i16; 256]> = LazyLock::new(|| {
    let mut table = [0i16; 256];
    
    // Generate table for all possible 8-bit A-law values
    for i in 0..256 {
        table[i] = alaw_expand(i as u8);
    }
    
    table
});

/// Fast μ-law compression using lookup table
///
/// This function provides O(1) μ-law compression using pre-computed lookup tables.
/// It's significantly faster than the algorithmic approach for bulk processing.
///
/// # Arguments
///
/// * `sample` - 16-bit signed linear PCM sample
///
/// # Returns
///
/// 8-bit μ-law encoded value
pub fn mulaw_compress_table(sample: i16) -> u8 {
    let index = (sample as u16).wrapping_add(32768) as usize;
    MULAW_ENCODE_TABLE[index]
}

/// Fast μ-law expansion using lookup table
///
/// This function provides O(1) μ-law expansion using pre-computed lookup tables.
/// It's significantly faster than the algorithmic approach for bulk processing.
///
/// # Arguments
///
/// * `encoded` - 8-bit μ-law encoded value
///
/// # Returns
///
/// 16-bit signed linear PCM sample
pub fn mulaw_expand_table(encoded: u8) -> i16 {
    MULAW_DECODE_TABLE[encoded as usize]
}

/// Fast A-law compression using lookup table
///
/// This function provides O(1) A-law compression using pre-computed lookup tables.
/// It's significantly faster than the algorithmic approach for bulk processing.
///
/// # Arguments
///
/// * `sample` - 16-bit signed linear PCM sample
///
/// # Returns
///
/// 8-bit A-law encoded value
pub fn alaw_compress_table(sample: i16) -> u8 {
    let index = (sample as u16).wrapping_add(32768) as usize;
    ALAW_ENCODE_TABLE[index]
}

/// Fast A-law expansion using lookup table
///
/// This function provides O(1) A-law expansion using pre-computed lookup tables.
/// It's significantly faster than the algorithmic approach for bulk processing.
///
/// # Arguments
///
/// * `encoded` - 8-bit A-law encoded value
///
/// # Returns
///
/// 16-bit signed linear PCM sample
pub fn alaw_expand_table(encoded: u8) -> i16 {
    ALAW_DECODE_TABLE[encoded as usize]
}

/// Batch μ-law compression using lookup tables
///
/// This function provides high-performance batch μ-law compression using
/// pre-computed lookup tables.
///
/// # Arguments
///
/// * `samples` - Input linear PCM samples
/// * `output` - Output buffer for μ-law encoded samples
///
/// # Panics
///
/// Panics if the input and output slices have different lengths.
pub fn mulaw_compress_batch_table(samples: &[i16], output: &mut [u8]) {
    assert_eq!(samples.len(), output.len());
    
    for (i, &sample) in samples.iter().enumerate() {
        output[i] = mulaw_compress_table(sample);
    }
}

/// Batch μ-law expansion using lookup tables
///
/// This function provides high-performance batch μ-law expansion using
/// pre-computed lookup tables.
///
/// # Arguments
///
/// * `encoded` - μ-law encoded samples
/// * `output` - Output buffer for linear PCM samples
///
/// # Panics
///
/// Panics if the input and output slices have different lengths.
pub fn mulaw_expand_batch_table(encoded: &[u8], output: &mut [i16]) {
    assert_eq!(encoded.len(), output.len());
    
    for (i, &encoded_sample) in encoded.iter().enumerate() {
        output[i] = mulaw_expand_table(encoded_sample);
    }
}

/// Batch A-law compression using lookup tables
///
/// This function provides high-performance batch A-law compression using
/// pre-computed lookup tables.
///
/// # Arguments
///
/// * `samples` - Input linear PCM samples
/// * `output` - Output buffer for A-law encoded samples
///
/// # Panics
///
/// Panics if the input and output slices have different lengths.
pub fn alaw_compress_batch_table(samples: &[i16], output: &mut [u8]) {
    assert_eq!(samples.len(), output.len());
    
    for (i, &sample) in samples.iter().enumerate() {
        output[i] = alaw_compress_table(sample);
    }
}

/// Batch A-law expansion using lookup tables
///
/// This function provides high-performance batch A-law expansion using
/// pre-computed lookup tables.
///
/// # Arguments
///
/// * `encoded` - A-law encoded samples
/// * `output` - Output buffer for linear PCM samples
///
/// # Panics
///
/// Panics if the input and output slices have different lengths.
pub fn alaw_expand_batch_table(encoded: &[u8], output: &mut [i16]) {
    assert_eq!(encoded.len(), output.len());
    
    for (i, &encoded_sample) in encoded.iter().enumerate() {
        output[i] = alaw_expand_table(encoded_sample);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_table_initialization() {
        // Test that tables are initialized correctly
        assert_eq!(MULAW_ENCODE_TABLE.len(), 65536);
        assert_eq!(MULAW_DECODE_TABLE.len(), 256);
        assert_eq!(ALAW_ENCODE_TABLE.len(), 65536);
        assert_eq!(ALAW_DECODE_TABLE.len(), 256);
    }

    #[test]
    fn test_mulaw_table_vs_reference() {
        // Test that table lookups match reference implementation
        let test_samples = vec![
            -32768i16, -16384, -8192, -4096, -2048, -1024, -512, -256, -128, -64, -32, -16, -8, -4, -2, -1,
            0, 1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32767
        ];
        
        for sample in test_samples {
            let ref_encoded = ulaw_compress(sample);
            let table_encoded = mulaw_compress_table(sample);
            assert_eq!(ref_encoded, table_encoded, "μ-law encode mismatch for sample {}", sample);
        }
        
        // Test decode table
        for i in 0..256 {
            let ref_decoded = ulaw_expand(i as u8);
            let table_decoded = mulaw_expand_table(i as u8);
            assert_eq!(ref_decoded, table_decoded, "μ-law decode mismatch for encoded value {}", i);
        }
    }

    #[test]
    fn test_alaw_table_vs_reference() {
        // Test that table lookups match reference implementation
        let test_samples = vec![
            -32768i16, -16384, -8192, -4096, -2048, -1024, -512, -256, -128, -64, -32, -16, -8, -4, -2, -1,
            0, 1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32767
        ];
        
        for sample in test_samples {
            let ref_encoded = alaw_compress(sample);
            let table_encoded = alaw_compress_table(sample);
            assert_eq!(ref_encoded, table_encoded, "A-law encode mismatch for sample {}", sample);
        }
        
        // Test decode table
        for i in 0..256 {
            let ref_decoded = alaw_expand(i as u8);
            let table_decoded = alaw_expand_table(i as u8);
            assert_eq!(ref_decoded, table_decoded, "A-law decode mismatch for encoded value {}", i);
        }
    }

    #[test]
    fn test_batch_processing() {
        let samples = vec![0i16, 100, -100, 1000, -1000, 10000, -10000];
        let mut mulaw_encoded = vec![0u8; samples.len()];
        let mut mulaw_decoded = vec![0i16; samples.len()];
        let mut alaw_encoded = vec![0u8; samples.len()];
        let mut alaw_decoded = vec![0i16; samples.len()];
        
        // Test μ-law batch processing
        mulaw_compress_batch_table(&samples, &mut mulaw_encoded);
        mulaw_expand_batch_table(&mulaw_encoded, &mut mulaw_decoded);
        
        for (original, recovered) in samples.iter().zip(mulaw_decoded.iter()) {
            let error = (recovered - original).abs();
            assert!(error < 2000, "μ-law batch table error: {} vs {}", original, recovered);
        }
        
        // Test A-law batch processing
        alaw_compress_batch_table(&samples, &mut alaw_encoded);
        alaw_expand_batch_table(&alaw_encoded, &mut alaw_decoded);
        
        for (original, recovered) in samples.iter().zip(alaw_decoded.iter()) {
            let error = (recovered - original).abs();
            assert!(error < 2000, "A-law batch table error: {} vs {}", original, recovered);
        }
    }

    #[test]
    fn test_table_validation() {
        // Test that our tables produce the same results as reference implementation
        let test_samples = vec![
            -32768i16, -16384, -8192, -4096, -2048, -1024, -512, -256, -128, -64, -32, -16, -8, -4, -2, -1,
            0, 1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32767
        ];
        
        for sample in test_samples {
            // Test μ-law round-trip with tables
            let ref_encoded = ulaw_compress(sample);
            let table_encoded = mulaw_compress_table(sample);
            assert_eq!(ref_encoded, table_encoded);
            
            let ref_decoded = ulaw_expand(table_encoded);
            let table_decoded = mulaw_expand_table(table_encoded);
            assert_eq!(ref_decoded, table_decoded);
            
            // Test A-law round-trip with tables
            let ref_encoded = alaw_compress(sample);
            let table_encoded = alaw_compress_table(sample);
            assert_eq!(ref_encoded, table_encoded);
            
            let ref_decoded = alaw_expand(table_encoded);
            let table_decoded = alaw_expand_table(table_encoded);
            assert_eq!(ref_decoded, table_decoded);
        }
    }

    #[test]
    fn test_performance_batch_vs_individual() {
        use std::time::Instant;
        
        let samples = vec![0i16; 1000];
        let mut encoded = vec![0u8; samples.len()];
        let mut decoded = vec![0i16; samples.len()];
        
        // Test batch processing (should be faster)
        let start = Instant::now();
        mulaw_compress_batch_table(&samples, &mut encoded);
        let batch_time = start.elapsed();
        
        // Test individual processing
        let start = Instant::now();
        for (i, &sample) in samples.iter().enumerate() {
            encoded[i] = mulaw_compress_table(sample);
        }
        let individual_time = start.elapsed();
        
        println!("Batch time: {:?}, Individual time: {:?}", batch_time, individual_time);
        
        // Both should work correctly
        for (i, &sample) in samples.iter().enumerate() {
            assert_eq!(encoded[i], mulaw_compress_table(sample));
        }
    }
} 