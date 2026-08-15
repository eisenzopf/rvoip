//! The AMR-NB encoder, 3GPP TS 26.090 (prose) and TS 26.073 (`cod_amr.c` and
//! the files it drives).
//!
//! Speech in, bitstream out. The decoder next door is bit-exact; this is the
//! harder half, for the reason its wideband twin states: a decoder that
//! computes the wrong number produces audible damage, while an encoder that
//! makes the wrong *choice* — picks an equally good codebook entry, resolves a
//! tie the other way, visits candidates in a different order — produces
//! perfectly plausible speech at the far end and a bitstream no conformant
//! decoder reproduces. Nothing about the audio says so.
//!
//! Every module here that searches therefore documents its objective, its
//! comparison operator, its tie-break direction and its visit order as
//! normative, and tests the chosen *index* rather than only the vector the
//! index selects.
//!
//! # Ground truth
//!
//! `testdata/amrnb_enc_input.pcm` is 50 frames of deterministic pseudo-speech;
//! `testdata/amrnb_enc_mode*.amr` is what TS 26.073's own encoder makes of it
//! at each of the eight rates; `testdata/nb_enc_trace.txt` is three frames of
//! that encoder's per-stage intermediates at 7.40 kbit/s. The last is what
//! these modules are tested against, because a bitstream comparison alone says
//! only that something is wrong.
//!
//! `tools/trace-amrnb-encoder.sh <mode>` regenerates the full 50-frame trace at
//! any rate, and asserts the instrumented build still reproduces the committed
//! bitstream byte for byte — so a trace point that changes behaviour rather
//! than observing it fails loudly rather than quietly moving the target. Note
//! that 4.75 kbit/s codes two subframes jointly and re-runs the first
//! subframe's post-processing afterwards, so some subframes appear twice in
//! its trace and the *second* occurrence is the one that counts.

pub mod analysis;
pub mod codebook;
pub mod dtx;
pub mod encoder;
pub mod gain_quant;
pub mod lsp_quant;
pub mod pitch;
pub mod preproc;
pub mod vad;
/// TS 26.073 `vad2.c`: the narrowband detector option 2.
pub mod vad2;
