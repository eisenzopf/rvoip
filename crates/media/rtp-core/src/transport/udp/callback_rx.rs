//! A graduated RTP receive loop as a lane-table entry
//! ([`MediaExecutorConfig::callback_rx_kernel`]).
//!
//! Under v2 a call's receive loop is a future hosted on its lane's poller.
//! Every datagram used to cost a waker, a re-poll through the poller's
//! budget, abort and panic wrappers, and a `select!` that re-registered every
//! branch (the socket and the owner timer's lane-clock sleep). Here, once
//! the loop owns its call state, that state ([`RxCore`] and its socket) moves
//! into an [`RxCall`] installed in the lane table, and the lane calls it:
//! - **Socket readiness.** The socket's readiness wakes the entry's
//!   [`RxWake`]: one mark on the lane's ready list, served by
//!   [`RxCall::serve`], which reads until the socket is empty (one batch per
//!   readiness edge) and registers again.
//! - **Owner deadline.** The inline ingress's next deadline (the async
//!   loop's owner timer) is filed on the table's wheel and served by
//!   [`RxCall::serve`] when due.
//!
//! Every call runs the async loop's iterations, unchanged in order and
//! effect: [`InlineRx::next_wake_now`] is `next_wake` up to the point where
//! it would park, [`RxCore::on_datagram`] is the loop body, and the loop's
//! `active` check follows each wake. Where the async loop parks, the entry
//! polls the same sources once with its own waker, in the same biased order,
//! and returns. The loop's future stays, parked on the entry's return: it is
//! the supervisor for control (`stop_receiver` aborts it, which retires the
//! entry synchronously on its lane, so `join` still resolves after the
//! loop's state is dropped) and for the rare async path (a receive error:
//! the entry comes back, the loop reports it, pauses 10 ms as it always did
//! and graduates again).
//!
//! [`MediaExecutorConfig::callback_rx_kernel`]: crate::executor::MediaExecutorConfig::callback_rx_kernel

use super::*;
use crate::lane_table::{self, LaneRxReturn, LaneRxSource, RxNext, RxWake};
use std::any::Any;
use std::task::{Context, Poll};

/// Steps one call makes at most. The drain budget stops a busy socket well
/// before this; it only bounds an owner deadline that keeps falling due.
const STEPS_PER_SERVICE: usize = 4 * INLINE_DRAIN_BUDGET;

/// Whether one receive loop may graduate: only a v2 loop hosted on its
/// lane's poller, with the switch on. Read once per loop, when the
/// transport starts receiving.
pub(super) fn switch_for_loop(on_lane: bool) -> bool {
    on_lane && crate::executor::callback_rx_kernel_enabled()
}

/// What one step of an entry found ([`InlineRx::next_wake_now`]).
pub(super) enum NowWake {
    /// What `InlineRx::next_wake` would have returned.
    Wake(InlineWake),
    /// The drain budget is used up: the async loop yields here.
    Yield,
    /// Nothing ready: the entry's waker is registered where the async wait
    /// would have parked.
    Parked,
}

impl InlineRx {
    /// [`Self::next_wake`] for a lane-table entry. The same steps in the
    /// same order (drain budget, then the socket); then, where the async
    /// wait would park, its branches are polled once with the entry's waker
    /// `cx`, in its biased order: the socket, then the owner timer (due by
    /// the pass reading, never early).
    pub(super) fn next_wake_now(
        &mut self,
        socket: &UdpSocket,
        buffer: &mut [u8],
        cx: &mut Context<'_>,
    ) -> NowWake {
        if self.drained >= INLINE_DRAIN_BUDGET {
            return NowWake::Yield;
        }
        match socket.try_recv_from(buffer) {
            Ok(received) => {
                self.drained += 1;
                return NowWake::Wake(InlineWake::Received(Ok(received)));
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => note_empty_read(),
            Err(error) => {
                self.drained = 0;
                return NowWake::Wake(InlineWake::Received(Err(error)));
            }
        }
        self.drained = 0;
        // Park: the async wait's branches, once, with the entry's waker.
        let mut read = tokio::io::ReadBuf::new(buffer);
        match socket.poll_recv_from(cx, &mut read) {
            Poll::Ready(Ok(source)) => {
                let size = read.filled().len();
                self.drained = 1;
                return NowWake::Wake(InlineWake::Received(Ok((size, source))));
            }
            Poll::Ready(Err(error)) => {
                self.drained = 1;
                return NowWake::Wake(InlineWake::Received(Err(error)));
            }
            Poll::Pending => {}
        }
        // The owner timer, last (the `select!` is biased).
        if self.timer.is_due(crate::media_clock::now_std()) {
            self.on_timer();
            return NowWake::Wake(InlineWake::Timer);
        }
        NowWake::Parked
    }
}

/// A graduated receive loop: its call state and socket, driven by the lane
/// table (see the module docs).
pub(super) struct RxCall {
    core: RxCore,
    socket: Arc<UdpSocket>,
    /// Set while installed.
    wake: Option<RxWake>,
    /// The previous call stopped at the drain budget (the async loop's
    /// yield); the next one continues after it.
    yielded: bool,
    /// A receive error for the async loop to report.
    error: Option<std::io::Error>,
}

impl RxCall {
    pub(super) fn new(core: RxCore, socket: Arc<UdpSocket>) -> Self {
        Self {
            core,
            socket,
            wake: None,
            yielded: false,
            error: None,
        }
    }

    /// Back to the async loop: its state and the receive error to report
    /// (if that is why it came back).
    pub(super) fn into_loop(self) -> (RxCore, Option<std::io::Error>) {
        (self.core, self.error)
    }

    /// After a budget stop: what the async loop does after its yield (a due
    /// owner deadline is served, then the loop's `active` check).
    fn resume(&mut self) -> Option<RxNext> {
        if !std::mem::take(&mut self.yielded) {
            return None;
        }
        let served = self
            .core
            .inline
            .as_mut()
            .is_some_and(|rx| rx.resume_after_yield());
        (served && self.core.stopped()).then_some(RxNext::Stopped)
    }

    /// The async loop's iterations from its next wait on, until it would
    /// park (or yield, or leave).
    fn run(&mut self) -> RxNext {
        let RxCall {
            core,
            socket,
            wake,
            yielded,
            error,
        } = self;
        let Some(wake) = wake.as_ref() else {
            return RxNext::Escalate("detached");
        };
        let mut cx = Context::from_waker(&wake.waker);
        for _ in 0..STEPS_PER_SERVICE {
            let Some(rx) = core.inline.as_mut() else {
                return RxNext::Escalate("no_inline");
            };
            match rx.next_wake_now(socket, &mut core.recv_storage, &mut cx) {
                NowWake::Wake(InlineWake::Received(Ok((size, addr)))) => {
                    core.on_datagram(size, addr);
                }
                NowWake::Wake(InlineWake::Received(Err(e))) => {
                    *error = Some(e);
                    return RxNext::Escalate("socket_error");
                }
                NowWake::Wake(InlineWake::Timer) => {}
                NowWake::Yield => {
                    // Served again in a later pass (the async loop's
                    // `yield_now`), after its neighbours.
                    *yielded = true;
                    wake.waker.wake_by_ref();
                    return RxNext::More(deadline_of(core));
                }
                NowWake::Parked => return RxNext::Wait(deadline_of(core)),
            }
            // The loop's top: its `active` check (hot call state: gated by
            // the doorbell).
            if core.stopped() {
                return RxNext::Stopped;
            }
        }
        wake.waker.wake_by_ref();
        RxNext::More(deadline_of(core))
    }
}

/// The owner deadline a call's entry files (the async loop's owner timer).
fn deadline_of(core: &RxCore) -> Option<tokio::time::Instant> {
    core.inline
        .as_ref()
        .and_then(|rx| rx.timer.deadline())
        .map(tokio::time::Instant::from_std)
}

impl LaneRxSource for RxCall {
    fn attach(&mut self, wake: RxWake) {
        self.wake = Some(wake);
    }

    fn serve(&mut self, ready: bool) -> RxNext {
        if ready {
            if let Some(wake) = self.wake.as_ref() {
                wake.begin_serve();
            }
        }
        if let Some(next) = self.resume() {
            return next;
        }
        self.run()
    }

    fn on_leave(&mut self) {
        self.wake = None;
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
        self
    }
}

/// How a graduated loop came back to its future.
pub(super) enum Graduated {
    /// The transport stopped (the entry's `active` check), the entry
    /// panicked (its state was dropped) or its lane went away: the loop
    /// ends.
    Ended,
    /// The loop continues as its async future: the entry escalated (a
    /// receive error) or (`refused`) no table took it, in which case it
    /// never tries again.
    Back { call: Box<RxCall>, refused: bool },
}

/// Graduate: install `call` in the table of the lane pass polling this
/// future and wait until it comes back.
pub(super) async fn run_on_lane(mut call: RxCall) -> Graduated {
    if let Some(rx) = call.core.inline.as_mut() {
        rx.graduate();
    }
    let mut handle = match lane_table::install_rx(Box::new(call)) {
        Ok(handle) => handle,
        Err(source) => {
            return match back(source) {
                Some(call) => Graduated::Back {
                    call,
                    refused: true,
                },
                None => Graduated::Ended,
            };
        }
    };
    match handle.finished().await {
        Some(LaneRxReturn::Escalated(source, _reason)) => match back(source) {
            Some(call) => Graduated::Back {
                call,
                refused: false,
            },
            None => Graduated::Ended,
        },
        // The state is dropped here, on the lane, as the loop's was when it
        // ended.
        Some(LaneRxReturn::Stopped(_) | LaneRxReturn::Retired(_) | LaneRxReturn::Poisoned)
        | None => Graduated::Ended,
    }
}

/// The source back as the loop's state, waiting on its owner timer again.
fn back(source: Box<dyn LaneRxSource>) -> Option<Box<RxCall>> {
    let mut call = source.into_any().downcast::<RxCall>().ok()?;
    if let Some(rx) = call.core.inline.as_mut() {
        rx.degrade(crate::media_clock::now_std());
    }
    Some(call)
}

#[cfg(test)]
mod tests;
