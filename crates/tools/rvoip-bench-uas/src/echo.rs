//! RTP echo through rvoip's media plane.
//!
//! Each established call binds a [`SipMediaStream`] and turns every inbound
//! media frame straight around: rvoip receives the RTP packet (SRTP:
//! authenticates and decrypts it with the offer's key), hands the G.711
//! payload to the echo, and the echo writes it back to the call, which rvoip
//! packetizes and (SRTP) encrypts with the answer key and sends to the
//! caller's latched RTP address. RTCP stays muxed on the RTP port and is
//! answered by rvoip's own RTCP.
//!
//! With media executor v2 (the DUT's default) both directions run on the
//! RTP socket's media lane without a task wakeup per packet: the echo is an
//! [`InboundFrameSink`] that the fused inbound pump calls inline, and it
//! sends through the stream's [`EncodedG711Writer`] on the synchronous TX
//! path. A frame the synchronous path cannot take, and every frame of a
//! stream that cannot host the sink (executor v1, a non-G.711 call), goes
//! through a per-call task that awaits the same writer (or the stream's
//! outbound channel), so every frame rvoip delivers is echoed or counted as
//! dropped.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use bytes::Bytes;
use dashmap::DashMap;
use rvoip_core_traits::connection::Direction;
use rvoip_core_traits::ids::StreamId;
use rvoip_core_traits::stream::{
    EncodedG711Writer, InboundFrameDelivery, InboundFrameSink, InboundFrameView, MediaFrame,
    MediaStream, StreamKind,
};
use rvoip_sip::media_stream::SipMediaStream;
use rvoip_sip::{CallId, UnifiedCoordinator};
use tokio::sync::{mpsc, Mutex};

use crate::stats::Counters;

/// Frames counted locally before they are added to the shared counters, so
/// the per-packet path touches no shared cache line.
const FLUSH_EVERY: u64 = 64;
/// Frames the inline sink may hand to the call's task when the synchronous
/// TX path declines them.
const DEFERRED_FRAMES: usize = 32;

/// Per-path frame counts, added to the shared counters in batches and when
/// dropped.
struct LocalCounts {
    counters: Arc<Counters>,
    received: u64,
    sent: u64,
    dropped: u64,
    inline: u64,
}

impl LocalCounts {
    fn new(counters: Arc<Counters>) -> Self {
        Self {
            counters,
            received: 0,
            sent: 0,
            dropped: 0,
            inline: 0,
        }
    }

    fn record(&mut self, received: u64, sent: u64, dropped: u64) {
        self.received += received;
        self.sent += sent;
        self.dropped += dropped;
        if self.received + self.sent + self.dropped >= FLUSH_EVERY {
            self.flush();
        }
    }

    fn flush(&mut self) {
        let counters = &self.counters;
        for (shared, local) in [
            (&counters.rtp_frames_received, &mut self.received),
            (&counters.rtp_frames_sent, &mut self.sent),
            (&counters.echo_frames_dropped, &mut self.dropped),
            (&counters.rtp_frames_sent_inline, &mut self.inline),
        ] {
            if *local > 0 {
                shared.fetch_add(*local, Ordering::Relaxed);
                *local = 0;
            }
        }
    }
}

impl Drop for LocalCounts {
    fn drop(&mut self) {
        self.flush();
    }
}

/// A frame waiting for the call's task.
struct Deferred {
    payload: Bytes,
    timestamp: u32,
    payload_type: u8,
}

type SharedWriter = Arc<Mutex<Box<dyn EncodedG711Writer>>>;

/// The inline echo: runs on the RTP socket's owner for every inbound frame.
struct EchoSink {
    writer: SharedWriter,
    deferred: mpsc::Sender<Deferred>,
    counts: LocalCounts,
}

impl InboundFrameSink for EchoSink {
    fn on_frame(&mut self, frame: InboundFrameView<'_>) -> InboundFrameDelivery {
        let (Some(payload_type), StreamKind::Audio, false) =
            (frame.payload_type, frame.kind, frame.payload.is_empty())
        else {
            self.counts.record(1, 0, 1);
            return InboundFrameDelivery::Accepted;
        };
        let payload = Bytes::copy_from_slice(frame.payload);
        // The writer is only contended while the call's task is sending a
        // deferred frame; then this frame queues behind it, in order.
        let handled = match self.writer.try_lock() {
            Ok(mut writer) => {
                let report = writer.try_send_encoded_now(
                    &[(&payload, frame.timestamp_rtp)],
                    payload_type,
                    false,
                );
                if report.sent == 1 {
                    Some(true)
                } else if report.failure.is_some() {
                    Some(false)
                } else {
                    None
                }
            }
            Err(_) => None,
        };
        match handled {
            Some(true) => {
                self.counts.inline += 1;
                self.counts.record(1, 1, 0);
            }
            Some(false) => self.counts.record(1, 0, 1),
            None => {
                let deferred = Deferred {
                    payload,
                    timestamp: frame.timestamp_rtp,
                    payload_type,
                };
                // Received here; sent or dropped by the task.
                match self.deferred.try_send(deferred) {
                    Ok(()) => self.counts.record(1, 0, 0),
                    Err(_) => self.counts.record(1, 0, 1),
                }
            }
        }
        InboundFrameDelivery::Accepted
    }
}

/// Where the call's task writes echoed frames.
enum Output {
    /// The stream's encoded G.711 writer (shared with the inline sink).
    Writer(SharedWriter),
    /// The stream's outbound frame channel.
    Channel(mpsc::Sender<MediaFrame>, StreamId),
}

impl Output {
    async fn send(&self, payload: Bytes, timestamp: u32, payload_type: Option<u8>) -> bool {
        match self {
            Self::Writer(writer) => match payload_type {
                Some(payload_type) => writer
                    .lock()
                    .await
                    .send_encoded(&payload, timestamp, payload_type)
                    .await
                    .is_ok(),
                None => false,
            },
            Self::Channel(frames_out, stream_id) => frames_out
                .send(MediaFrame {
                    stream_id: stream_id.clone(),
                    kind: StreamKind::Audio,
                    payload,
                    timestamp_rtp: timestamp,
                    captured_at: chrono::Utc::now(),
                    payload_type,
                })
                .await
                .is_ok(),
        }
    }
}

/// The call's task: echoes frames that arrive on `frames_in` (the stream did
/// not, or not yet, adopt the inline sink) and frames the sink deferred.
async fn echo_task(
    mut frames_in: Option<mpsc::Receiver<MediaFrame>>,
    mut deferred: Option<mpsc::Receiver<Deferred>>,
    output: Output,
    counters: Arc<Counters>,
) {
    let mut counts = LocalCounts::new(counters);
    loop {
        let (payload, timestamp, payload_type, received_here) = tokio::select! {
            frame = async { frames_in.as_mut().expect("guarded").recv().await },
                if frames_in.is_some() =>
            {
                match frame {
                    Some(frame) => (frame.payload, frame.timestamp_rtp, frame.payload_type, true),
                    None => {
                        frames_in = None;
                        continue;
                    }
                }
            }
            frame = async { deferred.as_mut().expect("guarded").recv().await },
                if deferred.is_some() =>
            {
                match frame {
                    Some(frame) => (frame.payload, frame.timestamp, Some(frame.payload_type), false),
                    None => {
                        deferred = None;
                        continue;
                    }
                }
            }
            else => break,
        };
        let sent = output.send(payload, timestamp, payload_type).await;
        counts.record(u64::from(received_here), u64::from(sent), u64::from(!sent));
    }
}

/// The echo streams of live calls.
pub struct EchoRegistry {
    streams: DashMap<CallId, Arc<SipMediaStream>>,
    counters: Arc<Counters>,
}

impl EchoRegistry {
    /// An empty registry updating `counters`.
    pub fn new(counters: Arc<Counters>) -> Self {
        Self {
            streams: DashMap::new(),
            counters,
        }
    }

    /// Close `call_id`'s echo stream, if it has one.
    pub fn close(&self, call_id: &CallId) {
        let Some((_, stream)) = self.streams.remove(call_id) else {
            return;
        };
        let counters = Arc::clone(&self.counters);
        counters.echo_streams_open.fetch_sub(1, Ordering::Relaxed);
        counters
            .echo_streams_closing
            .fetch_add(1, Ordering::Relaxed);
        tokio::spawn(async move {
            if let Err(error) = stream.close().await {
                tracing::debug!("echo stream close: {error}");
            }
            counters
                .echo_streams_closing
                .fetch_sub(1, Ordering::Relaxed);
        });
    }

    /// Echo streams currently registered.
    pub fn len(&self) -> usize {
        self.streams.len()
    }

    /// Whether no echo stream is registered.
    pub fn is_empty(&self) -> bool {
        self.streams.is_empty()
    }
}

/// Bind `call_id`'s media and start echoing it. `is_live` tells whether the
/// call is still up once the bind finished; a call that ended meanwhile has
/// its stream closed at once.
pub async fn start(
    registry: Arc<EchoRegistry>,
    coordinator: Arc<UnifiedCoordinator>,
    call_id: CallId,
    is_live: impl Fn(&CallId) -> bool,
) {
    let counters = Arc::clone(&registry.counters);
    let stream = match SipMediaStream::new(coordinator, call_id.clone(), Direction::Inbound).await {
        Ok(stream) => stream,
        Err(error) => {
            if is_live(&call_id) {
                counters.echo_bind_failures.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(call = %call_id.0, "echo stream did not bind: {error}");
            }
            return;
        }
    };
    install(&stream, &counters);
    registry.streams.insert(call_id.clone(), stream);
    counters.echo_streams_open.fetch_add(1, Ordering::Relaxed);
    // The call may have ended while the stream bound; its end then found no
    // stream to close, so close it here. Whichever side removes it closes it.
    if !is_live(&call_id) {
        registry.close(&call_id);
    }
}

/// Wire the echo onto a bound stream.
fn install(stream: &Arc<SipMediaStream>, counters: &Arc<Counters>) {
    // The writer must be taken before anything else claims the output.
    let writer = stream.try_take_encoded_g711_writer().ok().flatten();
    let frames_in = stream.try_frames_in().ok();
    let (output, deferred) = match writer {
        Some(writer) => {
            let writer: SharedWriter = Arc::new(Mutex::new(writer));
            let (deferred_tx, deferred_rx) = mpsc::channel(DEFERRED_FRAMES);
            let sink = EchoSink {
                writer: Arc::clone(&writer),
                deferred: deferred_tx,
                counts: LocalCounts::new(Arc::clone(counters)),
            };
            // Refused (no inline pump on this executor): the sink and its
            // deferred sender drop, and frames keep coming on `frames_in`.
            let _ = stream.offer_inbound_frame_sink(Box::new(sink));
            (Output::Writer(writer), Some(deferred_rx))
        }
        None => match stream.try_frames_out() {
            Ok(frames_out) => (Output::Channel(frames_out, stream.id()), None),
            Err(error) => {
                counters.echo_bind_failures.fetch_add(1, Ordering::Relaxed);
                tracing::warn!("echo stream is not writable: {error}");
                return;
            }
        },
    };
    tokio::spawn(echo_task(frames_in, deferred, output, Arc::clone(counters)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_counts_reach_the_shared_counters_in_batches_and_on_drop() {
        let counters = Arc::new(Counters::default());
        let mut counts = LocalCounts::new(Arc::clone(&counters));
        for _ in 0..10 {
            counts.record(1, 1, 0);
        }
        // 20 events: below the batch, nothing shared yet.
        assert_eq!(counters.rtp_frames_received.load(Ordering::Relaxed), 0);
        for _ in 0..30 {
            counts.record(1, 0, 1);
        }
        // The batch flushed once at 64 events (after 22 of the drops); the
        // last 8 records are still local.
        assert_eq!(counters.rtp_frames_received.load(Ordering::Relaxed), 32);
        assert_eq!(counters.rtp_frames_sent.load(Ordering::Relaxed), 10);
        assert_eq!(counters.echo_frames_dropped.load(Ordering::Relaxed), 22);
        counts.record(1, 1, 0);
        drop(counts);
        assert_eq!(counters.rtp_frames_received.load(Ordering::Relaxed), 41);
        assert_eq!(counters.rtp_frames_sent.load(Ordering::Relaxed), 11);
        assert_eq!(counters.echo_frames_dropped.load(Ordering::Relaxed), 30);
    }

    /// A writer whose synchronous path takes every other frame.
    struct HalfSyncWriter {
        sync_sent: Arc<std::sync::Mutex<Vec<u32>>>,
        async_sent: Arc<std::sync::Mutex<Vec<u32>>>,
        calls: u32,
    }

    #[async_trait::async_trait]
    impl EncodedG711Writer for HalfSyncWriter {
        async fn send_encoded(
            &mut self,
            _payload: &Bytes,
            timestamp: u32,
            _payload_type: u8,
        ) -> rvoip_core_traits::error::Result<()> {
            self.async_sent.lock().unwrap().push(timestamp);
            Ok(())
        }

        fn try_send_encoded_now(
            &mut self,
            frames: &[(&Bytes, u32)],
            _payload_type: u8,
            _batch: bool,
        ) -> rvoip_core_traits::stream::EncodedSendNow {
            self.calls += 1;
            let mut report = rvoip_core_traits::stream::EncodedSendNow::default();
            if self.calls % 2 == 1 {
                self.sync_sent.lock().unwrap().push(frames[0].1);
                report.sent = 1;
            }
            report
        }
    }

    #[tokio::test]
    async fn the_sink_echoes_inline_and_defers_what_the_sync_path_declines() {
        let counters = Arc::new(Counters::default());
        let sync_sent = Arc::new(std::sync::Mutex::new(Vec::new()));
        let async_sent = Arc::new(std::sync::Mutex::new(Vec::new()));
        let writer: SharedWriter = Arc::new(Mutex::new(Box::new(HalfSyncWriter {
            sync_sent: Arc::clone(&sync_sent),
            async_sent: Arc::clone(&async_sent),
            calls: 0,
        })));
        let (deferred_tx, deferred_rx) = mpsc::channel(DEFERRED_FRAMES);
        let mut sink = EchoSink {
            writer: Arc::clone(&writer),
            deferred: deferred_tx,
            counts: LocalCounts::new(Arc::clone(&counters)),
        };
        let task = tokio::spawn(echo_task(
            None,
            Some(deferred_rx),
            Output::Writer(writer),
            Arc::clone(&counters),
        ));
        let payload = [0x55u8; 160];
        for index in 0..10u32 {
            let delivery = sink.on_frame(InboundFrameView::new(
                StreamKind::Audio,
                &payload,
                index * 160,
                Some(0),
            ));
            assert_eq!(delivery, InboundFrameDelivery::Accepted);
        }
        // A frame without a payload type cannot be echoed.
        sink.on_frame(InboundFrameView::new(
            StreamKind::Audio,
            &payload,
            9_999,
            None,
        ));
        drop(sink);
        task.await.unwrap();

        let sync_sent = sync_sent.lock().unwrap().clone();
        let async_sent = async_sent.lock().unwrap().clone();
        assert_eq!(sync_sent, [0, 320, 640, 960, 1280]);
        assert_eq!(async_sent, [160, 480, 800, 1120, 1440]);
        assert_eq!(counters.rtp_frames_received.load(Ordering::Relaxed), 11);
        assert_eq!(counters.rtp_frames_sent.load(Ordering::Relaxed), 10);
        assert_eq!(counters.echo_frames_dropped.load(Ordering::Relaxed), 1);
        assert_eq!(counters.rtp_frames_sent_inline.load(Ordering::Relaxed), 5);
    }

    #[tokio::test]
    async fn the_task_echoes_channel_frames_until_the_stream_closes() {
        let counters = Arc::new(Counters::default());
        let (in_tx, in_rx) = mpsc::channel(8);
        let (out_tx, mut out_rx) = mpsc::channel(8);
        let stream_id = StreamId::new();
        let task = tokio::spawn(echo_task(
            Some(in_rx),
            None,
            Output::Channel(out_tx, stream_id.clone()),
            Arc::clone(&counters),
        ));
        for index in 0..3u32 {
            in_tx
                .send(MediaFrame {
                    stream_id: stream_id.clone(),
                    kind: StreamKind::Audio,
                    payload: Bytes::from(vec![index as u8; 160]),
                    timestamp_rtp: index * 160,
                    captured_at: chrono::Utc::now(),
                    payload_type: Some(8),
                })
                .await
                .unwrap();
        }
        drop(in_tx);
        task.await.unwrap();
        let mut echoed = Vec::new();
        while let Ok(frame) = out_rx.try_recv() {
            assert_eq!(frame.payload_type, Some(8));
            echoed.push(frame.timestamp_rtp);
        }
        assert_eq!(echoed, [0, 160, 320]);
        assert_eq!(counters.rtp_frames_received.load(Ordering::Relaxed), 3);
        assert_eq!(counters.rtp_frames_sent.load(Ordering::Relaxed), 3);
    }
}
