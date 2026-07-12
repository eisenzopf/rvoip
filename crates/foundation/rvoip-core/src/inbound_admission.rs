//! Opt-in, fail-closed admission for inbound transport connections.
//!
//! The ordinary normalized event bus is intentionally a lossy broadcast and
//! must not be used as a security decision boundary. An application that
//! installs this gate receives each inbound connection through one bounded,
//! single-consumer queue before `ConnectionInbound` is published. Dropping a
//! ticket, missing its deadline, or overrunning the queue rejects the transport
//! route and erases its retained authentication context.

use std::fmt;
use std::sync::{Arc, Weak};
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::{mpsc, oneshot, Semaphore};

use crate::adapter::{InboundConnectionContext, RejectReason};
use crate::connection::Transport;
use crate::error::{Result, RvoipError};
use crate::identity::AuthenticatedPrincipal;
use crate::ids::ConnectionId;
use crate::orchestrator::Orchestrator;

pub(crate) enum InboundAdmissionDisposition {
    Accept,
    Reject(RejectReason),
}

pub(crate) struct InboundAdmissionDecision {
    pub(crate) disposition: InboundAdmissionDisposition,
    pub(crate) completion: Option<oneshot::Sender<bool>>,
}

/// Installed bounded admission channel and its independent waiter budget.
pub(crate) struct InboundAdmissionGate {
    pub(crate) sender: mpsc::Sender<InboundAdmission>,
    pub(crate) permits: Arc<Semaphore>,
    pub(crate) decision_timeout: Duration,
}

impl InboundAdmissionGate {
    pub(crate) fn new(
        capacity: usize,
        decision_timeout: Duration,
    ) -> (Self, mpsc::Receiver<InboundAdmission>) {
        let (sender, receiver) = mpsc::channel(capacity);
        (
            Self {
                sender,
                permits: Arc::new(Semaphore::new(capacity)),
                decision_timeout,
            },
            receiver,
        )
    }
}

/// One unresolved inbound connection presented to an application policy.
///
/// The ticket deliberately does not expose authentication or routing material
/// through `Debug`, `Display`, or serialization. Callers explicitly obtain the
/// current complete principal and single-take context through methods that
/// revalidate the connection's lifecycle generation. The normalized public
/// inbound event is emitted only after [`Self::accept`] completes.
pub struct InboundAdmission {
    connection_id: ConnectionId,
    transport: Transport,
    observed_at: DateTime<Utc>,
    lifecycle_generation: u64,
    orchestrator: Weak<Orchestrator>,
    decision: Option<oneshot::Sender<InboundAdmissionDecision>>,
    context_taken: bool,
}

impl InboundAdmission {
    pub(crate) fn new(
        connection_id: ConnectionId,
        transport: Transport,
        observed_at: DateTime<Utc>,
        lifecycle_generation: u64,
        orchestrator: Weak<Orchestrator>,
        decision: oneshot::Sender<InboundAdmissionDecision>,
    ) -> Self {
        Self {
            connection_id,
            transport,
            observed_at,
            lifecycle_generation,
            orchestrator,
            decision: Some(decision),
            context_taken: false,
        }
    }

    /// Exact adapter connection awaiting policy admission.
    pub fn connection_id(&self) -> &ConnectionId {
        &self.connection_id
    }

    /// Transport that owns the pending connection.
    pub const fn transport(&self) -> Transport {
        self.transport
    }

    /// Adapter observation time retained for diagnostics and policy deadlines.
    pub const fn observed_at(&self) -> DateTime<Utc> {
        self.observed_at
    }

    /// Return the complete principal still bound to this live generation.
    ///
    /// Anonymous or legacy adapters without a retained complete principal fail
    /// closed with `InvalidState`; callers must never infer one from a routing
    /// hint.
    pub fn authenticated_principal(&self) -> Result<AuthenticatedPrincipal> {
        let orchestrator = self.orchestrator.upgrade().ok_or(RvoipError::InvalidState(
            "inbound admission owner is unavailable",
        ))?;
        orchestrator.inbound_admission_principal(
            &self.connection_id,
            self.transport,
            self.lifecycle_generation,
        )
    }

    /// Consume this connection's principal-bound adapter context exactly once.
    ///
    /// A failed lifecycle or ownership check does not expose the context. A
    /// successful call returning `None` means the adapter supplied no context;
    /// subsequent calls also return `None`.
    pub fn take_inbound_context(&mut self) -> Result<Option<InboundConnectionContext>> {
        if self.context_taken {
            return Ok(None);
        }
        let orchestrator = self.orchestrator.upgrade().ok_or(RvoipError::InvalidState(
            "inbound admission owner is unavailable",
        ))?;
        let context = orchestrator.take_inbound_admission_context(
            &self.connection_id,
            self.transport,
            self.lifecycle_generation,
        )?;
        self.context_taken = true;
        Ok(context)
    }

    /// Admit the durably authorized connection and wait until normalized
    /// publication has either committed or lost its lifecycle race.
    pub async fn accept(self) -> Result<()> {
        self.resolve(InboundAdmissionDisposition::Accept).await
    }

    /// Reject the connection and wait until its core route has been erased.
    pub async fn reject(self, reason: RejectReason) -> Result<()> {
        self.resolve(InboundAdmissionDisposition::Reject(reason))
            .await
    }

    async fn resolve(mut self, disposition: InboundAdmissionDisposition) -> Result<()> {
        let decision = self.decision.take().ok_or(RvoipError::InvalidState(
            "inbound admission was already resolved",
        ))?;
        let (completion, completed) = oneshot::channel();
        decision
            .send(InboundAdmissionDecision {
                disposition,
                completion: Some(completion),
            })
            .map_err(|_| RvoipError::AdmissionRejected("inbound admission decision expired"))?;
        match completed.await {
            Ok(true) => Ok(()),
            Ok(false) | Err(_) => Err(RvoipError::AdmissionRejected(
                "inbound connection ended during admission",
            )),
        }
    }
}

impl fmt::Debug for InboundAdmission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InboundAdmission")
            .field("connection_id", &self.connection_id)
            .field("transport", &self.transport)
            .field("observed_at", &self.observed_at)
            .field("context_taken", &self.context_taken)
            .field("resolved", &self.decision.is_none())
            .finish()
    }
}

impl Drop for InboundAdmission {
    fn drop(&mut self) {
        let Some(decision) = self.decision.take() else {
            return;
        };
        let _ = decision.send(InboundAdmissionDecision {
            disposition: InboundAdmissionDisposition::Reject(RejectReason::ServerError),
            completion: None,
        });
    }
}
