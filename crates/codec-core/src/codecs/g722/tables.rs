//! G.722 Tables and Constants
//!
//! This module contains all the quantization tables and constants
//! extracted from the ITU-T G.722 reference implementation.

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

/// Low-band quantization table (6-bit)
/// 
/// This table is used to quantize the low-band signal to 6 bits.
/// The values are threshold levels for the quantization process.
pub const QUANTL_TABLE: [i16; 31] = [
    -124, -324, -564, -844, -1164, -1524, -1924, -2364,
    -2844, -3364, -3924, -4524, -5164, -5844, -6564, -7324,
    -8124, -8964, -9844, -10764, -11724, -12724, -13764, -14844,
    -15964, -17124, -18324, -19564, -20844, -22164, -23524
];

/// High-band quantization table (2-bit)
/// 
/// This table is used to quantize the high-band signal to 2 bits.
/// Since there are only 4 levels, the table is much smaller.
pub const QUANTH_TABLE: [i16; 3] = [
    -348, -1188, -3212
];

/// Inverse quantization table for low-band (6-bit)
/// 
/// This table is used to reconstruct the quantized low-band signal.
/// Each entry corresponds to a quantization level.
pub const INVQAL_TABLE: [i16; 32] = [
    -136, -136, -136, -136, -24808, -21904, -19008, -16704,
    -14984, -13512, -12280, -11192, -10232, -9360, -8576, -7856,
    -7192, -6576, -6000, -5456, -4944, -4464, -4008, -3576,
    -3168, -2776, -2400, -2032, -1688, -1360, -1040, -728
];

/// Inverse quantization table for high-band (2-bit)
/// 
/// This table is used to reconstruct the quantized high-band signal.
pub const INVQAH_TABLE: [i16; 4] = [
    -168, -440, -1224, -3624
];

/// Scale factor table for low-band
/// 
/// Used in the adaptive scale factor computation for low-band ADPCM.
pub const SCALEL_TABLE: [i16; 19] = [
    32, 35, 39, 42, 47, 51, 56, 62, 68, 74, 
    82, 89, 98, 107, 116, 127, 139, 152, 166
];

/// Scale factor table for high-band
/// 
/// Used in the adaptive scale factor computation for high-band ADPCM.
pub const SCALEH_TABLE: [i16; 19] = [
    32, 35, 39, 42, 47, 51, 56, 62, 68, 74,
    82, 89, 98, 107, 116, 127, 139, 152, 166
];

/// Logarithmic scale factor table for low-band
/// 
/// Used in the adaptive quantizer scale factor update for low-band.
pub const LOGSCL_TABLE: [i16; 32] = [
    0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3,
    4, 4, 4, 4, 4, 5, 5, 5, 5, 5, 6, 6, 6, 6, 6, 7
];

/// Logarithmic scale factor table for high-band
/// 
/// Used in the adaptive quantizer scale factor update for high-band.
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

/// Helper function to apply saturation limits
pub fn limit(value: i32) -> i16 {
    if value > LIMIT_MAX as i32 {
        LIMIT_MAX
    } else if value < LIMIT_MIN as i32 {
        LIMIT_MIN
    } else {
        value as i16
    }
}

/// Helper function to get quantization level for low-band
pub fn get_quantl_level(index: usize) -> i16 {
    if index < QUANTL_TABLE.len() {
        QUANTL_TABLE[index]
    } else {
        QUANTL_TABLE[QUANTL_TABLE.len() - 1]
    }
}

/// Helper function to get quantization level for high-band
pub fn get_quanth_level(index: usize) -> i16 {
    if index < QUANTH_TABLE.len() {
        QUANTH_TABLE[index]
    } else {
        QUANTH_TABLE[QUANTH_TABLE.len() - 1]
    }
}

/// Helper function to get inverse quantization value for low-band
pub fn get_invqal_value(index: usize) -> i16 {
    if index < INVQAL_TABLE.len() {
        INVQAL_TABLE[index]
    } else {
        INVQAL_TABLE[INVQAL_TABLE.len() - 1]
    }
}

/// Helper function to get inverse quantization value for high-band
pub fn get_invqah_value(index: usize) -> i16 {
    if index < INVQAH_TABLE.len() {
        INVQAH_TABLE[index]
    } else {
        INVQAH_TABLE[INVQAH_TABLE.len() - 1]
    }
} 