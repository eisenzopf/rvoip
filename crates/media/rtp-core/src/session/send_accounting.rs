//! Sender totals published by the RTP wire-order owner.
//!
//! Every successful send used to take the session statistics mutex (shared
//! with the receive path and RTCP) and bump a separate octet counter. The
//! packet writer already serializes sends under its own state lock, so it
//! keeps the running totals there and publishes them here with atomic
//! stores only. RTCP reads both RFC 3550 counters with one atomic load and
//! API snapshots read the 64-bit totals through a version check, without
//! taking either lock.

use std::sync::atomic::{fence, AtomicU64, Ordering};

/// Running send totals, owned by the packet writer's state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct SendTotals {
    /// RTP packets sent.
    pub packets: u64,
    /// Serialized RTP bytes sent (header and payload).
    pub wire_bytes: u64,
    /// RTP payload octets sent, the RFC 3550 sender octet count.
    pub payload_octets: u64,
}

/// Lock-free publication of [`SendTotals`] from a single writer.
#[derive(Debug, Default)]
pub(super) struct SendAccounting {
    /// Even when stable, odd while a publication is in progress.
    version: AtomicU64,
    packets: AtomicU64,
    wire_bytes: AtomicU64,
    /// RFC 3550 packet count (high 32 bits) and octet count (low 32 bits).
    rtcp: AtomicU64,
}

impl SendAccounting {
    /// Publish the writer's totals after a successful send.
    ///
    /// Call only while holding the packet writer's state lock, so there is
    /// exactly one publisher. Nothing between opening and closing the
    /// version awaits, allocates or calls out.
    pub fn publish(&self, totals: SendTotals) {
        let version = self.version.fetch_add(1, Ordering::AcqRel);
        debug_assert_eq!(version & 1, 0, "RTP send totals need a single writer");
        self.packets.store(totals.packets, Ordering::Relaxed);
        self.wire_bytes.store(totals.wire_bytes, Ordering::Relaxed);
        // RFC 3550 defines two independently wrapping 32-bit counters. Pack
        // them explicitly so an octet wrap cannot carry into the packet count.
        let packed =
            (u64::from(totals.packets as u32) << 32) | u64::from(totals.payload_octets as u32);
        self.rtcp.store(packed, Ordering::Release);
        self.version
            .store(version.wrapping_add(2), Ordering::Release);
    }

    /// The RFC 3550 sender packet and octet counts, from one consistent send.
    pub fn report_totals(&self) -> (u32, u32) {
        let packed = self.rtcp.load(Ordering::Acquire);
        ((packed >> 32) as u32, packed as u32)
    }

    /// Full-width packet and wire byte totals, from one consistent send.
    pub fn snapshot(&self) -> (u64, u64) {
        loop {
            let version = self.version.load(Ordering::Acquire);
            if version & 1 != 0 {
                std::hint::spin_loop();
                continue;
            }
            let packets = self.packets.load(Ordering::Relaxed);
            let bytes = self.wire_bytes.load(Ordering::Relaxed);
            fence(Ordering::Acquire);
            if self.version.load(Ordering::Relaxed) == version {
                return (packets, bytes);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtcp_counters_wrap_independently_and_api_totals_remain_u64() {
        let accounting = SendAccounting::default();
        assert_eq!(accounting.snapshot(), (0, 0));
        assert_eq!(accounting.report_totals(), (0, 0));
        let totals = SendTotals {
            packets: u64::from(u32::MAX) + 3,
            wire_bytes: (1_u64 << 34) + 190,
            payload_octets: (1_u64 << 33) + 160,
        };
        accounting.publish(totals);
        assert_eq!(accounting.snapshot(), (totals.packets, totals.wire_bytes));
        assert_eq!(accounting.report_totals(), (2, 160));
    }

    /// Replaces the old test that an RTCP snapshot waits for a complete
    /// concurrent update: readers racing one writer never combine the packet
    /// count of one send with the byte or octet count of another.
    #[test]
    fn concurrent_snapshots_never_mix_packet_and_byte_generations() {
        let accounting = std::sync::Arc::new(SendAccounting::default());
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(5));
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let accounting = accounting.clone();
                let barrier = barrier.clone();
                scope.spawn(move || {
                    barrier.wait();
                    for _ in 0..50_000 {
                        let (packets, bytes) = accounting.snapshot();
                        assert_eq!(bytes, packets * 172);
                        let (packets, octets) = accounting.report_totals();
                        assert_eq!(octets, packets * 160);
                    }
                });
            }
            barrier.wait();
            for packets in 1..=50_000 {
                accounting.publish(SendTotals {
                    packets,
                    wire_bytes: packets * 172,
                    payload_octets: packets * 160,
                });
            }
        });
        assert_eq!(accounting.snapshot(), (50_000, 8_600_000));
        assert_eq!(accounting.report_totals(), (50_000, 8_000_000));
    }
}
