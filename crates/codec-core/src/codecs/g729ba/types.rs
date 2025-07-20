//! ITU-T G.729BA Types and Constants
//!
//! This module defines the types and constants used in the G.729BA implementation,
//! based on the ITU reference code LD8A.H, DTX.H, VAD.H and other headers from c_codeBA

use crate::codecs::g729a::types::*;

// Re-export all G729A types and constants
pub use crate::codecs::g729a::types::*;

// Additional G.729B/BA specific constants

/// VAD frame size (should match L_FRAME)
pub const VAD_FRAME_SIZE: usize = L_FRAME;

/// DTX hangover time in frames
pub const DTX_HANG_TIME: usize = 6;

/// CNG seed initialization value
pub const CNG_SEED_INIT: u32 = 12345;

/// VAD energy buffer size
pub const VAD_BUFFER_SIZE: usize = 16;

/// SID frame size in bits (15 bits for SID vs 80 bits for speech)
pub const SID_FRAME_BITS: usize = 15;

/// Number of LSF quantization stages for SID frames
pub const SID_LSF_STAGES: usize = 3;

/// Size of SID LSF codebook stage 0
pub const SID_LSF_CB0_SIZE: usize = 64;

/// Size of SID LSF codebook stage 1 
pub const SID_LSF_CB1_SIZE: usize = 64;

/// Size of SID LSF codebook stage 2
pub const SID_LSF_CB2_SIZE: usize = 64;

/// Size of SID gain codebook
pub const SID_GAIN_CB_SIZE: usize = 32;

/// Number of autocorrelation lags for VAD
pub const VAD_NP: usize = 12;

/// Length of spectral comparison for VAD
pub const VAD_SPEC_LEN: usize = 9;

/// VAD parameter count
pub const VAD_PARAM_COUNT: usize = 3;

/// DTX hangover period (frames)
pub const DTX_HANGOVER: usize = 7;

/// CNG update period (frames)  
pub const CNG_UPDATE_PERIOD: usize = 8;

/// Maximum SID gain in Q14 format
pub const SID_MAX_GAIN: Word16 = 16383;

/// Minimum SID gain in Q14 format  
pub const SID_MIN_GAIN: Word16 = 32;

/// Frame types for G.729B/BA
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum G729BFrameType {
    /// Active speech frame (80 bits)
    Speech = 0,
    /// SID frame (15 bits) - first SID  
    SidFirst = 1,
    /// SID frame (15 bits) - update SID
    SidUpdate = 2,
    /// No transmission (0 bits)
    NoData = 3,
}

/// VAD decision results
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadDecision {
    /// Voice detected
    Voice = 1,
    /// Noise/silence detected  
    Noise = 0,
}

/// DTX encoder state
#[derive(Debug, Clone)]
pub struct DtxEncoderState {
    /// SID frame flag
    pub sid_frame: Flag,
    /// Number of consecutive non-transmission frames
    pub nb_ener: Word16,
    /// Energy history for SID computation
    pub ener_hist: [Word32; DTX_HANGOVER + 1],
    /// LSF history for SID computation  
    pub lsf_hist: [[Word16; M]; DTX_HANGOVER + 1],
    /// Current SID LSF parameters
    pub sid_lsf: [Word16; M],
    /// Current SID energy
    pub sid_ener: Word32,
    /// DTX hangover counter
    pub hangover_cnt: Word16,
    /// Previous VAD decision
    pub prev_vad: Word16,
}

/// VAD state structure  
#[derive(Debug, Clone)]
pub struct VadState {
    /// Mean energy
    pub mean_se: Word16,
    /// Mean low band energy  
    pub mean_sle: Word16,
    /// Mean zero crossing rate
    pub mean_szc: Word16,
    /// Mean total energy
    pub mean_e: Word16,
    /// Previous energy values
    pub prev_energy: [Word16; VAD_PARAM_COUNT],
    /// Spectral distortion measures
    pub spectral_dist: [Word16; VAD_SPEC_LEN],
    /// VAD parameters
    pub vad_params: [Word16; VAD_PARAM_COUNT],
    /// Tone detection flag
    pub tone_flag: Word16,
    /// Adaptation counter
    pub adapt_count: Word16,
}

/// CNG decoder state
#[derive(Debug, Clone)]
pub struct CngDecoderState {
    /// SID frame LSF parameters
    pub sid_lsf: [Word16; M],
    /// SID frame energy
    pub sid_ener: Word32,
    /// Current noise LSF parameters
    pub cur_lsf: [Word16; M],
    /// Current noise energy  
    pub cur_ener: Word32,
    /// Interpolation factor
    pub interp_factor: Word16,
    /// Random seed for noise generation
    pub random_seed: Word16,
    /// CNG update counter
    pub update_cnt: Word16,
}

/// Extended encoder state for G.729BA
#[derive(Debug, Clone)]
pub struct G729BAEncoderState {
    /// Base G.729A encoder state
    pub base_state: G729AEncoderState,
    /// VAD state
    pub vad_state: VadState,
    /// DTX encoder state
    pub dtx_state: DtxEncoderState,
    /// Current frame type
    pub frame_type: G729BFrameType,
    /// VAD enable flag
    pub vad_enable: bool,
    /// DTX enable flag  
    pub dtx_enable: bool,
}

/// Extended decoder state for G.729BA
#[derive(Debug, Clone)]
pub struct G729BADecoderState {
    /// Base G.729A decoder state
    pub base_state: G729ADecoderState,
    /// CNG decoder state
    pub cng_state: CngDecoderState,
    /// Previous frame type
    pub prev_frame_type: G729BFrameType,
    /// CNG enable flag
    pub cng_enable: bool,
}

impl Default for VadState {
    fn default() -> Self {
        Self {
            mean_se: 0,
            mean_sle: 0,
            mean_szc: 0,
            mean_e: 0,
            prev_energy: [0; VAD_PARAM_COUNT],
            spectral_dist: [0; VAD_SPEC_LEN],
            vad_params: [0; VAD_PARAM_COUNT],
            tone_flag: 0,
            adapt_count: 0,
        }
    }
}

impl Default for DtxEncoderState {
    fn default() -> Self {
        Self {
            sid_frame: 0,
            nb_ener: 0,
            ener_hist: [0; DTX_HANGOVER + 1],
            lsf_hist: [[0; M]; DTX_HANGOVER + 1],
            sid_lsf: [0; M],
            sid_ener: 0,
            hangover_cnt: DTX_HANGOVER as Word16,
            prev_vad: 1, // Start with voice assumption
        }
    }
}

impl Default for CngDecoderState {
    fn default() -> Self {
        Self {
            sid_lsf: [0; M],
            sid_ener: 0,
            cur_lsf: [0; M],
            cur_ener: 0,
            interp_factor: 0,
            random_seed: 1,
            update_cnt: 0,
        }
    }
}

impl Default for G729BAEncoderState {
    fn default() -> Self {
        Self {
            base_state: G729AEncoderState::default(),
            vad_state: VadState::default(),
            dtx_state: DtxEncoderState::default(),
            frame_type: G729BFrameType::Speech,
            vad_enable: true,
            dtx_enable: true,
        }
    }
}

impl Default for G729BADecoderState {
    fn default() -> Self {
        Self {
            base_state: G729ADecoderState::default(),
            cng_state: CngDecoderState::default(),
            prev_frame_type: G729BFrameType::Speech,
            cng_enable: true,
        }
    }
} 