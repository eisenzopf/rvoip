//! `QuicDatagramMediaStream` — implements `rvoip_core::MediaStream` over
//! QUIC datagrams using the UCTP 8-byte header.
//!
//! Per design doc §4.5 / §3.6. Construction spawns two driver tasks: an
//! **outbound pump** that drains `frames_out` → `substrate::datagram::pack`
//! → `quinn::Connection::send_datagram`, and an **inbound pump** the
//! adapter feeds via [`QuicDatagramMediaStream::inbound_tx`] from its
//! own `quinn::Connection::read_datagram` loop (which is per-Connection,
//! not per-Stream).

use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::Utc;
use rvoip_core::capability::CodecInfo;
use rvoip_core::connection::Direction;
use rvoip_core::error::Result as RvoipResult;
use rvoip_core::ids::StreamId;
use rvoip_core::stream::{MediaFrame, MediaStream, QualitySnapshot, StreamKind};
use rvoip_uctp::substrate::datagram::{pack, pack_rtp, unpack_rtp, MediaDatagram};
use tokio::sync::mpsc;
use tracing::{debug, trace_span, warn};

const FRAME_CAP: usize = 1024;

pub struct QuicDatagramMediaStream {
    id: StreamId,
    kind: StreamKind,
    codec: CodecInfo,
    direction: Direction,
    stream_local_id: u16,
    in_rx: StdMutex<Option<mpsc::Receiver<MediaFrame>>>,
    out_tx: mpsc::Sender<MediaFrame>,
    /// Adapter feeds inbound `MediaFrame`s here from its per-Connection
    /// datagram reader (one reader per connection serves many streams).
    inbound_tx: mpsc::Sender<MediaFrame>,
    quality: parking_lot::RwLock<QualitySnapshot>,
}

impl QuicDatagramMediaStream {
    /// Construct the stream and spawn the outbound pump task. The
    /// returned `Arc<Self>` exposes [`MediaStream::frames_in`] /
    /// [`MediaStream::frames_out`] to consumers; the adapter retains
    /// a clone for feeding inbound frames via `inbound_tx`.
    pub fn start(
        id: StreamId,
        kind: StreamKind,
        codec: CodecInfo,
        direction: Direction,
        stream_local_id: u16,
        conn: quinn::Connection,
    ) -> Arc<Self> {
        let (in_tx, in_rx) = mpsc::channel::<MediaFrame>(FRAME_CAP);
        let (out_tx, mut out_rx) = mpsc::channel::<MediaFrame>(FRAME_CAP);

        // Outbound pump: frames_out → pack → quinn::Connection::send_datagram.
        let conn_for_pump = conn.clone();
        let stream_id_for_pump = id.clone();
        let default_payload_type = rvoip_core::bridge::codec_to_pt(&codec.name).unwrap_or(111);
        let ssrc = stable_ssrc(&id);
        tokio::spawn(async move {
            let mut seq: u32 = 0;
            let mut rtp_seq: u16 = 0;
            while let Some(frame) = out_rx.recv().await {
                let _span = trace_span!(
                    "uctp.stream.frame",
                    stream_local_id,
                    direction = "out",
                    transport = "quic",
                    seq,
                )
                .entered();
                let rtp = match pack_rtp(
                    frame.payload,
                    frame.payload_type.unwrap_or(default_payload_type),
                    rtp_seq,
                    frame.timestamp_rtp,
                    ssrc,
                ) {
                    Ok(packet) => packet,
                    Err(error) => {
                        metrics::counter!(
                            "uctp_datagram_drops_total",
                            "direction" => "out",
                            "transport" => "quic",
                            "reason" => "rtp-encode"
                        )
                        .increment(1);
                        debug!(%error, "rvoip-quic: RTP encode failed");
                        continue;
                    }
                };
                let datagram = MediaDatagram {
                    flags: 0,
                    stream_local_id,
                    seq,
                    payload: rtp,
                };
                let bytes = pack(&datagram);
                if let Err(e) = conn_for_pump.send_datagram(bytes) {
                    // Backpressure policy §3.5: drop + counter, do NOT close.
                    metrics::counter!(
                        "uctp_datagram_drops_total",
                        "direction" => "out",
                        "transport" => "quic",
                        "reason" => "send-failed"
                    )
                    .increment(1);
                    debug!(error = %e, stream = %stream_id_for_pump, "rvoip-quic: send_datagram failed");
                    continue;
                }
                metrics::counter!(
                    "uctp_datagrams_total",
                    "direction" => "out",
                    "transport" => "quic"
                )
                .increment(1);
                seq = seq.wrapping_add(1);
                rtp_seq = rtp_seq.wrapping_add(1);
            }
            debug!("rvoip-quic: outbound pump exiting");
        });

        let stream = Arc::new(Self {
            id,
            kind,
            codec,
            direction,
            stream_local_id,
            in_rx: StdMutex::new(Some(in_rx)),
            out_tx,
            inbound_tx: in_tx,
            quality: parking_lot::RwLock::new(QualitySnapshot::default()),
        });

        stream
    }

    /// Allow the adapter's per-connection datagram reader to push
    /// frames it has unpacked for this stream's `stream_local_id`.
    pub fn inbound_tx(&self) -> mpsc::Sender<MediaFrame> {
        self.inbound_tx.clone()
    }

    pub fn stream_local_id(&self) -> u16 {
        self.stream_local_id
    }

    pub fn update_quality(&self, q: QualitySnapshot) {
        *self.quality.write() = q;
    }
}

#[async_trait]
impl MediaStream for QuicDatagramMediaStream {
    fn id(&self) -> StreamId {
        self.id.clone()
    }

    fn kind(&self) -> StreamKind {
        self.kind
    }

    fn codec(&self) -> CodecInfo {
        self.codec.clone()
    }

    fn direction(&self) -> Direction {
        self.direction
    }

    fn frames_in(&self) -> mpsc::Receiver<MediaFrame> {
        let mut guard = self.in_rx.lock().expect("poisoned");
        guard.take().unwrap_or_else(|| {
            let (_tx, rx) = mpsc::channel(1);
            rx
        })
    }

    fn frames_out(&self) -> mpsc::Sender<MediaFrame> {
        self.out_tx.clone()
    }

    fn quality_snapshot(&self) -> QualitySnapshot {
        self.quality.read().clone()
    }

    async fn close(self: Arc<Self>) -> RvoipResult<()> {
        Ok(())
    }
}

/// Multi-party fanout context. When present on
/// [`spawn_datagram_reader`], every successfully-routed inbound frame
/// is also handed to `orchestrator.fanout_frame(...)` so subscribers
/// in this Session see the publisher's media. See `Orchestrator::fanout_frame`
/// (rvoip-core) for the routing-table lookup.
#[derive(Clone)]
pub struct FanoutContext {
    pub orchestrator: Arc<rvoip_core::Orchestrator>,
    pub sid: rvoip_core::ids::SessionId,
    pub publisher_connid: rvoip_core::ids::ConnectionId,
}

/// Per-`quinn::Connection` datagram reader. One reader serves all
/// `QuicDatagramMediaStream`s on this connection; the reader looks up
/// the matching stream by `stream_local_id` in the supplied router and
/// forwards the unpacked `MediaFrame` to its `inbound_tx`.
///
/// When `fanout` is `Some`, after the local route succeeds the reader
/// also calls `Orchestrator::fanout_frame(...)` to deliver the frame to
/// every subscriber registered against this publisher's `(sid, connid,
/// strm_id)`. Fanout is best-effort: a slow subscriber doesn't block
/// the publisher's local path.
pub fn spawn_datagram_reader(
    conn: quinn::Connection,
    router: Arc<parking_lot::RwLock<Vec<Arc<QuicDatagramMediaStream>>>>,
    fanout: Option<FanoutContext>,
) {
    tokio::spawn(async move {
        loop {
            match conn.read_datagram().await {
                Ok(bytes) => {
                    let datagram = match rvoip_uctp::substrate::datagram::unpack(&bytes) {
                        Ok(d) => d,
                        Err(_) => {
                            metrics::counter!(
                                "uctp_datagram_drops_total",
                                "direction" => "in",
                                "transport" => "quic",
                                "reason" => "unpack-error"
                            )
                            .increment(1);
                            continue;
                        }
                    };
                    let target = {
                        let guard = router.read();
                        guard
                            .iter()
                            .find(|s| s.stream_local_id() == datagram.stream_local_id)
                            .cloned()
                    };
                    match target {
                        Some(stream) => {
                            // Build the frame and do the local route
                            // inside a tight scope so the non-Send
                            // tracing span guard is dropped before we
                            // await the fanout call.
                            let rtp = match unpack_rtp(datagram.payload) {
                                Ok(rtp) => rtp,
                                Err(_) => {
                                    metrics::counter!(
                                        "uctp_datagram_drops_total",
                                        "direction" => "in",
                                        "transport" => "quic",
                                        "reason" => "invalid-rtp"
                                    )
                                    .increment(1);
                                    continue;
                                }
                            };
                            let frame = {
                                let _span = trace_span!(
                                    "uctp.stream.frame",
                                    stream_local_id = datagram.stream_local_id,
                                    direction = "in",
                                    transport = "quic",
                                    seq = datagram.seq,
                                )
                                .entered();
                                let frame = MediaFrame {
                                    stream_id: stream.id(),
                                    kind: stream.kind(),
                                    payload: rtp.payload,
                                    timestamp_rtp: rtp.timestamp,
                                    captured_at: Utc::now(),
                                    payload_type: Some(rtp.payload_type),
                                };
                                match stream.inbound_tx().try_send(frame.clone()) {
                                    Ok(_) => {
                                        metrics::counter!(
                                            "uctp_datagrams_total",
                                            "direction" => "in",
                                            "transport" => "quic"
                                        )
                                        .increment(1);
                                    }
                                    Err(_) => {
                                        metrics::counter!(
                                            "uctp_datagram_drops_total",
                                            "direction" => "in",
                                            "transport" => "quic",
                                            "reason" => "channel-full"
                                        )
                                        .increment(1);
                                    }
                                }
                                frame
                            };
                            // MP3b — best-effort fanout to multi-party
                            // subscribers. Local route already succeeded
                            // above; this fans the same frame out to
                            // subscribers in `(sid, publisher_connid,
                            // strm_id)`. A wedged subscriber must not
                            // block the publisher's path — we await but
                            // any backpressure on the subscriber's
                            // frames_out is local to that subscriber's
                            // adapter.
                            if let Some(ref ctx) = fanout {
                                let _ = ctx
                                    .orchestrator
                                    .fanout_frame(
                                        &ctx.sid,
                                        &ctx.publisher_connid,
                                        &stream.id(),
                                        frame,
                                    )
                                    .await;
                            }
                        }
                        None => {
                            metrics::counter!(
                                "uctp_datagram_drops_total",
                                "direction" => "in",
                                "transport" => "quic",
                                "reason" => "unknown-stream"
                            )
                            .increment(1);
                        }
                    }
                }
                Err(quinn::ConnectionError::ApplicationClosed(_))
                | Err(quinn::ConnectionError::LocallyClosed) => {
                    debug!("rvoip-quic: datagram reader exiting on close");
                    return;
                }
                Err(e) => {
                    warn!(error = %e, "rvoip-quic: datagram reader error; continuing");
                    let _ = Bytes::new();
                }
            }
        }
    });
}

fn stable_ssrc(stream_id: &StreamId) -> u32 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    stream_id.hash(&mut hasher);
    hasher.finish() as u32
}
