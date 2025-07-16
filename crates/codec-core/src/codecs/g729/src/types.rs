//! G.729 Types and Constants
//!
//! This module defines the core types and constants used throughout the G.729 implementation,
//! based on the ITU-T G.729 reference implementation.

/// 16-bit word type (equivalent to ITU Word16)
pub type Word16 = i16;

/// 32-bit word type (equivalent to ITU Word32)  
pub type Word32 = i32;

/// Flag type (equivalent to ITU Flag)
pub type Flag = i32;

/// Frame size in samples (10ms at 8kHz)
pub const L_FRAME: usize = 80;

/// Subframe size in samples (5ms at 8kHz)
pub const L_SUBFR: usize = 40;

/// Total speech buffer size
pub const L_TOTAL: usize = 240;

/// LPC analysis window size
pub const L_WINDOW: usize = 240;

/// Lookahead samples for LP analysis
pub const L_NEXT: usize = 40;

/// LPC filter order
pub const M: usize = 10;

/// LPC filter order + 1
pub const MP1: usize = M + 1;

/// LPC filter order - 1
pub const MM1: usize = M - 1;

/// Minimum pitch lag
pub const PIT_MIN: usize = 20;

/// Maximum pitch lag
pub const PIT_MAX: usize = 143;

/// Length of interpolation filter
pub const L_INTERPOL: usize = 11;

/// Size of analysis parameter vector
pub const PRM_SIZE: usize = 11;

/// Bitstream size per frame (80 bits + 2 overhead)
pub const SERIAL_SIZE: usize = 82;

/// Subframe size + 1
pub const L_SUBFRP1: usize = L_SUBFR + 1;

/// Maximum pitch sharpening value (0.8 in Q14)
pub const SHARPMAX: Word16 = 13017;

/// Minimum pitch sharpening value (0.2 in Q14)
pub const SHARPMIN: Word16 = 3277;

/// Maximum pitch gain if taming needed (Q14)
pub const GPCLIP: Word16 = 15564;

/// Maximum pitch gain if taming needed (Q9)
pub const GPCLIP2: Word16 = 481;

/// Maximum pitch gain if taming needed
pub const GP0999: Word16 = 16383;

/// Error threshold for taming (16384 * 60000)
pub const L_THRESH_ERR: Word32 = 983040000;

/// G.729 codec configuration
#[derive(Debug, Clone)]
pub struct G729Config {
    /// Sample rate (must be 8000 Hz)
    pub sample_rate: u32,
    /// Number of channels (must be 1)
    pub channels: u8,
    /// Enable Voice Activity Detection
    pub vad_enabled: bool,
    /// Enable Comfort Noise Generation
    pub cng_enabled: bool,
    /// Use reduced complexity mode (Annex A)
    pub reduced_complexity: bool,
}

impl Default for G729Config {
    fn default() -> Self {
        Self {
            sample_rate: 8000,
            channels: 1,
            vad_enabled: false,
            cng_enabled: false,
            reduced_complexity: true, // Use Annex A by default
        }
    }
}

/// G.729 encoder state
#[derive(Debug, Clone)]
pub struct G729EncoderState {
    /// Old speech buffer
    pub old_speech: [Word16; L_TOTAL],
    /// Speech pointer offset
    pub speech_offset: usize,
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
    /// Weighted synthesis filter memory
    pub mem_w0: [Word16; M],
    /// Weighted filter memory
    pub mem_w: [Word16; M],
    /// Error filter memory
    pub mem_err: [Word16; M + L_SUBFR],
    /// Pitch sharpening factor
    pub sharp: Word16,
    /// Excitation error for taming
    pub l_exc_err: [Word32; 4],
}

impl Default for G729EncoderState {
    fn default() -> Self {
        Self {
            old_speech: [0; L_TOTAL],
            speech_offset: L_TOTAL - L_FRAME - L_NEXT,
            old_wsp: [0; L_FRAME + PIT_MAX],
            old_exc: [0; L_FRAME + PIT_MAX + L_INTERPOL],
            // Default LSP values from ITU reference
            lsp_old: [30000, 26000, 21000, 15000, 8000, 0, -8000, -15000, -21000, -26000],
            lsp_old_q: [0; M],
            mem_syn: [0; M],
            mem_w0: [0; M],
            mem_w: [0; M],
            mem_err: [0; M + L_SUBFR],
            sharp: SHARPMIN,
            l_exc_err: [0x00004000; 4], // Q14 format
        }
    }
}

/// G.729 decoder state
#[derive(Debug, Clone)]
pub struct G729DecoderState {
    /// Old excitation buffer
    pub old_exc: [Word16; L_FRAME + PIT_MAX + L_INTERPOL],
    /// Old LSP values
    pub lsp_old: [Word16; M],
    /// Synthesis filter memory
    pub mem_syn: [Word16; M],
    /// Pitch sharpening from previous frame
    pub sharp: Word16,
    /// Integer delay from previous frame
    pub old_t0: Word16,
    /// Code gain
    pub gain_code: Word16,
    /// Pitch gain
    pub gain_pitch: Word16,
}

impl Default for G729DecoderState {
    fn default() -> Self {
        Self {
            old_exc: [0; L_FRAME + PIT_MAX + L_INTERPOL],
            // Default LSP values from ITU reference
            lsp_old: [30000, 26000, 21000, 15000, 8000, 0, -8000, -15000, -21000, -26000],
            mem_syn: [0; M],
            sharp: SHARPMIN,
            old_t0: 60,
            gain_code: 0,
            gain_pitch: 0,
        }
    }
}

/// Analysis parameters structure
#[derive(Debug, Clone)]
pub struct AnalysisParams {
    /// LPC coefficients
    pub lpc_coeffs: [Word16; MP1],
    /// LSP parameters
    pub lsp_params: [Word16; M],
    /// Pitch parameters for each subframe
    pub pitch_params: [PitchParams; 2],
    /// Codebook parameters for each subframe
    pub codebook_params: [CodebookParams; 2],
}

/// Pitch parameters for one subframe
#[derive(Debug, Clone)]
pub struct PitchParams {
    /// Pitch lag
    pub lag: Word16,
    /// Pitch gain index
    pub gain_index: Word16,
    /// Fractional pitch lag
    pub frac: Word16,
}

/// Codebook parameters for one subframe
#[derive(Debug, Clone)]
pub struct CodebookParams {
    /// Pulse positions
    pub positions: [Word16; 4],
    /// Pulse signs
    pub signs: Word16,
    /// Gain indices
    pub gain_indices: [Word16; 2],
}

/// Synthesis parameters for decoder
#[derive(Debug, Clone)]
pub struct SynthesisParams {
    /// Bad frame indicator
    pub bfi: Flag,
    /// Analysis parameters
    pub analysis: AnalysisParams,
}

/// Q-format constants for fixed-point arithmetic
pub mod q_formats {
    /// Q0 format (no fractional bits)
    pub const Q0: i32 = 0;
    /// Q9 format (9 fractional bits)
    pub const Q9: i32 = 9;
    /// Q12 format (12 fractional bits)
    pub const Q12: i32 = 12;
    /// Q13 format (13 fractional bits)
    pub const Q13: i32 = 13;
    /// Q14 format (14 fractional bits)
    pub const Q14: i32 = 14;
    /// Q15 format (15 fractional bits)
    pub const Q15: i32 = 15;
    /// Q30 format (30 fractional bits)
    pub const Q30: i32 = 30;
    /// Q31 format (31 fractional bits)
    pub const Q31: i32 = 31;
}

/// Error types specific to G.729
#[derive(Debug, thiserror::Error)]
pub enum G729Error {
    /// Invalid frame size error
    #[error("Invalid frame size: expected {expected}, got {actual}")]
    InvalidFrameSize { 
        /// Expected frame size
        expected: usize, 
        /// Actual frame size received
        actual: usize 
    },
    
    /// Invalid sample rate error
    #[error("Invalid sample rate: expected 8000 Hz, got {actual} Hz")]
    InvalidSampleRate { 
        /// Actual sample rate received
        actual: u32 
    },
    
    /// Invalid channel count error
    #[error("Invalid channel count: expected 1, got {actual}")]
    InvalidChannelCount { 
        /// Actual channel count received
        actual: u8 
    },
    
    /// Invalid bitstream format
    #[error("Invalid bitstream format")]
    InvalidBitstream,
    
    /// Encoder not initialized
    #[error("Encoder not initialized")]
    EncoderNotInitialized,
    
    /// Decoder not initialized
    #[error("Decoder not initialized")]
    DecoderNotInitialized,
    
    /// Bad frame detected
    #[error("Bad frame detected")]
    BadFrame,
}

/// Result type for G.729 operations
pub type Result<T> = std::result::Result<T, G729Error>; 