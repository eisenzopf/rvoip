//! G.729A speech codec implementation
//! 
//! This is a pure Rust implementation of the G.729 Annex A (G.729A) speech codec.
//! G.729A is a reduced-complexity version of G.729 that operates at 8 kbit/s.

#![warn(missing_docs)]

pub mod constants;
pub mod types;
pub mod math;
pub mod signal;
pub mod spectral;
pub mod perception;
pub mod excitation;
pub mod synthesis;
pub mod codec;

// Re-export commonly used items
pub use constants::*;
pub use types::*;
pub use codec::{G729AEncoder, G729ADecoder};

/// G.729A codec version information
pub const CODEC_VERSION: &str = "0.1.0";

/// G.729A codec name
pub const CODEC_NAME: &str = "G.729A";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_codec_info() {
        assert_eq!(CODEC_NAME, "G.729A");
        assert_eq!(SAMPLE_RATE, 8000);
        assert_eq!(FRAME_SIZE, 80);
    }
} 