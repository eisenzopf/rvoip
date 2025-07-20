//! Mathematical lookup tables for G.729A

use crate::codecs::g729a::types::Q15;

/// Table of cos(x) in Q15 format
/// Used for LSP to LP conversion
pub const COS_TABLE: [i16; 65] = [
    32767, 32729, 32610, 32413, 32138, 31786, 31357, 30853,
    30274, 29622, 28899, 28106, 27246, 26320, 25330, 24279,
    23170, 22006, 20788, 19520, 18205, 16846, 15447, 14010,
    12540, 11039, 9512, 7962, 6393, 4808, 3212, 1608,
    0, -1608, -3212, -4808, -6393, -7962, -9512, -11039,
    -12540, -14010, -15447, -16846, -18205, -19520, -20788, -22006,
    -23170, -24279, -25330, -26320, -27246, -28106, -28899, -29622,
    -30274, -30853, -31357, -31786, -32138, -32413, -32610, -32729,
    -32767
];

/// Slope values in Q12 for computing acos(x)
/// Used in LSP conversion
pub const ACOS_SLOPE: [i16; 64] = [
    -26887, -8812, -5323, -3813, -2979, -2444, -2081, -1811,
    -1608, -1450, -1322, -1219, -1132, -1059, -998, -946,
    -901, -861, -827, -797, -772, -750, -730, -713,
    -699, -687, -677, -668, -662, -657, -654, -652,
    -652, -654, -657, -662, -668, -677, -687, -699,
    -713, -730, -750, -772, -797, -827, -861, -901,
    -946, -998, -1059, -1132, -1219, -1322, -1450, -1608,
    -1811, -2081, -2444, -2979, -3813, -5323, -8812, -26887
];

/// Power of 2 table for Pow2() function
/// 33 values in Q15 format
pub const POW2_TABLE: [i16; 33] = [
    16384, 16743, 17109, 17484, 17867, 18258, 18658, 19066, 19484, 19911,
    20347, 20792, 21247, 21713, 22188, 22674, 23170, 23678, 24196, 24726,
    25268, 25821, 26386, 26964, 27554, 28158, 28774, 29405, 30048, 30706,
    31379, 32066, 32767
];

/// Log2 table for Log2() function
/// 33 values in Q15 format
pub const LOG2_TABLE: [i16; 33] = [
    0, 1455, 2866, 4236, 5568, 6863, 8124, 9352, 10549, 11716,
    12855, 13967, 15054, 16117, 17156, 18172, 19167, 20142, 21097, 22033,
    22951, 23852, 24735, 25603, 26455, 27291, 28113, 28922, 29716, 30497,
    31266, 32023, 32767
];

/// Inverse square root table for Inv_sqrt() function
/// 49 values in Q15 format
pub const INV_SQRT_TABLE: [i16; 49] = [
    32767, 31790, 30894, 30070, 29309, 28602, 27945, 27330, 26755, 26214,
    25705, 25225, 24770, 24339, 23930, 23541, 23170, 22817, 22479, 22155,
    21845, 21548, 21263, 20988, 20724, 20470, 20225, 19988, 19760, 19539,
    19326, 19119, 18919, 18725, 18536, 18354, 18176, 18004, 17837, 17674,
    17515, 17361, 17211, 17064, 16921, 16782, 16646, 16514, 16384
];

/// Grid points for Chebyshev polynomial evaluation
/// Used in LSP root finding
pub const GRID_POINTS: [i16; 60] = [
    32767, 32729, 32610, 32413, 32138, 31786, 31357, 30853,
    30274, 29622, 28899, 28106, 27246, 26320, 25330, 24279,
    23170, 22006, 20788, 19520, 18205, 16846, 15447, 14010,
    12540, 11039, 9512, 7962, 6393, 4808, 3212, 1608,
    0, -1608, -3212, -4808, -6393, -7962, -9512, -11039,
    -12540, -14010, -15447, -16846, -18205, -19520, -20788, -22006,
    -23170, -24279, -25330, -26320, -27246, -28106, -28899, -29622,
    -30274, -30853, -31357, -31786
];

/// Get cosine value from table
pub fn get_cos(index: usize) -> Q15 {
    if index < COS_TABLE.len() {
        Q15(COS_TABLE[index])
    } else {
        Q15(0)
    }
}

/// Get inverse square root value from table
pub fn get_inv_sqrt(index: usize) -> Q15 {
    if index < INV_SQRT_TABLE.len() {
        Q15(INV_SQRT_TABLE[index])
    } else {
        Q15(INV_SQRT_TABLE[INV_SQRT_TABLE.len() - 1])
    }
}

/// Get inverse square root value from table with interpolation
/// The ITU-T G.729A uses a 49-entry table, not 256
/// Index should be in range [0, 48] for direct lookup
pub fn get_inv_sqrt_value(index: usize) -> i16 {
    if index < INV_SQRT_TABLE.len() {
        INV_SQRT_TABLE[index]
    } else {
        // For indices beyond table, use last value
        INV_SQRT_TABLE[INV_SQRT_TABLE.len() - 1]
    }
}

/// Extended inverse square root with interpolation
/// Maps a wider range to the 49-entry table
pub fn get_inv_sqrt_extended(norm_val: usize) -> i16 {
    // Map normalized value (0-255) to table index (0-48)
    let table_idx = (norm_val * 48) / 255;
    
    if table_idx >= 48 {
        return INV_SQRT_TABLE[48];
    }
    
    // Linear interpolation between table entries
    let idx1 = table_idx;
    let idx2 = (table_idx + 1).min(48);
    let frac = ((norm_val * 48) % 255) * 256 / 255;
    
    let val1 = INV_SQRT_TABLE[idx1] as i32;
    let val2 = INV_SQRT_TABLE[idx2] as i32;
    
    ((val1 * (256 - frac as i32) + val2 * (frac as i32)) >> 8) as i16
} 