//! Receive statistics published by the socket owner without the statistics
//! mutex ([`MediaExecutorConfig::owner_receive_stats`]).
//!
//! Under executor v2 the session's inline ingress, on the task that owns the
//! socket, is the only writer of the per-packet receive statistics
//! (`packets_received`, `bytes_received`, `packets_lost`,
//! `packets_duplicated`, `packets_out_of_order`, `jitter_ms`,
//! `remote_addr`). Without this switch it takes the session statistics mutex
//! for every packet to write them, as the v1 receive task does. Here the
//! owner keeps the totals in its own state and publishes them through
//! [`ReceiveAccounting`], a single-writer seqlock whose fields are all
//! atomics (no `unsafe`): plain stores and one release fence per packet, no
//! read-modify-write. Readers (`RtpSession::get_stats` and the XR report)
//! take the mutex as before and [`ReceiveAccounting::overlay`] the owner's
//! coherent snapshot onto it.
//!
//! Two fields are also written by the control plane, and are fenced by
//! generations bumped under the statistics mutex:
//!
//! - `jitter_ms` is zeroed by `set_clock_rate`. The owner reads the reset
//!   generation before it touches the stream and publishes jitter tagged
//!   with it; readers use the published jitter only while its tag is the
//!   current generation, the mutex's value otherwise. Either way the reader
//!   sees what v1's mutex would show for some order of the reset and the
//!   packet.
//! - `remote_addr` is set by `set_remote_addr`. v1 overwrites it with the
//!   packet's source for every packet; the owner writes it (under the mutex)
//!   only when the source differs from what it last wrote or a control-plane
//!   write happened since, which leaves the same value.
//!
//! Counters are additive: a reader adds the owner's totals to the mutex's,
//! which stay zero unless another path (a v1 receive task) also accounts.
//!
//! [`MediaExecutorConfig::owner_receive_stats`]: crate::executor::MediaExecutorConfig::owner_receive_stats

use std::net::SocketAddr;
use std::sync::atomic::{fence, AtomicU64, Ordering};
use std::sync::Arc;

use super::RtpSessionStats;

/// Published jitter that matches no reset generation (nothing published).
const NO_JITTER: u64 = u64::MAX;

/// The owner's running totals. `[lost, duplicated, out_of_order]` follow
/// v1's aggregate-of-stream-deltas rule exactly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ReceiveTotals {
    pub packets: u64,
    pub bytes: u64,
    pub lost: u64,
    pub duplicated: u64,
    pub out_of_order: u64,
}

impl ReceiveTotals {
    /// One packet, as v1's `MediaReceived` accounting adds it to the
    /// session statistics. `before`/`after` are the stream's
    /// `[packets_lost, duplicates, packets_out_of_order]` around it.
    pub(super) fn add_packet(&mut self, wire_bytes: u64, before: [u64; 3], after: [u64; 3]) {
        self.packets += 1;
        self.bytes += wire_bytes;
        self.lost = self.lost.saturating_sub(before[0]).saturating_add(after[0]);
        self.duplicated = self
            .duplicated
            .saturating_sub(before[1])
            .saturating_add(after[1]);
        self.out_of_order = self
            .out_of_order
            .saturating_sub(before[2])
            .saturating_add(after[2]);
    }
}

/// A coherent view of what the owner published.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ReceiveSnapshot {
    pub totals: ReceiveTotals,
    pub jitter_bits: u64,
    pub jitter_generation: u64,
}

pub(super) struct ReceiveAccounting {
    version: AtomicU64,
    packets: AtomicU64,
    bytes: AtomicU64,
    lost: AtomicU64,
    duplicated: AtomicU64,
    out_of_order: AtomicU64,
    jitter_bits: AtomicU64,
    jitter_generation: AtomicU64,
    /// Bumped, under the statistics mutex, by every control-plane write of
    /// `jitter_ms`.
    jitter_resets: AtomicU64,
    /// Bumped, under the statistics mutex, by every control-plane write of
    /// `remote_addr`.
    remote_writes: AtomicU64,
}

impl ReceiveAccounting {
    pub(super) fn new() -> Self {
        Self {
            version: AtomicU64::new(0),
            packets: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            lost: AtomicU64::new(0),
            duplicated: AtomicU64::new(0),
            out_of_order: AtomicU64::new(0),
            jitter_bits: AtomicU64::new(0),
            jitter_generation: AtomicU64::new(NO_JITTER),
            jitter_resets: AtomicU64::new(0),
            remote_writes: AtomicU64::new(0),
        }
    }

    /// The control plane wrote `jitter_ms`. Call while holding the session
    /// statistics mutex.
    pub(super) fn note_jitter_reset(&self) {
        self.jitter_resets.fetch_add(1, Ordering::AcqRel);
    }

    /// The control plane wrote `remote_addr`. Call while holding the session
    /// statistics mutex.
    pub(super) fn note_remote_write(&self) {
        self.remote_writes.fetch_add(1, Ordering::AcqRel);
    }

    /// Single writer (the owner's [`ReceiveAccountingWriter`]): plain stores
    /// bracketed by the version, no read-modify-write.
    fn publish(&self, totals: &ReceiveTotals, jitter_bits: u64, jitter_generation: u64) {
        let version = self.version.load(Ordering::Relaxed);
        debug_assert_eq!(version & 1, 0, "receive accounting has one writer");
        self.version
            .store(version.wrapping_add(1), Ordering::Relaxed);
        // Orders the odd version before the data: a reader that sees any of
        // the stores below also sees the odd version on its re-check.
        fence(Ordering::Release);
        self.packets.store(totals.packets, Ordering::Relaxed);
        self.bytes.store(totals.bytes, Ordering::Relaxed);
        self.lost.store(totals.lost, Ordering::Relaxed);
        self.duplicated.store(totals.duplicated, Ordering::Relaxed);
        self.out_of_order
            .store(totals.out_of_order, Ordering::Relaxed);
        self.jitter_bits.store(jitter_bits, Ordering::Relaxed);
        self.jitter_generation
            .store(jitter_generation, Ordering::Relaxed);
        self.version
            .store(version.wrapping_add(2), Ordering::Release);
    }

    /// A coherent snapshot (never a mix of two publications).
    pub(super) fn snapshot(&self) -> ReceiveSnapshot {
        loop {
            let version = self.version.load(Ordering::Acquire);
            if version & 1 != 0 {
                std::hint::spin_loop();
                continue;
            }
            let snapshot = ReceiveSnapshot {
                totals: ReceiveTotals {
                    packets: self.packets.load(Ordering::Relaxed),
                    bytes: self.bytes.load(Ordering::Relaxed),
                    lost: self.lost.load(Ordering::Relaxed),
                    duplicated: self.duplicated.load(Ordering::Relaxed),
                    out_of_order: self.out_of_order.load(Ordering::Relaxed),
                },
                jitter_bits: self.jitter_bits.load(Ordering::Relaxed),
                jitter_generation: self.jitter_generation.load(Ordering::Relaxed),
            };
            fence(Ordering::Acquire);
            if self.version.load(Ordering::Relaxed) == version {
                return snapshot;
            }
        }
    }

    /// Fold what the owner published into `stats`, a clone of the session
    /// statistics mutex's contents.
    pub(super) fn overlay(&self, stats: &mut RtpSessionStats) {
        let snapshot = self.snapshot();
        let totals = snapshot.totals;
        stats.packets_received = stats.packets_received.saturating_add(totals.packets);
        stats.bytes_received = stats.bytes_received.saturating_add(totals.bytes);
        stats.packets_lost = stats.packets_lost.saturating_add(totals.lost);
        stats.packets_duplicated = stats.packets_duplicated.saturating_add(totals.duplicated);
        stats.packets_out_of_order = stats
            .packets_out_of_order
            .saturating_add(totals.out_of_order);
        if snapshot.jitter_generation == self.jitter_resets.load(Ordering::Acquire) {
            stats.jitter_ms = f64::from_bits(snapshot.jitter_bits);
        }
    }
}

/// The socket owner's side: its totals, and what it last wrote to the
/// statistics mutex.
pub(super) struct ReceiveAccountingWriter {
    shared: Arc<ReceiveAccounting>,
    totals: ReceiveTotals,
    written_source: Option<SocketAddr>,
    seen_remote_writes: u64,
}

impl ReceiveAccountingWriter {
    pub(super) fn new(shared: Arc<ReceiveAccounting>) -> Self {
        crate::executor::record_owner_receive_stats();
        Self {
            shared,
            totals: ReceiveTotals::default(),
            written_source: None,
            seen_remote_writes: 0,
        }
    }

    /// Read before the packet touches its stream: the reset generation the
    /// packet's jitter is published under.
    pub(super) fn begin(&self) -> u64 {
        self.shared.jitter_resets.load(Ordering::Acquire)
    }

    /// v1's per-packet statistics update, published without the mutex.
    pub(super) fn account(
        &mut self,
        wire_bytes: u64,
        before: [u64; 3],
        after: [u64; 3],
        jitter_ms: f64,
        jitter_generation: u64,
    ) {
        self.totals.add_packet(wire_bytes, before, after);
        self.shared
            .publish(&self.totals, jitter_ms.to_bits(), jitter_generation);
    }

    /// v1 sets `remote_addr = Some(source)` for every packet. Write it only
    /// when that would change what the mutex holds: a new source, or a
    /// control-plane write since the last one.
    pub(super) fn remote(
        &mut self,
        source: SocketAddr,
        stats: &parking_lot::Mutex<RtpSessionStats>,
    ) {
        let writes = self.shared.remote_writes.load(Ordering::Acquire);
        if self.written_source == Some(source) && writes == self.seen_remote_writes {
            return;
        }
        stats.lock().remote_addr = Some(source);
        self.written_source = Some(source);
        self.seen_remote_writes = writes;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// v1's mutex update, field for field (`MediaReceived` arm).
    fn v1_update(
        stats: &mut RtpSessionStats,
        wire_bytes: u64,
        before: [u64; 3],
        after: [u64; 3],
        jitter_ms: f64,
        source: SocketAddr,
    ) {
        stats.packets_received += 1;
        stats.bytes_received += wire_bytes;
        stats.packets_lost = stats
            .packets_lost
            .saturating_sub(before[0])
            .saturating_add(after[0]);
        stats.packets_duplicated = stats
            .packets_duplicated
            .saturating_sub(before[1])
            .saturating_add(after[1]);
        stats.packets_out_of_order = stats
            .packets_out_of_order
            .saturating_sub(before[2])
            .saturating_add(after[2]);
        stats.jitter_ms = jitter_ms;
        stats.remote_addr = Some(source);
    }

    fn merged(
        mutex: &parking_lot::Mutex<RtpSessionStats>,
        shared: &ReceiveAccounting,
    ) -> RtpSessionStats {
        let mut stats = mutex.lock().clone();
        shared.overlay(&mut stats);
        stats
    }

    fn key(stats: &RtpSessionStats) -> (u64, u64, u64, u64, u64, u64, Option<SocketAddr>) {
        (
            stats.packets_received,
            stats.bytes_received,
            stats.packets_lost,
            stats.packets_duplicated,
            stats.packets_out_of_order,
            stats.jitter_ms.to_bits(),
            stats.remote_addr,
        )
    }

    /// A script of stream deltas with loss that a late packet recovers, a
    /// duplicate and reordering, from two sources: the owner-published
    /// statistics equal v1's after every packet.
    #[test]
    fn owner_published_statistics_equal_v1_after_every_packet() {
        let a: SocketAddr = "192.0.2.1:4000".parse().unwrap();
        let b: SocketAddr = "192.0.2.2:4000".parse().unwrap();
        // (wire bytes, before, after, jitter, source)
        let script: [(u64, [u64; 3], [u64; 3], f64, SocketAddr); 7] = [
            (172, [0, 0, 0], [0, 0, 0], 0.0, a),
            (172, [0, 0, 0], [1, 0, 0], 0.25, a),
            (172, [1, 0, 0], [0, 0, 1], 0.5, a),
            (172, [0, 0, 1], [0, 1, 1], 0.5, a),
            (180, [0, 1, 1], [2, 1, 1], 1.75, b),
            (172, [2, 1, 1], [2, 1, 1], 2.0, b),
            (172, [2, 1, 1], [1, 1, 2], 1.5, a),
        ];
        let mut v1 = RtpSessionStats::default();
        let mutex = parking_lot::Mutex::new(RtpSessionStats::default());
        let shared = Arc::new(ReceiveAccounting::new());
        let mut writer = ReceiveAccountingWriter::new(Arc::clone(&shared));
        assert_eq!(
            key(&merged(&mutex, &shared)),
            key(&v1),
            "nothing published yet"
        );
        for (wire, before, after, jitter, source) in script {
            v1_update(&mut v1, wire, before, after, jitter, source);
            let generation = writer.begin();
            writer.account(wire, before, after, jitter, generation);
            writer.remote(source, &mutex);
            assert_eq!(key(&merged(&mutex, &shared)), key(&v1));
        }
    }

    /// Generation fencing of `jitter_ms`: a reset shows the mutex's zero
    /// until the owner publishes under the new generation, and a packet that
    /// read the generation before the reset never shows its jitter after it.
    #[test]
    fn a_clock_reset_fences_jitter_published_under_the_old_generation() {
        let mutex = parking_lot::Mutex::new(RtpSessionStats::default());
        let shared = Arc::new(ReceiveAccounting::new());
        let mut writer = ReceiveAccountingWriter::new(Arc::clone(&shared));
        let generation = writer.begin();
        writer.account(172, [0; 3], [0; 3], 3.5, generation);
        assert_eq!(merged(&mutex, &shared).jitter_ms, 3.5);
        // A packet reads the generation, then the reset lands (as
        // `set_clock_rate`: zero under the mutex, bump), then the packet
        // publishes: v1 order "packet, reset" leaves zero.
        let stale = writer.begin();
        {
            let mut stats = mutex.lock();
            stats.jitter_ms = 0.0;
            shared.note_jitter_reset();
        }
        writer.account(172, [0; 3], [0; 3], 7.0, stale);
        let stats = merged(&mutex, &shared);
        assert_eq!(stats.jitter_ms, 0.0, "stale-generation jitter is not shown");
        assert_eq!(stats.packets_received, 2, "counters are never fenced");
        // The next packet publishes under the new generation.
        let fresh = writer.begin();
        assert_ne!(fresh, stale);
        writer.account(172, [0; 3], [0; 3], 0.75, fresh);
        assert_eq!(merged(&mutex, &shared).jitter_ms, 0.75);
    }

    /// `remote_addr` is written once per source and again after a
    /// control-plane write, never per packet otherwise.
    #[test]
    fn remote_address_is_rewritten_only_on_change_or_after_a_control_plane_write() {
        let a: SocketAddr = "192.0.2.1:4000".parse().unwrap();
        let x: SocketAddr = "198.51.100.9:5000".parse().unwrap();
        let mutex = parking_lot::Mutex::new(RtpSessionStats::default());
        let shared = Arc::new(ReceiveAccounting::new());
        let mut writer = ReceiveAccountingWriter::new(Arc::clone(&shared));
        writer.remote(a, &mutex);
        assert_eq!(mutex.lock().remote_addr, Some(a));
        // Someone writes the field without the control-plane fence (only to
        // observe the skip): the owner does not touch the mutex again for the
        // same source.
        mutex.lock().remote_addr = None;
        writer.remote(a, &mutex);
        assert_eq!(mutex.lock().remote_addr, None, "same source: no write");
        // `set_remote_addr`: write under the mutex and fence.
        {
            let mut stats = mutex.lock();
            stats.remote_addr = Some(x);
            shared.note_remote_write();
        }
        assert_eq!(mutex.lock().remote_addr, Some(x));
        writer.remote(a, &mutex);
        assert_eq!(
            mutex.lock().remote_addr,
            Some(a),
            "v1: the next packet's source wins"
        );
    }

    #[test]
    fn concurrent_snapshots_never_mix_two_publications() {
        let shared = Arc::new(ReceiveAccounting::new());
        let barrier = Arc::new(std::sync::Barrier::new(4));
        std::thread::scope(|scope| {
            for _ in 0..3 {
                let shared = Arc::clone(&shared);
                let barrier = Arc::clone(&barrier);
                scope.spawn(move || {
                    barrier.wait();
                    for _ in 0..50_000 {
                        let snapshot = shared.snapshot();
                        let totals = snapshot.totals;
                        assert_eq!(totals.bytes, totals.packets * 172);
                        assert_eq!(totals.lost, totals.packets / 3);
                        assert_eq!(snapshot.jitter_bits, (totals.packets as f64).to_bits());
                    }
                });
            }
            barrier.wait();
            let mut writer = ReceiveAccountingWriter::new(Arc::clone(&shared));
            let mut lost = 0u64;
            for packets in 1..=50_000u64 {
                let next_lost = packets / 3;
                writer.account(172, [lost, 0, 0], [next_lost, 0, 0], packets as f64, 0);
                lost = next_lost;
            }
        });
        let totals = shared.snapshot().totals;
        assert_eq!((totals.packets, totals.bytes), (50_000, 50_000 * 172));
    }
}
