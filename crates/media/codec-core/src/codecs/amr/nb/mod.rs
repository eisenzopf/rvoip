//! AMR narrowband, 3GPP TS 26.090 (prose) and TS 26.073 (the normative
//! fixed-point reference).
//!
//! 8 kHz, 160 samples per 20 ms frame, four 40-sample subframes, LP order 10,
//! eight rates from 4.75 to 12.2 kbit/s.
//!
//! # Relationship to the wideband implementation
//!
//! The two codecs are close relatives and it is tempting to share more than is
//! safe. They differ in ways that are invisible at a glance and produce audio
//! that sounds right while being wrong:
//!
//! - Narrowband uses **LSP/LSF**, wideband **ISP/ISF**. These are different
//!   representations, not the same one at a different order.
//! - Narrowband has a **decoder post-filter**; wideband has none.
//! - Each narrowband rate has its **own algebraic codebook**, where wideband
//!   has one family parameterised by width.
//! - `packed_size` in the two references uses different conventions: the
//!   narrowband table includes the `ToC` byte, the wideband one does not.
//!
//! Anything genuinely shared lives in [`crate::fixed_point`] or in the
//! variant-agnostic modules one level up ([`super::payload`],
//! [`super::storage`], [`super::mode`]).

pub mod bitstream;
pub mod decoder_tables;
pub mod tables;
