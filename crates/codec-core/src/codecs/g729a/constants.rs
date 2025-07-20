//! Constants and parameters for G.729A codec

// Frame and subframe sizes
pub const FRAME_SIZE: usize = 80;        // 10ms at 8kHz
pub const SUBFRAME_SIZE: usize = 40;     // 5ms at 8kHz
pub const LOOK_AHEAD: usize = 40;        // 5ms look-ahead
pub const SAMPLE_RATE: u32 = 8000;       // 8kHz sampling rate

// Linear prediction parameters
pub const LP_ORDER: usize = 10;          // 10th order LP filter
pub const WINDOW_SIZE: usize = 240;      // Analysis window size
pub const L_TOTAL: usize = 240;          // Total buffer size

// Pitch parameters
pub const PIT_MIN: u16 = 20;             // Minimum pitch delay
pub const PIT_MAX: u16 = 143;            // Maximum pitch delay
pub const L_INTERPOL: usize = 11;        // Interpolation filter length

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
        assert!(PIT_MIN < PIT_MAX);
        assert!(PIT_MIN >= 20); // Minimum physiological pitch period
        assert!(PIT_MAX <= 144); // Maximum for 8kHz sampling
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