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
//!
//! Lane tick ([`crate::lane_tick`]): on a dedicated current-thread lane the
//! poller may run on a tick. After a pass it parks the lane thread until the
//! next tick boundary instead of returning to `epoll_wait`, then yields once
//! (Tokio polls the I/O and timer drivers with a zero timeout before
//! re-polling a yielded task) and serves everything that became ready during
//! the tick in one pass.
//!
//! Only the lane's own readiness and timers wait for the tick. Work handed
//! over from another thread does not: a hosted loop woken from another
//! thread (and a newly hosted loop) unparks the lane at once through the
//! poller's [`TickWake`], and a non-hosted task on the lane woken from
//! another thread (Tokio's injection queue) ends the sleep within
//! [`TICK_SLICE`]: the lane sleeps in slices and checks the queue between
//! them. A parked thread is not in `epoll_wait`, so Tokio's own cross-thread
//! unpark cannot reach it; without this, waits on lane tasks from other
//! threads (closing a stream, for one) would last up to a whole tick.
//!
//! Timing wheel and pass clock
//! ([`crate::executor::MediaExecutorConfig::lane_timer_wheel`],
//! [`crate::executor::MediaExecutorConfig::lane_pass_clock`]): the poller's
//! lane clock keeps its deadlines in a timing wheel, and each pass reads the
//! clock once and serves that reading to the futures it polls (see
//! `crate::lane_clock`). A later timer round of the same pass and the tick's
//! due check before it sleeps read the clock afresh.

use std::cell::Cell;
use std::collections::HashMap;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll, Waker};

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

#[cfg(test)]
thread_local! {
    /// Poller passes run on this thread (one lane wakeup each), so a test
    /// can count its own poller's wakeups exactly.
    static TEST_PASSES: Cell<u64> = const { Cell::new(0) };
    /// Pass-clock reads of the pollers on this thread: (served from a pass
    /// reading, fresh clock reads).
    static TEST_PASS_READS: Cell<(u64, u64)> = const { Cell::new((0, 0)) };
}

/// Lane tick: the longest single park of a ticking lane thread. Between
/// slices the lane checks for work woken from other threads on non-hosted
/// lane tasks (Tokio's injection queue), which cannot unpark it directly.
pub(crate) const TICK_SLICE: std::time::Duration = std::time::Duration::from_micros(500);

static NEXT_LANE_TOKEN: AtomicU64 = AtomicU64::new(1);

thread_local! {
    /// Lane tick: the token of the ticking poller running on this thread
    /// (0: none).
    static LANE_TOKEN: Cell<u64> = const { Cell::new(0) };
}

/// Lane tick: the parent waker a ticking poller hands its hosted loops (in
/// place of its task waker), and the handle that cuts its tick sleep short.
///
/// A wake on the lane's own thread (socket readiness dispatched by the
/// lane's I/O driver, a loop waking itself) only wakes the poller task, as
/// without the tick, and is served by the next tick. A wake from any other
/// thread (a cross-thread hand-off, a cancellation, an abort) also unparks
/// the lane thread, so it is served at once. Cross-thread wakes of hosted
/// loops are control-plane rate (the media path of a lane is lane-local), so
/// this does not bring back per-packet wakeups.
pub(crate) struct TickWake {
    task: futures::task::AtomicWaker,
    poked: AtomicBool,
    lane: u64,
    thread: OnceLock<std::thread::Thread>,
}

impl TickWake {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            task: futures::task::AtomicWaker::new(),
            poked: AtomicBool::new(false),
            lane: NEXT_LANE_TOKEN.fetch_add(1, AtomicOrdering::Relaxed),
            thread: OnceLock::new(),
        })
    }

    /// The poller runs on this thread from now on.
    fn bind_to_current_thread(&self) {
        let _ = self.thread.set(std::thread::current());
        LANE_TOKEN.with(|token| token.set(self.lane));
    }

    /// End a tick sleep now (or the next one, if the lane is not sleeping).
    pub(crate) fn poke(&self) {
        self.poked.store(true, AtomicOrdering::Release);
        if let Some(thread) = self.thread.get() {
            thread.unpark();
        }
    }

    fn take_poke(&self) -> bool {
        self.poked.swap(false, AtomicOrdering::AcqRel)
    }
}

impl std::task::Wake for TickWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.task.wake();
        if LANE_TOKEN.with(Cell::get) != self.lane {
            self.poke();
        }
    }
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
                // Pass clock: a pass reading serves at most this many loop
                // polls; the first read after them refreshes it (a no-op
                // without the pass clock).
                let polled = LANE_SOCKET_POLLS_PER_WAKEUP - left;
                if polled > 0 && polled % crate::lane_clock::PASS_CLOCK_REFRESH_POLLS == 0 {
                    crate::lane_clock::pass_invalidate();
                }
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
    /// Lane tick: published by a poller that ticks, so that a newly hosted
    /// loop or task cuts a tick sleep short (call setup and first audio do
    /// not wait for the quantum).
    tick_wake: Arc<OnceLock<Arc<TickWake>>>,
}

impl LanePoller {
    /// Start a poller task on `runtime`.
    pub(crate) fn start(runtime: &Handle) -> Self {
        let (jobs, receiver) = mpsc::unbounded_channel();
        let tick_wake = Arc::new(OnceLock::new());
        drop(runtime.spawn(run_poller(receiver, Arc::clone(&tick_wake))));
        Self { jobs, tick_wake }
    }

    /// Lane tick test seam: start a poller with an explicit tick scheduler
    /// and lane-clock grid, on any runtime.
    #[cfg(test)]
    pub(crate) fn start_with_tick(
        runtime: &Handle,
        tick: crate::lane_tick::TickScheduler,
        grid: Option<crate::lane_tick::TickGrid>,
    ) -> Self {
        let (jobs, receiver) = mpsc::unbounded_channel();
        let tick_wake = Arc::new(OnceLock::new());
        let published = Arc::clone(&tick_wake);
        drop(runtime.spawn(async move {
            let wake = TickWake::new();
            wake.bind_to_current_thread();
            let _ = published.set(Arc::clone(&wake));
            run_poller_with(receiver, Some((tick, wake)), PollerTimers::configured(grid)).await;
        }));
        Self { jobs, tick_wake }
    }

    /// Timing wheel / pass clock test seam: start a poller (without the
    /// tick) with explicit lane-clock settings.
    #[cfg(test)]
    pub(crate) fn start_with_timers(runtime: &Handle, timers: PollerTimers) -> Self {
        let (jobs, receiver) = mpsc::unbounded_channel();
        let tick_wake = Arc::new(OnceLock::new());
        drop(runtime.spawn(run_poller_with(receiver, None, timers)));
        Self { jobs, tick_wake }
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
            Ok(()) => {
                if let Some(wake) = self.tick_wake.get() {
                    // Lane tick: a ticking lane serves the new loop now, not
                    // at the next boundary. Harmless when it is not parked.
                    wake.poke();
                }
                Ok(LaneTaskHandle {
                    abort,
                    done: done_rx,
                })
            }
            Err(mpsc::error::SendError(hosted)) => Err(hosted),
        }
    }
}

/// Lane-clock expiries one poller poll serves before it yields. Each
/// round wakes every entry that is due, so more than one round only happens
/// when polling the woken futures took past the next deadline.
const LANE_TIMER_ROUNDS_PER_WAKEUP: u32 = 4;

/// What one poller pass asks the poller loop to do next.
enum PassStep {
    /// The job channel closed and no loop is hosted.
    Finished,
    /// Lane tick: run the next pass after one zero-timeout driver poll.
    Continue,
    /// Lane tick: park the lane thread until this tick boundary first.
    Sleep(std::time::Instant),
}

async fn run_poller(
    jobs: mpsc::UnboundedReceiver<LaneFuture>,
    tick_slot: Arc<OnceLock<Arc<TickWake>>>,
) {
    // Lane tick: `Some` only with the tick active on a dedicated
    // current-thread media lane; grid timers only for a poller that ticks.
    let tick = crate::lane_tick::scheduler_for_current_lane().map(|scheduler| {
        let wake = TickWake::new();
        wake.bind_to_current_thread();
        let _ = tick_slot.set(Arc::clone(&wake));
        (scheduler, wake)
    });
    let grid = tick.as_ref().and_then(|_| crate::lane_tick::timer_grid());
    run_poller_with(jobs, tick, PollerTimers::configured(grid)).await;
}

/// How a poller keeps its hosted futures' deadlines and clock reads (read
/// once, when the poller starts).
#[derive(Clone, Copy, Debug)]
pub(crate) struct PollerTimers {
    /// Hosted futures file their sleeps in the lane clock.
    pub(crate) lane_timer: bool,
    /// Lane tick grid timers: file deadlines on this grid.
    pub(crate) grid: Option<crate::lane_tick::TickGrid>,
    /// The lane clock is a timing wheel.
    pub(crate) wheel: bool,
    /// Each pass serves one clock reading to the futures it polls.
    pub(crate) pass_clock: bool,
}

impl PollerTimers {
    /// The configuration in effect, filing on `grid`.
    fn configured(grid: Option<crate::lane_tick::TickGrid>) -> Self {
        let lane_timer = crate::executor::lane_timer_enabled();
        Self {
            lane_timer,
            grid,
            wheel: lane_timer && crate::executor::lane_timer_wheel_enabled(),
            pass_clock: crate::executor::lane_pass_clock_enabled(),
        }
    }
}

/// Lane tick: whether a non-hosted task on this lane was woken from another
/// thread (it waits in Tokio's injection queue).
fn remote_work(metrics: Option<&tokio::runtime::RuntimeMetrics>) -> bool {
    metrics.is_some_and(|metrics| metrics.global_queue_depth() > 0)
}

async fn run_poller_with(
    mut jobs: mpsc::UnboundedReceiver<LaneFuture>,
    tick: Option<(crate::lane_tick::TickScheduler, Arc<TickWake>)>,
    timers: PollerTimers,
) {
    use crate::lane_tick::{PassReport, TickDecision};
    let (mut tick, tick_wake) = match tick {
        Some((scheduler, wake)) => (Some(scheduler), Some(wake)),
        None => (None, None),
    };
    // Lane tick: the waker hosted loops see while ticking, and the lane
    // runtime's injection-queue depth (both unused without the tick).
    let tick_waker: Option<Waker> = tick_wake.as_ref().map(|wake| Waker::from(Arc::clone(wake)));
    let lane_metrics = tick_wake
        .as_ref()
        .and_then(|_| Handle::try_current().ok())
        .map(|handle| handle.metrics());
    let ticking = tick.is_some();
    let mut loops: FuturesUnordered<Budgeted> = FuturesUnordered::new();
    let mut counters = LanePollerCounters::new();
    // Counter and PCM pool cleanup: on a current-thread runtime (the poller
    // never moves) the per-event counters the hosted futures add during a
    // pass (PCM pool misses, owner RTCP) are held on this thread and
    // published every 64 passes, on idle passes and when the poller stops.
    let mut deferred = crate::executor::LaneDeferredBatch::for_this_poller();
    // The lane clock. Futures polled below file their deadlines in it
    // (through `media_clock::sleep_until`) instead of the runtime's timer
    // wheel when the lane timer is enabled; the poller arms one runtime timer
    // at its earliest deadline (none while ticking: the tick serves it).
    // Without the switch the clock stays empty. With grid timers the clock
    // files deadlines on the tick grid, and with the timing wheel it keeps
    // them in a wheel. With the pass clock a pass reads the clock once for
    // the futures it polls.
    let PollerTimers {
        lane_timer,
        grid,
        wheel,
        pass_clock,
    } = timers;
    let mut clock = crate::lane_clock::LaneClockDriver::with_timers(grid, wheel);
    let mut timer_counters = crate::executor::LaneTimerCounters::new();
    let mut wheel_counters =
        (wheel || pass_clock).then(crate::executor::LaneWheelCounterBatch::new);
    let mut tick_counters = crate::lane_tick::LaneTickCounterBatch::new();
    let mut accepting = true;
    loop {
        let step = std::future::poll_fn(|cx| {
            #[cfg(test)]
            TEST_PASSES.with(|passes| passes.set(passes.get() + 1));
            // Lane tick: the pass's start.
            let started = ticking.then(std::time::Instant::now);
            if let Some(wake) = tick_wake.as_ref() {
                wake.task.register(cx.waker());
            }
            if let (Some(scheduler), Some(started)) = (tick.as_mut(), started) {
                if let Some(late) = scheduler.begin_pass(started) {
                    tick_counters.lateness(late);
                }
            }
            LANE_DATAGRAMS.with(|count| count.set(0));
            LANE_POLL_BUDGET.with(|budget| budget.set(Some(LANE_SOCKET_POLLS_PER_WAKEUP)));
            let _deferring = deferred
                .as_ref()
                .map(crate::executor::LaneDeferredBatch::enter_pass);
            let _current_clock = lane_timer.then(|| crate::lane_clock::enter(clock.clock()));
            // Pass clock: the pass's one clock reading (the tick's start when
            // ticking), served to everything this pass polls.
            let _pass_clock = pass_clock.then(|| {
                crate::lane_clock::enter_pass(
                    started.map_or_else(tokio::time::Instant::now, tokio::time::Instant::from_std),
                )
            });
            // The tick's due-check reading, the last of the pass.
            let mut last_reading: Option<tokio::time::Instant> = None;
            while accepting {
                match jobs.poll_recv(cx) {
                    Poll::Ready(Some(job)) => loops.push(Budgeted { inner: job }),
                    Poll::Ready(None) => accepting = false,
                    Poll::Pending => break,
                }
            }
            let mut rounds = 0;
            let mut timer_wakes = 0;
            let mut deadlines_left = false;
            loop {
                // Wake every lane-clock entry that is due, so its future is
                // polled in this same pass.
                let mut fired = false;
                if lane_timer {
                    // Pass clock: the first round is served from the pass
                    // reading; a later round exists because something fell
                    // due while the previous one ran, so it reads afresh.
                    let now = if !pass_clock {
                        tokio::time::Instant::now()
                    } else if rounds == 0 {
                        crate::lane_clock::pass_cached_now()
                            .unwrap_or_else(tokio::time::Instant::now)
                    } else {
                        crate::lane_clock::pass_refresh().unwrap_or_else(tokio::time::Instant::now)
                    };
                    let (wakes, arms) = clock.wake_due_at(now);
                    timer_wakes += wakes;
                    timer_counters.poll(false, wakes, arms);
                }
                // Each `Ready(Some(()))` is one loop that ended; keep serving
                // the others. `Pending` means every ready loop was polled (or
                // the poller must yield); `Ready(None)` means no loop is
                // hosted.
                match tick_waker.as_ref() {
                    // Lane tick: hosted loops wake the poller through
                    // `TickWake`, so a wake from another thread also cuts a
                    // tick sleep short.
                    Some(waker) => {
                        let mut children = Context::from_waker(waker);
                        while let Poll::Ready(Some(())) = loops.poll_next_unpin(&mut children) {}
                    }
                    None => while let Poll::Ready(Some(())) = loops.poll_next_unpin(cx) {},
                }
                if lane_timer {
                    if ticking {
                        // No runtime timer while ticking. A deadline that
                        // fell due during this pass gets another round now;
                        // later ones are served by a later tick. Always a
                        // fresh reading: the decision to sleep depends on it.
                        let now = if pass_clock {
                            crate::lane_clock::pass_refresh()
                                .unwrap_or_else(tokio::time::Instant::now)
                        } else {
                            tokio::time::Instant::now()
                        };
                        last_reading = Some(now);
                        fired = clock.has_due(now);
                    } else if clock.poll_timer(cx).is_ready() {
                        fired = true;
                    }
                }
                if !fired {
                    break;
                }
                rounds += 1;
                if rounds >= LANE_TIMER_ROUNDS_PER_WAKEUP {
                    // Serve the rest on the next poll.
                    if ticking {
                        deadlines_left = true;
                    } else {
                        cx.waker().wake_by_ref();
                    }
                    break;
                }
            }
            // Runtime timer expiries this pass, each counted once by the
            // driver (also those whose entries `wake_due` served first).
            for _ in 0..clock.take_expiries() {
                timer_counters.poll(true, 0, 0);
            }
            let left = LANE_POLL_BUDGET.with(|budget| budget.replace(None));
            let exhausted = left == Some(0);
            let datagrams = LANE_DATAGRAMS.with(|count| count.replace(0));
            counters.wakeup(datagrams, exhausted);
            if let Some(batch) = deferred.as_mut() {
                // Idle: no hosted future was polled, or none is hosted.
                batch.pass_end(loops.is_empty() || left == Some(LANE_SOCKET_POLLS_PER_WAKEUP));
            }
            tick_counters.timers(clock.take_resets(), clock.clock().take_coalesced());
            if let Some(batch) = wheel_counters.as_mut() {
                let reads = crate::lane_clock::take_pass_clock_stats();
                #[cfg(test)]
                TEST_PASS_READS.with(|total| {
                    let (cached, fresh) = total.get();
                    total.set((cached + reads.cached, fresh + reads.fresh));
                });
                batch.pass(clock.clock().take_wheel_ops(), reads);
            }
            if !accepting && loops.is_empty() {
                return Poll::Ready(PassStep::Finished);
            }
            let (Some(scheduler), Some(started)) = (tick.as_mut(), started) else {
                return Poll::Pending;
            };
            let polled =
                LANE_SOCKET_POLLS_PER_WAKEUP - left.unwrap_or(LANE_SOCKET_POLLS_PER_WAKEUP);
            tick_counters.pass(polled > 0 || datagrams > 0 || timer_wakes > 0);
            // With the pass clock the tick's due-check reading, taken after
            // the last poll of the pass, also ends it.
            let ended = match (pass_clock, last_reading) {
                (true, Some(reading)) => reading.into_std(),
                _ => std::time::Instant::now(),
            };
            match scheduler.end_pass(PassReport {
                started,
                ended,
                hosting: !loops.is_empty(),
                must_continue: exhausted || deadlines_left,
            }) {
                TickDecision::Park => {
                    // Nothing hosted: park as without the tick. A lane clock
                    // that still holds a deadline needs its runtime timer.
                    if lane_timer && clock.poll_timer(cx).is_ready() {
                        cx.waker().wake_by_ref();
                    }
                    Poll::Pending
                }
                TickDecision::Continue => {
                    tick_counters.overrun();
                    Poll::Ready(PassStep::Continue)
                }
                TickDecision::SleepUntil(at) => Poll::Ready(PassStep::Sleep(at)),
            }
        })
        .await;
        match step {
            PassStep::Finished => break,
            PassStep::Continue => tokio::task::yield_now().await,
            PassStep::Sleep(until) => {
                // The lane's own readiness events queue in the kernel until
                // the boundary. Work from other threads ends the sleep: a
                // hosted loop woken elsewhere (or a new one) pokes and
                // unparks the thread; a non-hosted lane task woken elsewhere
                // sits in Tokio's injection queue, checked between slices.
                let wake = tick_wake.as_ref().expect("only a ticking poller sleeps");
                let mut slices = 0u64;
                let mut cut = false;
                loop {
                    if wake.take_poke() || remote_work(lane_metrics.as_ref()) {
                        cut = true;
                        break;
                    }
                    let Some(rest) = until
                        .checked_duration_since(std::time::Instant::now())
                        .filter(|rest| !rest.is_zero())
                    else {
                        break;
                    };
                    std::thread::park_timeout(rest.min(TICK_SLICE));
                    slices += 1;
                }
                tick_counters.slept(slices, cut);
                // Tokio polls its drivers with a zero timeout before it polls
                // a yielded task again: one `epoll_wait` for the whole tick.
                tokio::task::yield_now().await;
            }
        }
    }
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
        let poller = LanePoller {
            jobs,
            tick_wake: Arc::default(),
        };
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

    // ---- Timing wheel and pass clock ----

    fn wheel_and_pass_clock(pass_clock: bool) -> PollerTimers {
        PollerTimers {
            lane_timer: true,
            grid: None,
            wheel: true,
            pass_clock,
        }
    }

    #[tokio::test]
    async fn a_wheel_and_pass_clock_poller_serves_hosted_sleeps_never_early() {
        let _v2 = crate::executor::override_media_executor_config_on_this_thread(
            crate::executor::MediaExecutorConfig::v2(),
        );
        if !crate::executor::lane_timer_enabled() {
            return; // the process disabled the switch
        }
        let before = crate::executor::lane_wheel_counters();
        let timers_before = crate::executor::inline_executor_counters();
        let poller = LanePoller::start_with_timers(&Handle::current(), wheel_and_pass_clock(true));
        let start = tokio::time::Instant::now();
        let on_lane = Arc::new(AtomicUsize::new(0));
        let early = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for index in 0..10u64 {
            let on_lane = Arc::clone(&on_lane);
            let early = Arc::clone(&early);
            handles.push(
                poller
                    .host(async move {
                        for frame in 1..=5u64 {
                            let deadline = start
                                + Duration::from_millis(7 * frame)
                                + Duration::from_micros(150 * index);
                            let sleep = crate::media_clock::sleep_until(deadline);
                            if sleep.is_lane() {
                                on_lane.fetch_add(1, Ordering::AcqRel);
                            }
                            sleep.await;
                            // The pass reading that completed the sleep, and
                            // the true clock, are both at or after it.
                            if crate::media_clock::now() < deadline
                                || tokio::time::Instant::now() < deadline
                            {
                                early.fetch_add(1, Ordering::AcqRel);
                            }
                        }
                    })
                    .ok()
                    .expect("poller is running"),
            );
        }
        for handle in handles {
            tokio::time::timeout(Duration::from_secs(5), handle.join())
                .await
                .expect("every hosted sleep completes");
        }
        assert_eq!(
            on_lane.load(Ordering::Acquire),
            50,
            "filed in the lane clock"
        );
        assert_eq!(early.load(Ordering::Acquire), 0, "none completed early");
        drop(poller);
        tokio::time::sleep(Duration::from_millis(20)).await;
        let after = crate::executor::lane_wheel_counters();
        assert!(
            after.wheel_inserts >= before.wheel_inserts + 50,
            "every deadline linked in the wheel"
        );
        assert!(
            after.pass_clock_cached_reads > before.pass_clock_cached_reads,
            "reads served by the pass"
        );
        assert!(after.pass_clock_fresh_reads > before.pass_clock_fresh_reads);
        // The lane-clock counters keep their meaning with the wheel.
        let timers_after = crate::executor::inline_executor_counters();
        assert!(timers_after.lane_timer_wakes >= timers_before.lane_timer_wakes + 50);
        assert!(timers_after.lane_timer_arms >= timers_before.lane_timer_arms + 50);
        assert!(timers_after.lane_timer_fires > timers_before.lane_timer_fires);
    }

    /// Host ten loops that read the media clock three times per poll and
    /// then yield (waking themselves), until stopped. Returns, over a window
    /// of whole passes in the middle of the run: (poller passes, loop polls,
    /// reads served from a pass reading, fresh clock reads).
    async fn self_waking_readers(pass_clock: bool) -> (u64, u64, u64, u64) {
        let poller =
            LanePoller::start_with_timers(&Handle::current(), wheel_and_pass_clock(pass_clock));
        let stop = Arc::new(AtomicBool::new(false));
        let polls = Arc::new(AtomicU64::new(0));
        let backwards = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..10 {
            let stop = Arc::clone(&stop);
            let polls = Arc::clone(&polls);
            let backwards = Arc::clone(&backwards);
            handles.push(
                poller
                    .host(futures::future::poll_fn(move |cx| {
                        if stop.load(AtomicOrdering::Acquire) {
                            return Poll::Ready(());
                        }
                        let first = crate::media_clock::now();
                        let second = crate::media_clock::now();
                        let third = crate::media_clock::now_std();
                        if second < first || third < second.into_std() {
                            backwards.fetch_add(1, Ordering::AcqRel);
                        }
                        polls.fetch_add(1, AtomicOrdering::AcqRel);
                        cx.waker().wake_by_ref();
                        Poll::Pending
                    }))
                    .ok()
                    .expect("poller is running"),
            );
        }
        // Warm up, then measure a window of whole passes: this test's task
        // runs only between poller passes (one thread).
        tokio::time::sleep(Duration::from_millis(5)).await;
        let passes_before = TEST_PASSES.with(Cell::get);
        let reads_before = TEST_PASS_READS.with(Cell::get);
        let polls_before = polls.load(AtomicOrdering::Acquire);
        tokio::time::sleep(Duration::from_millis(20)).await;
        let passes = TEST_PASSES.with(Cell::get) - passes_before;
        let reads = TEST_PASS_READS.with(Cell::get);
        let loop_polls = polls.load(AtomicOrdering::Acquire) - polls_before;
        stop.store(true, AtomicOrdering::Release);
        for handle in handles {
            tokio::time::timeout(Duration::from_secs(5), handle.join())
                .await
                .expect("every loop stops");
        }
        drop(poller);
        assert_eq!(
            backwards.load(Ordering::Acquire),
            0,
            "media time never runs backwards"
        );
        (
            passes,
            loop_polls,
            reads.0 - reads_before.0,
            reads.1 - reads_before.1,
        )
    }

    #[tokio::test]
    async fn a_pass_clock_poller_reads_the_clock_once_per_pass() {
        let (passes, polls, cached, fresh) = self_waking_readers(true).await;
        assert!(
            passes > 10 && polls > passes,
            "passes {passes}, polls {polls}"
        );
        assert_eq!(fresh, passes, "one clock read per pass");
        // Every media-clock read of every loop poll, plus the lane clock's
        // first timer round, is served from that one reading.
        assert_eq!(cached, 3 * polls + passes, "polls {polls}, passes {passes}");

        // Without the pass clock nothing is served from a pass reading (each
        // read is a clock read, as before).
        let (passes, polls, cached, fresh) = self_waking_readers(false).await;
        assert!(
            passes > 10 && polls > passes,
            "passes {passes}, polls {polls}"
        );
        assert_eq!((cached, fresh), (0, 0));
    }

    #[test]
    fn a_pass_reading_serves_a_bounded_number_of_hosted_polls() {
        // Staleness bound in work: within a pass, the 17th and 33rd loop
        // polls read the clock afresh; the others read the pass reading.
        let seed = tokio::time::Instant::now() - Duration::from_millis(1);
        let _ = crate::lane_clock::take_pass_clock_stats();
        let readings = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let seen = Arc::clone(&readings);
        let mut hosted = Budgeted {
            inner: Box::pin(futures::future::poll_fn(move |_| {
                seen.lock().push(crate::media_clock::now());
                Poll::<()>::Pending
            })),
        };
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        LANE_POLL_BUDGET.with(|budget| budget.set(Some(LANE_SOCKET_POLLS_PER_WAKEUP)));
        {
            let _pass = crate::lane_clock::enter_pass(seed);
            for _ in 0..40 {
                assert!(Pin::new(&mut hosted).poll(&mut cx).is_pending());
            }
        }
        LANE_POLL_BUDGET.with(|budget| budget.set(None));
        let readings = readings.lock().clone();
        assert_eq!(readings.len(), 40);
        assert!(
            readings[..16].iter().all(|reading| *reading == seed),
            "16 polls on the pass reading"
        );
        assert!(readings[16] > seed, "the 17th poll reads the clock");
        assert!(readings[16..32]
            .iter()
            .all(|reading| *reading == readings[16]));
        assert!(readings[32] >= readings[16], "the 33rd again, monotonic");
        assert!(readings[33..]
            .iter()
            .all(|reading| *reading == readings[32]));
        let stats = crate::lane_clock::take_pass_clock_stats();
        assert_eq!((stats.cached, stats.fresh), (38, 3), "{stats:?}");
        // Outside a pass the clock is read.
        assert!(crate::media_clock::now() >= readings[32]);
        assert_eq!(crate::lane_clock::take_pass_clock_stats().cached, 0);
    }

    #[tokio::test]
    async fn paced_streams_on_a_ticking_lane_are_served_alike_with_the_heap_and_the_wheel() {
        // The v2 profile (2 ms tick, grid timers) with the heap and no pass
        // clock, then with the wheel and the pass clock: 40 paced streams of
        // ten 20 ms deadlines each are served never early and within the
        // same bound, in a comparable number of passes.
        let quantum = Duration::from_millis(2);
        let grid = crate::lane_tick::TickGrid::process(quantum);
        let mut outcomes = Vec::new();
        for wheel in [false, true] {
            let _v2 = crate::executor::override_media_executor_config_on_this_thread(
                crate::executor::MediaExecutorConfig::v2()
                    .with_lane_timer_wheel(wheel)
                    .with_lane_pass_clock(wheel),
            );
            if !crate::executor::lane_timer_enabled() {
                return; // the process disabled the switch
            }
            let before = crate::executor::lane_wheel_counters();
            let outcome = serve_paced_streams(
                LanePoller::start_with_tick(&Handle::current(), ticking(quantum), Some(grid)),
                40,
            )
            .await;
            tokio::time::sleep(Duration::from_millis(5)).await;
            let after = crate::executor::lane_wheel_counters();
            if wheel {
                assert!(
                    after.wheel_inserts >= before.wheel_inserts + 400,
                    "every deadline filed in the wheel"
                );
                assert!(after.pass_clock_fresh_reads > before.pass_clock_fresh_reads);
            }
            outcomes.push(outcome);
        }
        let (heap_passes, heap_early, heap_worst) = outcomes[0];
        let (wheel_passes, wheel_early, wheel_worst) = outcomes[1];
        assert_eq!(heap_early + wheel_early, 0, "never early");
        for worst in [heap_worst, wheel_worst] {
            // One quantum plus generous scheduling noise for a shared host.
            assert!(
                worst < quantum + Duration::from_millis(25),
                "worst lateness {worst:?}"
            );
        }
        assert!(
            wheel_passes <= 2 * heap_passes + 20 && heap_passes <= 2 * wheel_passes + 20,
            "passes: heap {heap_passes}, wheel {wheel_passes}"
        );
    }

    // ---- Lane tick: a ticking poller ----

    fn ticking(quantum: Duration) -> crate::lane_tick::TickScheduler {
        crate::lane_tick::TickScheduler::new(crate::lane_tick::TickGrid::process(quantum))
    }

    /// Host `streams` paced loops on `poller`, each waiting for ten 20 ms
    /// deadlines (`media_clock::sleep_until`, so the lane clock under v2) at
    /// phases 500 µs apart, and wait for them. Returns (poller passes run on
    /// this thread meanwhile, deadlines observed early, worst lateness).
    async fn serve_paced_streams(poller: LanePoller, streams: u64) -> (u64, usize, Duration) {
        let passes_before = TEST_PASSES.with(Cell::get);
        let start = tokio::time::Instant::now() + Duration::from_millis(10);
        let early = Arc::new(AtomicUsize::new(0));
        let worst_us = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for index in 0..streams {
            let early = Arc::clone(&early);
            let worst_us = Arc::clone(&worst_us);
            handles.push(
                poller
                    .host(async move {
                        for frame in 0..10u32 {
                            let deadline = start
                                + Duration::from_millis(20) * frame
                                + Duration::from_micros(500 * index);
                            crate::media_clock::sleep_until(deadline).await;
                            let now = tokio::time::Instant::now();
                            if now < deadline {
                                early.fetch_add(1, Ordering::AcqRel);
                            }
                            worst_us
                                .fetch_max((now - deadline).as_micros() as usize, Ordering::AcqRel);
                        }
                    })
                    .ok()
                    .expect("poller is running"),
            );
        }
        for handle in handles {
            tokio::time::timeout(Duration::from_secs(5), handle.join())
                .await
                .expect("every paced loop finishes");
        }
        drop(poller);
        let passes = TEST_PASSES.with(Cell::get) - passes_before;
        let worst = Duration::from_micros(worst_us.load(Ordering::Acquire) as u64);
        (passes, early.load(Ordering::Acquire), worst)
    }

    #[tokio::test]
    async fn a_ticking_poller_with_grid_timers_serves_paced_streams_in_far_fewer_wakeups() {
        let _v2 = crate::executor::override_media_executor_config_on_this_thread(
            crate::executor::MediaExecutorConfig::v2(),
        );
        if !crate::executor::lane_timer_enabled() {
            return; // the process disabled the switch
        }
        let quantum = Duration::from_millis(5);
        let tick_counters_before = crate::lane_tick::lane_tick_counters();
        // This test thread is not a media lane, so `start` keeps immediate
        // wakeups: one lane-clock expiry (and pass) per distinct deadline.
        let (exact_passes, exact_early, _) =
            serve_paced_streams(LanePoller::start(&Handle::current()), 40).await;
        let grid = crate::lane_tick::TickGrid::process(quantum);
        let (ticked_passes, ticked_early, worst) = serve_paced_streams(
            LanePoller::start_with_tick(&Handle::current(), ticking(quantum), Some(grid)),
            40,
        )
        .await;
        assert_eq!(exact_early + ticked_early, 0, "never early");
        // 400 deadlines over ~210 ms. Exact: an expiry per distinct
        // millisecond (~200). Ticked: one pass per 5 ms boundary (~43), plus
        // a few for hosting and the end.
        assert!(
            ticked_passes <= 80,
            "ticked passes {ticked_passes} (exact {exact_passes})"
        );
        assert!(
            exact_passes >= 2 * ticked_passes,
            "the tick coalesces wakeups: exact {exact_passes} vs ticked {ticked_passes}"
        );
        // One quantum plus generous scheduling noise for a shared test host.
        assert!(
            worst < quantum + Duration::from_millis(25),
            "worst lateness {worst:?}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
        let after = crate::lane_tick::lane_tick_counters();
        assert!(after.tick_passes > tick_counters_before.tick_passes);
        assert!(after.tick_sleeps > tick_counters_before.tick_sleeps);
        assert!(
            after.unsupported_lanes > tick_counters_before.unsupported_lanes,
            "the non-lane poller was counted as unable to tick"
        );
    }

    /// Real time with loose bounds (the exact bounds are proven against a
    /// fake clock in `lane_tick`): hosted runtime timers complete never
    /// early, and join after they return.
    #[tokio::test]
    async fn a_ticking_poller_serves_hosted_timers_on_ticks_and_never_early() {
        let quantum = Duration::from_millis(2);
        let before = crate::lane_tick::lane_tick_counters();
        let poller = LanePoller::start_with_tick(&Handle::current(), ticking(quantum), None);
        let start = tokio::time::Instant::now();
        let early = Arc::new(AtomicUsize::new(0));
        let worst_us = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for index in 0..20u64 {
            let early = Arc::clone(&early);
            let worst_us = Arc::clone(&worst_us);
            handles.push(
                poller
                    .host(async move {
                        for frame in 1..=10u64 {
                            let deadline = start
                                + Duration::from_millis(7 * frame)
                                + Duration::from_micros(300 * index);
                            tokio::time::sleep_until(deadline).await;
                            let now = tokio::time::Instant::now();
                            if now < deadline {
                                early.fetch_add(1, Ordering::AcqRel);
                            }
                            worst_us
                                .fetch_max((now - deadline).as_micros() as usize, Ordering::AcqRel);
                        }
                    })
                    .ok()
                    .expect("poller is running"),
            );
        }
        for handle in handles {
            tokio::time::timeout(Duration::from_secs(5), handle.join())
                .await
                .expect("every hosted loop finishes on a ticking lane");
        }
        assert_eq!(early.load(Ordering::Acquire), 0, "never early");
        let worst = Duration::from_micros(worst_us.load(Ordering::Acquire) as u64);
        assert!(
            worst < quantum + Duration::from_millis(25),
            "worst lateness {worst:?}"
        );
        drop(poller);
        tokio::time::sleep(Duration::from_millis(20)).await;
        let after = crate::lane_tick::lane_tick_counters();
        assert!(
            after.tick_passes > before.tick_passes,
            "passes ran as ticks"
        );
        assert!(
            after.tick_sleeps > before.tick_sleeps,
            "the lane thread slept between ticks"
        );
    }

    #[tokio::test]
    async fn abort_and_join_work_on_a_ticking_lane() {
        let poller = LanePoller::start_with_tick(
            &Handle::current(),
            ticking(Duration::from_millis(5)),
            None,
        );
        let owned = Arc::new(());
        let held = Arc::clone(&owned);
        let handle = poller
            .host(async move {
                let _held = held;
                std::future::pending::<()>().await;
            })
            .ok()
            .expect("poller is running");
        tokio::time::sleep(Duration::from_millis(12)).await;
        assert_eq!(Arc::strong_count(&owned), 2, "the loop is alive");
        handle.abort();
        tokio::time::timeout(Duration::from_secs(2), handle.join())
            .await
            .expect("join resolves after abort on a ticking lane");
        assert_eq!(Arc::strong_count(&owned), 1);
    }

    #[test]
    fn hosting_a_loop_cuts_a_tick_sleep_short() {
        // A 50 ms quantum makes the effect unambiguous: a loop hosted 10 ms
        // into a tick would otherwise wait for the 50 ms boundary.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let poller = Arc::new(runtime.block_on(async {
            LanePoller::start_with_tick(
                &Handle::current(),
                ticking(Duration::from_millis(50)),
                None,
            )
        }));
        let (first_poll_tx, first_poll_rx) = std::sync::mpsc::channel::<std::time::Instant>();
        let remote = Arc::clone(&poller);
        let host = std::thread::spawn(move || {
            // Keep the lane ticking, then host from another thread mid-tick.
            let keep = remote
                .host(std::future::pending::<()>())
                .ok()
                .expect("poller is running");
            std::thread::sleep(Duration::from_millis(60));
            let hosted_at = std::time::Instant::now();
            let _probe = remote
                .host(async move {
                    let _ = first_poll_tx.send(std::time::Instant::now());
                })
                .ok()
                .expect("poller is running");
            std::thread::sleep(Duration::from_millis(200));
            (hosted_at, keep)
        });
        // Drive the lane thread (it ticks inside this block_on).
        let polled_at = runtime.block_on(async {
            loop {
                if let Ok(at) = first_poll_rx.try_recv() {
                    break at;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        });
        let (hosted_at, keep) = host.join().unwrap();
        keep.abort();
        let waited = polled_at.saturating_duration_since(hosted_at);
        assert!(
            waited < Duration::from_millis(25),
            "the new loop waited {waited:?}"
        );
        drop(poller);
    }

    /// The outcome of [`cross_thread_handoff`].
    struct Handoff {
        latencies: Vec<Duration>,
        before: crate::lane_tick::LaneTickCounters,
        after: crate::lane_tick::LaneTickCounters,
    }

    /// A producer on another thread hands five items, 37 ms apart, to a
    /// consumer on a ticking lane with a 50 ms quantum. `hosted`: the
    /// consumer is a loop hosted on the poller (woken through `TickWake`);
    /// otherwise it is an ordinary task on the lane runtime (woken through
    /// Tokio's injection queue).
    fn cross_thread_handoff(hosted: bool) -> Handoff {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let poller = runtime.block_on(async {
            LanePoller::start_with_tick(
                &Handle::current(),
                ticking(Duration::from_millis(50)),
                None,
            )
        });
        // A hosted loop that never ends keeps the lane ticking.
        let keep = poller
            .host(std::future::pending::<()>())
            .ok()
            .expect("poller is running");
        let (items_tx, mut items_rx) = mpsc::unbounded_channel::<std::time::Instant>();
        let (served_tx, mut served_rx) = mpsc::unbounded_channel::<Duration>();
        let consumer = async move {
            while let Some(sent) = items_rx.recv().await {
                let _ = served_tx.send(sent.elapsed());
            }
        };
        let _consumer = if hosted {
            Some(poller.host(consumer).ok().expect("poller is running"))
        } else {
            drop(runtime.spawn(consumer));
            None
        };
        let before = crate::lane_tick::lane_tick_counters();
        let producer = std::thread::spawn(move || {
            for _ in 0..5 {
                std::thread::sleep(Duration::from_millis(37));
                let _ = items_tx.send(std::time::Instant::now());
            }
        });
        let latencies = runtime.block_on(async {
            let mut latencies = Vec::new();
            while latencies.len() < 5 {
                let latency = tokio::time::timeout(Duration::from_secs(5), served_rx.recv())
                    .await
                    .expect("every hand-off is served")
                    .expect("consumer alive");
                latencies.push(latency);
            }
            latencies
        });
        producer.join().unwrap();
        keep.abort();
        drop(poller);
        // Dropping the runtime drops the poller, which flushes its counters.
        drop(runtime);
        let after = crate::lane_tick::lane_tick_counters();
        Handoff {
            latencies,
            before,
            after,
        }
    }

    fn assert_served_promptly_without_spinning(hosted: bool) {
        let Handoff {
            latencies,
            before,
            after,
        } = cross_thread_handoff(hosted);
        for latency in &latencies {
            // Within one tick is the requirement; the lane does far better:
            // it is unparked (hosted) or sees the injection queue within a
            // 500 µs slice. Generous bounds for a shared test host.
            assert!(
                *latency < Duration::from_millis(20),
                "hosted={hosted}: a hand-off waited {latency:?} on a 50 ms tick"
            );
        }
        // Nearly every hand-off lands while the lane sleeps and cuts that
        // sleep short (one landing mid-pass is served by that pass instead).
        assert!(
            after.tick_sleeps_cut >= before.tick_sleeps_cut + 3,
            "hosted={hosted}: hand-offs cut tick sleeps short"
        );
        // No busy-wait: ~190 ms of a 50 ms tick is a few dozen passes and a
        // few hundred 500 µs slices (other tests may add some); spinning
        // would be hundreds of thousands.
        let passes = after.tick_passes - before.tick_passes;
        let slices = after.tick_slices - before.tick_slices;
        assert!(passes < 2_000, "passes: {passes}");
        assert!(slices < 5_000, "slices: {slices}");
    }

    #[test]
    fn a_hosted_loop_woken_from_another_thread_is_served_without_waiting_for_the_tick() {
        assert_served_promptly_without_spinning(true);
    }

    #[test]
    fn a_lane_task_woken_from_another_thread_is_served_without_waiting_for_the_tick() {
        assert_served_promptly_without_spinning(false);
    }

    #[test]
    fn a_wake_on_the_lane_thread_does_not_unpark_it_but_a_foreign_one_does() {
        let wake = TickWake::new();
        wake.bind_to_current_thread();
        let waker = Waker::from(Arc::clone(&wake));
        waker.wake_by_ref();
        assert!(
            !wake.take_poke(),
            "a wake on the lane's own thread waits for the tick"
        );
        let foreign = waker.clone();
        std::thread::spawn(move || foreign.wake_by_ref())
            .join()
            .unwrap();
        assert!(
            wake.take_poke(),
            "a wake from another thread cuts the sleep short"
        );
        assert!(!wake.take_poke());
        LANE_TOKEN.with(|token| token.set(0));
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

    /// Counter and PCM pool cleanup: on a current-thread runtime a poller
    /// with the switch holds the per-event counts its hosted futures add in
    /// a pass and publishes them in batches (every value once); without the
    /// switch each is added at once.
    #[tokio::test]
    async fn a_poller_with_counter_pool_cleanup_batches_the_counts_of_its_passes() {
        static ON: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        static OFF: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        const LOOPS: usize = 8;
        const EVENTS_PER_LOOP: u64 = 10;
        for (config, total) in [
            (crate::executor::MediaExecutorConfig::v2(), &ON),
            (
                crate::executor::MediaExecutorConfig::v2().with_counter_pool_cleanup(false),
                &OFF,
            ),
        ] {
            let cleanup = config.counter_pool_cleanup_active();
            let _config = crate::executor::override_media_executor_config_on_this_thread(config);
            let poller = LanePoller::start(&Handle::current());
            let (release, gate) = tokio::sync::watch::channel(false);
            let most_held = Arc::new(AtomicUsize::new(0));
            let mut handles = Vec::new();
            for _ in 0..LOOPS {
                let mut gate = gate.clone();
                let held = most_held.clone();
                handles.push(
                    poller
                        .host(async move {
                            let _ = gate.wait_for(|open| *open).await;
                            for _ in 0..EVENTS_PER_LOOP {
                                crate::executor::add_lane_deferred(total, 1);
                                let pending =
                                    crate::executor::pending_lane_deferred_on_this_thread(total);
                                held.fetch_max(pending as usize, Ordering::AcqRel);
                                tokio::task::yield_now().await;
                            }
                        })
                        .ok()
                        .expect("poller is running"),
                );
            }
            // Let every loop park on the gate, then release them together:
            // one pass serves all of them.
            for _ in 0..4 {
                tokio::task::yield_now().await;
            }
            release.send_replace(true);
            for handle in handles {
                tokio::time::timeout(Duration::from_secs(2), handle.join())
                    .await
                    .expect("every loop finishes");
            }
            drop(poller);
            // The poller stops once its channel closed and it hosts nothing.
            tokio::time::sleep(Duration::from_millis(10)).await;
            assert_eq!(
                total.load(Ordering::Relaxed),
                LOOPS as u64 * EVENTS_PER_LOOP,
                "every event counted once (cleanup {cleanup})"
            );
            assert_eq!(
                crate::executor::pending_lane_deferred_on_this_thread(total),
                0
            );
            let most = most_held.load(Ordering::Acquire);
            if cleanup {
                assert!(
                    most >= LOOPS,
                    "the pass's counts were held for one publication ({most})"
                );
            } else {
                assert_eq!(most, 0, "without the switch every count is added at once");
            }
        }
    }
}
