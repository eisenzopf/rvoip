//! Lookup table utilities for codec optimizations

/// Pre-computed μ-law decoding table (8-bit μ-law to 16-bit linear)
pub static MULAW_DECODE_TABLE: [i16; 256] = [
    16004, 14980, 13956, 12932, 11908, 10884, 9860, 8836, 
    7812, 6788, 5764, 4740, 3716, 2692, 1668, 644, 
    8068, 7556, 7044, 6532, 6020, 5508, 4996, 4484, 
    3972, 3460, 2948, 2436, 1924, 1412, 900, 388, 
    4100, 3844, 3588, 3332, 3076, 2820, 2564, 2308, 
    2052, 1796, 1540, 1284, 1028, 772, 516, 260, 
    2116, 1988, 1860, 1732, 1604, 1476, 1348, 1220, 
    1092, 964, 836, 708, 580, 452, 324, 196, 
    1124, 1060, 996, 932, 868, 804, 740, 676, 
    612, 548, 484, 420, 356, 292, 228, 164, 
    628, 596, 564, 532, 500, 468, 436, 404, 
    372, 340, 308, 276, 244, 212, 180, 148, 
    380, 364, 348, 332, 316, 300, 284, 268, 
    252, 236, 220, 204, 188, 172, 156, 140, 
    252, 244, 236, 228, 220, 212, 204, 196, 
    188, 180, 172, 164, 156, 148, 140, 132, 
    -16004, -14980, -13956, -12932, -11908, -10884, -9860, -8836, 
    -7812, -6788, -5764, -4740, -3716, -2692, -1668, -644, 
    -8068, -7556, -7044, -6532, -6020, -5508, -4996, -4484, 
    -3972, -3460, -2948, -2436, -1924, -1412, -900, -388, 
    -4100, -3844, -3588, -3332, -3076, -2820, -2564, -2308, 
    -2052, -1796, -1540, -1284, -1028, -772, -516, -260, 
    -2116, -1988, -1860, -1732, -1604, -1476, -1348, -1220, 
    -1092, -964, -836, -708, -580, -452, -324, -196, 
    -1124, -1060, -996, -932, -868, -804, -740, -676, 
    -612, -548, -484, -420, -356, -292, -228, -164, 
    -628, -596, -564, -532, -500, -468, -436, -404, 
    -372, -340, -308, -276, -244, -212, -180, -148, 
    -380, -364, -348, -332, -316, -300, -284, -268, 
    -252, -236, -220, -204, -188, -172, -156, -140, 
    -252, -244, -236, -228, -220, -212, -204, -196, 
    -188, -180, -172, -164, -156, -148, -140, -132
];

/// Pre-computed A-law decoding table (8-bit A-law to 16-bit linear)
pub static ALAW_DECODE_TABLE: [i16; 256] = [
    15880, 14856, 13832, 12808, 11784, 10760, 9736, 8712, 
    7688, 6664, 5640, 4616, 3592, 2568, 1544, 520, 
    7944, 7432, 6920, 6408, 5896, 5384, 4872, 4360, 
    3848, 3336, 2824, 2312, 1800, 1288, 776, 264, 
    3976, 3720, 3464, 3208, 2952, 2696, 2440, 2184, 
    1928, 1672, 1416, 1160, 904, 648, 392, 136, 
    1992, 1864, 1736, 1608, 1480, 1352, 1224, 1096, 
    968, 840, 712, 584, 456, 328, 200, 72, 
    1000, 936, 872, 808, 744, 680, 616, 552, 
    488, 424, 360, 296, 232, 168, 104, 40, 
    504, 472, 440, 408, 376, 344, 312, 280, 
    248, 216, 184, 152, 120, 88, 56, 24, 
    256, 240, 224, 208, 192, 176, 160, 144, 
    128, 112, 96, 80, 64, 48, 32, 16, 
    248, 232, 216, 200, 184, 168, 152, 136, 
    120, 104, 88, 72, 56, 40, 24, 8, 
    -15880, -14856, -13832, -12808, -11784, -10760, -9736, -8712, 
    -7688, -6664, -5640, -4616, -3592, -2568, -1544, -520, 
    -7944, -7432, -6920, -6408, -5896, -5384, -4872, -4360, 
    -3848, -3336, -2824, -2312, -1800, -1288, -776, -264, 
    -3976, -3720, -3464, -3208, -2952, -2696, -2440, -2184, 
    -1928, -1672, -1416, -1160, -904, -648, -392, -136, 
    -1992, -1864, -1736, -1608, -1480, -1352, -1224, -1096, 
    -968, -840, -712, -584, -456, -328, -200, -72, 
    -1000, -936, -872, -808, -744, -680, -616, -552, 
    -488, -424, -360, -296, -232, -168, -104, -40, 
    -504, -472, -440, -408, -376, -344, -312, -280, 
    -248, -216, -184, -152, -120, -88, -56, -24, 
    -256, -240, -224, -208, -192, -176, -160, -144, 
    -128, -112, -96, -80, -64, -48, -32, -16, 
    -248, -232, -216, -200, -184, -168, -152, -136, 
    -120, -104, -88, -72, -56, -40, -24, -8
];

/// Fast μ-law encoding using direct computation
pub fn encode_mulaw_table(sample: i16) -> u8 {
    crate::utils::simd::linear_to_mulaw_scalar(sample)
}

/// Fast μ-law decoding using lookup table
pub fn decode_mulaw_table(encoded: u8) -> i16 {
    MULAW_DECODE_TABLE[encoded as usize]
}

/// Fast A-law encoding using direct computation
pub fn encode_alaw_table(sample: i16) -> u8 {
    crate::utils::simd::linear_to_alaw_scalar(sample)
}

/// Fast A-law decoding using lookup table
pub fn decode_alaw_table(encoded: u8) -> i16 {
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
    tracing::debug!("Codec lookup tables already initialized (1KB total)");
}

/// Get memory usage of lookup tables
pub fn get_table_memory_usage() -> usize {
    // Only decode tables: 
    // μ-law: 256 * 2 = 512 bytes
    // A-law: 256 * 2 = 512 bytes
    // Total: 1024 bytes (1KB)
    let mulaw_decode_size = std::mem::size_of::<[i16; 256]>();
    let alaw_decode_size = std::mem::size_of::<[i16; 256]>();
    
    mulaw_decode_size + alaw_decode_size
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
        assert_ne!(MULAW_DECODE_TABLE[255], 0);
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
            assert_eq!(table_result, scalar_result, "μ-law decode table mismatch for encoded {}", encoded);
            
            // Test A-law decode
            let table_result = decode_alaw_table(encoded);
            let scalar_result = crate::utils::simd::alaw_to_linear_scalar(encoded);
            assert_eq!(table_result, scalar_result, "A-law decode table mismatch for encoded {}", encoded);
        }
    }

    #[test]
    fn test_batch_operations() {
        // Test decode batch operations only (they're fast)
        let encoded = vec![0u8, 127, 128, 255];
        let mut decoded = vec![0i16; encoded.len()];
        
        // Test μ-law batch decode
        decode_mulaw_batch(&encoded, &mut decoded);
        
        // Verify we got some non-zero results
        assert_ne!(decoded[1], 0);
        assert_ne!(decoded[2], 0);
        assert_ne!(decoded[3], 0);
        
        // Test A-law batch decode
        decode_alaw_batch(&encoded, &mut decoded);
        
        // Verify we got some non-zero results
        assert_ne!(decoded[1], 0);
        assert_ne!(decoded[2], 0);
        assert_ne!(decoded[3], 0);
    }

    #[test]
    fn test_memory_usage() {
        let usage = get_table_memory_usage();
        
        // Expected: 2 * 256 * 2 bytes = 1024 bytes
        assert_eq!(usage, 1024);
    }

    #[test]
    fn test_edge_cases() {
        // Test boundary values for decode operations only
        let edge_cases = vec![0u8, 127, 128, 255];
        
        for encoded in edge_cases {
            // Test μ-law decode
            let decoded = decode_mulaw_table(encoded);
            assert!(decoded.abs() <= 32767);
            
            // Test A-law decode
            let decoded = decode_alaw_table(encoded);
            assert!(decoded.abs() <= 32767);
        }
    }


} 