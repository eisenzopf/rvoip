//! Authoritative, bounded operational lifecycle events.
//!
//! [`crate::Event`] remains a compatibility and observability broadcast. It
//! may lag or lose messages when a receiver overruns. Applications whose call
//! state depends on connection events can instead install a receiver of
//! [`OperationalEvent`] values through
//! [`crate::Orchestrator::install_operational_event_stream`]. Core awaits this
//! single-consumer stream before publishing the corresponding compatibility
//! event, so bounded backpressure is propagated to adapter ingestion without
//! an overflow queue or detached forwarding task.
//!
//! The per-adapter normalization task created by
//! [`crate::Orchestrator::register`] is retained by the Orchestrator and can
//! be joined through [`crate::Orchestrator::drain_connection_lifecycle_tasks`].
//! This module adds no separate forwarding task.

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::sync::{mpsc, watch};

use crate::adapter::{EndReason, TransferAttemptId, TransferStatus, TransferTarget};
use crate::connection::Transport;
use crate::ids::ConnectionId;
use crate::DataMessage;

/// Health of the optional authoritative operational stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum OperationalEventStreamHealth {
    /// No authoritative stream was requested; legacy broadcast behavior is
    /// unchanged.
    NotInstalled,
    /// The installed receiver remains available.
    Healthy,
    /// The receiver was lost or the process-local sequence space was
    /// exhausted. This state is sticky for the process lifetime.
    Degraded,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum OperationalEventStreamFailure {
    ReceiverLost,
    DeliveryCancelled,
    SequenceExhausted,
    SendFailed,
}

impl OperationalEventStreamFailure {
    fn metric_label(self) -> &'static str {
        match self {
            Self::ReceiverLost => "receiver_lost",
            Self::DeliveryCancelled => "delivery_cancelled",
            Self::SequenceExhausted => "sequence_exhausted",
            Self::SendFailed => "send_failed",
        }
    }
}

/// Task-free subscription to sticky authoritative-stream health.
///
/// [`Self::current`] exposes the initial state immediately. [`Self::changed`]
/// waits for either a published health transition or loss of the operational
/// event receiver. Receiver loss is converted to the same sticky `Degraded`
/// state inline, so subscribing does not create an idle background task that
/// could outlive core lifecycle drain.
#[derive(Clone)]
pub struct OperationalEventStreamHealthSubscription {
    updates: watch::Receiver<OperationalEventStreamHealth>,
    receiver_closed: mpsc::Sender<OperationalEvent>,
    health: Arc<OperationalEventStreamHealthState>,
}

impl OperationalEventStreamHealthSubscription {
    /// Return current sticky health without waiting.
    pub fn current(&self) -> OperationalEventStreamHealth {
        if self.receiver_closed.is_closed() {
            self.health
                .mark_degraded(OperationalEventStreamFailure::ReceiverLost);
        }
        self.health.current()
    }

    /// Wait for degradation and return the new sticky state.
    ///
    /// `Degraded` is terminal for this process-local stream. Calling this
    /// method after observing it therefore returns `Degraded` immediately;
    /// correctness consumers should stop accepting work at that point.
    pub async fn changed(&mut self) -> OperationalEventStreamHealth {
        let current = self.current();
        if current == OperationalEventStreamHealth::Degraded {
            self.updates.borrow_and_update();
            return current;
        }
        tokio::select! {
            changed = self.updates.changed() => {
                if changed.is_err() {
                    // The subscription owns the health state, which owns the
                    // watch sender, so this is unreachable without a future
                    // implementation error. Fail closed if that invariant is
                    // ever broken.
                    self.health.mark_degraded(
                        OperationalEventStreamFailure::DeliveryCancelled,
                    );
                }
            }
            () = self.receiver_closed.closed() => {
                self.health.mark_degraded(
                    OperationalEventStreamFailure::ReceiverLost,
                );
            }
        }
        let current = self.health.current();
        self.updates.borrow_and_update();
        current
    }
}

/// Sanitized terminal disposition. Adapter-owned free-form failure text is
/// deliberately excluded from the correctness stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum OperationalEndReason {
    Normal,
    Cancelled,
    Failed,
    Timeout,
    BridgeTorn,
}

impl From<&EndReason> for OperationalEndReason {
    fn from(reason: &EndReason) -> Self {
        match reason {
            EndReason::Normal => Self::Normal,
            EndReason::Cancelled => Self::Cancelled,
            EndReason::Failed { .. } => Self::Failed,
            EndReason::Timeout => Self::Timeout,
            EndReason::BridgeTorn => Self::BridgeTorn,
        }
    }
}

/// Stable failure category that cannot carry adapter credentials or peer
/// supplied diagnostic text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum OperationalFailureReason {
    AdapterReported,
    CoreReported,
}

/// Redacted transfer destination shape. URI contents remain with the caller
/// and adapter; they are not copied into the operational stream.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum OperationalTransferTarget {
    Uri,
    Connection(ConnectionId),
    Session(crate::ids::SessionId),
}

impl From<&TransferTarget> for OperationalTransferTarget {
    fn from(target: &TransferTarget) -> Self {
        match target {
            TransferTarget::Uri(_) => Self::Uri,
            TransferTarget::Connection(connection_id) => Self::Connection(connection_id.clone()),
            TransferTarget::Session(session_id) => Self::Session(session_id.clone()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum OperationalTransferOutcome {
    /// The adapter accepted the command for asynchronous processing.
    Submitted,
    Succeeded,
    Failed,
}

/// Transport-neutral events required to maintain authoritative call state.
///
/// Payload-bearing variants intentionally use a custom [`Debug`]
/// implementation. DTMF digits, DataChannel bytes and labels are available to
/// the owning receiver but never appear in routine formatting.
#[derive(Clone, Eq, PartialEq)]
#[non_exhaustive]
pub enum OperationalEventKind {
    Connected,
    /// Provisional signaling for this exact connection lifecycle.
    ///
    /// `early_media` means the adapter has received a media-bearing
    /// provisional response and can expose its negotiated media stream. It
    /// does not mean the connection has been finally answered.
    Progress {
        status_code: u16,
        early_media: bool,
    },
    /// Coalesced proof that core consumed media from this exact live
    /// Connection. `generation` is consecutive for the Connection lifecycle
    /// even when lower-level graph observations were overwritten under
    /// backpressure.
    MediaActivity {
        generation: u64,
    },
    Ended {
        reason: OperationalEndReason,
    },
    Failed {
        reason: OperationalFailureReason,
    },
    Dtmf {
        digits: String,
        duration_ms: u32,
    },
    DataMessage {
        message: DataMessage,
    },
    Transfer {
        /// Exact transfer submission when the caller used the correlated API.
        /// `None` identifies the legacy uncorrelated submission path.
        attempt_id: Option<TransferAttemptId>,
        target: OperationalTransferTarget,
        outcome: OperationalTransferOutcome,
    },
    /// Protocol-authoritative asynchronous transfer progress or completion.
    TransferStatus {
        attempt_id: Option<TransferAttemptId>,
        status: TransferStatus,
    },
}

impl fmt::Debug for OperationalEventKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connected => formatter.write_str("Connected"),
            Self::Progress {
                status_code,
                early_media,
            } => formatter
                .debug_struct("Progress")
                .field("status_code", status_code)
                .field("early_media", early_media)
                .finish(),
            Self::MediaActivity { generation } => formatter
                .debug_struct("MediaActivity")
                .field("generation", generation)
                .finish(),
            Self::Ended { reason } => formatter
                .debug_struct("Ended")
                .field("reason", reason)
                .finish(),
            Self::Failed { reason } => formatter
                .debug_struct("Failed")
                .field("reason", reason)
                .finish(),
            Self::Dtmf { duration_ms, .. } => formatter
                .debug_struct("Dtmf")
                .field("digits", &"[redacted]")
                .field("duration_ms", duration_ms)
                .finish(),
            Self::DataMessage { message } => formatter
                .debug_struct("DataMessage")
                .field("label", &"[redacted]")
                .field("content_type", &"[redacted]")
                .field("body_bytes", &message.bytes.len())
                .field("message_id", &"[redacted]")
                .field("reliability", &message.reliability)
                .finish(),
            Self::Transfer {
                attempt_id,
                target,
                outcome,
            } => formatter
                .debug_struct("Transfer")
                .field("attempt_id_present", &attempt_id.is_some())
                .field("target", &RedactedTransferTarget(target))
                .field("outcome", outcome)
                .finish(),
            Self::TransferStatus { attempt_id, status } => formatter
                .debug_struct("TransferStatus")
                .field("attempt_id_present", &attempt_id.is_some())
                .field("status", status)
                .finish(),
        }
    }
}

struct RedactedTransferTarget<'a>(&'a OperationalTransferTarget);

impl fmt::Debug for RedactedTransferTarget<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            OperationalTransferTarget::Uri => formatter.write_str("Uri([redacted])"),
            OperationalTransferTarget::Connection(_) => {
                formatter.write_str("Connection([redacted])")
            }
            OperationalTransferTarget::Session(_) => formatter.write_str("Session([redacted])"),
        }
    }
}

/// One globally ordered operational event.
#[derive(Clone, Eq, PartialEq)]
#[non_exhaustive]
pub struct OperationalEvent {
    pub sequence: u64,
    pub connection_id: ConnectionId,
    pub transport: Transport,
    pub at: DateTime<Utc>,
    pub kind: OperationalEventKind,
}

impl fmt::Debug for OperationalEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OperationalEvent")
            .field("sequence", &self.sequence)
            .field("connection_id", &self.connection_id)
            .field("transport", &self.transport)
            .field("at", &self.at)
            .field("kind", &self.kind)
            .finish()
    }
}

/// Core-owned half of the installed stream.
pub(crate) struct OperationalEventStream {
    sender: mpsc::Sender<OperationalEvent>,
    next_sequence: AtomicU64,
    health: Arc<OperationalEventStreamHealthState>,
}

struct OperationalEventStreamHealthState {
    degraded: AtomicBool,
    updates: watch::Sender<OperationalEventStreamHealth>,
}

pub(crate) struct OperationalSendGuard<'a> {
    stream: &'a OperationalEventStream,
    armed: bool,
}

impl OperationalSendGuard<'_> {
    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for OperationalSendGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.stream
                .mark_degraded(OperationalEventStreamFailure::DeliveryCancelled);
        }
    }
}

impl OperationalEventStreamHealthState {
    fn current(&self) -> OperationalEventStreamHealth {
        if self.degraded.load(Ordering::Acquire) {
            OperationalEventStreamHealth::Degraded
        } else {
            OperationalEventStreamHealth::Healthy
        }
    }

    fn mark_degraded(&self, failure: OperationalEventStreamFailure) {
        if !self.degraded.swap(true, Ordering::AcqRel) {
            self.updates
                .send_replace(OperationalEventStreamHealth::Degraded);
            metrics::counter!(
                "rvoip_core_operational_event_stream_failures_total",
                "reason" => failure.metric_label()
            )
            .increment(1);
            tracing::error!(
                reason = failure.metric_label(),
                "authoritative operational event stream degraded; failing closed"
            );
        }
    }
}

impl OperationalEventStream {
    pub(crate) fn new(capacity: usize) -> (Self, mpsc::Receiver<OperationalEvent>) {
        let (sender, receiver) = mpsc::channel(capacity);
        let (health_updates, _initial_health) =
            watch::channel(OperationalEventStreamHealth::Healthy);
        (
            Self {
                sender,
                next_sequence: AtomicU64::new(1),
                health: Arc::new(OperationalEventStreamHealthState {
                    degraded: AtomicBool::new(false),
                    updates: health_updates,
                }),
            },
            receiver,
        )
    }

    pub(crate) fn health(&self) -> OperationalEventStreamHealth {
        if self.sender.is_closed() {
            self.mark_degraded(OperationalEventStreamFailure::ReceiverLost);
        }
        self.health.current()
    }

    pub(crate) fn subscribe_health(&self) -> OperationalEventStreamHealthSubscription {
        // Detect a receiver that disappeared before this subscriber was
        // installed. `send_replace` retains the transition, so the returned
        // subscription always exposes the current sticky value.
        let _ = self.health();
        OperationalEventStreamHealthSubscription {
            updates: self.health.updates.subscribe(),
            receiver_closed: self.sender.clone(),
            health: Arc::clone(&self.health),
        }
    }

    /// Arm a cancellation boundary before core starts mutating lifecycle
    /// state whose authoritative outcome must subsequently be published.
    ///
    /// Callers disarm only after publication (or after proving that no
    /// authoritative event was owed). Dropping an armed guard permanently
    /// degrades the stream so cancellation cannot erase a lifecycle outcome
    /// while leaving the process apparently healthy.
    pub(crate) fn delivery_guard(&self) -> OperationalSendGuard<'_> {
        OperationalSendGuard {
            stream: self,
            armed: true,
        }
    }

    pub(crate) async fn send(
        &self,
        connection_id: ConnectionId,
        transport: Transport,
        at: DateTime<Utc>,
        kind: OperationalEventKind,
    ) -> bool {
        if self.health() == OperationalEventStreamHealth::Degraded {
            return false;
        }
        // If the owning adapter task is cancelled while awaiting receiver
        // capacity, an authoritative event may have been accepted by core but
        // not delivered. Make that loss observable and fail closed instead of
        // continuing with a falsely healthy stream.
        let mut cancellation_guard = self.delivery_guard();
        // Reserve bounded capacity before assigning the global sequence. A
        // cancelled waiter therefore cannot create a visible sequence gap.
        let permit = match self.sender.reserve().await {
            Ok(permit) => permit,
            Err(_) => {
                self.mark_degraded(OperationalEventStreamFailure::SendFailed);
                return false;
            }
        };
        let Ok(sequence) =
            self.next_sequence
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                    current.checked_add(1)
                })
        else {
            self.mark_degraded(OperationalEventStreamFailure::SequenceExhausted);
            return false;
        };
        permit.send(OperationalEvent {
            sequence,
            connection_id,
            transport,
            at,
            kind,
        });
        cancellation_guard.disarm();
        true
    }

    pub(crate) fn mark_degraded(&self, failure: OperationalEventStreamFailure) {
        self.health.mark_degraded(failure);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::MessageId;
    use bytes::Bytes;
    use rvoip_core_traits::data::DataReliability;

    #[test]
    fn debug_redacts_operational_payloads() {
        let secret = "credential-like-secret";
        let message = DataMessage {
            label: secret.into(),
            content_type: "application/secret".into(),
            bytes: Bytes::from(secret),
            reliability: DataReliability::ReliableOrdered,
            message_id: MessageId::from_string(secret),
        };
        let values = [
            OperationalEventKind::Dtmf {
                digits: secret.into(),
                duration_ms: 100,
            },
            OperationalEventKind::DataMessage { message },
            OperationalEventKind::Transfer {
                attempt_id: Some(TransferAttemptId::from_string(secret)),
                target: OperationalTransferTarget::Uri,
                outcome: OperationalTransferOutcome::Succeeded,
            },
            OperationalEventKind::TransferStatus {
                attempt_id: Some(TransferAttemptId::from_string(secret)),
                status: TransferStatus::Failed {
                    status_code: 503,
                    reason: secret.into(),
                },
            },
        ];
        for value in values {
            let debug = format!("{value:?}");
            assert!(!debug.contains(secret));
            assert!(debug.contains("[redacted]"));
        }
    }

    #[tokio::test]
    async fn cancelled_backpressured_send_marks_stream_degraded() {
        let (stream, _receiver) = OperationalEventStream::new(1);
        let stream = std::sync::Arc::new(stream);
        let mut health = stream.subscribe_health();
        assert_eq!(
            health.current(),
            OperationalEventStreamHealth::Healthy,
            "a new subscription exposes current health immediately"
        );
        assert!(
            stream
                .send(
                    ConnectionId::new(),
                    Transport::Sip,
                    Utc::now(),
                    OperationalEventKind::Connected,
                )
                .await
        );
        let blocked_stream = stream.clone();
        let blocked = tokio::spawn(async move {
            blocked_stream
                .send(
                    ConnectionId::new(),
                    Transport::Sip,
                    Utc::now(),
                    OperationalEventKind::Connected,
                )
                .await
        });
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert_eq!(stream.sender.capacity(), 0);
        assert!(!blocked.is_finished());
        blocked.abort();
        assert!(blocked.await.unwrap_err().is_cancelled());
        let changed = tokio::time::timeout(std::time::Duration::from_secs(1), health.changed())
            .await
            .expect("cancellation publishes a health transition");
        assert_eq!(changed, OperationalEventStreamHealth::Degraded);
        assert_eq!(stream.health(), OperationalEventStreamHealth::Degraded);
        assert_eq!(
            stream.subscribe_health().current(),
            OperationalEventStreamHealth::Degraded,
            "degradation is retained for later subscribers"
        );
    }

    #[tokio::test]
    async fn cancelled_lifecycle_after_mutation_before_send_marks_stream_degraded() {
        let (stream, _receiver) = OperationalEventStream::new(1);
        let stream = std::sync::Arc::new(stream);
        let (armed, armed_rx) = tokio::sync::oneshot::channel();
        let (_release, release_rx) = tokio::sync::oneshot::channel::<()>();
        let guarded_stream = std::sync::Arc::clone(&stream);
        let lifecycle = tokio::spawn(async move {
            let _delivery = guarded_stream.delivery_guard();
            let _ = armed.send(());
            // Deterministic stand-in for owned media/session teardown after
            // the connection registry has already been retired but before
            // the authoritative terminal send starts.
            let _ = release_rx.await;
        });
        armed_rx.await.unwrap();
        lifecycle.abort();
        assert!(lifecycle.await.unwrap_err().is_cancelled());
        assert_eq!(stream.health(), OperationalEventStreamHealth::Degraded);
    }

    #[tokio::test]
    async fn sequence_exhaustion_notifies_health_subscribers() {
        let (stream, _receiver) = OperationalEventStream::new(1);
        let mut health = stream.subscribe_health();
        stream.next_sequence.store(u64::MAX, Ordering::Release);

        assert!(
            !stream
                .send(
                    ConnectionId::new(),
                    Transport::Sip,
                    Utc::now(),
                    OperationalEventKind::Connected,
                )
                .await
        );
        let changed = tokio::time::timeout(std::time::Duration::from_secs(1), health.changed())
            .await
            .expect("sequence exhaustion publishes a health transition");
        assert_eq!(changed, OperationalEventStreamHealth::Degraded);
    }

    #[tokio::test]
    async fn failed_bounded_send_notifies_health_subscribers() {
        let (stream, mut receiver) = OperationalEventStream::new(1);
        let stream = Arc::new(stream);
        let mut health = stream.subscribe_health();
        assert!(
            stream
                .send(
                    ConnectionId::new(),
                    Transport::Sip,
                    Utc::now(),
                    OperationalEventKind::Connected,
                )
                .await
        );

        let blocked_stream = Arc::clone(&stream);
        let blocked = tokio::spawn(async move {
            blocked_stream
                .send(
                    ConnectionId::new(),
                    Transport::Sip,
                    Utc::now(),
                    OperationalEventKind::Connected,
                )
                .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while stream.sender.capacity() != 0 || blocked.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("second send waits for bounded capacity");

        receiver.close();
        assert!(!blocked.await.expect("sender task completed"));
        let changed = tokio::time::timeout(std::time::Duration::from_secs(1), health.changed())
            .await
            .expect("send failure publishes a health transition");
        assert_eq!(changed, OperationalEventStreamHealth::Degraded);
    }
}
