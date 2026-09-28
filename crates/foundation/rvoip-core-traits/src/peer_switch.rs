//! Shared channel-delivery boundary for staged bidirectional peer routes.
//! This does not replace transport buffers or commit bridge ownership.
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, watch};

struct State {
    current: u64,
    retired: bool,
    next: u64,
    deliveries: usize,
    paused: bool,
    idle: watch::Sender<usize>,
    changes: watch::Sender<u64>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            current: 0,
            retired: false,
            next: 0,
            deliveries: 0,
            paused: false,
            idle: watch::channel(0).0,
            changes: watch::channel(0).0,
        }
    }
}

/// Both directions of one speaking peer use clones of the same ticket.
#[derive(Clone)]
pub struct PeerRouteTicket {
    state: Arc<Mutex<State>>,
    generation: u64,
}

/// A transport holds this guard through its awaited local send operation.
/// Dropping it only releases the commit fence; it does not cancel a send or
/// establish remote delivery. Do not detach work that outlives this guard.
#[must_use]
pub struct PeerDeliveryGuard {
    state: Arc<Mutex<State>>,
}

impl Drop for PeerDeliveryGuard {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.deliveries -= 1;
        if state.deliveries == 0 {
            state.idle.send_replace(0);
        }
    }
}

/// Owns a temporary stop in delivery admission. Cancellation or dropping this
/// guard restores the old peer unless it has committed or retired meanwhile.
#[must_use]
pub struct PeerQuiescenceGuard {
    ticket: PeerRouteTicket,
}

impl Drop for PeerQuiescenceGuard {
    fn drop(&mut self) {
        let mut state = self.ticket.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.current == self.ticket.generation && !state.retired {
            state.paused = false;
        }
    }
}

impl PeerRouteTicket {
    /// Create the initial active peer. Staged tickets do not forward media.
    pub fn initial() -> Self {
        Self {
            state: Arc::new(Mutex::new(State::default())),
            generation: 0,
        }
    }

    /// Whether this generation currently owns speaking delivery.
    pub fn is_active(&self) -> bool {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        !state.retired && !state.paused && state.current == self.generation
    }

    /// Current ownership independent of a temporary delivery pause.
    pub fn is_current(&self) -> bool {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        !state.retired && state.current == self.generation
    }

    /// Stop new delivery and await the existing send guards. Wrap this future
    /// in a caller-selected timeout; dropping it restores admission. No mutex
    /// is held over the wait. A competing pause or retired generation fails.
    pub async fn quiesce(&self) -> Option<PeerQuiescenceGuard> {
        let mut idle = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.retired || state.paused || state.current != self.generation {
                return None;
            }
            state.paused = true;
            state.changes.send_replace(state.current);
            state.idle.subscribe()
        };
        let guard = PeerQuiescenceGuard {
            ticket: self.clone(),
        };
        loop {
            {
                let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                if state.retired || state.current != self.generation {
                    return None;
                }
                if state.deliveries == 0 {
                    return Some(guard);
                }
            }
            if idle.changed().await.is_err() {
                return None;
            }
        }
    }

    /// Admit one transport delivery only while this peer is current. Admission
    /// and commit use the same lock, so a successful commit cannot overlap an
    /// admitted old-peer send. Call only after dequeueing at the transport.
    pub fn try_begin_delivery(&self) -> Option<PeerDeliveryGuard> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.retired || state.paused || state.current != self.generation {
            return None;
        }
        state.deliveries = state.deliveries.checked_add(1)?;
        Some(PeerDeliveryGuard {
            state: Arc::clone(&self.state),
        })
    }

    /// Reserve a distinct inactive generation, without changing the live peer.
    pub fn stage(&self) -> Option<Self> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.retired {
            return None;
        }
        state.next = state.next.checked_add(1)?;
        Some(Self {
            state: self.state.clone(),
            generation: state.next,
        })
    }

    /// Commit only if the expected peer still owns delivery. A stale or foreign
    /// ticket cannot switch media, and an old generation cannot be reactivated.
    pub fn commit_from(&self, expected: &Self) -> bool {
        if !Arc::ptr_eq(&self.state, &expected.state) {
            return false;
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.retired
            || state.deliveries != 0
            || state.current != expected.generation
            || self.generation <= state.current
        {
            return false;
        }
        state.current = self.generation;
        state.paused = false;
        state.changes.send_replace(self.generation);
        true
    }

    /// Permanently stop this switch only if this ticket is still authoritative.
    /// Cleanup of an old bridge after cutover must not retire its successor.
    pub fn retire_if_current(&self) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.retired || state.current != self.generation {
            return false;
        }
        state.retired = true;
        state.idle.send_replace(state.deliveries);
        state.changes.send_replace(state.current);
        true
    }

    /// Forward only for this active generation. Capacity is reserved without
    /// holding the switch lock; the final check and channel send share the same
    /// lock as commit. Thus a send blocked before commit cannot publish later
    /// under the old peer. Frames already accepted by the transport are outside
    /// this channel boundary and still require transport-level cutoff handling.
    pub async fn forward<T>(
        &self,
        target: &mpsc::Sender<T>,
        frame: T,
    ) -> Result<bool, mpsc::error::SendError<T>> {
        let mut changes = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.retired || state.paused || state.current != self.generation {
                return Ok(false);
            }
            state.changes.subscribe()
        };
        let capacity = tokio::select! {
            biased;
            _ = changes.changed() => return Ok(false),
            capacity = target.reserve() => capacity,
        };
        let permit = match capacity {
            Ok(permit) => permit,
            Err(_) => return Err(mpsc::error::SendError(frame)),
        };
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.retired || state.paused || state.current != self.generation {
            return Ok(false);
        }
        permit.send(frame);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn quiescence_timeout_restores_admission() {
        let old = PeerRouteTicket::initial();
        let sending = old.try_begin_delivery().unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(5), old.quiesce())
                .await
                .is_err()
        );
        assert!(old.is_active());
        assert!(old.try_begin_delivery().is_some());
        drop(sending);
    }

    #[tokio::test]
    async fn quiescence_drains_sends_and_commit_preserves_successor() {
        let old = PeerRouteTicket::initial();
        let next = old.stage().unwrap();
        let sending = old.try_begin_delivery().unwrap();
        let pending = {
            let old = old.clone();
            tokio::spawn(async move { old.quiesce().await })
        };
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while old.is_active() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!pending.is_finished());
        assert!(old.try_begin_delivery().is_none());
        drop(sending);
        let paused = pending.await.unwrap().unwrap();
        assert!(old.is_current());
        assert!(!old.is_active());
        assert!(next.commit_from(&old));
        drop(paused);
        assert!(next.is_active());
    }

    #[tokio::test]
    async fn retiring_a_paused_peer_wakes_quiescence() {
        let old = PeerRouteTicket::initial();
        let _sending = old.try_begin_delivery().unwrap();
        let pending = {
            let old = old.clone();
            tokio::spawn(async move { old.quiesce().await })
        };
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while old.is_active() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(old.retire_if_current());
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), pending)
                .await
                .unwrap()
                .unwrap()
                .is_none()
        );
        assert!(!old.is_current());
    }

    #[test]
    fn in_flight_transport_delivery_fences_commit_in_both_directions() {
        let old = PeerRouteTicket::initial();
        let next = old.stage().unwrap();
        assert!(next.try_begin_delivery().is_none());
        let forward = old.try_begin_delivery().unwrap();
        let reverse = old.clone().try_begin_delivery().unwrap();
        assert!(!next.commit_from(&old));
        drop(forward);
        assert!(!next.commit_from(&old));
        drop(reverse);
        assert!(next.commit_from(&old));
        assert!(old.try_begin_delivery().is_none());
        assert!(next.try_begin_delivery().is_some());
    }

    #[test]
    fn retirement_prevents_new_delivery_while_existing_guard_can_finish() {
        let old = PeerRouteTicket::initial();
        let next = old.stage().unwrap();
        let in_flight = old.try_begin_delivery().unwrap();
        assert!(old.retire_if_current());
        assert!(old.try_begin_delivery().is_none());
        drop(in_flight);
        assert!(!next.commit_from(&old));
    }

    #[tokio::test]
    async fn staged_routes_are_silent_and_both_directions_switch_together() {
        let old = PeerRouteTicket::initial();
        let new = old.stage().unwrap();
        let reverse = new.clone();
        let (tx, mut rx) = mpsc::channel(4);
        assert!(!new.forward(&tx, 1).await.unwrap());
        assert!(old.forward(&tx, 2).await.unwrap());
        assert_eq!(rx.recv().await, Some(2));
        assert!(new.commit_from(&old));
        assert!(!old.forward(&tx, 3).await.unwrap());
        assert!(new.forward(&tx, 4).await.unwrap());
        assert!(reverse.forward(&tx, 5).await.unwrap());
        assert_eq!(rx.recv().await, Some(4));
        assert_eq!(rx.recv().await, Some(5));
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn blocked_old_send_cannot_cross_the_commit_boundary() {
        let old = PeerRouteTicket::initial();
        let new = old.stage().unwrap();
        let (tx, mut rx) = mpsc::channel(1);
        tx.send(1).await.unwrap();
        let send = old.forward(&tx, 2);
        tokio::pin!(send);
        // Poll the send to establish that it is waiting on a full channel.
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut send)
                .await
                .is_err()
        );
        assert!(new.commit_from(&old));
        assert_eq!(rx.recv().await, Some(1));
        assert!(!send.await.unwrap());
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn commit_wakes_old_sender_without_freeing_transport_capacity() {
        let old = PeerRouteTicket::initial();
        let next = old.stage().unwrap();
        let (tx, mut rx) = mpsc::channel(1);
        tx.send(1).await.unwrap();
        let send = old.forward(&tx, 2);
        tokio::pin!(send);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut send)
                .await
                .is_err()
        );
        assert!(next.commit_from(&old));
        assert!(
            !tokio::time::timeout(std::time::Duration::from_secs(1), &mut send)
                .await
                .unwrap()
                .unwrap()
        );
        assert_eq!(rx.recv().await, Some(1));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn retirement_prevents_late_commit_but_old_cleanup_preserves_successor() {
        let old = PeerRouteTicket::initial();
        let staged = old.stage().unwrap();
        assert!(old.retire_if_current());
        assert!(!old.is_active());
        assert!(old.stage().is_none());
        assert!(!staged.commit_from(&old));

        let old = PeerRouteTicket::initial();
        let next = old.stage().unwrap();
        assert!(next.commit_from(&old));
        assert!(!old.retire_if_current());
        assert!(next.is_active());
        assert!(next.retire_if_current());
        assert!(!next.is_active());
    }

    #[test]
    fn competing_stale_foreign_and_reactivated_tickets_are_refused() {
        let old = PeerRouteTicket::initial();
        let first = old.stage().unwrap();
        let second = old.stage().unwrap();
        assert!(!first.commit_from(&PeerRouteTicket::initial()));
        assert!(second.commit_from(&old));
        assert!(!first.commit_from(&old));
        assert!(!first.commit_from(&second));
        assert!(!old.commit_from(&second));
        assert!(!second.commit_from(&old));
        let next = second.stage().unwrap();
        assert!(next.commit_from(&second));
    }
}
