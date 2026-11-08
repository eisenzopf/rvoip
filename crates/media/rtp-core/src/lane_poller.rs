//! Executor v2: one receive poller per media lane worker.
//!
//! Under v2 each call's RTP receive loop used to be its own Tokio task, so a
//! media lane spent one task wakeup (schedule, poll, epoll round) per
//! inbound packet and another per owner-timer expiry. Here the receive loops
//! of a lane run *inside* one poller task per lane worker, in the style of
//! rtpengine's poller-per-thread: the loops are polled through a
//! [`FuturesUnordered`], which polls only the loops whose socket (or owner
//! timer) became ready, so every socket and timer that became ready in the
//! same reactor turn is served by one task wakeup.
//!
//! The receive loop itself is unchanged (same code, same per-socket drain
//! budget, same owner timer), so every per-packet semantic is unchanged;
//! only the task that polls it differs. What a dedicated task gave each loop
//! is kept explicitly:
//! - **Cancellation and join.** [`LaneTaskHandle`] aborts a hosted loop
//!   (`futures::future::Abortable`) and resolves `join` once the loop's
//!   future has been dropped, exactly when awaiting an aborted `JoinHandle`
//!   would.
//! - **Panic isolation.** A panicking loop is caught at its own poll
//!   boundary and dropped; the other loops on the lane keep running.
//! - **Fairness.** One poller wakeup polls at most
//!   [`LANE_SOCKET_POLLS_PER_WAKEUP`] loops, then yields to the runtime, and
//!   Tokio's cooperative budget still applies to the whole poller task.
//!   Within a socket, the v2 drain budget
//!   ([`crate::executor::INLINE_DRAIN_BUDGET`]) still yields a bursting
//!   socket to its lane neighbours.
//!
//! A lane that is a multi-thread runtime gets one poller per worker and
//! loops are assigned round-robin, so a lane keeps its parallelism.

use std::cell::Cell;
use std::collections::HashMap;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::OnceLock;
use std::task::{Context, Poll};

use futures::future::{AbortHandle, Abortable};
use futures::stream::{FuturesUnordered, StreamExt};
use futures::FutureExt;
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot};

use crate::executor::{record_lane_socket, LanePollerCounters};

/// A receive loop hosted on a lane poller.
pub(crate) type LaneFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// Loop polls one poller wakeup may spend before it yields to the runtime.
pub(crate) const LANE_SOCKET_POLLS_PER_WAKEUP: u32 = 256;

thread_local! {
    /// Datagrams received by hosted loops during the current poller poll.
    static LANE_DATAGRAMS: Cell<u64> = const { Cell::new(0) };
    /// Loop polls left in the current poller poll; `None` outside one.
    static LANE_POLL_BUDGET: Cell<Option<u32>> = const { Cell::new(None) };
}

/// A hosted receive loop received one datagram. Called only from loops that
/// run on a lane poller (one thread-local add).
#[inline]
pub(crate) fn note_lane_datagram() {
    LANE_DATAGRAMS.with(|count| count.set(count.get() + 1));
}

/// Cancellation and join authority over one hosted loop, the lane
/// counterpart of a `JoinHandle<()>`.
pub(crate) struct LaneTaskHandle {
    abort: AbortHandle,
    done: oneshot::Receiver<()>,
}

impl LaneTaskHandle {
    /// Stop the loop. Its future is dropped by the poller at its next poll,
    /// as an aborted task's future is dropped by the runtime.
    pub(crate) fn abort(&self) {
        self.abort.abort();
    }

    /// Resolve once the loop's future has been dropped (it returned, was
    /// aborted, panicked, or its poller shut down with the runtime).
    pub(crate) async fn join(self) {
        let _ = self.done.await;
    }
}

/// Polls a hosted loop only while the poller's per-wakeup budget lasts.
/// Out of budget it asks to be polled again; `FuturesUnordered` then yields
/// the poller task back to the runtime.
struct Budgeted {
    inner: LaneFuture,
}

impl Future for Budgeted {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let allowed = LANE_POLL_BUDGET.with(|budget| match budget.get() {
            Some(0) => false,
            Some(left) => {
                budget.set(Some(left - 1));
                true
            }
            None => true,
        });
        if !allowed {
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        self.get_mut().inner.as_mut().poll(cx)
    }
}

/// One poller task: hosts receive loops sent to it and polls the ready ones.
pub(crate) struct LanePoller {
    jobs: mpsc::UnboundedSender<LaneFuture>,
}

impl LanePoller {
    /// Start a poller task on `runtime`.
    pub(crate) fn start(runtime: &Handle) -> Self {
        let (jobs, receiver) = mpsc::unbounded_channel();
        drop(runtime.spawn(run_poller(receiver)));
        Self { jobs }
    }

    /// Host `future` on this poller. When the poller is gone (its runtime
    /// shut down) the future comes back boxed, ready to be spawned.
    pub(crate) fn host<F>(&self, future: F) -> Result<LaneTaskHandle, LaneFuture>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let hosted = self.host_uncounted(future)?;
        record_lane_socket();
        Ok(hosted)
    }

    /// [`Self::host`] without counting a socket (paced TX tasks are
    /// counted by their caller as hosted tasks).
    pub(crate) fn host_uncounted<F>(&self, future: F) -> Result<LaneTaskHandle, LaneFuture>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let (abort, registration) = AbortHandle::new_pair();
        let (done_tx, done_rx) = oneshot::channel::<()>();
        let hosted: LaneFuture = Box::pin(async move {
            // Dropped with this future, however it ends.
            let _done = done_tx;
            let guarded = Abortable::new(AssertUnwindSafe(future).catch_unwind(), registration);
            if let Ok(Err(_panic)) = guarded.await {
                tracing::error!(
                    "media receive loop panicked on its lane poller; only that loop ended"
                );
            }
        });
        match self.jobs.send(hosted) {
            Ok(()) => Ok(LaneTaskHandle {
                abort,
                done: done_rx,
            }),
            Err(mpsc::error::SendError(hosted)) => Err(hosted),
        }
    }
}

/// Lane-clock expiries one poller poll serves before it yields. Each
/// round wakes every entry that is due, so more than one round only happens
/// when polling the woken futures took past the next deadline.
const LANE_TIMER_ROUNDS_PER_WAKEUP: u32 = 4;

async fn run_poller(mut jobs: mpsc::UnboundedReceiver<LaneFuture>) {
    let mut loops: FuturesUnordered<Budgeted> = FuturesUnordered::new();
    let mut counters = LanePollerCounters::new();
    // The lane clock. Futures polled below file their deadlines in it
    // (through `media_clock::sleep_until`) instead of the runtime's timer
    // wheel when the lane timer is enabled; the poller arms one runtime timer
    // at its earliest deadline. Without the switch the clock stays empty.
    let lane_timer = crate::executor::lane_timer_enabled();
    let mut clock = crate::lane_clock::LaneClockDriver::new();
    let mut timer_counters = crate::executor::LaneTimerCounters::new();
    let mut accepting = true;
    std::future::poll_fn(|cx| {
        LANE_DATAGRAMS.with(|count| count.set(0));
        LANE_POLL_BUDGET.with(|budget| budget.set(Some(LANE_SOCKET_POLLS_PER_WAKEUP)));
        let _current_clock = lane_timer.then(|| crate::lane_clock::enter(clock.clock()));
        while accepting {
            match jobs.poll_recv(cx) {
                Poll::Ready(Some(job)) => loops.push(Budgeted { inner: job }),
                Poll::Ready(None) => accepting = false,
                Poll::Pending => break,
            }
        }
        let mut rounds = 0;
        loop {
            // Wake every lane-clock entry that is due, so its future is
            // polled in this same pass.
            let mut fired = false;
            if lane_timer {
                let (wakes, arms) = clock.wake_due();
                timer_counters.poll(false, wakes, arms);
            }
            // Each `Ready(Some(()))` is one loop that ended; keep serving the
            // others. `Pending` means every ready loop was polled (or the
            // poller must yield); `Ready(None)` means no loop is hosted.
            while let Poll::Ready(Some(())) = loops.poll_next_unpin(cx) {}
            if lane_timer && clock.poll_timer(cx).is_ready() {
                fired = true;
            }
            if !fired {
                break;
            }
            rounds += 1;
            if rounds >= LANE_TIMER_ROUNDS_PER_WAKEUP {
                // Serve the rest on the next poll.
                cx.waker().wake_by_ref();
                break;
            }
        }
        // Runtime timer expiries this pass, each counted once by the driver
        // (also those whose entries `wake_due` served first).
        for _ in 0..clock.take_expiries() {
            timer_counters.poll(true, 0, 0);
        }
        let exhausted = LANE_POLL_BUDGET.with(|budget| budget.replace(None)) == Some(0);
        let datagrams = LANE_DATAGRAMS.with(|count| count.replace(0));
        counters.wakeup(datagrams, exhausted);
        if !accepting && loops.is_empty() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
    .await;
}

struct LaneShards {
    pollers: Vec<LanePoller>,
    next: usize,
}

fn registry() -> &'static parking_lot::Mutex<HashMap<tokio::runtime::Id, LaneShards>> {
    static LANES: OnceLock<parking_lot::Mutex<HashMap<tokio::runtime::Id, LaneShards>>> =
        OnceLock::new();
    LANES.get_or_init(|| parking_lot::Mutex::new(HashMap::new()))
}

/// Host a receive loop on one of `runtime`'s lane pollers (one per worker of
/// that runtime, started on first use, assigned round-robin). Control-plane
/// rate: once per transport. When the runtime can no longer run a poller
/// the loop comes back to be spawned as its own task.
pub(crate) fn host_on_lane<F>(runtime: &Handle, future: F) -> Result<LaneTaskHandle, LaneFuture>
where
    F: Future<Output = ()> + Send + 'static,
{
    host_with(runtime, future, true)
}

/// Host a non-socket future (a paced TX task) the same way, without
/// counting it as a socket.
pub(crate) fn host_task_on_lane<F>(
    runtime: &Handle,
    future: F,
) -> Result<LaneTaskHandle, LaneFuture>
where
    F: Future<Output = ()> + Send + 'static,
{
    host_with(runtime, future, false)
}

fn host_with<F>(runtime: &Handle, future: F, socket: bool) -> Result<LaneTaskHandle, LaneFuture>
where
    F: Future<Output = ()> + Send + 'static,
{
    let mut lanes = registry().lock();
    let id = runtime.id();
    let shards = lanes.entry(id).or_insert_with(|| {
        let workers = runtime.metrics().num_workers().max(1);
        LaneShards {
            pollers: (0..workers).map(|_| LanePoller::start(runtime)).collect(),
            next: 0,
        }
    });
    let index = shards.next % shards.pollers.len();
    shards.next = shards.next.wrapping_add(1);
    let poller = &shards.pollers[index];
    let hosted = if socket {
        poller.host(future)
    } else {
        poller.host_uncounted(future)
    };
    if hosted.is_err() {
        // The runtime is shutting down; forget its pollers.
        lanes.remove(&id);
    }
    hosted
}

/// Whether `runtime` has lane pollers (test seam for placement checks).
#[cfg(test)]
pub(crate) fn hosts_pollers_on(runtime: &Handle) -> bool {
    registry().lock().contains_key(&runtime.id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn a_hosted_loop_runs_and_join_resolves_when_it_returns() {
        let poller = LanePoller::start(&Handle::current());
        let (tx, rx) = oneshot::channel();
        let handle = poller
            .host(async move {
                let _ = tx.send(7u8);
            })
            .ok()
            .expect("poller is running");
        assert_eq!(rx.await, Ok(7));
        tokio::time::timeout(Duration::from_secs(2), handle.join())
            .await
            .expect("join resolves after the loop returned");
    }

    #[tokio::test]
    async fn abort_drops_the_loop_and_releases_what_it_owned() {
        let poller = LanePoller::start(&Handle::current());
        let owned = Arc::new(());
        let held = Arc::clone(&owned);
        let handle = poller
            .host(async move {
                let _held = held;
                std::future::pending::<()>().await;
            })
            .ok()
            .expect("poller is running");
        tokio::task::yield_now().await;
        assert_eq!(Arc::strong_count(&owned), 2, "the loop is alive");
        handle.abort();
        tokio::time::timeout(Duration::from_secs(2), handle.join())
            .await
            .expect("join resolves after abort");
        assert_eq!(
            Arc::strong_count(&owned),
            1,
            "aborting dropped the loop's state"
        );
    }

    /// Always true; keeps the panicking loop's output type `()`.
    fn ticks_seen_once() -> bool {
        true
    }

    #[tokio::test]
    async fn a_panicking_loop_ends_alone() {
        let poller = LanePoller::start(&Handle::current());
        let ticks = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&ticks);
        let survivor = poller
            .host(async move {
                while counted.fetch_add(1, Ordering::AcqRel) < usize::MAX {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .ok()
            .expect("poller is running");
        let panicking = poller
            .host(async {
                tokio::task::yield_now().await;
                if ticks_seen_once() {
                    panic!("loop failure under test");
                }
            })
            .ok()
            .expect("poller is running");
        tokio::time::timeout(Duration::from_secs(2), panicking.join())
            .await
            .expect("a panicked loop is dropped and joins");
        let seen = ticks.load(Ordering::Acquire);
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(
            ticks.load(Ordering::Acquire) > seen,
            "the other loop keeps running"
        );
        survivor.abort();
        survivor.join().await;
    }

    #[tokio::test]
    async fn a_closed_poller_hands_the_loop_back() {
        let (jobs, receiver) = mpsc::unbounded_channel::<LaneFuture>();
        drop(receiver);
        let poller = LanePoller { jobs };
        let back = poller.host(async {});
        assert!(back.is_err(), "the caller spawns it instead");
    }

    #[tokio::test]
    async fn one_wakeup_serves_every_loop_that_became_ready_together() {
        // Eight loops wait on one shared notification; one release readies
        // all of them, and the poller serves them in a single wakeup.
        let before = crate::executor::inline_executor_counters();
        let poller = LanePoller::start(&Handle::current());
        let (release, gate) = tokio::sync::watch::channel(false);
        let served = Arc::new(AtomicUsize::new(0));
        let most_in_one_wakeup = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let mut gate = gate.clone();
            let served = Arc::clone(&served);
            let most = Arc::clone(&most_in_one_wakeup);
            handles.push(
                poller
                    .host(async move {
                        while !*gate.borrow_and_update() {
                            if gate.changed().await.is_err() {
                                return;
                            }
                        }
                        note_lane_datagram();
                        // Datagrams noted so far in this poller poll.
                        let so_far = LANE_DATAGRAMS.with(Cell::get) as usize;
                        most.fetch_max(so_far, Ordering::AcqRel);
                        served.fetch_add(1, Ordering::AcqRel);
                    })
                    .ok()
                    .expect("poller is running"),
            );
        }
        // Let every loop park on the gate.
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
        release.send_replace(true);
        for handle in handles {
            tokio::time::timeout(Duration::from_secs(2), handle.join())
                .await
                .expect("every loop finishes");
        }
        assert_eq!(served.load(Ordering::Acquire), 8);
        assert_eq!(
            most_in_one_wakeup.load(Ordering::Acquire),
            8,
            "all eight ready loops were served by one poller wakeup"
        );
        drop(poller);
        // Counters flush when the poller task ends (its channel closed and
        // no loop is hosted).
        tokio::time::sleep(Duration::from_millis(10)).await;
        let after = crate::executor::inline_executor_counters();
        assert!(after.lane_datagrams >= before.lane_datagrams + 8);
        assert!(after.lane_sockets >= before.lane_sockets + 8);
    }

    #[tokio::test]
    async fn hosted_sleeps_use_the_lane_clock_and_never_fire_early() {
        let _v2 = crate::executor::override_media_executor_config_on_this_thread(
            crate::executor::MediaExecutorConfig::v2(),
        );
        if !crate::executor::lane_timer_enabled() {
            return; // the process disabled the switch
        }
        let before = crate::executor::inline_executor_counters();
        let poller = LanePoller::start(&Handle::current());
        let start = tokio::time::Instant::now();
        let on_lane = Arc::new(AtomicUsize::new(0));
        let on_time = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for index in 0..10u64 {
            let on_lane = Arc::clone(&on_lane);
            let on_time = Arc::clone(&on_time);
            let deadline = start + Duration::from_millis(30) + Duration::from_micros(100 * index);
            handles.push(
                poller
                    .host(async move {
                        let sleep = crate::media_clock::sleep_until(deadline);
                        if sleep.is_lane() {
                            on_lane.fetch_add(1, Ordering::AcqRel);
                        }
                        sleep.await;
                        if tokio::time::Instant::now() >= deadline {
                            on_time.fetch_add(1, Ordering::AcqRel);
                        }
                    })
                    .ok()
                    .expect("poller is running"),
            );
        }
        for handle in handles {
            tokio::time::timeout(Duration::from_secs(2), handle.join())
                .await
                .expect("every hosted sleep completes");
        }
        assert_eq!(
            on_lane.load(Ordering::Acquire),
            10,
            "filed in the lane clock"
        );
        assert_eq!(on_time.load(Ordering::Acquire), 10, "none fired early");
        drop(poller);
        tokio::time::sleep(Duration::from_millis(10)).await;
        let after = crate::executor::inline_executor_counters();
        assert!(after.lane_timer_wakes >= before.lane_timer_wakes + 10);
        assert!(after.lane_timer_fires > before.lane_timer_fires);
        assert!(after.lane_timer_arms >= before.lane_timer_arms + 10);
    }

    #[tokio::test]
    async fn outside_a_poller_or_under_v1_sleeps_stay_on_the_runtime_timer() {
        let _v2 = crate::executor::override_media_executor_config_on_this_thread(
            crate::executor::MediaExecutorConfig::v2(),
        );
        let deadline = tokio::time::Instant::now() + Duration::from_millis(1);
        assert!(
            !crate::media_clock::sleep_until(deadline).is_lane(),
            "not inside a poll"
        );
        drop(_v2);
        let _v1 = crate::executor::override_media_executor_config_on_this_thread(
            crate::executor::MediaExecutorConfig::v1(),
        );
        let poller = LanePoller::start(&Handle::current());
        let (tx, rx) = oneshot::channel();
        let _handle = poller
            .host(async move {
                let _ = tx.send(crate::media_clock::sleep_until(deadline).is_lane());
            })
            .ok()
            .expect("poller is running");
        assert_eq!(rx.await, Ok(false));
    }

    #[test]
    fn the_budget_adapter_yields_once_the_wakeup_budget_is_spent() {
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        let polls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&polls);
        let mut hosted = Budgeted {
            inner: Box::pin(futures::future::poll_fn(move |_| {
                counted.fetch_add(1, Ordering::AcqRel);
                Poll::<()>::Pending
            })),
        };
        LANE_POLL_BUDGET.with(|budget| budget.set(Some(1)));
        assert!(Pin::new(&mut hosted).poll(&mut cx).is_pending());
        assert!(Pin::new(&mut hosted).poll(&mut cx).is_pending());
        assert_eq!(
            polls.load(Ordering::Acquire),
            1,
            "the second poll was refused"
        );
        LANE_POLL_BUDGET.with(|budget| budget.set(None));
        assert!(Pin::new(&mut hosted).poll(&mut cx).is_pending());
        assert_eq!(
            polls.load(Ordering::Acquire),
            2,
            "outside a poller poll it always runs"
        );
    }
}
