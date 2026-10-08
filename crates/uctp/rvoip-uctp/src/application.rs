//! Opt-in application command profiles over the shared UCTP envelope.
//!
//! Authentication, signature checks and transport limits remain coordinator
//! responsibilities. The handler authorizes Conversation membership and owns
//! durable operation idempotency. Profiles never replace legacy media dispatch.

use async_trait::async_trait;
use rvoip_core::AuthenticatedPrincipal;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::envelope::UctpEnvelope;

pub struct ApplicationContext {
    pub principal: AuthenticatedPrincipal,
    /// A bounded peer output channel. Do not block media on observer output.
    pub outbound: mpsc::Sender<UctpEnvelope>,
    /// Cancel observer work when this physical peer closes.
    pub closed: CancellationToken,
}

#[derive(Debug)]
pub struct ApplicationError {
    pub code: u16,
    pub reason: String,
}

impl ApplicationError {
    pub fn new(code: u16, reason: impl Into<String>) -> Self {
        Self {
            code,
            reason: reason.into(),
        }
    }
}

#[async_trait]
pub trait ApplicationHandler: Send + Sync {
    /// Explicit opt-in in `payload.profile`, advertised in auth.challenge.
    fn profile(&self) -> &'static str;

    fn required_scope(&self) -> &'static str {
        "uctp:conversation-control"
    }

    /// Authorize the principal's access to every referenced object before
    /// executing a typed protocol command. Persist idempotency before effects.
    ///
    /// Runs inline on the peer's signaling driver and is bounded by
    /// `UctpCoordinatorCaps::application_handler_timeout`. On expiry the
    /// future is dropped and the peer receives a `504` error, so long-running
    /// work belongs on a spawned task that reports through `context.outbound`.
    async fn handle(
        &self,
        context: ApplicationContext,
        request: UctpEnvelope,
    ) -> Result<UctpEnvelope, ApplicationError>;

    /// Replays may only retrieve an existing outcome, never repeat effects.
    /// Called after all auth/signature checks, when the replay cache recognizes
    /// the envelope. Implementations must compare the stored command content.
    async fn replay(
        &self,
        _context: ApplicationContext,
        _request: UctpEnvelope,
    ) -> Result<UctpEnvelope, ApplicationError> {
        Err(ApplicationError::new(409, "duplicate-request"))
    }
}
