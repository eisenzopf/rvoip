//! Constants and parameters for G.729A codec

// Frame parameters
pub const SAMPLE_RATE: u32 = 8000;
pub const FRAME_SIZE: usize = 80;       // 10ms at 8kHz
pub const SUBFRAME_SIZE: usize = 40;    // 5ms subframes
pub const LOOK_AHEAD: usize = 40;       // Look-ahead for pitch analysis

// Linear prediction parameters
pub const LP_ORDER: usize = 10;         // LP analysis order
pub const LSP_ORDER: usize = 10;        // LSP order (same as LP)
pub const WINDOW_SIZE: usize = 240;     // Analysis window (30ms)
pub const L_TOTAL: usize = 240;         // Total analysis buffer size

// ITU-T G.729A Initial LSP values in Q15 format
// These are critical for proper codec initialization
// Using MEAN_LSP values converted from Q13 to Q15 (multiply by 4)
pub const INITIAL_LSP_Q15: [i16; 10] = [
    3016,  // 754 * 4 = 0.0919 in Q15
    5376,  // 1344 * 4 = 0.1640 in Q15
    9240,  // 2310 * 4 = 0.2818 in Q15
    13684, // 3421 * 4 = 0.4175 in Q15
    17728, // 4432 * 4 = 0.5408 in Q15
    21908, // 5477 * 4 = 0.6682 in Q15
    25812, // 6453 * 4 = 0.7873 in Q15
    29264, // 7316 * 4 = 0.8923 in Q15
    31576, // 7894 * 4 = 0.9633 in Q15
    32508, // 8127 * 4 = 0.9915 in Q15
];

// Pitch parameters
pub const PITCH_MIN: usize = 20;        // 2.5ms (400 Hz)
pub const PITCH_MAX: usize = 143;       // 17.875ms (55.8 Hz)
pub const PITCH_FRAC: usize = 3;        // Fractional pitch resolution

// Compatibility aliases for old names
pub const PIT_MIN: u16 = PITCH_MIN as u16;
pub const PIT_MAX: u16 = PITCH_MAX as u16;

// Fixed-point constants
pub const Q15_ONE: i16 = 32767;          // 1.0 in Q15 format
pub const Q15_HALF: i16 = 16384;         // 0.5 in Q15 format
pub const Q15_GAMMA: i16 = 24576;        // 0.75 in Q15 format (gamma for G.729A)

// Codebook parameters
pub const NUM_PULSES: usize = 4;         // 4 pulses in algebraic codebook
pub const TRACK_SIZE: usize = 8;         // Positions per track
pub const GRID_POINTS: usize = 50;       // LSP search grid points (reduced for G.729A)

// Quantization parameters
pub const LSP_CODEBOOK_BITS_1: usize = 7;  // First stage LSP quantization
pub const LSP_CODEBOOK_BITS_2: usize = 5;  // Second stage LSP quantization

// Bitstream parameters
pub const ENCODED_FRAME_SIZE: usize = 10;   // 80 bits = 10 bytes
pub const SERIAL_SIZE: usize = 82;          // Including sync bits

// Filter coefficients
pub const HP_FILTER_COEFF: i16 = 30147;     // 0.92 in Q15 format

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_sizes() {
        assert_eq!(FRAME_SIZE, SAMPLE_RATE as usize / 100); // 10ms frame
        assert_eq!(SUBFRAME_SIZE, FRAME_SIZE / 2);
        assert_eq!(LOOK_AHEAD, SUBFRAME_SIZE);
    }

    #[test]
    fn test_pitch_range() {
        assert!(PITCH_MIN < PITCH_MAX);
        assert!(PITCH_MIN >= 20); // Minimum physiological pitch period
        assert!(PITCH_MAX <= 144); // Maximum for 8kHz sampling
    }

    #[test]
    fn test_q15_constants() {
        assert_eq!(Q15_ONE, i16::MAX);
        assert_eq!(Q15_HALF, 16384); // Exact value
        assert_eq!(Q15_GAMMA, 24576); // Exact value for 0.75
    }

    #[test]
    fn test_encoded_size() {
        // G.729A uses 80 bits per 10ms frame = 8kbps
        let bits_per_frame = ENCODED_FRAME_SIZE * 8;
        let frames_per_second = 1000 / 10; // 10ms frames
        let bitrate = bits_per_frame * frames_per_second;
        assert_eq!(bitrate, 8000); // 8kbps
    }
} 