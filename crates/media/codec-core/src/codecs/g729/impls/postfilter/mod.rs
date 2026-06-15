//! Decoder post-filter stages.
//! Provenance: Post-filter stages adapted from ITU G.729 Annex A formant/pitch/AGC post-processing.
//! Q-format: Post-filter coefficients and synthesis states use Q12/Q14/Q15 fixed-point paths.

/// Public module `agc`.
pub mod agc;
/// Public module `formant`.
pub mod formant;
/// Public module `pipeline`.
pub mod pipeline;
/// Public module `pitch_pf`.
pub mod pitch_pf;
