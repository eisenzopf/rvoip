//! Amazon Chime SDK signaling client (protobuf-over-secure-WebSocket).
//!
//! This speaks the same wire protocol the official `amazon-chime-sdk-js`
//! `DefaultSignalingClient` uses to join a meeting's media session. Each
//! WebSocket **binary** message is exactly one `prost`-encoded [`SdkSignalFrame`]
//! (the schema in [`super::proto`]); WebSocket framing delimits the messages, so
//! no extra length prefix is applied.
//!
//! Join handshake driven here:
//!
//! ```text
//!   client ──JOIN─────────▶ media server      (SdkJoinFrame: protocol_version=2, audio session)
//!   client ◀──JOIN_ACK──── media server       (SdkJoinAckFrame: TURN credentials)
//!   client ──SUBSCRIBE────▶ media server       (SdkSubscribeFrame: sdp_offer, audio_host, TX/DUPLEX)
//!   client ◀─SUBSCRIBE_ACK─ media server       (SdkSubscribeAckFrame: sdp_answer)
//!   client ◀──PING/PONG──▶ media server        (keepalive, every keepalive_interval)
//!   client ──LEAVE────────▶ media server       (teardown)
//! ```
//!
//! The peer connection is created by the caller *between* JOIN_ACK and
//! SUBSCRIBE so the TURN credentials returned in the JOIN_ACK can seed its ICE
//! servers — hence the two-step [`ChimeSignalingClient::join`] → [`ChimeJoin::subscribe`]
//! API.
//!
//! > **Live-validation note:** the exact signaling-URL query string and the
//! > JOIN frame's optional fields are reconstructed from the public JS SDK and
//! > schema. The wire format (one protobuf `SdkSignalFrame` per binary message)
//! > is stable; the URL/credential wiring is the piece to confirm against a
//! > live Amazon Connect instance (feature `aws-live`). All such wiring is
//! > localized to [`build_signaling_url`] and [`ChimeJoin::subscribe`].

use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use prost::Message as _;
use tokio::net::TcpStream;
use tokio::sync::{oneshot, Notify};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use rvoip_webrtc::IceServerConfig;

use crate::control::ConnectionData;
use crate::errors::{ConnectError, Result};
use crate::signaling::proto::{
    sdk_signal_frame::Type as FrameType, SdkClientDetails, SdkJoinFrame, SdkPingPongFrame,
    SdkPingPongType, SdkSignalFrame, SdkStreamDescriptor, SdkStreamMediaType, SdkStreamServiceType,
    SdkSubscribeFrame, SdkTurnCredentials,
};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Chime frames every binary WebSocket message with a 1-byte frame-type prefix
/// (`DefaultSignalingClient.FRAME_TYPE_RTC`). The protobuf `SdkSignalFrame`
/// follows. We prepend it on send and strip it on receive.
const FRAME_TYPE_RTC: u8 = 0x05;

/// A monotonically-increasing-ish timestamp for signal frames. `timestamp_ms`
/// is `required` in the schema; the server treats it as informational, so a
/// process-relative millisecond counter is sufficient and avoids depending on
/// wall-clock APIs.
fn now_ms() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Build the Chime signaling WebSocket URL.
///
/// Matches the Amazon Chime SDK exactly
/// (`SignalingClientConnectionRequest.url()`): the query is precisely these two
/// params. The attendee **join token is NOT in the URL** — it authenticates via
/// the `Sec-WebSocket-Protocol` subprotocol header (see [`ChimeSignalingClient::join`]).
fn build_signaling_url(signaling_url: &str) -> String {
    let sep = if signaling_url.contains('?') {
        '&'
    } else {
        '?'
    };
    format!(
        "{signaling_url}{sep}X-Chime-Control-Protocol-Version=3&X-Amzn-Chime-Send-Close-On-Error=1"
    )
}

/// Build the JOIN frame our client sends. Public so the `connect-probe`
/// `--dump-frames` path can emit the exact bytes for diffing against the
/// browser widget's captured frames.
pub fn build_join_frame() -> SdkSignalFrame {
    SdkSignalFrame {
        timestamp_ms: now_ms(),
        r#type: FrameType::Join as i32,
        join: Some(SdkJoinFrame {
            protocol_version: Some(2),
            max_num_of_videos: Some(0),
            flags: Some(0),
            client_details: Some(SdkClientDetails {
                app_name: Some("rvoip".into()),
                client_source: Some("rvoip-amazon-connect".into()),
                chime_sdk_version: Some(env!("CARGO_PKG_VERSION").into()),
                ..Default::default()
            }),
            wants_compressed_sdp: Some(false),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Build the SUBSCRIBE frame (carrying the SDP offer). Public for `--dump-frames`.
///
/// Matches the Amazon Chime SDK's audio-only `subscribe()`:
/// `duplex = RX` (DUPLEX is video-only — sending DUPLEX without a video m-line
/// makes the server fail with "failed to initialize video session"), and one
/// `AmazonChimeExpressAudio` send-stream descriptor carrying our attendee id.
pub fn build_subscribe_frame(
    offer_sdp: String,
    audio_host: String,
    attendee_id: String,
) -> SdkSignalFrame {
    let audio_stream = SdkStreamDescriptor {
        media_type: Some(SdkStreamMediaType::Audio as i32),
        track_label: Some("AmazonChimeExpressAudio".into()),
        attendee_id: Some(attendee_id),
        stream_id: Some(1),
        group_id: Some(1),
        framerate: Some(15),
        max_bitrate_kbps: Some(600),
        avg_bitrate_bps: Some(400_000),
        ..Default::default()
    };
    SdkSignalFrame {
        timestamp_ms: now_ms(),
        r#type: FrameType::Subscribe as i32,
        sub: Some(SdkSubscribeFrame {
            // Audio-only: RX (no local video). Audio still flows both ways via
            // the bundled connection + this send-stream + the SDP m-line.
            duplex: Some(SdkStreamServiceType::Rx as i32),
            send_streams: vec![audio_stream],
            sdp_offer: Some(offer_sdp),
            audio_host: Some(audio_host),
            audio_checkin: Some(false),
            audio_muted: Some(false),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Base64 (standard) of a frame's protobuf encoding — the exact bytes that go
/// on the wire as one binary WebSocket message. Matches what Chrome DevTools
/// shows for `Network.webSocketFrame*`, so probe output and browser captures
/// are directly comparable via the `chime-decode` tool.
pub fn frame_to_base64(frame: &SdkSignalFrame) -> String {
    use base64::Engine as _;
    let mut buf = Vec::with_capacity(frame.encoded_len());
    // Infallible for a well-formed message into a Vec.
    let _ = frame.encode(&mut buf);
    base64::engine::general_purpose::STANDARD.encode(buf)
}

/// The signaling URL for logging. The join token is sent as a WebSocket
/// subprotocol (not in the URL), so the URL is already safe to log as-is.
pub fn redacted_signaling_url(signaling_url: &str, _join_token: &str) -> String {
    build_signaling_url(signaling_url)
}

/// Encode and send one signal frame as a binary WebSocket message, with the
/// Chime 1-byte frame-type prefix.
async fn send_frame(ws: &mut Ws, frame: &SdkSignalFrame) -> Result<()> {
    let mut buf = Vec::with_capacity(frame.encoded_len() + 1);
    buf.push(FRAME_TYPE_RTC);
    frame
        .encode(&mut buf)
        .map_err(|e| ConnectError::Signaling(format!("encode frame: {e}")))?;
    // Wire trace for the protocol-oracle diff loop: enable with
    // `RUST_LOG=rvoip_amazon_connect::chime_wire=trace`.
    tracing::trace!(
        target: "rvoip_amazon_connect::chime_wire",
        direction = "tx",
        frame_type = frame.r#type,
        b64 = %{ use base64::Engine as _; base64::engine::general_purpose::STANDARD.encode(&buf) },
        "chime signal frame sent"
    );
    ws.send(WsMessage::Binary(buf.into()))
        .await
        .map_err(|e| ConnectError::Signaling(format!("websocket send: {e}")))?;
    Ok(())
}

/// Receive the next decodable signal frame, skipping non-binary control frames.
/// Returns `Ok(None)` when the socket closes.
async fn recv_frame(ws: &mut Ws) -> Result<Option<SdkSignalFrame>> {
    while let Some(msg) = ws.next().await {
        let msg = msg.map_err(|e| ConnectError::Signaling(format!("websocket recv: {e}")))?;
        match msg {
            WsMessage::Binary(bytes) => {
                if bytes.is_empty() {
                    continue;
                }
                // Strip the 1-byte frame-type prefix before decoding the protobuf.
                let frame = SdkSignalFrame::decode(&bytes[1..])
                    .map_err(|e| ConnectError::Signaling(format!("decode frame: {e}")))?;
                return Ok(Some(frame));
            }
            WsMessage::Close(_) => return Ok(None),
            // Ping/Pong/Text are not part of the Chime signaling payload.
            _ => continue,
        }
    }
    Ok(None)
}

/// Receive frames until one of `wanted` type arrives (or timeout / close).
/// Surfaces a server `SdkErrorFrame` as [`ConnectError::ServerFrame`].
async fn recv_until(ws: &mut Ws, wanted: FrameType, timeout: Duration) -> Result<SdkSignalFrame> {
    let deadline = async {
        loop {
            match recv_frame(ws).await? {
                None => {
                    return Err(ConnectError::Signaling(
                        "socket closed during handshake".into(),
                    ))
                }
                Some(frame) => {
                    if frame.r#type == FrameType::Notification as i32 {
                        continue;
                    }
                    if let Some(err) = &frame.error {
                        if err.status.unwrap_or(0) != 0 {
                            return Err(ConnectError::ServerFrame {
                                status: err.status,
                                description: err.description.clone().unwrap_or_default(),
                            });
                        }
                    }
                    if frame.r#type == wanted as i32 {
                        return Ok(frame);
                    }
                    // Other frame types before the one we want are benign
                    // (INDEX, AUDIO_*); keep reading.
                }
            }
        }
    };
    tokio::time::timeout(timeout, deadline)
        .await
        .map_err(|_| ConnectError::Timeout("chime signaling handshake"))?
}

/// Convert Chime TURN credentials into rvoip-webrtc ICE server configs.
pub fn ice_servers_from_turn(turn: &SdkTurnCredentials) -> Vec<IceServerConfig> {
    let username = turn.username.clone().unwrap_or_default();
    let credential = turn.password.clone().unwrap_or_default();
    turn.uris
        .iter()
        .map(|uri| IceServerConfig {
            urls: vec![uri.clone()],
            username: Some(username.clone()),
            credential: Some(credential.clone()),
        })
        .collect()
}

/// Install the `ring` rustls CryptoProvider as the process default, once.
///
/// rustls 0.23 refuses to auto-select a provider when more than one backend is
/// linked. The AWS SDK pulls in `aws-lc-rs` while the workspace standardizes on
/// `ring`, so without an explicit choice the first TLS handshake panics. We pin
/// `ring` to match the rest of the workspace. Idempotent and harmless if a
/// provider was already installed by the host application.
fn ensure_crypto_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// Entry point: opens the signaling socket and completes JOIN/JOIN_ACK.
pub struct ChimeSignalingClient;

impl ChimeSignalingClient {
    /// Connect to the meeting's signaling websocket and perform the JOIN
    /// handshake, returning the post-JOIN handle (which carries the TURN
    /// credentials needed to build the peer connection).
    pub async fn join(conn: &ConnectionData, timeout: Duration) -> Result<ChimeJoin> {
        ensure_crypto_provider();
        let url = build_signaling_url(&conn.media_placement.signaling_url);

        // Auth: the attendee join token rides in the `Sec-WebSocket-Protocol`
        // header, exactly as the Chime SDK does
        // (`protocols(): ['_aws_wt_session', joinToken]`).
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        use tokio_tungstenite::tungstenite::http::header::SEC_WEBSOCKET_PROTOCOL;
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|e| ConnectError::Signaling(format!("ws request: {e}")))?;
        request.headers_mut().insert(
            SEC_WEBSOCKET_PROTOCOL,
            format!("_aws_wt_session, {}", conn.join_token)
                .parse()
                .map_err(|e| ConnectError::Signaling(format!("subprotocol header: {e}")))?,
        );

        let (mut ws, _resp) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|e| ConnectError::Signaling(format!("websocket connect: {e}")))?;

        send_frame(&mut ws, &build_join_frame()).await?;

        let ack = recv_until(&mut ws, FrameType::JoinAck, timeout).await?;
        let turn = ack.joinack.and_then(|a| a.turn_credentials);
        let audio_host = conn.media_placement.audio_host_url.clone();

        Ok(ChimeJoin {
            ws,
            turn,
            audio_host,
            attendee_id: conn.attendee_id.clone(),
        })
    }
}

/// Post-JOIN handle. Exposes the TURN credentials (so the caller can build the
/// peer connection) and the [`ChimeJoin::subscribe`] step that exchanges SDP.
pub struct ChimeJoin {
    ws: Ws,
    turn: Option<SdkTurnCredentials>,
    audio_host: String,
    attendee_id: String,
}

impl ChimeJoin {
    /// ICE servers derived from the JOIN_ACK TURN credentials. Empty when the
    /// server did not return any (rare; the meeting may then rely on host/srflx
    /// candidates only).
    pub fn ice_servers(&self) -> Vec<IceServerConfig> {
        self.turn
            .as_ref()
            .map(ice_servers_from_turn)
            .unwrap_or_default()
    }

    /// Send the SDP offer in a SUBSCRIBE frame and wait for the SUBSCRIBE_ACK
    /// carrying the SDP answer. Returns the answer plus a running
    /// [`ChimeSession`] that owns the socket for keepalive + teardown.
    pub async fn subscribe(
        mut self,
        offer_sdp: String,
        timeout: Duration,
        keepalive_interval: Duration,
    ) -> Result<(String, ChimeSession)> {
        let sub =
            build_subscribe_frame(offer_sdp, self.audio_host.clone(), self.attendee_id.clone());
        send_frame(&mut self.ws, &sub).await?;

        let ack = recv_until(&mut self.ws, FrameType::SubscribeAck, timeout).await?;
        let answer = ack
            .suback
            .and_then(|s| s.sdp_answer)
            .ok_or(ConnectError::MissingConnectionData("sdp_answer"))?;

        let cancel = Arc::new(Notify::new());
        let (ended_tx, ended_rx) = oneshot::channel();
        let handle = spawn_session_loop(self.ws, keepalive_interval, Arc::clone(&cancel), ended_tx);

        Ok((
            answer,
            ChimeSession {
                handle,
                cancel,
                ended_rx: Some(ended_rx),
            },
        ))
    }
}

/// A live Chime media session: a background task drives keepalive PINGs and
/// drains inbound frames until [`ChimeSession::shutdown`] (which sends LEAVE).
pub struct ChimeSession {
    handle: JoinHandle<()>,
    cancel: Arc<Notify>,
    /// Fires when the session ends on its **own** (agent hangup / socket close /
    /// server error) — i.e. NOT via our [`Self::shutdown`]/[`Self::abort`]. When
    /// we tear down locally the sender is dropped, so the receiver resolves to
    /// `Err`. Lets the adapter surface a reverse-direction `Ended` so the SIP
    /// carrier leg can be hung up.
    ended_rx: Option<oneshot::Receiver<()>>,
}

impl ChimeSession {
    /// Take the "ended on its own" signal (consumed once, by the adapter).
    pub fn take_ended_signal(&mut self) -> Option<oneshot::Receiver<()>> {
        self.ended_rx.take()
    }

    /// Signal LEAVE and await the background task's exit.
    pub async fn shutdown(self) {
        self.cancel.notify_waiters();
        let _ = self.handle.await;
    }

    /// Abort without a graceful LEAVE (used on hard teardown / drop paths).
    pub fn abort(&self) {
        self.cancel.notify_waiters();
        self.handle.abort();
    }
}

/// Background loop: PING keepalive + inbound drain. Exits on cancel (sending a
/// LEAVE — local teardown, no `ended_tx`), or on socket close / server error /
/// send failure (the remote ended — fires `ended_tx`).
fn spawn_session_loop(
    mut ws: Ws,
    keepalive_interval: Duration,
    cancel: Arc<Notify>,
    ended_tx: oneshot::Sender<()>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ping_id: u32 = 1;
        let mut ticker = tokio::time::interval(keepalive_interval);
        // First tick fires immediately; skip it so we don't PING before media.
        ticker.tick().await;

        // `true` when the session ended on its own (remote close/error), `false`
        // when we tore it down via `cancel`.
        let ended_on_own = loop {
            tokio::select! {
                _ = cancel.notified() => {
                    let leave = SdkSignalFrame {
                        timestamp_ms: now_ms(),
                        r#type: FrameType::Leave as i32,
                        leave: Some(Default::default()),
                        ..Default::default()
                    };
                    let _ = send_frame(&mut ws, &leave).await;
                    let _ = ws.close(None).await;
                    break false;
                }
                _ = ticker.tick() => {
                    let ping = SdkSignalFrame {
                        timestamp_ms: now_ms(),
                        r#type: FrameType::PingPong as i32,
                        ping_pong: Some(SdkPingPongFrame {
                            r#type: SdkPingPongType::Ping as i32,
                            ping_id,
                        }),
                        ..Default::default()
                    };
                    ping_id = ping_id.wrapping_add(1);
                    if send_frame(&mut ws, &ping).await.is_err() {
                        break true;
                    }
                }
                frame = recv_frame(&mut ws) => {
                    match frame {
                        Ok(Some(frame)) => {
                            if let Some(err) = &frame.error {
                                if err.status.unwrap_or(0) != 0 {
                                    tracing::warn!(
                                        target: "rvoip_amazon_connect",
                                        status = ?err.status,
                                        description = %err.description.clone().unwrap_or_default(),
                                        "chime signaling server error frame"
                                    );
                                    let _ = ws.close(None).await;
                                    break true;
                                }
                            }
                            // AUDIO_METADATA / AUDIO_STREAM_ID_INFO / PONG etc. are
                            // informational for an audio-only bridge; ignore.
                        }
                        Ok(None) => break true,   // socket closed by the server
                        Err(e) => {
                            tracing::debug!(target: "rvoip_amazon_connect", error = %e, "chime signaling recv ended");
                            break true;
                        }
                    }
                }
            }
        };

        // Notify the adapter only when the remote ended; on local teardown we
        // drop `ended_tx` (receiver sees `Err`) so we don't loop back on
        // ourselves.
        if ended_on_own {
            let _ = ended_tx.send(());
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signaling_url_matches_chime_sdk() {
        // Must match SignalingClientConnectionRequest.url() exactly; the join
        // token is NOT in the URL (it rides in the subprotocol header).
        let url = build_signaling_url("wss://signal.example.com/control/m1");
        assert_eq!(
            url,
            "wss://signal.example.com/control/m1?X-Chime-Control-Protocol-Version=3&X-Amzn-Chime-Send-Close-On-Error=1"
        );
        assert!(!url.contains("tok"));
    }

    #[test]
    fn turn_creds_map_to_ice_servers() {
        let turn = SdkTurnCredentials {
            username: Some("user".into()),
            password: Some("pass".into()),
            ttl: Some(3600),
            uris: vec![
                "turn:1.2.3.4:3478?transport=udp".into(),
                "turn:1.2.3.4:3478?transport=tcp".into(),
            ],
        };
        let ice = ice_servers_from_turn(&turn);
        assert_eq!(ice.len(), 2);
        assert_eq!(ice[0].username.as_deref(), Some("user"));
        assert_eq!(ice[0].credential.as_deref(), Some("pass"));
        assert_eq!(
            ice[0].urls,
            vec!["turn:1.2.3.4:3478?transport=udp".to_string()]
        );
    }

    #[test]
    fn join_frame_roundtrips_through_prost() {
        let frame = SdkSignalFrame {
            timestamp_ms: 42,
            r#type: FrameType::Join as i32,
            join: Some(SdkJoinFrame {
                protocol_version: Some(2),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut buf = Vec::new();
        frame.encode(&mut buf).unwrap();
        let decoded = SdkSignalFrame::decode(&buf[..]).unwrap();
        assert_eq!(decoded.r#type, FrameType::Join as i32);
        assert_eq!(decoded.join.unwrap().protocol_version, Some(2));
    }
}
