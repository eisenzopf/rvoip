//! G3 — Perfect-negotiation helper for two-sided offer/answer state machines.
//!
//! Implements the W3C [Perfect Negotiation] pattern: each peer is configured
//! as `polite` or `impolite`. On offer collision the polite peer rolls back
//! its local offer and applies the remote one; the impolite peer ignores
//! the remote offer and proceeds with its own.
//!
//! Use it to coordinate calls between two `WebRtcClient` instances when
//! either side may add a track / restart ICE / change capability without
//! pre-negotiated control of which side is offerer.
//!
//! [Perfect Negotiation]: https://developer.mozilla.org/en-US/docs/Web/API/WebRTC_API/Perfect_negotiation
//!
//! ```ignore
//! use rvoip_webrtc::peer::RvoipPeerConnection;
//! use rvoip_webrtc::client::PerfectNegotiation;
//! use std::sync::Arc;
//!
//! let pn = PerfectNegotiation::new(/* polite */ true);
//! let action = pn.decide_remote_offer(&peer).await;
//! match action {
//!     NegotiationAction::Apply => { /* set remote, create answer */ }
//!     NegotiationAction::Ignore => { /* drop the offer */ }
//!     NegotiationAction::Rollback => {
//!         peer.rollback_local().await?;
//!         /* now set remote, create answer */
//!     }
//! }
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::peer::RvoipPeerConnection;

/// What the caller should do with a remote offer that just arrived.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NegotiationAction {
    /// Apply the remote offer normally (set remote → create answer →
    /// set local → send).
    Apply,
    /// Drop the remote offer; this peer's own pending offer wins.
    Ignore,
    /// Rollback the local pending offer (via
    /// [`RvoipPeerConnection::rollback_local`]) before applying the remote.
    Rollback,
}

/// Perfect-negotiation state machine. Cheap to clone; internally uses
/// atomics so multiple offer-creating tasks can coordinate.
#[derive(Clone, Debug)]
pub struct PerfectNegotiation {
    polite: bool,
    making_offer: Arc<AtomicBool>,
    ignore_offer: Arc<AtomicBool>,
    is_setting_remote_answer_pending: Arc<AtomicBool>,
}

impl PerfectNegotiation {
    /// `polite = true` means this peer yields on collision (rolls back
    /// its pending local offer). The other side must be `polite = false`
    /// or the call will deadlock on simultaneous offers.
    pub fn new(polite: bool) -> Self {
        Self {
            polite,
            making_offer: Arc::new(AtomicBool::new(false)),
            ignore_offer: Arc::new(AtomicBool::new(false)),
            is_setting_remote_answer_pending: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn is_polite(&self) -> bool {
        self.polite
    }

    /// Call before starting `createOffer` / `setLocalDescription`. Pair
    /// with [`Self::end_local_offer`] in a guard pattern.
    pub fn begin_local_offer(&self) {
        self.making_offer.store(true, Ordering::Release);
    }

    pub fn end_local_offer(&self) {
        self.making_offer.store(false, Ordering::Release);
    }

    /// Per the W3C algorithm: `ignoreOffer = !polite && (makingOffer ||
    /// signalingState != "stable")`.
    ///
    /// Returns the action the caller should take for a freshly arrived
    /// remote offer.
    pub async fn decide_remote_offer(&self, peer: &Arc<RvoipPeerConnection>) -> NegotiationAction {
        let making_offer = self.making_offer.load(Ordering::Acquire);
        let stable = peer.signaling_is_stable().await;
        let ready_for_offer = !making_offer
            && (stable
                || self
                    .is_setting_remote_answer_pending
                    .load(Ordering::Acquire));
        let offer_collision = !ready_for_offer;

        if offer_collision && !self.polite {
            // Impolite peer wins; tag the offer to ignore.
            self.ignore_offer.store(true, Ordering::Release);
            NegotiationAction::Ignore
        } else if offer_collision {
            // Polite peer yields — rollback then apply.
            NegotiationAction::Rollback
        } else {
            NegotiationAction::Apply
        }
    }

    /// Whether the last remote offer was tagged ignore (by an
    /// `Ignore` decision). Caller can use this to suppress the answer
    /// path entirely if they returned [`NegotiationAction::Ignore`].
    pub fn was_offer_ignored(&self) -> bool {
        self.ignore_offer.swap(false, Ordering::AcqRel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polite_setter_is_round_trippable() {
        let pn = PerfectNegotiation::new(true);
        assert!(pn.is_polite());
        let other = PerfectNegotiation::new(false);
        assert!(!other.is_polite());
    }

    #[test]
    fn begin_end_local_offer_toggles_state() {
        let pn = PerfectNegotiation::new(false);
        pn.begin_local_offer();
        assert!(pn.making_offer.load(Ordering::Acquire));
        pn.end_local_offer();
        assert!(!pn.making_offer.load(Ordering::Acquire));
    }
}
