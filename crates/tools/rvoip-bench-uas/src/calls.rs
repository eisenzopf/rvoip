//! The call table: one entry per inbound call, driven by rvoip's call events.
//!
//! rvoip's `CallbackPeer` dispatches each event on its own task, so the
//! events of one call can be observed out of order (a zero-hold call's
//! `CallEnded` or `CallEstablished` may be handled before its
//! `IncomingCall`). The table therefore keeps a few flags per call and
//! derives the gauges from them: a call is live from its `IncomingCall` (rvoip
//! answered it) until its first terminal event, a pending setup while live
//! and not yet established, and active while live and established. Every
//! transition applies only the difference to the gauges, so each call adds to
//! and removes from them exactly once, whatever the order of its events.
//!
//! An entry is removed once its start and its end have both been seen, and
//! the way it ended is counted then. Events seen before the start leave an
//! entry the start completes; entries whose start never arrives are swept
//! after [`TOMBSTONE_TTL`] and counted as unmatched.

use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock, Weak};
use std::time::{Duration, Instant};

use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use rvoip_sip::api::callback_peer::{CallHandler, CallHandlerDecision};
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::{CallId, Event, UnifiedCoordinator};

use crate::echo::{self, EchoRegistry};
use crate::stats::Counters;

/// How long an entry whose start never arrived is remembered.
pub const TOMBSTONE_TTL: Duration = Duration::from_secs(120);

/// The `CallEnded` reason rvoip publishes when the caller sent BYE.
const BYE_RECEIVED_REASON: &str = "BYE received";

/// How a call ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    /// `CallEnded` after the caller's BYE.
    Bye,
    /// `CallEnded` for any other reason.
    Other,
    /// `CallFailed`.
    Failed,
    /// `CallCancelled`.
    Cancelled,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CallEntry {
    /// `IncomingCall` seen: rvoip answered the call.
    started: bool,
    /// `CallEstablished` seen: the ACK arrived.
    established: bool,
    /// The first terminal event seen.
    ended: Option<Ending>,
    /// The echo was started for this call.
    echo_started: bool,
    /// Caller's signaling address, when rvoip reported it.
    remote: Option<SocketAddr>,
    /// When the entry was created (age of an entry without a start).
    created: Option<Instant>,
}

impl CallEntry {
    fn live(&self) -> bool {
        self.started && self.ended.is_none()
    }

    fn complete(&self) -> bool {
        self.started && self.ended.is_some()
    }

    /// Contribution to `(pending_setups, active_calls)`.
    fn contribution(&self) -> (i64, i64) {
        (
            i64::from(self.live() && !self.established),
            i64::from(self.live() && self.established),
        )
    }
}

/// What one event did to a call.
struct Transition {
    before: CallEntry,
    after: CallEntry,
    /// The call just became live and established: start its echo.
    start_echo: bool,
}

/// Shared state behind the [`BenchHandler`].
pub struct CallTable {
    calls: DashMap<CallId, CallEntry>,
    counters: Arc<Counters>,
    echo: Option<Arc<EchoRegistry>>,
    coordinator: OnceLock<Weak<UnifiedCoordinator>>,
}

impl CallTable {
    /// A table updating `counters`; with `echo_media`, established calls get
    /// an RTP echo.
    pub fn new(counters: Arc<Counters>, echo_media: bool) -> Arc<Self> {
        Arc::new(Self {
            calls: DashMap::new(),
            echo: echo_media.then(|| Arc::new(EchoRegistry::new(Arc::clone(&counters)))),
            counters,
            coordinator: OnceLock::new(),
        })
    }

    /// Attach the coordinator the echo binds media streams on.
    pub fn attach(&self, coordinator: &Arc<UnifiedCoordinator>) {
        let _ = self.coordinator.set(Arc::downgrade(coordinator));
    }

    /// The counters this table updates.
    pub fn counters(&self) -> &Arc<Counters> {
        &self.counters
    }

    /// Apply `change` to `call_id`'s entry, then the gauge difference and the
    /// end count of a call that just completed.
    fn transition(&self, call_id: &CallId, change: impl FnOnce(&mut CallEntry)) -> Transition {
        let apply = |entry: &mut CallEntry| {
            change(entry);
            let start_echo = entry.live() && entry.established && !entry.echo_started;
            if start_echo {
                entry.echo_started = true;
            }
            start_echo
        };
        let (before, after, start_echo) = match self.calls.entry(call_id.clone()) {
            Entry::Occupied(mut occupied) => {
                let before = *occupied.get();
                let start_echo = apply(occupied.get_mut());
                let after = *occupied.get();
                if after.complete() {
                    occupied.remove();
                }
                (before, after, start_echo)
            }
            Entry::Vacant(vacant) => {
                let before = CallEntry::default();
                let mut after = CallEntry {
                    created: Some(Instant::now()),
                    ..before
                };
                let start_echo = apply(&mut after);
                if !after.complete() {
                    vacant.insert(after);
                }
                (before, after, start_echo)
            }
        };
        let (pending_before, active_before) = before.contribution();
        let (pending_after, active_after) = after.contribution();
        self.counters
            .pending_setups
            .fetch_add(pending_after - pending_before, Ordering::Relaxed);
        self.counters
            .active_calls
            .fetch_add(active_after - active_before, Ordering::Relaxed);
        if !before.complete() {
            if let (true, Some(ending)) = (after.complete(), after.ended) {
                let counter = match ending {
                    Ending::Bye => &self.counters.byes_received,
                    Ending::Other => &self.counters.calls_ended_other,
                    Ending::Failed => &self.counters.calls_failed,
                    Ending::Cancelled => &self.counters.calls_cancelled,
                };
                counter.fetch_add(1, Ordering::Relaxed);
            }
        }
        Transition {
            before,
            after,
            start_echo,
        }
    }

    /// rvoip answered `call_id` (fast auto-accept sends the 200 OK before it
    /// publishes `IncomingCall`).
    pub fn on_started(self: &Arc<Self>, call_id: &CallId, remote: Option<SocketAddr>) {
        let transition = self.transition(call_id, |entry| {
            entry.started = true;
            if entry.remote.is_none() {
                entry.remote = remote;
            }
        });
        if !transition.before.started {
            self.counters.incoming_calls.fetch_add(1, Ordering::Relaxed);
        }
        if transition.start_echo {
            self.start_echo(call_id.clone());
        }
    }

    /// The ACK for `call_id` arrived.
    pub fn on_established(self: &Arc<Self>, call_id: &CallId) {
        let transition = self.transition(call_id, |entry| entry.established = true);
        if !transition.before.established {
            self.counters
                .calls_established
                .fetch_add(1, Ordering::Relaxed);
        }
        if transition.start_echo {
            self.start_echo(call_id.clone());
        }
    }

    /// `call_id` ended. Only the first terminal event of a call counts.
    pub fn on_ended(&self, call_id: &CallId, ending: Ending) {
        let transition = self.transition(call_id, |entry| {
            entry.ended.get_or_insert(ending);
        });
        if transition.before.ended.is_none() && transition.after.ended.is_some() {
            if let Some(echo) = &self.echo {
                echo.close(call_id);
            }
        }
    }

    /// Whether `call_id` is answered and has not ended.
    pub fn is_live(&self, call_id: &CallId) -> bool {
        self.calls.get(call_id).is_some_and(|entry| entry.live())
    }

    /// Distinct caller addresses with at least one live call.
    pub fn live_remote_count(&self) -> u64 {
        let mut remotes = std::collections::HashSet::new();
        for entry in self.calls.iter() {
            if entry.live() {
                if let Some(remote) = entry.remote {
                    remotes.insert(remote);
                }
            }
        }
        remotes.len() as u64
    }

    /// Forget entries older than `ttl` whose start never arrived.
    pub fn sweep_tombstones(&self, ttl: Duration) {
        let now = Instant::now();
        let mut swept = 0u64;
        self.calls.retain(|_, entry| {
            let expired = !entry.started
                && entry
                    .created
                    .is_some_and(|at| now.saturating_duration_since(at) >= ttl);
            swept += u64::from(expired);
            !expired
        });
        if swept > 0 {
            self.counters
                .unmatched_terminal_events
                .fetch_add(swept, Ordering::Relaxed);
        }
    }

    /// Entries currently held (live calls plus entries awaiting their start).
    pub fn len(&self) -> usize {
        self.calls.len()
    }

    /// Whether the table holds no entry.
    pub fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }

    fn start_echo(self: &Arc<Self>, call_id: CallId) {
        let (Some(echo), Some(coordinator)) = (
            self.echo.clone(),
            self.coordinator.get().and_then(Weak::upgrade),
        ) else {
            return;
        };
        let table = Arc::clone(self);
        tokio::spawn(async move {
            echo::start(echo, coordinator, call_id, move |id| table.is_live(id)).await;
        });
    }
}

/// The `CallHandler`: rvoip answers every INVITE itself (fast auto-accept),
/// so the handler only keeps the books and starts the echo.
#[derive(Clone)]
pub struct BenchHandler {
    table: Arc<CallTable>,
}

impl BenchHandler {
    /// A handler over `table`.
    pub fn new(table: Arc<CallTable>) -> Self {
        Self { table }
    }
}

#[async_trait::async_trait]
impl CallHandler for BenchHandler {
    async fn on_event(&self, event: Event) {
        match event {
            Event::CallEstablished { call_id } => self.table.on_established(&call_id),
            Event::CallEnded { call_id, reason } => {
                let ending = if reason == BYE_RECEIVED_REASON {
                    Ending::Bye
                } else {
                    Ending::Other
                };
                self.table.on_ended(&call_id, ending);
            }
            Event::CallFailed { call_id, .. } => self.table.on_ended(&call_id, Ending::Failed),
            Event::CallCancelled { call_id } => self.table.on_ended(&call_id, Ending::Cancelled),
            _ => {}
        }
    }

    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        let remote = call
            .transport_security_context()
            .remote_addr
            .as_deref()
            .and_then(|addr| addr.parse().ok());
        self.table.on_started(&call.call_id, remote);
        // Fast auto-accept already sent the 200 OK; the decision is ignored.
        CallHandlerDecision::Accept
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(name: &str) -> CallId {
        rvoip_sip::SessionId(name.to_string())
    }

    fn table() -> Arc<CallTable> {
        CallTable::new(Arc::new(Counters::default()), false)
    }

    fn gauges(table: &CallTable) -> (i64, i64) {
        (
            table.counters.pending_setups.load(Ordering::Relaxed),
            table.counters.active_calls.load(Ordering::Relaxed),
        )
    }

    #[test]
    fn a_call_in_order_moves_from_setup_to_active_to_gone() {
        let table = table();
        let call = id("a");
        let remote = Some("10.0.0.2:5070".parse().unwrap());
        table.on_started(&call, remote);
        assert_eq!(gauges(&table), (1, 0));
        assert_eq!(table.live_remote_count(), 1);
        table.on_established(&call);
        table.on_established(&call);
        assert_eq!(gauges(&table), (0, 1));
        table.on_ended(&call, Ending::Bye);
        assert_eq!(gauges(&table), (0, 0));
        assert!(table.is_empty());
        let counters = table.counters();
        assert_eq!(counters.incoming_calls.load(Ordering::Relaxed), 1);
        assert_eq!(counters.calls_established.load(Ordering::Relaxed), 1);
        assert_eq!(counters.byes_received.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn every_event_order_of_one_call_ends_at_zero() {
        // started (s), established (e), ended (x): all six orders.
        let orders = ["sex", "sxe", "esx", "exs", "xse", "xes"];
        for order in orders {
            let table = table();
            let call = id(order);
            for step in order.chars() {
                match step {
                    's' => table.on_started(&call, None),
                    'e' => table.on_established(&call),
                    _ => table.on_ended(&call, Ending::Bye),
                }
                let (pending, active) = gauges(&table);
                assert!(pending >= 0 && active >= 0, "{order}: gauges went negative");
                assert!(pending + active <= 1, "{order}: counted twice");
            }
            assert_eq!(gauges(&table), (0, 0), "{order}");
            // A late ACK after the end leaves an entry awaiting a start that
            // already came; the sweep forgets it.
            table.sweep_tombstones(Duration::ZERO);
            assert!(table.is_empty(), "{order}: entry left behind");
            assert_eq!(
                table.counters.byes_received.load(Ordering::Relaxed),
                1,
                "{order}: the end counts once"
            );
            assert_eq!(
                table.counters.incoming_calls.load(Ordering::Relaxed),
                1,
                "{order}"
            );
        }
    }

    #[test]
    fn only_the_first_terminal_event_counts() {
        let table = table();
        let call = id("dup");
        table.on_started(&call, None);
        table.on_ended(&call, Ending::Failed);
        table.on_ended(&call, Ending::Bye);
        table.on_ended(&call, Ending::Cancelled);
        let counters = table.counters();
        assert_eq!(counters.calls_failed.load(Ordering::Relaxed), 1);
        assert_eq!(counters.byes_received.load(Ordering::Relaxed), 0);
        assert_eq!(counters.calls_cancelled.load(Ordering::Relaxed), 0);
        assert_eq!(gauges(&table), (0, 0));
        // A failed setup never counted as active.
        assert_eq!(counters.calls_established.load(Ordering::Relaxed), 0);
        // The duplicates arrived after the call was gone; they are swept as
        // unmatched, never counted as ends.
        table.sweep_tombstones(Duration::ZERO);
        assert!(table.is_empty());
        assert_eq!(
            counters.unmatched_terminal_events.load(Ordering::Relaxed),
            1
        );
    }

    #[test]
    fn an_end_without_a_start_is_a_tombstone_until_swept() {
        let table = table();
        let call = id("orphan");
        table.on_ended(&call, Ending::Other);
        assert_eq!(gauges(&table), (0, 0));
        assert_eq!(table.len(), 1, "tombstone kept for a late start");
        table.sweep_tombstones(Duration::from_secs(3600));
        assert_eq!(table.len(), 1, "young tombstones stay");
        table.sweep_tombstones(Duration::ZERO);
        assert!(table.is_empty());
        assert_eq!(
            table
                .counters
                .unmatched_terminal_events
                .load(Ordering::Relaxed),
            1
        );
        // Live calls are never swept.
        table.on_started(&id("live"), None);
        table.sweep_tombstones(Duration::ZERO);
        assert_eq!(table.len(), 1);
        assert!(table.is_live(&id("live")));
    }

    #[test]
    fn many_calls_from_several_flows() {
        let table = table();
        for flow in 0..4 {
            let remote: SocketAddr = format!("10.0.0.{}:5070", flow + 2).parse().unwrap();
            for call in 0..25 {
                let call = id(&format!("{flow}-{call}"));
                table.on_started(&call, Some(remote));
                table.on_established(&call);
            }
        }
        assert_eq!(gauges(&table), (0, 100));
        assert_eq!(table.live_remote_count(), 4);
        for flow in 0..4 {
            for call in 0..25 {
                table.on_ended(&id(&format!("{flow}-{call}")), Ending::Bye);
            }
        }
        assert_eq!(gauges(&table), (0, 0));
        assert_eq!(table.live_remote_count(), 0);
        assert!(table.is_empty());
    }
}
