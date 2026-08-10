//! AMR wideband (3GPP TS 26.190 / ITU-T G.722.2).
//!
//! # Status
//!
//! Under construction. The LP analysis front end exists; there is no encoder or
//! decoder yet, and `AmrCodec` reports `FeatureNotEnabled` for both. See
//! `docs/AMR_IMPLEMENTATION_STATUS.md`.
//!
//! Everything here is fixed point, because that is what AMR is: TS 26.190 §8.1
//! specifies the codec "in a bit-exact arithmetic", and TS 26.173 — the ANSI-C
//! reference — is the definition. A floating-point implementation would be a
//! different codec, failing conformance and producing different bitstreams from
//! identical audio.

pub mod bitstream;
pub mod codebook;
pub mod highband;
pub mod lp;
pub mod decoder;
pub mod conceal;
pub mod enc;
pub mod enhance;
pub mod excitation;
pub mod gain;
pub mod gain_tables;
pub mod ltp;
pub mod math;
pub mod params;
pub mod sort_tables;
pub mod synthesis;
