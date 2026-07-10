//! `WebTransportDatagramMediaStream` — implements `rvoip_core::MediaStream`
//! over WebTransport datagrams (which ride QUIC datagrams underneath).
//!
//! Mirrors `rvoip_quic::QuicDatagramMediaStream`. The only
//! transport-specific differences are the datagram send path
//! (`web_transport_quinn::Session::send_datagram` vs.
//! `quinn::Connection::send_datagram`) and the read API.

use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use async_trait::async_trait;
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

pub struct WebTransportDatagramMediaStream {
    id: StreamId,
    kind: StreamKind,
    codec: CodecInfo,
    direction: Direction,
    stream_local_id: u16,
    in_rx: StdMutex<Option<mpsc::Receiver<MediaFrame>>>,
    out_tx: mpsc::Sender<MediaFrame>,
    inbound_tx: mpsc::Sender<MediaFrame>,
    quality: parking_lot::RwLock<QualitySnapshot>,
}

impl WebTransportDatagramMediaStream {
    pub fn start(
        id: StreamId,
        kind: StreamKind,
        codec: CodecInfo,
        direction: Direction,
        stream_local_id: u16,
        session: web_transport_quinn::Session,
    ) -> Arc<Self> {
        let (in_tx, in_rx) = mpsc::channel::<MediaFrame>(FRAME_CAP);
        let (out_tx, mut out_rx) = mpsc::channel::<MediaFrame>(FRAME_CAP);

        let session_for_pump = session.clone();
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
                    transport = "webtransport",
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
                            "transport" => "webtransport",
                            "reason" => "rtp-encode"
                        )
                        .increment(1);
                        debug!(%error, "rvoip-webtransport: RTP encode failed");
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
                if let Err(e) = session_for_pump.send_datagram(bytes) {
                    metrics::counter!(
                        "uctp_datagram_drops_total",
                        "direction" => "out",
                        "transport" => "webtransport",
                        "reason" => "send-failed"
                    )
                    .increment(1);
                    debug!(error = %e, stream = %stream_id_for_pump, "rvoip-webtransport: send_datagram failed");
                    continue;
                }
                metrics::counter!(
                    "uctp_datagrams_total",
                    "direction" => "out",
                    "transport" => "webtransport"
                )
                .increment(1);
                seq = seq.wrapping_add(1);
                rtp_seq = rtp_seq.wrapping_add(1);
            }
            debug!("rvoip-webtransport: outbound pump exiting");
        });

        Arc::new(Self {
            id,
            kind,
            codec,
            direction,
            stream_local_id,
            in_rx: StdMutex::new(Some(in_rx)),
            out_tx,
            inbound_tx: in_tx,
            quality: parking_lot::RwLock::new(QualitySnapshot::default()),
        })
    }

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
impl MediaStream for WebTransportDatagramMediaStream {
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

/// Mirror of `rvoip_quic::FanoutContext` for the WT adapter. See
/// that doc-comment.
#[derive(Clone)]
pub struct FanoutContext {
    pub orchestrator: Arc<rvoip_core::Orchestrator>,
    pub sid: rvoip_core::ids::SessionId,
    pub publisher_connid: rvoip_core::ids::ConnectionId,
}

/// Per-`web_transport_quinn::Session` datagram reader. One reader
/// serves all `WebTransportDatagramMediaStream`s on this session.
///
/// `fanout`: see `rvoip_quic::FanoutContext`; when `Some`, the reader
/// forwards every successfully-routed frame to
/// `Orchestrator::fanout_frame(...)` so multi-party subscribers receive
/// the publisher's media.
pub fn spawn_datagram_reader(
    session: web_transport_quinn::Session,
    router: Arc<parking_lot::RwLock<Vec<Arc<WebTransportDatagramMediaStream>>>>,
    fanout: Option<FanoutContext>,
) {
    tokio::spawn(async move {
        loop {
            match session.read_datagram().await {
                Ok(bytes) => {
                    let datagram = match rvoip_uctp::substrate::datagram::unpack(&bytes) {
                        Ok(d) => d,
                        Err(_) => {
                            metrics::counter!(
                                "uctp_datagram_drops_total",
                                "direction" => "in",
                                "transport" => "webtransport",
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
                            // Tight scope for the non-Send span guard;
                            // it must drop before the fanout await.
                            let rtp = match unpack_rtp(datagram.payload) {
                                Ok(rtp) => rtp,
                                Err(_) => {
                                    metrics::counter!(
                                        "uctp_datagram_drops_total",
                                        "direction" => "in",
                                        "transport" => "webtransport",
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
                                    transport = "webtransport",
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
                                            "transport" => "webtransport"
                                        )
                                        .increment(1);
                                    }
                                    Err(_) => {
                                        metrics::counter!(
                                            "uctp_datagram_drops_total",
                                            "direction" => "in",
                                            "transport" => "webtransport",
                                            "reason" => "channel-full"
                                        )
                                        .increment(1);
                                    }
                                }
                                frame
                            };
                            // MP3b — multi-party fanout (mirror of QUIC).
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
                                "transport" => "webtransport",
                                "reason" => "unknown-stream"
                            )
                            .increment(1);
                        }
                    }
                }
                Err(e) => {
                    warn!(error = %e, "rvoip-webtransport: datagram reader exiting");
                    return;
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
