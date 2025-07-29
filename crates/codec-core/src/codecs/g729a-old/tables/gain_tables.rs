//! Gain quantization tables for G.729A

use crate::codecs::g729a::types::{Q15, Q14};

/// MA gain prediction coefficients
/// Values are {0.68, 0.58, 0.34, 0.19} in Q13
pub const GAIN_PRED_COEF: [i16; 4] = [5571, 4751, 2785, 1556];

/// First stage gain codebook (3 bits, 8 entries)
/// Format: [gbk1_correction_factor (Q14), gbk1_energy (Q13)]
pub const GBK1: [[i16; 2]; 8] = [
    [1, 1516],      // Entry 0
    [1551, 2425],   // Entry 1
    [1831, 5022],   // Entry 2
    [57, 5404],     // Entry 3
    [1921, 9291],   // Entry 4
    [3242, 9949],   // Entry 5
    [356, 14756],   // Entry 6
    [2678, 27162],  // Entry 7
];

/// Second stage gain codebook (4 bits, 16 entries)
/// Format: [gbk2_correction_factor (Q14), gbk2_energy (Q13)]
pub const GBK2: [[i16; 2]; 16] = [
    [826, 2005],    // Entry 0
    [1994, 0],      // Entry 1
    [5142, 592],    // Entry 2
    [6160, 2395],   // Entry 3
    [8091, 4861],   // Entry 4
    [9120, 525],    // Entry 5
    [10573, 2966],  // Entry 6
    [11569, 1196],  // Entry 7
    [13260, 3256],  // Entry 8
    [14194, 1630],  // Entry 9
    [15132, 4914],  // Entry 10
    [15161, 14276], // Entry 11
    [15434, 237],   // Entry 12
    [16112, 3392],  // Entry 13
    [17299, 1861],  // Entry 14
    [18973, 5935],  // Entry 15
];

/// Mapping table for first stage gain codebook
pub const MAP1: [i16; 8] = [5, 1, 4, 7, 3, 0, 6, 2];

/// Mapping table for second stage gain codebook  
pub const MAP2: [i16; 16] = [4, 6, 0, 2, 12, 14, 8, 10, 15, 11, 9, 13, 7, 3, 1, 5];

/// Inverse mapping table for first stage
pub const IMAP1: [i16; 8] = [5, 1, 7, 4, 2, 0, 6, 3];

/// Inverse mapping table for second stage
pub const IMAP2: [i16; 16] = [2, 14, 3, 13, 0, 15, 1, 12, 6, 10, 7, 9, 4, 11, 5, 8];

/// Threshold values for first stage gain quantization (Q14)
pub const THR1: [i16; 4] = [10808, 12374, 19778, 32567];

/// Threshold values for second stage gain quantization (Q15)
pub const THR2: [i16; 8] = [14087, 16188, 20274, 21321, 23525, 25232, 27873, 30542];

/// Coefficients for gain interpolation
/// Format: [[coef_0_0 (Q10), coef_0_1 (Q14)], [coef_1_0 (Q16), coef_1_1 (Q19)]]
pub const COEF: [[i16; 2]; 2] = [
    [31881, 26416],  // Row 0: Q10, Q14
    [31548, 27816],  // Row 1: Q16, Q19
];

/// Long format coefficients for gain interpolation
/// Format: [[L_coef_0_0 (Q26), L_coef_0_1 (Q30)], [L_coef_1_0 (Q32), L_coef_1_1 (Q35)]]
pub const L_COEF: [[i32; 2]; 2] = [
    [2089405952, 1731217536],  // Row 0: Q26, Q30
    [2067549984, 1822990272],  // Row 1: Q32, Q35
];

/// Number of candidates for first stage (NCAN1)
pub const NCAN1: usize = 4;

/// Number of candidates for second stage (NCAN2)
pub const NCAN2: usize = 8;

/// Convert gain codebook values to appropriate Q formats
pub fn get_gbk1_correction(idx: usize) -> Q14 {
    Q14(GBK1[idx][0])
}

pub fn get_gbk1_energy(idx: usize) -> Q15 {
    // Convert from Q13 to Q15
    Q15((GBK1[idx][1] as i32 * 4) as i16)
}

pub fn get_gbk2_correction(idx: usize) -> Q14 {
    Q14(GBK2[idx][0])
}

pub fn get_gbk2_energy(idx: usize) -> Q15 {
    // Convert from Q13 to Q15
    Q15((GBK2[idx][1] as i32 * 4) as i16)
} 