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

use std::sync::Arc;

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
    pub fn remove(&self, publisher: &ConnectionId, strm_id: &StreamId, subscriber: &ConnectionId) -> bool {
        let key = (publisher.clone(), strm_id.clone());
        let removed = if let Some(entry) = self.inner.get(&key) {
            entry.remove(subscriber).is_some()
        } else {
            false
        };
        // Garbage-collect empty subscriber sets so the table doesn't
        // grow unboundedly across long-lived sessions with churn.
        if removed {
            if let Some(entry) = self.inner.get(&key) {
                if entry.is_empty() {
                    drop(entry);
                    self.inner.remove(&key);
                }
            }
        }
        removed
    }

    /// Look up subscribers for `(publisher, strm_id)`. Returns a
    /// snapshot — callers that fan out datagrams iterate this without
    /// holding the table lock.
    pub fn subscribers_for(&self, publisher: &ConnectionId, strm_id: &StreamId) -> Vec<ConnectionId> {
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
        let keys: Vec<(ConnectionId, StreamId)> = self
            .inner
            .iter()
            .map(|e| e.key().clone())
            .collect();
        for key in keys {
            if key.0 == *connid {
                self.inner.remove(&key);
                continue;
            }
            let collapsed = if let Some(set) = self.inner.get(&key) {
                set.remove(connid);
                set.is_empty()
            } else {
                false
            };
            if collapsed {
                self.inner.remove(&key);
            }
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
}

/// Workspace-level subscription registry — one [`SessionSubscriptions`]
/// per active `SessionId`. Lives on the [`crate::Orchestrator`].
#[derive(Default)]
pub struct SubscriptionRegistry {
    sessions: DashMap<SessionId, Arc<SessionSubscriptions>>,
}

impl SubscriptionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the per-Session table, creating it lazily on first use.
    pub fn for_session(&self, sid: &SessionId) -> Arc<SessionSubscriptions> {
        self.sessions
            .entry(sid.clone())
            .or_insert_with(|| Arc::new(SessionSubscriptions::new()))
            .clone()
    }

    /// Remove the entire table for a Session. Called on session.ended.
    pub fn drop_session(&self, sid: &SessionId) {
        self.sessions.remove(sid);
    }

    /// Drop every reference to `connid` across every Session's table.
    /// Called by `crate::Orchestrator::forget_connection`.
    pub fn drop_connection(&self, connid: &ConnectionId) {
        // Snapshot session ids so we don't hold the outer DashMap lock
        // while mutating individual SessionSubscriptions.
        let sids: Vec<SessionId> = self.sessions.iter().map(|e| e.key().clone()).collect();
        for sid in sids {
            if let Some(table) = self.sessions.get(&sid) {
                let table = Arc::clone(table.value());
                drop(self.sessions.get(&sid));
                table.drop_connection(connid);
                if table.is_empty() {
                    self.sessions.remove(&sid);
                }
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
#[derive(Clone, Debug)]
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

/// `(SessionId, strm_id-string) -> PublisherEntry`.
///
/// MP2 introduced the registry keyed by `strm_id`; MP2.5 added a
/// secondary index `(SessionId, participant_id) -> Vec<strm_id>` so
/// `from_participant` lookups don't have to scan the whole table.
#[derive(Default)]
pub struct PublisherRegistry {
    inner: DashMap<(SessionId, String), PublisherEntry>,
    /// Participant → streams index. Snapshots stay coherent with
    /// `inner` because every mutating path updates both.
    by_participant: DashMap<(SessionId, String), Vec<String>>,
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
        let participant_key = (sid.clone(), entry.participant.clone());
        self.inner
            .insert((sid.clone(), strm_id.clone()), entry);
        // Keep by_participant in sync. Push without dedup; if the
        // adapter announces the same stream twice we tolerate the
        // duplicate (the primary table overwrote anyway).
        self.by_participant
            .entry(participant_key)
            .and_modify(|v| {
                if !v.iter().any(|s| s == &strm_id) {
                    v.push(strm_id.clone());
                }
            })
            .or_insert_with(|| vec![strm_id]);
    }

    /// Resolve a wire-level `strm_id` to its publishing Connection.
    /// MP2-era API; preserved for back-compat.
    pub fn publisher(&self, sid: &SessionId, strm_id: &str) -> Option<ConnectionId> {
        self.inner
            .get(&(sid.clone(), strm_id.to_string()))
            .map(|e| e.value().connection.clone())
    }

    /// Full publisher entry — used by MP2.5+ for codec / kind / participant
    /// resolution. Returns `None` if no publisher has registered.
    pub fn entry(&self, sid: &SessionId, strm_id: &str) -> Option<PublisherEntry> {
        self.inner
            .get(&(sid.clone(), strm_id.to_string()))
            .map(|e| e.value().clone())
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

    /// Drop every registration that names `connid` as publisher (i.e.,
    /// the connection ended). Called by
    /// `crate::Orchestrator::forget_connection`.
    pub fn drop_publisher(&self, connid: &ConnectionId) {
        // Collect keys to remove; we can't mutate `inner` while
        // iterating it under DashMap.
        let to_remove: Vec<(SessionId, String, String)> = self
            .inner
            .iter()
            .filter(|e| &e.value().connection == connid)
            .map(|e| {
                let (sid, strm) = e.key();
                (sid.clone(), strm.clone(), e.value().participant.clone())
            })
            .collect();
        for (sid, strm, participant) in to_remove {
            self.inner.remove(&(sid.clone(), strm.clone()));
            if let Some(mut entry) = self.by_participant.get_mut(&(sid.clone(), participant.clone())) {
                entry.retain(|s| s != &strm);
            }
            // GC empty participant entries to avoid stale Vec growth.
            let key = (sid, participant);
            let is_empty = self
                .by_participant
                .get(&key)
                .map(|e| e.value().is_empty())
                .unwrap_or(false);
            if is_empty {
                self.by_participant.remove(&key);
            }
        }
    }

    /// Drop every registration for a Session. Called on session.ended.
    pub fn drop_session(&self, sid: &SessionId) {
        self.inner.retain(|(s, _), _| s != sid);
        self.by_participant.retain(|(s, _), _| s != sid);
    }
}
