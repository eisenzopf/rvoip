//! ITU-T / 3GPP fixed-point basic operators — the ETSI "basicop" library.
//!
//! Every fixed-point speech codec in this crate specifies its arithmetic in
//! terms of these exact saturating operations. G.729 and AMR both do, so a
//! single implementation is the only way both can be bit-exact against the same
//! definitions; two copies would be two things to keep in agreement with the
//! spec and with each other.
//!
//! # Provenance
//!
//! Written for the G.729A port, where it was validated by that codec reaching
//! bit-exactness, and promoted out of `codecs::g729::impls::dsp` when AMR
//! needed the same foundation. G.729 continues to reach it under the old name
//! via a re-export, so its ~318 call sites were untouched by the move.
//!
//! # Scope
//!
//! Basic operators only — the table-free saturating arithmetic. Table-driven
//! transcendentals (`pow2`, `log2`, `inv_sqrt`) live with their codec, because
//! each supplies its own lookup tables and sharing the code before proving the
//! tables identical would be an assumption dressed as reuse. G.729's are in
//! [`crate::codecs::g729::impls::math`].
//!
//! # Overflow flags
//!
//! The reference C carries `Overflow` and `Carry` as globals that operators set
//! and a few algorithms read. [`DspContext`] threads them explicitly instead,
//! preserving the observable behaviour without global mutable state.

// This subtree is a transcription of reference fixed-point arithmetic. The
// narrowly enumerated lints below conflict with its deliberate wrapping casts,
// reference-style names and layout — the same exemption, and the same
// reasoning, as the G.729 implementation subtree it came from. Everything else
// in codec-core stays under the full strict lint policy.
#![allow(
    clippy::bool_to_int_with_if,
    clippy::branches_sharing_code,
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::if_not_else,
    clippy::inline_always,
    clippy::many_single_char_names,
    clippy::manual_contains,
    clippy::manual_let_else,
    clippy::match_same_arms,
    clippy::missing_const_for_fn,
    clippy::needless_pass_by_ref_mut,
    clippy::needless_pass_by_value,
    clippy::redundant_pub_crate,
    clippy::similar_names,
    clippy::single_match_else,
    clippy::too_many_lines,
    clippy::trivially_copy_pass_by_ref,
    clippy::unreadable_literal,
    clippy::unused_self,
    clippy::use_self,
    clippy::useless_let_if_seq,
    clippy::verbose_bit_mask
)]
// A complete operator library: not every operator has a caller yet, and the
// ones AMR needs but G.729 does not must still be present and correct. The
// same applies to the re-export shims that keep reference-style names
// available alongside the snake_case ones.
#![allow(dead_code)]
#![allow(unused_imports)]

/// 16-bit saturating arithmetic.
pub mod arith;
/// 32-bit saturating arithmetic.
pub mod arith32;
/// Fractional division.
pub mod div;
/// Extended-precision 32-bit helpers.
pub mod oper32;
/// The reference pseudo-random generator.
pub mod random;
/// Saturating shifts and normalisation.
pub mod shift;
/// Fixed-point value types and the operator status flags.
pub mod types;

/// Public re-export.
pub use types::{DspContext, Word16, Word32};
