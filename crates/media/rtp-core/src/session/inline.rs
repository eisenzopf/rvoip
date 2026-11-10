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
//! DTMF, STUN and transport errors still reach the v1 receive task through
//! the transport broadcast. Multiplexed RTCP does too, unless
//! [`MediaExecutorConfig::owner_rtcp`] hands it to
//! [`InlineSessionIngress`]'s `on_rtcp`, which runs the session task's RTCP
//! handling (`RtcpIngress::process`) on the socket owner.
//!
//! [`MediaExecutorConfig::owner_rtcp`]: crate::executor::MediaExecutorConfig::owner_rtcp

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{broadcast, mpsc};
use tracing::{error, info, trace};

use super::receive_accounting::ReceiveAccountingWriter;
use super::{ReceivedSenderReport, RtcpIngress, RtpSessionEvent, RtpSessionStats, RtpStream};
use crate::executor::{
    record_inline_fallback, record_inline_sink_active, record_owner_rtcp_inbound, HandoffSlot,
    InlineFallbackReason, InlineMediaSink, InlinePacketCounters, InlineRtpIngress, InlineRtpPacket,
    InlineSinkVerdict,
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
    /// The session's RTCP handling, with `owner_rtcp`.
    rtcp: Option<RtcpIngress>,
    /// With `owner_receive_stats`: the per-packet receive statistics are
    /// published here, without the `stats` mutex.
    receive: Option<ReceiveAccountingWriter>,
    /// Hot call state, decided when the ingress is built: streams it creates
    /// skip their write-only loss history, and the sink slot is no longer
    /// polled once it is settled.
    hot_state: bool,
    /// Hot call state: the sink slot delivered its one value (a slot accepts
    /// a single offer, so it can never become dirty again); its cache line
    /// is not read per packet any more.
    sink_settled: bool,
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
        rtcp: Option<RtcpIngress>,
        receive: Option<ReceiveAccountingWriter>,
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
            rtcp,
            receive,
            hot_state: crate::executor::hot_state_enabled(),
            sink_settled: false,
        }
    }

    /// Adopt a sink offered by the control plane, once, before any packet
    /// has been published to the v1 consumer.
    fn poll_sink_slot(&mut self) {
        if self.sink_settled || !self.sink_slot.is_dirty() {
            return;
        }
        let Some(sink) = self.sink_slot.take() else {
            return;
        };
        // Hot call state: `HandoffSlot::offer` succeeds once, so after this
        // take the slot stays clean for the rest of the session.
        self.sink_settled = self.hot_state;
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
        // The jitter-reset generation, read before the stream is touched
        // (see `receive_accounting`).
        let jitter_generation = self
            .receive
            .as_ref()
            .map_or(0, ReceiveAccountingWriter::begin);
        let received_sender_reports = &self.received_sender_reports;
        let clock_rate = &self.clock_rate;
        let hot_state = self.hot_state;
        let mut created = false;
        let mut entry = self.streams.entry(ssrc).or_insert_with(|| {
            created = true;
            info!("New RTP stream detected with SSRC={:08x}", ssrc);
            let mut stream = RtpStream::new(ssrc, clock_rate.load(Ordering::Relaxed));
            if hot_state {
                stream.disable_loss_history();
            }
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

        if let Some(receive) = self.receive.as_mut() {
            receive.account(
                packet.payload.len() as u64 + 12 + u64::from(packet.padding_size),
                [
                    before.packets_lost,
                    before.duplicates,
                    before.packets_out_of_order,
                ],
                [
                    after.packets_lost,
                    after.duplicates,
                    after.packets_out_of_order,
                ],
                jitter_ms,
                jitter_generation,
            );
            receive.remote(packet.source, &self.stats);
            return created;
        }

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

    /// Hot call state: the inline sink's state, which serves every deadline.
    fn prefetch_hint(&self) -> usize {
        match &self.downstream {
            Downstream::Sink(sink) if self.hot_state => {
                &**sink as *const dyn InlineMediaSink as *const u8 as usize
            }
            _ => 0,
        }
    }

    /// With `owner_rtcp`: exactly the session task's `RtcpReceived` arm,
    /// run on the socket owner.
    fn on_rtcp(&mut self, data: &[u8], source: SocketAddr, _now: Instant) -> bool {
        let Some(rtcp) = self.rtcp.as_ref() else {
            return false;
        };
        rtcp.process(data, source);
        record_owner_rtcp_inbound();
        true
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

    thread_local! {
        /// Hot call state for the v2 sessions the helpers build on this
        /// thread (`None`: the preset's own setting); see [`with_hot`].
        static HOT_ARM: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
    }

    fn config_for(mode: MediaExecutorMode) -> MediaExecutorConfig {
        match mode {
            MediaExecutorMode::V2 => {
                let v2 = MediaExecutorConfig::v2();
                match HOT_ARM.with(std::cell::Cell::get) {
                    Some(hot) => v2.with_hot_state(hot),
                    None => v2,
                }
            }
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

    /// With the socket owner publishing receive statistics outside the
    /// mutex, every field a reader sees equals v1's: counters, the last
    /// source address (also after `set_remote_addr`), and jitter zeroed by a
    /// clock change until the next packet.
    #[tokio::test]
    async fn owner_published_receive_statistics_match_v1_across_a_clock_reset() {
        type Key = (u64, u64, u64, u64, u64, bool);
        fn key(stats: &RtpSessionStats, peer: SocketAddr) -> Key {
            (
                stats.packets_received,
                stats.bytes_received,
                stats.packets_lost,
                stats.packets_duplicated,
                stats.packets_out_of_order,
                stats.remote_addr == Some(peer),
            )
        }
        let mut results = Vec::new();
        for mode in [MediaExecutorMode::V1, MediaExecutorMode::V2] {
            let before = inline_executor_counters();
            let mut session = session_in(mode).await;
            let mut events = session.subscribe();
            let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let peer_addr = peer.local_addr().unwrap();
            let elsewhere: SocketAddr = "192.0.2.7:4000".parse().unwrap();
            let destination = session.local_addr().unwrap();
            // In order, a gap, a late arrival and a duplicate.
            for seq in [20_u16, 21, 23, 22, 22] {
                send_all(&peer, destination, &[seq]).await;
                let _ = next_published(&mut events).await;
            }
            let first = session.get_stats();
            let jitter_before_reset = first.jitter_ms;
            session.set_clock_rate(8_000);
            let reset = session.get_stats();
            // The control plane moves the remote address; the next packet's
            // source wins again, as in v1.
            session.set_remote_addr(elsewhere).await;
            let moved = session.get_stats();
            for seq in [24_u16, 25] {
                send_all(&peer, destination, &[seq]).await;
                let _ = next_published(&mut events).await;
            }
            let last = session.get_stats();
            let after = inline_executor_counters();
            if mode == MediaExecutorMode::V2 {
                assert!(after.owner_receive_stats > before.owner_receive_stats);
            }
            session.close().await.unwrap();
            assert_eq!(
                reset.jitter_ms, 0.0,
                "{mode:?}: a clock change zeroes jitter"
            );
            assert!(jitter_before_reset >= 0.0);
            assert_eq!(moved.remote_addr, Some(elsewhere), "{mode:?}");
            results.push((
                key(&first, peer_addr),
                key(&reset, peer_addr),
                key(&moved, peer_addr),
                key(&last, peer_addr),
            ));
        }
        assert_eq!(
            results[1].3,
            (7, 7 * 172, 0, 1, 1, true),
            "v2: seven packets, one duplicate, one late, from the peer"
        );
        assert_eq!(
            results[0], results[1],
            "v1 and v2 readers see the same statistics"
        );
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

    // ---- Hot call state: v2 with it on equals v2 with it off -------------

    /// Run `scenario` on executor v2 with hot call state forced to `hot` for
    /// everything built on this thread (sessions, transports, SRTP
    /// contexts); the receive loops themselves run on the media lanes.
    async fn with_hot<T, F>(hot: bool, scenario: impl FnOnce() -> F) -> T
    where
        F: std::future::Future<Output = T>,
    {
        HOT_ARM.with(|arm| arm.set(Some(hot)));
        let result = {
            let _config = override_media_executor_config_on_this_thread(
                MediaExecutorConfig::v2().with_hot_state(hot),
            );
            scenario().await
        };
        HOT_ARM.with(|arm| arm.set(None));
        result
    }

    #[tokio::test]
    async fn hot_state_matches_for_loss_reorder_duplicates_and_late_packets() {
        let sequence = [10_u16, 11, 13, 12, 12, 16, 14, 17, 20, 19, 19, 21];
        let before = crate::hot_state::hot_state_counters();
        let off = with_hot(false, || run_plain(MediaExecutorMode::V2, &sequence)).await;
        let on = with_hot(true, || run_plain(MediaExecutorMode::V2, &sequence)).await;
        assert_eq!(off, on);
        assert_eq!(on.published, sequence.to_vec());
        let after = crate::hot_state::hot_state_counters();
        assert!(
            after.rx_lane_entries + after.rx_local_entries
                > before.rx_lane_entries + before.rx_local_entries,
            "the hot run placed a receive-loop entry"
        );
    }

    #[tokio::test]
    async fn hot_state_matches_for_srtp_reorder_replay_and_tamper() {
        let before = crate::hot_state::hot_state_counters().srtp_contexts;
        let off = with_hot(false, || run_srtp(MediaExecutorMode::V2)).await;
        let on = with_hot(true, || run_srtp(MediaExecutorMode::V2)).await;
        assert_eq!(off, on);
        assert_eq!(on.published, vec![1, 2, 4, 3, 5]);
        assert!(
            crate::hot_state::hot_state_counters().srtp_contexts > before,
            "the hot run packed its SRTP contexts"
        );
    }

    /// A rekey and a rollback move the inbound SRTP epoch; with hot call
    /// state the receive loop sees them only through the doorbell, so this
    /// is the test that a ring is never missed.
    #[tokio::test]
    async fn hot_state_sees_srtp_rekey_and_rollback_through_the_doorbell() {
        let off = with_hot(false, || run_srtp_rekey(MediaExecutorMode::V2)).await;
        let on = with_hot(true, || run_srtp_rekey(MediaExecutorMode::V2)).await;
        assert_eq!(off, on);
    }

    #[tokio::test]
    async fn hot_state_matches_for_owner_rtcp() {
        for srtp in [false, true] {
            let off = with_hot(false, || run_inbound_rtcp(MediaExecutorMode::V2, srtp)).await;
            let on = with_hot(true, || run_inbound_rtcp(MediaExecutorMode::V2, srtp)).await;
            assert_eq!(off, on, "srtp={srtp}");
        }
    }

    /// A latch move (the far end changes its source port mid-call, passes
    /// probation and is rebound), then teardown: the inline sink sees the
    /// same packets in the same order, the session the same statistics, the
    /// transport the same destination, and close releases the sink.
    async fn run_latch_move_and_teardown(hot: bool) -> (Vec<u16>, (u64, u64, u64), bool, bool) {
        let mut session = session_in(MediaExecutorMode::V2).await;
        assert_eq!(udp(&session).hot_state_enabled(), hot);
        let (sink, seen, dropped) = recording_sink(None, Vec::new());
        assert!(session.install_inline_media_sink(sink).is_ok());
        let first = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let moved = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let destination = session.local_addr().unwrap();
        send_all(&first, destination, &[1, 2, 3]).await;
        wait_for(|| seen.lock().len() == 3).await;
        // Probation on the new port, then established.
        send_all(&moved, destination, &[4, 5, 6, 7, 8]).await;
        wait_for(|| seen.lock().len() >= 6).await;
        // The old source is now a stranger.
        send_all(&first, destination, &[9]).await;
        send_all(&moved, destination, &[10]).await;
        wait_for(|| seen.lock().contains(&10)).await;
        let stats = session.get_stats();
        let rebound = udp(&session).current_remote_rtp_addr() == Some(moved.local_addr().unwrap());
        assert!(!dropped.load(Ordering::Acquire));
        session.close().await.unwrap();
        let observed = seen.lock().clone();
        (
            observed,
            (
                stats.packets_received,
                stats.packets_lost,
                stats.packets_out_of_order,
            ),
            rebound,
            dropped.load(Ordering::Acquire),
        )
    }

    #[tokio::test]
    async fn hot_state_matches_for_a_latch_move_and_teardown() {
        let off = with_hot(false, || run_latch_move_and_teardown(false)).await;
        let on = with_hot(true, || run_latch_move_and_teardown(true)).await;
        assert_eq!(off, on);
        assert!(on.2, "the rebind took effect");
        assert!(on.3, "close released the sink");
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
        // same context, so the RTP loop must not own it. The session binds
        // the RTCP socket on an ephemeral port itself: probing for a free
        // port and binding it again later races parallel tests for it.
        let non_mux = {
            let _v2 = override_media_executor_config_on_this_thread(MediaExecutorConfig::v2());
            RtpSession::new_event_driven_with_rtcp_socket(
                RtpSessionConfig {
                    local_addr: "127.0.0.1:0".parse().unwrap(),
                    ..RtpSessionConfig::default()
                },
                crate::transport::SymmetricRtpPolicy::default(),
                "127.0.0.1:0".parse().unwrap(),
            )
            .await
            .unwrap()
        };
        let rtcp_addr = non_mux.local_rtcp_addr().expect("a separate RTCP socket");
        assert_ne!(rtcp_addr.port(), 0);
        assert_ne!(Some(rtcp_addr), non_mux.local_addr().ok());
        assert!(!non_mux.rtcp_mux());
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

    // ---- Owner RTCP: v1/v2 equivalence ----------------------------------------

    const LOCAL_SSRC: u32 = 0x0102_0304;
    const STRANGER_SSRC: u32 = 0x0bad_0bad;

    /// The session events inbound RTCP from the peer produces, as comparable
    /// text. The session's own periodic reports (local SSRC) are ignored.
    fn remote_rtcp_event(event: &RtpSessionEvent) -> Option<String> {
        match event {
            RtpSessionEvent::RtcpSenderReport { ssrc, .. }
            | RtpSessionEvent::RtcpReceiverReport { ssrc, .. }
            | RtpSessionEvent::Bye { ssrc, .. }
                if *ssrc != LOCAL_SSRC =>
            {
                Some(format!("{event:?}"))
            }
            _ => None,
        }
    }

    async fn next_rtcp_events(
        events: &mut broadcast::Receiver<RtpSessionEvent>,
        count: usize,
    ) -> Vec<String> {
        tokio::time::timeout(Duration::from_secs(2), async {
            let mut seen = Vec::new();
            while seen.len() < count {
                if let Some(text) = remote_rtcp_event(&events.recv().await.unwrap()) {
                    seen.push(text);
                }
            }
            seen
        })
        .await
        .expect("timed out waiting for RTCP events")
    }

    async fn assert_no_more_rtcp_events(events: &mut broadcast::Receiver<RtpSessionEvent>) {
        let quiet = tokio::time::timeout(Duration::from_millis(150), async {
            loop {
                match events.recv().await {
                    Ok(event) => {
                        if let Some(text) = remote_rtcp_event(&event) {
                            return Some(text);
                        }
                    }
                    Err(_) => return None,
                }
            }
        })
        .await;
        assert!(
            !matches!(quiet, Ok(Some(_))),
            "unexpected RTCP event {quiet:?}"
        );
    }

    fn peer_sender_report() -> crate::packet::rtcp::RtcpSenderReport {
        use crate::packet::rtcp::{NtpTimestamp, RtcpReportBlock, RtcpSenderReport};
        let mut sr = RtcpSenderReport::new(REMOTE_SSRC);
        sr.ntp_timestamp = NtpTimestamp {
            seconds: 0xaaaa_1234,
            fraction: 0x5678_bbbb,
        };
        sr.rtp_timestamp = 4_242;
        sr.sender_packet_count = 17;
        sr.sender_octet_count = 2_720;
        let mut about_us = RtcpReportBlock::new(LOCAL_SSRC);
        about_us.cumulative_lost = 3;
        about_us.highest_seq = 77;
        sr.report_blocks.push(about_us);
        sr
    }

    /// (SR about us; RR about us + an unknown member + BYE; a stranger's RR).
    fn inbound_compounds() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        use crate::packet::rtcp::{
            RtcpCompoundMember, RtcpCompoundPacket, RtcpGoodbye, RtcpPacket, RtcpReceiverReport,
            RtcpTolerantCompoundPacket, RtcpUnknownPacket,
        };
        let sr = peer_sender_report();
        let about_us = sr.report_blocks[0].clone();
        let sender_report = RtcpCompoundPacket::new_with_sr(sr)
            .serialize()
            .unwrap()
            .to_vec();
        let mut rr = RtcpReceiverReport::new(REMOTE_SSRC);
        rr.add_report_block(about_us.clone());
        let receiver_report_and_bye = RtcpTolerantCompoundPacket {
            packets: vec![
                RtcpCompoundMember::Known(RtcpPacket::ReceiverReport(rr)),
                RtcpCompoundMember::Unknown(RtcpUnknownPacket {
                    packet_type: 205,
                    count: 1,
                    payload: Bytes::from_static(&[0xaa, 0xbb, 0xcc, 0xdd]),
                    padding: Bytes::new(),
                }),
                RtcpCompoundMember::Known(RtcpPacket::Goodbye(RtcpGoodbye::new_for_source(
                    REMOTE_SSRC,
                ))),
            ],
        }
        .serialize()
        .unwrap()
        .to_vec();
        let mut stranger = RtcpReceiverReport::new(STRANGER_SSRC);
        stranger.add_report_block(about_us);
        let stranger = RtcpCompoundPacket::new_with_rr(stranger)
            .serialize()
            .unwrap()
            .to_vec();
        (sender_report, receiver_report_and_bye, stranger)
    }

    type PeerReportKey = (u32, u8, i32, u32);

    #[derive(Debug, PartialEq)]
    struct RtcpObserved {
        events: Vec<String>,
        published: Vec<u16>,
        rtcp_packets_received: u64,
        rtcp_packets_rejected: u64,
        peer_report: Option<PeerReportKey>,
        peer_bye: Option<(u32, Option<String>)>,
        retained_sender_reports: usize,
        rtcp_members: usize,
    }

    /// A stranger's report (dropped), the peer's SR before its RTP, RTP, an
    /// SRTCP replay of the SR (dropped), then RR + unknown member + BYE.
    async fn run_inbound_rtcp(mode: MediaExecutorMode, srtp: bool) -> RtcpObserved {
        let mut session = session_in(mode).await;
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let stranger = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        // Only the session's peer may feed its RTCP handling.
        session.set_remote_addr(peer.local_addr().unwrap()).await;
        let mut sender = srtp_context();
        if srtp {
            udp(&session)
                .set_srtp_contexts(srtp_context(), srtp_context())
                .await
                .unwrap();
        }
        let mut stranger_sender = srtp_context();
        let mut events = session.subscribe();
        let destination = session.local_addr().unwrap();
        let (sender_report, receiver_report_and_bye, stranger_report) = inbound_compounds();
        let wire = |context: &mut SrtpContext, plain: &[u8]| -> Vec<u8> {
            if srtp {
                context.protect_rtcp(plain).unwrap().to_vec()
            } else {
                plain.to_vec()
            }
        };
        let stranger_wire = wire(&mut stranger_sender, &stranger_report);
        let sr_wire = wire(&mut sender, &sender_report);
        let rr_wire = wire(&mut sender, &receiver_report_and_bye);

        stranger.send_to(&stranger_wire, destination).await.unwrap();
        peer.send_to(&sr_wire, destination).await.unwrap();
        let mut seen = next_rtcp_events(&mut events, 1).await;
        let mut published = Vec::new();
        for seq in [1_u16, 2] {
            let rtp = if srtp {
                protected(&mut sender, seq)
            } else {
                media_packet(seq).serialize().unwrap().to_vec()
            };
            peer.send_to(&rtp, destination).await.unwrap();
        }
        publish_all(&mut events, 2, &mut published).await;
        if srtp {
            peer.send_to(&sr_wire, destination).await.unwrap();
        }
        peer.send_to(&rr_wire, destination).await.unwrap();
        seen.extend(next_rtcp_events(&mut events, 2).await);
        assert_no_more_rtcp_events(&mut events).await;
        let stats = session.get_stats();
        let observed = RtcpObserved {
            events: seen,
            published,
            rtcp_packets_received: stats.rtcp_packets_received,
            rtcp_packets_rejected: stats.rtcp_packets_rejected,
            peer_report: stats.peer_report.map(|report| {
                (
                    report.reporter_ssrc,
                    report.fraction_lost,
                    report.cumulative_lost,
                    report.extended_highest_sequence,
                )
            }),
            peer_bye: stats.peer_bye.map(|bye| (bye.ssrc, bye.reason)),
            retained_sender_reports: session.received_sender_reports.len(),
            rtcp_members: session.rtcp_reporter.rtcp_members.len(),
        };
        session.close().await.unwrap();
        observed
    }

    #[tokio::test]
    async fn inbound_rtcp_on_the_owner_matches_the_session_task() {
        let before = inline_executor_counters();
        for srtp in [false, true] {
            let v1 = run_inbound_rtcp(MediaExecutorMode::V1, srtp).await;
            let v2 = run_inbound_rtcp(MediaExecutorMode::V2, srtp).await;
            assert_eq!(v1, v2, "srtp={srtp}");
            assert_eq!(v2.events.len(), 3, "SR, RR and BYE once each");
            assert_eq!(v2.published, vec![1, 2]);
            assert_eq!(v2.rtcp_packets_received, 2, "the SRTCP replay is dropped");
            assert_eq!(v2.rtcp_packets_rejected, 1, "the stranger is filtered");
            assert_eq!(v2.peer_report, Some((REMOTE_SSRC, 0, 3, 77)));
            assert_eq!(v2.peer_bye, Some((REMOTE_SSRC, None)));
            assert_eq!(v2.retained_sender_reports, 1);
            assert_eq!(v2.rtcp_members, 1, "the stranger is not a member");
        }
        let after = inline_executor_counters();
        assert!(after.owner_rtcp_inbound >= before.owner_rtcp_inbound + 6);
    }

    /// What a periodic report carries: everything RFC 3550 puts in an SR/RR
    /// except the clock-derived NTP/RTP timestamps, with jitter and DLSR
    /// reduced to "small" and "elapsed" (they follow the arrival clock).
    fn report_shape(datagram: &[u8]) -> Vec<String> {
        use crate::packet::rtcp::{RtcpCompoundMember, RtcpCompoundPacket, RtcpPacket};
        let blocks = |blocks: &[crate::packet::rtcp::RtcpReportBlock]| -> Vec<String> {
            blocks
                .iter()
                .map(|block| {
                    format!(
                        "ssrc={:08x} fraction={} cumulative={} highest={} lsr={:08x} dlsr>0={} jitter<20ms={}",
                        block.ssrc,
                        block.fraction_lost,
                        block.cumulative_lost,
                        block.highest_seq,
                        block.last_sr,
                        block.delay_since_last_sr > 0,
                        block.jitter < 160
                    )
                })
                .collect()
        };
        RtcpCompoundPacket::parse_tolerant(datagram)
            .expect("periodic report parses")
            .packets
            .into_iter()
            .map(|member| match member {
                RtcpCompoundMember::Known(RtcpPacket::SenderReport(sr)) => format!(
                    "SR ssrc={:08x} packets={} octets={} blocks={:?}",
                    sr.ssrc,
                    sr.sender_packet_count,
                    sr.sender_octet_count,
                    blocks(&sr.report_blocks)
                ),
                RtcpCompoundMember::Known(RtcpPacket::ReceiverReport(rr)) => format!(
                    "RR ssrc={:08x} blocks={:?}",
                    rr.ssrc,
                    blocks(&rr.report_blocks)
                ),
                RtcpCompoundMember::Known(RtcpPacket::SourceDescription(sdes)) => {
                    format!("SDES chunks={}", sdes.chunks.len())
                }
                RtcpCompoundMember::Known(RtcpPacket::Goodbye(bye)) => {
                    format!("BYE {:08x?} {:?}", bye.sources, bye.reason)
                }
                RtcpCompoundMember::Known(other) => format!("other {:?}", other.packet_type()),
                RtcpCompoundMember::Unknown(unknown) => format!("unknown {}", unknown.packet_type),
            })
            .collect()
    }

    fn is_rtcp(datagram: &[u8]) -> bool {
        datagram.len() >= 8 && (200..=206).contains(&datagram[1])
    }

    /// The next RTCP datagram the peer receives (RTP from the session is
    /// skipped).
    async fn next_report(peer: &UdpSocket) -> Vec<u8> {
        let mut buffer = [0u8; 2048];
        loop {
            let (size, _) =
                tokio::time::timeout(Duration::from_secs(3), peer.recv_from(&mut buffer))
                    .await
                    .expect("an RTCP report")
                    .unwrap();
            if is_rtcp(&buffer[..size]) {
                return buffer[..size].to_vec();
            }
        }
    }

    #[derive(Debug, PartialEq)]
    struct ReportsObserved {
        periodic: Vec<Vec<String>>,
        closing: Vec<String>,
        interval_in_range: bool,
    }

    /// Inbound SR then RTP 1, 2, 4 from the peer, two media packets from the
    /// session, then two periodic reports and the close.
    async fn run_periodic_reports(mode: MediaExecutorMode, srtp: bool) -> ReportsObserved {
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut session = {
            let _mode = override_media_executor_config_on_this_thread(config_for(mode));
            RtpSession::new_event_driven(RtpSessionConfig {
                local_addr: "127.0.0.1:0".parse().unwrap(),
                remote_addr: Some(peer.local_addr().unwrap()),
                ssrc: Some(LOCAL_SSRC),
                ..RtpSessionConfig::default()
            })
            .await
            .unwrap()
        };
        let mut peer_tx = srtp_context();
        let mut peer_rx = srtp_context();
        if srtp {
            udp(&session)
                .set_srtp_contexts(srtp_context(), srtp_context())
                .await
                .unwrap();
        }
        let mut events = session.subscribe();
        let destination = session.local_addr().unwrap();
        let sender_report =
            crate::packet::rtcp::RtcpCompoundPacket::new_with_sr(peer_sender_report())
                .serialize()
                .unwrap();
        let sr_wire = if srtp {
            peer_tx.protect_rtcp(&sender_report).unwrap().to_vec()
        } else {
            sender_report.to_vec()
        };
        peer.send_to(&sr_wire, destination).await.unwrap();
        next_rtcp_events(&mut events, 1).await;
        for seq in [1_u16, 2, 4] {
            let rtp = if srtp {
                protected(&mut peer_tx, seq)
            } else {
                media_packet(seq).serialize().unwrap().to_vec()
            };
            peer.send_to(&rtp, destination).await.unwrap();
        }
        let mut published = Vec::new();
        publish_all(&mut events, 3, &mut published).await;
        for timestamp in [160_u32, 320] {
            session
                .send_packet(timestamp, Bytes::from(vec![0xd5; 160]), false)
                .await
                .unwrap();
        }

        let mut buffer = [0u8; 2048];
        // Everything above happened well inside the first interval (at least
        // 1.25 s at the default bandwidth); now report fast.
        session.set_bandwidth(2_000_000);
        session.set_rtcp_reduced_minimum(true);
        let mut wires = Vec::new();
        let mut arrivals = Vec::new();
        while wires.len() < 2 {
            wires.push(next_report(&peer).await);
            arrivals.push(std::time::Instant::now());
        }
        let gap = arrivals[1].duration_since(arrivals[0]);
        session.close().await.unwrap();
        // A report already in flight may precede the closing compound; the
        // report task is gone after it.
        let mut after_close = Vec::new();
        while let Ok(Ok((size, _))) =
            tokio::time::timeout(Duration::from_millis(600), peer.recv_from(&mut buffer)).await
        {
            if is_rtcp(&buffer[..size]) {
                after_close.push(buffer[..size].to_vec());
            }
        }
        let closing = after_close.pop().expect("the closing BYE compound");
        let mut shape = |wire: &[u8]| -> Vec<String> {
            if srtp {
                report_shape(&peer_rx.unprotect_rtcp(wire).expect("authenticated SRTCP"))
            } else {
                report_shape(wire)
            }
        };
        if srtp {
            assert_ne!(wires[0], wires[1], "consecutive SRTCP indexes");
        }
        let periodic = wires.iter().map(|wire| shape(wire)).collect();
        for extra in &after_close {
            let _ = shape(extra);
        }
        let closing = shape(&closing);
        ReportsObserved {
            periodic,
            closing,
            interval_in_range: gap >= Duration::from_millis(30) && gap < Duration::from_secs(1),
        }
    }

    #[tokio::test]
    async fn periodic_reports_match_v1_in_content_srtcp_interval_and_teardown() {
        let before = inline_executor_counters();
        for srtp in [false, true] {
            let v1 = run_periodic_reports(MediaExecutorMode::V1, srtp).await;
            let v2 = run_periodic_reports(MediaExecutorMode::V2, srtp).await;
            assert_eq!(v1, v2, "srtp={srtp}");
            assert!(v2.interval_in_range);
            let block = "ssrc=00e40001 fraction=64 cumulative=1 highest=4 lsr=12345678 dlsr>0=true jitter<20ms=true";
            assert_eq!(
                v2.periodic[0],
                vec![
                    format!("SR ssrc={LOCAL_SSRC:08x} packets=2 octets=320 blocks=[{block:?}]"),
                    "SDES chunks=1".to_string(),
                ]
            );
            assert!(v2.periodic[1][0].contains("fraction=0 cumulative=1 highest=4"));
            // SR or RR, depending on whether the session still counts as a
            // sender two intervals after its media (RFC 3550 §6.4).
            assert!(v2.closing[0].contains(&format!("R ssrc={LOCAL_SSRC:08x}")));
            assert_eq!(v2.closing[1], "SDES chunks=1");
            assert_eq!(
                v2.closing[2],
                format!("BYE [{LOCAL_SSRC:08x}] Some(\"Session closed\")")
            );
        }
        let after = inline_executor_counters();
        assert!(
            after.rtcp_sync_sends + after.rtcp_sync_deferred
                >= before.rtcp_sync_sends + before.rtcp_sync_deferred + 4
        );
        assert!(after.rtcp_lane_tasks >= before.rtcp_lane_tasks + 2);
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
