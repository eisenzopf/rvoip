//! Executor v2: one runtime timer per lane poller.
//!
//! Without it every hosted receive loop owns a Tokio `Sleep` (its owner
//! timer), so each 20 ms playout deadline of every call was filed in the
//! runtime's global timer wheel, and every expiry was a timer-driver
//! operation plus an `AtomicWaker` registration. Here a lane poller owns a
//! [`LaneClock`]: the futures it hosts file their deadlines in the clock
//! (a binary heap keyed by deadline, with lazy deletion), and the poller
//! keeps exactly one Tokio `Sleep` armed at the clock's earliest deadline.
//! When it fires the poller wakes every entry that is due and polls those
//! futures in the same pass, so per-call timers never reach the global
//! wheel.
//!
//! Exactness is kept: an entry is woken only when its deadline has passed
//! (never early; [`LaneSleep`] re-checks the instant when polled), at the
//! same 1 ms driver granularity as the `Sleep` it replaces. Resetting a
//! deadline files a new heap item and makes the old one stale; stale items
//! are discarded when they reach the top, and the heap is rebuilt when stale
//! items dominate.
//!
//! A [`LaneSleep`] is created only inside a lane poller's poll (the clock is
//! found through a thread-local set for the duration of that poll); anywhere
//! else `media_clock::sleep_until` keeps returning a Tokio `Sleep`. If the
//! poller has gone away the sleep falls back to its own Tokio `Sleep`.
//!
//! Lane tick grid timers ([`crate::lane_tick`]): a clock built with a tick
//! grid files each deadline under its tick boundary (the first boundary at or
//! after it). A deadline that moves inside its tick files nothing new (it is
//! counted as coalesced), and every entry due in one tick wakes together. A
//! [`LaneSleep`] still completes only once its exact instant has passed.
//!
//! Timing wheel ([`MediaExecutorConfig::lane_timer_wheel`]): the clock keeps
//! its deadlines in a single-level timing wheel instead of the heap.
//! [`WHEEL_SLOTS`] buckets of one tick quantum each (with grid timers) or
//! [`WHEEL_RESOLUTION`] each (without them) cover the next 256 ticks; an
//! entry is linked into the bucket of its filed key through intrusive
//! doubly-linked lists threaded through the entry slab, so filing,
//! cancelling and re-arming are O(1) and never allocate. Deadlines beyond
//! the wheel's span wait in an overflow heap (lazy deletion, as before) and
//! cascade into their bucket once the cursor brings them within the span;
//! media deadlines (20 ms) never go there, call-length and RTCP deadlines do,
//! once each. Taking the due entries drains every bucket before the current
//! tick whole and checks the current bucket entry by entry (a bucket keeps a
//! lower bound of its keys, so a pass with nothing due touches no entry).
//! Semantics are the heap's exactly: an entry is taken iff its filed key is
//! at or before `now`, entries taken together are woken in (key, slot)
//! order (the heap's pop order), and the driver's rounds, self-wakes and
//! runtime-timer rules are unchanged (it only asks the clock for its
//! earliest key and the due entries).
//!
//! Pass clock ([`MediaExecutorConfig::lane_pass_clock`]): a lane poller pass
//! reads the clock once at its start and the futures it polls read that
//! value (through [`crate::media_clock::now`] and
//! [`crate::media_clock::now_std`]) instead of reading the clock themselves.
//! A cached value is always an earlier reading of the monotonic clock, never
//! a later one, so a "has this deadline passed?" check made with it can only
//! answer "not yet" too long, never "yes" too early. Its staleness is bounded
//! in work: one value serves at most [`PASS_CLOCK_MAX_READS`] reads and at
//! most [`PASS_CLOCK_REFRESH_POLLS`] hosted-future polls, and every
//! lane-clock round after the first starts from a fresh reading. The poller
//! still reads the clock afresh where a stale value would cost latency rather
//! than a little precision: before deciding to sleep (the tick's due check,
//! whose reading also ends the pass for the tick scheduler), for each timer
//! round after the first, and outside a pass (every caller then reads the
//! clock as before). The age a value reached is measured whenever it is
//! replaced ([`crate::executor::LaneWheelCounters`]).
//!
//! [`MediaExecutorConfig::lane_timer_wheel`]: crate::executor::MediaExecutorConfig::lane_timer_wheel
//! [`MediaExecutorConfig::lane_pass_clock`]: crate::executor::MediaExecutorConfig::lane_pass_clock

use std::cell::{Cell, RefCell};
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use tokio::time::{Instant, Sleep};

thread_local! {
    /// The clock of the lane poller currently polling on this thread.
    static CURRENT: RefCell<Option<Arc<LaneClock>>> = const { RefCell::new(None) };
}

/// Make `clock` current for the duration of one poller poll. The guard
/// restores the previous value (normally `None`).
pub(crate) fn enter(clock: &Arc<LaneClock>) -> CurrentClockGuard {
    let previous = CURRENT.with(|current| current.borrow_mut().replace(Arc::clone(clock)));
    CurrentClockGuard { previous }
}

pub(crate) struct CurrentClockGuard {
    previous: Option<Arc<LaneClock>>,
}

impl Drop for CurrentClockGuard {
    fn drop(&mut self) {
        let previous = self.previous.take();
        CURRENT.with(|current| *current.borrow_mut() = previous);
    }
}

/// The lane clock of the poller polling on this thread, if any.
pub(crate) fn current() -> Option<Arc<LaneClock>> {
    CURRENT.with(|current| current.borrow().clone())
}

// ---- Pass clock ----

/// Reads one pass-clock value serves before the next read refreshes it.
pub(crate) const PASS_CLOCK_MAX_READS: u32 = 32;
/// Hosted-future polls after which the pass-clock value is dropped
/// (the next read refreshes it). Applied by the lane poller's budget adapter.
pub(crate) const PASS_CLOCK_REFRESH_POLLS: u32 = 16;

thread_local! {
    /// A pass-clock pass is running on this thread.
    static PASS_ACTIVE: Cell<bool> = const { Cell::new(false) };
    /// The pass reading (kept after it is spent, to measure its age).
    static PASS_NOW: Cell<Option<Instant>> = const { Cell::new(None) };
    /// Reads the current value may still serve (0: the next read refreshes).
    static PASS_READS_LEFT: Cell<u32> = const { Cell::new(0) };
    /// Reads served from the cached value / fresh clock reads this pass.
    static PASS_CACHED: Cell<u64> = const { Cell::new(0) };
    static PASS_FRESH: Cell<u64> = const { Cell::new(0) };
    /// Ages of replaced values: (samples, sum ns, max ns).
    static PASS_STALENESS: Cell<(u64, u64, u64)> = const { Cell::new((0, 0, 0)) };
}

/// Pass-clock statistics of one thread since the last
/// [`take_pass_clock_stats`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PassClockStats {
    /// Reads answered from the pass reading (a `clock_gettime` saved each).
    pub(crate) cached: u64,
    /// Clock reads the pass clock made (the pass start and every refresh).
    pub(crate) fresh: u64,
    /// Replaced readings, and their age when replaced (sum and maximum).
    pub(crate) stale_samples: u64,
    pub(crate) stale_sum_ns: u64,
    pub(crate) stale_max_ns: u64,
}

/// Start a pass-clock pass seeded with `seed`, a clock reading the poller
/// just took. Ends when the guard drops.
pub(crate) fn enter_pass(seed: Instant) -> PassClockGuard {
    let previous = (PASS_ACTIVE.get(), PASS_NOW.get(), PASS_READS_LEFT.get());
    PASS_ACTIVE.set(true);
    PASS_NOW.set(Some(seed));
    PASS_READS_LEFT.set(PASS_CLOCK_MAX_READS);
    PASS_FRESH.set(PASS_FRESH.get() + 1);
    PassClockGuard { previous }
}

pub(crate) struct PassClockGuard {
    previous: (bool, Option<Instant>, u32),
}

impl Drop for PassClockGuard {
    fn drop(&mut self) {
        let (active, now, left) = self.previous;
        PASS_ACTIVE.set(active);
        PASS_NOW.set(now);
        PASS_READS_LEFT.set(left);
    }
}

/// The pass reading, inside a pass-clock pass (refreshed first when it is
/// spent); `None` outside one, where the caller reads the clock itself.
#[inline]
pub(crate) fn pass_cached_now() -> Option<Instant> {
    if !PASS_ACTIVE.get() {
        return None;
    }
    let left = PASS_READS_LEFT.get();
    if left > 0 {
        if let Some(now) = PASS_NOW.get() {
            PASS_READS_LEFT.set(left - 1);
            PASS_CACHED.set(PASS_CACHED.get() + 1);
            return Some(now);
        }
    }
    pass_refresh()
}

/// Read the clock afresh and, inside a pass-clock pass, make that the pass
/// reading. `None` outside a pass-clock pass.
pub(crate) fn pass_refresh() -> Option<Instant> {
    if !PASS_ACTIVE.get() {
        return None;
    }
    let now = Instant::now();
    if let Some(previous) = PASS_NOW.get() {
        let age =
            u64::try_from(now.saturating_duration_since(previous).as_nanos()).unwrap_or(u64::MAX);
        let (samples, sum, max) = PASS_STALENESS.get();
        PASS_STALENESS.set((samples + 1, sum.saturating_add(age), max.max(age)));
    }
    PASS_NOW.set(Some(now));
    PASS_READS_LEFT.set(PASS_CLOCK_MAX_READS);
    PASS_FRESH.set(PASS_FRESH.get() + 1);
    Some(now)
}

/// Spend the pass reading: the next read refreshes it.
#[inline]
pub(crate) fn pass_invalidate() {
    PASS_READS_LEFT.set(0);
}

/// This thread's pass-clock statistics since the previous call.
pub(crate) fn take_pass_clock_stats() -> PassClockStats {
    let (stale_samples, stale_sum_ns, stale_max_ns) = PASS_STALENESS.replace((0, 0, 0));
    PassClockStats {
        cached: PASS_CACHED.replace(0),
        fresh: PASS_FRESH.replace(0),
        stale_samples,
        stale_sum_ns,
        stale_max_ns,
    }
}

// ---- Timing wheel ----

/// Buckets in a lane clock's timing wheel (a power of two).
pub(crate) const WHEEL_SLOTS: usize = 256;
const WHEEL_SPAN: u64 = WHEEL_SLOTS as u64;
const WHEEL_MASK: u64 = WHEEL_SPAN - 1;
const WHEEL_WORDS: usize = WHEEL_SLOTS / 64;
/// Bucket width without grid timers (the runtime timer's resolution); with
/// grid timers a bucket is one tick quantum.
pub(crate) const WHEEL_RESOLUTION: Duration = Duration::from_millis(1);

/// End of a bucket list.
const NIL: u32 = u32::MAX;
/// `Entry::bucket`: not in the wheel.
const UNFILED: u32 = u32::MAX;
/// `Entry::bucket`: waiting in the overflow heap.
const IN_OVERFLOW: u32 = u32::MAX - 1;

/// Timing-wheel operations since the last read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct WheelOps {
    /// Deadlines linked into a bucket when filed.
    pub(crate) inserts: u64,
    /// Deadlines filed beyond the wheel's span (overflow heap).
    pub(crate) far_inserts: u64,
    /// Filed deadlines removed before the wheel served them (re-armed to
    /// another key, seen due by their sleep first, or their sleep dropped).
    pub(crate) cancels: u64,
    /// Far deadlines moved from the overflow heap into a bucket.
    pub(crate) cascades: u64,
    /// Entry-by-entry checks of the current bucket (its lower bound was due).
    pub(crate) slot_scans: u64,
}

/// A single-level timing wheel over the clock's entry slab.
///
/// Invariants: every linked entry's tick is below `cursor + WHEEL_SPAN`;
/// its bucket is that of `max(tick, cursor)` (deadlines filed at or after
/// the cursor's tick has passed share the cursor's bucket). Every live
/// overflow entry's tick is at least `cursor + WHEEL_SPAN`. A bucket's
/// `floor` is at most every key in it (`None` iff the bucket is empty) and
/// `floor_count` counts the keys equal to it, or is 0 when the floor is
/// only a lower bound.
struct Wheel {
    origin: Instant,
    resolution_ns: u64,
    cursor: u64,
    heads: Box<[u32]>,
    floor: Box<[Option<Instant>]>,
    floor_count: Box<[u32]>,
    occupied: [u64; WHEEL_WORDS],
    len: usize,
    overflow: BinaryHeap<Reverse<(Instant, u32, u32)>>,
    overflow_live: usize,
    /// Scratch for one `take_due` (kept to reuse its allocation).
    due: Vec<(Instant, u32)>,
    ops: WheelOps,
}

impl Wheel {
    fn new(origin: Instant, resolution: Duration) -> Self {
        Self {
            origin,
            resolution_ns: u64::try_from(resolution.as_nanos())
                .unwrap_or(u64::MAX)
                .max(1),
            cursor: 0,
            heads: vec![NIL; WHEEL_SLOTS].into_boxed_slice(),
            floor: vec![None; WHEEL_SLOTS].into_boxed_slice(),
            floor_count: vec![0; WHEEL_SLOTS].into_boxed_slice(),
            occupied: [0; WHEEL_WORDS],
            len: 0,
            overflow: BinaryHeap::new(),
            overflow_live: 0,
            due: Vec::new(),
            ops: WheelOps::default(),
        }
    }

    /// The tick holding `t` (instants before the origin are tick 0).
    #[inline]
    fn tick_of(&self, t: Instant) -> u64 {
        t.checked_duration_since(self.origin).map_or(0, |since| {
            u64::try_from(since.as_nanos()).unwrap_or(u64::MAX) / self.resolution_ns
        })
    }

    #[inline]
    fn bucket_of(&self, tick: u64) -> usize {
        (tick.max(self.cursor) & WHEEL_MASK) as usize
    }

    #[inline]
    fn set_occupied(&mut self, bucket: usize) {
        self.occupied[bucket / 64] |= 1u64 << (bucket % 64);
    }

    #[inline]
    fn clear_bucket(&mut self, bucket: usize) {
        self.heads[bucket] = NIL;
        self.floor[bucket] = None;
        self.floor_count[bucket] = 0;
        self.occupied[bucket / 64] &= !(1u64 << (bucket % 64));
    }

    /// The first occupied bucket among the `count` ticks from `start`, as
    /// an offset from `start` (bitmap words, not buckets, are visited).
    fn next_occupied(&self, start: u64, count: u64) -> Option<u64> {
        let mut offset = 0;
        while offset < count {
            let bucket = ((start + offset) & WHEEL_MASK) as usize;
            let bit = bucket % 64;
            let word = self.occupied[bucket / 64] >> bit;
            if word != 0 {
                let found = offset + u64::from(word.trailing_zeros());
                return (found < count).then_some(found);
            }
            offset += (64 - bit) as u64;
        }
        None
    }

    /// Link `slot` (filed under `key`) at the head of its bucket and keep
    /// the bucket's floor.
    fn link(&mut self, entries: &mut [Entry], slot: u32, key: Instant) {
        let bucket = self.bucket_of(self.tick_of(key));
        let head = self.heads[bucket];
        {
            let entry = &mut entries[slot as usize];
            entry.bucket = bucket as u32;
            entry.prev = NIL;
            entry.next = head;
        }
        if head != NIL {
            entries[head as usize].prev = slot;
        }
        self.heads[bucket] = slot;
        match self.floor[bucket] {
            Some(floor) if floor < key => {}
            // Equal to an exact floor: one more at it. Equal to a lower
            // bound: the floor is right but its count unknown; stays 0.
            Some(floor) if floor == key => {
                if self.floor_count[bucket] > 0 {
                    self.floor_count[bucket] += 1;
                }
            }
            _ => {
                self.floor[bucket] = Some(key);
                self.floor_count[bucket] = 1;
            }
        }
        self.set_occupied(bucket);
        self.len += 1;
    }

    /// Unlink `slot` from its bucket (`entry.bucket` names it). The floor
    /// is left to the caller.
    fn unlink(&mut self, entries: &mut [Entry], slot: u32) -> usize {
        let (bucket, prev, next) = {
            let entry = &mut entries[slot as usize];
            let links = (entry.bucket as usize, entry.prev, entry.next);
            entry.bucket = UNFILED;
            entry.prev = NIL;
            entry.next = NIL;
            links
        };
        if prev != NIL {
            entries[prev as usize].next = next;
        } else {
            self.heads[bucket] = next;
        }
        if next != NIL {
            entries[next as usize].prev = prev;
        }
        self.len -= 1;
        bucket
    }

    /// File `slot` under `key`: in its bucket, or in the overflow heap when
    /// beyond the span.
    fn file(&mut self, entries: &mut [Entry], slot: u32, generation: u32, key: Instant) {
        if self.tick_of(key) >= self.cursor.saturating_add(WHEEL_SPAN) {
            self.overflow.push(Reverse((key, slot, generation)));
            self.overflow_live += 1;
            entries[slot as usize].bucket = IN_OVERFLOW;
            self.ops.far_inserts += 1;
        } else {
            self.link(entries, slot, key);
            self.ops.inserts += 1;
        }
    }

    /// Remove `slot`'s filing (keyed `key`) wherever it is. O(1): an
    /// overflow item is left behind stale.
    fn cancel(&mut self, entries: &mut [Entry], slot: u32, key: Option<Instant>) {
        match entries[slot as usize].bucket {
            UNFILED => return,
            IN_OVERFLOW => {
                entries[slot as usize].bucket = UNFILED;
                self.overflow_live -= 1;
            }
            _ => {
                let bucket = self.unlink(entries, slot);
                if self.heads[bucket] == NIL {
                    self.clear_bucket(bucket);
                } else if key.is_some() && key == self.floor[bucket] && self.floor_count[bucket] > 0
                {
                    // Reaching 0 leaves the floor as a lower bound.
                    self.floor_count[bucket] -= 1;
                }
            }
        }
        self.ops.cancels += 1;
    }

    fn overflow_item_is_current(
        entries: &[Entry],
        key: Instant,
        slot: u32,
        generation: u32,
    ) -> bool {
        entries.get(slot as usize).is_some_and(|entry| {
            entry.in_use
                && entry.generation == generation
                && entry.deadline == Some(key)
                && entry.bucket == IN_OVERFLOW
        })
    }

    /// Rescan `bucket` for its exact floor and how many keys sit on it.
    fn refresh_floor(&mut self, entries: &[Entry], bucket: usize) {
        let mut floor: Option<Instant> = None;
        let mut count = 0;
        let mut slot = self.heads[bucket];
        while slot != NIL {
            let entry = &entries[slot as usize];
            if let Some(key) = entry.deadline {
                match floor {
                    Some(current) if current < key => {}
                    Some(current) if current == key => count += 1,
                    _ => {
                        floor = Some(key);
                        count = 1;
                    }
                }
            }
            slot = entry.next;
        }
        self.floor[bucket] = floor;
        self.floor_count[bucket] = count;
    }

    /// The earliest filed key (stale overflow items are discarded).
    fn next_deadline(&mut self, entries: &[Entry]) -> Option<Instant> {
        if self.len > 0 {
            if let Some(offset) = self.next_occupied(self.cursor, WHEEL_SPAN) {
                let bucket = ((self.cursor + offset) & WHEEL_MASK) as usize;
                if self.floor_count[bucket] == 0 {
                    self.refresh_floor(entries, bucket);
                }
                return self.floor[bucket];
            }
        }
        while let Some(Reverse((key, slot, generation))) = self.overflow.peek().copied() {
            if Self::overflow_item_is_current(entries, key, slot, generation) {
                return Some(key);
            }
            self.overflow.pop();
        }
        None
    }

    /// Unfile every entry whose key is at or before `now` and append
    /// `(key, slot)` for each to `self.due`, unsorted.
    fn collect_due(&mut self, entries: &mut [Entry], now: Instant) {
        let now_tick = self.tick_of(now);
        if now_tick > self.cursor {
            // Every bucket before the current tick is due whole: its keys
            // are before the tick's start, which is at or before `now`.
            let start = self.cursor;
            let span = (now_tick - start).min(WHEEL_SPAN);
            let mut offset = 0;
            while let Some(found) = self.next_occupied(start + offset, span - offset) {
                let bucket = ((start + offset + found) & WHEEL_MASK) as usize;
                let mut slot = self.heads[bucket];
                while slot != NIL {
                    let entry = &mut entries[slot as usize];
                    let next = entry.next;
                    if let Some(key) = entry.deadline {
                        self.due.push((key, slot));
                    }
                    entry.bucket = UNFILED;
                    entry.prev = NIL;
                    entry.next = NIL;
                    self.len -= 1;
                    slot = next;
                }
                self.clear_bucket(bucket);
                offset += found + 1;
            }
            self.cursor = now_tick;
            // Far deadlines that came within the span cascade into their
            // bucket (those already due into the current one).
            let horizon = now_tick.saturating_add(WHEEL_SPAN);
            while let Some(Reverse((key, slot, generation))) = self.overflow.peek().copied() {
                if self.tick_of(key) >= horizon {
                    break;
                }
                self.overflow.pop();
                if !Self::overflow_item_is_current(entries, key, slot, generation) {
                    continue;
                }
                self.overflow_live -= 1;
                self.link(entries, slot, key);
                self.ops.cascades += 1;
            }
        }
        // The current bucket holds the current tick and deadlines filed (or
        // cascaded) after their tick passed: take the keys up to `now`.
        let bucket = (self.cursor & WHEEL_MASK) as usize;
        if self.floor[bucket].is_some_and(|floor| floor <= now) {
            self.ops.slot_scans += 1;
            let mut slot = self.heads[bucket];
            while slot != NIL {
                let next = entries[slot as usize].next;
                if let Some(key) = entries[slot as usize].deadline.filter(|key| *key <= now) {
                    self.unlink(entries, slot);
                    self.due.push((key, slot));
                }
                slot = next;
            }
            if self.heads[bucket] == NIL {
                self.clear_bucket(bucket);
            } else {
                self.refresh_floor(entries, bucket);
            }
        }
    }

    /// Empty the wheel (the poller ended).
    fn clear(&mut self, entries: &mut [Entry]) {
        for entry in entries.iter_mut() {
            entry.bucket = UNFILED;
            entry.prev = NIL;
            entry.next = NIL;
        }
        for bucket in 0..WHEEL_SLOTS {
            self.clear_bucket(bucket);
        }
        self.len = 0;
        self.overflow.clear();
        self.overflow_live = 0;
    }

    /// Rebuild the overflow heap once stale items dominate (a far deadline
    /// that keeps moving cannot grow it without bound).
    fn compact_if_needed(&mut self, entries: &[Entry]) {
        if self.overflow.len() <= 64 + 4 * self.overflow_live {
            return;
        }
        let mut overflow = BinaryHeap::with_capacity(self.overflow_live);
        for (slot, entry) in entries.iter().enumerate() {
            if let (true, IN_OVERFLOW, Some(key)) = (entry.in_use, entry.bucket, entry.deadline) {
                overflow.push(Reverse((key, slot as u32, entry.generation)));
            }
        }
        self.overflow = overflow;
    }
}

#[derive(Debug)]
struct Entry {
    /// Bumped whenever the slot is released, so a heap item of a previous
    /// owner of the slot is recognised as stale.
    generation: u32,
    /// The filed key the entry waits for: its deadline, or (tick grid) the
    /// tick boundary at or after it (`None`: idle or already woken).
    deadline: Option<Instant>,
    /// The exact deadline behind `deadline`, to tell a move inside the tick
    /// (coalesced) from a repeat of the same instant.
    exact: Option<Instant>,
    waker: Option<Waker>,
    in_use: bool,
    /// Timing wheel: the bucket holding the entry ([`UNFILED`], [`IN_OVERFLOW`]
    /// or a bucket index) and its neighbours in that bucket's list.
    bucket: u32,
    prev: u32,
    next: u32,
}

#[derive(Default)]
struct ClockState {
    entries: Vec<Entry>,
    free: Vec<u32>,
    /// (deadline, slot, generation); stale when the slot's current deadline
    /// or generation differs. Unused with the timing wheel.
    heap: BinaryHeap<Reverse<(Instant, u32, u32)>>,
    /// The timing wheel (`None`: the heap above).
    wheel: Option<Box<Wheel>>,
    live: usize,
    /// Deadlines filed since the poller last read the counters.
    arms: u64,
    /// Deadline moves that stayed inside their tick and filed nothing.
    coalesced: u64,
}

impl ClockState {
    fn is_current(&self, deadline: Instant, slot: u32, generation: u32) -> bool {
        self.entries.get(slot as usize).is_some_and(|entry| {
            entry.in_use && entry.generation == generation && entry.deadline == Some(deadline)
        })
    }

    /// Rebuild the heap from live deadlines once stale items dominate, so a
    /// sleep that keeps moving a far deadline cannot grow it without bound.
    fn compact_if_needed(&mut self) {
        if let Some(wheel) = self.wheel.as_mut() {
            wheel.compact_if_needed(&self.entries);
            return;
        }
        if self.heap.len() <= 64 + 4 * self.live {
            return;
        }
        let mut heap = BinaryHeap::with_capacity(self.live);
        for (slot, entry) in self.entries.iter().enumerate() {
            if let (true, Some(deadline)) = (entry.in_use, entry.deadline) {
                heap.push(Reverse((deadline, slot as u32, entry.generation)));
            }
        }
        self.heap = heap;
    }

    /// Remove `slot`'s wheel filing, if any (before its key changes).
    fn wheel_cancel(&mut self, slot: u32) {
        let Self { entries, wheel, .. } = self;
        if let Some(wheel) = wheel.as_mut() {
            let key = entries[slot as usize].deadline;
            wheel.cancel(entries, slot, key);
        }
    }
}

/// The deadlines of every future hosted on one lane poller.
pub(crate) struct LaneClock {
    state: parking_lot::Mutex<ClockState>,
    closed: AtomicBool,
    /// File deadlines on this tick grid (`None`: at their instant).
    grid: Option<crate::lane_tick::TickGrid>,
}

impl LaneClock {
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        Self::with_grid(None)
    }

    #[cfg(test)]
    pub(crate) fn with_grid(grid: Option<crate::lane_tick::TickGrid>) -> Self {
        Self::with_timers(grid, false)
    }

    /// A clock filing on `grid` (lane tick grid timers), or at the exact
    /// instant, whose deadlines live in a timing wheel or the heap.
    pub(crate) fn with_timers(grid: Option<crate::lane_tick::TickGrid>, wheel: bool) -> Self {
        // With the grid the wheel's buckets are its ticks, so every key in a
        // bucket is its boundary; without it they are 1 ms from now.
        let origin = grid.map_or_else(Instant::now, |grid| Instant::from_std(grid.origin()));
        Self::with_timers_at(grid, wheel, origin)
    }

    /// [`Self::with_timers`] with an explicit wheel origin (used without
    /// the grid; tests drive a fake clock this way).
    pub(crate) fn with_timers_at(
        grid: Option<crate::lane_tick::TickGrid>,
        wheel: bool,
        origin: Instant,
    ) -> Self {
        let wheel = wheel.then(|| {
            let (origin, resolution) = match grid {
                Some(grid) => (Instant::from_std(grid.origin()), grid.quantum()),
                None => (origin, WHEEL_RESOLUTION),
            };
            Box::new(Wheel::new(origin, resolution))
        });
        Self {
            state: parking_lot::Mutex::new(ClockState {
                wheel,
                ..ClockState::default()
            }),
            closed: AtomicBool::new(false),
            grid,
        }
    }

    /// The key a deadline is filed under.
    fn filed_at(&self, deadline: Instant) -> Instant {
        match self.grid {
            Some(grid) => Instant::from_std(grid.ceil(deadline.into_std())),
            None => deadline,
        }
    }

    /// The poller ended: sleeps still alive fall back to their own timer.
    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::Release);
        let wakers: Vec<Waker> = {
            let mut state = self.state.lock();
            let ClockState {
                entries,
                wheel,
                heap,
                ..
            } = &mut *state;
            heap.clear();
            if let Some(wheel) = wheel.as_mut() {
                wheel.clear(entries);
            }
            entries
                .iter_mut()
                .filter_map(|entry| {
                    entry.deadline = None;
                    entry.waker.take()
                })
                .collect()
        };
        for waker in wakers {
            waker.wake();
        }
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    fn allocate(&self) -> (u32, u32) {
        let mut state = self.state.lock();
        state.live += 1;
        if let Some(slot) = state.free.pop() {
            let entry = &mut state.entries[slot as usize];
            entry.in_use = true;
            return (slot, entry.generation);
        }
        let slot = state.entries.len() as u32;
        state.entries.push(Entry {
            generation: 0,
            deadline: None,
            exact: None,
            waker: None,
            in_use: true,
            bucket: UNFILED,
            prev: NIL,
            next: NIL,
        });
        (slot, 0)
    }

    fn release(&self, slot: u32, generation: u32) {
        let mut state = self.state.lock();
        let valid = state
            .entries
            .get(slot as usize)
            .is_some_and(|entry| entry.in_use && entry.generation == generation);
        if !valid {
            return;
        }
        state.wheel_cancel(slot);
        let entry = &mut state.entries[slot as usize];
        entry.in_use = false;
        entry.generation = entry.generation.wrapping_add(1);
        entry.deadline = None;
        entry.exact = None;
        entry.waker = None;
        state.live -= 1;
        state.free.push(slot);
        state.compact_if_needed();
    }

    /// Wait for `deadline` with `waker`. Files a heap item (timing wheel:
    /// links the entry into its bucket) only when the filed key changed (tick
    /// grid: when the deadline left its tick); refreshes the waker only when
    /// it would wake a different task.
    fn arm(&self, slot: u32, generation: u32, deadline: Instant, waker: &Waker) {
        let key = self.filed_at(deadline);
        let mut guard = self.state.lock();
        let state = &mut *guard;
        let Some(entry) = state.entries.get_mut(slot as usize) else {
            return;
        };
        if !entry.in_use || entry.generation != generation {
            return;
        }
        if !entry
            .waker
            .as_ref()
            .is_some_and(|current| current.will_wake(waker))
        {
            entry.waker = Some(waker.clone());
        }
        if entry.deadline != Some(key) {
            let previous = entry.deadline.replace(key);
            entry.exact = Some(deadline);
            match state.wheel.as_mut() {
                // Timing wheel: unlink the old filing (O(1)), link the new one.
                Some(wheel) => {
                    wheel.cancel(&mut state.entries, slot, previous);
                    wheel.file(&mut state.entries, slot, generation, key);
                }
                None => state.heap.push(Reverse((key, slot, generation))),
            }
            state.arms += 1;
            state.compact_if_needed();
        } else if entry.exact != Some(deadline) {
            // The deadline moved inside its tick; the filed item still wakes
            // it no earlier than the boundary, which is not before it.
            entry.exact = Some(deadline);
            state.coalesced += 1;
        }
    }

    /// The entry no longer waits (its deadline passed and it was observed).
    fn disarm(&self, slot: u32, generation: u32) {
        let mut state = self.state.lock();
        let valid = state
            .entries
            .get(slot as usize)
            .is_some_and(|entry| entry.in_use && entry.generation == generation);
        if valid {
            state.wheel_cancel(slot);
            state.entries[slot as usize].deadline = None;
        }
    }

    /// The earliest live deadline, discarding stale items on the way.
    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        let mut state = self.state.lock();
        let ClockState { entries, wheel, .. } = &mut *state;
        if let Some(wheel) = wheel.as_mut() {
            return wheel.next_deadline(entries);
        }
        while let Some(Reverse((deadline, slot, generation))) = state.heap.peek().copied() {
            if state.is_current(deadline, slot, generation) {
                return Some(deadline);
            }
            state.heap.pop();
        }
        None
    }

    /// Collect the wakers of every entry due at `now` into `wakers` (woken
    /// by the caller after the lock is released), in (key, slot) order.
    /// Returns how many entries were due and how many deadlines were filed
    /// since the last call.
    pub(crate) fn take_due(&self, now: Instant, wakers: &mut Vec<Waker>) -> (u64, u64) {
        let mut state = self.state.lock();
        let mut due = 0;
        if state.wheel.is_some() {
            let ClockState { entries, wheel, .. } = &mut *state;
            let wheel = wheel.as_mut().expect("checked above");
            wheel.due.clear();
            wheel.collect_due(entries, now);
            // The heap's pop order: (key, slot); a live slot is filed once.
            wheel.due.sort_unstable();
            for &(_, slot) in &wheel.due {
                let entry = &mut entries[slot as usize];
                entry.deadline = None;
                if let Some(waker) = entry.waker.take() {
                    wakers.push(waker);
                }
                due += 1;
            }
            return (due, std::mem::take(&mut state.arms));
        }
        while let Some(Reverse((deadline, slot, generation))) = state.heap.peek().copied() {
            if deadline > now {
                break;
            }
            state.heap.pop();
            if !state.is_current(deadline, slot, generation) {
                continue;
            }
            let entry = &mut state.entries[slot as usize];
            entry.deadline = None;
            if let Some(waker) = entry.waker.take() {
                wakers.push(waker);
            }
            due += 1;
        }
        (due, std::mem::take(&mut state.arms))
    }

    /// Deadline moves coalesced inside their tick since the last call.
    pub(crate) fn take_coalesced(&self) -> u64 {
        if self.grid.is_none() {
            return 0;
        }
        std::mem::take(&mut self.state.lock().coalesced)
    }

    /// Timing-wheel operations since the last call (zero with the heap).
    pub(crate) fn take_wheel_ops(&self) -> WheelOps {
        self.state
            .lock()
            .wheel
            .as_mut()
            .map(|wheel| std::mem::take(&mut wheel.ops))
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn heap_len(&self) -> usize {
        self.state.lock().heap.len()
    }

    /// Timing-wheel test view: (entries linked in buckets, overflow heap items).
    #[cfg(test)]
    pub(crate) fn wheel_len(&self) -> (usize, usize) {
        let state = self.state.lock();
        let wheel = state.wheel.as_ref().expect("a wheel clock");
        (wheel.len, wheel.overflow.len())
    }

    #[cfg(test)]
    pub(crate) fn live(&self) -> usize {
        self.state.lock().live
    }
}

/// A sleep filed in a lane clock instead of the runtime's timer wheel.
pub struct LaneSleep {
    clock: Arc<LaneClock>,
    slot: u32,
    generation: u32,
    deadline: Instant,
    /// Used only when the clock's poller has gone away.
    fallback: Option<Pin<Box<Sleep>>>,
}

impl LaneSleep {
    pub(crate) fn new(clock: Arc<LaneClock>, deadline: Instant) -> Self {
        let (slot, generation) = clock.allocate();
        Self {
            clock,
            slot,
            generation,
            deadline,
            fallback: None,
        }
    }

    /// Move the deadline. Takes effect at the next poll, which files it.
    pub fn reset(&mut self, deadline: Instant) {
        self.deadline = deadline;
        if let Some(fallback) = self.fallback.as_mut() {
            fallback.as_mut().reset(deadline);
        }
    }
}

impl Future for LaneSleep {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        if let Some(fallback) = this.fallback.as_mut() {
            return fallback.as_mut().poll(cx);
        }
        if this.clock.is_closed() {
            let mut fallback = Box::pin(tokio::time::sleep_until(this.deadline));
            let polled = fallback.as_mut().poll(cx);
            this.fallback = Some(fallback);
            return polled;
        }
        // Never ready before the instant, however the wake arrived. The pass
        // reading is an earlier reading of the same clock, so it can
        // only defer completion (to the next round or pass), never advance it.
        let now = pass_cached_now().unwrap_or_else(Instant::now);
        if now >= this.deadline {
            this.clock.disarm(this.slot, this.generation);
            return Poll::Ready(());
        }
        this.clock
            .arm(this.slot, this.generation, this.deadline, cx.waker());
        Poll::Pending
    }
}

impl Drop for LaneSleep {
    fn drop(&mut self) {
        self.clock.release(self.slot, self.generation);
    }
}

/// The poller's side: one Tokio `Sleep` at the clock's earliest deadline,
/// re-registered only when that deadline moves earlier or after it fired
/// (the owner timer rule, see `crate::executor`).
pub(crate) struct LaneClockDriver {
    clock: Arc<LaneClock>,
    sleep: Option<Pin<Box<Sleep>>>,
    /// The instant the runtime entry was last set to and not yet observed
    /// to fire.
    armed: Option<Instant>,
    wakers: Vec<Waker>,
    /// The armed instant whose expiry was already counted, so that one
    /// expiry is counted once whoever observes it first.
    counted: Option<Instant>,
    /// Expiries of the runtime timer since the last read.
    expiries: u64,
    /// Registrations of the runtime timer since the last read (timer
    /// operations; counted with or without the lane tick).
    resets: u64,
}

impl LaneClockDriver {
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        Self::with_timers(None, false)
    }

    /// A driver whose clock files deadlines on `grid` (lane tick grid
    /// timers), or at their instant (`None`), in a timing wheel or the heap.
    pub(crate) fn with_timers(grid: Option<crate::lane_tick::TickGrid>, wheel: bool) -> Self {
        Self {
            clock: Arc::new(LaneClock::with_timers(grid, wheel)),
            sleep: None,
            armed: None,
            wakers: Vec::new(),
            counted: None,
            expiries: 0,
            resets: 0,
        }
    }

    /// Lane tick: whether a live deadline is due at `now` (a ticking poller
    /// arms no runtime timer, so it checks before it sleeps).
    pub(crate) fn has_due(&self, now: Instant) -> bool {
        self.clock
            .next_deadline()
            .is_some_and(|deadline| deadline <= now)
    }

    /// Runtime timer registrations since the last call.
    pub(crate) fn take_resets(&mut self) -> u64 {
        std::mem::take(&mut self.resets)
    }

    /// Count an expiry of the runtime timer, once. The expiry is what woke
    /// the poller, but `wake_due` runs first in a pass and serves the entries
    /// it was armed for, so `poll_timer` then finds the clock empty or a
    /// later deadline and never sees the `Sleep` ready: counting only a ready
    /// `poll_timer` would miss nearly every expiry.
    fn note_expiry(&mut self) {
        let Some(armed) = self.armed else {
            return;
        };
        if self.counted != Some(armed)
            && self.sleep.as_ref().is_some_and(|sleep| sleep.is_elapsed())
        {
            self.counted = Some(armed);
            self.expiries += 1;
        }
    }

    /// Runtime timer expiries since the last call.
    pub(crate) fn take_expiries(&mut self) -> u64 {
        std::mem::take(&mut self.expiries)
    }

    pub(crate) fn clock(&self) -> &Arc<LaneClock> {
        &self.clock
    }

    /// Wake every entry due now. Returns (entries woken, deadlines filed
    /// since the previous call).
    #[cfg(test)]
    pub(crate) fn wake_due(&mut self) -> (u64, u64) {
        self.wake_due_at(Instant::now())
    }

    /// Wake every entry due at `now` (a clock reading the caller took: a
    /// fresh one, or the pass reading). Returns (entries woken, deadlines
    /// filed since the previous call).
    pub(crate) fn wake_due_at(&mut self, now: Instant) -> (u64, u64) {
        self.note_expiry();
        let (due, arms) = self.clock.take_due(now, &mut self.wakers);
        for waker in self.wakers.drain(..) {
            waker.wake();
        }
        (due, arms)
    }

    /// Point the runtime timer at the clock's earliest deadline and poll it.
    /// `Ready` means it had expired: the caller wakes the due entries and
    /// polls again.
    pub(crate) fn poll_timer(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        let Some(next) = self.clock.next_deadline() else {
            self.note_expiry();
            return Poll::Pending;
        };
        if self.armed != Some(next) {
            self.note_expiry();
            // A later deadline extends the live entry in place (lock-free in
            // Tokio); an earlier one, or one after a fire, re-registers.
            match self.sleep.as_mut() {
                Some(sleep) => sleep.as_mut().reset(next),
                None => self.sleep = Some(Box::pin(tokio::time::sleep_until(next))),
            }
            self.armed = Some(next);
            self.counted = None;
            self.resets += 1;
        }
        let sleep = self.sleep.as_mut().expect("armed above");
        match sleep.as_mut().poll(cx) {
            Poll::Ready(()) => {
                self.note_expiry();
                self.armed = None;
                Poll::Ready(())
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for LaneClockDriver {
    fn drop(&mut self) {
        self.clock.close();
    }
}

#[cfg(test)]
mod tests {
    //! Real-time tests (rtp-core does not enable Tokio's `test-util`):
    //! deadlines are tens of milliseconds out and `take_due` is driven with
    //! explicit instants where only ordering matters.
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    fn counting_waker(count: Arc<AtomicUsize>) -> Waker {
        struct Count(Arc<AtomicUsize>);
        impl std::task::Wake for Count {
            fn wake(self: Arc<Self>) {
                self.0.fetch_add(1, Ordering::AcqRel);
            }
        }
        Waker::from(Arc::new(Count(count)))
    }

    #[tokio::test]
    async fn entries_wake_in_deadline_order_and_never_early() {
        let clock = Arc::new(LaneClock::new());
        let start = Instant::now();
        let woke = Arc::new(AtomicUsize::new(0));
        let waker = counting_waker(Arc::clone(&woke));
        let mut cx = Context::from_waker(&waker);
        let early_at = start + Duration::from_millis(30);
        let late_at = start + Duration::from_millis(400);
        let mut late = LaneSleep::new(Arc::clone(&clock), late_at);
        let mut early = LaneSleep::new(Arc::clone(&clock), early_at);
        assert!(Pin::new(&mut late).poll(&mut cx).is_pending());
        assert!(Pin::new(&mut early).poll(&mut cx).is_pending());
        assert_eq!(clock.next_deadline(), Some(early_at));

        let mut wakers = Vec::new();
        assert_eq!(
            clock.take_due(early_at - Duration::from_millis(1), &mut wakers),
            (0, 2),
            "nothing is due before the earliest deadline; two deadlines were filed"
        );
        assert_eq!(clock.take_due(early_at, &mut wakers), (1, 0));
        assert_eq!(wakers.len(), 1);
        for waker in wakers.drain(..) {
            waker.wake();
        }
        assert_eq!(
            woke.load(Ordering::Acquire),
            1,
            "only the due entry was woken"
        );
        tokio::time::sleep_until(early_at).await;
        assert!(Pin::new(&mut early).poll(&mut cx).is_ready());
        assert!(
            Pin::new(&mut late).poll(&mut cx).is_pending(),
            "never before its instant"
        );
        assert_eq!(clock.next_deadline(), Some(late_at));
    }

    #[tokio::test]
    async fn a_woken_entry_polled_before_its_instant_waits_again() {
        // A wake that arrives early (another source) must not complete it.
        let clock = Arc::new(LaneClock::new());
        let deadline = Instant::now() + Duration::from_secs(60);
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        let mut sleep = LaneSleep::new(Arc::clone(&clock), deadline);
        assert!(Pin::new(&mut sleep).poll(&mut cx).is_pending());
        assert!(Pin::new(&mut sleep).poll(&mut cx).is_pending());
        assert_eq!(clock.heap_len(), 1, "an unchanged deadline is filed once");
        assert_eq!(clock.next_deadline(), Some(deadline));
    }

    #[tokio::test]
    async fn a_reset_makes_the_old_item_stale_and_release_frees_the_slot() {
        let clock = Arc::new(LaneClock::new());
        let start = Instant::now();
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        let mut sleep = LaneSleep::new(Arc::clone(&clock), start + Duration::from_secs(10));
        assert!(Pin::new(&mut sleep).poll(&mut cx).is_pending());
        sleep.reset(start + Duration::from_secs(30));
        assert!(Pin::new(&mut sleep).poll(&mut cx).is_pending());
        assert_eq!(clock.heap_len(), 2, "the old item stays until it surfaces");
        assert_eq!(
            clock.next_deadline(),
            Some(start + Duration::from_secs(30)),
            "the 10 s item is stale"
        );
        assert_eq!(clock.heap_len(), 1);
        let mut wakers = Vec::new();
        assert_eq!(
            clock
                .take_due(start + Duration::from_secs(20), &mut wakers)
                .0,
            0
        );
        drop(sleep);
        assert_eq!(clock.live(), 0);
        assert_eq!(clock.next_deadline(), None, "a released slot never fires");
        // The slot is reused under a new generation.
        let mut reused = LaneSleep::new(Arc::clone(&clock), start + Duration::from_secs(50));
        assert!(Pin::new(&mut reused).poll(&mut cx).is_pending());
        assert_eq!(clock.next_deadline(), Some(start + Duration::from_secs(50)));
        assert_eq!(
            clock
                .take_due(start + Duration::from_secs(50), &mut wakers)
                .0,
            1
        );
    }

    #[tokio::test]
    async fn stale_items_are_compacted() {
        let clock = Arc::new(LaneClock::new());
        let start = Instant::now();
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        let mut sleep = LaneSleep::new(Arc::clone(&clock), start + Duration::from_secs(3600));
        for step in 0..1000u64 {
            sleep.reset(start + Duration::from_secs(3600) - Duration::from_millis(step));
            assert!(Pin::new(&mut sleep).poll(&mut cx).is_pending());
            assert!(
                clock.heap_len() <= 64 + 4,
                "heap stays bounded: {}",
                clock.heap_len()
            );
        }
        assert_eq!(
            clock.next_deadline(),
            Some(start + Duration::from_secs(3600) - Duration::from_millis(999))
        );
    }

    #[tokio::test]
    async fn a_closed_clock_falls_back_to_a_runtime_timer() {
        let driver = LaneClockDriver::new();
        let clock = Arc::clone(driver.clock());
        let deadline = Instant::now() + Duration::from_millis(15);
        let sleep = LaneSleep::new(clock, deadline);
        drop(driver);
        tokio::time::timeout(Duration::from_secs(2), sleep)
            .await
            .expect("the fallback timer fires");
        assert!(Instant::now() >= deadline);
    }

    #[tokio::test]
    async fn the_driver_fires_once_for_every_entry_due_in_the_same_tick() {
        let mut driver = LaneClockDriver::new();
        let start = Instant::now();
        let woke = Arc::new(AtomicUsize::new(0));
        let waker = counting_waker(Arc::clone(&woke));
        let mut cx = Context::from_waker(&waker);
        let mut sleeps: Vec<LaneSleep> = (0..5u64)
            .map(|i| {
                LaneSleep::new(
                    Arc::clone(driver.clock()),
                    start + Duration::from_millis(20) + Duration::from_micros(100 * i),
                )
            })
            .collect();
        for sleep in sleeps.iter_mut() {
            assert!(Pin::new(sleep).poll(&mut cx).is_pending());
        }
        let noop = futures::task::noop_waker();
        let mut driver_cx = Context::from_waker(&noop);
        assert!(driver.poll_timer(&mut driver_cx).is_pending());
        tokio::time::sleep_until(start + Duration::from_millis(25)).await;
        assert!(
            driver.poll_timer(&mut driver_cx).is_ready(),
            "one runtime timer for the whole lane"
        );
        let (due, arms) = driver.wake_due();
        assert_eq!((due, arms), (5, 5));
        assert_eq!(woke.load(Ordering::Acquire), 5);
        for sleep in sleeps.iter_mut() {
            assert!(Pin::new(sleep).poll(&mut cx).is_ready());
        }
        assert!(
            driver.poll_timer(&mut driver_cx).is_pending(),
            "nothing left to wait for"
        );
    }

    #[tokio::test]
    async fn an_expiry_whose_entries_wake_due_serves_is_still_counted_once() {
        // The poller's order: `wake_due` serves the entries the runtime timer
        // expired for, then `poll_timer` re-points the timer at the next
        // deadline (or finds none) and never sees the expiry as ready.
        for keep_later_entry in [true, false] {
            let mut driver = LaneClockDriver::new();
            let start = Instant::now();
            let waker = futures::task::noop_waker();
            let mut cx = Context::from_waker(&waker);
            let mut early = LaneSleep::new(
                Arc::clone(driver.clock()),
                start + Duration::from_millis(20),
            );
            let mut late =
                LaneSleep::new(Arc::clone(driver.clock()), start + Duration::from_secs(60));
            assert!(Pin::new(&mut early).poll(&mut cx).is_pending());
            if keep_later_entry {
                assert!(Pin::new(&mut late).poll(&mut cx).is_pending());
            }
            let noop = futures::task::noop_waker();
            let mut driver_cx = Context::from_waker(&noop);
            assert!(
                driver.poll_timer(&mut driver_cx).is_pending(),
                "armed at 20 ms"
            );
            // The runtime processes the lane clock's timer meanwhile.
            tokio::time::sleep_until(start + Duration::from_millis(30)).await;
            assert_eq!(driver.wake_due().0, 1, "the 20 ms entry is served first");
            assert!(
                driver.poll_timer(&mut driver_cx).is_pending(),
                "not ready: re-pointed or empty"
            );
            assert_eq!(
                driver.take_expiries(),
                1,
                "the expiry that woke the poller is counted"
            );
            assert!(driver.poll_timer(&mut driver_cx).is_pending());
            assert_eq!(driver.wake_due().0, 0);
            assert_eq!(driver.take_expiries(), 0, "and only once");
            drop(late);
        }
    }

    // ---- Lane tick: deadlines filed on the tick grid ----

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    #[tokio::test(start_paused = true)]
    async fn a_grid_clock_files_a_deadline_once_per_tick_and_counts_moves_inside_it() {
        let start = Instant::now();
        let grid = crate::lane_tick::TickGrid::new(start.into_std(), ms(5));
        let clock = Arc::new(LaneClock::with_grid(Some(grid)));
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        let mut sleep = LaneSleep::new(Arc::clone(&clock), start + ms(31));
        assert!(Pin::new(&mut sleep).poll(&mut cx).is_pending());
        assert_eq!(
            clock.next_deadline(),
            Some(start + ms(35)),
            "filed on its tick boundary"
        );
        // Moves inside the tick file nothing; repeating an instant is not a move.
        for offset in [32, 34, 35, 35, 33] {
            sleep.reset(start + ms(offset));
            assert!(Pin::new(&mut sleep).poll(&mut cx).is_pending());
        }
        assert_eq!(clock.heap_len(), 1);
        assert_eq!(clock.take_coalesced(), 4);
        assert_eq!(clock.take_coalesced(), 0);
        // Leaving the tick files once, under the new boundary.
        sleep.reset(start + ms(36));
        assert!(Pin::new(&mut sleep).poll(&mut cx).is_pending());
        assert_eq!(clock.heap_len(), 2);
        assert_eq!(
            clock.next_deadline(),
            Some(start + ms(40)),
            "the 35 ms item is stale"
        );
        let mut wakers = Vec::new();
        assert_eq!(
            clock.take_due(start + ms(39), &mut wakers),
            (0, 2),
            "two filings, nothing due"
        );
        assert_eq!(clock.take_due(start + ms(40), &mut wakers), (1, 0));
        // The sleep itself still waits for its exact instant.
        tokio::time::sleep_until(start + ms(35)).await;
        assert!(
            Pin::new(&mut sleep).poll(&mut cx).is_pending(),
            "never early"
        );
        tokio::time::sleep_until(start + ms(36)).await;
        assert!(Pin::new(&mut sleep).poll(&mut cx).is_ready());
        // Without a grid nothing is coalesced.
        let plain = LaneClock::with_grid(None);
        assert_eq!(plain.take_coalesced(), 0);
        assert_eq!(plain.filed_at(start + ms(31)), start + ms(31));
    }

    /// Serve every deadline of `driver`'s clock as a non-ticking poller does
    /// (arm the runtime timer, wait for it, wake what is due) and return
    /// (runtime timer expiries, registrations, entries woken).
    async fn serve_on_the_runtime_timer(driver: &mut LaneClockDriver) -> (u64, u64, u64) {
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        let mut woken = 0;
        for _ in 0..100 {
            woken += driver.wake_due().0;
            if driver.poll_timer(&mut cx).is_ready() {
                continue;
            }
            let Some(next) = driver.clock().next_deadline() else {
                break;
            };
            tokio::time::sleep_until(next).await;
        }
        (driver.take_expiries(), driver.take_resets(), woken)
    }

    #[tokio::test(start_paused = true)]
    async fn grid_timers_serve_the_deadlines_of_a_tick_with_one_runtime_timer_expiry() {
        // Five calls whose next frames are due 1 ms apart inside one 5 ms
        // tick: exact filing costs one runtime timer expiry and registration
        // per deadline, grid filing one for the whole tick. Counted, not
        // timed (paused clock). The timing wheel serves both alike.
        for wheel in [false, true] {
            let mut outcomes = Vec::new();
            for use_grid in [false, true] {
                let start = Instant::now();
                let grid =
                    use_grid.then(|| crate::lane_tick::TickGrid::new(start.into_std(), ms(5)));
                let mut driver = LaneClockDriver::with_timers(grid, wheel);
                let waker = futures::task::noop_waker();
                let mut cx = Context::from_waker(&waker);
                let mut sleeps: Vec<LaneSleep> = (11..=15u64)
                    .map(|offset| LaneSleep::new(Arc::clone(driver.clock()), start + ms(offset)))
                    .collect();
                for sleep in &mut sleeps {
                    assert!(Pin::new(sleep).poll(&mut cx).is_pending());
                }
                let outcome = serve_on_the_runtime_timer(&mut driver).await;
                for sleep in &mut sleeps {
                    assert!(
                        Pin::new(sleep).poll(&mut cx).is_ready(),
                        "served, never early"
                    );
                }
                assert!(
                    Instant::now() <= start + ms(15),
                    "and never late on a boundary"
                );
                outcomes.push(outcome);
            }
            assert_eq!(outcomes[0], (5, 5, 5), "exact: one expiry per deadline");
            assert_eq!(outcomes[1], (1, 1, 5), "grid: one expiry for the tick");
        }
    }

    #[tokio::test]
    async fn has_due_reports_a_due_live_deadline_only() {
        let driver = LaneClockDriver::new();
        let start = Instant::now();
        assert!(!driver.has_due(start), "an empty clock has nothing due");
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        let mut sleep = LaneSleep::new(Arc::clone(driver.clock()), start + ms(50));
        assert!(Pin::new(&mut sleep).poll(&mut cx).is_pending());
        assert!(!driver.has_due(start + ms(49)));
        assert!(driver.has_due(start + ms(50)));
        drop(sleep);
        assert!(!driver.has_due(start + ms(50)), "a released entry is stale");
    }

    #[test]
    fn grid_filing_serves_periodic_deadlines_on_their_tick_with_a_fake_clock() {
        grid_filing_case(false);
    }

    #[test]
    fn wheel_grid_filing_serves_periodic_deadlines_on_their_tick_with_a_fake_clock() {
        // The same schedule, filed in the timing wheel (one bucket per
        // tick): identical takes, tick by tick.
        grid_filing_case(true);
    }

    fn grid_filing_case(wheel: bool) {
        // 200 streams with 20 ms deadlines at random phases; each deadline is
        // nudged twice by up to ±300 µs before it is due (a pacing
        // correction). Driving `take_due` at every tick boundary of a fake
        // clock, every entry is taken exactly at the first boundary at or
        // after its final deadline (never before, less than a quantum
        // after), and the grid files far fewer heap items than exact filing.
        let base = Instant::now();
        let quantum = ms(2);
        let grid = crate::lane_tick::TickGrid::new(base.into_std(), quantum);
        let ticked = LaneClock::with_timers(Some(grid), wheel);
        let exact = LaneClock::with_grid(None);
        let waker = futures::task::noop_waker();
        let mut seed = 0x0e49_u64;
        let mut random = move |bound: u64| {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) % bound
        };
        struct Stream {
            ticked: (u32, u32),
            exact: (u32, u32),
            period: u32,
            due: Instant,
            armed: bool,
        }
        let arm =
            |stream: &mut Stream, base_deadline: Instant, random: &mut dyn FnMut(u64) -> u64| {
                let mut deadline = base_deadline;
                for nudge in 0..3 {
                    if nudge > 0 {
                        let offset = Duration::from_micros(random(600));
                        deadline = base_deadline + Duration::from_micros(300) - offset;
                    }
                    ticked.arm(stream.ticked.0, stream.ticked.1, deadline, &waker);
                    exact.arm(stream.exact.0, stream.exact.1, deadline, &waker);
                }
                stream.due = deadline;
                stream.armed = true;
            };
        let phases: Vec<u64> = (0..200).map(|_| random(20_000)).collect();
        let mut streams: Vec<Stream> = phases
            .iter()
            .map(|_| Stream {
                ticked: ticked.allocate(),
                exact: exact.allocate(),
                period: 0,
                due: base,
                armed: false,
            })
            .collect();
        let first = |phase: u64| base + ms(1) + Duration::from_micros(phase);
        for (stream, phase) in streams.iter_mut().zip(&phases) {
            arm(stream, first(*phase), &mut random);
        }
        let mut filed = 0;
        let mut served = 0;
        let mut wakers = Vec::new();
        for tick in 0..=120u32 {
            let now = base + quantum * tick;
            let (due, arms) = ticked.take_due(now, &mut wakers);
            filed += arms;
            let mut taken = 0;
            for (stream, phase) in streams.iter_mut().zip(&phases) {
                let slot = stream.ticked.0 as usize;
                if !stream.armed || ticked.state.lock().entries[slot].deadline.is_some() {
                    continue;
                }
                assert!(now >= stream.due, "never before the deadline");
                assert!(
                    now - stream.due < quantum,
                    "taken {:?} late",
                    now - stream.due
                );
                assert_eq!(
                    now.into_std(),
                    grid.ceil(stream.due.into_std()),
                    "on its own tick"
                );
                taken += 1;
                stream.armed = false;
                if stream.period < 9 {
                    stream.period += 1;
                    let next = first(*phase) + ms(20) * stream.period;
                    arm(stream, next, &mut random);
                }
            }
            assert_eq!(
                taken, due,
                "every due entry, and only those, at tick {tick}"
            );
            served += taken;
        }
        filed += ticked.take_due(base + ms(1_000), &mut wakers).1;
        assert_eq!(served, 200 * 10, "every deadline served");
        let exact_filed = exact.state.lock().arms;
        assert!(
            exact_filed > 2 * filed,
            "grid filings {filed} vs exact {exact_filed}"
        );
        assert!(ticked.state.lock().coalesced > 0);
    }

    // ---- Timing wheel and pass clock ----
    //
    // Fake-clock tests: every instant is `base + offset` (the wheel's origin
    // is `base`), so firing order, firing times and wrap-around are exact.

    fn us(value: u64) -> Duration {
        Duration::from_micros(value)
    }

    /// Deterministic pseudo-random sequence (no `rand` in this crate's tests).
    struct Lcg(u64);

    impl Lcg {
        fn below(&mut self, bound: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) % bound.max(1)
        }
    }

    /// A waker that appends `id` to `log` when woken.
    fn logging_waker(id: u32, log: Arc<parking_lot::Mutex<Vec<u32>>>) -> Waker {
        struct Log(u32, Arc<parking_lot::Mutex<Vec<u32>>>);
        impl std::task::Wake for Log {
            fn wake(self: Arc<Self>) {
                self.1.lock().push(self.0);
            }
        }
        Waker::from(Arc::new(Log(id, log)))
    }

    /// Wake `wakers` in order and return the ids they logged.
    fn woken_ids(wakers: &mut Vec<Waker>, log: &Arc<parking_lot::Mutex<Vec<u32>>>) -> Vec<u32> {
        for waker in wakers.drain(..) {
            waker.wake();
        }
        std::mem::take(&mut *log.lock())
    }

    /// A heap clock and a wheel clock driven with the same operations.
    struct Twin {
        heap: LaneClock,
        wheel: LaneClock,
        heap_log: Arc<parking_lot::Mutex<Vec<u32>>>,
        wheel_log: Arc<parking_lot::Mutex<Vec<u32>>>,
    }

    impl Twin {
        fn new(grid: Option<crate::lane_tick::TickGrid>, base: Instant) -> Self {
            Self {
                heap: LaneClock::with_timers_at(grid, false, base),
                wheel: LaneClock::with_timers_at(grid, true, base),
                heap_log: Arc::default(),
                wheel_log: Arc::default(),
            }
        }

        fn allocate(&self) -> (u32, u32) {
            let handle = self.heap.allocate();
            assert_eq!(self.wheel.allocate(), handle, "both slabs allocate alike");
            handle
        }

        fn arm(&self, (slot, generation): (u32, u32), deadline: Instant) {
            self.heap.arm(
                slot,
                generation,
                deadline,
                &logging_waker(slot, Arc::clone(&self.heap_log)),
            );
            self.wheel.arm(
                slot,
                generation,
                deadline,
                &logging_waker(slot, Arc::clone(&self.wheel_log)),
            );
        }

        fn disarm(&self, (slot, generation): (u32, u32)) {
            self.heap.disarm(slot, generation);
            self.wheel.disarm(slot, generation);
        }

        fn release(&self, (slot, generation): (u32, u32)) {
            self.heap.release(slot, generation);
            self.wheel.release(slot, generation);
        }

        /// Take the due entries of both clocks at `now`; assert the counts,
        /// the woken entries and their order are identical; return the ids.
        fn take_due(&self, now: Instant) -> Vec<u32> {
            let mut heap_wakers = Vec::new();
            let mut wheel_wakers = Vec::new();
            let heap = self.heap.take_due(now, &mut heap_wakers);
            let wheel = self.wheel.take_due(now, &mut wheel_wakers);
            assert_eq!(wheel, heap, "(due, arms) at {now:?}");
            let heap_ids = woken_ids(&mut heap_wakers, &self.heap_log);
            let wheel_ids = woken_ids(&mut wheel_wakers, &self.wheel_log);
            assert_eq!(wheel_ids, heap_ids, "firing order at {now:?}");
            heap_ids
        }

        fn assert_same_next(&self) -> Option<Instant> {
            let next = self.heap.next_deadline();
            assert_eq!(self.wheel.next_deadline(), next, "earliest deadline");
            next
        }
    }

    #[test]
    fn wheel_fires_exactly_like_the_heap_for_randomized_schedules() {
        // Property test with fixed seeds: random allocations, near, far and
        // past deadlines, re-arms, disarms and releases, and a fake clock
        // advancing by small steps, repeats and idle gaps far past the wheel's
        // span. After every operation both clocks report the same earliest
        // deadline; every take wakes the same entries in the same order; no
        // entry is ever taken before its exact deadline.
        let cases: [(u64, Option<u64>); 5] = [
            (0xe52_01, None),
            (0xe52_02, Some(2)),
            (0xe52_03, Some(5)),
            (0xe52_04, Some(1)),
            (0xe52_05, None),
        ];
        let mut total_ops = WheelOps::default();
        for (seed, quantum_ms) in cases {
            let base = Instant::now();
            let grid = quantum_ms.map(|q| crate::lane_tick::TickGrid::new(base.into_std(), ms(q)));
            let twin = Twin::new(grid, base);
            let mut rng = Lcg(seed);
            let mut handles: Vec<(u32, u32)> = Vec::new();
            let mut exact: std::collections::HashMap<u32, Instant> =
                std::collections::HashMap::new();
            let mut now = base;
            let mut fired = 0usize;
            for _ in 0..20_000 {
                match rng.below(100) {
                    0..=7 => handles.push(twin.allocate()),
                    8..=57 if !handles.is_empty() => {
                        let handle = handles[rng.below(handles.len() as u64) as usize];
                        let deadline = match rng.below(100) {
                            0..=59 => now + us(rng.below(60_000)),
                            60..=79 => now + us(rng.below(2_000)),
                            80..=91 => now + ms(250) + us(rng.below(9_000_000)),
                            _ => now.checked_sub(us(rng.below(5_000))).unwrap_or(now),
                        };
                        twin.arm(handle, deadline);
                        exact.insert(handle.0, deadline);
                    }
                    58..=61 if !handles.is_empty() => {
                        let handle = handles[rng.below(handles.len() as u64) as usize];
                        twin.disarm(handle);
                    }
                    62..=65 if !handles.is_empty() => {
                        let index = rng.below(handles.len() as u64) as usize;
                        twin.release(handles.swap_remove(index));
                    }
                    _ => {
                        now += match rng.below(100) {
                            0..=79 => us(rng.below(1_500)),
                            80..=94 => Duration::ZERO,
                            _ => ms(250) + us(rng.below(3_000_000)),
                        };
                        for id in twin.take_due(now) {
                            let deadline = exact[&id];
                            assert!(deadline <= now, "taken {:?} early", deadline - now);
                            fired += 1;
                        }
                    }
                }
                twin.assert_same_next();
            }
            // Drain: everything still filed fires once, identically.
            for id in twin.take_due(now + Duration::from_secs(60)) {
                assert!(exact[&id] <= now + Duration::from_secs(60));
                fired += 1;
            }
            assert_eq!(twin.assert_same_next(), None);
            assert!(fired > 1_000, "seed {seed:#x}: {fired} fires");
            let ops = twin.wheel.take_wheel_ops();
            total_ops.inserts += ops.inserts;
            total_ops.far_inserts += ops.far_inserts;
            total_ops.cancels += ops.cancels;
            total_ops.cascades += ops.cascades;
            total_ops.slot_scans += ops.slot_scans;
            assert_eq!(
                twin.heap.take_wheel_ops(),
                WheelOps::default(),
                "the heap counts no wheel ops"
            );
        }
        assert!(total_ops.inserts > 10_000, "{total_ops:?}");
        assert!(total_ops.far_inserts > 100, "{total_ops:?}");
        assert!(total_ops.cancels > 1_000, "{total_ops:?}");
        assert!(total_ops.cascades > 10, "{total_ops:?}");
        assert!(total_ops.slot_scans > 100, "{total_ops:?}");
    }

    #[test]
    fn wheel_takes_each_entry_at_its_first_check_at_or_after_its_deadline_and_never_before() {
        // Without the grid (1 ms buckets): 300 streams of 20 ms deadlines at
        // random µs phases, the clock checked every 300 µs. Each is taken at
        // the first check at or after it (so less than 300 µs late), never
        // before, including the many deadlines inside a partly elapsed bucket.
        let base = Instant::now();
        let clock = LaneClock::with_timers_at(None, true, base);
        let waker = futures::task::noop_waker();
        let mut rng = Lcg(0xe52_11);
        let step = us(300);
        let mut streams: Vec<((u32, u32), Instant)> = (0..300)
            .map(|_| {
                let handle = clock.allocate();
                let due = base + us(1 + rng.below(20_000));
                clock.arm(handle.0, handle.1, due, &waker);
                (handle, due)
            })
            .collect();
        let mut wakers = Vec::new();
        let mut served = 0;
        for check in 1..=10_000u32 {
            let now = base + step * check;
            let (due, _) = clock.take_due(now, &mut wakers);
            let mut taken = 0;
            for (handle, deadline) in streams.iter_mut() {
                let filed = clock.state.lock().entries[handle.0 as usize].deadline;
                if filed.is_some() {
                    assert!(*deadline > now, "a due entry was left filed");
                    continue;
                }
                assert!(*deadline <= now, "never before its deadline");
                assert!(now - *deadline < step, "taken {:?} late", now - *deadline);
                taken += 1;
                *deadline += ms(20);
                clock.arm(handle.0, handle.1, *deadline, &waker);
            }
            assert_eq!(taken, due);
            served += taken;
        }
        // 3 s of 20 ms periods for 300 streams.
        assert!(served >= 300 * 149, "served {served}");
        let ops = clock.take_wheel_ops();
        assert_eq!(
            ops.far_inserts, 0,
            "20 ms deadlines never leave the wheel's span"
        );
        assert!(ops.slot_scans > 0);
    }

    #[test]
    fn wheel_wraps_around_its_buckets_many_times() {
        // With the grid (Q = 2 ms, 256 buckets = 512 ms) a 20 ms series runs
        // for 10 s: the cursor wraps about 20 times. Every deadline is taken
        // exactly on its tick boundary; then an idle gap of 7 s (several
        // spans) takes everything left at once, in (key, slot) order.
        let base = Instant::now();
        let quantum = ms(2);
        let grid = crate::lane_tick::TickGrid::new(base.into_std(), quantum);
        let clock = LaneClock::with_timers(Some(grid), true);
        let log = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let mut rng = Lcg(0xe52_21);
        let mut streams: Vec<((u32, u32), Instant)> = (0..64)
            .map(|_| {
                let handle = clock.allocate();
                let due = base + us(1 + rng.below(40_000));
                clock.arm(
                    handle.0,
                    handle.1,
                    due,
                    &logging_waker(handle.0, Arc::clone(&log)),
                );
                (handle, due)
            })
            .collect();
        let mut wakers = Vec::new();
        for tick in 1..=5_000u32 {
            let now = base + quantum * tick;
            clock.take_due(now, &mut wakers);
            let ids = woken_ids(&mut wakers, &log);
            for id in ids {
                let (handle, deadline) = streams
                    .iter_mut()
                    .find(|(handle, _)| handle.0 == id)
                    .expect("a stream");
                assert_eq!(
                    now.into_std(),
                    grid.ceil(deadline.into_std()),
                    "on its own boundary"
                );
                *deadline += ms(20);
                let waker = logging_waker(handle.0, Arc::clone(&log));
                clock.arm(handle.0, handle.1, *deadline, &waker);
            }
            for (handle, deadline) in &streams {
                if clock.state.lock().entries[handle.0 as usize]
                    .deadline
                    .is_some()
                {
                    assert!(
                        grid.ceil(deadline.into_std()) > now.into_std(),
                        "left behind at tick {tick}"
                    );
                }
            }
        }
        // Idle gap: nothing is checked for 7 s.
        let after_gap = base + quantum * 5_000 + Duration::from_secs(7);
        let (due, _) = clock.take_due(after_gap, &mut wakers);
        assert_eq!(due, 64, "everything due after the gap");
        let mut expected: Vec<(Instant, u32)> = streams
            .iter()
            .map(|(handle, deadline)| (Instant::from_std(grid.ceil(deadline.into_std())), handle.0))
            .collect();
        expected.sort();
        assert_eq!(
            woken_ids(&mut wakers, &log),
            expected.iter().map(|(_, id)| *id).collect::<Vec<_>>()
        );
        assert_eq!(clock.next_deadline(), None);
        assert_eq!(clock.wheel_len(), (0, 0));
    }

    #[test]
    fn wheel_far_deadlines_wait_in_the_overflow_and_cascade_in_time() {
        let base = Instant::now();
        let clock = LaneClock::with_timers_at(None, true, base);
        let waker = futures::task::noop_waker();
        let far_handle = clock.allocate();
        let near_handle = clock.allocate();
        let far = base + Duration::from_secs(10) + us(300);
        let near = base + ms(5);
        clock.arm(far_handle.0, far_handle.1, far, &waker);
        clock.arm(near_handle.0, near_handle.1, near, &waker);
        assert_eq!(
            clock.wheel_len(),
            (1, 1),
            "one in a bucket, one in the overflow"
        );
        assert_eq!(clock.next_deadline(), Some(near));
        let mut wakers = Vec::new();
        assert_eq!(clock.take_due(near, &mut wakers).0, 1);
        assert_eq!(
            clock.next_deadline(),
            Some(far),
            "the overflow answers when the wheel is empty"
        );
        // Within the span (256 ms) of the far deadline it moves into a bucket.
        assert_eq!(clock.take_due(far - ms(200), &mut wakers).0, 0);
        assert_eq!(clock.wheel_len(), (1, 0), "cascaded");
        assert_eq!(clock.next_deadline(), Some(far));
        assert_eq!(clock.take_due(far - us(1), &mut wakers).0, 0, "never early");
        assert_eq!(clock.take_due(far, &mut wakers).0, 1);
        let ops = clock.take_wheel_ops();
        assert_eq!((ops.inserts, ops.far_inserts, ops.cascades), (1, 1, 1));

        // A far deadline moved to another far one, back near, and far again
        // to its first key leaves stale overflow items; it still fires once.
        let now = far;
        let handle = clock.allocate();
        let first = now + Duration::from_secs(5);
        clock.arm(handle.0, handle.1, first, &waker);
        clock.arm(handle.0, handle.1, now + Duration::from_secs(6), &waker);
        clock.arm(handle.0, handle.1, now + ms(3), &waker);
        clock.arm(handle.0, handle.1, first, &waker);
        assert_eq!(clock.next_deadline(), Some(first));
        assert_eq!(
            clock.take_due(now + ms(3), &mut wakers).0,
            0,
            "the near filing was cancelled"
        );
        assert_eq!(clock.take_due(first - us(1), &mut wakers).0, 0);
        assert_eq!(clock.take_due(first, &mut wakers).0, 1, "fires once");
        assert_eq!(
            clock
                .take_due(first + Duration::from_secs(10), &mut wakers)
                .0,
            0
        );
        assert_eq!(clock.next_deadline(), None);

        // A far deadline that keeps moving does not grow the overflow.
        let mover = clock.allocate();
        for step in 0..1_000u64 {
            clock.arm(
                mover.0,
                mover.1,
                first + Duration::from_secs(3_600) - ms(step),
                &waker,
            );
            assert!(
                clock.wheel_len().1 <= 64 + 4,
                "overflow stays bounded: {:?}",
                clock.wheel_len()
            );
        }
        assert_eq!(
            clock.next_deadline(),
            Some(first + Duration::from_secs(3_600) - ms(999))
        );
    }

    #[test]
    fn wheel_cancel_and_rearm_relink_in_constant_work() {
        let base = Instant::now();
        let clock = LaneClock::with_timers_at(None, true, base);
        let waker = futures::task::noop_waker();
        let handles: Vec<(u32, u32)> = (0..3).map(|_| clock.allocate()).collect();
        for (index, handle) in handles.iter().enumerate() {
            clock.arm(
                handle.0,
                handle.1,
                base + ms(10) + us(100 * index as u64),
                &waker,
            );
        }
        // Re-arming the earliest many times: one link each, no growth.
        for step in 0..500u64 {
            clock.arm(
                handles[0].0,
                handles[0].1,
                base + ms(1 + step % 200),
                &waker,
            );
            assert_eq!(clock.wheel_len(), (3, 0));
        }
        let ops = clock.take_wheel_ops();
        assert_eq!(ops.inserts, 3 + 500);
        assert_eq!(ops.cancels, 500);
        // The moved entry left the 10 ms bucket; its floor followed.
        let last = base + ms(1 + 499 % 200);
        assert_eq!(clock.next_deadline(), Some(base + ms(10) + us(100)));
        // Re-arming the same instant files nothing.
        clock.arm(handles[0].0, handles[0].1, last, &waker);
        assert_eq!(clock.take_wheel_ops(), WheelOps::default());
        // Disarm and release unlink; a reused slot never fires the old key.
        clock.disarm(handles[0].0, handles[0].1);
        assert_eq!(clock.next_deadline(), Some(base + ms(10) + us(100)));
        clock.release(handles[1].0, handles[1].1);
        assert_eq!(clock.next_deadline(), Some(base + ms(10) + us(200)));
        let reused = clock.allocate();
        assert_eq!(reused.0, handles[1].0);
        assert_ne!(reused.1, handles[1].1, "a new generation");
        assert_eq!(clock.wheel_len(), (1, 0));
        let mut wakers = Vec::new();
        assert_eq!(
            clock.take_due(base + ms(100), &mut wakers).0,
            1,
            "only the live entry"
        );
        let ops = clock.take_wheel_ops();
        assert_eq!(ops.cancels, 2, "disarm and release");
        // A cancelled earliest key leaves a lower bound; the next lookup is exact.
        let a = clock.allocate();
        let b = clock.allocate();
        clock.arm(a.0, a.1, base + ms(150) + us(100), &waker);
        clock.arm(b.0, b.1, base + ms(150) + us(700), &waker);
        clock.arm(a.0, a.1, base + ms(180), &waker);
        assert_eq!(clock.next_deadline(), Some(base + ms(150) + us(700)));
    }

    #[tokio::test]
    async fn wheel_lane_sleeps_wake_in_deadline_order_and_never_early() {
        let clock = Arc::new(LaneClock::with_timers(None, true));
        let start = Instant::now();
        let woke = Arc::new(AtomicUsize::new(0));
        let waker = counting_waker(Arc::clone(&woke));
        let mut cx = Context::from_waker(&waker);
        let early_at = start + Duration::from_millis(30);
        let late_at = start + Duration::from_millis(400);
        let mut late = LaneSleep::new(Arc::clone(&clock), late_at);
        let mut early = LaneSleep::new(Arc::clone(&clock), early_at);
        assert!(Pin::new(&mut late).poll(&mut cx).is_pending());
        assert!(Pin::new(&mut early).poll(&mut cx).is_pending());
        assert_eq!(clock.next_deadline(), Some(early_at));
        let mut wakers = Vec::new();
        assert_eq!(
            clock.take_due(early_at - Duration::from_millis(1), &mut wakers),
            (0, 2)
        );
        assert_eq!(clock.take_due(early_at, &mut wakers), (1, 0));
        for waker in wakers.drain(..) {
            waker.wake();
        }
        assert_eq!(
            woke.load(Ordering::Acquire),
            1,
            "only the due entry was woken"
        );
        tokio::time::sleep_until(early_at).await;
        assert!(Pin::new(&mut early).poll(&mut cx).is_ready());
        assert!(
            Pin::new(&mut late).poll(&mut cx).is_pending(),
            "never before its instant"
        );
        assert_eq!(clock.next_deadline(), Some(late_at));
        drop(early);
        drop(late);
        assert_eq!(clock.live(), 0);
        assert_eq!(clock.next_deadline(), None);
    }

    #[tokio::test]
    async fn wheel_driver_fires_once_for_every_entry_due_in_the_same_tick() {
        let mut driver = LaneClockDriver::with_timers(None, true);
        let start = Instant::now();
        let woke = Arc::new(AtomicUsize::new(0));
        let waker = counting_waker(Arc::clone(&woke));
        let mut cx = Context::from_waker(&waker);
        let mut sleeps: Vec<LaneSleep> = (0..5u64)
            .map(|i| LaneSleep::new(Arc::clone(driver.clock()), start + ms(20) + us(100 * i)))
            .collect();
        for sleep in sleeps.iter_mut() {
            assert!(Pin::new(sleep).poll(&mut cx).is_pending());
        }
        let noop = futures::task::noop_waker();
        let mut driver_cx = Context::from_waker(&noop);
        assert!(driver.poll_timer(&mut driver_cx).is_pending());
        tokio::time::sleep_until(start + ms(25)).await;
        assert!(
            driver.poll_timer(&mut driver_cx).is_ready(),
            "one runtime timer for the whole lane"
        );
        assert_eq!(driver.wake_due(), (5, 5));
        assert_eq!(woke.load(Ordering::Acquire), 5);
        for sleep in sleeps.iter_mut() {
            assert!(Pin::new(sleep).poll(&mut cx).is_ready());
        }
        assert!(
            driver.poll_timer(&mut driver_cx).is_pending(),
            "nothing left to wait for"
        );
        assert_eq!(driver.take_expiries(), 1, "one expiry, counted once");
        // A closed wheel clock falls back to runtime timers, as the heap does.
        let deadline = Instant::now() + ms(15);
        let late = LaneSleep::new(Arc::clone(driver.clock()), deadline);
        drop(driver);
        tokio::time::timeout(Duration::from_secs(2), late)
            .await
            .expect("the fallback timer fires");
        assert!(Instant::now() >= deadline);
    }

    #[test]
    fn pass_clock_serves_one_reading_within_its_read_bound() {
        assert_eq!(
            pass_cached_now(),
            None,
            "outside a pass the caller reads the clock"
        );
        assert_eq!(pass_refresh(), None);
        let _ = take_pass_clock_stats();
        let seed = Instant::now() - ms(1);
        {
            let _pass = enter_pass(seed);
            for _ in 0..PASS_CLOCK_MAX_READS {
                assert_eq!(
                    pass_cached_now(),
                    Some(seed),
                    "served from the pass reading"
                );
            }
            let fresh = pass_cached_now().expect("in a pass");
            assert!(
                fresh >= seed + ms(1),
                "the read bound forces a fresh reading"
            );
            assert_eq!(
                pass_cached_now(),
                Some(fresh),
                "which then serves the next reads"
            );
            pass_invalidate();
            let third = pass_cached_now().expect("in a pass");
            assert!(third >= fresh, "monotonic");
            let fourth = pass_refresh().expect("in a pass");
            assert!(fourth >= third);
            assert!(fourth <= Instant::now(), "never ahead of the clock");
        }
        assert_eq!(pass_cached_now(), None, "the guard ends the pass");
        let stats = take_pass_clock_stats();
        assert_eq!(stats.cached, u64::from(PASS_CLOCK_MAX_READS) + 1);
        assert_eq!(stats.fresh, 4, "the seed and three refreshes");
        assert_eq!(stats.stale_samples, 3);
        assert!(
            stats.stale_max_ns >= 1_000_000,
            "the 1 ms old seed was measured: {stats:?}"
        );
        assert!(stats.stale_sum_ns >= stats.stale_max_ns);
        assert_eq!(take_pass_clock_stats(), PassClockStats::default());
    }

    #[tokio::test]
    async fn a_stale_pass_reading_delays_a_lane_sleep_but_never_completes_it_early() {
        // The pass reading predates a deadline that has truly passed: the
        // sleep stays pending (filed, so the next round serves it) and
        // completes once a fresh reading is at or after its instant.
        let clock = Arc::new(LaneClock::with_timers(None, true));
        let real = Instant::now();
        let deadline = real - ms(5);
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        let mut sleep = LaneSleep::new(Arc::clone(&clock), deadline);
        {
            let _pass = enter_pass(real - ms(10));
            assert!(
                Pin::new(&mut sleep).poll(&mut cx).is_pending(),
                "judged by the pass reading"
            );
            assert_eq!(clock.next_deadline(), Some(deadline), "and filed");
            let mut wakers = Vec::new();
            let now = pass_refresh().expect("in a pass");
            assert_eq!(
                clock.take_due(now, &mut wakers).0,
                1,
                "the next round takes it"
            );
            assert!(Pin::new(&mut sleep).poll(&mut cx).is_ready());
        }
        let _ = take_pass_clock_stats();
    }
}
