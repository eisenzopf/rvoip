//! Hot call state: the per-call state a socket owner touches for every
//! packet, packed onto as few cache lines as possible
//! ([`MediaExecutorConfig::hot_state`], executor v2).
//!
//! With thousands of calls per media lane, every call's state is evicted
//! from the core's caches between two of its packets (20 ms apart), so the
//! memory cost of a packet is, to first order, the number of distinct cache
//! lines it touches. These building blocks cut that number without changing
//! any observable behaviour:
//!
//! - `CacheAligned`: a 64-byte aligned wrapper, so a packed group of hot
//!   fields starts on a line and never straddles one it does not need.
//! - `HotDoorbell`: one cache line per transport that every control-plane
//!   writer of the receive loop's fenced state (transport active, the
//!   symmetric-RTP generation and latch, the remote RTP address, the inbound
//!   SRTP context) rings after its own store. The socket owner reads that one
//!   line per datagram and re-reads the individual atomics (each in its own
//!   allocation) only when the bell moved, which happens at control-plane
//!   rate.
//! - `HotSlab` and `HotHandle`: a lane-owned (thread-local, never
//!   shared) slab of cache-line-aligned entries addressed by a small
//!   `(index, generation)` handle. A slot is reused only after its entry was
//!   removed, and its generation moves on at every removal, so a stale handle
//!   never reaches a later occupant; a slot whose generation would wrap is
//!   retired instead of reused.
//! - `prefetch_read`: a software prefetch for batch points (the lane
//!   clock's due list); a no-op off x86_64.
//!
//! What uses them, all decided when the object is built and never per
//! packet: the SRTP context's packed per-direction block (`srtp`), the
//! receive loop's doorbell gating and lane-slab entry (`transport::rx_hot`),
//! the synchronous TX path's destination cache (`session`), the inline
//! ingress's settled sink slot and write-only loss history (`session`), and
//! the lane clock's due-entry prefetch (`lane_clock`).
//!
//! [`MediaExecutorConfig::hot_state`]: crate::executor::MediaExecutorConfig::hot_state

use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU64, Ordering};

/// Cache line size the packing assumes (x86_64 and most aarch64 parts);
/// the layout tests check every packed type against it.
#[cfg(test)]
pub(crate) const CACHE_LINE: usize = 64;

/// A snapshot value no doorbell ever reaches: a fresh snapshot always
/// differs from the bell, so the first datagram runs every check.
pub(crate) const NEVER_SEEN: u64 = u64::MAX;

/// A value that starts on its own cache line and is padded to whole lines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C, align(64))]
pub(crate) struct CacheAligned<T>(pub(crate) T);

impl<T> Deref for CacheAligned<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for CacheAligned<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

/// One cache line per transport, rung by every control-plane writer of the
/// state its socket owner checks per datagram (see the module docs).
///
/// Protocol: a writer performs its own store first and rings after it
/// (`Release`). The owner reads the bell (`Acquire`) after it received a
/// datagram. When the bell equals its snapshot, no writer completed a ring
/// since the snapshot, so every fenced atomic still holds the value the owner
/// last checked, or a writer stored and has not rung yet, which is the same
/// as the owner having checked just before that store. When the bell moved,
/// the owner re-runs the original checks, in their original places, and
/// takes the new snapshot.
#[derive(Debug, Default)]
#[repr(C, align(64))]
pub(crate) struct HotDoorbell {
    bell: AtomicU64,
}

impl HotDoorbell {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// After a control-plane store the owner must observe.
    #[inline]
    pub(crate) fn ring(&self) {
        self.bell.fetch_add(1, Ordering::Release);
    }

    /// The owner's per-datagram read.
    #[inline]
    pub(crate) fn read(&self) -> u64 {
        self.bell.load(Ordering::Acquire)
    }
}

/// Update `snapshot` to `bell`; whether it moved (the gated check must run).
#[inline]
pub(crate) fn gate(snapshot: &mut u64, bell: u64) -> bool {
    if *snapshot == bell {
        return false;
    }
    *snapshot = bell;
    true
}

/// Prefetch the cache line holding `address` into L1 (x86_64 `PREFETCHT0`).
/// A hint only: nothing is dereferenced, any address (stale, freed or
/// unmapped) is allowed, and the result is never observable.
#[inline(always)]
#[allow(unused_unsafe)]
pub(crate) fn prefetch_read(address: usize) {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::{_mm_prefetch, _MM_HINT_T0};
        // SAFETY: PREFETCHh never faults and has no architectural effect for
        // any address (Intel SDM vol. 2B, "PREFETCHh"), and the pointer is
        // never dereferenced by Rust. The intrinsic takes a raw pointer, which
        // is why it is called in an `unsafe` block (`unused_unsafe` is allowed
        // for toolchains that declare it safe). There is no safe std API for
        // a non-blocking prefetch.
        unsafe {
            _mm_prefetch::<_MM_HINT_T0>(address as *const i8);
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = address;
    }
}

/// A small handle to a [`HotSlab`] entry. Valid while the entry it was
/// returned for is in the slab; any later occupant of the slot has a
/// different generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct HotHandle {
    index: u32,
    generation: u32,
}

impl HotHandle {
    #[cfg(test)]
    pub(crate) fn index(self) -> u32 {
        self.index
    }

    #[cfg(test)]
    pub(crate) fn generation(self) -> u32 {
        self.generation
    }
}

struct HotSlot<T> {
    value: Option<T>,
    generation: u32,
}

static NEXT_SLAB_ID: AtomicU64 = AtomicU64::new(1);

/// A lane-owned slab of cache-line-aligned entries (see the module docs).
///
/// Not shared in use: the receive loops keep their slab in a thread local of
/// the dedicated media lane thread, so no entry is ever reached from another
/// thread. `id` identifies the slab, so a handle presented to another
/// thread's slab is refused instead of reaching an unrelated entry.
pub(crate) struct HotSlab<T> {
    id: u64,
    slots: Vec<CacheAligned<HotSlot<T>>>,
    free: Vec<u32>,
    live: usize,
    retired: usize,
}

impl<T> Default for HotSlab<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> HotSlab<T> {
    pub(crate) fn new() -> Self {
        Self {
            id: NEXT_SLAB_ID.fetch_add(1, Ordering::Relaxed),
            slots: Vec::new(),
            free: Vec::new(),
            live: 0,
            retired: 0,
        }
    }

    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    /// Entries in use.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.live
    }

    /// Slots ever allocated (in use, free or retired).
    #[cfg(test)]
    pub(crate) fn slots(&self) -> usize {
        self.slots.len()
    }

    /// Slots retired because their generation would have wrapped.
    #[cfg(test)]
    pub(crate) fn retired(&self) -> usize {
        self.retired
    }

    /// Store `value`; a free slot is reused only if its last entry was
    /// removed.
    pub(crate) fn insert(&mut self, value: T) -> HotHandle {
        self.live += 1;
        if let Some(index) = self.free.pop() {
            if let Some(slot) = self.slots.get_mut(index as usize) {
                debug_assert!(slot.value.is_none(), "a free slot is empty");
                slot.value = Some(value);
                return HotHandle {
                    index,
                    generation: slot.generation,
                };
            }
        }
        let index = u32::try_from(self.slots.len()).unwrap_or(u32::MAX);
        self.slots.push(CacheAligned(HotSlot {
            value: Some(value),
            generation: 0,
        }));
        HotHandle {
            index,
            generation: 0,
        }
    }

    #[cfg(test)]
    fn slot(&self, handle: HotHandle) -> Option<&HotSlot<T>> {
        let slot = self.slots.get(handle.index as usize)?;
        (slot.generation == handle.generation && slot.value.is_some()).then_some(&slot.0)
    }

    #[cfg(test)]
    pub(crate) fn get(&self, handle: HotHandle) -> Option<&T> {
        self.slot(handle)?.value.as_ref()
    }

    pub(crate) fn get_mut(&mut self, handle: HotHandle) -> Option<&mut T> {
        let slot = self.slots.get_mut(handle.index as usize)?;
        if slot.generation != handle.generation {
            return None;
        }
        slot.value.as_mut()
    }

    /// The address of the entry's cache line, for [`prefetch_read`].
    #[cfg(test)]
    pub(crate) fn address(&self, handle: HotHandle) -> Option<usize> {
        self.slot(handle)
            .map(|slot| slot as *const HotSlot<T> as usize)
    }

    /// Take the entry out. The slot's generation moves on, so `handle` and
    /// every copy of it are dead from here on.
    pub(crate) fn remove(&mut self, handle: HotHandle) -> Option<T> {
        let slot = self.slots.get_mut(handle.index as usize)?;
        if slot.generation != handle.generation {
            return None;
        }
        let value = slot.value.take()?;
        self.live -= 1;
        match slot.generation.checked_add(1) {
            Some(next) if next != u32::MAX => {
                slot.generation = next;
                self.free.push(handle.index);
            }
            _ => {
                // Never reuse a slot whose generation would wrap: a handle
                // from its first occupant could match again.
                slot.generation = u32::MAX;
                self.retired += 1;
            }
        }
        Some(value)
    }

    #[cfg(test)]
    fn force_generation(&mut self, index: u32, generation: u32) {
        self.slots[index as usize].generation = generation;
    }
}

static HOT_SRTP_CONTEXTS: AtomicU64 = AtomicU64::new(0);
static HOT_RX_LANE_ENTRIES: AtomicU64 = AtomicU64::new(0);
static HOT_RX_LOCAL_ENTRIES: AtomicU64 = AtomicU64::new(0);
static HOT_RX_FALLBACKS: AtomicU64 = AtomicU64::new(0);
static HOT_RX_FOREIGN_DROPS: AtomicU64 = AtomicU64::new(0);
static HOT_PREFETCHES: AtomicU64 = AtomicU64::new(0);

/// Hot call state counters (process totals since start), for export by the
/// application.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct HotStateCounters {
    /// SRTP contexts built with the packed per-direction block.
    pub srtp_contexts: u64,
    /// Receive loops whose control entry lives in their lane's slab (a
    /// dedicated current-thread media lane).
    pub rx_lane_entries: u64,
    /// Receive loops that keep their control entry locally (any other
    /// runtime, where a task may move between worker threads).
    pub rx_local_entries: u64,
    /// Lane entries that could not be reached and were replaced by a local
    /// entry (expected to stay 0).
    pub rx_fallbacks: u64,
    /// Lane entries whose receive loop was dropped off its lane thread (the
    /// entry stays in that lane's slab until the thread exits; runtime
    /// teardown only).
    pub rx_foreign_drops: u64,
    /// Cache lines lane clocks prefetched for the entries they woke.
    pub prefetches: u64,
}

/// The hot call state counters.
pub fn hot_state_counters() -> HotStateCounters {
    HotStateCounters {
        srtp_contexts: HOT_SRTP_CONTEXTS.load(Ordering::Relaxed),
        rx_lane_entries: HOT_RX_LANE_ENTRIES.load(Ordering::Relaxed),
        rx_local_entries: HOT_RX_LOCAL_ENTRIES.load(Ordering::Relaxed),
        rx_fallbacks: HOT_RX_FALLBACKS.load(Ordering::Relaxed),
        rx_foreign_drops: HOT_RX_FOREIGN_DROPS.load(Ordering::Relaxed),
        prefetches: HOT_PREFETCHES.load(Ordering::Relaxed),
    }
}

/// Once per packed SRTP context (control plane).
pub(crate) fn record_hot_srtp_context() {
    HOT_SRTP_CONTEXTS.fetch_add(1, Ordering::Relaxed);
}

/// Once per receive loop (its first poll).
pub(crate) fn record_hot_rx_entry(on_lane: bool) {
    let counter = if on_lane {
        &HOT_RX_LANE_ENTRIES
    } else {
        &HOT_RX_LOCAL_ENTRIES
    };
    counter.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_hot_rx_fallback() {
    HOT_RX_FALLBACKS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_hot_rx_foreign_drop() {
    HOT_RX_FOREIGN_DROPS.fetch_add(1, Ordering::Relaxed);
}

/// Once per lane-clock round, not per packet.
pub(crate) fn record_hot_prefetches(lines: u64) {
    if lines > 0 {
        HOT_PREFETCHES.fetch_add(lines, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{
        hot_state_enabled, override_media_executor_config_on_this_thread, MediaExecutorConfig,
    };
    use std::collections::HashSet;

    #[test]
    fn the_switch_is_on_in_v2_off_in_v1_and_decided_per_thread() {
        assert!(!MediaExecutorConfig::v1().hot_state);
        assert!(!MediaExecutorConfig::v1().hot_state_active());
        assert!(MediaExecutorConfig::v2().hot_state);
        assert!(MediaExecutorConfig::v2().hot_state_active());
        {
            let _v1 = override_media_executor_config_on_this_thread(
                MediaExecutorConfig::v1().with_hot_state(true),
            );
            assert!(!hot_state_enabled(), "never under v1");
        }
        {
            let _on = override_media_executor_config_on_this_thread(MediaExecutorConfig::v2());
            assert!(hot_state_enabled());
            {
                let _off = override_media_executor_config_on_this_thread(
                    MediaExecutorConfig::v2().with_hot_state(false),
                );
                assert!(!hot_state_enabled());
            }
            assert!(hot_state_enabled(), "the inner override is restored");
        }
    }

    #[test]
    fn cache_aligned_values_start_on_a_line_and_fill_whole_lines() {
        assert_eq!(std::mem::align_of::<CacheAligned<u8>>(), CACHE_LINE);
        assert_eq!(std::mem::size_of::<CacheAligned<u8>>(), CACHE_LINE);
        assert_eq!(
            std::mem::size_of::<CacheAligned<[u8; 65]>>(),
            2 * CACHE_LINE
        );
        assert_eq!(std::mem::align_of::<HotDoorbell>(), CACHE_LINE);
        assert_eq!(
            std::mem::size_of::<HotDoorbell>(),
            CACHE_LINE,
            "the bell is one line"
        );
        let values = [CacheAligned(1u8), CacheAligned(2u8), CacheAligned(3u8)];
        for value in &values {
            assert_eq!(value as *const CacheAligned<u8> as usize % CACHE_LINE, 0);
        }
        let mut value = CacheAligned(5u32);
        *value += 1;
        assert_eq!(*value, 6);
    }

    #[test]
    fn a_doorbell_gate_fires_once_per_ring_and_first_time() {
        let bell = HotDoorbell::new();
        let mut snapshot = NEVER_SEEN;
        assert!(
            gate(&mut snapshot, bell.read()),
            "a fresh snapshot always checks"
        );
        assert!(!gate(&mut snapshot, bell.read()));
        bell.ring();
        bell.ring();
        assert!(
            gate(&mut snapshot, bell.read()),
            "any number of rings is one check"
        );
        assert!(!gate(&mut snapshot, bell.read()));
        assert_eq!(bell.read(), 2);
    }

    #[test]
    fn prefetch_accepts_any_address() {
        let value = CacheAligned([7u8; 64]);
        prefetch_read(&value as *const CacheAligned<[u8; 64]> as usize);
        prefetch_read(0);
        prefetch_read(usize::MAX);
        prefetch_read(0xdead_beef);
        assert_eq!(value.0[63], 7);
    }

    #[test]
    fn slab_entries_are_cache_aligned_and_handles_reach_only_their_entry() {
        let mut slab = HotSlab::new();
        let a = slab.insert(10u64);
        let b = slab.insert(20u64);
        assert_ne!(a, b);
        assert_eq!(slab.get(a), Some(&10));
        assert_eq!(slab.get(b), Some(&20));
        *slab.get_mut(b).unwrap() += 1;
        assert_eq!(slab.get(b), Some(&21));
        for handle in [a, b] {
            let address = slab.address(handle).unwrap();
            assert_eq!(address % CACHE_LINE, 0, "every entry starts a line");
        }
        assert_ne!(
            slab.address(a).unwrap() / CACHE_LINE,
            slab.address(b).unwrap() / CACHE_LINE
        );
        assert_eq!(slab.len(), 2);
        assert_eq!(slab.remove(a), Some(10));
        assert_eq!(slab.len(), 1);
        assert_eq!(slab.get(a), None, "a removed entry is unreachable");
        assert_eq!(slab.get_mut(a), None);
        assert_eq!(slab.address(a), None);
        assert_eq!(slab.remove(a), None, "removal is once");
        assert_eq!(slab.get(b), Some(&21), "neighbours are untouched");
    }

    #[test]
    fn a_slot_is_reused_only_after_removal_and_old_handles_stay_dead() {
        let mut slab = HotSlab::new();
        let first = slab.insert("first");
        let second = slab.insert("second");
        // No reuse while in use: every live entry keeps a distinct slot.
        let live: Vec<HotHandle> = (0..64)
            .map(|n| slab.insert(if n % 2 == 0 { "even" } else { "odd" }))
            .collect();
        let mut indices: HashSet<u32> = live.iter().map(|handle| handle.index()).collect();
        assert!(indices.insert(first.index()) && indices.insert(second.index()));
        assert_eq!(slab.slots(), 66);

        assert_eq!(slab.remove(first), Some("first"));
        let reused = slab.insert("third");
        assert_eq!(reused.index(), first.index(), "the freed slot is reused");
        assert_eq!(
            reused.generation(),
            first.generation() + 1,
            "with the next generation"
        );
        assert_eq!(
            slab.get(first),
            None,
            "the stale handle misses the new occupant"
        );
        assert_eq!(slab.remove(first), None, "and cannot remove it");
        assert_eq!(slab.get(reused), Some(&"third"));
        assert_eq!(slab.slots(), 66, "no new slot while one was free");

        // Churn: generations only grow; a stale handle never matches.
        let mut handle = reused;
        let mut stale = vec![first];
        for round in 0..100u32 {
            assert_eq!(
                slab.remove(handle),
                Some(if round == 0 { "third" } else { "churn" })
            );
            stale.push(handle);
            handle = slab.insert("churn");
            assert_eq!(handle.index(), first.index());
            for old in &stale {
                assert_eq!(slab.get(*old), None);
            }
        }
        assert_eq!(handle.generation(), first.generation() + 101);
        assert_eq!(slab.get(second), Some(&"second"));
    }

    #[test]
    fn a_slot_whose_generation_would_wrap_is_retired_not_reused() {
        let mut slab = HotSlab::new();
        let handle = slab.insert(1u8);
        slab.force_generation(handle.index(), u32::MAX - 1);
        let late = HotHandle {
            index: handle.index(),
            generation: u32::MAX - 1,
        };
        assert_eq!(slab.remove(late), Some(1));
        assert_eq!(slab.retired(), 1);
        let next = slab.insert(2u8);
        assert_ne!(
            next.index(),
            handle.index(),
            "the retired slot is never handed out again"
        );
        assert_eq!(slab.get(late), None);
        assert_eq!(slab.get(next), Some(&2));
    }

    #[test]
    fn slabs_have_distinct_identities() {
        let a: HotSlab<u8> = HotSlab::new();
        let b: HotSlab<u8> = HotSlab::new();
        assert_ne!(a.id(), b.id());
    }
}
