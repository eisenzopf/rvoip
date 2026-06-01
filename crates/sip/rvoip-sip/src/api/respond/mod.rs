//! Response builder surface — SIP_API_DESIGN_2 Phase D.
//!
//! Builders for AcceptBuilder / RejectBuilder / RedirectBuilder /
//! ProvisionalBuilder / AuthChallengeBuilder / GenericResponseBuilder
//! / RegisterResponseBuilder land here. Phase D ships the entry-point
//! wiring on the inbound wrapper types; this module is the
//! organizational home.

pub mod accept;
pub mod challenge;
pub mod generic;
pub mod provisional;
pub mod redirect;
pub mod register_response;
pub mod reject;

pub use accept::AcceptBuilder;
pub use challenge::{AuthChallengeBuilder, AuthScheme};
pub use generic::GenericResponseBuilder;
pub use provisional::ProvisionalBuilder;
pub use redirect::RedirectBuilder;
pub use register_response::RegisterResponseBuilder;
pub use reject::RejectBuilder;
