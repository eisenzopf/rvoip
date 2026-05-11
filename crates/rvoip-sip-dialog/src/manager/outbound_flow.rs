//! RFC 5626 §3.5.1 outbound flow state machine (Phase 2c).
//!
//! A5 Phase 2b-min kept a stateless per-flow ping task that wrote
//! CRLFCRLF on a tokio interval and silently died on send failure.
//! Phase 2c upgrades that task to a stateful [`OutboundFlow`] that:
//!
//! * arms a pong deadline after each ping,
//! * folds in transport `KeepAlivePongReceived` / `ConnectionClosed`
//!   events to either reset or fail the flow,
//! * emits a single
//!   [`SessionCoordinationEvent::OutboundFlowFailed`](crate::events::SessionCoordinationEvent::OutboundFlowFailed)
//!   on the first transition to [`FlowState::Failed`] so session-core
//!   can trigger a fresh REGISTER (RFC 5626 §4.4.1 flow recovery).
//!
//! This module owns the state transitions and unit tests; the spawn +
//! select-loop driver lives in [`super::core`] because it also owns the
//! transport handle and the flow-registration maps.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

/// Transport-side events relevant to outbound-flow liveness that the
/// transaction layer forwards to the dialog manager.
///
/// Carved out as its own enum (rather than re-using
/// [`rvoip_sip_transport::TransportEvent`]) so the transaction manager
/// can filter + enqueue only the variants that outbound-flow monitoring
/// cares about, keeping the forwarding channel small and typed.
#[derive(Debug, Clone)]
pub enum FlowTransportEvent {
    /// RFC 5626 §3.5.1 pong (bare CRLF) received from `source`.
    PongReceived {
        /// Remote address that sent the pong.
        source: SocketAddr,
    },
    /// Connection-oriented transport lost its connection to `remote_addr`.
    ConnectionClosed {
        /// Remote address whose connection was lost.
        remote_addr: SocketAddr,
    },
}

/// Lifecycle of a single outbound keep-alive flow.
///
/// Transitions:
/// ```text
/// Idle --[send ping]--> AwaitingPong
/// AwaitingPong --[pong]--> Idle
/// AwaitingPong --[timeout|close|send-err]--> Failed    (terminal)
/// Idle --[close|send-err]--> Failed                    (terminal)
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FlowState {
    /// Ping not yet sent, or most recent pong arrived in time.
    Idle,
    /// Ping sent; awaiting pong within `pong_timeout`.
    AwaitingPong,
    /// Flow has been marked failed (by pong timeout, connection closed,
    /// or unrecoverable send error). Terminal for this flow instance;
    /// a fresh one is created on the next successful REGISTER 2xx.
    Failed,
}

/// Per-flow state tracker keyed by `(aor, reg_id, instance)`.
#[derive(Debug)]
pub(crate) struct OutboundFlow {
    pub(crate) key: (String, u32, String),
    pub(crate) destination: SocketAddr,
    pub(crate) interval: Duration,
    pub(crate) pong_timeout: Duration,

    state: RwLock<FlowState>,
    last_ping_at: RwLock<Option<Instant>>,
    last_pong_at: RwLock<Option<Instant>>,
}

/// Minimum pong deadline. A 1 s interval would otherwise yield a 2 s
/// timeout, which is tight enough that a mildly loaded peer can look
/// dead. 10 s matches typical SIP transaction-timeout conventions.
const MIN_PONG_TIMEOUT: Duration = Duration::from_secs(10);

impl OutboundFlow {
    pub(crate) fn new(
        key: (String, u32, String),
        destination: SocketAddr,
        interval: Duration,
    ) -> Self {
        let pong_timeout = std::cmp::max(interval.saturating_mul(2), MIN_PONG_TIMEOUT);
        Self {
            key,
            destination,
            interval,
            pong_timeout,
            state: RwLock::new(FlowState::Idle),
            last_ping_at: RwLock::new(None),
            last_pong_at: RwLock::new(None),
        }
    }

    /// Record that a ping just went on the wire. No-op if already
    /// `Failed` so we never revive a terminated flow.
    pub(crate) async fn record_ping_sent(&self) {
        let mut state = self.state.write().await;
        if *state == FlowState::Failed {
            return;
        }
        *state = FlowState::AwaitingPong;
        *self.last_ping_at.write().await = Some(Instant::now());
    }

    /// Handle a `KeepAlivePongReceived` event: advance `last_pong_at`
    /// and drop back to `Idle`. No-op if `Failed`.
    pub(crate) async fn on_pong(&self) {
        let mut state = self.state.write().await;
        if *state == FlowState::Failed {
            return;
        }
        *self.last_pong_at.write().await = Some(Instant::now());
        *state = FlowState::Idle;
    }

    /// Idempotent transition to `Failed`. Returns `true` **only on the
    /// first** transition so the caller emits exactly one
    /// `OutboundFlowFailed` event per flow instance.
    pub(crate) async fn mark_failed(&self) -> bool {
        let mut state = self.state.write().await;
        if *state == FlowState::Failed {
            return false;
        }
        *state = FlowState::Failed;
        true
    }

    pub(crate) async fn state(&self) -> FlowState {
        *self.state.read().await
    }

    /// True iff we sent a ping and no equal-or-later pong has arrived
    /// — used by the deadline arm of the ping-loop `tokio::select!`.
    /// The pong-arrives-during-timeout race is resolved by taking the
    /// ping + pong reads under the same `state` guard and requiring
    /// `last_pong >= last_ping` to be satisfied for "pong on time".
    pub(crate) async fn is_pong_overdue(&self) -> bool {
        let state = self.state.read().await;
        if *state != FlowState::AwaitingPong {
            return false;
        }
        let last_ping = match *self.last_ping_at.read().await {
            Some(t) => t,
            None => return false,
        };
        match *self.last_pong_at.read().await {
            Some(last_pong) if last_pong >= last_ping => false,
            _ => true,
        }
    }

    #[cfg(test)]
    pub(crate) async fn last_pong_at(&self) -> Option<Instant> {
        *self.last_pong_at.read().await
    }

    #[cfg(test)]
    pub(crate) async fn last_ping_at(&self) -> Option<Instant> {
        *self.last_ping_at.read().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    fn test_addr() -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 5060)
    }

    fn test_key() -> (String, u32, String) {
        (
            "sip:alice@example.com".to_string(),
            1,
            "urn:uuid:deadbeef-0000-0000-0000-000000000001".to_string(),
        )
    }

    #[tokio::test]
    async fn new_flow_starts_idle() {
        let flow = OutboundFlow::new(test_key(), test_addr(), Duration::from_secs(25));
        assert_eq!(flow.state().await, FlowState::Idle);
        assert!(!flow.is_pong_overdue().await);
        assert!(flow.last_ping_at().await.is_none());
        assert!(flow.last_pong_at().await.is_none());
    }

    #[tokio::test]
    async fn pong_timeout_defaults_to_at_least_ten_seconds() {
        // Short interval → pong_timeout clamped to 10 s minimum.
        let flow = OutboundFlow::new(test_key(), test_addr(), Duration::from_secs(1));
        assert_eq!(flow.pong_timeout, Duration::from_secs(10));

        // Long interval → 2× scales up.
        let flow = OutboundFlow::new(test_key(), test_addr(), Duration::from_secs(25));
        assert_eq!(flow.pong_timeout, Duration::from_secs(50));
    }

    #[tokio::test]
    async fn record_ping_sent_transitions_to_awaiting_pong() {
        let flow = OutboundFlow::new(test_key(), test_addr(), Duration::from_secs(25));
        flow.record_ping_sent().await;
        assert_eq!(flow.state().await, FlowState::AwaitingPong);
        assert!(flow.last_ping_at().await.is_some());
        assert!(flow.is_pong_overdue().await);
    }

    #[tokio::test]
    async fn on_pong_resets_state_to_idle() {
        let flow = OutboundFlow::new(test_key(), test_addr(), Duration::from_secs(25));
        flow.record_ping_sent().await;
        flow.on_pong().await;
        assert_eq!(flow.state().await, FlowState::Idle);
        assert!(flow.last_pong_at().await.is_some());
        assert!(!flow.is_pong_overdue().await);
    }

    #[tokio::test]
    async fn mark_failed_is_idempotent() {
        let flow = OutboundFlow::new(test_key(), test_addr(), Duration::from_secs(25));
        flow.record_ping_sent().await;
        assert!(flow.mark_failed().await);
        assert_eq!(flow.state().await, FlowState::Failed);
        // Second call must NOT return true — otherwise we'd emit two
        // `OutboundFlowFailed` events and fire two re-REGISTERs.
        assert!(!flow.mark_failed().await);
        assert_eq!(flow.state().await, FlowState::Failed);
    }

    #[tokio::test]
    async fn failed_state_does_not_revive_on_pong_or_ping() {
        let flow = OutboundFlow::new(test_key(), test_addr(), Duration::from_secs(25));
        assert!(flow.mark_failed().await);
        flow.on_pong().await;
        flow.record_ping_sent().await;
        assert_eq!(flow.state().await, FlowState::Failed);
    }

    #[tokio::test]
    async fn pong_beats_timeout_check() {
        // Order: ping → pong → is_pong_overdue should be false.
        let flow = OutboundFlow::new(test_key(), test_addr(), Duration::from_secs(25));
        flow.record_ping_sent().await;
        // Sleep 1 ms so `Instant::now()` in `on_pong` is strictly >= `last_ping_at`.
        tokio::time::sleep(Duration::from_millis(1)).await;
        flow.on_pong().await;
        assert!(!flow.is_pong_overdue().await);
    }

    #[tokio::test]
    async fn stale_pong_before_latest_ping_is_still_overdue() {
        // Pong arrived once, then a later ping went out with no pong.
        // is_pong_overdue must report true because last_pong < last_ping.
        let flow = OutboundFlow::new(test_key(), test_addr(), Duration::from_secs(25));
        flow.record_ping_sent().await;
        tokio::time::sleep(Duration::from_millis(1)).await;
        flow.on_pong().await;
        tokio::time::sleep(Duration::from_millis(1)).await;
        flow.record_ping_sent().await;
        assert!(flow.is_pong_overdue().await);
    }
}
