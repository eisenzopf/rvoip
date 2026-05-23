//! `OrchestratorSubscriptionHandler` — the production concrete
//! [`SubscriptionHandler`] implementation backed by `rvoip-core`'s
//! `Orchestrator` and `PublisherRegistry`.
//!
//! Adapters (rvoip-quic, rvoip-webtransport, …) build one of these at
//! connection-acceptance time and hand it to
//! [`UctpCoordinator::start_full`]. The handler:
//!
//! 1. On `stream.subscribe`: resolves each subscription's `strm_id`
//!    against the publisher registry; calls
//!    [`Orchestrator::add_subscription`] for each resolved row.
//! 2. On `stream.unsubscribe`: calls
//!    [`Orchestrator::remove_subscription`] for each `strm_id`. The
//!    spec mandates idempotent semantics (§7.7) so unknown strm_ids
//!    are silently treated as successful no-ops.
//!
//! `from_participant` and `kinds`-filtered subscriptions are
//! recognized but rejected with `404 from-participant-resolution-not-implemented`
//! until v0.x MP2.5 lands Session-aware Participant tracking.

use std::sync::Arc;

use rvoip_core::ids::{ConnectionId, SessionId, StreamId};
use rvoip_core::subscriptions::{PublisherEntry, PublisherRegistry};
use rvoip_core::Orchestrator;

use crate::payloads::stream::{StreamSubscribe, StreamUnsubscribe};

use super::subscription::{PublisherInfo, SubscriptionHandler, SubscriptionOutcome};

/// Production handler that routes inbound subscribe/unsubscribe envelopes
/// through `rvoip-core::Orchestrator`'s subscription registry.
pub struct OrchestratorSubscriptionHandler {
    orch: Arc<Orchestrator>,
    publishers: Arc<PublisherRegistry>,
}

impl OrchestratorSubscriptionHandler {
    pub fn new(orch: Arc<Orchestrator>, publishers: Arc<PublisherRegistry>) -> Arc<Self> {
        Arc::new(Self { orch, publishers })
    }

    /// Borrow the publisher registry so adapters can call
    /// `register(sid, strm_id, publisher_connid)` when their connection
    /// emits `stream.opened`. Cleanup on connection-end is handled by
    /// the orchestrator's `forget_connection` path.
    pub fn publishers(&self) -> Arc<PublisherRegistry> {
        Arc::clone(&self.publishers)
    }
}

impl SubscriptionHandler for OrchestratorSubscriptionHandler {
    fn subscribe(
        &self,
        sid: &SessionId,
        subscriber: &ConnectionId,
        request: &StreamSubscribe,
    ) -> SubscriptionOutcome {
        // Empty subscription list — treat as a successful no-op so
        // clients can probe the surface without ill effect.
        if request.subscriptions.is_empty() {
            return SubscriptionOutcome::Ok;
        }

        // Walk subscriptions. Three shapes are possible per
        // §7.7 — strm_id, from_participant, from_participant+kinds.
        for sub in &request.subscriptions {
            match (&sub.strm_id, &sub.from_participant) {
                (Some(strm_id), _) => {
                    let Some(publisher) = self.publishers.publisher(sid, strm_id) else {
                        return SubscriptionOutcome::reject(
                            404,
                            format!("unknown strm_id: {strm_id}"),
                        );
                    };
                    self.orch.add_subscription(
                        sid.clone(),
                        subscriber.clone(),
                        publisher,
                        StreamId::from_string(strm_id.clone()),
                    );
                }
                (None, Some(participant)) => {
                    // MP2.5: resolve all of `participant`'s streams in
                    // `sid` via the publisher registry's secondary
                    // index. Empty result → 404; the subscriber asked
                    // about a Participant we have no streams for.
                    let strm_ids = self.publishers.streams_for_participant(sid, participant);
                    if strm_ids.is_empty() {
                        return SubscriptionOutcome::reject(
                            404,
                            format!("unknown participant or no streams: {participant}"),
                        );
                    }
                    // Optional `kinds` filter — when present, only
                    // subscribe to streams whose kind is in the list.
                    // When absent, subscribe to every stream the
                    // participant publishes.
                    let kinds_filter: Option<&[String]> = if sub.kinds.is_empty() {
                        None
                    } else {
                        Some(sub.kinds.as_slice())
                    };
                    let mut matched_any = false;
                    for strm_id in strm_ids {
                        // Look up the publisher entry to get the kind
                        // for the filter check + the actual ConnectionId.
                        let Some(entry) = self.publishers.entry(sid, &strm_id) else {
                            // Stream disappeared between participant
                            // lookup and entry lookup — skip silently.
                            continue;
                        };
                        if let Some(filter) = kinds_filter {
                            if !filter.iter().any(|k| k == &entry.kind) {
                                continue;
                            }
                        }
                        self.orch.add_subscription(
                            sid.clone(),
                            subscriber.clone(),
                            entry.connection,
                            StreamId::from_string(strm_id),
                        );
                        matched_any = true;
                    }
                    if !matched_any {
                        // Participant has streams but none match the
                        // kinds filter. Per spec §7.7 this is a 488
                        // (incompatible capabilities) rather than 404,
                        // because the participant exists.
                        return SubscriptionOutcome::reject(
                            488,
                            format!(
                                "no streams from {participant} match the requested kinds filter"
                            ),
                        );
                    }
                }
                (None, None) => {
                    return SubscriptionOutcome::reject(
                        400,
                        "subscription must specify strm_id or from_participant",
                    );
                }
            }
        }

        SubscriptionOutcome::Ok
    }

    fn unsubscribe(
        &self,
        sid: &SessionId,
        subscriber: &ConnectionId,
        request: &StreamUnsubscribe,
    ) -> SubscriptionOutcome {
        // Idempotent per §7.7. For each strm_id, look up the publisher
        // (best-effort — unknown strm_ids silently succeed) and remove
        // the subscriber from that row.
        for strm_id in &request.strm_ids {
            let strm = StreamId::from_string(strm_id.clone());
            if let Some(publisher) = self.publishers.publisher(sid, strm_id) {
                let _ = self
                    .orch
                    .remove_subscription(sid, subscriber, &publisher, &strm);
            }
        }
        SubscriptionOutcome::Ok
    }

    fn register_publisher(&self, info: PublisherInfo<'_>) {
        self.publishers.register(
            info.sid.clone(),
            info.strm_id.to_string(),
            PublisherEntry {
                connection: info.connection.clone(),
                participant: info.participant.to_string(),
                kind: info.kind.to_string(),
            },
        );
    }

    fn unregister_publisher(&self, sid: &SessionId, strm_id: &str) {
        // PublisherRegistry has no single-strm_id removal yet (drop_publisher
        // and drop_session are the available primitives — both broader).
        // Future MP3+ may want a fine-grained removal; for now we leave
        // single-stream entries until the publisher's connection ends or
        // the session ends, at which point the broader cleanup hooks
        // fire from `Orchestrator::forget_connection`.
        let _ = (sid, strm_id);
    }
}
