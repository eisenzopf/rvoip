//! `WebTransportDatagramMediaStream` — implements `rvoip_core::MediaStream`
//! over WebTransport datagrams (which ride QUIC datagrams underneath).
//!
//! Mirrors `rvoip_quic::QuicDatagramMediaStream`. The only
//! transport-specific differences are the datagram send path
//! (`web_transport_quinn::Session::send_datagram` vs.
//! `quinn::Connection::send_datagram`) and the read API.

use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::{collections::HashSet, num::NonZeroU16};

use async_trait::async_trait;
use chrono::Utc;
use rvoip_core::capability::CodecInfo;
use rvoip_core::connection::Direction;
use rvoip_core::error::{Result as RvoipResult, RvoipError};
use rvoip_core::ids::StreamId;
use rvoip_core::stream::{MediaFrame, MediaStream, QualitySnapshot, StreamKind};
use rvoip_uctp::substrate::datagram::{
    pack_rtp_datagram, unpack_rtp_datagram, RtpDatagram, RtpMediaPayload,
};
use rvoip_uctp::substrate::{PeerMediaRegistration, PeerMediaRouteKey, PeerMediaRouter};
use rvoip_uctp::CorrelationIdDiagnostic;
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
                let datagram = RtpDatagram {
                    flags: 0,
                    stream_local_id,
                    seq,
                    rtp: RtpMediaPayload {
                        payload: frame.payload,
                        payload_type: frame.payload_type.unwrap_or(default_payload_type),
                        sequence_number: rtp_seq,
                        timestamp: frame.timestamp_rtp,
                        ssrc,
                    },
                };
                let bytes = match pack_rtp_datagram(&datagram) {
                    Ok(bytes) => bytes,
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
                if let Err(e) = session_for_pump.send_datagram(bytes) {
                    metrics::counter!(
                        "uctp_datagram_drops_total",
                        "direction" => "out",
                        "transport" => "webtransport",
                        "reason" => "send-failed"
                    )
                    .increment(1);
                    debug!(error = %e, stream = ?CorrelationIdDiagnostic::new(stream_id_for_pump.as_str()), "rvoip-webtransport: send_datagram failed");
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

/// Compatibility wrapper for alpha callers that supplied a concrete stream
/// vector. Production peers use [`spawn_datagram_reader_with_cancel`] with a
/// single authenticated [`PeerMediaRouter`].
#[doc(hidden)]
pub fn spawn_datagram_reader(
    session: web_transport_quinn::Session,
    router: Arc<parking_lot::RwLock<Vec<Arc<WebTransportDatagramMediaStream>>>>,
    _legacy_fanout: Option<()>,
) {
    let peer_router = legacy_peer_router(&router.read());
    std::mem::drop(spawn_datagram_reader_with_cancel(
        session,
        peer_router,
        None,
        CancellationToken::new(),
    ));
}

/// Spawn the sole datagram reader for one authenticated physical peer.
///
/// The peer-global router resolves the header's sole `stream_local_id` into an
/// authenticated core route. Publisher fanout uses the key retained on that
/// exact binding, never a first-Session/global context.
pub fn spawn_datagram_reader_with_cancel(
    session: web_transport_quinn::Session,
    router: Arc<PeerMediaRouter>,
    orchestrator: Option<Arc<rvoip_core::Orchestrator>>,
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
                    let datagram = match unpack_rtp_datagram(&bytes) {
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
                    let target = NonZeroU16::new(datagram.stream_local_id)
                        .and_then(|local_id| router.lookup(local_id));
                    match target {
                        Some(binding) => {
                            if binding.is_cancelled() {
                                continue;
                            }
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
                                    stream_id: binding.stream().id(),
                                    kind: binding.stream().kind(),
                                    payload: datagram.rtp.payload,
                                    timestamp_rtp: datagram.rtp.timestamp,
                                    captured_at: Utc::now(),
                                    payload_type: Some(datagram.rtp.payload_type),
                                };
                                match binding.ingress().try_send(frame.clone()) {
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
                            if let (Some(orchestrator), Some(fanout)) =
                                (orchestrator.as_ref(), binding.fanout())
                            {
                                let _ = orchestrator
                                    .fanout_frame(
                                        &fanout.session_id,
                                        &fanout.publisher_connection_id,
                                        &fanout.stream_id,
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

/// Build a peer router for legacy direct-stream tests. The production server
/// never uses this path; it creates authenticated route bindings from
/// coordinator events.
fn legacy_peer_router(streams: &[Arc<WebTransportDatagramMediaStream>]) -> Arc<PeerMediaRouter> {
    let router = PeerMediaRouter::new();
    let owner = rvoip_core::identity::PrincipalOwnershipKey {
        issuer: None,
        tenant: None,
        subject: "legacy-webtransport-reader".into(),
    };
    let session_id = rvoip_core::ids::SessionId::from_string("legacy-webtransport-session");
    let connection_id =
        rvoip_core::ids::ConnectionId::from_string("legacy-webtransport-connection");
    let mut ordered = streams.to_vec();
    ordered.sort_unstable_by_key(|stream| stream.stream_local_id());
    let mut registered = HashSet::new();

    for stream in ordered {
        let Some(target) = NonZeroU16::new(stream.stream_local_id()) else {
            continue;
        };
        if !registered.insert(target) {
            continue;
        }
        let reservation = loop {
            let Ok(candidate) = router.reserve() else {
                return router;
            };
            if candidate.local_id() == target {
                break candidate;
            }
            if candidate.local_id() > target {
                return router;
            }
            drop(candidate);
        };
        let stream_dyn: Arc<dyn MediaStream> = stream.clone();
        let route = PeerMediaRouteKey::new(session_id.clone(), connection_id.clone(), stream.id());
        let registration =
            PeerMediaRegistration::new(owner.clone(), route, stream_dyn, stream.inbound_tx());
        if reservation.commit(registration).is_err() {
            return router;
        }
    }
    router
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
