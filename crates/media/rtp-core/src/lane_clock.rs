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
    /// The deadline the entry waits for (`None`: idle or already woken).
    deadline: Option<Instant>,
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
}

impl LaneClock {
    pub(crate) fn new() -> Self {
        Self {
            state: parking_lot::Mutex::new(ClockState::default()),
            closed: AtomicBool::new(false),
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
        entry.waker = None;
        state.live -= 1;
        state.free.push(slot);
        state.compact_if_needed();
    }

    /// Wait for `deadline` with `waker`. Files a heap item only when the
    /// deadline changed; refreshes the waker only when it would wake a
    /// different task.
    fn arm(&self, slot: u32, generation: u32, deadline: Instant, waker: &Waker) {
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
        if entry.deadline != Some(deadline) {
            entry.deadline = Some(deadline);
            state.heap.push(Reverse((deadline, slot, generation)));
            state.arms += 1;
            state.compact_if_needed();
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
}

impl LaneClockDriver {
    pub(crate) fn new() -> Self {
        Self {
            clock: Arc::new(LaneClock::new()),
            sleep: None,
            armed: None,
            wakers: Vec::new(),
            counted: None,
            expiries: 0,
        }
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
}
