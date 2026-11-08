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
//! # Receive entries
//!
//! With [`MediaExecutorConfig::callback_rx_kernel`] a call's RTP receive
//! loop (`transport::udp`) becomes an entry of its own slot once it owns its
//! call state: the loop's state, socket and owner timer move into a
//! crate-internal receive source, and the lane calls it
//! - from the table's ready list, when a waker it registered fired (its
//!   socket's readiness). The waker marks the slot ready on the lane thread
//!   (a thread-local push) or posts it through the mailbox from another
//!   thread;
//! - from the wheel, at the owner deadline it asked for (what the async
//!   loop's owner timer waited for).
//!
//! Each call runs the async loop's iterations (receive, process, `active`
//! check) until the loop would park, so every per-packet step is the loop's
//! own. A receive error or the stopped transport sends the source back to
//! its supervisor, the loop's future, which ends the loop or continues it
//! asynchronously. Ready entries are served after the woken hosted futures
//! and before the due fires, at most [`CALLBACK_RX_SERVICES_PER_PASS`] per
//! pass; what a budget leaves keeps the lane going. Panic isolation and
//! generation fencing are as for TX entries.
//!
//! With [`MediaExecutorConfig::lane_rx_readiness`] (Linux) a receive entry's
//! socket is registered with the table's level-triggered lane epoll
//! (`crate::lane_epoll`) instead: a pass asks it which sockets hold a
//! datagram and calls each reported entry to read once, asking again until
//! it reports none, at most [`LANE_RX_READINESS_WAITS_PER_PASS`] times per
//! pass. No read finds a socket empty, and nothing parks on the socket.
//!
//! [`MediaExecutorConfig::callback_send`]: crate::executor::MediaExecutorConfig::callback_send
//! [`MediaExecutorConfig::callback_rx_kernel`]: crate::executor::MediaExecutorConfig::callback_rx_kernel
//! [`MediaExecutorConfig::lane_rx_readiness`]: crate::executor::MediaExecutorConfig::lane_rx_readiness

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

pub use crate::executor::{
    callback_rx_kernel_enabled, callback_send_enabled, lane_rx_readiness_enabled,
};

/// Entries one pass fires at most (every timer round together). The rest
/// stay due and the pass asks the poller to continue.
pub const CALLBACK_FIRES_PER_PASS: u32 = 256;
/// Fires one entry gets per pass at most (a source that keeps asking for an
/// instant already due is carried to the next pass).
pub const CALLBACK_FIRES_PER_ENTRY_PER_PASS: u8 = 2;
/// Ready receive entries one pass serves at most (the lane poller's hosted
/// poll budget). The rest stay ready and the pass asks the poller to
/// continue.
pub const CALLBACK_RX_SERVICES_PER_PASS: u32 = 256;
/// Lane epoll waits one pass makes at most
/// ([`MediaExecutorConfig::lane_rx_readiness`]). A socket gets one read per
/// wait that reports it, so this is the per-call drain budget of a pass
/// ([`crate::executor::INLINE_DRAIN_BUDGET`], the async loop's yield point);
/// a socket still reported after it keeps the lane going.
///
/// [`MediaExecutorConfig::lane_rx_readiness`]: crate::executor::MediaExecutorConfig::lane_rx_readiness
pub const LANE_RX_READINESS_WAITS_PER_PASS: u32 = 50;

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

// ---- the receive source API (crate-internal) ----

/// What a receive source wants after a call (applied by the table once the
/// handler returned).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RxNext {
    /// Parked: its wakers are registered. Called again on the next wake, or
    /// once this owner deadline has passed (never earlier; filed on the
    /// lane's tick grid like a lane-clock sleep).
    Wait(Option<Instant>),
    /// Its drain budget ran out with input left: it marked itself ready
    /// again (served in a later pass), and this owner deadline.
    More(Option<Instant>),
    /// The transport stopped (the loop's `active` check): the source goes
    /// back to its supervisor, which ends the loop.
    Stopped,
    /// It needs the async loop (`reason`): it goes back to its supervisor.
    Escalate(&'static str),
}

/// A graduated receive loop driven by the lane table. Runs on its lane
/// thread inside a poller pass; never awaits.
pub(crate) trait LaneRxSource: Send + 'static {
    /// Installed: the waker for every source it parks on.
    fn attach(&mut self, wake: RxWake);
    /// Serve whatever is ready and park. `ready`: called from the ready
    /// list (its waker fired, or it was just installed or carried); else its
    /// owner deadline fell due.
    fn serve(&mut self, ready: bool) -> RxNext;
    /// The socket to register with the lane epoll, for a source that asked
    /// to read on its readiness reports (`None`: it parks on its own wakers).
    /// Asked once, at install; the socket stays open while the source is
    /// installed.
    fn readiness_fd(&self) -> Option<crate::lane_epoll::SocketFd> {
        None
    }
    /// The lane epoll reported the socket readable: read it once, then
    /// continue as [`Self::serve`] does. Called only for a source attached
    /// with [`RxWake::readiness`].
    fn on_readable(&mut self) -> RxNext {
        self.serve(true)
    }
    /// The entry leaves the table (not after a panic): undo [`Self::attach`].
    fn on_leave(&mut self) {}
    /// For the supervisor, which knows the concrete type, to take it back.
    fn into_any(self: Box<Self>) -> Box<dyn Any + Send>;
}

/// A receive entry's waker. On its lane thread a wake marks the entry ready
/// in a thread-local list (no lock) and, between passes, wakes the poller;
/// from another thread (or for a table that does not own this thread's
/// list) it posts through the mailbox, which wakes (and unparks) the lane.
/// Marked once until the entry is served.
struct SlotWake {
    table: u64,
    slot: u32,
    generation: u32,
    queued: AtomicBool,
    mailbox: Arc<Mailbox>,
}

impl std::task::Wake for SlotWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        if self.queued.swap(true, Ordering::AcqRel) {
            return;
        }
        // `Some(in_pass)`: pushed on the table's own thread.
        let local = RX_READY
            .try_with(|ready| {
                let mut ready = ready.try_borrow_mut().ok()?;
                (ready.table == self.table).then(|| {
                    ready.slots.push((self.slot, self.generation));
                    ready.in_pass
                })
            })
            .ok()
            .flatten();
        match local {
            // A pass is running: it serves the list before it ends.
            Some(true) => {}
            Some(false) => self.mailbox.poller.wake(),
            None => self.mailbox.post_ready(self.slot, self.generation),
        }
    }
}

/// What a receive source gets when it is installed.
pub(crate) struct RxWake {
    pub(crate) waker: Waker,
    /// Its socket is registered with the lane epoll: the lane calls
    /// [`LaneRxSource::on_readable`] once per readiness report, and the
    /// source neither reads its socket speculatively nor parks on it.
    /// `false` when it asked for that and the lane could not (counted in
    /// [`CallbackLaneCounters::rx_readiness_fallbacks`]).
    pub(crate) readiness: bool,
    slot: Arc<SlotWake>,
}

impl RxWake {
    /// Served from the ready list: unmark first, so a wake during the
    /// service marks it again (never lost).
    #[inline]
    pub(crate) fn begin_serve(&self) {
        // A read-modify-write: nothing the service reads moves before it.
        self.slot.queued.swap(false, Ordering::AcqRel);
    }
}

/// How an installed receive source came back to its supervisor.
pub(crate) enum LaneRxReturn {
    /// [`RxNext::Stopped`].
    Stopped(Box<dyn LaneRxSource>),
    /// [`RxNext::Escalate`].
    Escalated(Box<dyn LaneRxSource>, &'static str),
    /// A retirement posted from off the lane, completed by the lane.
    Retired(Box<dyn LaneRxSource>),
    /// A handler panicked; the source was dropped.
    Poisoned,
}

/// A receive supervisor's authority over its installed entry.
pub(crate) struct LaneRxHandle {
    slot: u32,
    generation: u32,
    table: u64,
    mailbox: Arc<Mailbox>,
    done: oneshot::Receiver<LaneRxReturn>,
    live: bool,
}

impl LaneRxHandle {
    /// Retire the entry: synchronously on its own lane inside a pass (the
    /// source is returned), otherwise through the mailbox (`None`; the lane
    /// retires it before it serves anything).
    pub(crate) fn retire(&mut self) -> Option<Box<dyn LaneRxSource>> {
        if !self.live {
            return None;
        }
        self.live = false;
        let (slot, generation, table) = (self.slot, self.generation, self.table);
        let local = with_current(|current| {
            (current.id == table).then(|| current.retire_rx(slot, generation))
        })
        .flatten();
        match local {
            Some(source) => source,
            None => {
                self.mailbox
                    .post(slot, generation, RetireReason::SupervisorDropped);
                None
            }
        }
    }

    /// How the source came back. `None`: the lane went away (its table and
    /// the source were dropped).
    pub(crate) async fn finished(&mut self) -> Option<LaneRxReturn> {
        let result = (&mut self.done).await;
        self.live = false;
        result.ok()
    }
}

impl Drop for LaneRxHandle {
    fn drop(&mut self) {
        // The supervisor went away (the loop was aborted, or its lane is
        // shutting down): the entry must not keep receiving without it.
        if let Some(source) = self.retire() {
            drop_quietly_rx(source);
        }
    }
}

/// Install a receive `source` in the table of the lane pass running on this
/// thread; it is served in this pass. `Err` gives it back when there is no
/// table here (the caller keeps its async loop); that is counted in
/// [`CallbackLaneCounters::install_fallbacks`].
pub(crate) fn install_rx(
    source: Box<dyn LaneRxSource>,
) -> Result<LaneRxHandle, Box<dyn LaneRxSource>> {
    let mut source = Some(source);
    let installed = with_current(|table| table.install_rx(source.take().expect("taken once")));
    match installed {
        Some(handle) => Ok(handle),
        None => {
            record_install_fallback();
            Err(source.expect("not installed"))
        }
    }
}

fn drop_quietly_rx(source: Box<dyn LaneRxSource>) {
    if catch_unwind(AssertUnwindSafe(move || drop(source))).is_err() {
        record_rx_panic("drop");
    }
}

fn record_rx_panic(stage: &'static str) {
    PANICS.fetch_add(1, Ordering::Relaxed);
    tracing::error!(
        stage,
        "lane table receive handler panicked; only its call's receive loop ended"
    );
}

/// The waker the lane epoll's reactor registration holds. Like a receive
/// entry's wake between passes, it wakes the poller (whose waker each pass
/// registers with the mailbox).
struct EpollWake {
    mailbox: Arc<Mailbox>,
}

impl std::task::Wake for EpollWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.mailbox.poller.wake();
    }
}

/// A table's lane epoll and its bookkeeping.
struct RxEpoll {
    epoll: crate::lane_epoll::LaneEpoll,
    /// Registered with the reactor for the epoll descriptor.
    waker: Waker,
    /// Wait in this pass even without a reactor report (a socket was just
    /// registered and may hold datagrams already).
    force: bool,
    /// A pass stopped waiting with reports outstanding (its wait or service
    /// budget ran out): the lane must not sleep on them, and the next pass
    /// waits (`force` is set with it).
    pending: bool,
    /// Waits made in this pass ([`LANE_RX_READINESS_WAITS_PER_PASS`]).
    waits: u32,
}

#[cfg(test)]
thread_local! {
    /// Tests: runs on this thread after a lane epoll wait that reported
    /// nothing, before the reactor's readiness is cleared (the window a
    /// datagram arriving between the drain and the re-arm falls into).
    static AFTER_EMPTY_WAIT: RefCell<Option<Box<dyn FnMut()>>> = const { RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_after_empty_wait(hook: Option<Box<dyn FnMut()>>) {
    AFTER_EMPTY_WAIT.with(|slot| *slot.borrow_mut() = hook);
}

#[cfg(test)]
fn run_after_empty_wait() {
    let hook = AFTER_EMPTY_WAIT.with(|slot| slot.borrow_mut().take());
    if let Some(mut hook) = hook {
        hook();
        AFTER_EMPTY_WAIT.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.is_none() {
                *slot = Some(hook);
            }
        });
    }
}

/// Ready receive entries marked on this thread, for the table that owns the
/// list (the first one a receive entry was installed in, normally the lane
/// poller's only table; a test's `ManualLane`).
struct LocalReady {
    /// The owning table (0: none).
    table: u64,
    /// A pass of that table is running (it serves the list before it ends).
    in_pass: bool,
    slots: Vec<(u32, u32)>,
}

thread_local! {
    static RX_READY: RefCell<LocalReady> = const {
        RefCell::new(LocalReady {
            table: 0,
            in_pass: false,
            slots: Vec::new(),
        })
    };
}

// ---- the table ----

/// Off-lane retirements, applied by the lane before it fires anything, and
/// receive entries marked ready from another thread.
pub(crate) struct Mailbox {
    pending: AtomicBool,
    posts: parking_lot::Mutex<Posts>,
    /// The poller's parent waker (its tick waker while ticking), registered
    /// once per pass, so a post wakes (and unparks) the lane.
    poller: futures::task::AtomicWaker,
}

#[derive(Default)]
struct Posts {
    retires: Vec<(u32, u32, RetireReason)>,
    /// (slot, generation) of receive entries woken off the lane.
    ready: Vec<(u32, u32)>,
}

impl Mailbox {
    fn new() -> Self {
        Self {
            pending: AtomicBool::new(false),
            posts: parking_lot::Mutex::new(Posts::default()),
            poller: futures::task::AtomicWaker::new(),
        }
    }

    fn post(&self, slot: u32, generation: u32, reason: RetireReason) {
        self.posts.lock().retires.push((slot, generation, reason));
        self.pending.store(true, Ordering::Release);
        self.poller.wake();
    }

    /// A receive entry's waker fired off its table's thread.
    fn post_ready(&self, slot: u32, generation: u32) {
        self.posts.lock().ready.push((slot, generation));
        self.pending.store(true, Ordering::Release);
        self.poller.wake();
    }

    fn take(&self) -> Option<Posts> {
        if !self.pending.load(Ordering::Acquire) {
            return None;
        }
        self.pending.store(false, Ordering::Release);
        Some(std::mem::take(&mut *self.posts.lock()))
    }
}

/// One part of a call on this lane: its TX source, or its receive loop. The
/// first cache line holds what a fire reads first: the slot's generation and
/// liveness, which kind of entry it is (`rx_entry`), its wheel links and
/// filed key, the per-pass budget stamp and the TX source; a receive entry's
/// source and return are on the second line, so a TX fire reads the first
/// line only (checked at compile time below).
///
/// A call's receive part takes a slot of its own (its owner deadline is
/// filed through that slot's link), next to its TX slot.
#[repr(C, align(64))]
pub(crate) struct CallEntry {
    generation: u32,
    in_use: bool,
    /// Fires this entry got in pass `pass`.
    pass_fires: u8,
    /// A receive entry (on the first line, so a fire tells the two kinds
    /// apart without reading the second).
    rx_entry: bool,
    pass: u32,
    links: WheelLinks,
    /// The filed key: the deadline the source asked for, or (tick grid) the
    /// tick boundary at or after it. `None`: not filed.
    key: Option<Instant>,
    tx: Option<Box<dyn LaneTxSource>>,
    done: Option<oneshot::Sender<LaneTxReturn>>,
    /// A receive entry's source and its supervisor's return.
    rx: Option<Box<dyn LaneRxSource>>,
    rx_done: Option<oneshot::Sender<LaneRxReturn>>,
}

impl CallEntry {
    fn vacant() -> Self {
        Self {
            generation: 0,
            in_use: false,
            pass_fires: 0,
            rx_entry: false,
            pass: 0,
            links: WheelLinks::UNFILED,
            key: None,
            tx: None,
            done: None,
            rx: None,
            rx_done: None,
        }
    }
}

// Everything a TX fire reads is on the entry's first cache line (`rx_entry`
// included), and the receive parts are on the second.
const _: () = {
    use std::mem::{offset_of, size_of};
    assert!(offset_of!(CallEntry, rx_entry) < 64);
    assert!(offset_of!(CallEntry, links) + size_of::<WheelLinks>() <= 64);
    assert!(offset_of!(CallEntry, key) + size_of::<Option<Instant>>() <= 64);
    assert!(offset_of!(CallEntry, tx) + size_of::<Option<Box<dyn LaneTxSource>>>() <= 64);
    assert!(offset_of!(CallEntry, rx) >= 64);
    assert!(offset_of!(CallEntry, rx_done) >= 64);
};

/// The parts a released slot held.
struct Released {
    tx: Option<Box<dyn LaneTxSource>>,
    done: Option<oneshot::Sender<LaneTxReturn>>,
    rx: Option<Box<dyn LaneRxSource>>,
    rx_done: Option<oneshot::Sender<LaneRxReturn>>,
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
    /// Receive handler calls made (ready services and due fires).
    pub(crate) rx_calls: u64,
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
    /// Receive entries to serve, in the order they became ready.
    ready: Vec<(u32, u32)>,
    /// Ready services left this pass, and receive handler calls made (the
    /// pass clock is refreshed every 16, as for hosted polls).
    rx_budget: u32,
    pass_rx_calls: u32,
    /// A receive entry was installed here once. Until then a pass does none
    /// of the receive work (no ready list, no thread-local marks, no in-pass
    /// flag): without receive entries the pass is the TX table's alone.
    rx_seen: bool,
    /// The lane epoll, created at the first registration.
    rx_epoll: Option<Box<RxEpoll>>,
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
            ready: Vec::new(),
            rx_budget: CALLBACK_RX_SERVICES_PER_PASS,
            pass_rx_calls: 0,
            rx_seen: false,
            rx_epoll: None,
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
        self.rx_budget = CALLBACK_RX_SERVICES_PER_PASS;
        self.pass_rx_calls = 0;
        if let Some(epoll) = self.rx_epoll.as_mut() {
            epoll.waits = 0;
        }
        self.mailbox.poller.register(poller);
    }

    fn take_slot(&mut self) -> u32 {
        match self.free.pop() {
            Some(slot) => slot,
            None => {
                self.entries.push(CallEntry::vacant());
                (self.entries.len() - 1) as u32
            }
        }
    }

    fn install(&mut self, source: Box<dyn LaneTxSource>, first: Instant) -> LaneTxHandle {
        let slot = self.take_slot();
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

    /// Install a receive source in a slot of its own. It is marked ready,
    /// so this pass serves it: the loop goes on to its next wait as an
    /// entry.
    fn install_rx(&mut self, mut source: Box<dyn LaneRxSource>) -> LaneRxHandle {
        if !self.rx_seen {
            // From here on passes do the receive work. This thread's ready
            // list becomes this table's unless another table owns it (that
            // table's entries keep it; ours then post through the mailbox).
            self.rx_seen = true;
            let id = self.id;
            let _ = RX_READY.try_with(|ready| {
                if let Ok(mut ready) = ready.try_borrow_mut() {
                    if ready.table == 0 {
                        ready.table = id;
                        // This pass did not mark itself (it had no receive
                        // entry when it began): a wake during it wakes the
                        // poller, which is only redundant.
                        ready.in_pass = false;
                        ready.slots.clear();
                    }
                }
            });
        }
        let slot = self.take_slot();
        let (done_tx, done_rx) = oneshot::channel();
        let generation = self.entries[slot as usize].generation;
        let wake = Arc::new(SlotWake {
            table: self.id,
            slot,
            generation,
            queued: AtomicBool::new(true),
            mailbox: Arc::clone(&self.mailbox),
        });
        // A source that asked for it reads on the lane epoll's reports.
        let readiness = match source.readiness_fd() {
            Some(socket) => self.register_readiness(slot, generation, socket),
            None => false,
        };
        source.attach(RxWake {
            waker: Waker::from(Arc::clone(&wake)),
            readiness,
            slot: wake,
        });
        {
            let entry = &mut self.entries[slot as usize];
            entry.in_use = true;
            entry.pass_fires = 0;
            entry.pass = self.pass.wrapping_sub(1);
            entry.rx_entry = true;
            entry.key = None;
            entry.rx = Some(source);
            entry.rx_done = Some(done_tx);
        }
        self.live += 1;
        self.ready.push((slot, generation));
        self.counters.rx.installs += 1;
        LaneRxHandle {
            slot,
            generation,
            table: self.id,
            mailbox: Arc::clone(&self.mailbox),
            done: done_rx,
            live: true,
        }
    }

    /// Register `socket` (entry `slot`, `generation`) with the lane epoll,
    /// created on first use. `false` (counted) when that is not possible:
    /// the entry then parks on its own wakers.
    fn register_readiness(
        &mut self,
        slot: u32,
        generation: u32,
        socket: crate::lane_epoll::SocketFd,
    ) -> bool {
        if self.rx_epoll.is_none() {
            match crate::lane_epoll::LaneEpoll::new() {
                Ok(epoll) => {
                    self.rx_epoll = Some(Box::new(RxEpoll {
                        epoll,
                        waker: Waker::from(Arc::new(EpollWake {
                            mailbox: Arc::clone(&self.mailbox),
                        })),
                        force: false,
                        pending: false,
                        waits: 0,
                    }));
                }
                Err(error) => {
                    tracing::warn!(%error, "no lane epoll; receive entries park on their sockets");
                    self.counters.rx.readiness_fallbacks += 1;
                    return false;
                }
            }
        }
        let epoll = self.rx_epoll.as_mut().expect("created above");
        match epoll.epoll.add(slot, generation, socket) {
            Ok(()) => {
                // It may hold datagrams already: wait in this pass.
                epoll.force = true;
                self.counters.rx.epoll_registered += 1;
                true
            }
            Err(error) => {
                tracing::warn!(%error, "socket not registered with the lane epoll; the entry parks on it");
                self.counters.rx.readiness_fallbacks += 1;
                false
            }
        }
    }

    /// Free `slot`: the generation moves on, so every handle and message of
    /// the previous occupant is stale.
    fn release(&mut self, slot: u32) -> Released {
        // Unregister its socket while the source (which keeps the socket
        // open) is still in the slot.
        if let Some(epoll) = self.rx_epoll.as_mut() {
            epoll.epoll.remove(slot);
        }
        let entry = &mut self.entries[slot as usize];
        debug_assert!(
            !entry.links.is_filed(),
            "released entries are unfiled first"
        );
        entry.in_use = false;
        entry.generation = entry.generation.wrapping_add(1);
        entry.key = None;
        entry.rx_entry = false;
        let parts = Released {
            tx: entry.tx.take(),
            done: entry.done.take(),
            rx: entry.rx.take(),
            rx_done: entry.rx_done.take(),
        };
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

    /// A live receive entry (first line only).
    fn is_rx(&self, slot: u32, generation: u32) -> bool {
        self.entries
            .get(slot as usize)
            .is_some_and(|entry| entry.in_use && entry.generation == generation && entry.rx_entry)
    }

    /// Retire a live TX entry; `None` when stale. On a panicking
    /// `on_retire` the supervisor is told through `done` and `None` is
    /// returned.
    fn retire(
        &mut self,
        slot: u32,
        generation: u32,
        reason: RetireReason,
    ) -> Option<Box<dyn LaneTxSource>> {
        if !self.is_current(slot, generation) || self.entries[slot as usize].rx_entry {
            return None;
        }
        let key = self.entries[slot as usize].key;
        self.wheel.cancel(&mut self.entries, slot, key);
        let Released { tx, done, .. } = self.release(slot);
        let mut source = tx?;
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

    /// Retire a live receive entry (its supervisor went away); `None` when
    /// stale. On a panicking `on_leave` the supervisor is told through
    /// `rx_done` and `None` is returned.
    fn retire_rx(&mut self, slot: u32, generation: u32) -> Option<Box<dyn LaneRxSource>> {
        if !self.is_rx(slot, generation) {
            return None;
        }
        let key = self.entries[slot as usize].key;
        self.wheel.cancel(&mut self.entries, slot, key);
        let Released { rx, rx_done, .. } = self.release(slot);
        let mut source = rx?;
        if catch_unwind(AssertUnwindSafe(|| source.on_leave())).is_err() {
            record_rx_panic("leave");
            drop_quietly_rx(source);
            send_back_rx(rx_done, LaneRxReturn::Poisoned);
            self.counters.rx.poisoned += 1;
            return None;
        }
        self.counters.rx.retired += 1;
        Some(source)
    }

    /// Off-lane retirements first: a control event posted before the pass
    /// beats every fire of the pass. Receive entries woken off the lane join
    /// the ready list.
    fn drain_mailbox(&mut self) -> u64 {
        let Some(posts) = self.mailbox.take() else {
            return 0;
        };
        let mut returned = 0;
        for (slot, generation, reason) in posts.retires {
            if !self.is_current(slot, generation) {
                continue;
            }
            if self.entries[slot as usize].rx_entry {
                let done = self.entries[slot as usize].rx_done.take();
                match self.retire_rx(slot, generation) {
                    Some(source) => send_back_rx(done, LaneRxReturn::Retired(source)),
                    // Its `on_leave` panicked (the slot was live).
                    None => send_back_rx(done, LaneRxReturn::Poisoned),
                }
            } else {
                let done = self.entries[slot as usize].done.take();
                match self.retire(slot, generation, reason) {
                    Some(source) => send_back(done, Some(LaneTxReturn::Retired(source))),
                    // Its `on_retire` panicked (the slot was live).
                    None => send_back(done, Some(LaneTxReturn::Poisoned)),
                }
            }
            returned += 1;
        }
        self.ready.extend(posts.ready);
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

    /// Receive entries are marked ready and wait for a pass (the lane must
    /// not sleep on them).
    pub(crate) fn rx_pending(&self) -> bool {
        self.rx_seen
            && (!self.ready.is_empty()
                || local_ready_pending(self.id)
                || self.mailbox.pending.load(Ordering::Acquire)
                || self.rx_epoll.as_ref().is_some_and(|epoll| epoll.pending))
    }

    /// Receive handler calls in this pass (ready services and due fires).
    pub(crate) fn pass_rx_calls(&self) -> u64 {
        u64::from(self.pass_rx_calls)
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
            if !entry.rx_entry {
                if let Some(source) = entry.tx.as_ref() {
                    crate::hot_state::prefetch_read(
                        &**source as *const dyn LaneTxSource as *const () as usize,
                    );
                    lines += 1;
                }
            } else if let Some(source) = entry.rx.as_ref() {
                crate::hot_state::prefetch_read(
                    &**source as *const dyn LaneRxSource as *const () as usize,
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
            // Told from the first line (`rx_entry`), not from `rx` on the
            // second, which a TX fire must not touch.
            if self.entries[slot as usize].rx_entry {
                // A receive entry's owner deadline (the async loop's owner
                // timer) fell due.
                report.rx_calls += 1;
                self.note_rx_call();
                self.counters.rx.due_calls += 1;
                let served = {
                    let source = self.entries[slot as usize]
                        .rx
                        .as_mut()
                        .expect("a receive entry has a source");
                    catch_unwind(AssertUnwindSafe(|| source.serve(false)))
                };
                report.returned += self.apply_rx(slot, served, "due");
                continue;
            }
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
                    let Released { tx, done, .. } = self.release(slot);
                    send_back(done, tx.map(LaneTxReturn::Done));
                    self.counters.finished += 1;
                    report.returned += 1;
                }
                Ok(LaneNext::Escalate(reason)) => {
                    let Released { tx, done, .. } = self.release(slot);
                    send_back(
                        done,
                        tx.map(|source| LaneTxReturn::Escalated(source, reason)),
                    );
                    self.counters.escalations += 1;
                    report.returned += 1;
                }
                Err(_panic) => {
                    record_panic("due");
                    let Released { tx, done, .. } = self.release(slot);
                    if let Some(source) = tx {
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

    /// Serve the receive entries marked ready (their wakers fired, or they
    /// were just installed or carried), in the order they were marked, at
    /// most [`CALLBACK_RX_SERVICES_PER_PASS`] per pass. Off-lane retirements
    /// go first, as for fires.
    pub(crate) fn serve_ready(&mut self) -> FireReport {
        if !self.rx_seen {
            // No receive entry was ever installed: nothing to serve (the
            // fires that follow drain the mailbox).
            return FireReport::default();
        }
        let mut report = FireReport {
            returned: self.drain_mailbox(),
            ..FireReport::default()
        };
        self.take_local_ready();
        if !self.ready.is_empty() {
            self.serve_ready_list(&mut report);
        }
        if self.rx_epoll.is_some() {
            self.serve_readiness(&mut report);
        }
        report
    }

    /// Serve the sockets the lane epoll reports readable, one read per
    /// report, asking again until it reports none (then the reactor's
    /// readiness for it is cleared and its waker registered) or the pass's
    /// wait or service budget is spent (then the lane keeps going). Only
    /// when the reactor saw a datagram arrive since the last clear, or a
    /// socket was just registered.
    fn serve_readiness(&mut self, report: &mut FireReport) {
        let Some(state) = self.rx_epoll.as_mut() else {
            return;
        };
        if state.epoll.registered() == 0 {
            state.pending = false;
            state.force = false;
            return;
        }
        let readiness = state.epoll.readiness();
        let waker = state.waker.clone();
        let mut cx = Context::from_waker(&waker);
        loop {
            let guard = match readiness.poll(&mut cx) {
                Poll::Ready(guard) => guard,
                // The waker is registered for the next arrival.
                Poll::Pending => None,
            };
            let state = self.rx_epoll.as_mut().expect("checked above");
            if guard.is_none() && !std::mem::take(&mut state.force) {
                return;
            }
            state.force = false;
            loop {
                let state = self.rx_epoll.as_mut().expect("checked above");
                if state.waits >= LANE_RX_READINESS_WAITS_PER_PASS || self.rx_budget == 0 {
                    // Reports may be outstanding: the lane goes on, and its
                    // next pass waits whatever the reactor says (the guard,
                    // if any, is dropped uncleared).
                    state.pending = true;
                    state.force = true;
                    self.counters.rx.epoll_budget_stops += 1;
                    return;
                }
                state.waits += 1;
                let max = (self.rx_budget as usize).min(crate::lane_epoll::MAX_EVENTS);
                let ready = match state.epoll.wait(max) {
                    Ok(ready) => ready,
                    Err(error) => {
                        // Try again in the next pass (EINTR; nothing else is
                        // expected of a zero-timeout wait on our epoll).
                        tracing::debug!(%error, "lane epoll wait failed");
                        state.pending = true;
                        state.force = true;
                        return;
                    }
                };
                if ready == 0 {
                    state.pending = false;
                    self.counters.rx.epoll_empty_waits += 1;
                    #[cfg(test)]
                    run_after_empty_wait();
                    if let Some(guard) = guard {
                        guard.clear();
                    }
                    // Poll again: registers the waker (or, should the
                    // reactor have reported another edge, waits again).
                    break;
                }
                self.counters.rx.epoll_waits += 1;
                self.counters.rx.epoll_reports += ready as u64;
                for index in 0..ready {
                    let (slot, generation) = self
                        .rx_epoll
                        .as_ref()
                        .expect("checked above")
                        .epoll
                        .report(index);
                    if !self.is_rx(slot, generation) || self.rx_budget == 0 {
                        // It left during this wait's services (its socket is
                        // unregistered now), or the budget is spent (it is
                        // reported again).
                        self.counters.rx.epoll_stale_reports += 1;
                        continue;
                    }
                    self.rx_budget -= 1;
                    report.rx_calls += 1;
                    self.note_rx_call();
                    self.counters.rx.readable_calls += 1;
                    let served = {
                        let source = self.entries[slot as usize]
                            .rx
                            .as_mut()
                            .expect("a receive entry has a source");
                        catch_unwind(AssertUnwindSafe(|| source.on_readable()))
                    };
                    report.returned += self.apply_rx(slot, served, "readable");
                }
            }
        }
    }

    /// The ready list, in order, within the pass's service budget.
    fn serve_ready_list(&mut self, report: &mut FireReport) {
        let mut serving = std::mem::take(&mut self.ready);
        let mut index = 0;
        while index < serving.len() && self.rx_budget > 0 {
            let (slot, generation) = serving[index];
            index += 1;
            if !self.is_rx(slot, generation) {
                // Stale: the entry left (its slot may hold another call).
                continue;
            }
            self.rx_budget -= 1;
            report.rx_calls += 1;
            self.note_rx_call();
            self.counters.rx.ready_calls += 1;
            let served = {
                let source = self.entries[slot as usize]
                    .rx
                    .as_mut()
                    .expect("a receive entry has a source");
                catch_unwind(AssertUnwindSafe(|| source.serve(true)))
            };
            report.returned += self.apply_rx(slot, served, "ready");
        }
        if index < serving.len() {
            // The pass budget left these ready: first in the next pass, and
            // the pass end keeps the lane going.
            let left = (serving.len() - index) as u64;
            report.carried += left;
            self.carried += left;
            self.counters.carried += left;
            serving.drain(..index);
        } else {
            serving.clear();
        }
        // Keep the allocation; entries marked meanwhile go after.
        serving.append(&mut self.ready);
        self.ready = serving;
    }

    /// One receive handler call; the pass reading is refreshed every 16, as
    /// for hosted-future polls.
    #[inline]
    fn note_rx_call(&mut self) {
        self.pass_rx_calls = self.pass_rx_calls.wrapping_add(1);
        if self.pass_rx_calls % crate::lane_clock::PASS_CLOCK_REFRESH_POLLS == 0 {
            crate::lane_clock::pass_invalidate();
        }
    }

    /// Move this thread's marks for this table to the ready list.
    fn take_local_ready(&mut self) {
        let id = self.id;
        let ready = &mut self.ready;
        let _ = RX_READY.try_with(|local| {
            if let Ok(mut local) = local.try_borrow_mut() {
                if local.table == id && !local.slots.is_empty() {
                    ready.append(&mut local.slots);
                }
            }
        });
    }

    /// Apply what a receive handler returned (after it returned, so a
    /// handler never holds the wheel). 1 when the source went back to its
    /// supervisor.
    fn apply_rx(
        &mut self,
        slot: u32,
        served: std::thread::Result<RxNext>,
        stage: &'static str,
    ) -> u64 {
        match served {
            Ok(RxNext::Wait(deadline)) | Ok(RxNext::More(deadline)) => {
                self.refile(slot, deadline);
                0
            }
            Ok(RxNext::Stopped) => {
                self.leave_rx(slot, false, LaneRxReturn::Stopped);
                1
            }
            Ok(RxNext::Escalate(reason)) => {
                self.leave_rx(slot, true, |source| LaneRxReturn::Escalated(source, reason));
                1
            }
            Err(_panic) => {
                // Poisoned: only this call's receive loop ends. Its state is
                // dropped (quietly), its supervisor is told, the wheel and
                // the other entries are untouched.
                record_rx_panic(stage);
                let key = self.entries[slot as usize].key;
                self.wheel.cancel(&mut self.entries, slot, key);
                let Released { rx, rx_done, .. } = self.release(slot);
                if let Some(source) = rx {
                    drop_quietly_rx(source);
                }
                send_back_rx(rx_done, LaneRxReturn::Poisoned);
                self.counters.rx.poisoned += 1;
                1
            }
        }
    }

    /// A receive entry leaves the table and goes back to its supervisor
    /// (`escalated`: for its async loop, else because it stopped).
    fn leave_rx(
        &mut self,
        slot: u32,
        escalated: bool,
        back: impl FnOnce(Box<dyn LaneRxSource>) -> LaneRxReturn,
    ) {
        let key = self.entries[slot as usize].key;
        self.wheel.cancel(&mut self.entries, slot, key);
        let Released { rx, rx_done, .. } = self.release(slot);
        let Some(mut source) = rx else {
            return;
        };
        if catch_unwind(AssertUnwindSafe(|| source.on_leave())).is_err() {
            record_rx_panic("leave");
            drop_quietly_rx(source);
            send_back_rx(rx_done, LaneRxReturn::Poisoned);
            self.counters.rx.poisoned += 1;
            return;
        }
        if escalated {
            self.counters.rx.escalated += 1;
        } else {
            self.counters.rx.stopped += 1;
        }
        send_back_rx(rx_done, back(source));
    }

    /// File a receive entry's owner deadline (`None`: none), moving it only
    /// when it changed.
    fn refile(&mut self, slot: u32, deadline: Option<Instant>) {
        let key = deadline.map(|deadline| self.filed_at(deadline));
        let entry = &self.entries[slot as usize];
        if entry.key == key && (key.is_none() || entry.links.is_filed()) {
            return;
        }
        let (old, generation) = (entry.key, entry.generation);
        self.wheel.cancel(&mut self.entries, slot, old);
        self.entries[slot as usize].key = key;
        if let Some(key) = key {
            self.wheel.file(&mut self.entries, slot, generation, key);
        }
        // A far deadline moved again leaves a stale overflow item behind.
        self.wheel.compact_if_needed(&self.entries);
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
            // A receive source drops its loop's state (socket, SRTP context)
            // here; its supervisor sees the lane go.
            if let Some(source) = entry.rx.take() {
                drop_quietly_rx(source);
            }
            entry.rx_done = None;
        }
        let id = self.id;
        let _ = RX_READY.try_with(|ready| {
            if let Ok(mut ready) = ready.try_borrow_mut() {
                if ready.table == id {
                    ready.table = 0;
                    ready.in_pass = false;
                    ready.slots.clear();
                }
            }
        });
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

/// Hand a receive source back to its supervisor (dropped quietly when
/// nobody awaits it any more).
fn send_back_rx(done: Option<oneshot::Sender<LaneRxReturn>>, value: LaneRxReturn) {
    let value = match done {
        Some(done) => match done.send(value) {
            Ok(()) => return,
            Err(value) => value,
        },
        None => value,
    };
    match value {
        LaneRxReturn::Stopped(source)
        | LaneRxReturn::Escalated(source, _)
        | LaneRxReturn::Retired(source) => drop_quietly_rx(source),
        LaneRxReturn::Poisoned => {}
    }
}

/// Whether this thread holds receive marks for `table`.
fn local_ready_pending(table: u64) -> bool {
    RX_READY
        .try_with(|ready| {
            ready
                .try_borrow()
                .is_ok_and(|ready| ready.table == table && !ready.slots.is_empty())
        })
        .unwrap_or(false)
}

/// A pass of `table` runs on this thread (receive wakes then leave the
/// poller alone: the pass serves them before it ends).
fn set_in_pass(table: u64, in_pass: bool) {
    let _ = RX_READY.try_with(|ready| {
        if let Ok(mut ready) = ready.try_borrow_mut() {
            if ready.table == table {
                ready.in_pass = in_pass;
            }
        }
    });
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
    let id = table.id;
    // Only a table that has hosted receive entries marks its passes
    // (receive wakes during a pass then leave the poller alone).
    let in_pass = table.rx_seen;
    if in_pass {
        set_in_pass(id, true);
    }
    let previous = CURRENT.with(|current| current.borrow_mut().replace(table));
    Some(TableGuard {
        slot,
        previous,
        id,
        in_pass,
    })
}

/// Gives a lent table back to its poller (see [`enter`]).
pub(crate) struct TableGuard<'a> {
    slot: &'a mut Option<Box<LaneTable>>,
    previous: Option<Box<LaneTable>>,
    id: u64,
    /// The in-pass flag was set for this pass (and is cleared on drop).
    in_pass: bool,
}

impl Drop for TableGuard<'_> {
    fn drop(&mut self) {
        if self.in_pass {
            set_in_pass(self.id, false);
        }
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
static RX_INSTALLS: AtomicU64 = AtomicU64::new(0);
static RX_READY_CALLS: AtomicU64 = AtomicU64::new(0);
static RX_DUE_CALLS: AtomicU64 = AtomicU64::new(0);
static RX_STOPPED: AtomicU64 = AtomicU64::new(0);
static RX_RETIRED: AtomicU64 = AtomicU64::new(0);
static RX_ESCALATED: AtomicU64 = AtomicU64::new(0);
static RX_POISONED: AtomicU64 = AtomicU64::new(0);
static RX_READABLE_CALLS: AtomicU64 = AtomicU64::new(0);
static RX_READINESS_FALLBACKS: AtomicU64 = AtomicU64::new(0);
static RX_EPOLL_REGISTERED: AtomicU64 = AtomicU64::new(0);
static RX_EPOLL_WAITS: AtomicU64 = AtomicU64::new(0);
static RX_EPOLL_EMPTY_WAITS: AtomicU64 = AtomicU64::new(0);
static RX_EPOLL_REPORTS: AtomicU64 = AtomicU64::new(0);
static RX_EPOLL_STALE_REPORTS: AtomicU64 = AtomicU64::new(0);
static RX_EPOLL_BUDGET_STOPS: AtomicU64 = AtomicU64::new(0);

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
    /// Receive loops installed in a lane table
    /// ([`MediaExecutorConfig::callback_rx_kernel`]).
    ///
    /// [`MediaExecutorConfig::callback_rx_kernel`]: crate::executor::MediaExecutorConfig::callback_rx_kernel
    pub rx_installs: u64,
    /// Receive handler calls from the ready list (a socket became readable,
    /// or the entry was installed or carried).
    pub rx_ready_calls: u64,
    /// Receive handler calls from the wheel (an owner deadline fell due).
    pub rx_due_calls: u64,
    /// Receive entries that left because their transport stopped.
    pub rx_stopped: u64,
    /// Receive entries retired by their supervisor (the loop was aborted or
    /// its future dropped).
    pub rx_retired: u64,
    /// Receive entries handed back to their async loop (a receive error).
    pub rx_escalated: u64,
    /// Receive entries whose handler panicked (only that loop ended).
    pub rx_poisoned: u64,
    /// Receive handler calls for a lane epoll readiness report
    /// ([`MediaExecutorConfig::lane_rx_readiness`]); each reads once.
    ///
    /// [`MediaExecutorConfig::lane_rx_readiness`]: crate::executor::MediaExecutorConfig::lane_rx_readiness
    pub rx_readable_calls: u64,
    /// Receive entries that asked for the lane epoll and park on their
    /// sockets instead (no epoll, or the socket could not be registered).
    pub rx_readiness_fallbacks: u64,
    /// Sockets registered with a lane epoll.
    pub rx_epoll_registered: u64,
    /// Lane epoll waits that reported sockets.
    pub rx_epoll_waits: u64,
    /// Lane epoll waits that reported none (at most one per pass that read:
    /// the only empty probe on this path).
    pub rx_epoll_empty_waits: u64,
    /// Sockets reported readable.
    pub rx_epoll_reports: u64,
    /// Reports for an entry that had left (or past the service budget).
    pub rx_epoll_stale_reports: u64,
    /// Passes that stopped waiting with reports outstanding (a budget).
    pub rx_epoll_budget_stops: u64,
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
        rx_installs: load(&RX_INSTALLS),
        rx_ready_calls: load(&RX_READY_CALLS),
        rx_due_calls: load(&RX_DUE_CALLS),
        rx_stopped: load(&RX_STOPPED),
        rx_retired: load(&RX_RETIRED),
        rx_escalated: load(&RX_ESCALATED),
        rx_poisoned: load(&RX_POISONED),
        rx_readable_calls: load(&RX_READABLE_CALLS),
        rx_readiness_fallbacks: load(&RX_READINESS_FALLBACKS),
        rx_epoll_registered: load(&RX_EPOLL_REGISTERED),
        rx_epoll_waits: load(&RX_EPOLL_WAITS),
        rx_epoll_empty_waits: load(&RX_EPOLL_EMPTY_WAITS),
        rx_epoll_reports: load(&RX_EPOLL_REPORTS),
        rx_epoll_stale_reports: load(&RX_EPOLL_STALE_REPORTS),
        rx_epoll_budget_stops: load(&RX_EPOLL_BUDGET_STOPS),
    }
}

/// A table's receive entry counts (published with [`TableCounters`]).
#[derive(Default)]
struct RxTableCounters {
    installs: u64,
    ready_calls: u64,
    due_calls: u64,
    stopped: u64,
    retired: u64,
    escalated: u64,
    poisoned: u64,
    readable_calls: u64,
    readiness_fallbacks: u64,
    epoll_registered: u64,
    epoll_waits: u64,
    epoll_empty_waits: u64,
    epoll_reports: u64,
    epoll_stale_reports: u64,
    epoll_budget_stops: u64,
}

impl RxTableCounters {
    fn flush(&mut self) {
        for (pending, total) in [
            (&mut self.installs, &RX_INSTALLS),
            (&mut self.ready_calls, &RX_READY_CALLS),
            (&mut self.due_calls, &RX_DUE_CALLS),
            (&mut self.stopped, &RX_STOPPED),
            (&mut self.retired, &RX_RETIRED),
            (&mut self.escalated, &RX_ESCALATED),
            (&mut self.poisoned, &RX_POISONED),
            (&mut self.readable_calls, &RX_READABLE_CALLS),
            (&mut self.readiness_fallbacks, &RX_READINESS_FALLBACKS),
            (&mut self.epoll_registered, &RX_EPOLL_REGISTERED),
            (&mut self.epoll_waits, &RX_EPOLL_WAITS),
            (&mut self.epoll_empty_waits, &RX_EPOLL_EMPTY_WAITS),
            (&mut self.epoll_reports, &RX_EPOLL_REPORTS),
            (&mut self.epoll_stale_reports, &RX_EPOLL_STALE_REPORTS),
            (&mut self.epoll_budget_stops, &RX_EPOLL_BUDGET_STOPS),
        ] {
            let value = std::mem::take(pending);
            if value > 0 {
                total.fetch_add(value, Ordering::Relaxed);
            }
        }
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
    rx: RxTableCounters,
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
        self.rx.flush();
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
    /// retire work there), then the ready receive entries are served and
    /// the due entries fire, the way the poller orders a round. Returns the
    /// TX handler calls made.
    pub fn pass(&mut self, now: Instant, poll: impl FnOnce()) -> u64 {
        self.pass_with_waker(now, &futures::task::noop_waker(), poll)
            .0
    }

    /// [`Self::pass`], also returning the receive handler calls made in the
    /// pass (ready services and due fires).
    #[cfg(test)]
    pub(crate) fn pass_with_rx(&mut self, now: Instant, poll: impl FnOnce()) -> (u64, u64) {
        self.pass_with_waker(now, &futures::task::noop_waker(), poll)
    }

    /// [`Self::pass_with_rx`] with `poller` as the poller's waker (what a
    /// receive mark between passes or a mailbox post wakes).
    pub(crate) fn pass_with_waker(
        &mut self,
        now: Instant,
        poller: &Waker,
        poll: impl FnOnce(),
    ) -> (u64, u64) {
        let calls = {
            let _guard = enter(&mut self.table, poller);
            poll();
            with_current(|table| {
                table.serve_ready();
                let fires = table.fire_due(now).fires;
                (fires, table.pass_rx_calls())
            })
            .unwrap_or((0, 0))
        };
        if let Some(table) = self.table.as_mut() {
            table.end_pass();
        }
        calls
    }

    /// Receive entries are marked ready (a pass would serve them).
    #[cfg(test)]
    pub(crate) fn rx_pending(&self) -> bool {
        self.table.as_ref().is_some_and(|table| table.rx_pending())
    }

    /// Sockets registered with this lane's epoll.
    #[cfg(test)]
    pub(crate) fn rx_readiness_registered(&self) -> usize {
        self.table
            .as_ref()
            .and_then(|table| table.rx_epoll.as_ref())
            .map_or(0, |epoll| epoll.epoll.registered())
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
