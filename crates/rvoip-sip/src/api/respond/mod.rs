//! Response builder surface — SIP_API_DESIGN_2 Phase D.
//!
//! Builders for AcceptBuilder / RejectBuilder / RedirectBuilder /
//! ProvisionalBuilder / AuthChallengeBuilder / GenericResponseBuilder
//! / RegisterResponseBuilder land here. Phase D ships the entry-point
//! wiring on the inbound wrapper types; this module is the
//! organizational home.

pub mod accept;
pub mod reject;
pub mod redirect;
pub mod provisional;
pub mod challenge;
pub mod generic;
pub mod register_response;

pub use accept::AcceptBuilder;
pub use reject::RejectBuilder;
pub use redirect::RedirectBuilder;
pub use provisional::ProvisionalBuilder;
pub use challenge::{AuthChallengeBuilder, AuthScheme};
pub use generic::GenericResponseBuilder;
pub use register_response::RegisterResponseBuilder;
