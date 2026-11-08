//! Deterministic: every instant is explicit (the lane clock tests'
//! fake-clock style); `ManualLane` stands in for the poller's pass.
use super::*;
use std::sync::Mutex;
use std::time::Duration;

use crate::executor::{override_media_executor_config_on_this_thread, MediaExecutorConfig};

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

/// What a scripted source saw, shared with the test.
#[derive(Default)]
struct Log {
    fires: Vec<(u32, Duration)>,
    retired: Vec<(u32, RetireReason)>,
    dropped: Vec<u32>,
}

/// Fires every `period` from `start` until `frames` fires, then is done.
/// `panic_at`: panic on that fire. `escalate_at`: escalate on that fire.
/// `again`: ask for an instant already due this many times in a row.
struct Scripted {
    id: u32,
    start: Instant,
    period: Duration,
    frames: u32,
    fired: u32,
    panic_at: Option<u32>,
    escalate_at: Option<u32>,
    again: u32,
    log: Arc<Mutex<Log>>,
}

impl Scripted {
    fn new(id: u32, start: Instant, period: Duration, frames: u32, log: &Arc<Mutex<Log>>) -> Self {
        Self {
            id,
            start,
            period,
            frames,
            fired: 0,
            panic_at: None,
            escalate_at: None,
            again: 0,
            log: Arc::clone(log),
        }
    }

    fn deadline(&self) -> Instant {
        self.start + self.period * self.fired
    }
}

impl LaneTxSource for Scripted {
    fn on_due(&mut self, now: Instant) -> LaneNext {
        assert!(now >= self.deadline(), "never fired before its deadline");
        self.log
            .lock()
            .unwrap()
            .fires
            .push((self.id, now - self.start));
        if self.panic_at == Some(self.fired) {
            panic!("scripted handler panic");
        }
        if self.escalate_at == Some(self.fired) {
            self.fired += 1;
            return LaneNext::Escalate(Escalation::SendDeferred);
        }
        if self.again > 0 {
            self.again -= 1;
            return LaneNext::At(now);
        }
        self.fired += 1;
        if self.fired >= self.frames {
            return LaneNext::Done;
        }
        LaneNext::At(self.deadline())
    }

    fn on_retire(&mut self, reason: RetireReason) {
        self.log.lock().unwrap().retired.push((self.id, reason));
    }
}

impl Drop for Scripted {
    fn drop(&mut self) {
        if let Ok(mut log) = self.log.lock() {
            log.dropped.push(self.id);
        }
    }
}

fn install(lane: &mut ManualLane, source: Scripted, first: Instant) -> LaneTxHandle {
    lane.enter(|| {
        install_tx(Box::new(source), first)
            .ok()
            .expect("a table is current")
    })
}

/// Step `lane` through every instant from `from` to `to` in `step`s.
fn run(lane: &mut ManualLane, from: Instant, to: Instant, step: Duration) {
    let mut now = from;
    while now <= to {
        lane.pass(now, || {});
        now += step;
    }
}

fn finished_now(handle: &mut LaneTxHandle) -> Option<Option<LaneTxReturn>> {
    let waker = futures::task::noop_waker();
    let mut cx = Context::from_waker(&waker);
    match handle.poll_finished(&mut cx) {
        Poll::Ready(value) => Some(value),
        Poll::Pending => None,
    }
}

fn fire_times(log: &Arc<Mutex<Log>>) -> Vec<Duration> {
    log.lock()
        .unwrap()
        .fires
        .iter()
        .map(|(_, at)| *at)
        .collect()
}

#[test]
fn the_switch_is_v2_only_and_a_poller_keeps_a_table_only_on_a_dedicated_lane() {
    let accessor = |config: MediaExecutorConfig| {
        let _guard = override_media_executor_config_on_this_thread(config);
        (callback_send_enabled(), table_for_current_lane(true, true))
    };
    assert_eq!(accessor(MediaExecutorConfig::v1()), (false, false));
    assert_eq!(
        accessor(MediaExecutorConfig::v1().with_callback_send(true)),
        (false, false),
        "never under v1"
    );
    let before = callback_lane_counters();
    // A test thread is not a dedicated media lane: the switch is on, but the
    // poller keeps no table (counted).
    assert_eq!(accessor(MediaExecutorConfig::v2()), (true, false));
    assert!(callback_lane_counters().unsupported_lanes > before.unsupported_lanes);
    assert_eq!(
        accessor(MediaExecutorConfig::v2().with_callback_send(false)),
        (false, false)
    );
    let _v2 = override_media_executor_config_on_this_thread(MediaExecutorConfig::v2());
    assert!(!table_for_current_lane(true, false), "needs the wheel");
    assert!(!table_for_current_lane(false, true), "needs the lane clock");
}

#[test]
fn installing_without_a_table_gives_the_source_back() {
    let log = Arc::new(Mutex::new(Log::default()));
    let start = Instant::now();
    let before = callback_lane_counters().install_fallbacks;
    let source = Box::new(Scripted::new(1, start, ms(20), 3, &log));
    let back = install_tx(source, start)
        .err()
        .expect("no table on this thread");
    let back = back.downcast::<Scripted>().ok().expect("the same source");
    assert_eq!(back.id, 1);
    assert!(log.lock().unwrap().fires.is_empty());
    assert!(callback_lane_counters().install_fallbacks > before);
}

#[test]
fn a_returned_source_downcasts_only_to_its_own_type() {
    struct Other;
    impl LaneTxSource for Other {
        fn on_due(&mut self, _now: Instant) -> LaneNext {
            LaneNext::Done
        }
    }
    let log = Arc::new(Mutex::new(Log::default()));
    let source: Box<dyn LaneTxSource> = Box::new(Scripted::new(7, Instant::now(), ms(20), 1, &log));
    let source = source
        .downcast::<Other>()
        .err()
        .expect("not an Other: given back");
    assert!(log.lock().unwrap().dropped.is_empty(), "not dropped");
    assert_eq!(source.downcast::<Scripted>().ok().expect("ours").id, 7);
}

/// Cadence, never early: a 20 ms source fires on its deadlines exactly, at
/// its first check at or after each, and finishes.
#[test]
fn entries_fire_on_their_deadlines_never_early_and_finish() {
    let log = Arc::new(Mutex::new(Log::default()));
    let start = Instant::now();
    let mut lane = ManualLane::new(start, None);
    let mut handle = install(&mut lane, Scripted::new(1, start, ms(20), 5, &log), start);
    run(&mut lane, start, start + ms(200), ms(1));
    assert_eq!(
        fire_times(&log),
        vec![ms(0), ms(20), ms(40), ms(60), ms(80)]
    );
    assert!(matches!(
        finished_now(&mut handle),
        Some(Some(LaneTxReturn::Done(_)))
    ));
    assert!(lane.is_empty());
    assert_eq!(lane.next_deadline(), None);
}

/// On a tick grid a deadline inside a tick fires at the tick boundary at
/// or after it, as a lane-clock sleep does.
#[test]
fn on_a_tick_grid_entries_fire_at_the_boundary_at_or_after_their_deadline() {
    let log = Arc::new(Mutex::new(Log::default()));
    let start = Instant::now();
    let mut lane = ManualLane::new(start, Some(ms(2)));
    let first = start + Duration::from_micros(2_500);
    let _handle = install(&mut lane, Scripted::new(1, first, ms(20), 3, &log), first);
    // Passes on the 2 ms grid only, as a ticking lane runs them.
    run(&mut lane, start, start + ms(60), ms(2));
    assert_eq!(
        fire_times(&log),
        vec![
            Duration::from_micros(1_500),
            Duration::from_micros(21_500),
            Duration::from_micros(41_500)
        ],
        "4 ms, 24 ms, 44 ms: the boundary at or after each deadline"
    );
}

/// Hot call state: a table built with hot state on prefetches each due
/// entry and its source before the first fire of a pass (two lines per due
/// entry, counted with the lane clock's prefetches), and fires exactly as a
/// table without it. Without hot state (v1, or v2 with it off) a table never
/// prefetches.
#[test]
fn with_hot_state_due_entries_and_their_sources_are_prefetched_before_firing() {
    let run_with = |config: MediaExecutorConfig| {
        let _guard = override_media_executor_config_on_this_thread(config);
        let log = Arc::new(Mutex::new(Log::default()));
        let start = Instant::now();
        let mut lane = ManualLane::new(start, None);
        let mut handles = Vec::new();
        for (id, offset) in [(1u32, 3u64), (2, 1), (3, 3)] {
            let first = start + ms(offset);
            handles.push(install(
                &mut lane,
                Scripted::new(id, first, ms(20), 2, &log),
                first,
            ));
        }
        // (fires, lines prefetched) of every pass that fired, as
        // `ManualLane::pass` runs it.
        let mut passes = Vec::new();
        let mut now = start;
        while now <= start + ms(30) {
            let report = lane.enter(|| with_current(|table| table.fire_due(now)).unwrap());
            lane.table.as_mut().unwrap().end_pass();
            if report.fires > 0 || report.prefetched > 0 {
                passes.push((report.fires, report.prefetched));
            }
            now += ms(1);
        }
        assert!(lane.is_empty());
        let fires = log.lock().unwrap().fires.clone();
        (passes, fires)
    };

    let before = crate::hot_state::hot_state_counters().prefetches;
    let (on, fires_on) = run_with(MediaExecutorConfig::v2());
    assert_eq!(
        on,
        vec![(1, 2), (2, 4), (1, 2), (2, 4)],
        "v2 runs hot state: each due entry and its source, every pass"
    );
    assert!(crate::hot_state::hot_state_counters().prefetches >= before + 12);

    let (off, fires_off) = run_with(MediaExecutorConfig::v2().with_hot_state(false));
    assert_eq!(off, vec![(1, 0), (2, 0), (1, 0), (2, 0)]);
    assert_eq!(fires_on, fires_off, "prefetching changes no fire");
    let (v1, _) = run_with(MediaExecutorConfig::v1());
    assert!(v1.iter().all(|&(_, lines)| lines == 0), "never under v1");
}

/// Entries due together fire in (key, slot) order, the lane clock's wake
/// order, and every one is served.
#[test]
fn due_entries_fire_in_key_then_slot_order() {
    let log = Arc::new(Mutex::new(Log::default()));
    let start = Instant::now();
    let mut lane = ManualLane::new(start, None);
    let mut handles = Vec::new();
    for (id, offset) in [(1u32, 3u64), (2, 1), (3, 3), (4, 2)] {
        let first = start + ms(offset);
        handles.push(install(
            &mut lane,
            Scripted::new(id, first, ms(20), 1, &log),
            first,
        ));
    }
    lane.pass(start + ms(5), || {});
    let order: Vec<u32> = log
        .lock()
        .unwrap()
        .fires
        .iter()
        .map(|(id, _)| *id)
        .collect();
    assert_eq!(order, vec![2, 4, 1, 3]);
    assert!(lane.is_empty());
}

/// Budgets: one pass fires at most the pass budget and an entry at most
/// its per-pass budget; the rest stays due, is reported (the poller
/// continues) and fires in the next pass.
#[test]
fn pass_and_entry_budgets_carry_the_rest_to_the_next_pass() {
    let log = Arc::new(Mutex::new(Log::default()));
    let start = Instant::now();
    let mut lane = ManualLane::new(start, None);
    let count = CALLBACK_FIRES_PER_PASS + 10;
    let mut handles = Vec::new();
    for id in 0..count {
        handles.push(install(
            &mut lane,
            Scripted::new(id, start, ms(20), 1, &log),
            start,
        ));
    }
    let now = start + ms(1);
    let carried = lane
        .enter(|| {
            with_current(|table| {
                table.begin_pass(&futures::task::noop_waker());
                let report = table.fire_due(now);
                assert_eq!(report.fires, u64::from(CALLBACK_FIRES_PER_PASS));
                assert_eq!(report.carried, 10);
                assert!(table.has_due(now), "the carried entries are still due");
                table.carried()
            })
        })
        .expect("table");
    assert_eq!(carried, 10);
    assert_eq!(lane.pass(now, || {}), 10, "served in the next pass");
    assert!(lane.is_empty());
    // Per entry: a source that keeps asking for "now" fires twice per pass,
    // then waits for the next pass.
    let mut greedy = Scripted::new(99, now, ms(20), 1, &log);
    greedy.again = 5;
    let _greedy = install(&mut lane, greedy, now);
    log.lock().unwrap().fires.clear();
    let fired = lane
        .enter(|| {
            with_current(|table| {
                table.begin_pass(&futures::task::noop_waker());
                let first = table.fire_due(now);
                let second = table.fire_due(now);
                let third = table.fire_due(now);
                (first, second, third)
            })
        })
        .expect("table");
    assert_eq!(
        fired.0.fires + fired.1.fires,
        u64::from(CALLBACK_FIRES_PER_ENTRY_PER_PASS)
    );
    assert_eq!(
        fired.2,
        FireReport {
            fires: 0,
            carried: 1,
            returned: 0,
            // Hot call state prefetching has its own test.
            prefetched: fired.2.prefetched,
            rx_calls: 0,
        },
        "over its budget: carried"
    );
    assert_eq!(lane.pass(now, || {}), 1, "the next pass serves it again");
}

/// Teardown and the ordering rule: a retirement inside a pass, before the
/// fire step, wins over a frame due in that pass; the source comes back at
/// once and `done` never resolves with a value.
#[test]
fn a_retirement_before_the_fire_step_beats_a_same_instant_frame() {
    let log = Arc::new(Mutex::new(Log::default()));
    let start = Instant::now();
    let mut lane = ManualLane::new(start, None);
    let mut handle = install(&mut lane, Scripted::new(1, start, ms(20), 50, &log), start);
    run(&mut lane, start, start + ms(39), ms(1));
    assert_eq!(log.lock().unwrap().fires.len(), 2);
    let mut back = None;
    lane.pass(start + ms(40), || {
        back = Some(handle.retire(RetireReason::Control));
    });
    let Some(Retired::Now(source)) = back else {
        panic!("retired synchronously on its lane");
    };
    assert_eq!(
        log.lock().unwrap().fires.len(),
        2,
        "the frame due at 40 ms was not fired"
    );
    assert_eq!(
        log.lock().unwrap().retired,
        vec![(1, RetireReason::Control)]
    );
    assert!(lane.is_empty());
    assert_eq!(lane.next_deadline(), None, "nothing left filed");
    assert!(
        matches!(finished_now(&mut handle), Some(None)),
        "done closes without a value"
    );
    drop(source);
    assert_eq!(log.lock().unwrap().dropped, vec![1]);
}

/// A retirement posted from off the lane (or with no table current) is
/// applied by the lane before it fires anything, and the source comes back
/// through `done`.
#[test]
fn an_off_lane_retirement_is_applied_before_the_next_fire() {
    let log = Arc::new(Mutex::new(Log::default()));
    let start = Instant::now();
    let mut lane = ManualLane::new(start, None);
    let mut handle = install(&mut lane, Scripted::new(1, start, ms(20), 50, &log), start);
    lane.pass(start, || {});
    assert!(
        matches!(handle.retire(RetireReason::Control), Retired::Pending),
        "no table current here"
    );
    lane.pass(start + ms(20), || {});
    assert_eq!(
        log.lock().unwrap().fires.len(),
        1,
        "retired before the 20 ms fire"
    );
    assert!(matches!(
        finished_now(&mut handle),
        Some(Some(LaneTxReturn::Retired(_)))
    ));
    assert!(lane.is_empty());
}

/// Panic containment: a panicking handler poisons only its entry. The
/// other entries keep their deadlines, the wheel stays consistent and the
/// supervisor is told.
#[test]
fn a_panicking_handler_poisons_only_its_entry() {
    let log = Arc::new(Mutex::new(Log::default()));
    let start = Instant::now();
    let mut lane = ManualLane::new(start, None);
    let mut bad = Scripted::new(1, start, ms(20), 10, &log);
    bad.panic_at = Some(2);
    let mut bad = install(&mut lane, bad, start);
    let mut good = install(&mut lane, Scripted::new(2, start, ms(20), 10, &log), start);
    let panics_before = callback_lane_counters().panics;
    run(&mut lane, start, start + ms(200), ms(1));
    assert!(matches!(
        finished_now(&mut bad),
        Some(Some(LaneTxReturn::Poisoned))
    ));
    assert!(matches!(
        finished_now(&mut good),
        Some(Some(LaneTxReturn::Done(_)))
    ));
    let log = log.lock().unwrap();
    let good_fires: Vec<Duration> = log
        .fires
        .iter()
        .filter(|(id, _)| *id == 2)
        .map(|(_, at)| *at)
        .collect();
    assert_eq!(
        good_fires,
        (0..10).map(|frame| ms(20 * frame)).collect::<Vec<_>>()
    );
    assert_eq!(
        log.fires.iter().filter(|(id, _)| *id == 1).count(),
        3,
        "fires 0 and 1, then the panic"
    );
    assert!(log.dropped.contains(&1), "the poisoned source was dropped");
    assert!(callback_lane_counters().panics > panics_before);
    assert!(lane.is_empty());
    assert_eq!(lane.next_deadline(), None);
}

/// A panicking `Drop` of a poisoned source is contained too.
#[test]
fn a_panicking_drop_is_contained() {
    struct Bomb;
    impl LaneTxSource for Bomb {
        fn on_due(&mut self, _now: Instant) -> LaneNext {
            panic!("first panic");
        }
    }
    impl Drop for Bomb {
        fn drop(&mut self) {
            panic!("panic while dropping");
        }
    }
    let start = Instant::now();
    let mut lane = ManualLane::new(start, None);
    let mut handle = lane.enter(|| install_tx(Box::new(Bomb), start).ok().expect("installed"));
    lane.pass(start, || {});
    assert!(matches!(
        finished_now(&mut handle),
        Some(Some(LaneTxReturn::Poisoned))
    ));
    assert!(lane.is_empty());
}

/// Escalation: the source leaves the table with its reason, the slot is
/// free, and installing it again continues where it stopped.
#[test]
fn an_escalated_source_comes_back_and_resumes_after_reinstall() {
    let log = Arc::new(Mutex::new(Log::default()));
    let start = Instant::now();
    let mut lane = ManualLane::new(start, None);
    let mut source = Scripted::new(1, start, ms(20), 5, &log);
    source.escalate_at = Some(1);
    let mut handle = install(&mut lane, source, start);
    run(&mut lane, start, start + ms(20), ms(1));
    let Some(Some(LaneTxReturn::Escalated(back, Escalation::SendDeferred))) =
        finished_now(&mut handle)
    else {
        panic!("escalated");
    };
    assert!(lane.is_empty());
    let back = back.downcast::<Scripted>().ok().expect("same type");
    let next = back.deadline();
    let resumes = callback_lane_counters().resumes;
    record_resume(true);
    assert!(callback_lane_counters().resumes > resumes);
    let mut handle = lane.enter(|| install_tx(back, next).ok().expect("installed again"));
    run(&mut lane, start + ms(21), start + ms(100), ms(1));
    assert!(matches!(
        finished_now(&mut handle),
        Some(Some(LaneTxReturn::Done(_)))
    ));
    assert_eq!(
        fire_times(&log),
        vec![ms(0), ms(20), ms(40), ms(60), ms(80)]
    );
}

/// Generation fencing: under randomized slot reuse no stale handle,
/// retirement or wheel item ever reaches a later occupant of its slot.
#[test]
fn reused_slots_are_fenced_by_generation() {
    let log = Arc::new(Mutex::new(Log::default()));
    let start = Instant::now();
    let mut lane = ManualLane::new(start, None);
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut live: Vec<LaneTxHandle> = Vec::new();
    // (slot, generation) of every retired entry.
    let mut stale: Vec<(u32, u32)> = Vec::new();
    let mut now = start;
    for id in 0..400u32 {
        now += Duration::from_micros(next() % 3_000);
        let frames = 1 + (next() % 4) as u32;
        let first = now + Duration::from_micros(next() % 30_000);
        live.push(install(
            &mut lane,
            Scripted::new(id, first, ms(7), frames, &log),
            first,
        ));
        if next() % 3 == 0 {
            let mut handle = live.swap_remove((next() % live.len() as u64) as usize);
            stale.push((handle.slot, handle.generation));
            lane.pass(now, || {
                if let Retired::Now(source) = handle.retire(RetireReason::Control) {
                    drop(source);
                }
            });
        } else {
            lane.pass(now, || {});
        }
        // Every retired (slot, generation) is stale now, whoever holds the
        // slot: retiring it again changes nothing.
        let before = lane.len();
        for &(slot, generation) in &stale {
            let retired = lane.enter(|| {
                with_current(|table| {
                    table
                        .retire(slot, generation, RetireReason::Control)
                        .is_some()
                })
            });
            assert_eq!(retired, Some(false), "slot {slot} generation {generation}");
        }
        assert_eq!(lane.len(), before, "a stale retirement changed nothing");
    }
    run(&mut lane, now, now + ms(200), ms(1));
    assert!(lane.is_empty(), "every live entry finished");
    assert_eq!(
        lane.table.as_ref().map(|table| table.wheel_lengths()),
        Some((0, 0))
    );
    // A source retired by its supervisor never fired again afterwards: each
    // source fires at most its frame count.
    let log = log.lock().unwrap();
    let mut per_source = std::collections::HashMap::<u32, u32>::new();
    for (id, _) in &log.fires {
        *per_source.entry(*id).or_default() += 1;
    }
    assert!(per_source.values().all(|fires| *fires <= 4));
}

/// Lane shutdown drops every source and every supervisor sees its lane go;
/// dropping a supervisor's handle retires its entry.
#[test]
fn shutdown_and_a_dropped_supervisor_release_everything() {
    let log = Arc::new(Mutex::new(Log::default()));
    let start = Instant::now();
    let mut lane = ManualLane::new(start, None);
    let mut kept = install(&mut lane, Scripted::new(1, start, ms(20), 100, &log), start);
    let dropped = install(&mut lane, Scripted::new(2, start, ms(20), 100, &log), start);
    lane.pass(start, || {});
    lane.enter(|| drop(dropped));
    assert_eq!(lane.len(), 1);
    assert_eq!(
        log.lock().unwrap().retired,
        vec![(2, RetireReason::SupervisorDropped)]
    );
    lane.shut_down();
    assert!(
        matches!(finished_now(&mut kept), Some(None)),
        "the lane went"
    );
    let dropped: Vec<u32> = log.lock().unwrap().dropped.clone();
    assert!(dropped.contains(&1) && dropped.contains(&2));
}

/// No lost wakeup, table side: after every pass the table reports exactly
/// whether something is due, and its earliest key, so a poller that sleeps
/// only when nothing is due never strands an entry; an off-lane post is
/// pending work by itself.
#[test]
fn the_table_never_hides_due_work_from_the_poller() {
    let log = Arc::new(Mutex::new(Log::default()));
    let start = Instant::now();
    let mut lane = ManualLane::new(start, Some(ms(2)));
    let mut handle = install(
        &mut lane,
        Scripted::new(1, start + ms(3), ms(20), 3, &log),
        start + ms(3),
    );
    let due = |lane: &mut ManualLane, now: Instant| {
        lane.enter(|| with_current(|table| table.has_due(now)))
            .unwrap()
    };
    assert!(!due(&mut lane, start + ms(2)));
    assert!(due(&mut lane, start + ms(4)), "filed at the 4 ms boundary");
    assert_eq!(lane.next_deadline(), Some(start + ms(4)));
    lane.pass(start + ms(4), || {});
    assert!(!due(&mut lane, start + ms(4)));
    assert_eq!(lane.next_deadline(), Some(start + ms(24)));
    assert!(matches!(
        handle.retire(RetireReason::Control),
        Retired::Pending
    ));
    assert!(
        due(&mut lane, start + ms(5)),
        "a posted retirement is due work"
    );
    lane.pass(start + ms(5), || {});
    assert!(!due(&mut lane, start + ms(100)));
    assert_eq!(lane.next_deadline(), None);
}

/// Table counters reach the process totals in batches and when the table
/// is dropped.
#[test]
fn table_counters_publish_on_flush_and_drop() {
    let log = Arc::new(Mutex::new(Log::default()));
    let start = Instant::now();
    let before = callback_lane_counters();
    let mut lane = ManualLane::new(start, None);
    let _handle = install(&mut lane, Scripted::new(1, start, ms(20), 3, &log), start);
    run(&mut lane, start, start + ms(60), ms(20));
    lane.shut_down();
    let after = callback_lane_counters();
    assert!(after.installs > before.installs);
    assert!(after.fires >= before.fires + 3);
    assert!(after.finished > before.finished);
    assert!(after.wheel_filings >= before.wheel_filings + 3);
    let late_after: u64 = after.late.iter().sum();
    let late_before: u64 = before.late.iter().sum();
    assert!(late_after >= late_before + 3, "every fire is bucketed");
}

// ---- receive entries ----

/// What scripted receive sources saw, shared with the test.
#[derive(Default)]
struct RxLog {
    /// (id, from the ready list)
    serves: Vec<(u32, bool)>,
    left: Vec<u32>,
    dropped: Vec<u32>,
}

type SharedWaker = Arc<Mutex<Option<Waker>>>;

/// A receive source whose calls return `plan` in turn (the last one
/// repeats). It keeps a clone of its waker in `waker` for the test.
struct ScriptedRx {
    id: u32,
    plan: Vec<RxNext>,
    panic_on_serve: bool,
    wake: Option<RxWake>,
    waker: SharedWaker,
    log: Arc<Mutex<RxLog>>,
}

impl ScriptedRx {
    fn new(id: u32, plan: Vec<RxNext>, log: &Arc<Mutex<RxLog>>) -> Self {
        Self {
            id,
            plan,
            panic_on_serve: false,
            wake: None,
            waker: Arc::new(Mutex::new(None)),
            log: Arc::clone(log),
        }
    }

    fn next(&mut self) -> RxNext {
        if self.plan.len() > 1 {
            self.plan.remove(0)
        } else {
            self.plan[0]
        }
    }
}

impl LaneRxSource for ScriptedRx {
    fn attach(&mut self, wake: RxWake) {
        *self.waker.lock().unwrap() = Some(wake.waker.clone());
        self.wake = Some(wake);
    }

    fn serve(&mut self, ready: bool) -> RxNext {
        if ready {
            if let Some(wake) = self.wake.as_ref() {
                wake.begin_serve();
            }
        }
        self.log.lock().unwrap().serves.push((self.id, ready));
        if self.panic_on_serve {
            panic!("scripted receive handler panic");
        }
        let next = self.next();
        if matches!(next, RxNext::More(_)) {
            if let Some(wake) = self.wake.as_ref() {
                wake.waker.wake_by_ref();
            }
        }
        next
    }

    fn on_leave(&mut self) {
        self.log.lock().unwrap().left.push(self.id);
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
        self
    }
}

impl Drop for ScriptedRx {
    fn drop(&mut self) {
        if let Ok(mut log) = self.log.lock() {
            log.dropped.push(self.id);
        }
    }
}

fn install_rx_on(lane: &mut ManualLane, source: ScriptedRx) -> (LaneRxHandle, SharedWaker) {
    let waker = Arc::clone(&source.waker);
    let handle = lane.enter(|| {
        install_rx(Box::new(source))
            .ok()
            .expect("a table is current")
    });
    (handle, waker)
}

fn rx_finished(handle: &mut LaneRxHandle) -> Option<Option<LaneRxReturn>> {
    let waker = futures::task::noop_waker();
    let mut cx = Context::from_waker(&waker);
    let mut finished = Box::pin(handle.finished());
    match finished.as_mut().poll(&mut cx) {
        Poll::Ready(value) => Some(value),
        Poll::Pending => None,
    }
}

fn wake(waker: &SharedWaker) {
    waker
        .lock()
        .unwrap()
        .as_ref()
        .expect("attached")
        .wake_by_ref();
}

/// No lost wakeup, receive side: an installed entry is served in its install
/// pass; a wake between passes marks it once (and the table reports it
/// pending until a pass serves it); a wake during a pass is served before
/// the pass ends; a wake from another thread arrives through the mailbox.
#[test]
fn a_ready_receive_entry_is_served_once_per_mark_and_never_lost() {
    let log = Arc::new(Mutex::new(RxLog::default()));
    let start = Instant::now();
    let mut lane = ManualLane::new(start, None);
    let (mut handle, waker) = install_rx_on(
        &mut lane,
        ScriptedRx::new(1, vec![RxNext::Wait(None)], &log),
    );
    assert!(lane.rx_pending(), "installed: marked ready");
    assert_eq!(
        lane.pass_with_rx(start, || {}),
        (0, 1),
        "served in the install pass"
    );
    assert!(!lane.rx_pending());
    assert_eq!(lane.pass_with_rx(start, || {}), (0, 0), "nothing marked");
    // Two wakes between passes: one mark, one service.
    wake(&waker);
    wake(&waker);
    assert!(lane.rx_pending());
    assert_eq!(lane.pass_with_rx(start, || {}).1, 1);
    assert!(!lane.rx_pending());
    // A wake inside a pass (a readiness edge dispatched mid-pass) is served
    // before the pass ends.
    let (_, calls) = lane.pass_with_rx(start, || wake(&waker));
    assert_eq!(calls, 1);
    // A wake from another thread posts through the mailbox: due work.
    let remote = Arc::clone(&waker);
    std::thread::spawn(move || wake(&remote)).join().unwrap();
    assert!(lane.rx_pending());
    assert!(lane
        .enter(|| with_current(|table| table.has_due(start)))
        .unwrap());
    assert_eq!(lane.pass_with_rx(start, || {}).1, 1);
    assert!(!lane.rx_pending());
    let serves = log.lock().unwrap().serves.clone();
    assert_eq!(serves, vec![(1, true); 4]);
    assert!(rx_finished(&mut handle).is_none(), "still installed");
}

/// A wake between passes wakes the poller (a parked lane comes back for
/// it); a wake during a pass does not (the pass serves it).
#[test]
fn a_receive_wake_between_passes_wakes_the_poller() {
    use std::sync::atomic::AtomicUsize;
    struct Count(AtomicUsize);
    impl std::task::Wake for Count {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let log = Arc::new(Mutex::new(RxLog::default()));
    let start = Instant::now();
    let mut lane = ManualLane::new(start, None);
    let count = Arc::new(Count(AtomicUsize::new(0)));
    let poller = Waker::from(Arc::clone(&count));
    let (_handle, waker) = install_rx_on(
        &mut lane,
        ScriptedRx::new(1, vec![RxNext::Wait(None)], &log),
    );
    lane.pass_with_waker(start, &poller, || {});
    let woken = count.0.load(Ordering::SeqCst);
    lane.pass_with_waker(start, &poller, || wake(&waker));
    assert_eq!(count.0.load(Ordering::SeqCst), woken, "served in the pass");
    wake(&waker);
    assert_eq!(
        count.0.load(Ordering::SeqCst),
        woken + 1,
        "between passes: the poller is woken"
    );
    assert_eq!(lane.pass_with_waker(start, &poller, || {}).1, 1);
}

/// Receive deadlines: a `Wait(deadline)` is filed on the wheel (on the tick
/// grid when there is one) and served when due, never before; a moved
/// deadline is moved, not duplicated.
#[test]
fn receive_deadlines_are_filed_and_served_never_early() {
    let log = Arc::new(Mutex::new(RxLog::default()));
    let start = Instant::now();
    let mut lane = ManualLane::new(start, Some(ms(2)));
    let plan = vec![
        RxNext::Wait(Some(start + ms(5))),
        RxNext::Wait(Some(start + ms(9))),
        RxNext::Wait(None),
    ];
    let (_handle, _waker) = install_rx_on(&mut lane, ScriptedRx::new(1, plan, &log));
    lane.pass(start, || {});
    assert_eq!(lane.next_deadline(), Some(start + ms(6)), "the 2 ms grid");
    run(&mut lane, start + ms(1), start + ms(5), ms(1));
    assert_eq!(
        log.lock().unwrap().serves.len(),
        1,
        "not before the boundary"
    );
    lane.pass(start + ms(6), || {});
    assert_eq!(log.lock().unwrap().serves, vec![(1, true), (1, false)]);
    assert_eq!(lane.next_deadline(), Some(start + ms(10)));
    run(&mut lane, start + ms(7), start + ms(30), ms(1));
    assert_eq!(log.lock().unwrap().serves.len(), 3);
    assert_eq!(lane.next_deadline(), None, "nothing left filed");
    assert_eq!(
        lane.table.as_ref().map(|table| table.wheel_lengths()),
        Some((0, 0))
    );
}

/// Receive budgets: a pass serves at most `CALLBACK_RX_SERVICES_PER_PASS`
/// ready entries; the rest stay ready (first in the next pass) and the pass
/// reports them; an entry that returns `More` is served again in a later
/// pass, after the others.
#[test]
fn receive_services_are_budgeted_per_pass_and_carry_in_order() {
    let log = Arc::new(Mutex::new(RxLog::default()));
    let start = Instant::now();
    let mut lane = ManualLane::new(start, None);
    let count = CALLBACK_RX_SERVICES_PER_PASS + 44;
    let mut handles = Vec::new();
    for id in 0..count {
        handles.push(install_rx_on(
            &mut lane,
            ScriptedRx::new(id, vec![RxNext::Wait(None)], &log),
        ));
    }
    let carried = lane
        .enter(|| {
            with_current(|table| {
                let report = table.serve_ready();
                assert_eq!(report.rx_calls, u64::from(CALLBACK_RX_SERVICES_PER_PASS));
                assert_eq!(report.carried, 44);
                assert!(table.rx_pending());
                table.carried()
            })
        })
        .unwrap();
    assert_eq!(carried, 44);
    assert_eq!(lane.pass_with_rx(start, || {}).1, 44, "the rest, next pass");
    let order: Vec<u32> = log
        .lock()
        .unwrap()
        .serves
        .iter()
        .map(|(id, _)| *id)
        .collect();
    assert_eq!(
        order,
        (0..count).collect::<Vec<_>>(),
        "FIFO across the carry"
    );
    // `More`: served once per pass until it parks.
    log.lock().unwrap().serves.clear();
    let plan = vec![RxNext::More(None), RxNext::More(None), RxNext::Wait(None)];
    let (_greedy, _waker) = install_rx_on(&mut lane, ScriptedRx::new(999, plan, &log));
    assert_eq!(lane.pass_with_rx(start, || {}).1, 1);
    assert!(lane.rx_pending(), "it asked for more");
    assert_eq!(lane.pass_with_rx(start, || {}).1, 1);
    assert_eq!(lane.pass_with_rx(start, || {}).1, 1);
    assert!(!lane.rx_pending());
    assert_eq!(lane.pass_with_rx(start, || {}).1, 0);
}

/// A panicking receive handler poisons only its entry: its state is dropped
/// (no `on_leave`), its supervisor is told, and a neighbour keeps its
/// deadline and its service.
#[test]
fn a_panicking_receive_handler_poisons_only_its_entry() {
    let log = Arc::new(Mutex::new(RxLog::default()));
    let start = Instant::now();
    let mut lane = ManualLane::new(start, None);
    let (mut good_handle, good_waker) = install_rx_on(
        &mut lane,
        ScriptedRx::new(2, vec![RxNext::Wait(Some(start + ms(50)))], &log),
    );
    let mut bad = ScriptedRx::new(3, vec![RxNext::Wait(None)], &log);
    bad.panic_on_serve = true;
    let (mut bad_handle, _bad_waker) = install_rx_on(&mut lane, bad);
    let before = callback_lane_counters();
    assert_eq!(
        lane.pass_with_rx(start, || {}).1,
        2,
        "both served in their install pass"
    );
    assert!(matches!(
        rx_finished(&mut bad_handle),
        Some(Some(LaneRxReturn::Poisoned))
    ));
    assert!(callback_lane_counters().panics > before.panics);
    {
        let log = log.lock().unwrap();
        assert!(log.dropped.contains(&3), "its state was dropped");
        assert!(!log.left.contains(&3), "no on_leave after a panic");
        assert!(!log.dropped.contains(&2));
    }
    assert_eq!(lane.len(), 1);
    assert_eq!(
        lane.next_deadline(),
        Some(start + ms(50)),
        "the neighbour's deadline stays"
    );
    wake(&good_waker);
    assert_eq!(lane.pass_with_rx(start + ms(1), || {}).1, 1);
    assert!(rx_finished(&mut good_handle).is_none());
    lane.shut_down();
    assert!(callback_lane_counters().rx_poisoned > before.rx_poisoned);
}

/// Receive teardown: dropping the supervisor's handle in a pass on its lane
/// retires the entry at once (`on_leave`, state dropped, deadline unfiled);
/// off the lane the retirement is posted and applied before the next pass
/// serves anything; `Stopped` and `Escalate` hand the source back with
/// `on_leave` run; lane shutdown drops the rest.
#[test]
fn receive_entries_leave_by_every_path_and_release_everything() {
    let log = Arc::new(Mutex::new(RxLog::default()));
    let start = Instant::now();
    let mut lane = ManualLane::new(start, None);
    let before = callback_lane_counters();
    let (on_lane, _) = install_rx_on(
        &mut lane,
        ScriptedRx::new(1, vec![RxNext::Wait(Some(start + ms(40)))], &log),
    );
    let (off_lane, off_waker) = install_rx_on(
        &mut lane,
        ScriptedRx::new(2, vec![RxNext::Wait(Some(start + ms(40)))], &log),
    );
    let (mut stops, _) = install_rx_on(&mut lane, ScriptedRx::new(3, vec![RxNext::Stopped], &log));
    let (mut escalates, _) = install_rx_on(
        &mut lane,
        ScriptedRx::new(4, vec![RxNext::Escalate("socket_error")], &log),
    );
    let (mut lane_goes, _) = install_rx_on(
        &mut lane,
        ScriptedRx::new(5, vec![RxNext::Wait(Some(start + ms(40)))], &log),
    );
    lane.pass(start, || {});
    match rx_finished(&mut stops) {
        Some(Some(LaneRxReturn::Stopped(source))) => {
            assert!(source.into_any().downcast::<ScriptedRx>().is_ok());
        }
        _ => panic!("stopped"),
    }
    assert!(matches!(
        rx_finished(&mut escalates),
        Some(Some(LaneRxReturn::Escalated(_, "socket_error")))
    ));
    assert_eq!(lane.len(), 3);
    // On its lane, inside a pass: synchronous.
    lane.pass(start + ms(1), || drop(on_lane));
    assert_eq!(lane.len(), 2);
    // Off the lane: posted; applied before anything is served, even a wake
    // that raced it.
    drop(off_lane);
    wake(&off_waker);
    lane.pass(start + ms(2), || {});
    assert_eq!(lane.len(), 1);
    // Lane shutdown: the source is dropped, its supervisor sees `None`.
    lane.shut_down();
    assert!(matches!(rx_finished(&mut lane_goes), Some(None)));
    let log = log.lock().unwrap();
    let served_2: usize = log.serves.iter().filter(|(id, _)| *id == 2).count();
    assert_eq!(served_2, 1, "not served after its retirement was posted");
    for id in 1..=4 {
        assert!(log.left.contains(&id), "{id} left the table");
    }
    for id in 1..=5 {
        assert!(log.dropped.contains(&id), "{id} was dropped");
    }
    let after = callback_lane_counters();
    assert!(after.rx_installs >= before.rx_installs + 5);
    assert!(after.rx_stopped > before.rx_stopped);
    assert!(after.rx_escalated > before.rx_escalated);
    assert!(after.rx_retired >= before.rx_retired + 2);
}

/// Marks and mailbox posts of a retired entry never reach a later occupant
/// of its slot, and receive and TX entries never retire each other.
#[test]
fn stale_receive_marks_are_fenced_by_generation() {
    let log = Arc::new(Mutex::new(RxLog::default()));
    let start = Instant::now();
    let mut lane = ManualLane::new(start, None);
    let (first, first_waker) = install_rx_on(
        &mut lane,
        ScriptedRx::new(1, vec![RxNext::Wait(None)], &log),
    );
    lane.pass(start, || drop(first));
    let (_second, _) = install_rx_on(
        &mut lane,
        ScriptedRx::new(2, vec![RxNext::Wait(None)], &log),
    );
    lane.pass(start, || {});
    let slots: Vec<(u32, u32)> = lane
        .enter(|| {
            with_current(|table| {
                table
                    .entries
                    .iter()
                    .enumerate()
                    .filter(|(_, entry)| entry.in_use)
                    .map(|(slot, entry)| (slot as u32, entry.generation))
                    .collect()
            })
        })
        .unwrap();
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].0, 0, "the slot was reused");
    log.lock().unwrap().serves.clear();
    // The first occupant's waker (still registered somewhere) fires, on the
    // lane and from another thread: nothing is served.
    wake(&first_waker);
    let remote = Arc::clone(&first_waker);
    std::thread::spawn(move || wake(&remote)).join().unwrap();
    assert_eq!(lane.pass_with_rx(start, || {}).1, 0);
    assert!(log.lock().unwrap().serves.is_empty());
    // A TX retirement for the receive entry's slot is refused.
    let refused = lane
        .enter(|| with_current(|table| table.retire(slots[0].0, slots[0].1, RetireReason::Control)))
        .unwrap();
    assert!(refused.is_none());
    assert_eq!(lane.len(), 1);
}

/// Receive entries cost a table nothing until one is installed: no receive
/// work in a pass, nothing pending, and a TX fire is unchanged.
#[test]
fn without_receive_entries_a_pass_does_no_receive_work() {
    let log = Arc::new(Mutex::new(Log::default()));
    let start = Instant::now();
    let mut lane = ManualLane::new(start, None);
    let _handle = install(&mut lane, Scripted::new(1, start, ms(20), 3, &log), start);
    assert_eq!(lane.pass_with_rx(start, || {}), (1, 0));
    assert!(!lane.rx_pending());
    let seen = lane.enter(|| with_current(|table| table.rx_seen)).unwrap();
    assert!(!seen);
}
