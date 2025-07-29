//! Main encoder and decoder implementations for G.729A

pub mod encoder;
pub mod decoder;
pub mod bitstream;

pub use encoder::G729AEncoder;
pub use decoder::G729ADecoder;
pub use bitstream::*; 