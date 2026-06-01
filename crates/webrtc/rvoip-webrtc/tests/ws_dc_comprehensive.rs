//! WS signaling + data channel (mirrors comprehensive client path).

#![cfg(feature = "comprehensive")]

use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use rvoip_core::adapter::ConnectionAdapter;
use rvoip_core::commands::InboundAction;
use rvoip_core::config::Config;
use rvoip_core::events::Event;
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId};
use rvoip_core::orchestrator::Orchestrator;
use rvoip_webrtc::client::comprehensive::{handle_server_connection, prepare_offer_media, run_client_checks};
use rvoip_webrtc::client::{CallTarget, SessionMedium, WebRtcClient, WsSignaler};
use rvoip_webrtc::peer::{PeerRole, RvoipPeerConnection};
use rvoip_webrtc::signaling::websocket::{serve_listener, SignalingMessage};
use rvoip_webrtc::{WebRtcAdapter, WebRtcConfig, WebRtcServerBuilder};

#[tokio::test]
async fn ws_webrtc_client_with_orchestrator() {
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
    let adapter_for_inbound = server.adapter().clone();
    let orchestrator_for_inbound = Arc::clone(&orchestrator);
    let mut events = orchestrator.subscribe_events();
    tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            if let Event::ConnectionInbound { connection_id, .. } = event {
                let adapter_spawn = Arc::clone(&adapter_for_inbound);
                let conn_spawn = connection_id.clone();
                tokio::spawn(async move {
                    handle_server_connection(adapter_spawn, conn_spawn).await;
                });
                let _ = orchestrator_for_inbound
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

    let client = WebRtcClient::connect(WebRtcConfig::loopback(), &ws_url)
        .await
        .expect("client");
    let session = client
        .call(
            &WsSignaler::new(&ws_url),
            CallTarget::Uri("orch".into()),
            SessionMedium::Audio,
        )
        .await
        .expect("call");

    run_client_checks(&session, SessionMedium::Audio)
        .await
        .expect("checks");

    server.shutdown().await;
}

#[tokio::test]
async fn ws_webrtc_client_comprehensive_checks() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let server = WebRtcServerBuilder::new(WebRtcConfig::loopback())
        .with_ws("127.0.0.1:0")
        .build()
        .await
        .expect("server");
    let ws_url = format!("ws://{}", server.ws_addr().expect("ws"));

    let client = WebRtcClient::connect(WebRtcConfig::loopback(), &ws_url)
        .await
        .expect("client");
    let session = client
        .call(
            &WsSignaler::new(&ws_url),
            CallTarget::Uri("ws-client".into()),
            SessionMedium::Audio,
        )
        .await
        .expect("call");

    let conn = session
        .answer()
        .connection_id
        .as_ref()
        .expect("connection id");
    let conn_id = rvoip_core::ids::ConnectionId::from_string(conn.clone());
    let adapter = server.adapter().clone();
    tokio::spawn(async move {
        handle_server_connection(adapter, conn_id).await;
    });

    run_client_checks(&session, SessionMedium::Audio)
        .await
        .expect("checks");

    server.shutdown().await;
}

#[tokio::test]
async fn ws_offer_with_data_channel_ping_pong() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = WebRtcConfig::loopback();
    let adapter = WebRtcAdapter::new(config.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");

    let ws_adapter = Arc::clone(&adapter);
    let server = tokio::spawn(async move {
        serve_listener(listener, ws_adapter).await.expect("ws");
    });

    let offerer = RvoipPeerConnection::new(&config, PeerRole::Offerer).await.expect("offerer");
    let client_dc = prepare_offer_media(&offerer, SessionMedium::Audio)
        .await
        .expect("prepare");
    let offer_sdp = offerer.create_offer_and_gather().await.expect("offer");

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
        .await
        .expect("connect");
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        serde_json::to_string(&SignalingMessage {
            msg_type: "offer".into(),
            sdp: offer_sdp,
            connection_id: String::new(),
            candidate: String::new(),
        })
        .unwrap()
        .into(),
    ))
    .await
    .expect("send offer");

    let answer_frame = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("answer timeout")
        .expect("frame")
        .expect("ws");
    let answer: SignalingMessage =
        serde_json::from_str(answer_frame.to_text().unwrap()).expect("json");
    offerer.set_remote_answer(&answer.sdp).await.expect("set answer");

    let conn_id = rvoip_core::ids::ConnectionId::from_string(answer.connection_id);
    let adapter_spawn = Arc::clone(&adapter);
    tokio::spawn(async move {
        handle_server_connection(adapter_spawn, conn_id).await;
    });

    offerer.wait_connected(Duration::from_secs(10)).await.expect("connected");
    RvoipPeerConnection::wait_data_channel_open(&client_dc, Duration::from_secs(10))
        .await
        .expect("dc open");
    client_dc.send_text("ping").await.expect("ping");
    let reply = RvoipPeerConnection::recv_data_channel_text(&client_dc, Duration::from_secs(10))
        .await
        .expect("pong");
    assert_eq!(reply, "pong");

    server.abort();
}
