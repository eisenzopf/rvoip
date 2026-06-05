//! H2: ICE restart + hold/resume SDP renegotiation.

#![cfg(feature = "signaling-whip")]

use std::sync::Arc;
use std::time::Duration;

use rvoip_core::adapter::{ConnectionAdapter, OriginateRequest};
use rvoip_core::connection::Direction;
use rvoip_core::ids::{ParticipantId, SessionId};
use rvoip_webrtc::peer::{PeerRole, RvoipPeerConnection};
use rvoip_webrtc::{WebRtcAdapter, WebRtcConfig};

fn ice_ufrag(sdp: &str) -> Option<&str> {
    for line in sdp.lines() {
        if let Some(rest) = line.trim().strip_prefix("a=ice-ufrag:") {
            return Some(rest);
        }
    }
    None
}

#[tokio::test]
async fn restart_ice_produces_new_ufrag_on_offerer() {
    // ICE restart on the offerer rolls the local ufrag/pwd. The answerer's
    // ufrag only changes in response to a *received* offer with new ufrag —
    // see `whip_patch_ice_restart_returns_new_answer` for that path.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let server = WebRtcAdapter::new(WebRtcConfig::loopback());
    let handle = server
        .originate(OriginateRequest {
            session_id: SessionId::new(),
            participant_id: ParticipantId::new(),
            target: String::new(),
            direction: Direction::Outbound,
            capabilities: server.capabilities(),
            transport: None,
        })
        .await
        .expect("originate");
    let conn_id = handle.connection.id.clone();
    let offer1 = server.local_sdp(&conn_id).expect("offer1");
    let ufrag1 = ice_ufrag(&offer1).expect("ufrag1").to_owned();

    let offer2 = server.restart_ice(&conn_id).await.expect("restart");
    let ufrag2 = ice_ufrag(&offer2).expect("ufrag2").to_owned();

    assert_ne!(
        ufrag1, ufrag2,
        "ICE restart on offerer must produce a new ufrag (was {ufrag1}, still {ufrag2})"
    );

    let _ = tokio::time::timeout(
        Duration::from_secs(2),
        server.end(conn_id, rvoip_core::adapter::EndReason::Normal),
    )
    .await;
}

#[tokio::test]
async fn hold_resume_updates_local_sdp_when_renegotiation_enabled() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let config = WebRtcConfig {
        hold_renegotiate: true,
        ..WebRtcConfig::loopback()
    };
    let server = WebRtcAdapter::new(config);

    // Outbound offerer route so we have a stable peer to renegotiate.
    let handle = server
        .originate(OriginateRequest {
            session_id: SessionId::new(),
            participant_id: ParticipantId::new(),
            target: String::new(),
            direction: Direction::Outbound,
            capabilities: server.capabilities(),
            transport: None,
        })
        .await
        .expect("originate");
    let conn_id = handle.connection.id;
    let sdp1 = server.local_sdp(&conn_id).expect("sdp1");

    server.hold(conn_id.clone()).await.expect("hold");
    let sdp2 = server.local_sdp(&conn_id).expect("sdp2");
    assert_ne!(sdp1, sdp2, "hold renegotiation should update the local SDP");

    server.resume(conn_id.clone()).await.expect("resume");
    let sdp3 = server.local_sdp(&conn_id).expect("sdp3");
    assert_ne!(
        sdp2, sdp3,
        "resume renegotiation should update the local SDP again"
    );

    // Tear down so the session reaper doesn't hold the test runtime.
    let _ = tokio::time::timeout(
        Duration::from_secs(2),
        server.end(conn_id, rvoip_core::adapter::EndReason::Normal),
    )
    .await;
}
