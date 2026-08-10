//! The AMR-WB encoder, 3GPP TS 26.190 and TS 26.173 `cod_main.c`.
//!
//! Speech in, bitstream out. The decoder next door is bit-exact; this is the
//! harder half, and for a reason worth stating before any of it is read.
//!
//! # An encoder has to reproduce a decision
//!
//! A decoder that computes the wrong number produces audible damage. An encoder
//! that makes the wrong *choice* — picks a codebook entry that scores equally
//! well, resolves a tie the other way, visits candidates in a different order —
//! produces perfectly plausible speech at the far end and a bitstream no
//! conformant decoder reproduces. Nothing about the audio says so.
//!
//! Every module here that searches therefore documents its objective, its
//! comparison operator, its tie-break direction and its visit order as
//! normative, and tests the chosen *index* rather than only the vector the
//! index selects. Two codebook entries can dequantise to spectra a few LSBs
//! apart; an index test is strictly stronger than a vector test and is the one
//! that catches a tie broken the wrong way.
//!
//! # Ground truth
//!
//! `testdata/amrwb_enc_input.pcm` is 50 frames of deterministic pseudo-speech;
//! `testdata/amrwb_enc_mode*.amr` is what TS 26.173's own encoder makes of it at
//! each of the nine rates; `testdata/wb_enc_trace.txt` is three frames of that
//! encoder's per-stage intermediates at 12.65 kbit/s. The last is what these
//! modules are tested against, because a bitstream comparison alone says only
//! that something is wrong.
//!
//! `tools/trace-amrwb-encoder.sh <mode>` regenerates the full trace at any rate,
//! and asserts the instrumented build still reproduces the committed bitstream
//! byte for byte — so a trace point that changes behaviour rather than observing
//! it fails loudly rather than quietly moving the target.

pub mod analysis;
pub mod isf_quant;
pub mod pitch;
pub mod preproc;
