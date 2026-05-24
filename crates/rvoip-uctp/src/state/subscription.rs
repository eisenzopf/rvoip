//! `SubscriptionHandler` trait — the coordinator's escape hatch into
//! whatever multi-party-routing implementation the deployment provides.
//!
//! Architecture (v0.x MP1/MP2/MP3 sequencing):
//!
//! - **MP1** (done): orchestrator stores per-Session subscription rows.
//! - **MP2** (this file): coordinator decodes `stream.subscribe` /
//!   `stream.unsubscribe` envelopes and routes them through a
//!   `SubscriptionHandler`. The concrete implementation lives in
//!   `rvoip-core` (so it can hold `Arc<Orchestrator>`); the trait
//!   stays here to keep `rvoip-uctp` substrate-agnostic.
//! - **MP3** (future): adapter media path consults
//!   `orchestrator.subscribers_for(...)` to fan datagrams out.
//!
//! The trait deliberately takes the parsed payload structs directly so
//! implementations don't have to re-decode the JSON. Wire-format
//! changes flow through the payload types, not through this trait.

use std::sync::Arc;

use crate::ids::{ConnectionId, SessionId};
use crate::payloads::stream::{StreamSubscribe, StreamUnsubscribe};

/// Outcome of a `stream.subscribe` request.
///
/// `Ok` → coordinator emits `ack` in_reply_to the request envelope.
/// `Reject{code, reason}` → coordinator emits `error` with that code
/// and reason, also in_reply_to. Codes follow the
/// CONVERSATION_PROTOCOL.md §11.2 catalog: 404 (unknown participant /
/// stream), 488 (capability mismatch), 503 (capacity / not-ready).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubscriptionOutcome {
    Ok,
    Reject { code: u16, reason: String },
}

impl SubscriptionOutcome {
    pub fn ok() -> Self {
        Self::Ok
    }

    pub fn reject(code: u16, reason: impl Into<String>) -> Self {
        Self::Reject {
            code,
            reason: reason.into(),
        }
    }
}

/// Plug-in trait implemented by whatever owns the multi-party routing
/// table. The UCTP coordinator calls into this on inbound
/// `stream.subscribe` / `stream.unsubscribe` envelopes. The default
/// `None` handler keeps the legacy 503 reject for back-compat.
///
/// Implementations are typically not blocking, so the trait is sync
/// (no `async fn`). If a future impl needs to block, switch the trait
/// to `async-trait` — the coordinator already awaits the result.
pub trait SubscriptionHandler: Send + Sync {
    /// Handle a `stream.subscribe` envelope. The subscriber is the
    /// peer Connection that sent the envelope; the SessionId is taken
    /// from `env.sid`.
    fn subscribe(
        &self,
        sid: &SessionId,
        subscriber: &ConnectionId,
        request: &StreamSubscribe,
    ) -> SubscriptionOutcome;

    /// Handle a `stream.unsubscribe` envelope. Idempotent — removing a
    /// subscription that doesn't exist must succeed.
    fn unsubscribe(
        &self,
        sid: &SessionId,
        subscriber: &ConnectionId,
        request: &StreamUnsubscribe,
    ) -> SubscriptionOutcome;

    /// Announce that a Stream is available for subscription. The
    /// coordinator calls this once per Stream when it emits
    /// `stream.opened` (i.e. at `connection.ready` time, per
    /// CONVERSATION_PROTOCOL.md §7.4). Default impl is a no-op so
    /// [`RejectingHandler`] and similar don't have to opt in.
    ///
    /// `info` carries the publisher's `ConnectionId`, `participant`
    /// (Participant ID from `connection.offer.by_participant`), and
    /// `kind` (`"audio"` / `"video"` / `"data"`). MP2.5+ uses
    /// `participant` and `kind` to resolve `from_participant`-form
    /// and `kinds`-filtered subscriptions.
    fn register_publisher(&self, _info: PublisherInfo<'_>) {}

    /// Drop a publisher registration. The coordinator calls this when
    /// it emits `stream.closed` for one of its own streams. Default
    /// no-op.
    fn unregister_publisher(&self, _sid: &SessionId, _strm_id: &str) {}
}

/// Bundle passed to [`SubscriptionHandler::register_publisher`]. Carries
/// everything the orchestrator needs to resolve `strm_id` and
/// `from_participant` subscription forms; future fields land here
/// without breaking the trait surface.
pub struct PublisherInfo<'a> {
    pub sid: &'a SessionId,
    pub strm_id: &'a str,
    pub connection: &'a ConnectionId,
    pub participant: &'a str,
    pub kind: &'a str,
    /// The codec the publisher negotiated for this Stream (the chosen
    /// codec out of [`rvoip_core::capability::negotiate_streams`]'s
    /// answer). Propagated to the `PublisherRegistry` so
    /// [`rvoip_core::Orchestrator::fanout_frame`] can hand the right
    /// `CodecInfo` to the subscriber-side adapter when allocating a
    /// fresh per-subscription MediaStream (plan B1 / MP3c).
    pub codec: Option<rvoip_core::capability::CodecInfo>,
}

/// Default handler — every request is rejected with 503
/// `multi-party-routing-not-implemented`. Used when no handler is
/// configured so the legacy v0 behavior is preserved.
pub struct RejectingHandler;

impl SubscriptionHandler for RejectingHandler {
    fn subscribe(&self, _: &SessionId, _: &ConnectionId, _: &StreamSubscribe) -> SubscriptionOutcome {
        SubscriptionOutcome::reject(503, "multi-party-routing-not-implemented")
    }

    fn unsubscribe(&self, _: &SessionId, _: &ConnectionId, _: &StreamUnsubscribe) -> SubscriptionOutcome {
        SubscriptionOutcome::reject(503, "multi-party-routing-not-implemented")
    }
}

/// Convenience: wrap the default rejecting handler in an `Arc`.
pub fn rejecting_handler() -> Arc<dyn SubscriptionHandler> {
    Arc::new(RejectingHandler)
}
