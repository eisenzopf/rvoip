//! The lane table: per-call TX sources fired from their lane's own timing
//! wheel ([`MediaExecutorConfig::callback_send`]).
//!
//! Under executor v2 a paced per-call sender (a recording or announcement
//! player, a tone generator, any loop that sends one frame every 20 ms) is
//! normally a future hosted on its media lane's poller: each frame the lane
//! clock wakes it through a `Waker`, the poller re-polls it through its
//! budget, abort and panic wrappers, and its `select!` re-polls every
//! control branch and re-arms its sleeps. On a dense lane that executor
//! plumbing, not the media work, is a large share of the CPU per call
//! (mostly cache misses on those lines).
//!
//! Here the lane owns a table of call entries instead. An entry holds a
//! synchronous [`LaneTxSource`] (the application's pacer, cursor and
//! writer) and is linked directly into a lane-owned timing wheel (the lane
//! clock's wheel, threaded through the table's slab, no mutex). When its
//! deadline is due the poller calls [`LaneTxSource::on_due`] from the wheel:
//! no waker, no hosted-future poll, no `select!` re-registration. The source
//! returns what it wants next ([`LaneNext`]) and the table applies it after
//! the call returns, so a handler never holds a borrow of the wheel.
//!
//! # Implementing a source
//!
//! Async stays at the edges. Each entry has one **supervisor**: a future the
//! application hosts on the call's media lane with
//! [`spawn_on_media_lane`](crate::executor::spawn_on_media_lane) (on the
//! runtime of the call's transport, e.g.
//! [`RtpSendHandle::media_runtime_handle`](crate::session::RtpSendHandle::media_runtime_handle)).
//! From its first poll, on the lane thread, it calls [`install_tx`]; then it
//! waits only on the call's control events (cancellation, end of input) and
//! on [`LaneTxHandle::finished`]. It is never woken per frame.
//!
//! - [`install_tx`] gives the source back where there is no table (the
//!   switch is off, the executor is v1, the lane is not a dedicated
//!   current-thread lane, or the supervisor is not polled by a lane poller):
//!   the supervisor then drives the source itself as an ordinary paced loop.
//! - A source that needs to await (its synchronous writer deferred frames)
//!   returns [`LaneNext::Escalate`]; it comes back to the supervisor
//!   ([`LaneTxReturn::Escalated`]), which runs the async path and installs
//!   it again.
//! - A supervisor that sees a terminal control event calls
//!   [`LaneTxHandle::retire`]; dropping the handle retires the entry too.
//! - [`<dyn LaneTxSource>::downcast`](trait.LaneTxSource.html#method.downcast)
//!   takes a returned source back as its concrete type.
//!
//! ```no_run
//! use std::time::Duration;
//!
//! use rvoip_rtp_core::lane_table::{self, LaneNext, LaneTxReturn, LaneTxSource};
//! use rvoip_rtp_core::session::{RtpFrameNow, RtpSendHandle};
//! use tokio::time::Instant;
//!
//! /// Sends one 20 ms G.711 frame per fire.
//! struct Pacer {
//!     sender: RtpSendHandle,
//!     next: Instant,
//!     timestamp: u32,
//!     left: u32,
//! }
//!
//! impl LaneTxSource for Pacer {
//!     fn on_due(&mut self, _now: Instant) -> LaneNext {
//!         let payload = [0xffu8; 160];
//!         let frame = RtpFrameNow {
//!             timestamp: self.timestamp,
//!             payload: &payload,
//!             marker: false,
//!             payload_type: 0,
//!         };
//!         match self.sender.try_send_now(&[frame], false) {
//!             Ok(report) if report.sent == 1 => {}
//!             // Busy writer or full socket: let the supervisor await it.
//!             _ => return LaneNext::Escalate(lane_table::Escalation::SendDeferred),
//!         }
//!         self.timestamp = self.timestamp.wrapping_add(160);
//!         self.left -= 1;
//!         if self.left == 0 {
//!             return LaneNext::Done;
//!         }
//!         self.next += Duration::from_millis(20);
//!         LaneNext::At(self.next)
//!     }
//! }
//!
//! /// The supervisor, hosted on the call's lane with `spawn_on_media_lane`.
//! async fn supervise(mut pacer: Box<Pacer>) {
//!     loop {
//!         let first = pacer.next;
//!         let mut handle = match lane_table::install_tx(pacer, first) {
//!             Ok(handle) => handle,
//!             Err(_source) => return, // no table here: drive it as a paced loop
//!         };
//!         match handle.finished().await {
//!             Some(LaneTxReturn::Escalated(source, _)) => {
//!                 pacer = source.downcast::<Pacer>().ok().expect("our source");
//!                 // ... send the deferred frame with `send_packet_with_pt` ...
//!             }
//!             _ => return, // done, retired, poisoned, or the lane went away
//!         }
//!     }
//! }
//! ```
//!
//! # Guarantees
//!
//! - **Never early, on the lane's grid.** An entry fires once the pass
//!   reading has reached its filed key: its deadline, or (lane tick grid
//!   timers) the tick boundary at or after it, as a lane-clock sleep does.
//! - **Control beats a same-instant frame.** In each timer round the poller
//!   polls the hosted futures that were woken (supervisors included)
//!   **before** it fires the table's due entries, and a supervisor that
//!   observed a terminal event retires its entry synchronously. A retirement
//!   posted from another thread is applied before the next fire.
//! - **Budgets.** A pass fires at most [`CALLBACK_FIRES_PER_PASS`] entries
//!   and an entry at most [`CALLBACK_FIRES_PER_ENTRY_PER_PASS`] times; what
//!   is left stays due and the poller continues instead of sleeping. A
//!   source bounds its own work per fire.
//! - **Panic isolation.** Every handler call runs under `catch_unwind`. A
//!   panic poisons only its entry: the source is dropped (its own drop under
//!   a second `catch_unwind`), the slot is freed (generation bumped) and the
//!   supervisor is told ([`LaneTxReturn::Poisoned`]).
//! - **Generation fencing.** A freed slot's generation moves on, so no stale
//!   handle, retirement or wheel item reaches a later occupant.
//! - **Teardown.** When the lane goes, every source is dropped and every
//!   supervisor sees [`LaneTxHandle::finished`] resolve to `None`.
//!
//! [`MediaExecutorConfig::callback_send`]: crate::executor::MediaExecutorConfig::callback_send

use std::any::Any;
use std::cell::RefCell;
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use tokio::sync::oneshot;
use tokio::time::{Instant, Sleep};

use crate::lane_clock::{Wheel, WheelEntry, WheelLinks, WHEEL_RESOLUTION};

pub use crate::executor::callback_send_enabled;

/// Entries one pass fires at most (every timer round together). The rest
/// stay due and the pass asks the poller to continue.
pub const CALLBACK_FIRES_PER_PASS: u32 = 256;
/// Fires one entry gets per pass at most (a source that keeps asking for an
/// instant already due is carried to the next pass).
pub const CALLBACK_FIRES_PER_ENTRY_PER_PASS: u8 = 2;

// ---- the source API ----

/// What a source wants after a fire (applied by the table once the handler
/// returned).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LaneNext {
    /// Fire again once this instant has passed (never earlier; filed on the
    /// lane's tick grid like a lane-clock sleep).
    At(Instant),
    /// The source finished: it goes back to its supervisor
    /// ([`LaneTxReturn::Done`]).
    Done,
    /// The source needs its async driver: it goes back to its supervisor
    /// ([`LaneTxReturn::Escalated`]), which installs it again afterwards.
    Escalate(Escalation),
}

/// Why a source left the lane for its async driver.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Escalation {
    /// The synchronous writer handled fewer frames than were due (a busy
    /// writer, a socket that would block, a frame it cannot send
    /// synchronously); the rest take the awaited send, in order.
    SendDeferred,
}

impl Escalation {
    /// A stable lower-case name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SendDeferred => "send_deferred",
        }
    }
}

/// Why an entry is retired before it finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RetireReason {
    /// The supervisor saw a terminal control event (cancellation, end of
    /// input).
    Control,
    /// The supervisor went away while the entry was installed (its
    /// [`LaneTxHandle`] was dropped).
    SupervisorDropped,
}

/// A synchronous per-call TX source fired by the lane table. Runs on its
/// lane thread inside a poller pass; see the [module docs](self).
pub trait LaneTxSource: Any + Send {
    /// The instant the source asked for has passed: do the due work
    /// (bounded, never awaiting, never blocking) and say what comes next.
    /// `now` is the pass reading the table judged the deadline by.
    fn on_due(&mut self, now: Instant) -> LaneNext;

    /// The entry is retired before it finished. No I/O; the source goes back
    /// to its supervisor afterwards (or is dropped).
    fn on_retire(&mut self, _reason: RetireReason) {}
}

impl dyn LaneTxSource {
    /// Take a source the table gave back as its concrete type; `Err` gives
    /// it back unchanged when it is not a `T`.
    pub fn downcast<T: LaneTxSource>(self: Box<Self>) -> Result<Box<T>, Box<dyn LaneTxSource>> {
        let any: &dyn Any = &*self;
        if !any.is::<T>() {
            return Err(self);
        }
        let any: Box<dyn Any> = self;
        match any.downcast::<T>() {
            Ok(source) => Ok(source),
            Err(_) => unreachable!("the type was checked above"),
        }
    }
}

/// How an installed source came back to its supervisor.
#[non_exhaustive]
pub enum LaneTxReturn {
    /// It returned [`LaneNext::Done`].
    Done(Box<dyn LaneTxSource>),
    /// It returned [`LaneNext::Escalate`].
    Escalated(Box<dyn LaneTxSource>, Escalation),
    /// A retirement posted from off the lane, completed by the lane.
    Retired(Box<dyn LaneTxSource>),
    /// A handler panicked; the source was dropped.
    Poisoned,
}

impl std::fmt::Debug for LaneTxReturn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Done(_) => f.write_str("Done(..)"),
            Self::Escalated(_, reason) => write!(f, "Escalated(.., {reason:?})"),
            Self::Retired(_) => f.write_str("Retired(..)"),
            Self::Poisoned => f.write_str("Poisoned"),
        }
    }
}

/// The result of [`LaneTxHandle::retire`].
pub enum Retired {
    /// Retired now; here is the source.
    Now(Box<dyn LaneTxSource>),
    /// The entry already left the table (finished, escalated or poisoned in
    /// an earlier step) or the retirement was posted from off the lane:
    /// [`LaneTxHandle::finished`] says how it came back.
    Pending,
}

impl std::fmt::Debug for Retired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Now(_) => f.write_str("Now(..)"),
            Self::Pending => f.write_str("Pending"),
        }
    }
}

/// A supervisor's authority over its installed entry.
pub struct LaneTxHandle {
    slot: u32,
    generation: u32,
    table: u64,
    mailbox: Arc<Mailbox>,
    done: oneshot::Receiver<LaneTxReturn>,
    /// The entry may still be in the table (not yet returned or retired).
    live: bool,
}

impl std::fmt::Debug for LaneTxHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LaneTxHandle")
            .field("slot", &self.slot)
            .field("generation", &self.generation)
            .field("live", &self.live)
            .finish_non_exhaustive()
    }
}

impl LaneTxHandle {
    /// Retire the entry. On its own lane, inside a pass, this is
    /// synchronous: the source is back before the next fire of the pass.
    /// Elsewhere the lane retires it at its next pass and the source comes
    /// back through [`Self::finished`] ([`LaneTxReturn::Retired`]).
    pub fn retire(&mut self, reason: RetireReason) -> Retired {
        if !self.live {
            return Retired::Pending;
        }
        let (slot, generation, table) = (self.slot, self.generation, self.table);
        let local = with_current(|current| {
            (current.id == table).then(|| current.retire(slot, generation, reason))
        })
        .flatten();
        self.live = false;
        match local {
            Some(Some(source)) => Retired::Now(source),
            // Stale: it already came back through `done`.
            Some(None) => Retired::Pending,
            None => {
                // Off the lane: the lane retires it at its next pass.
                self.mailbox.post(slot, generation, reason);
                Retired::Pending
            }
        }
    }

    /// How the source came back. `None`: the lane went away (its table and
    /// the source were dropped), or the entry was retired with
    /// [`Retired::Now`].
    pub fn poll_finished(&mut self, cx: &mut Context<'_>) -> Poll<Option<LaneTxReturn>> {
        Pin::new(&mut self.done).poll(cx).map(|result| {
            self.live = false;
            result.ok()
        })
    }

    /// [`Self::poll_finished`] as a future.
    pub async fn finished(&mut self) -> Option<LaneTxReturn> {
        std::future::poll_fn(|cx| self.poll_finished(cx)).await
    }
}

impl Drop for LaneTxHandle {
    fn drop(&mut self) {
        if self.live {
            // The supervisor went away (its lane is shutting down, or it was
            // dropped): no entry may keep sending without one.
            if let Retired::Now(source) = self.retire(RetireReason::SupervisorDropped) {
                drop_quietly(source);
            }
        }
    }
}

/// Install `source` in the table of the lane pass running on this thread,
/// first due at `first`. `Err` gives the source back when there is no table
/// here (the caller drives it itself); that is counted in
/// [`CallbackLaneCounters::install_fallbacks`].
pub fn install_tx(
    source: Box<dyn LaneTxSource>,
    first: Instant,
) -> Result<LaneTxHandle, Box<dyn LaneTxSource>> {
    let mut source = Some(source);
    let installed = with_current(|table| table.install(source.take().expect("taken once"), first));
    match installed {
        Some(handle) => Ok(handle),
        None => {
            record_install_fallback();
            Err(source.expect("not installed"))
        }
    }
}

/// Count a source the application drives with its async path because it
/// could not be installed for a reason of its own ([`install_tx`] counts
/// its own refusals). Control-plane rate.
pub fn record_install_fallback() {
    INSTALL_FALLBACKS.fetch_add(1, Ordering::Relaxed);
}

/// Count an escalated burst completed by its supervisor, after which the
/// source was installed again (`resumed`) or driven by its async path from
/// then on. Control-plane rate.
pub fn record_resume(resumed: bool) {
    if resumed {
        RESUMES.fetch_add(1, Ordering::Relaxed);
    } else {
        RESUMES_ASYNC.fetch_add(1, Ordering::Relaxed);
    }
}

fn drop_quietly(source: Box<dyn LaneTxSource>) {
    if catch_unwind(AssertUnwindSafe(move || drop(source))).is_err() {
        record_panic("drop");
    }
}

fn record_panic(stage: &'static str) {
    PANICS.fetch_add(1, Ordering::Relaxed);
    tracing::error!(stage, "lane table handler panicked; only its call ended");
}

// ---- the table ----

/// Off-lane retirements, applied by the lane before it fires anything.
pub(crate) struct Mailbox {
    pending: AtomicBool,
    retires: parking_lot::Mutex<Vec<(u32, u32, RetireReason)>>,
    /// The poller's parent waker (its tick waker while ticking), registered
    /// once per pass, so a post wakes (and unparks) the lane.
    poller: futures::task::AtomicWaker,
}

impl Mailbox {
    fn new() -> Self {
        Self {
            pending: AtomicBool::new(false),
            retires: parking_lot::Mutex::new(Vec::new()),
            poller: futures::task::AtomicWaker::new(),
        }
    }

    fn post(&self, slot: u32, generation: u32, reason: RetireReason) {
        self.retires.lock().push((slot, generation, reason));
        self.pending.store(true, Ordering::Release);
        self.poller.wake();
    }

    fn take(&self) -> Option<Vec<(u32, u32, RetireReason)>> {
        if !self.pending.load(Ordering::Acquire) {
            return None;
        }
        self.pending.store(false, Ordering::Release);
        Some(std::mem::take(&mut *self.retires.lock()))
    }
}

/// One call on this lane (its TX source). The first cache line holds what a
/// fire reads first: the slot's generation and liveness, its wheel links and
/// filed key, and the per-pass budget stamp.
///
/// A later receive stage adds the call's receive part beside `tx`, with its
/// own deadline filed through a second link.
#[repr(C, align(64))]
pub(crate) struct CallEntry {
    generation: u32,
    in_use: bool,
    /// Fires this entry got in pass `pass`.
    pass_fires: u8,
    pass: u32,
    links: WheelLinks,
    /// The filed key: the deadline the source asked for, or (tick grid) the
    /// tick boundary at or after it. `None`: not filed.
    key: Option<Instant>,
    tx: Option<Box<dyn LaneTxSource>>,
    done: Option<oneshot::Sender<LaneTxReturn>>,
}

impl CallEntry {
    fn vacant() -> Self {
        Self {
            generation: 0,
            in_use: false,
            pass_fires: 0,
            pass: 0,
            links: WheelLinks::UNFILED,
            key: None,
            tx: None,
            done: None,
        }
    }
}

impl WheelEntry for CallEntry {
    #[inline]
    fn links(&self) -> &WheelLinks {
        &self.links
    }

    #[inline]
    fn links_mut(&mut self) -> &mut WheelLinks {
        &mut self.links
    }

    #[inline]
    fn filed_key(&self) -> Option<Instant> {
        self.key
    }

    #[inline]
    fn generation(&self) -> u32 {
        self.generation
    }

    #[inline]
    fn in_use(&self) -> bool {
        self.in_use
    }
}

/// What one [`LaneTable::fire_due`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FireReport {
    /// Handler calls made.
    pub(crate) fires: u64,
    /// Due entries left due by a budget (the pass must continue).
    pub(crate) carried: u64,
    /// Sources sent back to their supervisors (finished, escalated,
    /// poisoned or retired from the mailbox): those supervisors were woken
    /// and the pass serves them in another round.
    pub(crate) returned: u64,
    /// Hot call state: cache lines prefetched before the first fire.
    pub(crate) prefetched: u64,
}

static NEXT_TABLE_ID: AtomicU64 = AtomicU64::new(1);

/// The call entries of one lane poller (see the module docs).
pub(crate) struct LaneTable {
    id: u64,
    entries: Vec<CallEntry>,
    free: Vec<u32>,
    live: usize,
    wheel: Box<Wheel>,
    grid: Option<crate::lane_tick::TickGrid>,
    /// The due list being served (swapped with the wheel's scratch).
    serving: Vec<(Instant, u32)>,
    /// Pass stamp for the per-entry budget, and fires left this pass.
    pass: u32,
    pass_budget: u32,
    /// Entries a budget left due this pass.
    carried: u64,
    mailbox: Arc<Mailbox>,
    /// Hot call state: prefetch every due entry and its source before the
    /// first call of a pass.
    prefetch: bool,
    /// The runtime timer at the earliest key while the lane does not tick.
    timer: Option<Pin<Box<Sleep>>>,
    armed: Option<Instant>,
    counters: TableCounters,
}

impl LaneTable {
    /// A table filing on `grid` (lane tick grid timers) or at exact
    /// instants, on a wheel with the lane clock's origin and bucket width.
    pub(crate) fn new(grid: Option<crate::lane_tick::TickGrid>) -> Self {
        Self::with_origin(grid, Instant::now())
    }

    pub(crate) fn with_origin(grid: Option<crate::lane_tick::TickGrid>, origin: Instant) -> Self {
        let (origin, resolution) = match grid {
            Some(grid) => (Instant::from_std(grid.origin()), grid.quantum()),
            None => (origin, WHEEL_RESOLUTION),
        };
        Self {
            id: NEXT_TABLE_ID.fetch_add(1, Ordering::Relaxed),
            entries: Vec::new(),
            free: Vec::new(),
            live: 0,
            wheel: Box::new(Wheel::new(origin, resolution)),
            grid,
            serving: Vec::new(),
            pass: 0,
            pass_budget: CALLBACK_FIRES_PER_PASS,
            carried: 0,
            mailbox: Arc::new(Mailbox::new()),
            prefetch: crate::executor::hot_state_enabled(),
            timer: None,
            armed: None,
            counters: TableCounters::default(),
        }
    }

    /// Entries installed.
    pub(crate) fn len(&self) -> usize {
        self.live
    }

    fn filed_at(&self, deadline: Instant) -> Instant {
        match self.grid {
            Some(grid) => Instant::from_std(grid.ceil(deadline.into_std())),
            None => deadline,
        }
    }

    /// A pass starts: reset the budgets, note the poller's waker.
    pub(crate) fn begin_pass(&mut self, poller: &Waker) {
        self.pass = self.pass.wrapping_add(1);
        self.pass_budget = CALLBACK_FIRES_PER_PASS;
        self.carried = 0;
        self.mailbox.poller.register(poller);
    }

    fn install(&mut self, source: Box<dyn LaneTxSource>, first: Instant) -> LaneTxHandle {
        let slot = match self.free.pop() {
            Some(slot) => slot,
            None => {
                self.entries.push(CallEntry::vacant());
                (self.entries.len() - 1) as u32
            }
        };
        let (done_tx, done_rx) = oneshot::channel();
        let key = self.filed_at(first);
        let generation = {
            let entry = &mut self.entries[slot as usize];
            entry.in_use = true;
            entry.pass_fires = 0;
            entry.pass = self.pass.wrapping_sub(1);
            entry.key = Some(key);
            entry.tx = Some(source);
            entry.done = Some(done_tx);
            entry.generation
        };
        self.wheel.file(&mut self.entries, slot, generation, key);
        self.live += 1;
        self.counters.installs += 1;
        LaneTxHandle {
            slot,
            generation,
            table: self.id,
            mailbox: Arc::clone(&self.mailbox),
            done: done_rx,
            live: true,
        }
    }

    /// Free `slot`: the generation moves on, so every handle and message of
    /// the previous occupant is stale.
    #[allow(clippy::type_complexity)]
    fn release(
        &mut self,
        slot: u32,
    ) -> (
        Option<Box<dyn LaneTxSource>>,
        Option<oneshot::Sender<LaneTxReturn>>,
    ) {
        let entry = &mut self.entries[slot as usize];
        debug_assert!(
            !entry.links.is_filed(),
            "released entries are unfiled first"
        );
        entry.in_use = false;
        entry.generation = entry.generation.wrapping_add(1);
        entry.key = None;
        let parts = (entry.tx.take(), entry.done.take());
        self.free.push(slot);
        self.live -= 1;
        // A retired far deadline leaves a stale overflow item behind.
        self.wheel.compact_if_needed(&self.entries);
        parts
    }

    fn is_current(&self, slot: u32, generation: u32) -> bool {
        self.entries
            .get(slot as usize)
            .is_some_and(|entry| entry.in_use && entry.generation == generation)
    }

    /// Retire a live entry; `None` when stale. On a panicking `on_retire`
    /// the supervisor is told through `done` and `None` is returned.
    fn retire(
        &mut self,
        slot: u32,
        generation: u32,
        reason: RetireReason,
    ) -> Option<Box<dyn LaneTxSource>> {
        if !self.is_current(slot, generation) {
            return None;
        }
        let key = self.entries[slot as usize].key;
        self.wheel.cancel(&mut self.entries, slot, key);
        let (source, done) = self.release(slot);
        let mut source = source?;
        self.counters.retired += 1;
        if catch_unwind(AssertUnwindSafe(|| source.on_retire(reason))).is_err() {
            record_panic("retire");
            drop_quietly(source);
            if let Some(done) = done {
                let _ = done.send(LaneTxReturn::Poisoned);
            }
            return None;
        }
        Some(source)
    }

    /// Off-lane retirements first: a control event posted before the pass
    /// beats every fire of the pass.
    fn drain_mailbox(&mut self) -> u64 {
        let Some(retires) = self.mailbox.take() else {
            return 0;
        };
        let mut returned = 0;
        for (slot, generation, reason) in retires {
            if !self.is_current(slot, generation) {
                continue;
            }
            let done = self.entries[slot as usize].done.take();
            match self.retire(slot, generation, reason) {
                Some(source) => send_back(done, Some(LaneTxReturn::Retired(source))),
                // Its `on_retire` panicked (the slot was live).
                None => send_back(done, Some(LaneTxReturn::Poisoned)),
            }
            returned += 1;
        }
        returned
    }

    /// Whether an entry is due at `now` (a fresh reading before the lane
    /// decides to sleep), or an off-lane retirement waits.
    pub(crate) fn has_due(&mut self, now: Instant) -> bool {
        self.mailbox.pending.load(Ordering::Acquire)
            || self
                .wheel
                .next_deadline(&self.entries)
                .is_some_and(|key| key <= now)
    }

    /// The earliest filed key.
    pub(crate) fn next_deadline(&mut self) -> Option<Instant> {
        self.wheel.next_deadline(&self.entries)
    }

    /// Entries a budget left due in this pass.
    pub(crate) fn carried(&self) -> u64 {
        self.carried
    }

    /// Hot call state: issue a prefetch for the first cache line of every
    /// entry about to fire and of its source, all before the first handler
    /// call, so their misses overlap instead of stalling one fire each.
    /// Returns (and counts) the lines prefetched.
    fn prefetch_serving(&self) -> u64 {
        let mut lines = 0;
        for &(_, slot) in &self.serving {
            let entry = &self.entries[slot as usize];
            crate::hot_state::prefetch_read(entry as *const CallEntry as usize);
            lines += 1;
            if let Some(source) = entry.tx.as_ref() {
                crate::hot_state::prefetch_read(
                    &**source as *const dyn LaneTxSource as *const () as usize,
                );
                lines += 1;
            }
        }
        crate::hot_state::record_hot_prefetches(lines);
        lines
    }

    /// Fire every entry due at `now`, in (key, slot) order, within the
    /// budgets.
    pub(crate) fn fire_due(&mut self, now: Instant) -> FireReport {
        let mut report = FireReport {
            returned: self.drain_mailbox(),
            ..FireReport::default()
        };
        self.wheel.due.clear();
        self.wheel.collect_due(&mut self.entries, now);
        if self.wheel.due.is_empty() {
            return report;
        }
        // The heap's pop order, as the lane clock wakes its sleeps.
        self.wheel.due.sort_unstable();
        std::mem::swap(&mut self.serving, &mut self.wheel.due);
        if self.prefetch {
            report.prefetched = self.prefetch_serving();
        }
        let mut index = 0;
        while index < self.serving.len() {
            let (key, slot) = self.serving[index];
            index += 1;
            let pass = self.pass;
            let over_entry_budget = {
                let entry = &mut self.entries[slot as usize];
                entry.key = None;
                if entry.pass != pass {
                    entry.pass = pass;
                    entry.pass_fires = 0;
                }
                entry.pass_fires >= CALLBACK_FIRES_PER_ENTRY_PER_PASS
            };
            if self.pass_budget == 0 || over_entry_budget {
                // Left due: filed again at its key (the current bucket).
                let generation = self.entries[slot as usize].generation;
                self.entries[slot as usize].key = Some(key);
                self.wheel.file(&mut self.entries, slot, generation, key);
                report.carried += 1;
                continue;
            }
            self.pass_budget -= 1;
            self.entries[slot as usize].pass_fires += 1;
            self.counters.fired(now.saturating_duration_since(key));
            report.fires += 1;
            let fired = {
                let source = self.entries[slot as usize]
                    .tx
                    .as_mut()
                    .expect("a filed entry has a source");
                catch_unwind(AssertUnwindSafe(|| source.on_due(now)))
            };
            match fired {
                Ok(LaneNext::At(deadline)) => {
                    let key = self.filed_at(deadline);
                    let generation = self.entries[slot as usize].generation;
                    self.entries[slot as usize].key = Some(key);
                    self.wheel.file(&mut self.entries, slot, generation, key);
                }
                Ok(LaneNext::Done) => {
                    let (source, done) = self.release(slot);
                    send_back(done, source.map(LaneTxReturn::Done));
                    self.counters.finished += 1;
                    report.returned += 1;
                }
                Ok(LaneNext::Escalate(reason)) => {
                    let (source, done) = self.release(slot);
                    send_back(
                        done,
                        source.map(|source| LaneTxReturn::Escalated(source, reason)),
                    );
                    self.counters.escalations += 1;
                    report.returned += 1;
                }
                Err(_panic) => {
                    record_panic("due");
                    let (source, done) = self.release(slot);
                    if let Some(source) = source {
                        drop_quietly(source);
                    }
                    if let Some(done) = done {
                        let _ = done.send(LaneTxReturn::Poisoned);
                    }
                    report.returned += 1;
                }
            }
        }
        self.serving.clear();
        std::mem::swap(&mut self.serving, &mut self.wheel.due);
        self.carried += report.carried;
        self.counters.carried += report.carried;
        report
    }

    /// Point the runtime timer at the earliest key and poll it (while the
    /// lane does not tick). `Ready`: it expired; fire again.
    pub(crate) fn poll_timer(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        let Some(next) = self.next_deadline() else {
            return Poll::Pending;
        };
        if self.armed != Some(next) {
            match self.timer.as_mut() {
                Some(sleep) => sleep.as_mut().reset(next),
                None => self.timer = Some(Box::pin(tokio::time::sleep_until(next))),
            }
            self.armed = Some(next);
        }
        let sleep = self.timer.as_mut().expect("armed above");
        match sleep.as_mut().poll(cx) {
            Poll::Ready(()) => {
                self.armed = None;
                Poll::Ready(())
            }
            Poll::Pending => Poll::Pending,
        }
    }

    /// One poller pass ended (counters are published in batches).
    pub(crate) fn end_pass(&mut self) {
        let ops = self.wheel.take_ops();
        self.counters.pass(ops.inserts + ops.far_inserts);
    }

    #[cfg(test)]
    pub(crate) fn wheel_lengths(&self) -> (usize, usize) {
        self.wheel.lengths()
    }
}

impl Drop for LaneTable {
    fn drop(&mut self) {
        // Lane shutdown: every source is dropped (quietly) and every
        // supervisor sees its lane go (`done` dropped).
        self.wheel.clear(&mut self.entries);
        for entry in self.entries.iter_mut() {
            if let Some(source) = entry.tx.take() {
                drop_quietly(source);
            }
            entry.done = None;
        }
        self.counters.flush();
    }
}

fn send_back(done: Option<oneshot::Sender<LaneTxReturn>>, value: Option<LaneTxReturn>) {
    let (Some(done), Some(value)) = (done, value) else {
        return;
    };
    if let Err(value) = done.send(value) {
        // The supervisor is gone: nothing awaits the source.
        match value {
            LaneTxReturn::Done(source)
            | LaneTxReturn::Escalated(source, _)
            | LaneTxReturn::Retired(source) => drop_quietly(source),
            LaneTxReturn::Poisoned => {}
        }
    }
}

// ---- the current table ----

thread_local! {
    /// The table of the lane pass running on this thread, lent by its
    /// poller for the pass.
    static CURRENT: RefCell<Option<Box<LaneTable>>> = const { RefCell::new(None) };
}

/// Run `f` on the table of the pass running on this thread. `None` when
/// there is none, or when it is in use (a handler is running: re-entry is
/// treated as being off the lane).
pub(crate) fn with_current<R>(f: impl FnOnce(&mut LaneTable) -> R) -> Option<R> {
    CURRENT.with(|current| {
        let mut current = current.try_borrow_mut().ok()?;
        current.as_deref_mut().map(f)
    })
}

/// Lend the poller's table to this thread for one pass. The guard gives it
/// back (and restores whatever was current before).
pub(crate) fn enter<'a>(
    slot: &'a mut Option<Box<LaneTable>>,
    poller: &Waker,
) -> Option<TableGuard<'a>> {
    let mut table = slot.take()?;
    table.begin_pass(poller);
    let previous = CURRENT.with(|current| current.borrow_mut().replace(table));
    Some(TableGuard { slot, previous })
}

/// Gives a lent table back to its poller (see [`enter`]).
pub(crate) struct TableGuard<'a> {
    slot: &'a mut Option<Box<LaneTable>>,
    previous: Option<Box<LaneTable>>,
}

impl Drop for TableGuard<'_> {
    fn drop(&mut self) {
        let previous = self.previous.take();
        *self.slot =
            CURRENT.with(|current| std::mem::replace(&mut *current.borrow_mut(), previous));
    }
}

/// Whether the lane poller starting on this thread keeps a table: the
/// switch is active (it needs the lane clock and its timing wheel, passed
/// as the poller decided them) and the thread is a dedicated current-thread
/// media lane. Counted either way; called once per poller.
pub(crate) fn table_for_current_lane(lane_timer: bool, wheel: bool) -> bool {
    if !(lane_timer && wheel && callback_send_enabled()) {
        return false;
    }
    let supported = crate::task_runtime::current_media_lane().is_some()
        && tokio::runtime::Handle::try_current().is_ok_and(|handle| {
            matches!(
                handle.runtime_flavor(),
                tokio::runtime::RuntimeFlavor::CurrentThread
            )
        });
    if supported {
        TABLE_LANES.fetch_add(1, Ordering::Relaxed);
    } else {
        UNSUPPORTED_LANES.fetch_add(1, Ordering::Relaxed);
        static LOGGED: AtomicBool = AtomicBool::new(false);
        if !LOGGED.swap(true, Ordering::Relaxed) {
            tracing::info!(
                "lane table configured, but media lanes are not dedicated current-thread lanes; \
                 per-call TX sources stay paced futures there"
            );
        }
    }
    supported
}

// ---- counters ----

/// Slots of [`CallbackLaneCounters::late`]: one per bucket of
/// [`crate::lane_tick::LATENESS_BUCKETS_US`] plus the open bucket.
pub const LATENESS_SLOTS: usize = crate::lane_tick::LATENESS_BUCKETS_US.len() + 1;

static INSTALLS: AtomicU64 = AtomicU64::new(0);
static FIRES: AtomicU64 = AtomicU64::new(0);
static FINISHED: AtomicU64 = AtomicU64::new(0);
static RETIRED: AtomicU64 = AtomicU64::new(0);
static ESCALATIONS: AtomicU64 = AtomicU64::new(0);
static CARRIED: AtomicU64 = AtomicU64::new(0);
static WHEEL_FILINGS: AtomicU64 = AtomicU64::new(0);
static INSTALL_FALLBACKS: AtomicU64 = AtomicU64::new(0);
static RESUMES: AtomicU64 = AtomicU64::new(0);
static RESUMES_ASYNC: AtomicU64 = AtomicU64::new(0);
static PANICS: AtomicU64 = AtomicU64::new(0);
static TABLE_LANES: AtomicU64 = AtomicU64::new(0);
static UNSUPPORTED_LANES: AtomicU64 = AtomicU64::new(0);
static LATE: [AtomicU64; LATENESS_SLOTS] = [const { AtomicU64::new(0) }; LATENESS_SLOTS];
static LATE_SUM_US: AtomicU64 = AtomicU64::new(0);

/// Lane table totals for this process since start. Each field is
/// individually atomic; the set is not a transaction. Table counters are
/// published by each table every 64 passes and when it is dropped.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct CallbackLaneCounters {
    /// Sources installed in a lane table.
    pub installs: u64,
    /// Handler calls from the wheel.
    pub fires: u64,
    /// Entries that finished on the lane ([`LaneNext::Done`]).
    pub finished: u64,
    /// Entries retired by their supervisor (on or off the lane).
    pub retired: u64,
    /// Entries handed to their async driver ([`LaneNext::Escalate`]).
    pub escalations: u64,
    /// Due entries a pass or entry budget left for the next pass.
    pub carried: u64,
    /// Deadlines filed in table wheels (installs, re-arms, carries).
    pub wheel_filings: u64,
    /// Sources driven by their async path because no table took them
    /// (counted by [`install_tx`] and [`record_install_fallback`]).
    pub install_fallbacks: u64,
    /// Escalated sources installed again by their supervisor
    /// ([`record_resume`]`(true)`).
    pub resumes: u64,
    /// Escalated sources their supervisor kept on the async path
    /// ([`record_resume`]`(false)`).
    pub resumes_async: u64,
    /// Handler panics (each poisoned one entry).
    pub panics: u64,
    /// Lane pollers that started with a table.
    pub table_lanes: u64,
    /// Lane pollers with the switch active that could not keep a table (the
    /// lane is not a dedicated current-thread lane).
    pub unsupported_lanes: u64,
    /// Fire lateness (pass reading minus filed key) per bucket of
    /// [`crate::lane_tick::LATENESS_BUCKETS_US`] plus the open bucket.
    pub late: [u64; LATENESS_SLOTS],
    /// Sum of the fire lateness, in microseconds.
    pub late_sum_us: u64,
}

/// The lane table totals ([`CallbackLaneCounters`]).
pub fn callback_lane_counters() -> CallbackLaneCounters {
    let load = |total: &AtomicU64| total.load(Ordering::Relaxed);
    CallbackLaneCounters {
        installs: load(&INSTALLS),
        fires: load(&FIRES),
        finished: load(&FINISHED),
        retired: load(&RETIRED),
        escalations: load(&ESCALATIONS),
        carried: load(&CARRIED),
        wheel_filings: load(&WHEEL_FILINGS),
        install_fallbacks: load(&INSTALL_FALLBACKS),
        resumes: load(&RESUMES),
        resumes_async: load(&RESUMES_ASYNC),
        panics: load(&PANICS),
        table_lanes: load(&TABLE_LANES),
        unsupported_lanes: load(&UNSUPPORTED_LANES),
        late: std::array::from_fn(|slot| load(&LATE[slot])),
        late_sum_us: load(&LATE_SUM_US),
    }
}

/// Table counters, kept by the table and published every 64 passes (no
/// shared cache line per fire).
#[derive(Default)]
struct TableCounters {
    passes: u64,
    installs: u64,
    fires: u64,
    finished: u64,
    retired: u64,
    escalations: u64,
    carried: u64,
    filings: u64,
    late: [u64; LATENESS_SLOTS],
    late_sum_us: u64,
}

impl TableCounters {
    const FLUSH_EVERY: u64 = 64;

    #[inline]
    fn fired(&mut self, late: std::time::Duration) {
        self.fires += 1;
        self.late[crate::lane_tick::lateness_bucket(late)] += 1;
        self.late_sum_us += u64::try_from(late.as_micros()).unwrap_or(u64::MAX);
    }

    #[inline]
    fn pass(&mut self, filings: u64) {
        self.filings += filings;
        self.passes += 1;
        if self.passes >= Self::FLUSH_EVERY {
            self.flush();
        }
    }

    fn flush(&mut self) {
        self.passes = 0;
        for (pending, total) in [
            (&mut self.installs, &INSTALLS),
            (&mut self.fires, &FIRES),
            (&mut self.finished, &FINISHED),
            (&mut self.retired, &RETIRED),
            (&mut self.escalations, &ESCALATIONS),
            (&mut self.carried, &CARRIED),
            (&mut self.filings, &WHEEL_FILINGS),
            (&mut self.late_sum_us, &LATE_SUM_US),
        ] {
            let value = std::mem::take(pending);
            if value > 0 {
                total.fetch_add(value, Ordering::Relaxed);
            }
        }
        for (pending, total) in self.late.iter_mut().zip(LATE.iter()) {
            let value = std::mem::take(pending);
            if value > 0 {
                total.fetch_add(value, Ordering::Relaxed);
            }
        }
    }
}

// ---- test support ----

/// A lane without a poller, for deterministic tests in this crate and in
/// applications that implement [`LaneTxSource`]: the caller owns the clock
/// and the order of a pass (poll the woken futures, then fire the table),
/// exactly as the lane poller orders them.
#[doc(hidden)]
pub struct ManualLane {
    table: Option<Box<LaneTable>>,
}

#[doc(hidden)]
impl ManualLane {
    /// A table filing at exact instants, or on a tick grid of `grid`
    /// quantum with its origin at `origin`.
    pub fn new(origin: Instant, grid: Option<std::time::Duration>) -> Self {
        let grid = grid.map(|quantum| crate::lane_tick::TickGrid::new(origin.into_std(), quantum));
        Self {
            table: Some(Box::new(LaneTable::with_origin(grid, origin))),
        }
    }

    /// One pass at `now`: `poll` runs with the table current (install and
    /// retire work there), then the due entries fire, the way the poller
    /// orders a round. Returns the handler calls made.
    pub fn pass(&mut self, now: Instant, poll: impl FnOnce()) -> u64 {
        let waker = futures::task::noop_waker();
        let fires = {
            let _guard = enter(&mut self.table, &waker);
            poll();
            with_current(|table| table.fire_due(now).fires).unwrap_or(0)
        };
        if let Some(table) = self.table.as_mut() {
            table.end_pass();
        }
        fires
    }

    /// Run `f` with the table current (no fire).
    pub fn enter<R>(&mut self, f: impl FnOnce() -> R) -> R {
        let waker = futures::task::noop_waker();
        let _guard = enter(&mut self.table, &waker);
        f()
    }

    /// The earliest filed key.
    pub fn next_deadline(&mut self) -> Option<Instant> {
        self.table.as_mut().and_then(|table| table.next_deadline())
    }

    /// Entries installed.
    pub fn len(&self) -> usize {
        self.table.as_ref().map_or(0, |table| table.len())
    }

    /// No entry installed.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Lane shutdown: the table and every installed source go.
    pub fn shut_down(&mut self) {
        self.table = None;
    }
}

#[cfg(test)]
mod rtp_tests;
#[cfg(test)]
mod tests;
