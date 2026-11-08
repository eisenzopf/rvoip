//! Executor v2: the session's receive work, run inline on the task that owns
//! the socket.
//!
//! [`InlineSessionIngress`] performs, for each RTP media packet, exactly what
//! the v1 session receive task does for `RtpEvent::MediaReceived`
//! (`session/mod.rs`, the `MediaReceived` arm of `recv_task`): stream lookup
//! or creation (with retained SR reconciliation), sequence/loss/duplicate/
//! reorder/jitter accounting, session statistics deltas, `NewStreamDetected`,
//! and the polling-queue mirror. It then hands
//! the packet to the installed [`InlineMediaSink`] instead of publishing
//! `PacketReceived` — or publishes `PacketReceived` exactly as v1 does when
//! no sink is installed or the sink demoted itself.
//!
//! RTCP, DTMF, STUN and transport errors still reach the v1 receive task
//! through the transport broadcast.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{broadcast, mpsc};
use tracing::{error, info, trace};

use super::{ReceivedSenderReport, RtpSessionEvent, RtpSessionStats, RtpStream};
use crate::executor::{
    record_inline_fallback, record_inline_sink_active, HandoffSlot, InlineFallbackReason,
    InlineMediaSink, InlinePacketCounters, InlineRtpIngress, InlineRtpPacket, InlineSinkVerdict,
};
use crate::packet::RtpPacket;

pub(super) type InlineSinkSlot = HandoffSlot<Box<dyn InlineMediaSink>>;

/// Where packets go after session accounting.
enum Downstream {
    /// No sink yet: publish `PacketReceived` (v1 behaviour).
    Broadcast,
    /// A sink owns the packets.
    Sink(Box<dyn InlineMediaSink>),
    /// The sink was refused or demoted: publish `PacketReceived` for the rest
    /// of the session.
    Fallback,
}

pub(super) struct InlineSessionIngress {
    streams: Arc<super::SsrcMap<RtpStream>>,
    received_sender_reports: Arc<super::SsrcMap<ReceivedSenderReport>>,
    clock_rate: Arc<AtomicU32>,
    stats: Arc<parking_lot::Mutex<RtpSessionStats>>,
    event_tx: broadcast::Sender<RtpSessionEvent>,
    receive_queue: Option<mpsc::Sender<RtpPacket>>,
    sink_slot: Arc<InlineSinkSlot>,
    downstream: Downstream,
    /// Packets published as `PacketReceived` so far. A sink offered after
    /// this is non-zero would split one stream across two consumers.
    broadcast_packets: u64,
    counters: Option<InlinePacketCounters>,
}

impl InlineSessionIngress {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        streams: Arc<super::SsrcMap<RtpStream>>,
        received_sender_reports: Arc<super::SsrcMap<ReceivedSenderReport>>,
        clock_rate: Arc<AtomicU32>,
        stats: Arc<parking_lot::Mutex<RtpSessionStats>>,
        event_tx: broadcast::Sender<RtpSessionEvent>,
        receive_queue: Option<mpsc::Sender<RtpPacket>>,
        sink_slot: Arc<InlineSinkSlot>,
    ) -> Self {
        Self {
            streams,
            received_sender_reports,
            clock_rate,
            stats,
            event_tx,
            receive_queue,
            sink_slot,
            downstream: Downstream::Broadcast,
            broadcast_packets: 0,
            counters: None,
        }
    }

    /// Adopt a sink offered by the control plane, once, before any packet
    /// has been published to the v1 consumer.
    fn poll_sink_slot(&mut self) {
        if !self.sink_slot.is_dirty() {
            return;
        }
        let Some(sink) = self.sink_slot.take() else {
            return;
        };
        match self.downstream {
            Downstream::Broadcast if self.broadcast_packets == 0 => {
                record_inline_sink_active();
                self.counters = Some(InlinePacketCounters::new());
                self.downstream = Downstream::Sink(sink);
            }
            _ => {
                // Packets already reached the v1 consumer (or a sink was
                // demoted); keep one consumer per stream.
                drop(sink);
                record_inline_fallback(InlineFallbackReason::LateInstall);
                self.downstream = Downstream::Fallback;
            }
        }
    }

    fn publish(&mut self, packet: RtpPacket) {
        self.broadcast_packets = self.broadcast_packets.saturating_add(1);
        let _ = self.event_tx.send(RtpSessionEvent::PacketReceived(packet));
    }

    fn apply(&mut self, verdict: InlineSinkVerdict, current: Option<&InlineRtpPacket<'_>>) {
        match verdict {
            InlineSinkVerdict::Continue => {}
            InlineSinkVerdict::Demote { reason, held } => {
                // Dropping the sink here releases its state on the owner.
                self.downstream = Downstream::Fallback;
                record_inline_fallback(reason);
                if let Some(counters) = self.counters.as_mut() {
                    counters.flush();
                }
                for packet in held {
                    self.publish(packet);
                }
                if let Some(packet) = current {
                    self.publish(packet.to_packet());
                }
            }
        }
    }

    /// The v1 `MediaReceived` accounting, field for field. Returns whether
    /// this packet created the stream.
    fn account(&mut self, packet: &InlineRtpPacket<'_>, now: Instant) -> bool {
        let ssrc = packet.ssrc;
        let received_sender_reports = &self.received_sender_reports;
        let clock_rate = &self.clock_rate;
        let mut created = false;
        let mut entry = self.streams.entry(ssrc).or_insert_with(|| {
            created = true;
            info!("New RTP stream detected with SSRC={:08x}", ssrc);
            let mut stream = RtpStream::new(ssrc, clock_rate.load(Ordering::Relaxed));
            if let Some(sender_report) = received_sender_reports.get(&ssrc) {
                stream.update_last_sr_info(sender_report.lsr, sender_report.received_at);
            }
            stream
        });

        let before = entry.get_stats();
        debug_assert!(!entry.has_jitter_buffer(), "session streams are unbuffered");
        entry.account_packet_at(
            packet.sequence_number,
            packet.timestamp,
            packet.accounted_size(),
            now,
        );
        let after = entry.get_stats();
        let jitter_ms = entry.get_jitter_ms();
        drop(entry);

        let mut session_stats = self.stats.lock();
        session_stats.packets_received += 1;
        session_stats.bytes_received +=
            packet.payload.len() as u64 + 12 + u64::from(packet.padding_size);
        session_stats.packets_lost = session_stats
            .packets_lost
            .saturating_sub(before.packets_lost)
            .saturating_add(after.packets_lost);
        session_stats.packets_duplicated = session_stats
            .packets_duplicated
            .saturating_sub(before.duplicates)
            .saturating_add(after.duplicates);
        session_stats.packets_out_of_order = session_stats
            .packets_out_of_order
            .saturating_sub(before.packets_out_of_order)
            .saturating_add(after.packets_out_of_order);
        session_stats.jitter_ms = jitter_ms;
        session_stats.remote_addr = Some(packet.source);
        drop(session_stats);
        created
    }
}

impl InlineRtpIngress for InlineSessionIngress {
    fn on_rtp(&mut self, packet: &InlineRtpPacket<'_>, now: Instant) {
        self.poll_sink_slot();
        let is_new_stream = self.account(packet, now);
        if is_new_stream {
            let _ = self
                .event_tx
                .send(RtpSessionEvent::NewStreamDetected { ssrc: packet.ssrc });
        }

        if let Some(queue) = self.receive_queue.as_ref() {
            match queue.try_send(packet.to_packet()) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    trace!("RTP receive polling queue full; dropping duplicate packet");
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    error!("Failed to forward RTP packet to receiver: channel closed");
                }
            }
        }

        let verdict = match &mut self.downstream {
            Downstream::Sink(sink) => Some(sink.on_packet(packet, now)),
            Downstream::Broadcast | Downstream::Fallback => None,
        };
        match verdict {
            Some(verdict) => {
                if matches!(verdict, InlineSinkVerdict::Continue) {
                    if let Some(counters) = self.counters.as_mut() {
                        counters.packet();
                    }
                }
                self.apply(verdict, Some(packet));
            }
            None => self.publish(packet.to_packet()),
        }
    }

    fn next_deadline(&mut self, now: Instant) -> Option<Instant> {
        match &mut self.downstream {
            Downstream::Sink(sink) => sink.next_deadline(now),
            Downstream::Broadcast | Downstream::Fallback => None,
        }
    }

    fn on_deadline(&mut self, now: Instant) {
        let verdict = match &mut self.downstream {
            Downstream::Sink(sink) => sink.on_deadline(now),
            Downstream::Broadcast | Downstream::Fallback => return,
        };
        self.apply(verdict, None);
    }
}

#[cfg(test)]
mod tests {
    //! v1/v2 equivalence for the session half of the inline executor. Each
    //! test builds the same session twice, with `MediaExecutorConfig::v1()`
    //! and `v2()` selected per thread, feeds both the same datagrams over
    //! loopback and compares what consumers observe.
    use super::*;
    use crate::executor::{
        inline_executor_counters, override_media_executor_config_on_this_thread,
        MediaExecutorConfig, MediaExecutorMode,
    };
    use crate::packet::RtpHeader;
    use crate::session::{RtpSession, RtpSessionConfig};
    use crate::srtp::{SrtpContext, SrtpCryptoKey, SRTP_AES128_CM_SHA1_80};
    use crate::transport::UdpRtpTransport;
    use bytes::Bytes;
    use std::net::SocketAddr;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;
    use tokio::net::UdpSocket;

    const REMOTE_SSRC: u32 = 0x00e4_0001;

    fn config_for(mode: MediaExecutorMode) -> MediaExecutorConfig {
        match mode {
            MediaExecutorMode::V2 => MediaExecutorConfig::v2(),
            _ => MediaExecutorConfig::v1(),
        }
    }

    async fn session_in(mode: MediaExecutorMode) -> RtpSession {
        let _mode = override_media_executor_config_on_this_thread(config_for(mode));
        let session = RtpSession::new_event_driven(RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            ssrc: Some(0x0102_0304),
            ..RtpSessionConfig::default()
        })
        .await
        .unwrap();
        assert_eq!(session.inline_rx_enabled(), mode == MediaExecutorMode::V2);
        session
    }

    fn media_packet(seq: u16) -> RtpPacket {
        RtpPacket::new(
            RtpHeader::new(0, seq, u32::from(seq) * 160, REMOTE_SSRC),
            Bytes::from(vec![seq as u8; 160]),
        )
    }

    async fn next_published(events: &mut broadcast::Receiver<RtpSessionEvent>) -> RtpPacket {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let RtpSessionEvent::PacketReceived(packet) = events.recv().await.unwrap() {
                    return packet;
                }
            }
        })
        .await
        .expect("timed out waiting for PacketReceived")
    }

    async fn assert_nothing_more_published(events: &mut broadcast::Receiver<RtpSessionEvent>) {
        let quiet = tokio::time::timeout(Duration::from_millis(150), async {
            loop {
                match events.recv().await {
                    Ok(RtpSessionEvent::PacketReceived(packet)) => return Some(packet),
                    Ok(_) => continue,
                    Err(_) => return None,
                }
            }
        })
        .await;
        assert!(
            !matches!(quiet, Ok(Some(_))),
            "unexpected PacketReceived {:?}",
            quiet
                .ok()
                .flatten()
                .map(|packet| packet.header.sequence_number)
        );
    }

    #[derive(Debug, PartialEq, Eq)]
    struct Observed {
        published: Vec<u16>,
        session: (u64, u64, u64, u64, u64),
        stream: (u32, u64, u64, u64, u64),
    }

    async fn observe(session: &RtpSession, published: Vec<u16>) -> Observed {
        let stats = session.get_stats();
        let stream = session.get_stream(REMOTE_SSRC).await.expect("stream");
        Observed {
            published,
            session: (
                stats.packets_received,
                stats.bytes_received,
                stats.packets_lost,
                stats.packets_duplicated,
                stats.packets_out_of_order,
            ),
            stream: (
                stream.highest_seq,
                stream.packets_received,
                stream.packets_lost,
                stream.duplicates,
                stream.packets_out_of_order,
            ),
        }
    }

    async fn run_plain(mode: MediaExecutorMode, sequence: &[u16]) -> Observed {
        let mut session = session_in(mode).await;
        let mut events = session.subscribe();
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let destination = session.local_addr().unwrap();
        for &seq in sequence {
            peer.send_to(&media_packet(seq).serialize().unwrap(), destination)
                .await
                .unwrap();
        }
        let mut published = Vec::new();
        for _ in sequence {
            let packet = next_published(&mut events).await;
            assert_eq!(packet, media_packet(packet.header.sequence_number));
            published.push(packet.header.sequence_number);
        }
        assert_nothing_more_published(&mut events).await;
        let observed = observe(&session, published).await;
        session.close().await.unwrap();
        observed
    }

    #[tokio::test]
    async fn in_order_reordered_lossy_and_duplicate_accounting_matches_v1() {
        // In order (10, 11), reordered (13 before 12), duplicate (12 again),
        // a loss (15 never arrives) and a late arrival (14 after 16).
        let sequence = [10_u16, 11, 13, 12, 12, 16, 14, 17];
        let v1 = run_plain(MediaExecutorMode::V1, &sequence).await;
        let v2 = run_plain(MediaExecutorMode::V2, &sequence).await;
        assert_eq!(v1, v2);
        assert_eq!(v2.published, sequence.to_vec());
        assert_eq!(v2.stream.2, 1, "exactly the missing sequence 15 is lost");
        assert_eq!(v2.stream.3, 1, "one duplicate");
    }

    /// One random SRTP master key and salt per test process: generated at
    /// run time rather than hard-coded, and shared by every context a test
    /// builds so both ends of a call agree.
    fn test_srtp_key() -> &'static ([u8; 16], [u8; 14]) {
        use rand::Rng;
        static KEY: std::sync::OnceLock<([u8; 16], [u8; 14])> = std::sync::OnceLock::new();
        KEY.get_or_init(|| (rand::thread_rng().gen(), rand::thread_rng().gen()))
    }

    fn srtp_context() -> SrtpContext {
        SrtpContext::new(
            SRTP_AES128_CM_SHA1_80,
            SrtpCryptoKey::new(test_srtp_key().0.to_vec(), test_srtp_key().1.to_vec()),
        )
        .unwrap()
    }

    async fn run_srtp(mode: MediaExecutorMode) -> Observed {
        let mut session = session_in(mode).await;
        session
            .transport
            .as_any()
            .downcast_ref::<UdpRtpTransport>()
            .expect("UDP session transport")
            .set_srtp_contexts(srtp_context(), srtp_context())
            .await
            .unwrap();
        let mut sender = srtp_context();
        let mut wires = Vec::new();
        for seq in [1_u16, 2, 4, 3] {
            wires.push(
                sender
                    .protect(&media_packet(seq))
                    .unwrap()
                    .serialize()
                    .unwrap()
                    .to_vec(),
            );
        }
        // Replay of an accepted packet, then a tampered packet followed by
        // its genuine twin: both rejections must leave no trace.
        wires.push(wires[1].clone());
        let genuine = sender
            .protect(&media_packet(5))
            .unwrap()
            .serialize()
            .unwrap()
            .to_vec();
        let mut tampered = genuine.clone();
        let middle = tampered.len() / 2;
        tampered[middle] ^= 0x80;
        wires.push(tampered);
        wires.push(genuine);

        let mut events = session.subscribe();
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let destination = session.local_addr().unwrap();
        for wire in &wires {
            peer.send_to(wire, destination).await.unwrap();
        }
        let mut published = Vec::new();
        for _ in 0..5 {
            let packet = next_published(&mut events).await;
            assert_eq!(packet, media_packet(packet.header.sequence_number));
            published.push(packet.header.sequence_number);
        }
        assert_nothing_more_published(&mut events).await;
        let observed = observe(&session, published).await;
        session.close().await.unwrap();
        observed
    }

    #[tokio::test]
    async fn srtp_in_place_path_matches_v1_for_reorder_replay_and_tamper() {
        let v1 = run_srtp(MediaExecutorMode::V1).await;
        let v2 = run_srtp(MediaExecutorMode::V2).await;
        assert_eq!(v1, v2);
        assert_eq!(v2.published, vec![1, 2, 4, 3, 5]);
        assert_eq!(
            v2.session.0, 5,
            "replayed and tampered packets are never counted"
        );
    }

    /// Records inline packets; optionally demotes on the nth packet.
    struct RecordingSink {
        seen: Arc<parking_lot::Mutex<Vec<u16>>>,
        dropped: Arc<AtomicBool>,
        demote_on: Option<usize>,
        held: Vec<RtpPacket>,
    }

    impl Drop for RecordingSink {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::Release);
        }
    }

    impl InlineMediaSink for RecordingSink {
        fn on_packet(&mut self, packet: &InlineRtpPacket<'_>, _now: Instant) -> InlineSinkVerdict {
            let mut seen = self.seen.lock();
            if self.demote_on == Some(seen.len()) {
                return InlineSinkVerdict::Demote {
                    reason: InlineFallbackReason::Demoted,
                    held: std::mem::take(&mut self.held),
                };
            }
            assert_eq!(packet.payload, &[packet.sequence_number as u8; 160][..]);
            seen.push(packet.sequence_number);
            InlineSinkVerdict::Continue
        }

        fn next_deadline(&mut self, _now: Instant) -> Option<Instant> {
            None
        }

        fn on_deadline(&mut self, _now: Instant) -> InlineSinkVerdict {
            InlineSinkVerdict::Continue
        }
    }

    fn recording_sink(
        demote_on: Option<usize>,
        held: Vec<RtpPacket>,
    ) -> (
        Box<RecordingSink>,
        Arc<parking_lot::Mutex<Vec<u16>>>,
        Arc<AtomicBool>,
    ) {
        let seen = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let dropped = Arc::new(AtomicBool::new(false));
        let sink = Box::new(RecordingSink {
            seen: seen.clone(),
            dropped: dropped.clone(),
            demote_on,
            held,
        });
        (sink, seen, dropped)
    }

    async fn wait_for(condition: impl Fn() -> bool) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while !condition() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("condition not reached");
    }

    async fn send_all(peer: &UdpSocket, destination: SocketAddr, sequence: &[u16]) {
        for &seq in sequence {
            peer.send_to(&media_packet(seq).serialize().unwrap(), destination)
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn sink_receives_packets_inline_without_broadcast_and_is_released_by_close() {
        let before = inline_executor_counters();
        let mut session = session_in(MediaExecutorMode::V2).await;
        let (sink, seen, dropped) = recording_sink(None, Vec::new());
        assert!(session.install_inline_media_sink(sink).is_ok());
        let (second, _, second_dropped) = recording_sink(None, Vec::new());
        assert!(
            session.install_inline_media_sink(second).is_err(),
            "one sink per session"
        );
        assert!(
            second_dropped.load(Ordering::Acquire),
            "the refused sink is returned"
        );

        let mut events = session.subscribe();
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let destination = session.local_addr().unwrap();
        send_all(&peer, destination, &[1, 2, 4, 3]).await;
        wait_for(|| seen.lock().len() == 4).await;
        assert_eq!(
            *seen.lock(),
            vec![1, 2, 4, 3],
            "arrival order, accounted inline"
        );
        assert_nothing_more_published(&mut events).await;
        let stats = session.get_stats();
        assert_eq!((stats.packets_received, stats.packets_out_of_order), (4, 1));

        assert!(!dropped.load(Ordering::Acquire));
        session.close().await.unwrap();
        assert!(
            dropped.load(Ordering::Acquire),
            "closing the transport joins the owner task and drops the sink"
        );
        let after = inline_executor_counters();
        assert!(after.inline_packets >= before.inline_packets + 4);
        assert!(after.wakeups_avoided >= before.wakeups_avoided + 8);
        assert!(after.inline_sinks > before.inline_sinks);
    }

    #[tokio::test]
    async fn demotion_republishes_held_packets_then_stays_on_v1() {
        let before = inline_executor_counters();
        let mut session = session_in(MediaExecutorMode::V2).await;
        // Demote on the third packet, handing back one packet it "held".
        let (sink, seen, dropped) = recording_sink(Some(2), vec![media_packet(40)]);
        assert!(session.install_inline_media_sink(sink).is_ok());
        let mut events = session.subscribe();
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let destination = session.local_addr().unwrap();
        send_all(&peer, destination, &[41, 42, 43, 44]).await;
        let mut published = Vec::new();
        for _ in 0..3 {
            published.push(next_published(&mut events).await.header.sequence_number);
        }
        assert_eq!(published, vec![40, 43, 44]);
        assert_eq!(*seen.lock(), vec![41, 42]);
        assert!(
            dropped.load(Ordering::Acquire),
            "a demoted sink is released at once"
        );
        assert!(inline_executor_counters().fallbacks > before.fallbacks);
        session.close().await.unwrap();
    }

    #[tokio::test]
    async fn sink_offered_after_media_was_published_is_refused_on_the_owner() {
        let before = inline_executor_counters();
        let mut session = session_in(MediaExecutorMode::V2).await;
        let mut events = session.subscribe();
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let destination = session.local_addr().unwrap();
        send_all(&peer, destination, &[1]).await;
        assert_eq!(next_published(&mut events).await.header.sequence_number, 1);

        let (sink, seen, dropped) = recording_sink(None, Vec::new());
        assert!(session.install_inline_media_sink(sink).is_ok());
        send_all(&peer, destination, &[2]).await;
        assert_eq!(next_published(&mut events).await.header.sequence_number, 2);
        assert!(seen.lock().is_empty());
        assert!(dropped.load(Ordering::Acquire));
        assert!(inline_executor_counters().fallbacks > before.fallbacks);
        session.close().await.unwrap();
    }

    // ---- Owner SRTP: v1/v2 equivalence through the transport's real API ---

    /// The keys a rekey moves to: a second random master key and salt per
    /// test process, independent of [`test_srtp_key`].
    fn rekeyed_context() -> SrtpContext {
        use rand::Rng;
        static KEY: std::sync::OnceLock<([u8; 16], [u8; 14])> = std::sync::OnceLock::new();
        let (key, salt) = KEY.get_or_init(|| (rand::thread_rng().gen(), rand::thread_rng().gen()));
        SrtpContext::new(
            SRTP_AES128_CM_SHA1_80,
            SrtpCryptoKey::new(key.to_vec(), salt.to_vec()),
        )
        .unwrap()
    }

    fn udp(session: &RtpSession) -> &UdpRtpTransport {
        session
            .transport
            .as_any()
            .downcast_ref::<UdpRtpTransport>()
            .expect("UDP session transport")
    }

    fn protected(sender: &mut SrtpContext, seq: u16) -> Vec<u8> {
        sender
            .protect(&media_packet(seq))
            .unwrap()
            .serialize()
            .unwrap()
            .to_vec()
    }

    async fn publish_all(
        events: &mut broadcast::Receiver<RtpSessionEvent>,
        count: usize,
        published: &mut Vec<u16>,
    ) {
        for _ in 0..count {
            let packet = next_published(events).await;
            assert_eq!(packet, media_packet(packet.header.sequence_number));
            published.push(packet.header.sequence_number);
        }
    }

    /// Rekey mid-stream through the transport's control API, then roll back:
    /// old keys after the rekey, cross-key and same-key replays, and a replay
    /// after the rollback must all be rejected, identically.
    async fn run_srtp_rekey(mode: MediaExecutorMode) -> Observed {
        let mut session = session_in(mode).await;
        udp(&session)
            .set_srtp_contexts(srtp_context(), srtp_context())
            .await
            .unwrap();
        let mut events = session.subscribe();
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let destination = session.local_addr().unwrap();
        let mut a = srtp_context();
        let mut b = rekeyed_context();
        let mut published = Vec::new();

        let a_wires: Vec<Vec<u8>> = (1..=8).map(|seq| protected(&mut a, seq)).collect();
        for wire in &a_wires[..3] {
            peer.send_to(wire, destination).await.unwrap();
        }
        publish_all(&mut events, 3, &mut published).await;
        assert_eq!(
            udp(&session).srtp_recv_location(),
            (mode == MediaExecutorMode::V2, mode == MediaExecutorMode::V1),
            "{mode:?}: the v2 receive loop holds the context"
        );

        let rollback = udp(&session)
            .replace_srtp_contexts(rekeyed_context(), rekeyed_context())
            .await
            .unwrap();
        // Old key after the rekey, then the new key.
        peer.send_to(&a_wires[3], destination).await.unwrap();
        for seq in [5_u16, 6] {
            peer.send_to(&protected(&mut b, seq), destination)
                .await
                .unwrap();
        }
        publish_all(&mut events, 2, &mut published).await;

        udp(&session)
            .rollback_srtp_contexts(rollback)
            .await
            .unwrap();
        // A replay of an A packet accepted before the rekey (A's window is
        // the one frozen at the rekey), a B packet after the rollback, then
        // A continues.
        peer.send_to(&a_wires[1], destination).await.unwrap();
        peer.send_to(&protected(&mut b, 7), destination)
            .await
            .unwrap();
        peer.send_to(&a_wires[7], destination).await.unwrap();
        publish_all(&mut events, 1, &mut published).await;
        assert_nothing_more_published(&mut events).await;
        assert!(udp(&session).srtp_enabled().await);

        let observed = observe(&session, published).await;
        let transport = session.transport.clone();
        session.close().await.unwrap();
        let transport = transport
            .as_any()
            .downcast_ref::<UdpRtpTransport>()
            .expect("UDP session transport");
        assert_eq!(
            transport.srtp_recv_location(),
            (false, true),
            "after close the inbound context is back home, never lost"
        );
        observed
    }

    #[tokio::test]
    async fn srtp_rekey_and_rollback_mid_stream_match_v1() {
        let before = inline_executor_counters();
        let v1 = run_srtp_rekey(MediaExecutorMode::V1).await;
        let v2 = run_srtp_rekey(MediaExecutorMode::V2).await;
        assert_eq!(v1, v2);
        assert_eq!(v2.published, vec![1, 2, 3, 5, 6, 8]);
        let after = inline_executor_counters();
        assert!(after.owner_srtp_loops > before.owner_srtp_loops);
        // The first check-out, the rekey and the rollback.
        assert!(after.owner_srtp_handoffs >= before.owner_srtp_handoffs + 3);
        assert!(after.srtp_locks_avoided >= before.srtp_locks_avoided);
    }

    #[tokio::test]
    async fn owner_srtp_can_be_switched_off_and_never_applies_with_a_separate_rtcp_socket() {
        // Switched off: v2 receive, but the context stays behind the lock.
        let session = {
            let _off = override_media_executor_config_on_this_thread(
                MediaExecutorConfig::v2().with_owner_srtp(false),
            );
            RtpSession::new_event_driven(RtpSessionConfig {
                local_addr: "127.0.0.1:0".parse().unwrap(),
                ..RtpSessionConfig::default()
            })
            .await
            .unwrap()
        };
        // A separate RTCP socket: its receive task unprotects SRTCP with the
        // same context, so the RTP loop must not own it.
        let rtcp_port = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let rtcp_addr = rtcp_port.local_addr().unwrap();
        drop(rtcp_port);
        let non_mux = {
            let _v2 = override_media_executor_config_on_this_thread(MediaExecutorConfig::v2());
            RtpSession::new_event_driven_with_rtcp_socket(
                RtpSessionConfig {
                    local_addr: "127.0.0.1:0".parse().unwrap(),
                    ..RtpSessionConfig::default()
                },
                crate::transport::SymmetricRtpPolicy::default(),
                rtcp_addr,
            )
            .await
            .unwrap()
        };
        for mut session in [session, non_mux] {
            assert!(session.inline_rx_enabled());
            udp(&session)
                .set_srtp_contexts(srtp_context(), srtp_context())
                .await
                .unwrap();
            let mut events = session.subscribe();
            let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let mut sender = srtp_context();
            for seq in [1_u16, 2] {
                peer.send_to(&protected(&mut sender, seq), session.local_addr().unwrap())
                    .await
                    .unwrap();
            }
            let mut published = Vec::new();
            publish_all(&mut events, 2, &mut published).await;
            assert_eq!(published, vec![1, 2]);
            assert_eq!(udp(&session).srtp_recv_location(), (false, true));
            session.close().await.unwrap();
        }
    }

    #[tokio::test]
    async fn v1_sessions_never_offer_an_inline_seam() {
        let session = session_in(MediaExecutorMode::V1).await;
        let (sink, _, dropped) = recording_sink(None, Vec::new());
        assert!(session.install_inline_media_sink(sink).is_err());
        assert!(
            dropped.load(Ordering::Acquire),
            "the refused sink is returned and dropped"
        );
    }
}
