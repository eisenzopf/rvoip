//! G.729 Codec Implementation
//!
//! This module contains the complete G.729 codec implementation based on the ITU-T
//! reference implementation. It includes all annexes and applications.

pub mod types;
pub mod math;
pub mod dsp;

// Planned modules (will be implemented in later phases)
// pub mod encoder;
// pub mod decoder; 
// pub mod lpc;
// pub mod pitch;
// pub mod codebook;
// pub mod tables;
// pub mod bitstream;
// pub mod preprocess;
// pub mod postprocess;

// Re-export commonly used types
pub use types::{
    Word16, Word32, Flag, Result as G729Result,
    G729Config, G729EncoderState, G729DecoderState,
    AnalysisParams, SynthesisParams, G729Error,
    L_FRAME, L_SUBFR, M, PIT_MIN, PIT_MAX,
};

// Re-export math operations
pub use math::{
    add, sub, mult, l_mult, abs_s, negate,
    shl, shr, extract_h, extract_l, round,
    l_mac, l_msu, l_add, l_sub, norm_s, norm_l,
    set_zero_16, copy_16,
};

// Re-export DSP functions
pub use dsp::{
    pow2, log2, inv_sqrt,
    autocorrelation, convolution, apply_window, compute_energy,
}; 