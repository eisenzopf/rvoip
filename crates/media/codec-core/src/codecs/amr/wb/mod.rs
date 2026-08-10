//! AMR wideband (3GPP TS 26.190 / ITU-T G.722.2).
//!
//! # Status
//!
//! Under construction. The LP analysis front end is present in floating point;
//! there is no encoder or decoder yet, and `AmrCodec` still reports
//! `FeatureNotEnabled` for both. See `docs/AMR_IMPLEMENTATION_STATUS.md`.
//!
//! Wideband is being built before narrowband because it is the HD-voice
//! deliverable; the two share `crate::fixed_point` and will share most of the
//! ACELP machinery.

pub mod lp;
