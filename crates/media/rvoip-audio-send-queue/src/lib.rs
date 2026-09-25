//! Bounded, generation-aware queue for outbound media frames.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::{watch, Notify};

/// A frame rejected before it could enter the media transport.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SendError {
    /// The media pump or the queue has closed.
    #[error("outbound audio queue is closed")]
    Closed,
    /// The caller attempted to submit an invalidated generation.
    #[error("outbound audio generation {submitted} is stale; current generation is {current}")]
    StaleGeneration {
        /// Generation attached to the rejected frame.
        submitted: u64,
        /// Current accepted generation.
        current: u64,
    },
    /// Pending and in-flight audio reached the configured bound.
    #[error(
        "outbound audio queue overloaded at {pending_frames} pending frames (capacity {capacity_frames})"
    )]
    Overloaded {
        /// Queue capacity, excluding the single frame in flight.
        capacity_frames: usize,
        /// Pending frames at the time of rejection.
        pending_frames: usize,
    },
}

/// Point-in-time counters for one outbound media queue.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Metrics {
    /// Configured pending-frame capacity.
    pub capacity_frames: usize,
    /// Frames waiting for the media pump.
    pub pending_frames: usize,
    /// Maximum observed pending frames.
    pub peak_pending_frames: usize,
    /// Frames currently owned by the media pump (zero or one).
    pub in_flight_frames: usize,
    /// Pending frames belonging to a generation older than `generation`.
    pub stale_pending_frames: usize,
    /// Pump-owned frames belonging to a generation older than `generation`.
    pub stale_in_flight_frames: usize,
    /// Maximum combined pending and in-flight frames.
    pub peak_total_frames: usize,
    /// Frames accepted into the queue.
    pub accepted_frames: u64,
    /// Frames submitted successfully by the media pump.
    pub submitted_frames: u64,
    /// Queued frames removed by a generation advance or terminal close.
    pub flushed_frames: u64,
    /// In-flight frames cancelled before transport submission completed.
    pub cancelled_in_flight_frames: u64,
    /// Stale frames rejected at the producer boundary.
    pub rejected_stale_frames: u64,
    /// Attempts rejected after the queue became full.
    pub rejected_overflow_frames: u64,
    /// Number of transitions into the terminal overload state.
    pub overload_terminal_transitions: u64,
    /// Current accepted generation.
    pub generation: u64,
    /// Whether the receiver/media pump remains alive.
    pub pump_active: bool,
    /// Whether the queue has reached a terminal state.
    pub terminal: bool,
}

/// Atomic result of a successful outbound response-generation advance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GenerationAdvance {
    /// Accepted generation before the advance.
    pub previous_generation: u64,
    /// Accepted generation after the advance.
    pub current_generation: u64,
    /// Queued frames synchronously removed by this advance.
    pub flushed_on_advance: u64,
    /// Whether the generation notification armed cancellation for a stale
    /// pump-owned transport future.
    pub cancellation_armed: bool,
    /// Queue counters captured atomically after the advance and notification.
    pub metrics: Metrics,
}

struct Entry<T> {
    generation: u64,
    value: T,
}

struct State<T> {
    queue: VecDeque<Entry<T>>,
    capacity: usize,
    generation: u64,
    sender_count: usize,
    receiver_open: bool,
    closed: bool,
    overloaded: bool,
    peak_pending: usize,
    in_flight: usize,
    in_flight_generation: Option<u64>,
    peak_total: usize,
    accepted: u64,
    submitted: u64,
    flushed: u64,
    cancelled_in_flight: u64,
    rejected_stale: u64,
    rejected_overflow: u64,
    overload_transitions: u64,
}

struct Shared<T> {
    state: Mutex<State<T>>,
    ready: Notify,
    generation: watch::Sender<u64>,
}

impl<T> Shared<T> {
    fn lock(&self) -> MutexGuard<'_, State<T>> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn flush_locked(state: &mut State<T>) {
        state.flushed = state.flushed.saturating_add(state.queue.len() as u64);
        state.queue.clear();
    }
}

/// Producer half of a bounded generation-aware queue.
pub struct Sender<T> {
    shared: Arc<Shared<T>>,
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        self.shared.lock().sender_count += 1;
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        let should_wake = {
            let mut state = self.shared.lock();
            state.sender_count = state.sender_count.saturating_sub(1);
            state.sender_count == 0
        };
        if should_wake {
            self.shared.ready.notify_waiters();
        }
    }
}

impl<T> Sender<T> {
    /// Enqueue a value without waiting for capacity.
    ///
    /// The first full-queue observation atomically closes and drains the queue.
    /// This makes overload a single terminal transition rather than a sequence
    /// of independently recoverable frame losses.
    pub fn try_send(&self, generation: u64, value: T) -> Result<(), SendError> {
        let mut wake_generation = false;
        let result = {
            let mut state = self.shared.lock();
            if state.closed || !state.receiver_open {
                if state.overloaded {
                    state.rejected_overflow = state.rejected_overflow.saturating_add(1);
                }
                Err(SendError::Closed)
            } else if generation != state.generation {
                state.rejected_stale = state.rejected_stale.saturating_add(1);
                Err(SendError::StaleGeneration {
                    submitted: generation,
                    current: state.generation,
                })
            } else if state.queue.len() == state.capacity {
                let pending_frames = state.queue.len();
                state.rejected_overflow = state.rejected_overflow.saturating_add(1);
                state.overload_transitions = state.overload_transitions.saturating_add(1);
                state.overloaded = true;
                state.closed = true;
                Shared::flush_locked(&mut state);
                self.shared.generation.send_replace(state.generation);
                wake_generation = true;
                Err(SendError::Overloaded {
                    capacity_frames: state.capacity,
                    pending_frames,
                })
            } else {
                state.queue.push_back(Entry { generation, value });
                state.accepted = state.accepted.saturating_add(1);
                state.peak_pending = state.peak_pending.max(state.queue.len());
                state.peak_total = state
                    .peak_total
                    .max(state.queue.len().saturating_add(state.in_flight));
                Ok(())
            }
        };
        if result.is_ok() {
            self.shared.ready.notify_one();
        } else if wake_generation {
            self.shared.ready.notify_waiters();
        }
        result
    }

    /// Advance to `generation`, synchronously removing every older queued value.
    /// Returns the atomic post-action snapshot when the generation changed.
    pub fn advance_to(&self, generation: u64) -> Option<GenerationAdvance> {
        let advanced = {
            let mut state = self.shared.lock();
            if state.closed || generation <= state.generation {
                None
            } else {
                let previous_generation = state.generation;
                state.generation = generation;
                let before = state.queue.len();
                state.queue.retain(|entry| entry.generation >= generation);
                let flushed_on_advance = before.saturating_sub(state.queue.len()) as u64;
                state.flushed = state.flushed.saturating_add(flushed_on_advance);
                self.shared.generation.send_replace(generation);
                let cancellation_armed = state
                    .in_flight_generation
                    .is_some_and(|in_flight| in_flight < generation);
                Some(GenerationAdvance {
                    previous_generation,
                    current_generation: generation,
                    flushed_on_advance,
                    cancellation_armed,
                    metrics: snapshot(&state),
                })
            }
        };
        if advanced.is_some() {
            self.shared.ready.notify_waiters();
        }
        advanced
    }

    /// Close the queue and synchronously remove all pending values.
    pub fn close_and_flush(&self) {
        {
            let mut state = self.shared.lock();
            state.closed = true;
            Shared::flush_locked(&mut state);
            self.shared.generation.send_replace(state.generation);
        }
        self.shared.ready.notify_waiters();
    }

    /// Return the current accepted generation.
    pub fn generation(&self) -> u64 {
        self.shared.lock().generation
    }

    /// Return current queue counters.
    pub fn metrics(&self) -> Metrics {
        snapshot(&self.shared.lock())
    }

    /// Return `true` while the media pump can accept frames.
    pub fn is_open(&self) -> bool {
        let state = self.shared.lock();
        state.receiver_open && !state.closed
    }
}

/// Consumer half of a bounded generation-aware queue.
pub struct Receiver<T> {
    shared: Arc<Shared<T>>,
    generation: watch::Receiver<u64>,
}

impl<T> Receiver<T> {
    /// Wait for the next current-generation value.
    pub async fn recv(&mut self) -> Option<Delivery<T>> {
        loop {
            let notified = self.shared.ready.notified();
            {
                let mut state = self.shared.lock();
                while let Some(entry) = state.queue.pop_front() {
                    if entry.generation != state.generation {
                        state.flushed = state.flushed.saturating_add(1);
                        continue;
                    }
                    state.in_flight += 1;
                    state.in_flight_generation = Some(entry.generation);
                    state.peak_total = state
                        .peak_total
                        .max(state.queue.len().saturating_add(state.in_flight));
                    return Some(Delivery {
                        shared: Arc::clone(&self.shared),
                        generation: entry.generation,
                        value: Some(entry.value),
                        finished: false,
                    });
                }
                if state.closed || state.sender_count == 0 {
                    return None;
                }
            }
            notified.await;
        }
    }

    /// Subscribe to generation advances and terminal close notifications.
    pub fn generation_changes(&self) -> watch::Receiver<u64> {
        self.generation.clone()
    }

    /// Return `true` once this queue has been closed.
    pub fn is_closed(&self) -> bool {
        self.shared.lock().closed
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        {
            let mut state = self.shared.lock();
            state.receiver_open = false;
            state.closed = true;
            Shared::flush_locked(&mut state);
            self.shared.generation.send_replace(state.generation);
        }
        self.shared.ready.notify_waiters();
    }
}

/// A value currently owned by the media pump.
pub struct Delivery<T> {
    shared: Arc<Shared<T>>,
    generation: u64,
    value: Option<T>,
    finished: bool,
}

impl<T> Delivery<T> {
    /// Generation attached to the value.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Move the queued value into the transport future while retaining the
    /// completion guard.
    pub fn take(&mut self) -> T {
        self.value.take().expect("delivery value can be taken once")
    }

    /// Record successful submission to the underlying transport.
    pub fn mark_submitted(mut self) {
        let mut state = self.shared.lock();
        state.in_flight = state.in_flight.saturating_sub(1);
        state.in_flight_generation = None;
        state.submitted = state.submitted.saturating_add(1);
        self.finished = true;
    }
}

impl<T> Drop for Delivery<T> {
    fn drop(&mut self) {
        if !self.finished {
            let mut state = self.shared.lock();
            state.in_flight = state.in_flight.saturating_sub(1);
            state.in_flight_generation = None;
            state.cancelled_in_flight = state.cancelled_in_flight.saturating_add(1);
        }
    }
}

/// Create a bounded queue accepting generation zero.
pub fn channel<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    assert!(capacity > 0, "audio queue capacity must be positive");
    let (generation_tx, generation_rx) = watch::channel(0);
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            queue: VecDeque::with_capacity(capacity),
            capacity,
            generation: 0,
            sender_count: 1,
            receiver_open: true,
            closed: false,
            overloaded: false,
            peak_pending: 0,
            in_flight: 0,
            in_flight_generation: None,
            peak_total: 0,
            accepted: 0,
            submitted: 0,
            flushed: 0,
            cancelled_in_flight: 0,
            rejected_stale: 0,
            rejected_overflow: 0,
            overload_transitions: 0,
        }),
        ready: Notify::new(),
        generation: generation_tx,
    });
    (
        Sender {
            shared: Arc::clone(&shared),
        },
        Receiver {
            shared,
            generation: generation_rx,
        },
    )
}

fn snapshot<T>(state: &State<T>) -> Metrics {
    Metrics {
        capacity_frames: state.capacity,
        pending_frames: state.queue.len(),
        peak_pending_frames: state.peak_pending,
        in_flight_frames: state.in_flight,
        stale_pending_frames: state
            .queue
            .iter()
            .filter(|entry| entry.generation < state.generation)
            .count(),
        stale_in_flight_frames: usize::from(
            state
                .in_flight_generation
                .is_some_and(|generation| generation < state.generation),
        ),
        peak_total_frames: state.peak_total,
        accepted_frames: state.accepted,
        submitted_frames: state.submitted,
        flushed_frames: state.flushed,
        cancelled_in_flight_frames: state.cancelled_in_flight,
        rejected_stale_frames: state.rejected_stale,
        rejected_overflow_frames: state.rejected_overflow,
        overload_terminal_transitions: state.overload_transitions,
        generation: state.generation,
        pump_active: state.receiver_open,
        terminal: state.closed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[tokio::test]
    async fn sustained_overflow_is_bounded_and_has_one_terminal_transition() {
        let (sender, mut receiver) = channel(9);
        let delivered = Arc::new(AtomicUsize::new(0));
        let delivered_by_pump = Arc::clone(&delivered);
        let pump = tokio::spawn(async move {
            let mut changes = receiver.generation_changes();
            while let Some(mut delivery) = receiver.recv().await {
                let _value = delivery.take();
                tokio::select! {
                    biased;
                    changed = changes.changed() => {
                        assert!(changed.is_ok());
                        drop(delivery);
                    }
                    () = tokio::time::sleep(Duration::from_secs(30)) => {
                        delivery.mark_submitted();
                        delivered_by_pump.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        });

        sender.try_send(0, 0_u8).unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while sender.metrics().in_flight_frames != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        for value in 1..=9 {
            sender.try_send(0, value).unwrap();
        }
        assert_eq!(sender.metrics().peak_total_frames, 10);

        let first = sender.try_send(0, 10).unwrap_err();
        assert_eq!(
            first,
            SendError::Overloaded {
                capacity_frames: 9,
                pending_frames: 9,
            }
        );
        for value in 11..=100 {
            assert_eq!(sender.try_send(0, value), Err(SendError::Closed));
        }
        pump.await.unwrap();

        let metrics = sender.metrics();
        assert_eq!(metrics.pending_frames, 0);
        assert_eq!(metrics.in_flight_frames, 0);
        assert_eq!(metrics.peak_pending_frames, 9);
        assert_eq!(metrics.peak_total_frames, 10);
        assert_eq!(metrics.overload_terminal_transitions, 1);
        assert!(metrics.terminal);
        assert!(!metrics.pump_active);
        assert_eq!(delivered.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn generation_advance_flushes_stale_values_before_delivery() {
        let (sender, mut receiver) = channel(9);
        for value in 0..5_u8 {
            sender.try_send(0, value).unwrap();
        }
        let advance = sender.advance_to(1).unwrap();
        assert_eq!(advance.previous_generation, 0);
        assert_eq!(advance.current_generation, 1);
        assert_eq!(advance.flushed_on_advance, 5);
        assert!(!advance.cancellation_armed);
        assert_eq!(advance.metrics.pending_frames, 0);
        assert_eq!(advance.metrics.stale_pending_frames, 0);
        assert_eq!(
            sender.try_send(0, 99),
            Err(SendError::StaleGeneration {
                submitted: 0,
                current: 1,
            })
        );
        sender.try_send(1, 42).unwrap();

        let mut delivery = receiver.recv().await.unwrap();
        assert_eq!(delivery.generation(), 1);
        assert_eq!(delivery.take(), 42);
        delivery.mark_submitted();
        sender.close_and_flush();
        assert!(receiver.recv().await.is_none());
        drop(receiver);

        let metrics = sender.metrics();
        assert_eq!(metrics.flushed_frames, 5);
        assert_eq!(metrics.rejected_stale_frames, 1);
        assert_eq!(metrics.submitted_frames, 1);
        assert_eq!(metrics.pending_frames, 0);
        assert_eq!(metrics.in_flight_frames, 0);
        assert!(!metrics.pump_active);
    }

    #[tokio::test]
    async fn generation_advance_cancels_an_in_flight_stale_submission() {
        let (sender, mut receiver) = channel(9);
        let (delivered_tx, mut delivered_rx) = tokio::sync::mpsc::unbounded_channel();
        let pump = tokio::spawn(async move {
            let mut changes = receiver.generation_changes();
            while let Some(mut delivery) = receiver.recv().await {
                let value = delivery.take();
                if value == 0_u8 {
                    tokio::select! {
                        biased;
                        changed = changes.changed() => {
                            assert!(changed.is_ok());
                            drop(delivery);
                        }
                        () = tokio::time::sleep(Duration::from_secs(30)) => {
                            delivery.mark_submitted();
                            delivered_tx.send(value).unwrap();
                        }
                    }
                } else {
                    delivery.mark_submitted();
                    delivered_tx.send(value).unwrap();
                }
            }
        });

        sender.try_send(0, 0_u8).unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while sender.metrics().in_flight_frames != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let advance = sender.advance_to(1).unwrap();
        assert!(advance.cancellation_armed);
        assert_eq!(advance.metrics.stale_pending_frames, 0);
        assert_eq!(advance.metrics.stale_in_flight_frames, 1);
        sender.try_send(1, 1_u8).unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), delivered_rx.recv())
                .await
                .unwrap(),
            Some(1)
        );
        sender.close_and_flush();
        pump.await.unwrap();

        let metrics = sender.metrics();
        assert_eq!(metrics.cancelled_in_flight_frames, 1);
        assert_eq!(metrics.submitted_frames, 1);
        assert_eq!(metrics.pending_frames, 0);
        assert_eq!(metrics.in_flight_frames, 0);
        assert!(!metrics.pump_active);
        assert!(delivered_rx.try_recv().is_err());
    }
}

