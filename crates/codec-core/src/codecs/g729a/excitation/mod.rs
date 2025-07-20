//! Excitation generation modules for G.729A codec

pub mod adaptive_codebook;
pub mod algebraic_codebook;
pub mod gain_processor;

pub use adaptive_codebook::*;
pub use algebraic_codebook::*;
pub use gain_processor::*; 