//! Outbound builder surface — SIP_API_DESIGN_2 §3.3.
//!
//! One builder per outbound SIP method, every one implementing
//! [`SipRequestOptions`](crate::api::headers::SipRequestOptions). Each
//! builder is reachable from
//! [`UnifiedCoordinator`](crate::api::unified::UnifiedCoordinator) (and
//! through the [`Surface`] adapter from the other three surfaces) by a
//! verb-named entry point:
//!
//! ```text
//! coord.invite(from, to).with_credentials(c).send().await
//! coord.register(reg, user, pw).with_expires(3600).send().await
//! coord.refer(&sess, target).with_replaces(rep).send().await
//! ```
//!
//! Every `.send()` consumes the builder. Header staging goes through
//! the shared `BuilderHeaderState` so header-policy enforcement
//! behaves identically across builders.

pub mod outbound_call;
pub mod refer;
pub mod bye;
pub mod cancel;
pub mod notify;
pub mod info;
pub mod update;
pub mod reinvite;
pub mod register;
pub mod subscribe;
pub mod message;
pub mod options;
pub mod surface;

pub use outbound_call::{OutboundCallBuilder, PaiOverride, ProxyOverride};
pub use refer::ReferBuilder;
pub use bye::ByeBuilder;
pub use cancel::CancelBuilder;
pub use notify::NotifyBuilder;
pub use info::InfoBuilder;
pub use update::UpdateBuilder;
pub use reinvite::ReInviteBuilder;
pub use register::{RegisterBuilder, RegisterRefreshBuilder};
pub use subscribe::{SubscribeBuilder, SubscribeRefreshBuilder};
pub use message::MessageBuilder;
pub use options::OptionsBuilder;
pub use surface::{Surface, SurfaceBuilder};
