//! Injectable Amazon Connect media-session lifecycle.
//!
//! The adapter owns only this rvoip-level seam. Production uses
//! [`ChimeWebRtcMediaConnector`], which composes the existing Chime signaling
//! client with [`rvoip_webrtc::RvoipPeerConnection`]. Tests can provide a
//! hermetic session without AWS credentials, public ICE, or another media
//! implementation.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::Mutex as SyncMutex;
use tokio::sync::{mpsc, watch, Mutex as AsyncMutex, Notify};
use tokio::task::JoinHandle;

use rvoip_core::capability::{CodecInfo, NegotiatedCodecs};
use rvoip_core::ids::StreamId;
use rvoip_core::stream::MediaStream;
use rvoip_webrtc::media::{from_tracks_with_dtmf_events, WebRtcMediaStream};
use rvoip_webrtc::{PeerRole, RvoipPeerConnection, WebRtcConfig};

use crate::control::ConnectionData;
use crate::errors::{ConnectError, Result};
use crate::signaling::chime::{
    ChimeCloseOutcome, ChimeSession, ChimeSessionHealth, ChimeSignalingClient, ChimeTerminalCause,
};

/// Immutable options for one Chime/WebRTC media connection attempt.
#[derive(Clone)]
pub struct ConnectMediaConnectOptions {
    /// Base rvoip WebRTC configuration. Per-contact Chime TURN servers are
    /// appended by the production connector.
    pub webrtc: WebRtcConfig,
    /// Absolute duration bound for JOIN/SUBSCRIBE signaling operations.
    pub signaling_timeout: Duration,
    /// Duration bound for ICE/DTLS to become connected.
    pub media_connect_timeout: Duration,
    /// Chime protocol PING interval.
    pub keepalive_interval: Duration,
}

impl fmt::Debug for ConnectMediaConnectOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConnectMediaConnectOptions")
            .field("ice_server_count", &self.webrtc.ice_servers.len())
            .field("signaling_timeout", &self.signaling_timeout)
            .field("media_connect_timeout", &self.media_connect_timeout)
            .field("keepalive_interval", &self.keepalive_interval)
            .finish()
    }
}

/// Typed terminal cause emitted by a connected Amazon media session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ConnectMediaTerminalCause {
    /// The Chime peer explicitly left the meeting.
    RemoteEnded,
    /// Chime returned a non-zero protocol error.
    RemoteError { status: Option<u32> },
    /// The signaling transport closed cleanly without a LEAVE frame.
    TransportClosed,
    /// The signaling transport or wire codec failed.
    TransportError,
    /// ICE/DTLS entered a failed state.
    PeerFailed,
}

impl From<ChimeTerminalCause> for ConnectMediaTerminalCause {
    fn from(cause: ChimeTerminalCause) -> Self {
        match cause {
            ChimeTerminalCause::RemoteLeave => Self::RemoteEnded,
            ChimeTerminalCause::RemoteError { status } => Self::RemoteError { status },
            ChimeTerminalCause::TransportClosed => Self::TransportClosed,
            ChimeTerminalCause::TransportError => Self::TransportError,
        }
    }
}

/// Value-free liveness snapshot for a media session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConnectMediaHealth {
    /// Whether ICE/DTLS currently reports connected.
    pub peer_connected: bool,
    /// Whether the owned Chime signaling task is running.
    pub signaling_running: bool,
    /// Time since the most recent inbound Chime frame.
    pub last_signaling_activity_ago: Duration,
    /// Time since the most recent Chime PONG, if observed.
    pub last_pong_ago: Option<Duration>,
    /// Sticky non-local terminal cause.
    pub terminal: Option<ConnectMediaTerminalCause>,
}

/// Outcome of closing all media resources to one absolute deadline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectMediaCloseOutcome {
    /// Streams, signaling, peer, and owned supervisor completed in time.
    Graceful,
    /// The absolute deadline elapsed and remaining owned tasks were aborted.
    DeadlineAborted,
}

/// Inbound RFC 4733 event surfaced by a media session.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ConnectMediaDtmfEvent {
    /// Received digit. Diagnostics intentionally redact this value.
    pub digit: char,
    /// Normalized event duration.
    pub duration_ms: u32,
}

impl fmt::Debug for ConnectMediaDtmfEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConnectMediaDtmfEvent")
            .field("digit", &"[redacted]")
            .field("duration_ms", &self.duration_ms)
            .finish()
    }
}

/// One connected, controllable Amazon media session.
#[async_trait]
pub trait ConnectMediaSession: Send + Sync {
    /// Negotiated codecs for the bridge route.
    fn negotiated_codecs(&self) -> NegotiatedCodecs;

    /// Snapshot bridgeable media streams without transferring receiver
    /// ownership.
    fn streams(&self) -> Vec<Arc<dyn MediaStream>>;

    /// Take the single inbound DTMF event receiver.
    fn take_dtmf_events(&self) -> Option<mpsc::Receiver<ConnectMediaDtmfEvent>>;

    /// Subscribe to a sticky typed terminal cause.
    fn subscribe_terminal(&self) -> watch::Receiver<Option<ConnectMediaTerminalCause>>;

    /// Snapshot media and signaling liveness.
    fn health(&self) -> ConnectMediaHealth;

    /// Hold outbound audio.
    async fn hold(&self) -> Result<()>;

    /// Resume outbound audio.
    async fn resume(&self) -> Result<()>;

    /// Send RFC 4733 DTMF.
    async fn send_dtmf(&self, digits: &str, duration_ms: u32) -> Result<()>;

    /// Gracefully close all resources until one absolute monotonic deadline.
    async fn close_until(&self, deadline: Instant) -> Result<ConnectMediaCloseOutcome>;

    /// Abort owned background work without awaiting it.
    fn abort(&self);
}

/// Injectable factory for one connected Amazon media session.
#[async_trait]
pub trait ConnectMediaConnector: Send + Sync {
    /// Connect using already-validated Amazon response data and immutable
    /// media options.
    async fn connect(
        &self,
        connection: &ConnectionData,
        options: ConnectMediaConnectOptions,
    ) -> Result<Arc<dyn ConnectMediaSession>>;
}

/// Production connector backed by Chime signaling and rvoip WebRTC.
#[derive(Clone, Copy, Debug, Default)]
pub struct ChimeWebRtcMediaConnector;

#[async_trait]
impl ConnectMediaConnector for ChimeWebRtcMediaConnector {
    async fn connect(
        &self,
        connection: &ConnectionData,
        options: ConnectMediaConnectOptions,
    ) -> Result<Arc<dyn ConnectMediaSession>> {
        connection.validate()?;

        let join = ChimeSignalingClient::join(connection, options.signaling_timeout).await?;
        let mut webrtc = options.webrtc;
        webrtc.ice_servers.extend(join.ice_servers());

        let peer = RvoipPeerConnection::new(&webrtc, PeerRole::Offerer).await?;
        peer.add_local_audio_track().await?;
        let offer_sdp = peer.create_offer_and_gather().await?;
        let (answer_sdp, chime) = join
            .subscribe(
                offer_sdp,
                options.signaling_timeout,
                options.keepalive_interval,
            )
            .await?;
        peer.set_remote_answer(&answer_sdp).await?;
        peer.wait_connected(options.media_connect_timeout).await?;

        let negotiated = NegotiatedCodecs::default();
        let codec = negotiated.audio.clone().unwrap_or_else(opus_codec);
        let payload_type = payload_type_for_audio_codec(&codec);
        let local = peer.local_audio_track().ok_or_else(|| {
            ConnectError::Signaling("connected peer has no local audio track".into())
        })?;
        let local_ssrc = peer.local_audio_ssrc().ok_or_else(|| {
            ConnectError::Signaling("connected peer has no local audio SSRC".into())
        })?;
        let remote = peer.wait_remote_track(Duration::from_millis(500)).await;
        let await_remote_audio = remote.is_none();
        let (native_dtmf_tx, native_dtmf_rx) = mpsc::channel(32);
        let stream = from_tracks_with_dtmf_events(
            StreamId::new(),
            codec,
            local,
            local_ssrc,
            payload_type,
            remote,
            Some(native_dtmf_tx),
        );

        Ok(Arc::new(ChimeWebRtcMediaSession::new(
            peer,
            chime,
            negotiated,
            vec![stream],
            native_dtmf_rx,
            await_remote_audio,
        )))
    }
}

struct ChimeWebRtcMediaSession {
    peer: Arc<RvoipPeerConnection>,
    chime: SyncMutex<Option<ChimeSession>>,
    last_chime_health: SyncMutex<ChimeSessionHealth>,
    negotiated: NegotiatedCodecs,
    streams: Vec<Arc<WebRtcMediaStream>>,
    dtmf_rx: SyncMutex<Option<mpsc::Receiver<ConnectMediaDtmfEvent>>>,
    terminal_rx: watch::Receiver<Option<ConnectMediaTerminalCause>>,
    supervisor: SyncMutex<Option<JoinHandle<()>>>,
    cancel: Arc<Notify>,
    cancelled: Arc<AtomicBool>,
    close_gate: AsyncMutex<()>,
    close_outcome: SyncMutex<Option<ConnectMediaCloseOutcome>>,
    close_failed: AtomicBool,
}

impl ChimeWebRtcMediaSession {
    fn new(
        peer: Arc<RvoipPeerConnection>,
        chime: ChimeSession,
        negotiated: NegotiatedCodecs,
        streams: Vec<Arc<WebRtcMediaStream>>,
        mut native_dtmf_rx: mpsc::Receiver<rvoip_webrtc::media::dtmf::DecodedDtmfEvent>,
        await_remote_audio: bool,
    ) -> Self {
        let mut chime_terminal = chime.subscribe_terminal();
        let initial_health = chime.health();
        let (terminal_tx, terminal_rx) = watch::channel(None);
        let (dtmf_tx, dtmf_rx) = mpsc::channel(32);
        let cancel = Arc::new(Notify::new());
        let cancelled = Arc::new(AtomicBool::new(false));
        let peer_for_supervisor = Arc::clone(&peer);
        let stream_for_late_audio = streams.first().cloned();
        let cancel_for_supervisor = Arc::clone(&cancel);
        let cancelled_for_supervisor = Arc::clone(&cancelled);

        let supervisor = tokio::spawn(async move {
            let peer_failed = peer_for_supervisor.wait_failed();
            tokio::pin!(peer_failed);
            let mut dtmf_open = true;
            let mut remote_audio_pending = await_remote_audio && stream_for_late_audio.is_some();
            loop {
                if cancelled_for_supervisor.load(Ordering::Acquire) {
                    break;
                }
                tokio::select! {
                    _ = cancel_for_supervisor.notified() => break,
                    _ = &mut peer_failed => {
                        let _ = terminal_tx.send(Some(ConnectMediaTerminalCause::PeerFailed));
                        break;
                    }
                    changed = chime_terminal.changed() => {
                        if changed.is_err() {
                            break;
                        }
                        if let Some(cause) = *chime_terminal.borrow_and_update() {
                            let _ = terminal_tx.send(Some(cause.into()));
                            break;
                        }
                    }
                    event = native_dtmf_rx.recv(), if dtmf_open => {
                        if let Some(event) = event {
                            if dtmf_tx.send(ConnectMediaDtmfEvent {
                                digit: event.digit,
                                duration_ms: event.duration_ms,
                            }).await.is_err() {
                                // The adapter may deliberately decline inbound DTMF;
                                // keep terminal supervision alive regardless.
                            }
                        } else {
                            dtmf_open = false;
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_millis(50)), if remote_audio_pending => {
                        if let (Some(stream), Some(remote)) = (
                            stream_for_late_audio.as_ref(),
                            peer_for_supervisor.discover_remote_audio_track().await,
                        ) {
                            stream.attach_remote(remote);
                            remote_audio_pending = false;
                        }
                    }
                }
            }
        });

        Self {
            peer,
            chime: SyncMutex::new(Some(chime)),
            last_chime_health: SyncMutex::new(initial_health),
            negotiated,
            streams,
            dtmf_rx: SyncMutex::new(Some(dtmf_rx)),
            terminal_rx,
            supervisor: SyncMutex::new(Some(supervisor)),
            cancel,
            cancelled,
            close_gate: AsyncMutex::new(()),
            close_outcome: SyncMutex::new(None),
            close_failed: AtomicBool::new(false),
        }
    }

    fn signal_cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.cancel.notify_waiters();
    }

    fn abort_owned(&self) {
        self.signal_cancel();
        if let Some(chime) = self.chime.lock().as_ref() {
            chime.abort();
        }
        if let Some(supervisor) = self.supervisor.lock().as_ref() {
            supervisor.abort();
        }
    }

    fn terminal(&self) -> Option<ConnectMediaTerminalCause> {
        *self.terminal_rx.borrow()
    }
}

#[async_trait]
impl ConnectMediaSession for ChimeWebRtcMediaSession {
    fn negotiated_codecs(&self) -> NegotiatedCodecs {
        self.negotiated.clone()
    }

    fn streams(&self) -> Vec<Arc<dyn MediaStream>> {
        self.streams
            .iter()
            .map(|stream| Arc::clone(stream) as Arc<dyn MediaStream>)
            .collect()
    }

    fn take_dtmf_events(&self) -> Option<mpsc::Receiver<ConnectMediaDtmfEvent>> {
        self.dtmf_rx.lock().take()
    }

    fn subscribe_terminal(&self) -> watch::Receiver<Option<ConnectMediaTerminalCause>> {
        self.terminal_rx.clone()
    }

    fn health(&self) -> ConnectMediaHealth {
        let chime_health = if let Some(chime) = self.chime.lock().as_ref() {
            let health = chime.health();
            *self.last_chime_health.lock() = health;
            health
        } else {
            *self.last_chime_health.lock()
        };
        ConnectMediaHealth {
            peer_connected: !self.cancelled.load(Ordering::Acquire) && self.peer.is_connected(),
            signaling_running: chime_health.running,
            last_signaling_activity_ago: chime_health.last_activity_ago,
            last_pong_ago: chime_health.last_pong_ago,
            terminal: self.terminal().or(chime_health.terminal.map(Into::into)),
        }
    }

    async fn hold(&self) -> Result<()> {
        self.peer.hold_audio().await.map_err(ConnectError::from)
    }

    async fn resume(&self) -> Result<()> {
        self.peer.resume_audio().await.map_err(ConnectError::from)
    }

    async fn send_dtmf(&self, digits: &str, duration_ms: u32) -> Result<()> {
        rvoip_webrtc::media::dtmf::send_dtmf(&self.peer, digits, duration_ms)
            .await
            .map_err(ConnectError::from)
    }

    async fn close_until(&self, deadline: Instant) -> Result<ConnectMediaCloseOutcome> {
        if let Some(outcome) = *self.close_outcome.lock() {
            if self.close_failed.load(Ordering::Acquire) {
                return Err(ConnectError::Signaling(
                    "Amazon media resource close failed".into(),
                ));
            }
            return Ok(outcome);
        }
        if Instant::now() >= deadline {
            self.abort_owned();
            *self.close_outcome.lock() = Some(ConnectMediaCloseOutcome::DeadlineAborted);
            return Ok(ConnectMediaCloseOutcome::DeadlineAborted);
        }

        let close_guard = match tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            self.close_gate.lock(),
        )
        .await
        {
            Ok(guard) => guard,
            Err(_) => {
                self.abort_owned();
                *self.close_outcome.lock() = Some(ConnectMediaCloseOutcome::DeadlineAborted);
                return Ok(ConnectMediaCloseOutcome::DeadlineAborted);
            }
        };
        if let Some(outcome) = *self.close_outcome.lock() {
            drop(close_guard);
            if self.close_failed.load(Ordering::Acquire) {
                return Err(ConnectError::Signaling(
                    "Amazon media resource close failed".into(),
                ));
            }
            return Ok(outcome);
        }

        self.signal_cancel();
        let mut deadline_aborted = false;
        let mut close_failed = false;

        let chime = self.chime.lock().take();
        if let Some(chime) = chime {
            *self.last_chime_health.lock() = chime.health();
            if chime.close_until(deadline).await == ChimeCloseOutcome::DeadlineAborted {
                deadline_aborted = true;
            }
        }

        for stream in &self.streams {
            if Instant::now() >= deadline {
                deadline_aborted = true;
                break;
            }
            match tokio::time::timeout_at(
                tokio::time::Instant::from_std(deadline),
                (Arc::clone(stream) as Arc<dyn MediaStream>).close(),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(_)) => close_failed = true,
                Err(_) => {
                    deadline_aborted = true;
                    break;
                }
            }
        }

        if !deadline_aborted {
            match tokio::time::timeout_at(
                tokio::time::Instant::from_std(deadline),
                self.peer.close(),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(_)) => close_failed = true,
                Err(_) => deadline_aborted = true,
            }
        }

        let supervisor = self.supervisor.lock().take();
        if let Some(mut supervisor) = supervisor {
            if deadline_aborted || Instant::now() >= deadline {
                supervisor.abort();
                let _ = supervisor.await;
                deadline_aborted = true;
            } else if tokio::time::timeout_at(
                tokio::time::Instant::from_std(deadline),
                &mut supervisor,
            )
            .await
            .is_err()
            {
                supervisor.abort();
                let _ = supervisor.await;
                deadline_aborted = true;
            }
        }

        let outcome = if deadline_aborted {
            self.abort_owned();
            ConnectMediaCloseOutcome::DeadlineAborted
        } else {
            ConnectMediaCloseOutcome::Graceful
        };
        if close_failed {
            self.close_failed.store(true, Ordering::Release);
        }
        *self.close_outcome.lock() = Some(outcome);
        drop(close_guard);

        if close_failed {
            return Err(ConnectError::Signaling(
                "Amazon media resource close failed".into(),
            ));
        }
        Ok(outcome)
    }

    fn abort(&self) {
        self.abort_owned();
    }
}

impl Drop for ChimeWebRtcMediaSession {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        self.cancel.notify_waiters();
        if let Some(chime) = self.chime.get_mut().take() {
            chime.abort();
        }
        if let Some(supervisor) = self.supervisor.get_mut().take() {
            supervisor.abort();
        }
    }
}

fn opus_codec() -> CodecInfo {
    CodecInfo {
        name: "opus".into(),
        clock_rate_hz: 48_000,
        channels: 2,
        fmtp: None,
    }
}

fn payload_type_for_audio_codec(codec: &CodecInfo) -> u8 {
    let name = codec.name.to_ascii_lowercase();
    if name.contains("pcmu") {
        0
    } else if name.contains("pcma") {
        8
    } else {
        111
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{SinkExt, StreamExt};
    use prost::Message as _;
    use tokio::net::{TcpListener, TcpStream};
    use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
    use tokio_tungstenite::tungstenite::http::header::SEC_WEBSOCKET_PROTOCOL;
    use tokio_tungstenite::tungstenite::Message as WsMessage;
    use tokio_tungstenite::{accept_hdr_async, WebSocketStream};

    use crate::control::MediaPlacement;
    use crate::signaling::proto::{
        sdk_signal_frame::Type as FrameType, SdkJoinAckFrame, SdkPingPongFrame, SdkPingPongType,
        SdkSignalFrame, SdkSubscribeAckFrame,
    };

    const TEST_FRAME_TYPE_RTC: u8 = 0x05;

    async fn recv_test_frame(ws: &mut WebSocketStream<TcpStream>) -> SdkSignalFrame {
        loop {
            let message = ws
                .next()
                .await
                .expect("test Chime socket remains open")
                .expect("valid test WebSocket message");
            if let WsMessage::Binary(bytes) = message {
                assert_eq!(bytes.first().copied(), Some(TEST_FRAME_TYPE_RTC));
                return SdkSignalFrame::decode(&bytes[1..]).expect("valid Chime frame");
            }
        }
    }

    async fn send_test_frame(ws: &mut WebSocketStream<TcpStream>, frame: SdkSignalFrame) {
        let mut bytes = vec![TEST_FRAME_TYPE_RTC];
        frame.encode(&mut bytes).expect("encode Chime frame");
        ws.send(WsMessage::Binary(bytes.into()))
            .await
            .expect("send Chime frame");
    }

    fn local_webrtc_config() -> WebRtcConfig {
        WebRtcConfig::loopback()
    }

    async fn local_chime_connection() -> (TcpListener, ConnectionData) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind local Chime server");
        let address = listener.local_addr().expect("local Chime address");
        (
            listener,
            ConnectionData {
                contact_id: "contact-local".into(),
                participant_id: "participant-local".into(),
                participant_token: "participant-token-local".into(),
                meeting_id: "meeting-local".into(),
                media_region: "local".into(),
                attendee_id: "attendee-local".into(),
                join_token: "join-token-local".into(),
                media_placement: MediaPlacement {
                    signaling_url: format!("ws://{address}/control/meeting-local"),
                    audio_host_url: "audio.local.test".into(),
                    ..Default::default()
                },
            },
        )
    }

    #[test]
    fn diagnostics_do_not_expose_dtmf_or_ice_material() {
        let options = ConnectMediaConnectOptions {
            webrtc: WebRtcConfig {
                ice_servers: vec![rvoip_webrtc::IceServerConfig::turn(
                    "turn:secret-host",
                    "secret-user",
                    "secret-credential",
                )],
                ..WebRtcConfig::default()
            },
            signaling_timeout: Duration::from_secs(1),
            media_connect_timeout: Duration::from_secs(2),
            keepalive_interval: Duration::from_secs(3),
        };
        let event = ConnectMediaDtmfEvent {
            digit: '9',
            duration_ms: 100,
        };
        let diagnostics = format!("{options:?} {event:?}");
        for secret in ["secret-host", "secret-user", "secret-credential", "'9'"] {
            assert!(!diagnostics.contains(secret), "leaked {secret}");
        }
    }

    #[test]
    fn chime_causes_map_without_opaque_details() {
        assert_eq!(
            ConnectMediaTerminalCause::from(ChimeTerminalCause::RemoteLeave),
            ConnectMediaTerminalCause::RemoteEnded
        );
        assert_eq!(
            ConnectMediaTerminalCause::from(ChimeTerminalCause::RemoteError { status: Some(500) }),
            ConnectMediaTerminalCause::RemoteError { status: Some(500) }
        );
    }

    #[tokio::test]
    async fn production_connector_uses_local_chime_and_rvoip_webrtc_only() {
        let (listener, connection) = local_chime_connection().await;
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.expect("accept Chime client");
            let mut ws = accept_hdr_async(tcp, |request: &Request, mut response: Response| {
                let protocols = request
                    .headers()
                    .get(SEC_WEBSOCKET_PROTOCOL)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default();
                assert!(protocols
                    .split(',')
                    .any(|value| value.trim() == "_aws_wt_session"));
                response.headers_mut().insert(
                    SEC_WEBSOCKET_PROTOCOL,
                    "_aws_wt_session".parse().expect("static protocol"),
                );
                Ok(response)
            })
            .await
            .expect("accept Chime WebSocket");

            assert_eq!(
                recv_test_frame(&mut ws).await.r#type,
                FrameType::Join as i32
            );
            send_test_frame(
                &mut ws,
                SdkSignalFrame {
                    timestamp_ms: 1,
                    r#type: FrameType::JoinAck as i32,
                    joinack: Some(SdkJoinAckFrame::default()),
                    ..Default::default()
                },
            )
            .await;

            let subscribe = recv_test_frame(&mut ws).await;
            assert_eq!(subscribe.r#type, FrameType::Subscribe as i32);
            let offer = subscribe
                .sub
                .and_then(|frame| frame.sdp_offer)
                .expect("client SDP offer");
            let answerer = RvoipPeerConnection::new(&local_webrtc_config(), PeerRole::Answerer)
                .await
                .expect("build local answerer");
            let answer = answerer
                .accept_offer_and_gather(&offer)
                .await
                .expect("answer local offer");
            send_test_frame(
                &mut ws,
                SdkSignalFrame {
                    timestamp_ms: 2,
                    r#type: FrameType::SubscribeAck as i32,
                    suback: Some(SdkSubscribeAckFrame {
                        sdp_answer: Some(answer),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
            .await;
            answerer
                .wait_connected(Duration::from_secs(5))
                .await
                .expect("local answerer connected");

            loop {
                let frame = recv_test_frame(&mut ws).await;
                if frame.r#type == FrameType::Leave as i32 {
                    break;
                }
                if let Some(ping) = frame.ping_pong {
                    if ping.r#type == SdkPingPongType::Ping as i32 {
                        send_test_frame(
                            &mut ws,
                            SdkSignalFrame {
                                timestamp_ms: 3,
                                r#type: FrameType::PingPong as i32,
                                ping_pong: Some(SdkPingPongFrame {
                                    r#type: SdkPingPongType::Pong as i32,
                                    ping_id: ping.ping_id,
                                }),
                                ..Default::default()
                            },
                        )
                        .await;
                    }
                }
            }
            answerer.close().await.expect("close local answerer");
        });

        let session = ChimeWebRtcMediaConnector
            .connect(
                &connection,
                ConnectMediaConnectOptions {
                    webrtc: local_webrtc_config(),
                    signaling_timeout: Duration::from_secs(5),
                    media_connect_timeout: Duration::from_secs(5),
                    keepalive_interval: Duration::from_millis(20),
                },
            )
            .await
            .expect("production connector succeeds hermetically");
        assert_eq!(session.streams().len(), 1);
        assert!(session.health().peer_connected);
        assert!(session.health().signaling_running);
        session.hold().await.expect("hold local media");
        session.resume().await.expect("resume local media");
        session.send_dtmf("5", 80).await.expect("send local DTMF");
        assert_eq!(
            session
                .close_until(Instant::now() + Duration::from_secs(5))
                .await
                .expect("close local media"),
            ConnectMediaCloseOutcome::Graceful
        );
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .expect("local Chime server drains")
            .expect("local Chime server task");
    }
}
