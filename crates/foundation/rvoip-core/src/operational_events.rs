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
//! The pre-existing per-adapter normalization task created by
//! [`crate::Orchestrator::register`] remains process-lived until that
//! adapter's receiver closes. This module adds no forwarding task; explicit
//! adapter-task ownership and shutdown remain a separate lifecycle seam.

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use tokio::sync::mpsc;

use crate::adapter::{EndReason, TransferTarget};
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
        target: OperationalTransferTarget,
        outcome: OperationalTransferOutcome,
    },
}

impl fmt::Debug for OperationalEventKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connected => formatter.write_str("Connected"),
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
                target, outcome, ..
            } => formatter
                .debug_struct("Transfer")
                .field("target", &RedactedTransferTarget(target))
                .field("outcome", outcome)
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
    degraded: AtomicBool,
}

struct OperationalSendGuard<'a> {
    stream: &'a OperationalEventStream,
    armed: bool,
}

impl Drop for OperationalSendGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.stream.mark_degraded();
        }
    }
}

impl OperationalEventStream {
    pub(crate) fn new(capacity: usize) -> (Self, mpsc::Receiver<OperationalEvent>) {
        let (sender, receiver) = mpsc::channel(capacity);
        (
            Self {
                sender,
                next_sequence: AtomicU64::new(1),
                degraded: AtomicBool::new(false),
            },
            receiver,
        )
    }

    pub(crate) fn health(&self) -> OperationalEventStreamHealth {
        if self.sender.is_closed() {
            self.mark_degraded();
        }
        if self.degraded.load(Ordering::Acquire) {
            OperationalEventStreamHealth::Degraded
        } else {
            OperationalEventStreamHealth::Healthy
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
        let mut cancellation_guard = OperationalSendGuard {
            stream: self,
            armed: true,
        };
        // Reserve bounded capacity before assigning the global sequence. A
        // cancelled waiter therefore cannot create a visible sequence gap.
        let permit = match self.sender.reserve().await {
            Ok(permit) => permit,
            Err(_) => {
                self.mark_degraded();
                return false;
            }
        };
        let Ok(sequence) =
            self.next_sequence
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                    current.checked_add(1)
                })
        else {
            self.mark_degraded();
            return false;
        };
        permit.send(OperationalEvent {
            sequence,
            connection_id,
            transport,
            at,
            kind,
        });
        cancellation_guard.armed = false;
        true
    }

    fn mark_degraded(&self) {
        if !self.degraded.swap(true, Ordering::AcqRel) {
            metrics::counter!("rvoip_core_operational_event_stream_failures_total").increment(1);
            tracing::error!("authoritative operational event receiver unavailable; failing closed");
        }
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
                target: OperationalTransferTarget::Uri,
                outcome: OperationalTransferOutcome::Succeeded,
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
        assert_eq!(stream.health(), OperationalEventStreamHealth::Degraded);
    }
}
