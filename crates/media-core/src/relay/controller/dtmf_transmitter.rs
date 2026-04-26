//! RFC 4733 §2.5 telephone-event sender.
//!
//! Schedules the full RFC-conformant packet sequence for one DTMF
//! digit:
//!
//! 1. **Start packet** — `E=0`, `marker=1` (first packet of a new
//!    event per RFC 3550 §5.1), duration set to one tick worth of
//!    samples.
//! 2. **Continuation packets** — emitted every 20 ms while the tone
//!    is active. `E=0`, `marker=0`, duration incrementing by
//!    `SAMPLES_PER_TICK` each step. Timestamp stays anchored to the
//!    start timestamp (the "tone start" per RFC 4733 §2.1).
//! 3. **Three end-of-event retransmits** — `E=1`, all sharing the
//!    start timestamp + final duration value, sent back-to-back per
//!    RFC 4733 §2.5.1.3. Receivers dedup on `(ssrc, rtp_timestamp)`
//!    so the duplicates are collapsed into one logical digit
//!    upstream (Sprint 2.5 P4 — the dedup lives in
//!    `rtp-core::transport::udp::UdpRtpTransport`).
//!
//! All packets share the audio stream's SSRC and the tone-start
//! timestamp, which lets the receiver correlate the DTMF event with
//! the audio it overlays. The audio cursor itself does NOT advance
//! during the tone — `RtpSession::send_packet_with_pt` accepts an
//! explicit timestamp, so no audio-vs-DTMF clock drift occurs.
//!
//! `send_digit` is fire-and-forget: it spawns a `tokio::task` that
//! runs the schedule and returns the `JoinHandle` immediately so a
//! softphone key-down handler doesn't block on the full tone
//! duration. Drop the handle to ignore completion; await it for
//! coordination.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use tokio::sync::Mutex;
use tracing::{debug, warn};

use rvoip_rtp_core::RtpSession;

use crate::codec::audio::dtmf::{DtmfEvent, TelephoneEvent};
use crate::error::{Error, Result};

/// One audio tick = 20 ms (matches the audio frame cadence used for
/// PCMU / PCMA / Opus across the stack).
const TICK: Duration = Duration::from_millis(20);
/// 20 ms × 8 kHz = 160 samples per tick at the RFC 4733 telephone-event
/// clock rate.
const SAMPLES_PER_TICK: u16 = 160;
/// RFC 4733 §2.5.1.3 — the sender emits up to three identical
/// end-of-event packets back-to-back for loss resilience. The
/// receive-side dedup at `rtp-core::transport::udp` collapses these
/// into one downstream `DtmfEvent`.
const END_OF_EVENT_RETRANSMITS: usize = 3;
/// Reasonable default volume for DTMF: -10 dBm0. Saturates to 63 (the
/// 6-bit field's max).
const DEFAULT_VOLUME: u8 = 10;

/// Multi-packet RFC 4733 DTMF sender. Owns no per-call state — each
/// `send_digit` spawns an independent task. Construct one per RTP
/// session.
pub struct DtmfTransmitter {
    rtp_session: Arc<Mutex<RtpSession>>,
}

impl DtmfTransmitter {
    pub fn new(rtp_session: Arc<Mutex<RtpSession>>) -> Self {
        Self { rtp_session }
    }

    /// Spawn the RFC 4733 §2.5.1.3 packet schedule for one digit.
    /// Returns immediately with a `JoinHandle` — the caller can drop
    /// it for fire-and-forget semantics or await it to know when the
    /// tone has fully drained onto the wire.
    pub fn send_digit(
        &self,
        digit: char,
        duration_ms: u32,
    ) -> tokio::task::JoinHandle<Result<()>> {
        let rtp_session = self.rtp_session.clone();
        tokio::spawn(async move { run_schedule(rtp_session, digit, duration_ms).await })
    }
}

async fn run_schedule(
    rtp_session: Arc<Mutex<RtpSession>>,
    digit: char,
    duration_ms: u32,
) -> Result<()> {
    let event_code = DtmfEvent::from_digit(digit).map(|d| d.0).unwrap_or(0);

    // Anchor the tone on the audio stream's current cursor (RFC 4733 §2.1).
    let start_timestamp = {
        let session = rtp_session.lock().await;
        session.current_timestamp()
    };

    // Total audio ticks to span. A short digit (e.g. 100 ms) becomes
    // 5 ticks: one start + 3 continuations + the final tick covered
    // by the E=1 retransmits.
    let total_ticks = (duration_ms / 20).max(1);

    // Start packet: E=0, marker=1, duration = one tick.
    let mut duration_samples: u16 = SAMPLES_PER_TICK;
    send_packet(
        &rtp_session,
        event_code,
        false,
        DEFAULT_VOLUME,
        duration_samples,
        start_timestamp,
        true,
    )
    .await?;

    // Continuations: tick-spaced E=0 packets with monotonically
    // growing duration. We send one fewer than `total_ticks - 1` so
    // the last tick is reserved for the E=1 retransmits below.
    let continuation_count = total_ticks.saturating_sub(2);
    for _ in 0..continuation_count {
        tokio::time::sleep(TICK).await;
        duration_samples = duration_samples.saturating_add(SAMPLES_PER_TICK);
        send_packet(
            &rtp_session,
            event_code,
            false,
            DEFAULT_VOLUME,
            duration_samples,
            start_timestamp,
            false,
        )
        .await?;
    }

    // Final tick → switch to E=1 and emit RFC 4733 §2.5.1.3 retransmits.
    tokio::time::sleep(TICK).await;
    duration_samples = duration_samples.saturating_add(SAMPLES_PER_TICK);
    for _ in 0..END_OF_EVENT_RETRANSMITS {
        send_packet(
            &rtp_session,
            event_code,
            true,
            DEFAULT_VOLUME,
            duration_samples,
            start_timestamp,
            false,
        )
        .await?;
    }

    debug!(
        "RFC 4733 DTMF '{}' transmitted: {} continuations + 3 end retransmits, ts={}, dur={}",
        digit, continuation_count, start_timestamp, duration_samples
    );
    Ok(())
}

async fn send_packet(
    rtp_session: &Arc<Mutex<RtpSession>>,
    event: u8,
    end_of_event: bool,
    volume: u8,
    duration: u16,
    timestamp: u32,
    marker: bool,
) -> Result<()> {
    let tele = TelephoneEvent {
        event,
        end_of_event,
        volume,
        duration,
    };
    let wire = tele.encode();
    let mut session = rtp_session.lock().await;
    session
        .send_packet_with_pt(timestamp, Bytes::copy_from_slice(&wire), marker, /*PT*/ 101)
        .await
        .map_err(|e| {
            warn!("DTMF send failed: {}", e);
            Error::config(format!("DTMF send failed: {}", e))
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_rtp_core::session::{RtpSession, RtpSessionConfig};
    use rvoip_rtp_core::traits::RtpEvent;
    use rvoip_rtp_core::transport::{RtpTransport, RtpTransportConfig, UdpRtpTransport};
    use std::collections::HashSet;
    use std::time::Duration;

    /// Bind a sender `RtpSession` (PCMU, 8 kHz) and a passive receiver
    /// `UdpRtpTransport`. Wire the sender's remote to the receiver so
    /// every PT 101 packet the transmitter emits surfaces on the
    /// receiver's broadcast channel as `RtpEvent::DtmfEvent`. Returns
    /// `(sender_session, receiver_events)`.
    async fn pair() -> (
        Arc<Mutex<RtpSession>>,
        tokio::sync::broadcast::Receiver<RtpEvent>,
    ) {
        let receiver_cfg = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some("dtmf-tx-test-rx".to_string()),
            use_port_allocator: false,
        };
        let receiver = UdpRtpTransport::new(receiver_cfg).await.unwrap();
        let receiver_addr = receiver.local_rtp_addr().unwrap();
        let events = receiver.subscribe();

        // Sender RtpSession — bind to ephemeral port, target the
        // receiver. Fixed SSRC so the receive side sees a stable
        // identity across the test's packet stream.
        let session_cfg = RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            remote_addr: Some(receiver_addr),
            ssrc: Some(0xCAFE_BABE),
            payload_type: 0,
            clock_rate: 8000,
            ..RtpSessionConfig::default()
        };
        let rtp_session = RtpSession::new(session_cfg).await.expect("rtp session");

        (Arc::new(Mutex::new(rtp_session)), events)
    }

    /// Drain DTMF events from the receiver until a `timeout` elapses
    /// without further frames. Returns the collected events in arrival
    /// order.
    async fn drain_dtmf(
        rx: &mut tokio::sync::broadcast::Receiver<RtpEvent>,
        idle_timeout: Duration,
    ) -> Vec<RtpEvent> {
        let mut out = Vec::new();
        loop {
            match tokio::time::timeout(idle_timeout, rx.recv()).await {
                Ok(Ok(ev)) => out.push(ev),
                _ => break,
            }
        }
        out
    }

    #[tokio::test]
    async fn start_packet_carries_e0_and_initial_duration() {
        let (session, mut rx) = pair().await;
        let tx = DtmfTransmitter::new(session.clone());
        let _handle = tx.send_digit('5', /*duration*/ 100);

        // Grab just the first DtmfEvent — the start packet must have
        // E=0 and a single-tick duration. The receiver also dedups the
        // 3× E=1 retransmits, so subsequent events represent only the
        // continuations + the (collapsed) final E=1 frame.
        let first = tokio::time::timeout(Duration::from_millis(500), rx.recv())
            .await
            .expect("receive timeout")
            .expect("recv");
        match first {
            RtpEvent::DtmfEvent {
                event,
                end_of_event,
                duration,
                ..
            } => {
                assert_eq!(event, 5, "event code maps to digit '5'");
                assert!(!end_of_event, "start packet must have E=0");
                assert_eq!(duration, SAMPLES_PER_TICK, "start packet carries one tick");
            }
            other => panic!("expected start DtmfEvent, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn continuation_packets_share_timestamp_increment_duration() {
        let (session, mut rx) = pair().await;
        let tx = DtmfTransmitter::new(session.clone());
        // 100 ms tone → 5 ticks: 1 start + 3 continuations + final E=1.
        let _handle = tx.send_digit('1', 100);

        // Collect every event: receive-side dedup means the three E=1
        // retransmits arrive as a single DtmfEvent, so we expect:
        // start (E=0) + 3 continuations (E=0) + 1 dedup'd end (E=1) = 5.
        let evs = drain_dtmf(&mut rx, Duration::from_millis(150)).await;
        assert!(
            evs.len() >= 4,
            "expected at least start + continuations, got {} events",
            evs.len()
        );

        // Pull timestamps + durations from each event, asserting the
        // monotone-by-160 duration progression while the timestamp
        // stays fixed.
        let mut timestamps = HashSet::new();
        let mut durations: Vec<u16> = Vec::new();
        for ev in &evs {
            if let RtpEvent::DtmfEvent {
                duration,
                timestamp,
                ..
            } = ev
            {
                timestamps.insert(*timestamp);
                durations.push(*duration);
            }
        }
        assert_eq!(
            timestamps.len(),
            1,
            "all DTMF packets in one tone must share start timestamp, got {:?}",
            timestamps
        );
        // Durations must be strictly increasing in 160-sample steps.
        for w in durations.windows(2) {
            assert_eq!(
                w[1].saturating_sub(w[0]),
                SAMPLES_PER_TICK,
                "duration must grow by one tick (160 samples) per continuation"
            );
        }
    }

    #[tokio::test]
    async fn three_end_of_event_packets_collapse_to_one() {
        let (session, mut rx) = pair().await;
        let tx = DtmfTransmitter::new(session.clone());
        let handle = tx.send_digit('#', 60);
        let _ = handle.await.expect("send task");

        let evs = drain_dtmf(&mut rx, Duration::from_millis(150)).await;
        let end_events: Vec<&RtpEvent> = evs
            .iter()
            .filter(|ev| matches!(ev, RtpEvent::DtmfEvent { end_of_event: true, .. }))
            .collect();
        assert_eq!(
            end_events.len(),
            1,
            "RFC 4733 §2.5.1.3 retransmits must dedup to one event, got {}: {:?}",
            end_events.len(),
            end_events
        );
        if let Some(RtpEvent::DtmfEvent { event, .. }) = end_events.first() {
            assert_eq!(*event, 11, "digit '#' encodes as event 11");
        }
    }
}
