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
/// stream), 488 (capability mismatch), 501 (recognized but not wired
/// in this build), 503 (transient capacity / not-ready).
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
/// `None` handler keeps the legacy 501 reject for back-compat.
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

    /// Drop every publisher/subscriber resource owned by a Connection.
    /// Called on explicit end, transport loss, expiry, and coordinator drain.
    fn unregister_connection(&self, _sid: &SessionId, _connid: &ConnectionId) {}
}

/// Peer-local namespace wrapper for a shared production handler. Wire Session
/// and Connection IDs are supplied by the remote peer, so they must not be
/// used as process-global registry keys without an authenticated peer scope.
pub struct NamespacedSubscriptionHandler {
    namespace: String,
    inner: Arc<dyn SubscriptionHandler>,
}

impl NamespacedSubscriptionHandler {
    pub fn new(namespace: impl Into<String>, inner: Arc<dyn SubscriptionHandler>) -> Arc<Self> {
        Arc::new(Self {
            namespace: namespace.into(),
            inner,
        })
    }

    fn session(&self, sid: &SessionId) -> SessionId {
        SessionId::from_string(format!("{}:{}", self.namespace, sid))
    }

    fn connection(&self, connid: &ConnectionId) -> ConnectionId {
        ConnectionId::from_string(format!("{}:{}", self.namespace, connid))
    }
}

impl SubscriptionHandler for NamespacedSubscriptionHandler {
    fn subscribe(
        &self,
        sid: &SessionId,
        subscriber: &ConnectionId,
        request: &StreamSubscribe,
    ) -> SubscriptionOutcome {
        self.inner
            .subscribe(&self.session(sid), &self.connection(subscriber), request)
    }

    fn unsubscribe(
        &self,
        sid: &SessionId,
        subscriber: &ConnectionId,
        request: &StreamUnsubscribe,
    ) -> SubscriptionOutcome {
        self.inner
            .unsubscribe(&self.session(sid), &self.connection(subscriber), request)
    }

    fn register_publisher(&self, info: PublisherInfo<'_>) {
        let sid = self.session(info.sid);
        let connection = self.connection(info.connection);
        self.inner.register_publisher(PublisherInfo {
            sid: &sid,
            strm_id: info.strm_id,
            connection: &connection,
            participant: info.participant,
            kind: info.kind,
            codec: info.codec,
        });
    }

    fn unregister_publisher(&self, sid: &SessionId, strm_id: &str) {
        self.inner.unregister_publisher(&self.session(sid), strm_id);
    }

    fn unregister_connection(&self, sid: &SessionId, connid: &ConnectionId) {
        self.inner
            .unregister_connection(&self.session(sid), &self.connection(connid));
    }
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

/// Default handler — every request is rejected with `501 not-implemented`
/// (`multi-party-routing-not-implemented`). The receiver recognized the
/// envelope type but lacks the wiring to service it; another build of
/// the same server might. Used when no handler is configured.
///
/// Pre-v0.x servers conflated `501` and `501` as `501`; per
/// `CONVERSATION_PROTOCOL.md` §11.2 these are now distinct.
pub struct RejectingHandler;

impl SubscriptionHandler for RejectingHandler {
    fn subscribe(
        &self,
        _: &SessionId,
        _: &ConnectionId,
        _: &StreamSubscribe,
    ) -> SubscriptionOutcome {
        SubscriptionOutcome::reject(501, "multi-party-routing-not-implemented")
    }

    fn unsubscribe(
        &self,
        _: &SessionId,
        _: &ConnectionId,
        _: &StreamUnsubscribe,
    ) -> SubscriptionOutcome {
        SubscriptionOutcome::reject(501, "multi-party-routing-not-implemented")
    }
}

/// Convenience: wrap the default rejecting handler in an `Arc`.
pub fn rejecting_handler() -> Arc<dyn SubscriptionHandler> {
    Arc::new(RejectingHandler)
}

#[cfg(test)]
mod namespace_tests {
    use super::*;

    #[test]
    fn identical_wire_ids_from_two_peers_map_to_distinct_registry_ids() {
        let peer_a = NamespacedSubscriptionHandler::new("peer-a", rejecting_handler());
        let peer_b = NamespacedSubscriptionHandler::new("peer-b", rejecting_handler());
        let sid = SessionId::from_string("shared-sid");
        let connid = ConnectionId::from_string("shared-connid");

        assert_ne!(peer_a.session(&sid), peer_b.session(&sid));
        assert_ne!(peer_a.connection(&connid), peer_b.connection(&connid));
    }
}
