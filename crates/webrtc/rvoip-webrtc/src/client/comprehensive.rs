//! Shared WebRTC basics validation — audio, video, SCTP data channel, DTMF, chat.

use std::sync::Arc;
use std::time::Duration;

use crate::client::{SessionHandle, SessionMedium};
use crate::errors::{Result, WebRtcError};
use crate::media::dtmf;
use crate::media::fixtures::send_fixture_media_burst;
use crate::peer::RvoipPeerConnection;
use rtc::rtp_transceiver::rtp_sender::RtpCodecKind;
use webrtc::data_channel::DataChannel;

const DC_LABEL: &str = "rvoip-comprehensive";
const CHAT_PREFIX: &str = "chat:";
const CHAT_ECHO_PREFIX: &str = "chat-echo:";

/// Result of the comprehensive client↔server validation suite.
#[derive(Debug, Default)]
pub struct ComprehensiveReport {
    pub sdp_has_audio: bool,
    pub sdp_has_video: bool,
    pub sdp_full_ice: bool,
    pub ice_connected: bool,
    pub gathered_ice_candidates: usize,
    pub data_channel_open: bool,
    pub data_channel_ping_pong: bool,
    pub chat_echo: bool,
    pub local_audio_track: bool,
    pub local_video_track: bool,
    pub remote_audio_track: bool,
    pub remote_video_track: bool,
    pub fixture_media_sent: bool,
    pub dtmf_sent: bool,
    pub server_confirmed_audio: bool,
    pub server_confirmed_video: bool,
}

impl ComprehensiveReport {
    pub fn all_passed(&self, medium: SessionMedium) -> bool {
        let base = self.sdp_has_audio
            && self.sdp_full_ice
            && self.ice_connected
            && self.gathered_ice_candidates > 0
            && self.data_channel_open
            && self.data_channel_ping_pong
            && self.chat_echo
            && self.local_audio_track
            && self.remote_audio_track
            && self.fixture_media_sent
            && self.dtmf_sent
            && self.server_confirmed_audio;
        match medium {
            SessionMedium::Audio => base,
            SessionMedium::Video | SessionMedium::AudioVideo => {
                base && self.sdp_has_video
                    && self.local_video_track
                    && self.remote_video_track
                    && self.server_confirmed_video
            }
        }
    }

    pub fn failures(&self, medium: SessionMedium) -> Vec<&'static str> {
        let mut out = Vec::new();
        if !self.sdp_has_audio {
            out.push("sdp missing m=audio");
        }
        if !self.sdp_full_ice {
            out.push("sdp missing inline ICE candidates (full gather)");
        }
        if !self.ice_connected {
            out.push("ICE/DTLS not connected");
        }
        if self.gathered_ice_candidates == 0 {
            out.push("no local ICE candidates gathered");
        }
        if !self.data_channel_open {
            out.push("data channel did not open");
        }
        if !self.data_channel_ping_pong {
            out.push("data channel ping/pong failed");
        }
        if !self.chat_echo {
            out.push("data channel chat echo failed");
        }
        if !self.local_audio_track {
            out.push("no local audio track");
        }
        if !self.remote_audio_track {
            out.push("no remote audio track");
        }
        if !self.fixture_media_sent {
            out.push("fixture-encoded media burst failed");
        }
        if !self.dtmf_sent {
            out.push("DTMF send failed");
        }
        if !self.server_confirmed_audio {
            out.push("server did not confirm remote audio");
        }
        if matches!(medium, SessionMedium::Video | SessionMedium::AudioVideo) {
            if !self.sdp_has_video {
                out.push("sdp missing m=video");
            }
            if !self.local_video_track {
                out.push("no local video track");
            }
            if !self.remote_video_track {
                out.push("no remote video track from server");
            }
            if !self.server_confirmed_video {
                out.push("server did not confirm remote video");
            }
        }
        out
    }
}

/// Run the comprehensive validation suite (default chat message).
pub async fn run_client_checks(
    session: &SessionHandle,
    medium: SessionMedium,
) -> Result<ComprehensiveReport> {
    run_client_checks_with_chat(session, medium, "Hello from comprehensive suite!").await
}

/// Run the comprehensive validation suite on an established client session.
pub async fn run_client_checks_with_chat(
    session: &SessionHandle,
    medium: SessionMedium,
    chat_body: &str,
) -> Result<ComprehensiveReport> {
    let mut report = ComprehensiveReport::default();
    let answer_sdp = &session.answer().sdp;
    report.sdp_has_audio = answer_sdp.contains("m=audio");
    report.sdp_has_video = answer_sdp.contains("m=video");
    report.sdp_full_ice = crate::sdp::sdp_has_inline_ice_candidates(answer_sdp);

    session.wait_connected(Duration::from_secs(15)).await?;
    report.ice_connected = true;

    let peer = session.peer();
    report.local_audio_track = peer.local_audio_track().is_some();
    report.local_video_track = peer.local_video_track().is_some();
    report.gathered_ice_candidates = peer.gathered_ice_candidates().len();

    let dc = session.data_channel();
    RvoipPeerConnection::wait_data_channel_open(dc, Duration::from_secs(10)).await?;
    report.data_channel_open = true;

    dc.send_text("ping")
        .await
        .map_err(|e| WebRtcError::Webrtc(format!("dc send: {e}")))?;
    let reply = RvoipPeerConnection::recv_data_channel_text(dc, Duration::from_secs(10)).await?;
    report.data_channel_ping_pong = reply == "pong";

    let chat_msg = format!("{CHAT_PREFIX}{chat_body}");
    dc.send_text(&chat_msg)
        .await
        .map_err(|e| WebRtcError::Webrtc(format!("dc chat send: {e}")))?;
    let chat_reply =
        RvoipPeerConnection::recv_data_channel_text(dc, Duration::from_secs(10)).await?;
    report.chat_echo = chat_reply == format!("{CHAT_ECHO_PREFIX}{chat_body}");

    let include_video = matches!(medium, SessionMedium::Video | SessionMedium::AudioVideo);
    send_fixture_media_burst(peer, include_video).await;
    report.fixture_media_sent = true;

    report.remote_audio_track = peer
        .wait_remote_track_kind(RtpCodecKind::Audio, Duration::from_secs(10))
        .await
        .is_some();

    if include_video {
        report.remote_video_track = peer
            .wait_remote_video_track(Duration::from_secs(10))
            .await
            .is_some();
    }

    if dtmf::send_dtmf(peer, "1", 100).await.is_ok() {
        report.dtmf_sent = true;
    }

    dc.send_text("stats")
        .await
        .map_err(|e| WebRtcError::Webrtc(format!("dc stats send: {e}")))?;
    let stats = RvoipPeerConnection::recv_data_channel_text(dc, Duration::from_secs(10)).await?;
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&stats) {
        report.server_confirmed_audio = v
            .get("remote_audio")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        report.server_confirmed_video = v
            .get("remote_video")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
    }

    if !report.all_passed(medium) {
        let fails = report.failures(medium).join(", ");
        return Err(WebRtcError::Adapter(format!(
            "comprehensive checks failed: {fails}"
        )));
    }

    Ok(report)
}

/// Prepare local media + outbound data channel before creating an offer.
pub async fn prepare_offer_media(
    peer: &Arc<RvoipPeerConnection>,
    medium: SessionMedium,
) -> Result<Arc<dyn DataChannel>> {
    match medium {
        SessionMedium::Audio => peer.add_local_audio_track().await?,
        SessionMedium::Video => peer.add_local_video_track().await?,
        SessionMedium::AudioVideo => {
            peer.add_local_audio_track().await?;
            peer.add_local_video_track().await?;
        }
    }
    peer.create_data_channel(DC_LABEL, crate::peer::DataChannelOptions::reliable())
        .await
}

/// Server-side handlers for comprehensive demo: DC echo + bidirectional fixture media.
pub async fn handle_server_connection(
    adapter: std::sync::Arc<crate::WebRtcAdapter>,
    connection_id: rvoip_core::ids::ConnectionId,
) {
    let Some(route) = adapter.routes().get(&connection_id) else {
        return;
    };
    let peer = route.peer.clone();
    // IMPORTANT: drop the DashMap read guard before `await`ing on the peer.
    // Holding a DashMap ref across await points can deadlock with any task
    // that wants a write lock (notably `WebRtcAdapter::end` / the session
    // reaper) on the same shard. This was the root cause of the H4
    // comprehensive-test hang — the original `let Some(route) = ...` binding
    // held its guard through every `wait_connected` / `wait_data_channel` /
    // `poll_data_channel` await, blocking writers indefinitely.
    drop(route);

    if peer.wait_connected(Duration::from_secs(15)).await.is_err() {
        return;
    }

    let dc = match peer.wait_data_channel(Duration::from_secs(10)).await {
        Some(dc) => dc,
        None => return,
    };

    let peer_media = Arc::clone(&peer);
    tokio::spawn(async move {
        send_fixture_media_burst(&peer_media, peer_media.local_video_track().is_some()).await;
    });

    loop {
        let Some(event) =
            RvoipPeerConnection::poll_data_channel(&dc, Duration::from_millis(100)).await
        else {
            continue;
        };
        if let webrtc::data_channel::DataChannelEvent::OnMessage(msg) = event {
            if !msg.is_string {
                continue;
            }
            let text = String::from_utf8_lossy(&msg.data);
            match text.as_ref() {
                "ping" => {
                    let _ = dc.send_text("pong").await;
                }
                text if text.starts_with(CHAT_PREFIX) => {
                    let body = &text[CHAT_PREFIX.len()..];
                    let _ = dc.send_text(&format!("{CHAT_ECHO_PREFIX}{body}")).await;
                }
                "stats" => {
                    let (remote_audio, remote_video) = peer.remote_media_ready().await;
                    let body = serde_json::json!({
                        "remote_audio": remote_audio,
                        "remote_video": remote_video,
                    });
                    let _ = dc.send_text(&body.to_string()).await;
                }
                _ => {}
            }
        }
    }
}
