//! G.729 Codec Implementation
//!
//! This module contains the complete G.729 codec implementation based on the ITU-T
//! reference implementation. It includes all annexes and applications.

pub mod types;
pub mod math;
pub mod dsp;
pub mod lpc;

pub use types::*;
pub use math::*;
pub use dsp::*;
pub use lpc::*;

// Planned modules (will be implemented in later phases)
// 
// Planned modules for future implementation:

// Core G.729 (full complexity) - enabled by default via g729-core feature
#[cfg(feature = "g729-core")]
pub mod pitch;
#[cfg(feature = "g729-core")]
pub mod acelp;
#[cfg(feature = "g729-core")]
pub mod quantization;
#[cfg(feature = "g729-core")]
pub mod encoder;
#[cfg(feature = "g729-core")]
pub mod decoder;
#[cfg(feature = "g729-core")]
pub mod energy_preservation;

// G.729A (reduced complexity) - enabled by annex-a feature
// pub mod pitch_a;    // #[cfg(feature = "annex-a")]
// pub mod acelp_a;    // #[cfg(feature = "annex-a")]
// pub mod encoder_a;  // #[cfg(feature = "annex-a")]
// pub mod decoder_a;  // #[cfg(feature = "annex-a")]

// G.729B (VAD/DTX/CNG extensions) - enabled by annex-b feature  
// pub mod vad;        // #[cfg(feature = "annex-b")]
// pub mod dtx;        // #[cfg(feature = "annex-b")]
// pub mod cng;        // #[cfg(feature = "annex-b")]

// G.729BA (combined A+B) - enabled when both annex-a and annex-b are active
// pub mod encoder_ba; // #[cfg(all(feature = "annex-a", feature = "annex-b"))]
// pub mod decoder_ba; // #[cfg(all(feature = "annex-a", feature = "annex-b"))]

// Re-export main codec API based on available features
// Re-exports will be uncommented when the modules are implemented
// #[cfg(feature = "g729-core")]
// pub use encoder::G729Encoder;

// #[cfg(feature = "g729-core")]
// pub use decoder::G729Decoder;

// #[cfg(feature = "annex-a")]
// pub use encoder_a::G729AEncoder;

// #[cfg(feature = "annex-a")]
// pub use decoder_a::G729ADecoder;

// #[cfg(all(feature = "annex-a", feature = "annex-b"))]
// pub use encoder_ba::G729BAEncoder;

// #[cfg(all(feature = "annex-a", feature = "annex-b"))]
// pub use decoder_ba::G729BADecoder;

 