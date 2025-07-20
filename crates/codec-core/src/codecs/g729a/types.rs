//! ITU-T G.729A Types and Constants
//!
//! This module defines the types and constants used in the G.729A implementation,
//! directly based on the ITU reference code LD8A.H and typedef.h

/// 16-bit signed integer (ITU Word16)
pub type Word16 = i16;

/// 32-bit signed integer (ITU Word32)  
pub type Word32 = i32;

/// Flag type (ITU Flag)
pub type Flag = i32;

/// Unsigned 16-bit integer
pub type UWord16 = u16;

/// Unsigned 32-bit integer
pub type UWord32 = u32;

// G.729A Constants from LD8A.H

/// Total size of speech buffer
pub const L_TOTAL: usize = 240;

/// Window size in LP analysis
pub const L_WINDOW: usize = 240;

/// Lookahead in LP analysis
pub const L_NEXT: usize = 40;

/// Frame size (10ms at 8kHz)
pub const L_FRAME: usize = 80;

/// Subframe size (5ms at 8kHz)
pub const L_SUBFR: usize = 40;

/// Order of LP filter
pub const M: usize = 10;

/// Order of LP filter + 1
pub const MP1: usize = M + 1;

/// Minimum pitch lag
pub const PIT_MIN: usize = 20;

/// Maximum pitch lag  
pub const PIT_MAX: usize = 143;

/// Length of filter for interpolation
pub const L_INTERPOL: usize = 11;

/// Bandwidth factor = 0.75 in Q15
pub const GAMMA1: Word16 = 24576;

/// Size of vector of analysis parameters
pub const PRM_SIZE: usize = 11;

/// BFI + number of speech bits
pub const SERIAL_SIZE: usize = 82;

/// Maximum value of pitch sharpening (0.8 in Q14)
pub const SHARPMAX: Word16 = 13017;

/// Minimum value of pitch sharpening (0.2 in Q14)
pub const SHARPMIN: Word16 = 3277;

/// Maximum pitch gain if taming is needed (Q14)
pub const GPCLIP: Word16 = 15564;

/// Maximum pitch gain if taming is needed (Q9)
pub const GPCLIP2: Word16 = 481;

/// Maximum pitch gain if taming is needed
pub const GP0999: Word16 = 16383;

/// Error threshold taming 16384. * 60000.
pub const L_THRESH_ERR: Word32 = 983040000;

// Additional G.729A specific constants

/// Number of subframes per frame
pub const N_SUBFR: usize = 2;

/// Size of fixed codebook in bits
pub const ACELP_BITS: usize = 17;

/// Size of gain codebook in bits  
pub const GAIN_BITS: usize = 7;

/// Size of LSP codebook in bits (stage 1)
pub const LSP_BITS_1: usize = 7;

/// Size of LSP codebook in bits (stage 2)  
pub const LSP_BITS_2: usize = 6;

/// Total bits per frame
pub const TOTAL_BITS: usize = 80;

// LSP processing constants already defined earlier

// Q-format constants

/// Q0 format (integer)
pub const Q0: i32 = 0;

/// Q12 format (12 fractional bits)
pub const Q12: i32 = 12;

/// Q13 format (13 fractional bits)
pub const Q13: i32 = 13;

/// Q14 format (14 fractional bits)
pub const Q14: i32 = 14;

/// Q15 format (15 fractional bits)
pub const Q15: i32 = 15;

/// Q31 format (31 fractional bits)
pub const Q31: i32 = 31;

// Mathematical constants in fixed point

/// Maximum 16-bit value
pub const MAX_16: Word16 = 0x7fff;

/// Minimum 16-bit value
pub const MIN_16: Word16 = -32768;

/// Maximum 32-bit value
pub const MAX_32: Word32 = 0x7fffffff;

/// Minimum 32-bit value
pub const MIN_32: Word32 = -2147483648;

/// Encoder state structure
#[derive(Debug, Clone)]
pub struct G729AEncoderState {
    /// Old speech buffer
    pub old_speech: [Word16; L_TOTAL],
    /// Old weighted speech buffer  
    pub old_wsp: [Word16; L_FRAME + PIT_MAX],
    /// Old excitation buffer
    pub old_exc: [Word16; L_FRAME + PIT_MAX + L_INTERPOL],
    /// Old LSP values
    pub lsp_old: [Word16; M],
    /// Old quantized LSP values
    pub lsp_old_q: [Word16; M],
    /// Synthesis filter memory
    pub mem_syn: [Word16; M],
    /// Weighting filter memory
    pub mem_w0: [Word16; M],
    /// Weighting filter memory
    pub mem_w: [Word16; M],
    /// Error filter memory
    pub mem_err: [Word16; M + L_SUBFR],
    /// Pitch sharpening factor
    pub sharp: Word16,
}

/// Decoder state structure
#[derive(Debug, Clone)]
pub struct G729ADecoderState {
    /// Old excitation buffer
    pub old_exc: [Word16; L_FRAME + PIT_MAX + L_INTERPOL],
    /// Old LSP values
    pub lsp_old: [Word16; M],
    /// Synthesis filter memory
    pub mem_syn: [Word16; M],
    /// Pitch sharpening factor
    pub sharp: Word16,
    /// Previous integer pitch lag
    pub old_t0: Word16,
    /// Previous pitch gain
    pub gain_pitch: Word16,
    /// Previous code gain
    pub gain_code: Word16,
    /// Bad LSF indicator
    pub bad_lsf: Word16,
}

/// Analysis parameters structure
#[derive(Debug, Clone)]
pub struct AnalysisParams {
    /// LPC coefficients for 2 subframes
    pub a_t: [Word16; MP1 * 2],
    /// LSP quantization indices
    pub lsp_indices: [Word16; 2],
    /// Pitch parameters for 2 subframes
    pub pitch_params: [PitchParams; 2],
    /// Fixed codebook parameters for 2 subframes
    pub fixed_cb_params: [FixedCbParams; 2],
    /// Gain parameters for 2 subframes
    pub gain_params: [GainParams; 2],
}

/// Pitch parameters for one subframe
#[derive(Debug, Clone)]
pub struct PitchParams {
    /// Pitch lag
    pub lag: Word16,
    /// Fractional pitch lag
    pub frac: Word16,
    /// Pitch gain index
    pub gain_index: Word16,
}

/// Fixed codebook parameters for one subframe
#[derive(Debug, Clone)]
pub struct FixedCbParams {
    /// Fixed codebook index
    pub index: Word32,
    /// Fixed codebook gain index
    pub gain_index: Word16,
}

/// Gain parameters for one subframe
#[derive(Debug, Clone)]
pub struct GainParams {
    /// Pitch gain
    pub pitch_gain: Word16,
    /// Code gain
    pub code_gain: Word16,
    /// Combined gain index
    pub gain_index: Word16,
}

impl Default for G729AEncoderState {
    fn default() -> Self {
        Self {
            old_speech: [0; L_TOTAL],
            old_wsp: [0; L_FRAME + PIT_MAX],
            old_exc: [0; L_FRAME + PIT_MAX + L_INTERPOL],
            // Default LSP values (30000, 26000, 21000, 15000, 8000, 0, -8000, -15000, -21000, -26000)
            lsp_old: [30000, 26000, 21000, 15000, 8000, 0, -8000, -15000, -21000, -26000],
            lsp_old_q: [0; M],
            mem_syn: [0; M],
            mem_w0: [0; M],
            mem_w: [0; M],
            mem_err: [0; M + L_SUBFR],
            sharp: SHARPMIN,
        }
    }
}

impl Default for G729ADecoderState {
    fn default() -> Self {
        Self {
            old_exc: [0; L_FRAME + PIT_MAX + L_INTERPOL],
            // Default LSP values
            lsp_old: [30000, 26000, 21000, 15000, 8000, 0, -8000, -15000, -21000, -26000],
            mem_syn: [0; M],
            sharp: SHARPMIN,
            old_t0: 60,
            gain_pitch: 0,
            gain_code: 0,
            bad_lsf: 0,
        }
    }
} 

// Additional constants for LSP processing
/// NC = M/2 for LSP polynomial processing
pub const NC: usize = 5;

/// Grid points for LSP root finding 
pub const GRID_POINTS: usize = 50;

/// LSP search grid for Chebyshev polynomial evaluation (Q15)
pub const LSP_GRID: [Word16; GRID_POINTS + 1] = [
    32760, 32703, 32509, 32187, 31738, 31164, 30466, 29649, 28714, 27666,
    26509, 25248, 23886, 22431, 20887, 19260, 17557, 15786, 13951, 12062,
    10125,  8149,  6140,  4106,  2057,     0, -2057, -4106, -6140, -8149,
   -10125,-12062,-13951,-15786,-17557,-19260,-20887,-22431,-23886,-25248,
   -26509,-27666,-28714,-29649,-30466,-31164,-31738,-32187,-32509,-32703,
   -32760
];

// LSP quantization constants (from ITU G.729A LD8A.H)
/// LSP VQ first stage dimension
pub const NC0: usize = 256;

/// LSP VQ first stage number of bits  
pub const NC0_B: usize = 8;

/// LSP VQ second stage dimension
pub const NC1: usize = 256;

/// LSP VQ second stage number of bits
pub const NC1_B: usize = 8;

/// LSP MA prediction order
pub const MA_NP: usize = 4;

/// LSP VQ modes
pub const MODE: usize = 2;

/// LSP to LSF conversion constant  
pub const LSP_PRED_FAC_1: Word16 = 0x5000; // 0.625 in Q15
/// LSP to LSF conversion constant
pub const LSP_PRED_FAC_2: Word16 = 0x1999; // 0.2 in Q15

/// LSP quantization limits and gaps (ITU-T G.729A constants)
/// Minimum LSP value (Q13: 0.005)
pub const L_LIMIT: Word16 = 40;  
/// Maximum LSP value (Q13: 3.135) 
pub const M_LIMIT: Word16 = 25681;
/// LSP expansion gap 1 (Q13)
pub const GAP1: Word16 = 10;     
/// LSP expansion gap 2 (Q13)
pub const GAP2: Word16 = 5;      
/// LSP expansion gap 3 (Q13)
pub const GAP3: Word16 = 321;    

// Note: MAX_16, MIN_16, MAX_32, MIN_32 constants already defined above 