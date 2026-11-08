//! Lane tick: wakeup and timer coalescing for lane pollers (executor v2).
//!
//! With the lane poller a media lane's poller (`crate::lane_poller`) still
//! wakes for every readiness event: on a dedicated current-thread lane the
//! thread sits in `epoll_wait` and returns for nearly every datagram when
//! arrivals are smooth. Wakeup frequency, not packet count, then drives a
//! large share of the media CPU.
//!
//! With [`MediaExecutorConfig::lane_tick`](crate::executor::MediaExecutorConfig::lane_tick)
//! set to a quantum `Q`, a lane poller on a dedicated current-thread media
//! lane runs on a **tick**: after a pass it parks the lane thread until the
//! next boundary of a `Q` grid (a divisor of the 20 ms media clock) instead
//! of parking in `epoll_wait`, then lets the runtime poll the I/O and timer
//! drivers once with a zero timeout and serves, in one pass, every socket
//! that became ready and every lane-clock deadline that fell due during the
//! tick. The thread sleep is the "epoll timeout at the next due deadline",
//! at tick granularity.
//!
//! Guarantees, all independent of load:
//! - **Never early.** Deadlines are still checked against their exact
//!   instant (lane sleeps, the owner timer, the pacers); a tick only delays.
//! - **At most one quantum added.** A pass is followed by a sleep to the
//!   next boundary, which is less than `Q` away, so an event waits at most
//!   `Q` (plus the thread's wake-up latency) longer than it would have while
//!   the lane was busy anyway. Periodic 20 ms deadlines keep a constant
//!   phase against the grid, so the added *jitter* of a stream is ~0.
//! - **No throughput loss under overload.** A pass that took a whole
//!   quantum or more, a pass that ran out of its poll budget and a pass that
//!   left lane-clock deadlines due skip the sleep: the lane degrades to
//!   continuous processing, as without the tick (the drain budget and the
//!   lane poll budget are unchanged). A lane that hosts nothing parks as
//!   without the tick.
//! - **Cross-thread work is not delayed.** Hosting a new receive loop or
//!   task, or waking a hosted loop from another thread, unparks a sleeping
//!   lane thread at once; a non-hosted lane task woken from another thread
//!   ends the sleep within 500 µs (the lane parks in slices and checks the
//!   runtime's injection queue between them). Only the lane's own socket
//!   readiness and timers wait for the boundary.
//!
//! With [`LaneTick::grid_timers`] (which also needs the lane clock,
//! [`MediaExecutorConfig::lane_timer`](crate::executor::MediaExecutorConfig::lane_timer))
//! lane-clock deadlines are filed on the tick grid: a deadline that moves
//! inside its tick files nothing new, and the deadlines of every call due in
//! one tick are served by one pass.
//!
//! Paced queues that schedule the next frame from the time they were served
//! would drift by up to a quantum per frame on a tick; with the tick
//! configured, the media layer's decoded pacing queue schedules from the
//! served deadline instead ([`lane_tick_pacing_enabled`]).
//!
//! The tick is used only on a [`MediaLanes::Dedicated`] lane (a dedicated
//! current-thread runtime). On any other lane the tick, grid timers
//! included, reads as off for that lane's poller, which keeps immediate
//! wakeups; such pollers are counted in [`LaneTickCounters::unsupported_lanes`].
//!
//! [`MediaLanes::Dedicated`]: crate::task_runtime::MediaLanes::Dedicated

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// The tick quanta [`LaneTick::quantum`] accepts, in milliseconds: divisors
/// of the 20 ms media clock that the runtime's 1 ms timer resolution can
/// represent.
pub const LANE_TICK_QUANTA_MS: [u64; 4] = [1, 2, 4, 5];

/// The media clock period a tick quantum must divide.
pub const MEDIA_CLOCK_PERIOD: Duration = Duration::from_millis(20);

/// Lane tick settings, the value of
/// [`MediaExecutorConfig::lane_tick`](crate::executor::MediaExecutorConfig::lane_tick).
///
/// ```
/// use std::time::Duration;
/// use rvoip_rtp_core::executor::{LaneTick, MediaExecutorConfig};
///
/// let tick = LaneTick::new(Duration::from_millis(2)).with_grid_timers(true);
/// let config = MediaExecutorConfig::v2().with_lane_tick(Some(tick));
/// config.validate().unwrap();
///
/// // A quantum must divide the 20 ms media clock.
/// let bad = MediaExecutorConfig::v2().with_lane_tick(Some(LaneTick::new(Duration::from_millis(3))));
/// assert!(bad.validate().is_err());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct LaneTick {
    /// The tick period: 1, 2, 4 or 5 ms ([`LANE_TICK_QUANTA_MS`]).
    pub quantum: Duration,
    /// File lane-clock deadlines on the tick grid. Needs
    /// [`MediaExecutorConfig::lane_timer`](crate::executor::MediaExecutorConfig::lane_timer).
    pub grid_timers: bool,
}

impl LaneTick {
    /// A tick of `quantum` without grid timers.
    pub fn new(quantum: Duration) -> Self {
        Self {
            quantum,
            grid_timers: false,
        }
    }

    /// Set [`Self::grid_timers`].
    pub fn with_grid_timers(mut self, enabled: bool) -> Self {
        self.grid_timers = enabled;
        self
    }

    /// Whether [`Self::quantum`] is one of [`LANE_TICK_QUANTA_MS`].
    pub fn has_valid_quantum(&self) -> bool {
        LANE_TICK_QUANTA_MS
            .iter()
            .any(|ms| self.quantum == Duration::from_millis(*ms))
    }
}

/// Whether a paced queue built now should schedule its next delivery from
/// the deadline it served rather than from the (up to one quantum later)
/// time it was served. On whenever the lane tick is active. Read when a
/// queue is built, never per frame.
pub fn lane_tick_pacing_enabled() -> bool {
    crate::executor::active_lane_tick().is_some()
}

/// The tick grid lane-clock deadlines are filed on, for a poller that
/// ticks, when grid timers are active.
pub(crate) fn timer_grid() -> Option<TickGrid> {
    if !crate::executor::lane_tick_grid_timers_enabled() {
        return None;
    }
    crate::executor::active_lane_tick().map(|tick| TickGrid::process(tick.quantum))
}

/// A tick scheduler for the lane poller starting on this thread, when the
/// tick is active and the thread is a dedicated current-thread media lane.
/// Counted either way. Called once per poller.
pub(crate) fn scheduler_for_current_lane() -> Option<TickScheduler> {
    let tick = crate::executor::active_lane_tick()?;
    let supported = crate::task_runtime::current_media_lane().is_some()
        && tokio::runtime::Handle::try_current().is_ok_and(|handle| {
            matches!(
                handle.runtime_flavor(),
                tokio::runtime::RuntimeFlavor::CurrentThread
            )
        });
    record_lane_tick_lane(supported);
    if !supported {
        static LOGGED: AtomicBool = AtomicBool::new(false);
        if !LOGGED.swap(true, Ordering::Relaxed) {
            tracing::info!(
                quantum_ms = tick.quantum.as_millis() as u64,
                "lane tick configured, but media lanes are not dedicated current-thread lanes; \
                 their pollers keep immediate wakeups"
            );
        }
        return None;
    }
    Some(TickScheduler::new(TickGrid::process(tick.quantum)))
}

/// Tick boundaries `origin + k·quantum`. Every lane of a process shares one
/// origin. A quantum dividing 20 ms keeps any 20 ms deadline series at a
/// constant phase against the grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TickGrid {
    origin: Instant,
    quantum: Duration,
}

impl TickGrid {
    /// A grid with an explicit origin.
    ///
    /// # Panics
    ///
    /// If `quantum` is zero.
    pub fn new(origin: Instant, quantum: Duration) -> Self {
        assert!(!quantum.is_zero(), "a tick quantum is positive");
        Self { origin, quantum }
    }

    /// The process grid for `quantum` (its origin is fixed at first use).
    pub fn process(quantum: Duration) -> Self {
        static ORIGIN: OnceLock<Instant> = OnceLock::new();
        Self::new(*ORIGIN.get_or_init(Instant::now), quantum)
    }

    /// The distance between boundaries.
    pub fn quantum(&self) -> Duration {
        self.quantum
    }

    fn at(&self, ticks: u128) -> Instant {
        let nanos = ticks.saturating_mul(self.quantum.as_nanos());
        self.origin + Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
    }

    /// The first boundary at or after `t` (the origin for any earlier `t`).
    pub fn ceil(&self, t: Instant) -> Instant {
        let Some(since) = t.checked_duration_since(self.origin) else {
            return self.origin;
        };
        self.at(since.as_nanos().div_ceil(self.quantum.as_nanos()))
    }

    /// The first boundary strictly after `t` (the origin for any earlier
    /// `t`).
    pub fn next_after(&self, t: Instant) -> Instant {
        let Some(since) = t.checked_duration_since(self.origin) else {
            return self.origin;
        };
        self.at(since.as_nanos() / self.quantum.as_nanos() + 1)
    }
}

/// What one poller pass did, for [`TickScheduler::end_pass`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct PassReport {
    pub(crate) started: Instant,
    pub(crate) ended: Instant,
    /// At least one loop or task is hosted.
    pub(crate) hosting: bool,
    /// The pass ran out of its poll budget or left lane-clock deadlines due.
    pub(crate) must_continue: bool,
}

/// What the poller does after a pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TickDecision {
    /// Park as without the tick: the next readiness event, timer or job
    /// wakes the lane.
    Park,
    /// Run the next pass at once, after one zero-timeout driver poll.
    Continue,
    /// Park the lane thread until this boundary, then poll the drivers once
    /// and run a pass.
    SleepUntil(Instant),
}

/// The per-poller tick state machine. Pure: every instant comes from the
/// caller, so it is tested with a fake clock.
#[derive(Debug)]
pub(crate) struct TickScheduler {
    grid: TickGrid,
    /// The boundary the last sleep was for; its pass reports its lateness.
    target: Option<Instant>,
}

impl TickScheduler {
    pub(crate) fn new(grid: TickGrid) -> Self {
        Self { grid, target: None }
    }

    #[cfg(test)]
    pub(crate) fn grid(&self) -> TickGrid {
        self.grid
    }

    /// A pass starts at `now`. Returns how late it is against the boundary
    /// it slept for, when it follows a tick sleep.
    pub(crate) fn begin_pass(&mut self, now: Instant) -> Option<Duration> {
        self.target
            .take()
            .map(|target| now.saturating_duration_since(target))
    }

    /// A pass ended: decide what the poller does next.
    pub(crate) fn end_pass(&mut self, report: PassReport) -> TickDecision {
        self.target = None;
        if !report.hosting {
            // Nothing to tick for: park on the job channel.
            return TickDecision::Park;
        }
        let took = report.ended.saturating_duration_since(report.started);
        if report.must_continue || took >= self.grid.quantum() {
            // Saturated: every pass already batches a quantum or more of
            // arrivals; sleeping would only cost capacity.
            return TickDecision::Continue;
        }
        let next = self.grid.next_after(report.ended);
        self.target = Some(next);
        TickDecision::SleepUntil(next)
    }
}

/// Upper bounds (µs) of the tick lateness buckets of
/// [`LaneTickCounters::lateness`]; the last bucket is open.
pub const LATENESS_BUCKETS_US: [u64; 10] =
    [50, 100, 250, 500, 1_000, 2_000, 3_000, 4_000, 5_000, 10_000];

const LATENESS_SLOTS: usize = LATENESS_BUCKETS_US.len() + 1;

fn lateness_bucket(late: Duration) -> usize {
    let micros = u64::try_from(late.as_micros()).unwrap_or(u64::MAX);
    LATENESS_BUCKETS_US
        .iter()
        .position(|bound| micros <= *bound)
        .unwrap_or(LATENESS_BUCKETS_US.len())
}

static TICK_PASSES: AtomicU64 = AtomicU64::new(0);
static TICK_IDLE_PASSES: AtomicU64 = AtomicU64::new(0);
static TICK_SLEEPS: AtomicU64 = AtomicU64::new(0);
static TICK_SLICES: AtomicU64 = AtomicU64::new(0);
static TICK_SLEEPS_CUT: AtomicU64 = AtomicU64::new(0);
static TICK_OVERRUNS: AtomicU64 = AtomicU64::new(0);
static RUNTIME_TIMER_RESETS: AtomicU64 = AtomicU64::new(0);
static TIMERS_COALESCED: AtomicU64 = AtomicU64::new(0);
static TICK_LANES: AtomicU64 = AtomicU64::new(0);
static TICK_UNSUPPORTED_LANES: AtomicU64 = AtomicU64::new(0);
static LATENESS_SUM_US: AtomicU64 = AtomicU64::new(0);
static LATENESS: [AtomicU64; LATENESS_SLOTS] = [const { AtomicU64::new(0) }; LATENESS_SLOTS];

/// Lane tick totals for this process since start. Each field is
/// individually atomic; the set is not a transaction.
///
/// Per-packet ratios, with [`crate::executor::inline_executor_counters`]:
/// - wakeups per packet = `lane_wakeups / lane_datagrams` (a tick pass is one
///   lane wakeup), [`Self::wakeups_per_packet`];
/// - timer operations per packet = `(lane_timer_arms + runtime_timer_resets
///   + tick_slices) / lane_datagrams`: lane-clock filings, registrations of
///   the lane clock's runtime timer, and timed parks of a ticking lane,
///   [`Self::timer_ops_per_packet`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct LaneTickCounters {
    /// Passes run by ticking pollers (each is also a lane wakeup).
    pub tick_passes: u64,
    /// Of those, passes that found nothing to do.
    pub tick_idle_passes: u64,
    /// Lane-thread sleeps to a tick boundary.
    pub tick_sleeps: u64,
    /// Timed parks making up those sleeps (a sleep parks in slices of at
    /// most 500 µs and checks for cross-thread work in between).
    pub tick_slices: u64,
    /// Sleeps ended early by work from another thread (a hosted loop woken
    /// elsewhere, a newly hosted loop, a non-hosted lane task woken
    /// elsewhere).
    pub tick_sleeps_cut: u64,
    /// Passes that skipped the sleep (a quantum or more of work, an
    /// exhausted poll budget, deadlines still due).
    pub tick_overruns: u64,
    /// Registrations of a lane clock's runtime timer (with or without the
    /// tick).
    pub runtime_timer_resets: u64,
    /// Lane-clock deadline moves that stayed inside their tick and filed
    /// nothing ([`LaneTick::grid_timers`]).
    pub timers_coalesced: u64,
    /// Pollers started with the tick.
    pub tick_lanes: u64,
    /// Pollers started with the tick configured on a lane that cannot tick
    /// (not a dedicated current-thread lane); they keep immediate wakeups.
    pub unsupported_lanes: u64,
    /// Tick lateness (a pass's start after the boundary it slept for), per
    /// bucket of [`LATENESS_BUCKETS_US`] plus the open bucket.
    pub lateness: [u64; LATENESS_SLOTS],
    /// The sum of all tick lateness, in µs.
    pub lateness_sum_us: u64,
}

impl LaneTickCounters {
    /// Lane wakeups per datagram received on a lane poller.
    pub fn wakeups_per_packet(inline: &crate::executor::InlineExecutorCounters) -> Option<f64> {
        (inline.lane_datagrams > 0)
            .then(|| inline.lane_wakeups as f64 / inline.lane_datagrams as f64)
    }

    /// Timer operations per datagram received on a lane poller (see the type
    /// docs).
    pub fn timer_ops_per_packet(
        &self,
        inline: &crate::executor::InlineExecutorCounters,
    ) -> Option<f64> {
        (inline.lane_datagrams > 0).then(|| {
            (inline.lane_timer_arms + self.runtime_timer_resets + self.tick_slices) as f64
                / inline.lane_datagrams as f64
        })
    }
}

/// The lane tick totals since start.
pub fn lane_tick_counters() -> LaneTickCounters {
    let mut lateness = [0u64; LATENESS_SLOTS];
    for (slot, total) in lateness.iter_mut().zip(LATENESS.iter()) {
        *slot = total.load(Ordering::Relaxed);
    }
    LaneTickCounters {
        tick_passes: TICK_PASSES.load(Ordering::Relaxed),
        tick_idle_passes: TICK_IDLE_PASSES.load(Ordering::Relaxed),
        tick_sleeps: TICK_SLEEPS.load(Ordering::Relaxed),
        tick_slices: TICK_SLICES.load(Ordering::Relaxed),
        tick_sleeps_cut: TICK_SLEEPS_CUT.load(Ordering::Relaxed),
        tick_overruns: TICK_OVERRUNS.load(Ordering::Relaxed),
        runtime_timer_resets: RUNTIME_TIMER_RESETS.load(Ordering::Relaxed),
        timers_coalesced: TIMERS_COALESCED.load(Ordering::Relaxed),
        tick_lanes: TICK_LANES.load(Ordering::Relaxed),
        unsupported_lanes: TICK_UNSUPPORTED_LANES.load(Ordering::Relaxed),
        lateness,
        lateness_sum_us: LATENESS_SUM_US.load(Ordering::Relaxed),
    }
}

/// One poller started with the tick active. Control-plane rate.
fn record_lane_tick_lane(supported: bool) {
    let total = if supported {
        &TICK_LANES
    } else {
        &TICK_UNSUPPORTED_LANES
    };
    total.fetch_add(1, Ordering::Relaxed);
}

/// Poller-local lane tick counters, published in batches like the other
/// lane counters (no shared cache line per pass).
#[derive(Default)]
pub(crate) struct LaneTickCounterBatch {
    passes: u64,
    idle: u64,
    sleeps: u64,
    slices: u64,
    cut: u64,
    overruns: u64,
    resets: u64,
    coalesced: u64,
    lateness: [u64; LATENESS_SLOTS],
    lateness_sum_us: u64,
    events: u64,
}

impl LaneTickCounterBatch {
    const FLUSH_EVERY: u64 = 64;

    pub(crate) fn new() -> Self {
        Self::default()
    }

    #[inline]
    fn bump(&mut self) {
        self.events += 1;
        if self.events >= Self::FLUSH_EVERY {
            self.flush();
        }
    }

    /// One pass of a ticking poller, and whether it found work.
    #[inline]
    pub(crate) fn pass(&mut self, busy: bool) {
        self.passes += 1;
        self.idle += u64::from(!busy);
        self.bump();
    }

    #[inline]
    pub(crate) fn lateness(&mut self, late: Duration) {
        self.lateness[lateness_bucket(late)] += 1;
        self.lateness_sum_us += u64::try_from(late.as_micros()).unwrap_or(u64::MAX);
    }

    /// One tick sleep of `slices` timed parks; `cut`: ended early by work
    /// from another thread.
    #[inline]
    pub(crate) fn slept(&mut self, slices: u64, cut: bool) {
        self.sleeps += 1;
        self.slices += slices;
        self.cut += u64::from(cut);
    }

    #[inline]
    pub(crate) fn overrun(&mut self) {
        self.overruns += 1;
    }

    /// Lane-clock runtime timer registrations and coalesced filings since
    /// the previous pass. Called every pass, with or without the tick.
    #[inline]
    pub(crate) fn timers(&mut self, resets: u64, coalesced: u64) {
        self.resets += resets;
        self.coalesced += coalesced;
        if resets + coalesced > 0 {
            self.bump();
        }
    }

    pub(crate) fn flush(&mut self) {
        self.events = 0;
        let scalars = [
            (&mut self.passes, &TICK_PASSES),
            (&mut self.idle, &TICK_IDLE_PASSES),
            (&mut self.sleeps, &TICK_SLEEPS),
            (&mut self.slices, &TICK_SLICES),
            (&mut self.cut, &TICK_SLEEPS_CUT),
            (&mut self.overruns, &TICK_OVERRUNS),
            (&mut self.resets, &RUNTIME_TIMER_RESETS),
            (&mut self.coalesced, &TIMERS_COALESCED),
            (&mut self.lateness_sum_us, &LATENESS_SUM_US),
        ];
        for (pending, total) in scalars {
            let value = std::mem::take(pending);
            if value > 0 {
                total.fetch_add(value, Ordering::Relaxed);
            }
        }
        for (pending, total) in self.lateness.iter_mut().zip(LATENESS.iter()) {
            let value = std::mem::take(pending);
            if value > 0 {
                total.fetch_add(value, Ordering::Relaxed);
            }
        }
    }
}

impl Drop for LaneTickCounterBatch {
    fn drop(&mut self) {
        self.flush();
    }
}

#[cfg(test)]
mod tests {
    //! Fake-clock tests: every instant is `base + offset`, so the schedule
    //! and the lateness bounds are exact.
    use super::*;
    use crate::executor::{
        override_media_executor_config_on_this_thread, MediaExecutorConfig, MediaExecutorMode,
    };

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    fn us(value: u64) -> Duration {
        Duration::from_micros(value)
    }

    /// Deterministic pseudo-random sequence (no `rand` in this crate's tests).
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            self.0 >> 33
        }

        fn below(&mut self, bound: u64) -> u64 {
            self.next() % bound
        }
    }

    #[test]
    fn only_divisors_of_the_media_clock_are_valid_quanta() {
        for value in LANE_TICK_QUANTA_MS {
            assert!(LaneTick::new(ms(value)).has_valid_quantum());
            assert_eq!(
                MEDIA_CLOCK_PERIOD.as_millis() as u64 % value,
                0,
                "divides 20 ms"
            );
        }
        for bad in [
            Duration::ZERO,
            us(500),
            us(1_500),
            ms(3),
            ms(6),
            ms(10),
            ms(20),
        ] {
            assert!(!LaneTick::new(bad).has_valid_quantum(), "{bad:?}");
        }
        let tick = LaneTick::new(ms(2));
        assert!(!tick.grid_timers, "grid timers are opt-in");
        assert!(tick.with_grid_timers(true).grid_timers);
    }

    #[test]
    fn the_tick_is_never_active_under_v1_or_without_the_lane_poller() {
        let configured = Some(LaneTick::new(ms(2)).with_grid_timers(true));
        for config in [
            MediaExecutorConfig::v1().with_lane_tick(configured),
            MediaExecutorConfig::v2().with_mode(MediaExecutorMode::V1),
            MediaExecutorConfig::v2().with_lane_poller(false),
        ] {
            let _guard = override_media_executor_config_on_this_thread(config);
            assert_eq!(crate::executor::active_lane_tick(), None);
            assert!(!crate::executor::lane_tick_grid_timers_enabled());
            assert!(!lane_tick_pacing_enabled());
            assert!(timer_grid().is_none());
            assert!(scheduler_for_current_lane().is_none());
        }
    }

    #[test]
    fn grid_timers_need_the_lane_clock() {
        let _guard = override_media_executor_config_on_this_thread(
            MediaExecutorConfig::v2().with_lane_timer(false),
        );
        assert!(crate::executor::active_lane_tick().is_some());
        assert!(lane_tick_pacing_enabled());
        assert!(!crate::executor::lane_tick_grid_timers_enabled());
        assert!(timer_grid().is_none());
    }

    #[test]
    fn a_thread_that_is_not_a_dedicated_lane_never_ticks_and_is_counted() {
        let _guard = override_media_executor_config_on_this_thread(MediaExecutorConfig::v2());
        let before = lane_tick_counters();
        assert!(
            scheduler_for_current_lane().is_none(),
            "the test thread is no media lane"
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            assert!(
                scheduler_for_current_lane().is_none(),
                "a current-thread runtime that is not a media lane does not tick"
            );
        });
        let after = lane_tick_counters();
        assert!(after.unsupported_lanes >= before.unsupported_lanes + 2);
    }

    #[test]
    fn grid_ceil_and_next_after_bound_every_instant_by_one_quantum() {
        let base = Instant::now();
        let mut rng = Lcg(49);
        for quantum_ms in LANE_TICK_QUANTA_MS {
            let quantum = ms(quantum_ms);
            let grid = TickGrid::new(base, quantum);
            assert_eq!(grid.quantum(), quantum);
            assert_eq!(grid.ceil(base), base, "a boundary is its own ceiling");
            assert_eq!(grid.next_after(base), base + quantum);
            for _ in 0..2_000 {
                let t = base + Duration::from_nanos(rng.below(3_000_000_000));
                let ceil = grid.ceil(t);
                let next = grid.next_after(t);
                assert!(ceil >= t && ceil - t < quantum);
                assert!(next > t && next - t <= quantum);
                assert_eq!(
                    (ceil - base).as_nanos() % quantum.as_nanos(),
                    0,
                    "on the grid"
                );
                assert_eq!(
                    (next - base).as_nanos() % quantum.as_nanos(),
                    0,
                    "on the grid"
                );
            }
        }
    }

    #[test]
    fn a_20ms_deadline_series_keeps_a_constant_phase_on_the_grid() {
        // The added jitter of a periodic media stream is zero: every
        // deadline of a series is served the same time after it.
        let base = Instant::now();
        let mut rng = Lcg(20);
        for quantum_ms in LANE_TICK_QUANTA_MS {
            let grid = TickGrid::new(base, ms(quantum_ms));
            for _ in 0..50 {
                let first = base + Duration::from_nanos(rng.below(40_000_000));
                let phase = grid.ceil(first) - first;
                for frame in 1..200u32 {
                    let deadline = first + MEDIA_CLOCK_PERIOD * frame;
                    assert_eq!(grid.ceil(deadline) - deadline, phase);
                }
            }
        }
    }

    /// The outcome of [`simulate`].
    struct Simulated {
        /// Per event, the start of the pass that served it.
        served: Vec<Instant>,
        /// The lateness reported by every pass that followed a sleep.
        lateness: Vec<Duration>,
        /// Passes that skipped the sleep.
        continues: usize,
        /// Every pass (each one lane wakeup).
        passes: usize,
    }

    /// Simulate a ticking lane against a fake clock: events (datagram
    /// arrivals or deadlines) at arbitrary instants, passes of a given cost
    /// and a thread wake-up latency.
    fn simulate(
        scheduler: &mut TickScheduler,
        events: &[Instant],
        start: Instant,
        pass_cost: impl Fn(usize) -> Duration,
        wake_latency: Duration,
    ) -> Simulated {
        let mut served = vec![None; events.len()];
        let mut lateness = Vec::new();
        let mut continues = 0;
        let mut now = start;
        let mut next_event = 0;
        let mut passes = 0usize;
        while next_event < events.len() {
            if let Some(late) = scheduler.begin_pass(now) {
                lateness.push(late);
            }
            let pass_start = now;
            let mut handled = 0;
            while next_event < events.len() && events[next_event] <= pass_start {
                served[next_event] = Some(pass_start);
                next_event += 1;
                handled += 1;
            }
            let ended = pass_start + pass_cost(handled);
            let decision = scheduler.end_pass(PassReport {
                started: pass_start,
                ended,
                hosting: true,
                must_continue: false,
            });
            now = match decision {
                TickDecision::SleepUntil(at) => {
                    assert!(at > ended && at - ended <= scheduler.grid().quantum());
                    at + wake_latency
                }
                TickDecision::Continue => {
                    continues += 1;
                    ended
                }
                TickDecision::Park => panic!("a hosting lane never parks"),
            };
            passes += 1;
            assert!(passes < 1_000_000);
        }
        Simulated {
            served: served.into_iter().map(Option::unwrap).collect(),
            lateness,
            continues,
            passes,
        }
    }

    #[test]
    fn a_ticking_lane_serves_every_event_within_one_quantum_in_far_fewer_wakeups() {
        let base = Instant::now();
        let wake_latency = us(60);
        for quantum_ms in LANE_TICK_QUANTA_MS {
            let quantum = ms(quantum_ms);
            let mut rng = Lcg(quantum_ms);
            // 400 calls' worth of 20 ms streams with random phases plus
            // jitter: about 20 events per millisecond.
            let mut events: Vec<Instant> = (0..400)
                .flat_map(|_| {
                    let phase = rng.below(20_000_000);
                    let jitter: Vec<u64> = (0..25).map(|_| rng.below(500_000)).collect();
                    (0..25u64).map(move |frame| {
                        base + Duration::from_nanos(
                            phase + frame * 20_000_000 + jitter[frame as usize],
                        )
                    })
                })
                .collect();
            events.sort();
            let mut scheduler = TickScheduler::new(TickGrid::new(base, quantum));
            // A cheap pass: 2 µs plus 1 µs per event.
            let cost = |handled: usize| us(2 + handled as u64);
            let run = simulate(&mut scheduler, &events, base, cost, wake_latency);
            for (event, at) in events.iter().zip(&run.served) {
                assert!(at >= event, "never served before it happened");
                // Waiting = busy time of the pass in flight + < one quantum of
                // sleep + the thread's wake-up latency.
                let waited = *at - *event;
                assert!(
                    waited <= quantum + wake_latency + us(500),
                    "Q={quantum:?}: waited {waited:?}"
                );
            }
            assert_eq!(run.continues, 0, "cheap passes never overrun");
            assert!(!run.lateness.is_empty());
            assert!(
                run.lateness.iter().all(|late| *late == wake_latency),
                "lateness is the wake-up latency"
            );
            // Wakeups are bounded by the tick count, not by the event count:
            // one pass per boundary over the ~500 ms the events span.
            let span = *events.last().unwrap() - base;
            let boundaries = (span.as_nanos() / quantum.as_nanos()) as usize + 2;
            assert!(
                run.passes <= boundaries,
                "Q={quantum:?}: {} passes for {boundaries} boundaries",
                run.passes
            );
            assert!(
                run.passes < events.len() / 4,
                "Q={quantum:?}: {} passes for {} events",
                run.passes,
                events.len()
            );
        }
    }

    #[test]
    fn a_pass_of_a_quantum_or_more_and_an_exhausted_budget_skip_the_sleep() {
        let base = Instant::now();
        let grid = TickGrid::new(base, ms(2));
        let mut scheduler = TickScheduler::new(grid);
        let report = |started: Instant, took: Duration, must_continue: bool| PassReport {
            started,
            ended: started + took,
            hosting: true,
            must_continue,
        };
        let decision = scheduler.end_pass(report(base, us(1_999), false));
        assert_eq!(decision, TickDecision::SleepUntil(base + ms(2)));
        assert_eq!(scheduler.begin_pass(base + ms(2) + us(40)), Some(us(40)));
        let decision = scheduler.end_pass(report(base + ms(2), ms(2), false));
        assert_eq!(
            decision,
            TickDecision::Continue,
            "a full quantum of work: no sleep"
        );
        assert_eq!(
            scheduler.begin_pass(base + ms(4)),
            None,
            "an unscheduled pass has no lateness"
        );
        let decision = scheduler.end_pass(report(base + ms(4), us(10), true));
        assert_eq!(
            decision,
            TickDecision::Continue,
            "budget exhausted or deadlines due"
        );
        let decision = scheduler.end_pass(PassReport {
            started: base + ms(5),
            ended: base + ms(5),
            hosting: false,
            must_continue: false,
        });
        assert_eq!(
            decision,
            TickDecision::Park,
            "nothing hosted: park on the job channel"
        );
        assert_eq!(
            scheduler.begin_pass(base + ms(9)),
            None,
            "a park has no lateness"
        );
        let decision = scheduler.end_pass(report(base + ms(9), us(10), false));
        assert_eq!(
            decision,
            TickDecision::SleepUntil(base + ms(10)),
            "the lane ticks again when work returns"
        );
    }

    #[test]
    fn saturated_passes_degrade_to_continuous_processing() {
        // Overload: a pass costs 900 µs + 2 µs per event and events arrive
        // every 10 µs, so a steady pass (≈ 1.125 ms) is longer than the 1 ms
        // quantum; the lane must not add sleeps (capacity and the overload
        // path stay as without the tick).
        let base = Instant::now();
        let mut scheduler = TickScheduler::new(TickGrid::new(base, ms(1)));
        let events: Vec<Instant> = (0..5_000u64).map(|i| base + us(i * 10)).collect();
        let run = simulate(
            &mut scheduler,
            &events,
            base,
            |handled| us(900 + 2 * handled as u64),
            us(60),
        );
        assert!(
            run.continues > 20,
            "saturated passes run back to back: {}",
            run.continues
        );
        assert!(
            run.lateness.len() <= 2,
            "at most the warm-up pass slept: {}",
            run.lateness.len()
        );
        // Throughput: the lane never idles while work is queued.
        for (event, at) in events.iter().zip(&run.served) {
            assert!(*at - *event < ms(3), "waited {:?}", *at - *event);
        }
    }

    #[test]
    fn lateness_buckets_cover_every_value_once() {
        assert_eq!(lateness_bucket(Duration::ZERO), 0);
        assert_eq!(lateness_bucket(us(50)), 0);
        assert_eq!(lateness_bucket(us(51)), 1);
        assert_eq!(lateness_bucket(us(999)), 4);
        assert_eq!(lateness_bucket(ms(1)), 4);
        assert_eq!(lateness_bucket(ms(5)), 8);
        assert_eq!(lateness_bucket(ms(10)), 9);
        assert_eq!(lateness_bucket(ms(11)), 10);
        assert_eq!(lateness_bucket(Duration::MAX), 10);
    }

    #[test]
    fn tick_counters_flush_in_batches_and_on_drop() {
        let before = lane_tick_counters();
        let mut batch = LaneTickCounterBatch::new();
        batch.pass(false);
        batch.pass(true);
        batch.lateness(us(70));
        batch.lateness(ms(3));
        batch.slept(4, true);
        batch.slept(2, false);
        batch.overrun();
        batch.timers(3, 2);
        assert_eq!(
            lane_tick_counters().tick_slices,
            before.tick_slices,
            "batched"
        );
        drop(batch);
        record_lane_tick_lane(true);
        record_lane_tick_lane(false);
        let after = lane_tick_counters();
        assert!(after.tick_passes >= before.tick_passes + 2);
        assert!(after.tick_idle_passes > before.tick_idle_passes);
        assert!(after.tick_sleeps >= before.tick_sleeps + 2);
        assert!(after.tick_slices >= before.tick_slices + 6);
        assert!(after.tick_sleeps_cut > before.tick_sleeps_cut);
        assert!(after.tick_overruns > before.tick_overruns);
        assert!(after.runtime_timer_resets >= before.runtime_timer_resets + 3);
        assert!(after.timers_coalesced >= before.timers_coalesced + 2);
        assert!(after.lateness[1] > before.lateness[1]);
        assert!(after.lateness[6] > before.lateness[6]);
        assert!(after.lateness_sum_us >= before.lateness_sum_us + 3_070);
        assert!(after.tick_lanes > before.tick_lanes);
        assert!(after.unsupported_lanes > before.unsupported_lanes);
        let inline = crate::executor::InlineExecutorCounters {
            lane_wakeups: 10,
            lane_datagrams: 40,
            lane_timer_arms: 60,
            ..Default::default()
        };
        assert_eq!(LaneTickCounters::wakeups_per_packet(&inline), Some(0.25));
        let tick = LaneTickCounters {
            runtime_timer_resets: 10,
            tick_slices: 10,
            ..Default::default()
        };
        assert_eq!(tick.timer_ops_per_packet(&inline), Some(2.0));
        assert_eq!(
            LaneTickCounters::wakeups_per_packet(&Default::default()),
            None
        );
    }
}
