//! Opt-in, fail-closed admission for inbound transport connections.
//!
//! The ordinary normalized event bus is intentionally a lossy broadcast and
//! must not be used as a security decision boundary. An application that
//! installs this gate receives each inbound connection through one bounded,
//! single-consumer queue before `ConnectionInbound` is published. Dropping a
//! ticket, missing its deadline, or overrunning the queue rejects the transport
//! route and erases its retained authentication context.

use std::collections::BTreeSet;
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
use crate::media_graph::ManagedMediaRoute;
use crate::orchestrator::Orchestrator;
use crate::{DataMessage, MAX_DATA_LABEL_BYTES};

/// Hard ceiling for a pending admission's private control queue.
///
/// Staged data exists only for a small, protocol-owned pre-answer handshake;
/// it is not a general application DataChannel. Keeping the bound here makes
/// that distinction enforceable for every adapter.
pub const MAX_STAGED_INBOUND_DATA_CAPACITY: usize = 256;

/// Hard ceiling for each direction's reserved-label allowlist.
pub const MAX_STAGED_INBOUND_DATA_LABELS: usize = 16;

/// Exact, bounded policy for the private data exchange allowed while one
/// inbound connection is still awaiting admission.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StagedInboundDataPolicy {
    pub send_labels: BTreeSet<String>,
    pub receive_labels: BTreeSet<String>,
    pub capacity: usize,
}

impl StagedInboundDataPolicy {
    pub fn new(
        send_labels: impl IntoIterator<Item = impl Into<String>>,
        receive_labels: impl IntoIterator<Item = impl Into<String>>,
        capacity: usize,
    ) -> Self {
        Self {
            send_labels: send_labels.into_iter().map(Into::into).collect(),
            receive_labels: receive_labels.into_iter().map(Into::into).collect(),
            capacity,
        }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.capacity == 0 || self.capacity > MAX_STAGED_INBOUND_DATA_CAPACITY {
            return Err(RvoipError::InvalidState(
                "staged inbound data capacity is outside the supported range",
            ));
        }
        if self.send_labels.len() > MAX_STAGED_INBOUND_DATA_LABELS
            || self.receive_labels.len() > MAX_STAGED_INBOUND_DATA_LABELS
        {
            return Err(RvoipError::InvalidState(
                "staged inbound data allowlist is too large",
            ));
        }
        if self.send_labels.is_empty() && self.receive_labels.is_empty() {
            return Err(RvoipError::InvalidState(
                "staged inbound data policy has no allowed labels",
            ));
        }
        if self
            .send_labels
            .iter()
            .chain(self.receive_labels.iter())
            .any(|label| {
                label.is_empty()
                    || label.len() > MAX_DATA_LABEL_BYTES
                    || label.chars().any(char::is_control)
            })
        {
            return Err(RvoipError::InvalidState(
                "staged inbound data policy contains an invalid label",
            ));
        }
        Ok(())
    }
}

/// Private pre-answer data channel bound to one exact inbound admission.
///
/// The channel is revoked before accept/reject/timeout completes. Its messages
/// never enter the ordinary normalized or operational event streams.
#[must_use = "split the staged channel and retain the endpoints while admission is pending"]
pub struct StagedInboundDataChannel {
    sender: StagedInboundDataSender,
    receiver: StagedInboundDataReceiver,
}

impl StagedInboundDataChannel {
    pub(crate) fn new(
        connection_id: ConnectionId,
        transport: Transport,
        lifecycle_generation: u64,
        orchestrator: Weak<Orchestrator>,
        receiver: mpsc::Receiver<DataMessage>,
    ) -> Self {
        Self {
            sender: StagedInboundDataSender {
                connection_id,
                transport,
                lifecycle_generation,
                orchestrator,
            },
            receiver: StagedInboundDataReceiver { receiver },
        }
    }

    pub fn split(self) -> (StagedInboundDataSender, StagedInboundDataReceiver) {
        (self.sender, self.receiver)
    }
}

/// Cloneable sender for reserved-label messages to the still-pending peer.
#[derive(Clone)]
pub struct StagedInboundDataSender {
    connection_id: ConnectionId,
    transport: Transport,
    lifecycle_generation: u64,
    orchestrator: Weak<Orchestrator>,
}

impl StagedInboundDataSender {
    pub fn connection_id(&self) -> &ConnectionId {
        &self.connection_id
    }

    pub async fn send(&self, message: DataMessage) -> Result<()> {
        let orchestrator = self
            .orchestrator
            .upgrade()
            .ok_or(RvoipError::AdmissionRejected(
                "staged inbound data owner is unavailable",
            ))?;
        orchestrator
            .send_staged_inbound_data(
                &self.connection_id,
                self.transport,
                self.lifecycle_generation,
                message,
            )
            .await
    }
}

impl fmt::Debug for StagedInboundDataSender {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StagedInboundDataSender")
            .field("connection_id", &self.connection_id)
            .field("transport", &self.transport)
            .field("active_owner", &(self.orchestrator.strong_count() > 0))
            .finish()
    }
}

/// Single-consumer receiver for reserved-label messages from the pending peer.
pub struct StagedInboundDataReceiver {
    receiver: mpsc::Receiver<DataMessage>,
}

impl StagedInboundDataReceiver {
    pub async fn recv(&mut self) -> Option<DataMessage> {
        self.receiver.recv().await
    }

    pub fn try_recv(&mut self) -> std::result::Result<DataMessage, mpsc::error::TryRecvError> {
        self.receiver.try_recv()
    }
}

impl fmt::Debug for StagedInboundDataReceiver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StagedInboundDataReceiver")
            .field("closed", &self.receiver.is_closed())
            .finish()
    }
}

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
    provisional_media_started: bool,
    staged_data_opened: bool,
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
            provisional_media_started: false,
            staged_data_opened: false,
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

    /// Route one established connection's audio into this still-provisional
    /// inbound connection without sending a final answer.
    ///
    /// Core validates this admission's exact lifecycle generation, asks the
    /// target adapter to establish provisional media, and adds one managed
    /// sink to the source connection's reusable [`MediaGraph`](crate::MediaGraphHandle).
    /// It never consumes the provisional target's source receiver, so a later
    /// bidirectional bridge can acquire that direction exactly once.
    ///
    /// One admission may create at most one provisional route. Dropping the
    /// returned lease removes only this sink; call [`ProvisionalMediaRoute::stop`]
    /// when acknowledged removal is required before final-answer promotion.
    pub async fn bridge_early_media_from(
        &mut self,
        source_connection_id: ConnectionId,
    ) -> Result<ProvisionalMediaRoute> {
        if self.provisional_media_started {
            return Err(RvoipError::InvalidState(
                "inbound admission already started provisional media",
            ));
        }
        let orchestrator = self.orchestrator.upgrade().ok_or(RvoipError::InvalidState(
            "inbound admission owner is unavailable",
        ))?;
        let route = orchestrator
            .bridge_early_media_to_pending_inbound(
                source_connection_id,
                self.connection_id.clone(),
                self.transport,
                self.lifecycle_generation,
            )
            .await?;
        self.provisional_media_started = true;
        Ok(route)
    }

    /// Open the one private, bounded reserved-label channel permitted for this
    /// exact pending admission generation.
    ///
    /// This is intended for transport control handshakes that must finish
    /// before final answer. It is deliberately unavailable after admission and
    /// does not publish ordinary data-message events.
    pub fn open_staged_data_channel(
        &mut self,
        policy: StagedInboundDataPolicy,
    ) -> Result<StagedInboundDataChannel> {
        if self.staged_data_opened {
            return Err(RvoipError::InvalidState(
                "inbound admission already opened staged data",
            ));
        }
        let orchestrator = self
            .orchestrator
            .upgrade()
            .ok_or(RvoipError::AdmissionRejected(
                "staged inbound data owner is unavailable",
            ))?;
        let receiver = orchestrator.install_staged_inbound_data(
            &self.connection_id,
            self.transport,
            self.lifecycle_generation,
            policy,
        )?;
        self.staged_data_opened = true;
        Ok(StagedInboundDataChannel::new(
            self.connection_id.clone(),
            self.transport,
            self.lifecycle_generation,
            Arc::downgrade(&orchestrator),
            receiver,
        ))
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
            .field("provisional_media_started", &self.provisional_media_started)
            .field("staged_data_opened", &self.staged_data_opened)
            .field("resolved", &self.decision.is_none())
            .finish()
    }
}

/// Owning lease for a one-way provisional early-media route.
///
/// The route is source-to-target only. It contains no application-data route,
/// does not mark either connection as fully bridged, and does not accept the
/// target admission. Dropping the lease requests bounded graph cleanup.
#[must_use = "dropping the provisional media route removes its graph sink"]
pub struct ProvisionalMediaRoute {
    source_connection_id: ConnectionId,
    target_connection_id: ConnectionId,
    route: Option<ManagedMediaRoute>,
}

impl ProvisionalMediaRoute {
    pub(crate) fn new(
        source_connection_id: ConnectionId,
        target_connection_id: ConnectionId,
        route: ManagedMediaRoute,
    ) -> Self {
        Self {
            source_connection_id,
            target_connection_id,
            route: Some(route),
        }
    }

    pub fn source_connection_id(&self) -> &ConnectionId {
        &self.source_connection_id
    }

    pub fn target_connection_id(&self) -> &ConnectionId {
        &self.target_connection_id
    }

    /// Remove the sink and wait for the media graph to acknowledge it.
    pub async fn stop(mut self) -> Result<()> {
        if let Some(route) = self.route.take() {
            let _ = route.remove().await?;
        }
        Ok(())
    }
}

impl fmt::Debug for ProvisionalMediaRoute {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProvisionalMediaRoute")
            .field("source_connection_id", &self.source_connection_id)
            .field("target_connection_id", &self.target_connection_id)
            .field("active", &self.route.is_some())
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
