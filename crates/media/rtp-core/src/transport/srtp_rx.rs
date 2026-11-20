//! The inbound SRTP/SRTCP context owned by the socket's receive loop
//! ([`MediaExecutorConfig::owner_srtp`]).
//!
//! Under v1 every inbound RTP and RTCP datagram takes the transport's
//! `parking_lot::Mutex` around the receive context, shared with the control
//! plane (rekey, rollback, `srtp_enabled`). The context belongs to one call
//! and its only per-packet user is the receive loop, so here the loop *checks
//! the installed context out* and keeps it: the per-datagram path is one
//! Acquire load of an epoch counter and a compare. The mutex is taken only
//! when the epoch moved (a context was installed, replaced, rolled back or
//! removed), which is rekey rate.
//!
//! Semantics are unchanged because the crypto object is the same object:
//! authentication, the 64-packet replay windows (SRTP and SRTCP), ROC
//! estimation and every commit order live inside `SrtpContext` and are not
//! touched. Only *where* the context is while it is in use changes, and the
//! hand-off rules make every observable transition equal to v1's:
//!
//! - **Install / replace** (`replace_srtp_contexts`): the new context goes
//!   home and the epoch moves. The receive loop sees the new epoch before its
//!   next datagram, gives back the context it held and takes the new one, so
//!   no datagram after the replacement is processed with the old keys (as in
//!   v1, where the mutex serialises the two). A datagram the loop was already
//!   processing when the replacement happened is ordered before it, exactly
//!   as if it had held the v1 mutex first.
//! - **The previous context** that v1 moves into the rollback token is the
//!   one the loop holds (if the loop has it checked out). The token records
//!   only its identity ([`PreviousRecv::Owner`]); the loop gives it back
//!   unchanged when it adopts the replacement (`SrtpRecvSlot::returned`).
//!   Its replay and ROC state then equal v1's token: frozen at the moment of
//!   the replacement, because no later datagram used it.
//! - **Rollback** restores that exact context: from `returned` when the loop
//!   already gave it back, or by re-installing the identity the loop still
//!   holds when it never adopted the replacement (no datagram arrived in
//!   between). A stale token is refused before this module is reached
//!   (`srtp_context_generation`, unchanged).
//! - **Teardown** gives the held context back (home when it is still the
//!   installed one), so `srtp_enabled` and a later receive loop see it.
//!
//! Only a loop that is the context's sole datagram user owns it: a transport
//! with a separate RTCP socket, whose own task unprotects SRTCP with the same
//! context, keeps the shared mutex.
//!
//! [`MediaExecutorConfig::owner_srtp`]: crate::executor::MediaExecutorConfig::owner_srtp

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::executor::{record_owner_srtp_handoff, record_owner_srtp_loop, SrtpOwnerCounters};
use crate::srtp::SrtpContext;

/// The inbound SRTP context of one transport and its hand-off state.
pub(crate) struct SrtpRecvShared {
    /// Moves (under `slot`'s lock) whenever the installed context changes.
    /// The receive owner compares it once per datagram.
    epoch: AtomicU64,
    slot: parking_lot::Mutex<SrtpRecvSlot>,
}

/// Identity and cached validation of the installed context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Installed {
    id: u64,
    /// `validate_for_secure_transport` of the context, which only depends on
    /// its suites and `enabled` flag; neither changes while it is installed.
    valid: bool,
}

/// Where the inbound context is. Accessed under `SrtpRecvShared::lock`.
pub(crate) struct SrtpRecvSlot {
    /// The installed context while it is at home. Under v1, and whenever no
    /// receive loop owns it, this is the only copy and is used under the
    /// lock exactly as before.
    pub(crate) home: Option<SrtpContext>,
    /// The installed context's identity; `None` when none is installed.
    installed: Option<Installed>,
    /// Identity of the context the receive owner holds, if any.
    owner: Option<u64>,
    /// A context the owner gave back when it adopted a newer installation,
    /// kept so the rollback token of that replacement can restore it
    /// exactly. Cleared by the next replacement or rollback.
    returned: Option<(u64, SrtpContext)>,
    next_id: u64,
}

/// What a replacement displaced: v1 moved the previous context itself into
/// the rollback token; here it may be held by the receive owner.
pub(crate) enum PreviousRecv {
    None,
    Home(SrtpContext),
    /// The receive owner held the displaced context (identity `id`).
    Owner {
        id: u64,
        valid: bool,
    },
}

impl SrtpRecvSlot {
    fn fresh_id(&mut self) -> u64 {
        self.next_id = self.next_id.wrapping_add(1);
        self.next_id
    }

    /// Whether an installed context exists and validates (v1:
    /// `home.is_some_and(validate)`).
    pub(crate) fn installed_valid(&self) -> bool {
        match (&self.home, self.installed) {
            (Some(context), _) => context.validate_for_secure_transport().is_ok(),
            (None, Some(installed)) => installed.valid,
            (None, None) => false,
        }
    }

    /// Whether the installed context is checked out by the receive loop.
    pub(crate) fn owned_by_receiver(&self) -> bool {
        self.home.is_none() && self.installed.is_some_and(|i| self.owner == Some(i.id))
    }
}

impl SrtpRecvShared {
    pub(crate) fn new() -> Self {
        Self {
            epoch: AtomicU64::new(0),
            slot: parking_lot::Mutex::new(SrtpRecvSlot {
                home: None,
                installed: None,
                owner: None,
                returned: None,
                next_id: 0,
            }),
        }
    }

    pub(crate) fn lock(&self) -> parking_lot::MutexGuard<'_, SrtpRecvSlot> {
        self.slot.lock()
    }

    #[inline]
    fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    /// Install `context` (the caller holds `slot`, this transport's lock)
    /// and return what it displaced.
    pub(crate) fn install(&self, slot: &mut SrtpRecvSlot, context: SrtpContext) -> PreviousRecv {
        let previous = match (slot.home.take(), slot.installed) {
            (Some(displaced), _) => PreviousRecv::Home(displaced),
            (None, Some(installed)) if slot.owner == Some(installed.id) => PreviousRecv::Owner {
                id: installed.id,
                valid: installed.valid,
            },
            (None, _) => PreviousRecv::None,
        };
        // Only the newest replacement's token can still restore; an older
        // one is refused by the transport's generation fence.
        slot.returned = None;
        let id = slot.fresh_id();
        slot.installed = Some(Installed {
            id,
            valid: context.validate_for_secure_transport().is_ok(),
        });
        slot.home = Some(context);
        self.epoch.fetch_add(1, Ordering::Release);
        previous
    }

    /// Whether [`Self::restore`] can put `previous` back exactly. Checked
    /// before a rollback changes anything.
    pub(crate) fn can_restore(&self, slot: &SrtpRecvSlot, previous: &PreviousRecv) -> bool {
        match previous {
            PreviousRecv::Owner { id, .. } => {
                slot.returned
                    .as_ref()
                    .is_some_and(|(returned, _)| returned == id)
                    || slot.owner == Some(*id)
            }
            PreviousRecv::None | PreviousRecv::Home(_) => true,
        }
    }

    /// Retire the installed context and reinstall `previous` (the caller
    /// checked [`Self::can_restore`] under the same lock).
    pub(crate) fn restore(&self, slot: &mut SrtpRecvSlot, previous: PreviousRecv) {
        // At home: dropped here. Held by the owner: given back at its next
        // datagram, when it sees the new epoch.
        slot.home = None;
        let returned = slot.returned.take();
        match previous {
            PreviousRecv::None => slot.installed = None,
            PreviousRecv::Home(context) => {
                let id = slot.fresh_id();
                slot.installed = Some(Installed {
                    id,
                    valid: context.validate_for_secure_transport().is_ok(),
                });
                slot.home = Some(context);
            }
            PreviousRecv::Owner { id, valid } => {
                match returned {
                    // The owner adopted the replacement and gave this back.
                    Some((returned_id, context)) if returned_id == id => {
                        slot.home = Some(context);
                    }
                    // The owner still holds it (no datagram since the
                    // replacement): it keeps it at its next sync.
                    _ => debug_assert_eq!(slot.owner, Some(id)),
                }
                slot.installed = Some(Installed { id, valid });
            }
        }
        self.epoch.fetch_add(1, Ordering::Release);
    }
}

/// The receive loop's side: the context it holds and the epoch it saw.
pub(crate) struct OwnerSrtpRx {
    shared: Arc<SrtpRecvShared>,
    /// Epoch at the last sync; `None` before the first datagram.
    seen_epoch: Option<u64>,
    held: Option<(u64, SrtpContext)>,
    counters: SrtpOwnerCounters,
}

impl OwnerSrtpRx {
    pub(crate) fn new(shared: Arc<SrtpRecvShared>) -> Self {
        record_owner_srtp_loop();
        Self {
            shared,
            seen_epoch: None,
            held: None,
            counters: SrtpOwnerCounters::new(),
        }
    }

    /// The inbound context for one datagram. Lock-free unless the installed
    /// context changed since the previous datagram.
    #[cfg(test)]
    pub(crate) fn context(&mut self) -> Option<&mut SrtpContext> {
        self.context_gated(true)
    }

    /// [`Self::context`] whose epoch load is skipped when `check` is false:
    /// with hot call state the transport's doorbell has not rung since the
    /// previous check, and every installation, replacement and rollback
    /// rings it after it moves the epoch, so the skipped load would have
    /// found the epoch this loop already synced to. Before the first sync
    /// the epoch is always checked.
    #[inline]
    pub(crate) fn context_gated(&mut self, check: bool) -> Option<&mut SrtpContext> {
        if check || self.seen_epoch.is_none() {
            let epoch = self.shared.epoch();
            if self.seen_epoch != Some(epoch) {
                self.sync();
            }
        }
        match self.held.as_mut() {
            Some((_, context)) => {
                self.counters.lock_avoided();
                Some(context)
            }
            None => None,
        }
    }

    /// Adopt whatever is installed now (rekey rate).
    #[cold]
    fn sync(&mut self) {
        let mut slot = self.shared.lock();
        let installed = slot.installed.map(|installed| installed.id);
        let held = self.held.as_ref().map(|(id, _)| *id);
        if held.is_none() || held != installed {
            if let Some(previous) = self.held.take() {
                // Kept for an exact rollback of the replacement.
                slot.returned = Some(previous);
            }
            slot.owner = None;
            if let Some(id) = installed {
                if let Some(context) = slot.home.take() {
                    self.held = Some((id, context));
                    slot.owner = Some(id);
                }
            }
            if held.is_some() || installed.is_some() {
                record_owner_srtp_handoff();
            }
        }
        // Read under the lock: the epoch only moves while it is held.
        self.seen_epoch = Some(self.shared.epoch());
    }
}

impl Drop for OwnerSrtpRx {
    fn drop(&mut self) {
        let Some((id, context)) = self.held.take() else {
            return;
        };
        let mut slot = self.shared.lock();
        slot.owner = None;
        if slot.installed.map(|installed| installed.id) == Some(id) && slot.home.is_none() {
            slot.home = Some(context);
        } else {
            slot.returned = Some((id, context));
        }
    }
}

#[cfg(test)]
mod tests {
    //! Deterministic v1/owner equivalence. The v1 model is the transport's
    //! old shape: `Mutex<Option<SrtpContext>>`, a replacement swaps the
    //! context and moves the old one into the token, a rollback swaps it
    //! back. The owner model drives `SrtpRecvShared`/`OwnerSrtpRx` exactly as
    //! the transport does. Both see the same datagrams and the same control
    //! operations in the same order; every per-datagram outcome (accept with
    //! identical plaintext, or reject) must match.
    use super::*;
    use crate::packet::{RtpHeader, RtpPacket};
    use crate::srtp::{SrtpCryptoKey, SRTP_AES128_CM_SHA1_80, SRTP_AES256_CM_SHA1_80};
    use bytes::Bytes;

    const SSRC: u32 = 0x0042_0042;

    /// Two random master key/salt pairs per test process (AES-128 and
    /// AES-256), generated at run time rather than hard-coded; every context
    /// built from one of them agrees with every other.
    fn test_keys() -> &'static (([u8; 16], [u8; 14]), ([u8; 32], [u8; 14])) {
        use rand::Rng;
        type Keys = (([u8; 16], [u8; 14]), ([u8; 32], [u8; 14]));
        static KEYS: std::sync::OnceLock<Keys> = std::sync::OnceLock::new();
        KEYS.get_or_init(|| {
            let mut rng = rand::thread_rng();
            ((rng.gen(), rng.gen()), (rng.gen(), rng.gen()))
        })
    }

    fn key_a() -> SrtpContext {
        let (key, salt) = test_keys().0;
        SrtpContext::new(
            SRTP_AES128_CM_SHA1_80,
            SrtpCryptoKey::new(key.to_vec(), salt.to_vec()),
        )
        .unwrap()
    }

    fn key_b() -> SrtpContext {
        let (key, salt) = test_keys().1;
        SrtpContext::new(
            SRTP_AES256_CM_SHA1_80,
            SrtpCryptoKey::new(key.to_vec(), salt.to_vec()),
        )
        .unwrap()
    }

    fn rtp(seq: u16) -> RtpPacket {
        RtpPacket::new(
            RtpHeader::new(0, seq, u32::from(seq).wrapping_mul(160), SSRC),
            Bytes::from(vec![(seq % 251) as u8; 160]),
        )
    }

    fn rtcp_rr() -> Vec<u8> {
        crate::packet::rtcp::RtcpCompoundPacket::new_with_rr(
            crate::packet::rtcp::RtcpReceiverReport::new(SSRC),
        )
        .serialize()
        .unwrap()
        .to_vec()
    }

    /// One step of a script.
    enum Step {
        Rtp(Vec<u8>),
        Rtcp(Vec<u8>),
        Replace(fn() -> SrtpContext),
        Rollback,
        Remove,
    }

    #[derive(Debug, PartialEq, Eq)]
    enum Outcome {
        Rtp(Option<(u16, Vec<u8>)>),
        Rtcp(Option<Vec<u8>>),
        Control,
    }

    fn unprotect_rtp(context: Option<&mut SrtpContext>, wire: &[u8]) -> Outcome {
        let Some(context) = context else {
            return Outcome::Rtp(None);
        };
        // The inline path decrypts in place; both models use it.
        let mut data = wire.to_vec();
        Outcome::Rtp(
            context
                .unprotect_in_place(&mut data)
                .ok()
                .map(|bounds| (bounds.header.sequence_number, data[bounds.payload].to_vec())),
        )
    }

    fn unprotect_rtcp(context: Option<&mut SrtpContext>, wire: &[u8]) -> Outcome {
        Outcome::Rtcp(
            context.and_then(|context| context.unprotect_rtcp(wire).ok().map(|b| b.to_vec())),
        )
    }

    fn run_v1(script: &[Step]) -> Vec<Outcome> {
        let recv = parking_lot::Mutex::new(None::<SrtpContext>);
        let mut token: Option<Option<SrtpContext>> = None;
        script
            .iter()
            .map(|step| match step {
                Step::Rtp(wire) => unprotect_rtp(recv.lock().as_mut(), wire),
                Step::Rtcp(wire) => unprotect_rtcp(recv.lock().as_mut(), wire),
                Step::Replace(make) => {
                    token = Some(std::mem::replace(&mut *recv.lock(), Some(make())));
                    Outcome::Control
                }
                Step::Rollback => {
                    *recv.lock() = token.take().expect("a replacement to roll back");
                    Outcome::Control
                }
                Step::Remove => {
                    token = None;
                    *recv.lock() = None;
                    Outcome::Control
                }
            })
            .collect()
    }

    fn run_owner(script: &[Step]) -> (Vec<Outcome>, Arc<SrtpRecvShared>) {
        let shared = Arc::new(SrtpRecvShared::new());
        let mut owner = OwnerSrtpRx::new(shared.clone());
        let mut token: Option<PreviousRecv> = None;
        let outcomes = script
            .iter()
            .map(|step| match step {
                Step::Rtp(wire) => unprotect_rtp(owner.context(), wire),
                Step::Rtcp(wire) => unprotect_rtcp(owner.context(), wire),
                Step::Replace(make) => {
                    let mut slot = shared.lock();
                    token = Some(shared.install(&mut slot, make()));
                    Outcome::Control
                }
                Step::Rollback => {
                    let previous = token.take().expect("a replacement to roll back");
                    let mut slot = shared.lock();
                    assert!(shared.can_restore(&slot, &previous));
                    shared.restore(&mut slot, previous);
                    Outcome::Control
                }
                Step::Remove => {
                    token = None;
                    let mut slot = shared.lock();
                    shared.restore(&mut slot, PreviousRecv::None);
                    Outcome::Control
                }
            })
            .collect();
        drop(owner);
        (outcomes, shared)
    }

    fn assert_equivalent(script: &[Step]) -> Vec<Outcome> {
        let v1 = run_v1(script);
        let (owner, _) = run_owner(script);
        assert_eq!(v1, owner);
        owner
    }

    fn protect(sender: &mut SrtpContext, seq: u16) -> Vec<u8> {
        sender
            .protect(&rtp(seq))
            .unwrap()
            .serialize()
            .unwrap()
            .to_vec()
    }

    fn accepted(outcomes: &[Outcome]) -> Vec<u16> {
        outcomes
            .iter()
            .filter_map(|outcome| match outcome {
                Outcome::Rtp(Some((seq, _))) => Some(*seq),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn rekey_mid_stream_matches_v1() {
        let mut a = key_a();
        let mut b = key_b();
        let a1 = protect(&mut a, 1);
        let a2 = protect(&mut a, 2);
        let a3 = protect(&mut a, 3);
        let a4 = protect(&mut a, 4);
        let b5 = protect(&mut b, 5);
        let b6 = protect(&mut b, 6);
        let script = [
            Step::Replace(key_a),
            Step::Rtp(a1),
            Step::Rtp(a2.clone()),
            Step::Rtp(a3),
            Step::Replace(key_b),
            // Old keys after the rekey: rejected.
            Step::Rtp(a4),
            Step::Rtp(b5.clone()),
            Step::Rtp(b6),
            // Replays under the new keys and across keys.
            Step::Rtp(b5),
            Step::Rtp(a2),
        ];
        let outcomes = assert_equivalent(&script);
        assert_eq!(accepted(&outcomes), vec![1, 2, 3, 5, 6]);
    }

    #[test]
    fn replay_window_and_reordering_match_v1() {
        let mut a = key_a();
        let wires: Vec<Vec<u8>> = (100..=200).map(|seq| protect(&mut a, seq)).collect();
        let mut script = vec![Step::Replace(key_a)];
        // In order, then a reordered pair, duplicates, a packet just inside
        // and one just outside the 64-packet window.
        for wire in &wires[..80] {
            script.push(Step::Rtp(wire.clone()));
        }
        script.push(Step::Rtp(wires[85].clone()));
        script.push(Step::Rtp(wires[82].clone()));
        script.push(Step::Rtp(wires[85].clone()));
        script.push(Step::Rtp(wires[82].clone()));
        script.push(Step::Rtp(wires[30].clone()));
        script.push(Step::Rtp(wires[10].clone()));
        let mut tampered = wires[90].clone();
        let middle = tampered.len() / 2;
        tampered[middle] ^= 1;
        script.push(Step::Rtp(tampered));
        script.push(Step::Rtp(wires[90].clone()));
        let outcomes = assert_equivalent(&script);
        let accepted = accepted(&outcomes);
        assert_eq!(accepted.len(), 83, "80 in order, 185, 182, 190");
    }

    #[test]
    fn roc_rollover_across_a_rekey_and_rollback_matches_v1() {
        let mut a = key_a();
        let mut b = key_b();
        // Sequence numbers across the 16-bit wrap: ROC moves 0 → 1.
        let before: Vec<Vec<u8>> = (65_530..=65_535).map(|seq| protect(&mut a, seq)).collect();
        let after_wrap: Vec<Vec<u8>> = (0..=4).map(|seq| protect(&mut a, seq)).collect();
        let b_wires: Vec<Vec<u8>> = (7..=9).map(|seq| protect(&mut b, seq)).collect();
        let mut script = vec![Step::Replace(key_a)];
        script.extend(before.iter().cloned().map(Step::Rtp));
        script.extend(after_wrap[..2].iter().cloned().map(Step::Rtp));
        // Rekey, receive under B, then roll back: A must continue with its
        // ROC (1) and its replay window exactly as frozen at the rekey.
        script.push(Step::Replace(key_b));
        script.extend(b_wires.iter().cloned().map(Step::Rtp));
        script.push(Step::Rollback);
        script.push(Step::Rtp(after_wrap[1].clone())); // replay: rejected
        script.extend(after_wrap[2..].iter().cloned().map(Step::Rtp));
        script.push(Step::Rtp(before[5].clone())); // pre-wrap replay: rejected
        script.push(Step::Rtp(b_wires[2].clone())); // B after rollback: rejected
        let outcomes = assert_equivalent(&script);
        assert_eq!(
            accepted(&outcomes),
            vec![65_530, 65_531, 65_532, 65_533, 65_534, 65_535, 0, 1, 7, 8, 9, 2, 3, 4]
        );
    }

    #[test]
    fn rollback_before_the_owner_adopted_the_replacement_keeps_the_held_context() {
        let mut a = key_a();
        let wires: Vec<Vec<u8>> = (1..=6).map(|seq| protect(&mut a, seq)).collect();
        // No datagram between the replacement and its rollback: the owner
        // never adopted B and keeps A with its state.
        let script = [
            Step::Replace(key_a),
            Step::Rtp(wires[0].clone()),
            Step::Rtp(wires[1].clone()),
            Step::Replace(key_b),
            Step::Rollback,
            Step::Rtp(wires[1].clone()),
            Step::Rtp(wires[2].clone()),
            Step::Rtp(wires[5].clone()),
            Step::Rtp(wires[3].clone()),
        ];
        let outcomes = assert_equivalent(&script);
        assert_eq!(accepted(&outcomes), vec![1, 2, 3, 6, 4]);
    }

    #[test]
    fn srtcp_replay_and_rekey_match_v1() {
        let mut a = key_a();
        let mut b = key_b();
        let report = rtcp_rr();
        let a1 = a.protect_rtcp(&report).unwrap().to_vec();
        let a2 = a.protect_rtcp(&report).unwrap().to_vec();
        let b1 = b.protect_rtcp(&report).unwrap().to_vec();
        let mut c = key_a();
        let rtp1 = protect(&mut c, 10);
        let script = [
            Step::Replace(key_a),
            Step::Rtcp(a1.clone()),
            Step::Rtp(rtp1),
            Step::Rtcp(a1.clone()), // SRTCP replay
            Step::Rtcp(a2.clone()),
            Step::Replace(key_b),
            Step::Rtcp(a2.clone()),
            Step::Rtcp(b1.clone()),
            Step::Rollback,
            Step::Rtcp(a2), // replay after rollback: rejected
            Step::Rtcp(b1),
            Step::Remove,
            Step::Rtcp(a1),
        ];
        let outcomes = assert_equivalent(&script);
        let rtcp_accepted = outcomes
            .iter()
            .filter(|outcome| matches!(outcome, Outcome::Rtcp(Some(_))))
            .count();
        assert_eq!(rtcp_accepted, 3, "a1, a2 and b1 once each");
    }

    #[test]
    fn repeated_rekeys_and_rollbacks_never_resurrect_a_superseded_context() {
        let mut a = key_a();
        let mut b = key_b();
        let a_wires: Vec<Vec<u8>> = (1..=4).map(|seq| protect(&mut a, seq)).collect();
        let b_wires: Vec<Vec<u8>> = (1..=4).map(|seq| protect(&mut b, seq)).collect();
        let script = [
            Step::Replace(key_a),
            Step::Rtp(a_wires[0].clone()),
            Step::Replace(key_b),
            Step::Rtp(b_wires[0].clone()),
            Step::Replace(key_a), // a fresh A: its own replay state
            Step::Rtp(a_wires[0].clone()),
            Step::Rollback, // back to B (with b1 seen)
            Step::Rtp(b_wires[0].clone()),
            Step::Rtp(b_wires[1].clone()),
            Step::Rtp(a_wires[1].clone()),
            Step::Replace(key_a),
            Step::Rollback,
            Step::Rtp(b_wires[2].clone()),
            Step::Remove,
            Step::Rtp(b_wires[3].clone()),
        ];
        let outcomes = assert_equivalent(&script);
        assert_eq!(accepted(&outcomes), vec![1, 1, 1, 2, 3]);
    }

    #[test]
    fn teardown_returns_the_held_context_home_and_releases_everything() {
        let mut a = key_a();
        let wire = protect(&mut a, 1);
        let replay = wire.clone();
        let (outcomes, shared) = run_owner(&[Step::Replace(key_a), Step::Rtp(wire)]);
        assert_eq!(accepted(&outcomes), vec![1]);
        {
            let slot = shared.lock();
            assert!(
                slot.home.is_some(),
                "the owner's drop gives the context back"
            );
            assert!(slot.owner.is_none());
            assert!(slot.returned.is_none());
            assert!(slot.installed_valid());
        }
        // A later user (v1 path, or a new owner) continues with the state:
        // the replay is still rejected.
        let mut next = OwnerSrtpRx::new(shared.clone());
        assert_eq!(unprotect_rtp(next.context(), &replay), Outcome::Rtp(None));
        assert!(shared.lock().owned_by_receiver());
        drop(next);
        assert!(!shared.lock().owned_by_receiver());
        assert_eq!(
            Arc::strong_count(&shared),
            1,
            "no owner keeps the slot alive"
        );
        let mut slot = shared.lock();
        shared.restore(&mut slot, PreviousRecv::None);
        assert!(slot.home.is_none() && !slot.installed_valid());
    }

    #[test]
    fn the_per_datagram_path_takes_no_lock_until_the_epoch_moves() {
        let shared = Arc::new(SrtpRecvShared::new());
        {
            let mut slot = shared.lock();
            let _ = shared.install(&mut slot, key_a());
        }
        let mut owner = OwnerSrtpRx::new(shared.clone());
        assert!(owner.context().is_some());
        // Holding the lock elsewhere does not block the owner: it never
        // touches the mutex while the epoch is unchanged.
        let guard = shared.lock();
        for _ in 0..10 {
            assert!(owner.context().is_some());
        }
        assert!(guard.owned_by_receiver());
        assert!(
            guard.installed_valid(),
            "validation is cached while checked out"
        );
        drop(guard);
        drop(owner);
    }

    #[test]
    fn a_disabled_context_reports_invalid_while_checked_out() {
        let shared = Arc::new(SrtpRecvShared::new());
        let mut disabled = key_a();
        disabled.set_enabled(false);
        {
            let mut slot = shared.lock();
            let _ = shared.install(&mut slot, disabled);
            assert!(!slot.installed_valid());
        }
        let mut owner = OwnerSrtpRx::new(shared.clone());
        assert!(owner.context().is_some());
        assert!(!shared.lock().installed_valid());
    }
}
