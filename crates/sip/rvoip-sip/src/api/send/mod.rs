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
//! coord.invite(from, to).with_auth(auth).send().await
//! coord.register(reg, user, pw).with_expires(3600).send().await
//! coord.refer(&sess, target).with_replaces(rep).send().await
//! ```
//!
//! In-dialog builders (`bye`, `cancel`, `refer`, `notify`, `info`,
//! `update`, `reinvite`) are also reachable directly on a
//! [`SessionHandle`](crate::api::handle::SessionHandle) returned from
//! `invite().send().await` / `accept().await`:
//!
//! ```text
//! session.refer(target).with_replaces(rep).send().await
//! session.bye().with_reason(reason).send().await
//! session.info("application/dtmf-relay").with_body(dtmf).send().await
//! ```
//!
//! Every `.send()` consumes the builder. Header staging goes through
//! the shared `BuilderHeaderState` so header-policy enforcement
//! behaves identically across builders.

use crate::api::handle::CallId;
use crate::errors::{Result, SessionError};
use crate::session_registry::SessionRegistryHandle;

/// Private authority carried by in-dialog builders. Coordinator-created
/// builders preserve the public capture-current facade; SessionHandle-created
/// builders must retain the handle's exact generation or fail closed.
#[derive(Clone)]
pub(crate) enum InDialogRequestAuthority {
    CaptureCurrent,
    Captured(Option<SessionRegistryHandle>),
}

impl InDialogRequestAuthority {
    pub(crate) fn captured(handle: Option<SessionRegistryHandle>) -> Self {
        Self::Captured(handle)
    }

    pub(crate) fn exact_handle(
        &self,
        session_id: &CallId,
    ) -> Result<Option<SessionRegistryHandle>> {
        match self {
            Self::CaptureCurrent => Ok(None),
            Self::Captured(Some(handle)) if handle.session_id() == session_id => {
                Ok(Some(handle.clone()))
            }
            Self::Captured(Some(_)) => Err(SessionError::InvalidTransition(
                "captured builder authority does not match its session".to_string(),
            )),
            Self::Captured(None) => Err(SessionError::SessionNotFound(format!(
                "Session {} has no exact builder authority",
                session_id
            ))),
        }
    }
}

pub mod bye;
pub mod cancel;
pub mod info;
pub mod message;
pub mod notify;
pub mod options;
pub mod outbound_call;
pub mod refer;
pub mod register;
pub mod reinvite;
pub mod subscribe;
pub mod surface;
pub mod update;

pub use bye::ByeBuilder;
pub use cancel::CancelBuilder;
pub use info::InfoBuilder;
pub use message::MessageBuilder;
pub use notify::NotifyBuilder;
pub use options::OptionsBuilder;
pub use outbound_call::{OutboundCallBuilder, PaiOverride, ProxyOverride};
pub use refer::ReferBuilder;
pub use register::{RegisterBuilder, RegisterRefreshBuilder};
pub use reinvite::ReInviteBuilder;
pub use subscribe::{SubscribeBuilder, SubscribeRefreshBuilder};
pub use surface::{Surface, SurfaceBuilder};
pub use update::UpdateBuilder;
