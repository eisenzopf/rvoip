//! Lookup tables for G.729A codec

pub mod lsp_tables;
pub mod gain_tables;
pub mod window_tables;
pub mod math_tables;

pub use lsp_tables::*;
pub use gain_tables::*;
pub use window_tables::*;
pub use math_tables::*;

// Re-export the specific function needed by fixed_point.rs
pub use math_tables::get_inv_sqrt_value;

//// Taming procedure zones for pitch search
/// Used in test_err function for error detection
pub const TAB_ZONE: [i16; 153] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2,
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
    3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3
];

/// Filter coefficients for post-processing high-pass filter (fc = 100 Hz)
/// Q13 format
pub const B100: [i16; 3] = [7699, -15398, 7699];
pub const A100: [i16; 3] = [8192, 15836, -7667];

/// Filter coefficients for pre-processing high-pass filter (fc = 140 Hz)
/// b[] coefficients are divided by 2, Q12 format
pub const B140: [i16; 3] = [1899, -3798, 1899];
pub const A140: [i16; 3] = [4096, 7807, -3733];

/// Bit allocation for each parameter in the encoded frame
pub const BITS_PER_PARAM: [i16; 11] = [
    8,   // L0+L1: LSP first stage (7+1 bits)
    5,   // L2: LSP second stage
    8,   // P1: Pitch delay (first subframe)
    1,   // P0: Pitch parity
    13,  // C1: Fixed codebook (first subframe)
    4,   // GA1: Gain codebook (first subframe)
    7,   // GB1: Gain codebook stage 2
    5,   // P2: Pitch delay (second subframe)
    13,  // C2: Fixed codebook (second subframe)
    4,   // GA2: Gain codebook (second subframe)
    7    // GB2: Gain codebook stage 2
]; 