//! Per-Session subscription routing table for N-Participant Sessions.
//!
//! Implements the data structure called out in INTERFACE_DESIGN.md §10.6
//! and CONVERSATION_PROTOCOL.md §7.7: for each `(publisher Connection,
//! Stream)` in a Session, the set of subscriber Connections that should
//! receive that Stream's media datagrams. Used by the orchestrator's
//! `add_subscription` / `remove_subscription` / `subscribers_for` surface;
//! the adapter media path will consult `subscribers_for` to fan out
//! datagrams once the wire-level coordinator handler lands (MP2).
//!
//! All operations are idempotent — the spec (§7.7) and the SDK API
//! design both rely on "subscribe what's already subscribed" being a
//! no-op, and "unsubscribe what's already gone" being a no-op too. That
//! lets cleanup paths (connection.end, session.end) be eager without
//! needing precise ordering.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use dashmap::{DashMap, DashSet};

use crate::ids::{ConnectionId, SessionId, StreamId};

/// Subscription routing table for one Session.
///
/// Key: `(publisher Connection, Stream)`. Value: set of subscriber
/// Connections. A subscriber receives every media datagram the
/// publisher emits on that Stream.
#[derive(Default)]
pub struct SessionSubscriptions {
    inner: DashMap<(ConnectionId, StreamId), Arc<DashSet<ConnectionId>>>,
}

impl SessionSubscriptions {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add `subscriber` to the set for `(publisher, strm_id)`.
    /// Idempotent — adding the same subscriber twice is a no-op.
    pub fn add(&self, publisher: ConnectionId, strm_id: StreamId, subscriber: ConnectionId) {
        let entry = self
            .inner
            .entry((publisher, strm_id))
            .or_insert_with(|| Arc::new(DashSet::new()))
            .clone();
        entry.insert(subscriber);
    }

    /// Remove `subscriber` from the set for `(publisher, strm_id)`.
    /// Idempotent. Returns `true` if the subscriber was actually
    /// present.
    pub fn remove(
        &self,
        publisher: &ConnectionId,
        strm_id: &StreamId,
        subscriber: &ConnectionId,
    ) -> bool {
        let key = (publisher.clone(), strm_id.clone());
        let removed = if let Some(entry) = self.inner.get(&key) {
            entry.remove(subscriber).is_some()
        } else {
            false
        };
        // Keep empty rows until Session teardown. A check-then-remove here
        // can delete a subscriber concurrently inserted into the same Arc.
        // Stream/session caps bound these rows for the Session lifetime.
        removed
    }

    /// Look up subscribers for `(publisher, strm_id)`. Returns a
    /// snapshot — callers that fan out datagrams iterate this without
    /// holding the table lock.
    pub fn subscribers_for(
        &self,
        publisher: &ConnectionId,
        strm_id: &StreamId,
    ) -> Vec<ConnectionId> {
        match self.inner.get(&(publisher.clone(), strm_id.clone())) {
            Some(set) => set.iter().map(|e| e.clone()).collect(),
            None => Vec::new(),
        }
    }

    /// Drop every subscription that names `connid` — either as
    /// publisher (the Stream went away) or as subscriber (the
    /// subscriber's Connection ended). Called from
    /// `crate::Orchestrator::forget_connection` so cleanup happens
    /// eagerly without requiring callers to track teardown order.
    pub fn drop_connection(&self, connid: &ConnectionId) {
        // Walk every (publisher, strm_id) entry. If the publisher is
        // this connid, remove the whole entry. Otherwise remove this
        // connid from the subscriber set.
        let keys: Vec<(ConnectionId, StreamId)> =
            self.inner.iter().map(|e| e.key().clone()).collect();
        for key in keys {
            if key.0 == *connid {
                self.inner.remove(&key);
                continue;
            }
            if let Some(set) = self.inner.get(&key) {
                set.remove(connid);
            }
            // Empty subscriber rows are reclaimed by `drop_session`; removing
            // them here races a concurrent subscription to the same row.
        }
    }

    /// Snapshot of every (publisher, strm_id, subscribers) row. Used
    /// by tests and operator diagnostics. Not for the fanout hot path.
    pub fn rows(&self) -> Vec<(ConnectionId, StreamId, Vec<ConnectionId>)> {
        self.inner
            .iter()
            .map(|e| {
                let (pub_id, strm) = e.key().clone();
                let subs = e.value().iter().map(|s| s.clone()).collect();
                (pub_id, strm, subs)
            })
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Drop one exact publisher Stream row and every subscriber it names.
    pub fn drop_publisher_stream(&self, publisher: &ConnectionId, stream: &StreamId) {
        self.inner.remove(&(publisher.clone(), stream.clone()));
    }
}

/// Workspace-level subscription registry — one [`SessionSubscriptions`]
/// per active `SessionId`. Lives on the [`crate::Orchestrator`].
pub struct SubscriptionRegistry {
    sessions: DashMap<SessionId, Arc<SessionSubscriptions>>,
    max_direct_subscribers: usize,
    direct: Mutex<DirectSubscriptionState>,
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct DirectSubscriptionRoute {
    sid: SessionId,
    publisher: ConnectionId,
    stream: StreamId,
}

#[derive(Default)]
struct DirectSubscriptionState {
    /// One row per physical direct subscriber Connection. Multiple stream
    /// routes on that Connection consume exactly one worker permit.
    listeners: HashMap<ConnectionId, HashSet<DirectSubscriptionRoute>>,
}

/// Atomic direct-listener admission failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectSubscriptionAdmissionError {
    /// Direct subscriptions were disabled by setting the worker limit to zero.
    Disabled,
    /// Admitting a new distinct subscriber would exceed the worker limit.
    CapacityExhausted,
}

impl fmt::Display for DirectSubscriptionAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Disabled => "direct subscriptions are disabled",
            Self::CapacityExhausted => "direct subscriber capacity is exhausted",
        })
    }
}

impl std::error::Error for DirectSubscriptionAdmissionError {}

impl Default for SubscriptionRegistry {
    fn default() -> Self {
        Self::with_direct_listener_limit(1_000)
    }
}

impl SubscriptionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_direct_listener_limit(max_direct_subscribers: usize) -> Self {
        Self {
            sessions: DashMap::new(),
            max_direct_subscribers,
            direct: Mutex::new(DirectSubscriptionState::default()),
        }
    }

    /// Configured process-wide ceiling for direct fanout listeners.
    pub fn direct_listener_limit(&self) -> usize {
        self.max_direct_subscribers
    }

    /// Get the per-Session table, creating it lazily on first use.
    pub fn for_session(&self, sid: &SessionId) -> Arc<SessionSubscriptions> {
        self.sessions
            .entry(sid.clone())
            .or_insert_with(|| Arc::new(SessionSubscriptions::new()))
            .clone()
    }

    /// Atomically admit every validated route in one subscribe request.
    ///
    /// Capacity is process-wide for this registry, not handler-local, so raw
    /// QUIC, WebTransport, and any other UCTP ingress sharing an Orchestrator
    /// cannot each admit up to the configured maximum. No route is installed
    /// when admission fails.
    pub fn try_add_direct(
        &self,
        sid: &SessionId,
        subscriber: &ConnectionId,
        routes: &[(ConnectionId, StreamId)],
    ) -> Result<(), DirectSubscriptionAdmissionError> {
        if routes.is_empty() {
            return Ok(());
        }
        if self.max_direct_subscribers == 0 {
            return Err(DirectSubscriptionAdmissionError::Disabled);
        }
        let mut direct = self
            .direct
            .lock()
            .expect("direct subscription lock poisoned");
        if !direct.listeners.contains_key(subscriber)
            && direct.listeners.len() >= self.max_direct_subscribers
        {
            return Err(DirectSubscriptionAdmissionError::CapacityExhausted);
        }

        let table = self.for_session(sid);
        let subscriber_routes = direct.listeners.entry(subscriber.clone()).or_default();
        for (publisher, stream) in routes {
            table.add(publisher.clone(), stream.clone(), subscriber.clone());
            subscriber_routes.insert(DirectSubscriptionRoute {
                sid: sid.clone(),
                publisher: publisher.clone(),
                stream: stream.clone(),
            });
        }
        Ok(())
    }

    /// Remove one direct route and release the worker permit only when this
    /// was the subscriber's final route.
    pub fn remove_direct(
        &self,
        sid: &SessionId,
        subscriber: &ConnectionId,
        publisher: &ConnectionId,
        stream: &StreamId,
    ) -> bool {
        let mut direct = self
            .direct
            .lock()
            .expect("direct subscription lock poisoned");
        let removed = self.for_session(sid).remove(publisher, stream, subscriber);
        if let Some(routes) = direct.listeners.get_mut(subscriber) {
            routes.remove(&DirectSubscriptionRoute {
                sid: sid.clone(),
                publisher: publisher.clone(),
                stream: stream.clone(),
            });
            if routes.is_empty() {
                direct.listeners.remove(subscriber);
            }
        }
        removed
    }

    pub fn active_direct_listener_count(&self) -> usize {
        self.direct
            .lock()
            .expect("direct subscription lock poisoned")
            .listeners
            .len()
    }

    /// Remove one exact publisher Stream and release each listener whose final
    /// direct route disappeared with it.
    pub fn drop_publisher_stream(
        &self,
        sid: &SessionId,
        publisher: &ConnectionId,
        stream: &StreamId,
    ) {
        let mut direct = self
            .direct
            .lock()
            .expect("direct subscription lock poisoned");
        direct.listeners.retain(|_, routes| {
            routes.remove(&DirectSubscriptionRoute {
                sid: sid.clone(),
                publisher: publisher.clone(),
                stream: stream.clone(),
            });
            !routes.is_empty()
        });
        self.for_session(sid)
            .drop_publisher_stream(publisher, stream);
    }

    /// Remove the entire table for a Session. Called on session.ended.
    pub fn drop_session(&self, sid: &SessionId) {
        let mut direct = self
            .direct
            .lock()
            .expect("direct subscription lock poisoned");
        direct.listeners.retain(|_, routes| {
            routes.retain(|route| &route.sid != sid);
            !routes.is_empty()
        });
        self.sessions.remove(sid);
    }

    /// Drop every reference to `connid` across every Session's table.
    /// Called by `crate::Orchestrator::forget_connection`.
    pub fn drop_connection(&self, connid: &ConnectionId) {
        let mut direct = self
            .direct
            .lock()
            .expect("direct subscription lock poisoned");
        direct.listeners.remove(connid);
        direct.listeners.retain(|_, routes| {
            routes.retain(|route| &route.publisher != connid);
            !routes.is_empty()
        });
        // Snapshot session ids so we don't hold the outer DashMap lock
        // while mutating individual SessionSubscriptions.
        let sids: Vec<SessionId> = self.sessions.iter().map(|e| e.key().clone()).collect();
        for sid in sids {
            if let Some(table_ref) = self.sessions.get(&sid) {
                let table = Arc::clone(table_ref.value());
                drop(table_ref);
                table.drop_connection(connid);
                // Do not remove the outer row here: a concurrent `for_session`
                // can add to this same table after the empty check and before
                // removal. Session teardown owns outer-row GC.
            }
        }
    }
}

// =====================================================================
// Publisher registry — maps a Session's logical `strm_id` (the string
// the wire uses) to the ConnectionId that publishes it. Populated by
// adapters at `stream.opened` time so the SubscriptionHandler can
// resolve subscribers' subscribe requests.
// =====================================================================

/// Rich publisher metadata. v0.x MP2 stored only the `ConnectionId`;
/// MP2.5 extended this to support `from_participant` and `kinds`-form
/// subscriptions by carrying the participant_id and the Stream's kind
/// (audio/video/data) alongside. MP3c (plan B1) adds the negotiated
/// codec so [`crate::Orchestrator::fanout_frame`] can hand the right
/// `CodecInfo` to the subscriber-side adapter when allocating a fresh
/// per-subscription MediaStream.
#[derive(Clone)]
pub struct PublisherEntry {
    pub connection: ConnectionId,
    /// The Participant that published this Stream. From the wire
    /// `connection.offer.by_participant`. Powers `from_participant`
    /// subscription resolution.
    pub participant: String,
    /// The Stream's `kind` (`"audio"`, `"video"`, `"data"`). Powers
    /// the `kinds` filter on `stream.subscribe` requests.
    pub kind: String,
    /// The codec the publisher negotiated for this Stream. Populated
    /// from the chosen codec in `negotiate_streams`' answer. `None`
    /// means the publisher's coordinator never propagated codec info
    /// (older test paths, or future stream kinds where codec doesn't
    /// apply); fanout falls back to [`crate::capability::default_audio_codec`].
    pub codec: Option<crate::capability::CodecInfo>,
}

impl fmt::Debug for PublisherEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PublisherEntry")
            .field("connection", &self.connection)
            .field("participant_present", &!self.participant.is_empty())
            .field("participant_bytes", &self.participant.len())
            .field("kind_present", &!self.kind.is_empty())
            .field("kind_bytes", &self.kind.len())
            .field("codec_present", &self.codec.is_some())
            .finish()
    }
}

/// `(SessionId, strm_id-string) -> PublisherEntry`.
///
/// MP2 introduced the registry keyed by `strm_id`; MP2.5 added a
/// secondary index `(SessionId, participant_id) -> Vec<strm_id>` so
/// `from_participant` lookups don't have to scan the whole table.
pub struct PublisherRegistry {
    inner: DashMap<(SessionId, String), PublisherRecord>,
    /// Participant → streams index. Snapshots stay coherent with
    /// `inner` because every mutating path updates both.
    by_participant: DashMap<(SessionId, String), Vec<String>>,
    /// Serializes the small control-plane mutations that must update both
    /// indexes. Media fanout reads remain lock-free.
    mutation_lock: Mutex<()>,
    next_registration_id: AtomicU64,
}

#[derive(Clone, Debug)]
struct PublisherRecord {
    entry: PublisherEntry,
    registration_id: PublisherRegistrationId,
}

/// Generation token for an owning publisher registration.
///
/// This stays crate-private: public callers keep using the compatible
/// overwrite-oriented [`PublisherRegistry::register`] API, while managed
/// orchestrator resources use the token to ensure an old handle can never
/// remove a newer registration that reused the same Session/Stream key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PublisherRegistrationId(u64);

impl Default for PublisherRegistry {
    fn default() -> Self {
        Self {
            inner: DashMap::new(),
            by_participant: DashMap::new(),
            mutation_lock: Mutex::new(()),
            next_registration_id: AtomicU64::new(1),
        }
    }
}

impl PublisherRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a Stream published by `entry.connection` (which is owned
    /// by `entry.participant`) on `(sid, strm_id)`. Adapters call this
    /// when their connection emits `stream.opened`. Idempotent — repeat
    /// registrations overwrite cleanly.
    pub fn register(&self, sid: SessionId, strm_id: String, entry: PublisherEntry) {
        let _guard = self
            .mutation_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let registration_id = self.next_registration_id();
        let participant_key = (sid.clone(), entry.participant.clone());
        if let Some(previous) = self.inner.insert(
            (sid.clone(), strm_id.clone()),
            PublisherRecord {
                entry,
                registration_id,
            },
        ) {
            self.remove_participant_stream(&sid, &previous.entry.participant, &strm_id);
        }
        self.add_participant_stream(participant_key, strm_id);
    }

    /// Install an owning publisher row only when the canonical
    /// `(SessionId, StreamId)` is unoccupied.
    ///
    /// The returned generation is required for exact managed cleanup. It
    /// prevents a delayed Drop from deleting a replacement registered after
    /// the managed publisher stopped.
    pub(crate) fn register_managed(
        &self,
        sid: SessionId,
        strm_id: String,
        entry: PublisherEntry,
    ) -> std::result::Result<PublisherRegistrationId, PublisherEntry> {
        let _guard = self
            .mutation_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let key = (sid.clone(), strm_id.clone());
        if let Some(existing) = self.inner.get(&key) {
            return Err(existing.entry.clone());
        }

        let registration_id = self.next_registration_id();
        let participant_key = (sid, entry.participant.clone());
        self.inner.insert(
            key,
            PublisherRecord {
                entry,
                registration_id,
            },
        );
        self.add_participant_stream(participant_key, strm_id);
        Ok(registration_id)
    }

    pub(crate) fn registration_is_current(
        &self,
        sid: &SessionId,
        strm_id: &str,
        registration_id: PublisherRegistrationId,
    ) -> bool {
        self.inner
            .get(&(sid.clone(), strm_id.to_string()))
            .is_some_and(|record| record.registration_id == registration_id)
    }

    /// Remove only the row installed by `register_managed`.
    pub(crate) fn remove_registration(
        &self,
        sid: &SessionId,
        strm_id: &str,
        registration_id: PublisherRegistrationId,
    ) -> bool {
        let _guard = self
            .mutation_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let key = (sid.clone(), strm_id.to_string());
        let Some(record) = self.inner.get(&key) else {
            return false;
        };
        if record.registration_id != registration_id {
            return false;
        }
        drop(record);
        let Some((_, removed)) = self.inner.remove(&key) else {
            return false;
        };
        self.remove_participant_stream(sid, &removed.entry.participant, strm_id);
        true
    }

    fn next_registration_id(&self) -> PublisherRegistrationId {
        PublisherRegistrationId(self.next_registration_id.fetch_add(1, Ordering::Relaxed))
    }

    fn add_participant_stream(&self, participant_key: (SessionId, String), strm_id: String) {
        self.by_participant
            .entry(participant_key)
            .and_modify(|v| {
                if !v.iter().any(|s| s == &strm_id) {
                    v.push(strm_id.clone());
                }
            })
            .or_insert_with(|| vec![strm_id]);
    }

    fn remove_participant_stream(&self, sid: &SessionId, participant: &str, strm_id: &str) {
        let participant_key = (sid.clone(), participant.to_string());
        if let Some(mut streams) = self.by_participant.get_mut(&participant_key) {
            streams.retain(|stream| stream != strm_id);
        }
        let empty = self
            .by_participant
            .get(&participant_key)
            .is_some_and(|streams| streams.is_empty());
        if empty {
            self.by_participant.remove(&participant_key);
        }
    }

    /// Resolve a wire-level `strm_id` to its publishing Connection.
    /// MP2-era API; preserved for back-compat.
    pub fn publisher(&self, sid: &SessionId, strm_id: &str) -> Option<ConnectionId> {
        self.inner
            .get(&(sid.clone(), strm_id.to_string()))
            .map(|record| record.entry.connection.clone())
    }

    /// Full publisher entry — used by MP2.5+ for codec / kind / participant
    /// resolution. Returns `None` if no publisher has registered.
    pub fn entry(&self, sid: &SessionId, strm_id: &str) -> Option<PublisherEntry> {
        self.inner
            .get(&(sid.clone(), strm_id.to_string()))
            .map(|record| record.entry.clone())
    }

    /// All `strm_id`s published by `participant` in `sid`. Returns an
    /// empty Vec if the Participant has no registered streams.
    /// Used by `OrchestratorSubscriptionHandler` to resolve
    /// `from_participant`-form subscriptions.
    pub fn streams_for_participant(&self, sid: &SessionId, participant: &str) -> Vec<String> {
        self.by_participant
            .get(&(sid.clone(), participant.to_string()))
            .map(|e| e.value().clone())
            .unwrap_or_default()
    }

    /// Execute one admission operation only while every resolved
    /// `(publisher, stream)` pair is still current.
    ///
    /// Publisher removal uses this same mutation lock. Keeping it through the
    /// synchronous subscription-registry operation establishes one ordering:
    /// either admission wins and teardown subsequently removes its routes, or
    /// teardown wins and admission observes a stale publisher without
    /// mutating. Callers must not re-enter this PublisherRegistry from
    /// `operation`.
    pub fn with_current_routes<R>(
        &self,
        sid: &SessionId,
        routes: &[(ConnectionId, StreamId)],
        operation: impl FnOnce() -> R,
    ) -> Option<R> {
        let _guard = self
            .mutation_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let all_current = routes.iter().all(|(publisher, stream)| {
            self.inner
                .get(&(sid.clone(), stream.as_str().to_string()))
                .is_some_and(|record| &record.entry.connection == publisher)
        });
        all_current.then(operation)
    }

    /// Remove exactly one published stream and its participant-index row.
    /// Idempotent: unknown streams are a no-op.
    pub fn remove_stream(&self, sid: &SessionId, strm_id: &str) {
        let _guard = self
            .mutation_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let key = (sid.clone(), strm_id.to_string());
        let Some((_, removed)) = self.inner.remove(&key) else {
            return;
        };
        self.remove_participant_stream(sid, &removed.entry.participant, strm_id);
    }

    /// Remove a Stream only when its current registration belongs to the
    /// expected publishing Connection.
    ///
    /// Transport teardown must use this conditional form: a delayed close for
    /// an older publisher must not delete a same-named replacement that was
    /// registered by another Connection in the meantime.
    pub fn remove_stream_if_publisher(
        &self,
        sid: &SessionId,
        strm_id: &str,
        publisher: &ConnectionId,
    ) -> bool {
        let _guard = self
            .mutation_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let key = (sid.clone(), strm_id.to_string());
        let belongs_to_publisher = self
            .inner
            .get(&key)
            .is_some_and(|record| &record.entry.connection == publisher);
        if !belongs_to_publisher {
            return false;
        }
        let Some((_, removed)) = self.inner.remove(&key) else {
            return false;
        };
        self.remove_participant_stream(sid, &removed.entry.participant, strm_id);
        true
    }

    /// Drop every registration that names `connid` as publisher (i.e.,
    /// the connection ended). Called by
    /// `crate::Orchestrator::forget_connection`.
    pub fn drop_publisher(&self, connid: &ConnectionId) {
        let _guard = self
            .mutation_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Collect keys to remove; we can't mutate `inner` while
        // iterating it under DashMap.
        let to_remove: Vec<(SessionId, String, PublisherRegistrationId)> = self
            .inner
            .iter()
            .filter(|record| &record.entry.connection == connid)
            .map(|e| {
                let (sid, strm) = e.key();
                (sid.clone(), strm.clone(), e.registration_id)
            })
            .collect();
        for (sid, strm, registration_id) in to_remove {
            let key = (sid.clone(), strm.clone());
            let is_same_registration = self
                .inner
                .get(&key)
                .is_some_and(|record| record.registration_id == registration_id);
            if is_same_registration {
                if let Some((_, removed)) = self.inner.remove(&key) {
                    self.remove_participant_stream(&sid, &removed.entry.participant, &strm);
                }
            }
        }
    }

    /// Drop every registration for a Session. Called on session.ended.
    pub fn drop_session(&self, sid: &SessionId) {
        let _guard = self
            .mutation_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.inner.retain(|(s, _), _| s != sid);
        self.by_participant.retain(|(s, _), _| s != sid);
    }
}

#[cfg(test)]
mod publisher_registry_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn publisher_entry_debug_redacts_participant_kind_and_codec_values() {
        const CANARY: &str = "publisher-canary\r\nAuthorization: exposed";
        let entry = PublisherEntry {
            connection: ConnectionId::from_string(CANARY),
            participant: CANARY.into(),
            kind: CANARY.into(),
            codec: Some(crate::capability::CodecInfo {
                name: CANARY.into(),
                clock_rate_hz: 48_000,
                channels: 1,
                fmtp: Some(CANARY.into()),
            }),
        };
        let debug = format!("{entry:?}");
        assert!(!debug.contains(CANARY));
        assert!(debug.contains("codec_present: true"));
    }

    #[test]
    fn remove_stream_updates_primary_and_participant_indexes() {
        let registry = PublisherRegistry::new();
        let sid = SessionId::new();
        let connection = ConnectionId::new();
        for stream in ["audio-main", "audio-backup"] {
            registry.register(
                sid.clone(),
                stream.to_string(),
                PublisherEntry {
                    connection: connection.clone(),
                    participant: "alice".to_string(),
                    kind: "audio".to_string(),
                    codec: None,
                },
            );
        }

        registry.remove_stream(&sid, "audio-main");
        assert!(registry.entry(&sid, "audio-main").is_none());
        assert_eq!(
            registry.streams_for_participant(&sid, "alice"),
            vec!["audio-backup".to_string()]
        );

        registry.remove_stream(&sid, "audio-backup");
        assert!(registry.streams_for_participant(&sid, "alice").is_empty());
        registry.remove_stream(&sid, "audio-backup");
    }

    #[test]
    fn conditional_remove_cannot_delete_same_named_replacement() {
        let registry = PublisherRegistry::new();
        let sid = SessionId::new();
        let old_publisher = ConnectionId::new();
        let replacement = ConnectionId::new();
        registry.register(
            sid.clone(),
            "audio-main".to_string(),
            PublisherEntry {
                connection: old_publisher.clone(),
                participant: "old".to_string(),
                kind: "audio".to_string(),
                codec: None,
            },
        );
        registry.register(
            sid.clone(),
            "audio-main".to_string(),
            PublisherEntry {
                connection: replacement.clone(),
                participant: "replacement".to_string(),
                kind: "audio".to_string(),
                codec: None,
            },
        );

        assert!(!registry.remove_stream_if_publisher(&sid, "audio-main", &old_publisher,));
        assert_eq!(
            registry.publisher(&sid, "audio-main"),
            Some(replacement.clone())
        );
        assert!(registry.remove_stream_if_publisher(&sid, "audio-main", &replacement,));
        assert!(registry.entry(&sid, "audio-main").is_none());
        assert!(registry
            .streams_for_participant(&sid, "replacement")
            .is_empty());
    }

    #[test]
    fn subscription_drop_connection_releases_map_guard_without_outer_row_race() {
        let registry = Arc::new(SubscriptionRegistry::new());
        let sid = SessionId::new();
        let publisher = ConnectionId::new();
        let subscriber = ConnectionId::new();
        registry
            .for_session(&sid)
            .add(publisher, StreamId::new(), subscriber.clone());
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let registry_for_thread = Arc::clone(&registry);
        std::thread::spawn(move || {
            registry_for_thread.drop_connection(&subscriber);
            let _ = done_tx.send(());
        });

        done_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("drop_connection must not self-deadlock while removing an empty session");
        let table = registry.for_session(&sid);
        assert!(table
            .rows()
            .iter()
            .all(|(_, _, subscribers)| subscribers.is_empty()));
        registry.drop_session(&sid);
        assert!(registry.for_session(&sid).is_empty());
    }
}
