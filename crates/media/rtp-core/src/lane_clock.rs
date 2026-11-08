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

use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

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
}

#[derive(Default)]
struct ClockState {
    entries: Vec<Entry>,
    free: Vec<u32>,
    /// (deadline, slot, generation); stale when the slot's current deadline
    /// or generation differs.
    heap: BinaryHeap<Reverse<(Instant, u32, u32)>>,
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

    pub(crate) fn with_grid(grid: Option<crate::lane_tick::TickGrid>) -> Self {
        Self {
            state: parking_lot::Mutex::new(ClockState::default()),
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
            state.heap.clear();
            state
                .entries
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
        });
        (slot, 0)
    }

    fn release(&self, slot: u32, generation: u32) {
        let mut state = self.state.lock();
        let Some(entry) = state.entries.get_mut(slot as usize) else {
            return;
        };
        if !entry.in_use || entry.generation != generation {
            return;
        }
        entry.in_use = false;
        entry.generation = entry.generation.wrapping_add(1);
        entry.deadline = None;
        entry.exact = None;
        entry.waker = None;
        state.live -= 1;
        state.free.push(slot);
        state.compact_if_needed();
    }

    /// Wait for `deadline` with `waker`. Files a heap item only when the
    /// filed key changed (tick grid: when the deadline left its tick);
    /// refreshes the waker only when it would wake a different task.
    fn arm(&self, slot: u32, generation: u32, deadline: Instant, waker: &Waker) {
        let key = self.filed_at(deadline);
        let mut state = self.state.lock();
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
            entry.deadline = Some(key);
            entry.exact = Some(deadline);
            state.heap.push(Reverse((key, slot, generation)));
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
        if let Some(entry) = state.entries.get_mut(slot as usize) {
            if entry.in_use && entry.generation == generation {
                entry.deadline = None;
            }
        }
    }

    /// The earliest live deadline, discarding stale items on the way.
    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        let mut state = self.state.lock();
        while let Some(Reverse((deadline, slot, generation))) = state.heap.peek().copied() {
            if state.is_current(deadline, slot, generation) {
                return Some(deadline);
            }
            state.heap.pop();
        }
        None
    }

    /// Collect the wakers of every entry due at `now` into `wakers` (woken
    /// by the caller after the lock is released). Returns how many entries
    /// were due and how many deadlines were filed since the last call.
    pub(crate) fn take_due(&self, now: Instant, wakers: &mut Vec<Waker>) -> (u64, u64) {
        let mut state = self.state.lock();
        let mut due = 0;
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

    #[cfg(test)]
    pub(crate) fn heap_len(&self) -> usize {
        self.state.lock().heap.len()
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
        // Never ready before the instant, however the wake arrived.
        if Instant::now() >= this.deadline {
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
        Self::with_grid(None)
    }

    /// A driver whose clock files deadlines on `grid` (lane tick grid
    /// timers), or at their instant (`None`).
    pub(crate) fn with_grid(grid: Option<crate::lane_tick::TickGrid>) -> Self {
        Self {
            clock: Arc::new(LaneClock::with_grid(grid)),
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
    pub(crate) fn wake_due(&mut self) -> (u64, u64) {
        self.note_expiry();
        let (due, arms) = self.clock.take_due(Instant::now(), &mut self.wakers);
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
        // timed (paused clock).
        let mut outcomes = Vec::new();
        for use_grid in [false, true] {
            let start = Instant::now();
            let grid = use_grid.then(|| crate::lane_tick::TickGrid::new(start.into_std(), ms(5)));
            let mut driver = LaneClockDriver::with_grid(grid);
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
        // 200 streams with 20 ms deadlines at random phases; each deadline is
        // nudged twice by up to ±300 µs before it is due (a pacing
        // correction). Driving `take_due` at every tick boundary of a fake
        // clock, every entry is taken exactly at the first boundary at or
        // after its final deadline (never before, less than a quantum
        // after), and the grid files far fewer heap items than exact filing.
        let base = Instant::now();
        let quantum = ms(2);
        let grid = crate::lane_tick::TickGrid::new(base.into_std(), quantum);
        let ticked = LaneClock::with_grid(Some(grid));
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
}
