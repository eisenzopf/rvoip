//! Lookup table utilities for codec optimizations

/// Pre-computed μ-law decoding table (8-bit μ-law to 16-bit linear).
pub static MULAW_DECODE_TABLE: [i16; 256] = decode_table(true);

/// Pre-computed A-law decoding table (8-bit A-law to 16-bit linear).
pub static ALAW_DECODE_TABLE: [i16; 256] = decode_table(false);

/// Pre-computed μ-law encoding table, indexed by the top 14 bits of the
/// sample (`sample as u16 >> 2`).
///
/// μ-law ignores the two low bits of a 16-bit sample, so 16 KiB covers the
/// whole PCM domain with a single load per sample instead of the segment
/// search in [`crate::codecs::g711::ulaw_compress`].
pub static MULAW_ENCODE_TABLE: [u8; 1 << 14] = mulaw_encode_table();

/// Pre-computed A-law encoding table, indexed by the top 12 bits of the
/// sample (`sample as u16 >> 4`).
///
/// A-law ignores the four low bits of a 16-bit sample, so 4 KiB covers the
/// whole PCM domain.
pub static ALAW_ENCODE_TABLE: [u8; 1 << 12] = alaw_encode_table();

// Each entry encodes the lowest sample of its index's bucket. The reference
// encoder reads the sample only through its sign and `sample >> 2` (μ-law)
// or `sample >> 4` (A-law), or the same shift of `!sample` for negative
// samples, so every sample in a bucket has the same code.
#[allow(clippy::cast_possible_truncation)]
const fn mulaw_encode_table() -> [u8; 1 << 14] {
    let mut table = [0; 1 << 14];
    let mut index = 0usize;
    while index < table.len() {
        let sample = ((index as u16) << 2).cast_signed();
        table[index] = crate::codecs::g711_reference::ulaw_compress(sample);
        index += 1;
    }
    table
}

#[allow(clippy::cast_possible_truncation)]
const fn alaw_encode_table() -> [u8; 1 << 12] {
    let mut table = [0; 1 << 12];
    let mut index = 0usize;
    while index < table.len() {
        let sample = ((index as u16) << 4).cast_signed();
        table[index] = crate::codecs::g711_reference::alaw_compress(sample);
        index += 1;
    }
    table
}

const fn decode_table(mulaw: bool) -> [i16; 256] {
    let mut table = [0; 256];
    let mut index = 0usize;
    while index < table.len() {
        let encoded = index.to_le_bytes()[0];
        table[index] = if mulaw {
            crate::codecs::g711_reference::ulaw_expand(encoded)
        } else {
            crate::codecs::g711_reference::alaw_expand(encoded)
        };
        index += 1;
    }
    table
}

/// Fast μ-law encoding using lookup table
#[must_use]
pub const fn encode_mulaw_table(sample: i16) -> u8 {
    MULAW_ENCODE_TABLE[(sample.cast_unsigned() >> 2) as usize]
}

/// Fast μ-law decoding using lookup table
#[must_use]
pub const fn decode_mulaw_table(encoded: u8) -> i16 {
    MULAW_DECODE_TABLE[encoded as usize]
}

/// Fast A-law encoding using lookup table
#[must_use]
pub const fn encode_alaw_table(sample: i16) -> u8 {
    ALAW_ENCODE_TABLE[(sample.cast_unsigned() >> 4) as usize]
}

/// Fast A-law decoding using lookup table
#[must_use]
pub const fn decode_alaw_table(encoded: u8) -> i16 {
    ALAW_DECODE_TABLE[encoded as usize]
}

/// Batch μ-law encoding using lookup tables
pub fn encode_mulaw_batch(samples: &[i16], output: &mut [u8]) {
    for (i, &sample) in samples.iter().enumerate() {
        output[i] = encode_mulaw_table(sample);
    }
}

/// Batch μ-law decoding using lookup tables
pub fn decode_mulaw_batch(encoded: &[u8], output: &mut [i16]) {
    for (i, &byte) in encoded.iter().enumerate() {
        output[i] = decode_mulaw_table(byte);
    }
}

/// Batch A-law encoding using lookup tables
pub fn encode_alaw_batch(samples: &[i16], output: &mut [u8]) {
    for (i, &sample) in samples.iter().enumerate() {
        output[i] = encode_alaw_table(sample);
    }
}

/// Batch A-law decoding using lookup tables
pub fn decode_alaw_batch(encoded: &[u8], output: &mut [i16]) {
    for (i, &byte) in encoded.iter().enumerate() {
        output[i] = decode_alaw_table(byte);
    }
}

/// Initialize all lookup tables
pub fn init_tables() {
    // Static arrays are already initialized at compile time
    tracing::debug!(
        bytes = get_table_memory_usage(),
        "Codec lookup tables already initialized"
    );
}

/// Get memory usage of lookup tables
#[must_use]
pub const fn get_table_memory_usage() -> usize {
    // Decode: 2 * 256 * 2 bytes = 1 KiB.
    // Encode: μ-law 16 KiB + A-law 4 KiB.
    let mulaw_decode_size = std::mem::size_of::<[i16; 256]>();
    let alaw_decode_size = std::mem::size_of::<[i16; 256]>();
    let mulaw_encode_size = std::mem::size_of::<[u8; 1 << 14]>();
    let alaw_encode_size = std::mem::size_of::<[u8; 1 << 12]>();

    mulaw_decode_size + alaw_decode_size + mulaw_encode_size + alaw_encode_size
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_table_initialization() {
        // Test that static arrays are available
        assert_eq!(MULAW_DECODE_TABLE.len(), 256);
        assert_eq!(ALAW_DECODE_TABLE.len(), 256);

        // Test first and last values are reasonable
        assert_ne!(MULAW_DECODE_TABLE[0], 0);
        assert_eq!(MULAW_DECODE_TABLE[255], 0);
        assert_ne!(ALAW_DECODE_TABLE[0], 0);
        assert_ne!(ALAW_DECODE_TABLE[255], 0);
    }

    #[test]
    fn test_table_vs_scalar() {
        // Test decode tables only (they're small and fast)
        let test_encoded = vec![0, 127, 128, 255];

        for encoded in test_encoded {
            // Test μ-law decode
            let table_result = decode_mulaw_table(encoded);
            let scalar_result = crate::utils::simd::mulaw_to_linear_scalar(encoded);
            assert_eq!(
                table_result, scalar_result,
                "μ-law decode table mismatch for encoded {encoded}"
            );

            // Test A-law decode
            let table_result = decode_alaw_table(encoded);
            let scalar_result = crate::utils::simd::alaw_to_linear_scalar(encoded);
            assert_eq!(
                table_result, scalar_result,
                "A-law decode table mismatch for encoded {encoded}"
            );
        }
    }

    #[test]
    fn test_batch_operations() {
        // Test decode batch operations only (they're fast)
        let encoded = vec![0u8, 127, 128, 255];
        let mut decoded = vec![0i16; encoded.len()];

        // Test μ-law batch decode
        decode_mulaw_batch(&encoded, &mut decoded);

        assert_eq!(decoded, [-32124, 0, 32124, 0]);

        // Test A-law batch decode
        decode_alaw_batch(&encoded, &mut decoded);

        assert_eq!(decoded, [-5504, -848, 5504, 848]);
    }

    #[test]
    fn test_memory_usage() {
        let usage = get_table_memory_usage();

        // Decode: 2 * 256 * 2 bytes; encode: 16 KiB + 4 KiB.
        assert_eq!(usage, 1024 + 16_384 + 4_096);
    }

    /// The encode tables reproduce the reference encoder for every one of
    /// the 65,536 PCM samples, and the decode tables for every wire byte.
    #[test]
    fn tables_match_the_reference_codec_over_entire_domains() {
        use crate::codecs::g711_reference::{
            alaw_compress, alaw_expand, ulaw_compress, ulaw_expand,
        };
        for bits in 0..=u16::MAX {
            let sample = bits.cast_signed();
            assert_eq!(
                encode_mulaw_table(sample),
                ulaw_compress(sample),
                "{sample}"
            );
            assert_eq!(encode_alaw_table(sample), alaw_compress(sample), "{sample}");
        }
        for byte in 0..=u8::MAX {
            assert_eq!(decode_mulaw_table(byte), ulaw_expand(byte), "{byte}");
            assert_eq!(decode_alaw_table(byte), alaw_expand(byte), "{byte}");
        }
    }

    #[test]
    fn test_edge_cases() {
        // Test boundary values for decode operations only
        let edge_cases = vec![0u8, 127, 128, 255];

        for encoded in edge_cases {
            // Decoders return `i16`; the value range is guaranteed by
            // the type. We only need to assert they don't panic.
            let _ = decode_mulaw_table(encoded);
            let _ = decode_alaw_table(encoded);
        }
    }
}
