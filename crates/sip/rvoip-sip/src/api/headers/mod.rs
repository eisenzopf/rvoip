//! Header API — uniform inbound inspection and outbound builder
//! shape.
//!
//! This module provides the four cornerstones of the gateway-grade
//! header surface introduced by `SIP_API_DESIGN_2.md`:
//!
//! - [`view::SipHeaderView`] — the trait every inbound wrapper
//!   (`IncomingCall`, `IncomingRequest`, `IncomingResponse`,
//!   `IncomingRegister`) implements so that B2BUA / SBC code can
//!   inspect headers generically.
//! - [`options::SipRequestOptions`] — the trait every outbound and
//!   response builder implements. Provides `with_header`,
//!   `with_headers`, `with_raw_header`, `strip_header`,
//!   `with_headers_from`, and `with_strictness` with default bodies
//!   driven by [`options::BuilderHeaderState`].
//! - [`policy`] — layer-boundary enforcement: classify any header for
//!   any method as `StackManaged` / `MethodShaped` /
//!   `ApplicationControlled` so the dialog state machine remains
//!   authoritative.
//! - [`convenience`] — typed constructors for headers without a
//!   first-class `TypedHeader` variant in sip-core
//!   (`Diversion`, `History-Info`, `Replaces`, …).

pub mod convenience;
pub mod options;
pub mod policy;
pub mod view;

pub use options::{
    take_staged, BuilderHeaderState, BuilderStrictness, HeaderCarryThroughReport,
    HeaderPolicyViolation, SipRequestOptions, ViolationReason,
};
pub use policy::{
    classify, forbidden_for_carry_through, validate_outbound, HeaderRole, MissingRequiredHeader,
};
pub use view::SipHeaderView;
