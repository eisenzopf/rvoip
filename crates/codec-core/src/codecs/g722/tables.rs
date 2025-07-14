//! G.722 Tables and Constants
//!
//! This module contains all the quantization tables and constants
//! extracted from the ITU-T G.722 reference implementation.
//! Updated to match ITU-T G.722 Annex E (Release 3.00, 2014-11) exactly.

/// QMF filter coefficients for both transmission and reception
/// 
/// From ITU-T G.722 Appendix IV reference implementation.
/// These coefficients are used in both the analysis (encoder) and synthesis (decoder) filters.
/// 
/// Original values from reference: `3*2, -11*2, -11*2, 53*2, 12*2, -156*2, ...`
pub const QMF_COEFFS: [i16; 24] = [
    6, -22, -22, 106, 24, -312, 64, 724, -420, -1610, 1902, 7752,
    7752, 1902, -1610, -420, 724, 64, -312, 24, 106, -22, -22, 6
];

/// Frame size constants
pub const DEF_FRAME_SIZE: usize = 160;  // 16kHz, 10ms input frame size
/// Maximum input 16kHz speech frame size
pub const MAX_INPUT_BUFFER: usize = 8190;

/// G.722 mode constants
pub const G722_MODE_1: u8 = 1;  // 64 kbit/s
/// G.722 mode 2 - 56 kbit/s
pub const G722_MODE_2: u8 = 2;
/// G.722 mode 3 - 48 kbit/s  
pub const G722_MODE_3: u8 = 3;

/// ADPCM quantization levels
pub const LOW_BAND_LEVELS: usize = 64;   // 6-bit quantization
/// High-band quantization levels (2-bit quantization)
pub const HIGH_BAND_LEVELS: usize = 4;

// ================ ITU-T REFERENCE TABLES ================
// These tables are taken directly from the ITU-T G.722 reference implementation
// to ensure bit-exact compliance

/// 6-bit quantization table for low-band (qtab6[64])
/// 
/// From ITU-T reference implementation g722_tables.c
pub const QTAB6: [i16; 64] = [
    -136, -136, -136, -136, -24808, -21904, -19008, -16704, 
    -14984, -13512, -12280, -11192, -10232, -9360, -8576, -7856, 
    -7192, -6576, -6000, -5456, -4944, -4464, -4008, -3576, 
    -3168, -2776, -2400, -2032, -1688, -1360, -1040, -728, 
    24808, 21904, 19008, 16704, 14984, 13512, 12280, 11192, 
    10232, 9360, 8576, 7856, 7192, 6576, 6000, 5456, 
    4944, 4464, 4008, 3576, 3168, 2776, 2400, 2032, 
    1688, 1360, 1040, 728, 432, 136, -432, -136
];

/// 5-bit quantization table for low-band (qtab5[32])
/// 
/// From ITU-T reference implementation g722_tables.c
pub const QTAB5: [i16; 32] = [
    -280, -280, -23352, -17560, -14120, -11664, -9752, -8184, 
    -6864, -5712, -4696, -3784, -2960, -2208, -1520, -880, 
    23352, 17560, 14120, 11664, 9752, 8184, 6864, 5712, 
    4696, 3784, 2960, 2208, 1520, 880, 280, -280
];

/// 4-bit quantization table for low-band (qtab4[16])
/// 
/// From ITU-T reference implementation g722_tables.c
pub const QTAB4: [i16; 16] = [
    0, -20456, -12896, -8968, -6288, -4240, -2584, -1200, 
    20456, 12896, 8968, 6288, 4240, 2584, 1200, 0
];

/// 2-bit quantization table for high-band (qtab2[4])
/// 
/// From ITU-T reference implementation g722_tables.c
pub const QTAB2: [i16; 4] = [
    -7408, -1616, 7408, 1616
];

/// 5-bit quantization levels (q5b[15])
/// 
/// From ITU-T reference implementation g722_tables.c
pub const Q5B: [i16; 15] = [
    576, 1200, 1864, 2584, 3376, 4240, 5200, 6288, 
    7520, 8968, 10712, 12896, 15840, 20456, 25600 
];

/// 2-bit quantization level (q2)
/// 
/// From ITU-T reference implementation g722_tables.c
pub const Q2: i16 = 4512;

/// Inverse quantization table for 4-bit (oq4new[16])
/// 
/// From ITU-T reference implementation g722_tables.c
pub const OQ4NEW: [i16; 16] = [
    -14552, -8768, -6832, -5256, -3776, -2512, -1416, -440,
    5256, 6832, 8768, 14552, 440, 1416, 2512, 3776
];

/// Inverse quantization table for 3-bit (oq3new[8])
/// 
/// From ITU-T reference implementation g722_tables.c
pub const OQ3NEW: [i16; 8] = [
    -9624, -5976, -3056, -872, 5976, 9624, 872, 3056
];

/// Mode-dependent inverse quantization table pointers for low-band
/// 
/// From ITU-T reference implementation g722_tables.c
/// invqbl_tab[mode] points to the appropriate table for each mode
pub const INVQBL_TAB_PTRS: [Option<&'static [i16]>; 4] = [
    None,           // mode 0 (unused)
    Some(&QTAB6),   // mode 1: 6-bit quantization
    Some(&QTAB5),   // mode 2: 5-bit quantization  
    Some(&QTAB4),   // mode 3: 4-bit quantization
];

/// Mode-dependent inverse quantization shifts for low-band
/// 
/// From ITU-T reference implementation g722_tables.c
pub const INVQBL_SHIFT: [u8; 4] = [0, 0, 1, 2];

/// Mode-dependent inverse quantization table pointers for high-band
/// 
/// From ITU-T reference implementation g722_tables.c
/// invqbh_tab[mode] points to the appropriate table for each mode
pub const INVQBH_TAB_PTRS: [Option<&'static [i16]>; 4] = [
    None,               // mode 0 (unused)
    Some(&OQ4NEW),      // mode 1: 4-bit quantization
    Some(&OQ3NEW),      // mode 2: 3-bit quantization
    Some(&QTAB2),       // mode 3: 2-bit quantization
];

/// Inverse adaptive law table (ila2[353])
/// 
/// From ITU-T reference implementation g722_tables.c
/// Used by scalel and scaleh functions for scale factor computation
pub const ILA2: [i16; 353] = [
    8, 8, 8, 8, 8, 8, 8, 8, 
    8, 8, 8, 8, 8, 8, 8, 8, 
    8, 8, 8, 12, 12, 12, 12, 12, 
    12, 12, 12, 12, 12, 12, 12, 12, 
    16, 16, 16, 16, 16, 16, 16, 16, 
    16, 16, 16, 20, 20, 20, 20, 20, 
    20, 20, 20, 24, 24, 24, 24, 24, 
    24, 24, 28, 28, 28, 28, 28, 28, 
    32, 32, 32, 32, 32, 32, 36, 36, 
    36, 36, 36, 40, 40, 40, 40, 44, 
    44, 44, 44, 48, 48, 48, 48, 52, 
    52, 52, 56, 56, 56, 56, 60, 60, 
    64, 64, 64, 68, 68, 68, 72, 72, 
    76, 76, 76, 80, 80, 84, 84, 88, 
    88, 92, 92, 96, 96, 100, 100, 104, 
    104, 108, 112, 112, 116, 116, 120, 124, 
    128, 128, 132, 136, 136, 140, 144, 148, 
    152, 152, 156, 160, 164, 168, 172, 176, 
    180, 184, 188, 192, 196, 200, 204, 208, 
    212, 220, 224, 228, 232, 236, 244, 248, 
    256, 260, 264, 272, 276, 284, 288, 296, 
    304, 308, 316, 324, 332, 336, 344, 352, 
    360, 368, 376, 384, 392, 400, 412, 420, 
    428, 440, 448, 456, 468, 476, 488, 500, 
    512, 520, 532, 544, 556, 568, 580, 592, 
    608, 620, 632, 648, 664, 676, 692, 708, 
    724, 740, 756, 772, 788, 804, 824, 840, 
    860, 880, 896, 916, 936, 956, 980, 1000, 
    1024, 1044, 1068, 1092, 1116, 1140, 1164, 1188, 
    1216, 1244, 1268, 1296, 1328, 1356, 1384, 1416, 
    1448, 1480, 1512, 1544, 1576, 1612, 1648, 1684, 
    1720, 1760, 1796, 1836, 1876, 1916, 1960, 2004, 
    2048, 2092, 2136, 2184, 2232, 2280, 2332, 2380, 
    2432, 2488, 2540, 2596, 2656, 2712, 2772, 2832, 
    2896, 2960, 3024, 3088, 3156, 3228, 3296, 3368, 
    3444, 3520, 3596, 3676, 3756, 3836, 3920, 4008, 
    4096, 4184, 4276, 4372, 4464, 4564, 4664, 4764, 
    4868, 4976, 5084, 5196, 5312, 5428, 5548, 5668, 
    5792, 5920, 6048, 6180, 6316, 6456, 6596, 6740, 
    6888, 7040, 7192, 7352, 7512, 7676, 7844, 8016, 
    8192, 8372, 8556, 8744, 8932, 9128, 9328, 9532, 
    9740, 9956, 10172, 10396, 10624, 10856, 11096, 11336, 
    11584, 11840, 12100, 12364, 12632, 12912, 13192, 13484, 
    13776, 14080, 14388, 14704, 15024, 15352, 15688, 16032, 
    16384
];

/// WLI table used by logscl function
///
/// From ITU-T reference implementation g722_tables.c
pub const WLI: [i16; 16] = [
    -60, 3042, 1198, 538, 334, 172, 58, -30, 
    3042, 1198, 538, 334, 172, 58, -30, -60
];

/// WHI table used by logsch function
///
/// From ITU-T reference implementation g722_tables.c
pub const WHI: [i16; 4] = [
    798, -214, 798, -214
];

/// Logarithmic scale factor update table for low-band
/// 
/// From ITU-T reference implementation - derived from logscl function
pub const LOGSCL_TABLE: [i16; 32] = [
    0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3,
    4, 4, 4, 4, 4, 5, 5, 5, 5, 5, 6, 6, 6, 6, 6, 7
];

/// Logarithmic scale factor update table for high-band
/// 
/// From ITU-T reference implementation - derived from logsch function
pub const LOGSCH_TABLE: [i16; 4] = [
    0, 1, 2, 3
];

/// Predictor update constants
pub const PREDICTOR_CONST_1: i16 = 15360;  // 15/16 in Q15
/// Predictor update constant 2 (1/32 in Q15)
pub const PREDICTOR_CONST_2: i16 = 1024;

/// Limit constants
pub const LIMIT_MIN: i16 = -32768;
/// Maximum value for 16-bit signed integer
pub const LIMIT_MAX: i16 = 32767;

/// Helper function to apply saturation limits (equivalent to ITU-T saturate2)
pub fn limit(value: i32) -> i16 {
    if value > LIMIT_MAX as i32 {
        LIMIT_MAX
    } else if value < LIMIT_MIN as i32 {
        LIMIT_MIN
    } else {
        value as i16
    }
}

/// Get inverse quantization table for low-band based on mode
/// 
/// This function implements the ITU-T reference invqbl_tab[mode] behavior
pub fn get_invqbl_table(mode: u8) -> Option<&'static [i16]> {
    if mode < 4 {
        INVQBL_TAB_PTRS[mode as usize]
    } else {
        None
    }
}

/// Get inverse quantization table for high-band based on mode
/// 
/// This function implements the ITU-T reference invqbh_tab[mode] behavior
pub fn get_invqbh_table(mode: u8) -> Option<&'static [i16]> {
    if mode < 4 {
        INVQBH_TAB_PTRS[mode as usize]
    } else {
        None
    }
}

/// Get inverse quantization shift for low-band based on mode
/// 
/// This function implements the ITU-T reference invqbl_shift[mode] behavior
pub fn get_invqbl_shift(mode: u8) -> u8 {
    if mode < 4 {
        INVQBL_SHIFT[mode as usize]
    } else {
        0
    }
}

// ================ DEPRECATED TABLES ================
// These tables are kept for backwards compatibility but should not be used
// for ITU-T compliance

/// Scale factor adaptation table for low-band - DEPRECATED
/// 
/// This table is deprecated in favor of using the ITU-T reference ILA2 table
#[deprecated(note = "Use ILA2 table with ITU-T reference functions instead")]
pub const SCALEL_TABLE: [i16; 19] = [
    32, 35, 39, 42, 47, 51, 56, 62, 68, 74, 
    82, 89, 98, 107, 116, 127, 139, 152, 166
];

/// Scale factor adaptation table for high-band - DEPRECATED
/// 
/// This table is deprecated in favor of using the ITU-T reference ILA2 table
#[deprecated(note = "Use ILA2 table with ITU-T reference functions instead")]
pub const SCALEH_TABLE: [i16; 19] = [
    32, 35, 39, 42, 47, 51, 56, 62, 68, 74,
    82, 89, 98, 107, 116, 127, 139, 152, 166
];

/// Low-band quantization table (6-bit) - DEPRECATED
/// 
/// Use QTAB6 instead for ITU-T compliance
#[deprecated(note = "Use QTAB6 for ITU-T compliance")]
pub const QUANTL_TABLE: [i16; 31] = [
    -124, -324, -564, -844, -1164, -1524, -1924, -2364,
    -2844, -3364, -3924, -4524, -5164, -5844, -6564, -7324,
    -8124, -8964, -9844, -10764, -11724, -12724, -13764, -14844,
    -15964, -17124, -18324, -19564, -20844, -22164, -23524
];

/// High-band quantization table (2-bit) - DEPRECATED
/// 
/// Use QTAB2 instead for ITU-T compliance
#[deprecated(note = "Use QTAB2 for ITU-T compliance")]
pub const QUANTH_TABLE: [i16; 3] = [
    -348, -1188, -3212
];

/// Inverse quantization table for low-band (6-bit) - DEPRECATED
/// 
/// Use QTAB6 with proper indexing instead for ITU-T compliance
#[deprecated(note = "Use QTAB6 with proper indexing for ITU-T compliance")]
pub const INVQAL_TABLE: [i16; 32] = [
    -136, -136, -136, -136, -24808, -21904, -19008, -16704,
    -14984, -13512, -12280, -11192, -10232, -9360, -8576, -7856,
    -7192, -6576, -6000, -5456, -4944, -4464, -4008, -3576,
    -3168, -2776, -2400, -2032, -1688, -1360, -1040, -728
];

/// Inverse quantization table for high-band (2-bit) - DEPRECATED
/// 
/// Use QTAB2 with proper indexing instead for ITU-T compliance
#[deprecated(note = "Use QTAB2 with proper indexing for ITU-T compliance")]
pub const INVQAH_TABLE: [i16; 4] = [
    -168, -440, -1224, -3624
];

// ================ DEPRECATED HELPER FUNCTIONS ================
// These functions are kept for backwards compatibility but should not be used
// for ITU-T compliance

/// Helper function to get quantization level for low-band - DEPRECATED
#[deprecated(note = "Use ITU-T reference functions instead")]
pub fn get_quantl_level(index: usize) -> i16 {
    if index < QUANTL_TABLE.len() {
        QUANTL_TABLE[index]
    } else {
        QUANTL_TABLE[QUANTL_TABLE.len() - 1]
    }
}

/// Helper function to get quantization level for high-band - DEPRECATED
#[deprecated(note = "Use ITU-T reference functions instead")]
pub fn get_quanth_level(index: usize) -> i16 {
    if index < QUANTH_TABLE.len() {
        QUANTH_TABLE[index]
    } else {
        QUANTH_TABLE[QUANTH_TABLE.len() - 1]
    }
}

/// Helper function to get inverse quantization value for low-band - DEPRECATED
#[deprecated(note = "Use ITU-T reference functions instead")]
pub fn get_invqal_value(index: usize) -> i16 {
    if index < INVQAL_TABLE.len() {
        INVQAL_TABLE[index]
    } else {
        INVQAL_TABLE[INVQAL_TABLE.len() - 1]
    }
}

/// Helper function to get inverse quantization value for high-band - DEPRECATED
#[deprecated(note = "Use ITU-T reference functions instead")]
pub fn get_invqah_value(index: usize) -> i16 {
    if index < INVQAH_TABLE.len() {
        INVQAH_TABLE[index]
    } else {
        INVQAH_TABLE[INVQAH_TABLE.len() - 1]
    }
} 