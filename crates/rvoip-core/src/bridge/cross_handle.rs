//! `CrossBridgeHandle` — the cross-transport sibling of the SIP-fast-path
//! `BridgeHandle` re-exported in `super`.
//!
//! Owns the abort handles for the two frame-pump tasks that copy media
//! between the bridged Connections. `Drop` aborts both pumps so an
//! `unbridge_connections` call (or the Orchestrator going away) tears
//! the bridge down promptly.
//!
//! Gap plan §4.2 v1 punch list — also holds the per-direction swap
//! channels used by [`Self::swap_transcoders`] to hot-swap the pump
//! transcoders after a mid-call codec renegotiation. Senders are
//! `Some(_)` for bridges built via the swap-aware path; the legacy
//! `new` constructor leaves them `None` for backward compatibility.

use chrono::{DateTime, Utc};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use super::frame_pump::TranscoderSwap;
use crate::error::{Result, RvoipError};
use crate::ids::{BridgeId, ConnectionId};

pub struct CrossBridgeHandle {
    pub id: BridgeId,
    pub a: ConnectionId,
    pub b: ConnectionId,
    pub created_at: DateTime<Utc>,
    a_to_b: AbortHandle,
    b_to_a: AbortHandle,
    /// Gap plan §4.2 — channel into the a→b pump's swap port. `None`
    /// for handles built via the legacy `new` constructor (no
    /// hot-swap support; calling `swap_transcoders` returns
    /// `NotImplemented`).
    swap_a_to_b: Option<mpsc::Sender<TranscoderSwap>>,
    /// Gap plan §4.2 — channel into the b→a pump's swap port.
    swap_b_to_a: Option<mpsc::Sender<TranscoderSwap>>,
}

impl CrossBridgeHandle {
    pub fn new(
        id: BridgeId,
        a: ConnectionId,
        b: ConnectionId,
        a_to_b: AbortHandle,
        b_to_a: AbortHandle,
    ) -> Self {
        Self {
            id,
            a,
            b,
            created_at: Utc::now(),
            a_to_b,
            b_to_a,
            swap_a_to_b: None,
            swap_b_to_a: None,
        }
    }

    /// Gap plan §4.2 v1 punch list — variant of [`Self::new`] that
    /// captures the per-direction swap-channel senders so the bridge
    /// can hot-swap its transcoders after a codec renegotiation.
    pub fn with_swap_channels(
        id: BridgeId,
        a: ConnectionId,
        b: ConnectionId,
        a_to_b: AbortHandle,
        b_to_a: AbortHandle,
        swap_a_to_b: mpsc::Sender<TranscoderSwap>,
        swap_b_to_a: mpsc::Sender<TranscoderSwap>,
    ) -> Self {
        Self {
            id,
            a,
            b,
            created_at: Utc::now(),
            a_to_b,
            b_to_a,
            swap_a_to_b: Some(swap_a_to_b),
            swap_b_to_a: Some(swap_b_to_a),
        }
    }

    /// Atomically swap the transcoders on both directions. Used by
    /// [`Orchestrator::renegotiate_media`] after a successful
    /// adapter-level renegotiation: the new (from_pt, to_pt) pairs
    /// reflect the post-renegotiation codecs on each leg.
    ///
    /// The swap is best-effort: if the swap channel for a direction
    /// is full or closed (pump exited), that direction is skipped
    /// and the call still returns `Ok(())` for the directions that
    /// did swap. A complete failure (no swap channels wired) returns
    /// [`RvoipError::NotImplemented`].
    pub async fn swap_transcoders(
        &self,
        a_to_b_swap: TranscoderSwap,
        b_to_a_swap: TranscoderSwap,
    ) -> Result<()> {
        let Some(a_tx) = self.swap_a_to_b.as_ref() else {
            return Err(RvoipError::NotImplemented(
                "CrossBridgeHandle::swap_transcoders — bridge built without swap channels",
            ));
        };
        let Some(b_tx) = self.swap_b_to_a.as_ref() else {
            return Err(RvoipError::NotImplemented(
                "CrossBridgeHandle::swap_transcoders — bridge built without swap channels",
            ));
        };
        // Best-effort: send on both. A closed receiver (pump exited)
        // is silently skipped — the bridge is on its way out anyway.
        let _ = a_tx.send(a_to_b_swap).await;
        let _ = b_tx.send(b_to_a_swap).await;
        Ok(())
    }
}

impl Drop for CrossBridgeHandle {
    fn drop(&mut self) {
        self.a_to_b.abort();
        self.b_to_a.abort();
    }
}
