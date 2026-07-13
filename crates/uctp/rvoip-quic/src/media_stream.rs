//! `QuicDatagramMediaStream` — implements `rvoip_core::MediaStream` over
//! QUIC datagrams using the UCTP 8-byte header.
//!
//! Per design doc §4.5 / §3.6. Construction spawns two driver tasks: an
//! **outbound pump** that drains `frames_out` → checked UCTP/RTP packing
//! → `quinn::Connection::send_datagram`, and an **inbound pump** the
//! adapter feeds via [`QuicDatagramMediaStream::inbound_tx`] from the
//! physical peer's sole `quinn::Connection::read_datagram` loop.

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
use rvoip_uctp::substrate::{
    pack_rtp_datagram, unpack_rtp_datagram, PeerMediaRegistration, PeerMediaRouteKey,
    PeerMediaRouter, RtpDatagram, RtpMediaPayload,
};
use rvoip_uctp::CorrelationIdDiagnostic;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
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
    /// Adapter feeds inbound `MediaFrame`s here from its peer-level datagram
    /// reader (one reader serves every logical Session and Stream).
    inbound_tx: mpsc::Sender<MediaFrame>,
    quality: parking_lot::RwLock<QualitySnapshot>,
    cancel: CancellationToken,
    outbound_task: StdMutex<Option<tokio::task::JoinHandle<()>>>,
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
        Self::start_with_cancel(
            id,
            kind,
            codec,
            direction,
            stream_local_id,
            conn,
            CancellationToken::new(),
        )
    }

    /// Construct a stream whose background media pump is coupled to the
    /// owning peer session. Cancelling `peer_cancel` terminates the pump even
    /// when application code still retains a `frames_out` sender.
    pub fn start_with_cancel(
        id: StreamId,
        kind: StreamKind,
        codec: CodecInfo,
        direction: Direction,
        stream_local_id: u16,
        conn: quinn::Connection,
        peer_cancel: CancellationToken,
    ) -> Arc<Self> {
        let (in_tx, in_rx) = mpsc::channel::<MediaFrame>(FRAME_CAP);
        let (out_tx, mut out_rx) = mpsc::channel::<MediaFrame>(FRAME_CAP);

        let stream_cancel = peer_cancel.child_token();
        let pump_cancel = stream_cancel.clone();
        let session_cancel = peer_cancel.clone();

        // Outbound pump: frames_out → pack → quinn::Connection::send_datagram.
        let conn_for_pump = conn.clone();
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
                    transport = "quic",
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
                            "transport" => "quic",
                            "reason" => "rtp-encode"
                        )
                        .increment(1);
                        debug!(%error, "rvoip-quic: RTP encode failed");
                        continue;
                    }
                };
                if let Err(e) = conn_for_pump.send_datagram(bytes) {
                    // Backpressure policy §3.5: drop + counter, do NOT close.
                    metrics::counter!(
                        "uctp_datagram_drops_total",
                        "direction" => "out",
                        "transport" => "quic",
                        "reason" => "send-failed"
                    )
                    .increment(1);
                    debug!(error = %e, stream = ?CorrelationIdDiagnostic::new(stream_id_for_pump.as_str()), "rvoip-quic: send_datagram failed");
                    if matches!(e, quinn::SendDatagramError::ConnectionLost(_)) {
                        session_cancel.cancel();
                        break;
                    }
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
            cancel: stream_cancel,
            outbound_task: StdMutex::new(Some(outbound_task)),
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

    pub fn is_closed(&self) -> bool {
        self.cancel.is_cancelled()
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
        self.try_frames_in().unwrap_or_else(|_| mpsc::channel(1).1)
    }

    fn try_frames_in(&self) -> RvoipResult<mpsc::Receiver<MediaFrame>> {
        self.in_rx
            .lock()
            .map_err(|_| RvoipError::InvalidState("QUIC media receiver lock is poisoned"))?
            .take()
            .ok_or(RvoipError::InvalidState(
                "QUIC media receiver has already been acquired",
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
            .map_err(|_| RvoipError::InvalidState("QUIC media task lock is poisoned"))?
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
    conn: quinn::Connection,
    router: Arc<parking_lot::RwLock<Vec<Arc<QuicDatagramMediaStream>>>>,
    _legacy_fanout: Option<()>,
) {
    let peer_router = legacy_peer_router(&router.read());
    std::mem::drop(spawn_datagram_reader_with_cancel(
        conn,
        peer_router,
        None,
        CancellationToken::new(),
    ));
}

/// Spawn the sole datagram reader for one authenticated physical peer.
///
/// Each datagram resolves through the peer-global local-ID namespace. The
/// selected binding supplies its own ingress channel and optional fanout key,
/// so simultaneous Sessions on one QUIC connection cannot alias or inherit the
/// first Session's fanout context.
pub fn spawn_datagram_reader_with_cancel(
    conn: quinn::Connection,
    router: Arc<PeerMediaRouter>,
    orchestrator: Option<Arc<rvoip_core::Orchestrator>>,
    peer_cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let received = tokio::select! {
                _ = peer_cancel.cancelled() => return,
                received = conn.read_datagram() => received,
            };
            match received {
                Ok(bytes) => {
                    let datagram = match unpack_rtp_datagram(&bytes) {
                        Ok(d) => d,
                        Err(_) => {
                            metrics::counter!(
                                "uctp_datagram_drops_total",
                                "direction" => "in",
                                "transport" => "quic",
                                "reason" => "invalid-uctp-rtp"
                            )
                            .increment(1);
                            continue;
                        }
                    };
                    let Some(local_id) = NonZeroU16::new(datagram.stream_local_id) else {
                        metrics::counter!(
                            "uctp_datagram_drops_total",
                            "direction" => "in",
                            "transport" => "quic",
                            "reason" => "zero-stream"
                        )
                        .increment(1);
                        continue;
                    };
                    match router.lookup(local_id) {
                        Some(binding) => {
                            if binding.is_cancelled() {
                                continue;
                            }
                            // Build the frame and do the local route
                            // inside a tight scope so the non-Send
                            // tracing span guard is dropped before we
                            // await the fanout call.
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
                                    stream_id: binding.route().stream_id.clone(),
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
                    peer_cancel.cancel();
                    return;
                }
                Err(e) => {
                    warn!(error = %e, "rvoip-quic: datagram reader exiting on connection error");
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
fn legacy_peer_router(streams: &[Arc<QuicDatagramMediaStream>]) -> Arc<PeerMediaRouter> {
    let router = PeerMediaRouter::new();
    let owner = rvoip_core::identity::PrincipalOwnershipKey {
        issuer: None,
        tenant: None,
        subject: "legacy-quic-reader".into(),
    };
    let session_id = rvoip_core::ids::SessionId::from_string("legacy-quic-session");
    let connection_id = rvoip_core::ids::ConnectionId::from_string("legacy-quic-connection");
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
        let stream = QuicDatagramMediaStream {
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
