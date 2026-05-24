//! In-process comprehensive WebRtcServer + WebRtcClient validation.

#![cfg(feature = "comprehensive")]

use std::sync::Arc;
use std::time::Duration;

use rvoip_core::adapter::ConnectionAdapter;
use rvoip_core::commands::InboundAction;
use rvoip_core::config::Config;
use rvoip_core::events::Event;
use rvoip_core::ids::{ParticipantId, SessionId};
use rvoip_core::orchestrator::Orchestrator;
use rvoip_webrtc::client::{
    comprehensive::{handle_server_connection, run_client_checks},
    CallTarget, SessionMedium, WebRtcClient, WsSignaler,
};
use rvoip_webrtc::{WebRtcConfig, WebRtcServerBuilder};

fn spawn_inbound_accept_and_handler(
    adapter: Arc<rvoip_webrtc::WebRtcAdapter>,
    orchestrator: Arc<Orchestrator>,
) {
    let mut events = orchestrator.subscribe_events();
    tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            if let Event::ConnectionInbound { connection_id, .. } = event {
                let adapter_spawn = Arc::clone(&adapter);
                let conn_spawn = connection_id.clone();
                tokio::spawn(async move {
                    handle_server_connection(adapter_spawn, conn_spawn).await;
                });
                let _ = orchestrator
                    .route_inbound_connection(
                        connection_id,
                        InboundAction::Accept {
                            session_id: SessionId::new(),
                            participant_id: ParticipantId::new(),
                        },
                    )
                    .await;
                break;
            }
        }
    });
}

#[tokio::test]
async fn comprehensive_client_server_audio_video_data() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let server = WebRtcServerBuilder::new(WebRtcConfig::loopback())
        .with_ws("127.0.0.1:0")
        .build()
        .await
        .expect("server");

    let ws_url = format!("ws://{}", server.ws_addr().expect("ws addr"));

    let orchestrator = Arc::new(Orchestrator::new(Config::default()));
    orchestrator
        .register(server.adapter() as Arc<dyn ConnectionAdapter>)
        .expect("register");
    spawn_inbound_accept_and_handler(server.adapter().clone(), Arc::clone(&orchestrator));

    tokio::time::sleep(Duration::from_millis(100)).await;

    let client = WebRtcClient::connect(WebRtcConfig::loopback(), &ws_url)
        .await
        .expect("client");
    let session = client
        .call(
            &WsSignaler::new(&ws_url),
            CallTarget::Uri("test".into()),
            SessionMedium::AudioVideo,
        )
        .await
        .expect("call");

    let report = run_client_checks(&session, SessionMedium::AudioVideo)
        .await
        .expect("checks");

    assert!(report.sdp_has_audio);
    assert!(report.sdp_has_video);
    assert!(report.sdp_full_ice);
    assert!(report.gathered_ice_candidates > 0);
    assert!(report.data_channel_ping_pong);
    assert!(report.chat_echo);
    assert!(report.fixture_media_sent);
    assert!(report.remote_video_track);
    assert!(report.dtmf_sent);
    assert!(report.server_confirmed_audio);
    assert!(report.server_confirmed_video);

    server.shutdown().await;
}

#[tokio::test]
async fn comprehensive_medium_audio_only() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let server = WebRtcServerBuilder::new(WebRtcConfig::loopback())
        .with_ws("127.0.0.1:0")
        .build()
        .await
        .expect("server");
    let ws_url = format!("ws://{}", server.ws_addr().expect("ws"));

    let orchestrator = Arc::new(Orchestrator::new(Config::default()));
    orchestrator
        .register(server.adapter() as Arc<dyn ConnectionAdapter>)
        .expect("register");
    spawn_inbound_accept_and_handler(server.adapter().clone(), Arc::clone(&orchestrator));

    tokio::time::sleep(Duration::from_millis(100)).await;

    let client = WebRtcClient::connect(WebRtcConfig::loopback(), &ws_url)
        .await
        .expect("client");
    let session = client
        .call(
            &WsSignaler::new(&ws_url),
            CallTarget::Uri("audio".into()),
            SessionMedium::Audio,
        )
        .await
        .expect("call");

    run_client_checks(&session, SessionMedium::Audio)
        .await
        .expect("audio checks");

    server.shutdown().await;
}
