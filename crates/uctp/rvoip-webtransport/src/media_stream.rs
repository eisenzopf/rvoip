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
use rvoip_core::error::{Result as RvoipResult, RvoipError};
use rvoip_core::ids::StreamId;
use rvoip_core::stream::{MediaFrame, MediaStream, QualitySnapshot, StreamKind};
use rvoip_uctp::substrate::datagram::{pack, pack_rtp, unpack_rtp, MediaDatagram};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
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
    cancel: CancellationToken,
    outbound_task: StdMutex<Option<tokio::task::JoinHandle<()>>>,
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
        Self::start_with_cancel(
            id,
            kind,
            codec,
            direction,
            stream_local_id,
            session,
            CancellationToken::new(),
        )
    }

    /// Construct a stream whose background media pump is coupled to the
    /// owning WebTransport peer session.
    pub fn start_with_cancel(
        id: StreamId,
        kind: StreamKind,
        codec: CodecInfo,
        direction: Direction,
        stream_local_id: u16,
        session: web_transport_quinn::Session,
        peer_cancel: CancellationToken,
    ) -> Arc<Self> {
        let (in_tx, in_rx) = mpsc::channel::<MediaFrame>(FRAME_CAP);
        let (out_tx, mut out_rx) = mpsc::channel::<MediaFrame>(FRAME_CAP);

        let stream_cancel = peer_cancel.child_token();
        let pump_cancel = stream_cancel.clone();
        let session_cancel = peer_cancel.clone();
        let session_for_pump = session.clone();
        let stream_id_for_pump = id.clone();
        let default_payload_type = rvoip_core::bridge::codec_to_pt(&codec.name).unwrap_or(111);
        let ssrc = stable_ssrc(&id);
        let outbound_task = tokio::spawn(async move {
            let mut seq: u32 = 0;
            let mut rtp_seq: u16 = 0;
            loop {
                let frame = tokio::select! {
                    _ = pump_cancel.cancelled() => break,
                    frame = out_rx.recv() => match frame {
                        Some(frame) => frame,
                        None => break,
                    },
                };
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
                    if matches!(
                        e,
                        web_transport_quinn::SessionError::ConnectionError(_)
                            | web_transport_quinn::SessionError::WebTransportError(_)
                            | web_transport_quinn::SessionError::SendDatagramError(
                                quinn::SendDatagramError::ConnectionLost(_)
                            )
                    ) {
                        session_cancel.cancel();
                        break;
                    }
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
            cancel: stream_cancel,
            outbound_task: StdMutex::new(Some(outbound_task)),
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

    pub fn is_closed(&self) -> bool {
        self.cancel.is_cancelled()
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
        self.try_frames_in().unwrap_or_else(|_| mpsc::channel(1).1)
    }

    fn try_frames_in(&self) -> RvoipResult<mpsc::Receiver<MediaFrame>> {
        self.in_rx
            .lock()
            .map_err(|_| RvoipError::InvalidState("WebTransport media receiver lock is poisoned"))?
            .take()
            .ok_or(RvoipError::InvalidState(
                "WebTransport media receiver has already been acquired",
            ))
    }

    fn frames_out(&self) -> mpsc::Sender<MediaFrame> {
        self.out_tx.clone()
    }

    fn quality_snapshot(&self) -> QualitySnapshot {
        self.quality.read().clone()
    }

    async fn close(self: Arc<Self>) -> RvoipResult<()> {
        self.cancel.cancel();
        let task = self
            .outbound_task
            .lock()
            .map_err(|_| RvoipError::InvalidState("WebTransport media task lock is poisoned"))?
            .take();
        if let Some(task) = task {
            let _ = task.await;
        }
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
    std::mem::drop(spawn_datagram_reader_with_cancel(
        session,
        router,
        fanout,
        CancellationToken::new(),
    ));
}

/// Spawn the per-peer WebTransport datagram reader with explicit lifecycle
/// coupling.
pub fn spawn_datagram_reader_with_cancel(
    session: web_transport_quinn::Session,
    router: Arc<parking_lot::RwLock<Vec<Arc<WebTransportDatagramMediaStream>>>>,
    fanout: Option<FanoutContext>,
    peer_cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let received = tokio::select! {
                _ = peer_cancel.cancelled() => return,
                received = session.read_datagram() => received,
            };
            match received {
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
                            .find(|s| {
                                !s.is_closed() && s.stream_local_id() == datagram.stream_local_id
                            })
                            .cloned()
                    };
                    if datagram.seq & 0xff == 0 {
                        router.write().retain(|stream| !stream.is_closed());
                    }
                    match target {
                        Some(stream) => {
                            if stream.is_closed() {
                                continue;
                            }
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
                    peer_cancel.cancel();
                    return;
                }
            }
        }
    })
}

fn stable_ssrc(stream_id: &StreamId) -> u32 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    stream_id.hash(&mut hasher);
    hasher.finish() as u32
}

#[cfg(test)]
mod receiver_ownership_tests {
    use super::*;

    #[test]
    fn second_receiver_acquisition_is_a_typed_error() {
        let (inbound_tx, inbound_rx) = mpsc::channel(1);
        let (out_tx, _out_rx) = mpsc::channel(1);
        let stream = WebTransportDatagramMediaStream {
            id: StreamId::new(),
            kind: StreamKind::Audio,
            codec: CodecInfo {
                name: "opus".into(),
                clock_rate_hz: 48_000,
                channels: 1,
                fmtp: None,
            },
            direction: Direction::Inbound,
            stream_local_id: 1,
            in_rx: StdMutex::new(Some(inbound_rx)),
            out_tx,
            inbound_tx,
            quality: parking_lot::RwLock::new(QualitySnapshot::default()),
            cancel: CancellationToken::new(),
            outbound_task: StdMutex::new(None),
        };

        assert!(stream.try_frames_in().is_ok());
        assert!(matches!(
            stream.try_frames_in(),
            Err(RvoipError::InvalidState(_))
        ));
    }
}
