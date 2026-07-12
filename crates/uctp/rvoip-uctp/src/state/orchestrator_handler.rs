//! `OrchestratorSubscriptionHandler` — the production concrete
//! [`SubscriptionHandler`] implementation backed by `rvoip-core`'s
//! `Orchestrator` and `PublisherRegistry`.
//!
//! Adapters (rvoip-quic, rvoip-webtransport, …) build one of these at
//! connection-acceptance time and hand it to
//! `UctpCoordinator::start_full`. The handler resolves all three
//! §7.7 subscription forms:
//!
//! 1. **Explicit `strm_id`**: looked up against the publisher
//!    registry; one [`Orchestrator::add_subscription`] call.
//! 2. **`from_participant`** (optionally with a `kinds` filter):
//!    resolved via the registry's `(SessionId, ParticipantId) →
//!    Vec<strm_id>` index (landed in MP2.5); one `add_subscription`
//!    per matching stream, with codec-gate filtering per §13.3 (B2).
//! 3. **`stream.unsubscribe`**: calls
//!    [`Orchestrator::remove_subscription`] for each `strm_id`. The
//!    spec mandates idempotent semantics (§7.7) so unknown strm_ids
//!    are silently treated as successful no-ops.

use std::collections::HashSet;
use std::sync::Arc;

use rvoip_core::ids::{ConnectionId, SessionId, StreamId};
use rvoip_core::subscriptions::{PublisherEntry, PublisherRegistry};
use rvoip_core::Orchestrator;

use crate::payloads::stream::{StreamSubscribe, StreamUnsubscribe};

use super::subscription::{PublisherInfo, SubscriptionHandler, SubscriptionOutcome};

/// Default set of audio codecs the orchestrator will fan out to
/// subscribers (plan B2). Restricted to the codecs UCTP-family
/// subscribers actually have decoders for in v0 — anything outside
/// this set means we'd be sending the subscriber bytes it can't play.
/// Tune via [`OrchestratorSubscriptionHandler::with_accepted_codecs`]
/// for deployments with custom decoder pipelines.
pub const DEFAULT_ACCEPTED_CODECS: &[&str] = &["opus", "g.711-mu", "g.711-a", "g.722", "g.729"];

/// Production handler that routes inbound subscribe/unsubscribe envelopes
/// through `rvoip-core::Orchestrator`'s subscription registry.
pub struct OrchestratorSubscriptionHandler {
    orch: Arc<Orchestrator>,
    publishers: Arc<PublisherRegistry>,
    /// Audio codecs the orchestrator will accept on subscribe (plan B2).
    /// Publishers with codecs outside this set get their subscriptions
    /// refused with `error 488`. Defaults to
    /// [`DEFAULT_ACCEPTED_CODECS`]; deployments override via
    /// [`Self::with_accepted_codecs`].
    accepted_codecs: HashSet<String>,
}

impl OrchestratorSubscriptionHandler {
    pub fn new(orch: Arc<Orchestrator>, publishers: Arc<PublisherRegistry>) -> Arc<Self> {
        Arc::new(Self {
            orch,
            publishers,
            accepted_codecs: DEFAULT_ACCEPTED_CODECS
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
        })
    }

    /// Variant of [`Self::new`] with an explicit accepted-codecs set.
    /// Empty set disables the check (accept everything — useful for
    /// experimental codecs that don't appear in
    /// [`DEFAULT_ACCEPTED_CODECS`] yet).
    pub fn with_accepted_codecs<I, S>(
        orch: Arc<Orchestrator>,
        publishers: Arc<PublisherRegistry>,
        codecs: I,
    ) -> Arc<Self>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Arc::new(Self {
            orch,
            publishers,
            accepted_codecs: codecs.into_iter().map(|s| s.into()).collect(),
        })
    }

    /// Borrow the publisher registry so adapters can call
    /// `register(sid, strm_id, publisher_connid)` when their connection
    /// emits `stream.opened`. Cleanup on connection-end is handled by
    /// the orchestrator's `forget_connection` path.
    pub fn publishers(&self) -> Arc<PublisherRegistry> {
        Arc::clone(&self.publishers)
    }

    /// Returns `true` if the publisher's recorded codec is acceptable
    /// for fanout. Publishers with `codec: None` get the benefit of
    /// the doubt (older test paths, or stream kinds where codec
    /// negotiation doesn't apply). Empty `accepted_codecs` disables
    /// the check entirely.
    fn codec_acceptable(&self, entry: &PublisherEntry) -> bool {
        if self.accepted_codecs.is_empty() {
            return true;
        }
        match &entry.codec {
            None => true,
            Some(codec) => self.accepted_codecs.contains(&codec.name),
        }
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
                    let Some(entry) = self.publishers.entry(sid, strm_id) else {
                        return SubscriptionOutcome::reject(
                            404,
                            format!("unknown strm_id: {strm_id}"),
                        );
                    };
                    // B2: codec gate. If the publisher's negotiated
                    // codec isn't in the orchestrator's accepted set,
                    // refuse — the subscriber would otherwise receive
                    // frames it can't decode. Publishers with no
                    // codec recorded (legacy paths) get the benefit
                    // of the doubt.
                    if !self.codec_acceptable(&entry) {
                        let codec_name = entry
                            .codec
                            .as_ref()
                            .map(|c| c.name.as_str())
                            .unwrap_or("unknown");
                        return SubscriptionOutcome::reject(
                            488,
                            format!(
                                "unsupported codec for fanout: strm_id={strm_id} codec={codec_name}"
                            ),
                        );
                    }
                    self.orch.add_subscription(
                        sid.clone(),
                        subscriber.clone(),
                        entry.connection,
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
                        // B2: silently skip streams with unsupported
                        // codecs. Unlike the strm_id case (which
                        // refuses 488), from_participant is a
                        // best-effort enumeration — subscribing to
                        // "alice's streams" shouldn't fail entirely
                        // because one of her streams uses an exotic
                        // codec. If NO streams pass both filters,
                        // we fall through to the same 488 path the
                        // kinds-mismatch case uses.
                        if !self.codec_acceptable(&entry) {
                            continue;
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
                        // kinds filter (or codec filter). Per spec §7.7
                        // this is a 488 (incompatible capabilities)
                        // rather than 404, because the participant
                        // exists.
                        return SubscriptionOutcome::reject(
                            488,
                            format!(
                                "no streams from {participant} match the requested kinds/codec filters"
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
                codec: info.codec.clone(),
            },
        );
    }

    fn unregister_publisher(&self, sid: &SessionId, strm_id: &str, publisher: &ConnectionId) {
        self.publishers
            .remove_stream_if_publisher(sid, strm_id, publisher);
    }

    fn unregister_connection(&self, _sid: &SessionId, connid: &ConnectionId) {
        self.orch.drop_connection_subscriptions(connid);
        self.publishers.drop_publisher(connid);
    }
}

#[cfg(test)]
mod cleanup_tests {
    use super::*;
    use rvoip_core::Config;

    #[test]
    fn unregister_connection_clears_publishers_and_subscription_rows() {
        let orchestrator = Orchestrator::new(Config::default());
        let publishers = Arc::new(PublisherRegistry::new());
        let handler = OrchestratorSubscriptionHandler::new(
            Arc::clone(&orchestrator),
            Arc::clone(&publishers),
        );
        let sid = SessionId::from_string("session-cleanup");
        let publisher = ConnectionId::from_string("publisher-cleanup");
        let subscriber = ConnectionId::from_string("subscriber-cleanup");
        let stream = StreamId::from_string("stream-cleanup");
        publishers.register(
            sid.clone(),
            stream.to_string(),
            PublisherEntry {
                connection: publisher.clone(),
                participant: "alice".to_string(),
                kind: "audio".to_string(),
                codec: None,
            },
        );
        orchestrator.add_subscription(sid.clone(), subscriber, publisher.clone(), stream.clone());

        handler.unregister_connection(&sid, &publisher);

        assert!(publishers.entry(&sid, stream.as_str()).is_none());
        assert!(orchestrator
            .subscribers_for(&sid, &publisher, &stream)
            .is_empty());
    }

    #[test]
    fn stale_stream_cleanup_cannot_remove_another_publishers_replacement() {
        let orchestrator = Orchestrator::new(Config::default());
        let publishers = Arc::new(PublisherRegistry::new());
        let handler = OrchestratorSubscriptionHandler::new(
            Arc::clone(&orchestrator),
            Arc::clone(&publishers),
        );
        let sid = SessionId::from_string("session-replacement");
        let old_publisher = ConnectionId::from_string("publisher-old");
        let replacement = ConnectionId::from_string("publisher-replacement");
        publishers.register(
            sid.clone(),
            "audio/main".to_string(),
            PublisherEntry {
                connection: old_publisher.clone(),
                participant: "old".to_string(),
                kind: "audio".to_string(),
                codec: None,
            },
        );
        publishers.register(
            sid.clone(),
            "audio/main".to_string(),
            PublisherEntry {
                connection: replacement.clone(),
                participant: "replacement".to_string(),
                kind: "audio".to_string(),
                codec: None,
            },
        );

        handler.unregister_publisher(&sid, "audio/main", &old_publisher);

        assert_eq!(publishers.publisher(&sid, "audio/main"), Some(replacement));
    }
}
