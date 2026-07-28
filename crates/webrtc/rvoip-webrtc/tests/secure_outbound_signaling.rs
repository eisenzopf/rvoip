//! Hermetic HTTPS/WSS qualification for target-contacting signaling.
//!
//! The generated CA is scoped to each originate context: the tests never
//! disable certificate or hostname verification and never mutate global TLS
//! roots.

#![cfg(all(
    feature = "tls-rustls",
    feature = "signaling-whip",
    feature = "signaling-ws"
))]

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::header::{AUTHORIZATION, LOCATION};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, post};
use axum::Router;
use rcgen::generate_simple_self_signed;
use rvoip_core::adapter::{AdapterEvent, ConnectionAdapter, EndReason, OriginateRequest};
use rvoip_core::config::Config as CoreConfig;
use rvoip_core::connection::Direction;
use rvoip_core::conversation::ConversationPolicy;
use rvoip_core::events::Event;
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId, TenantId};
use rvoip_core::orchestrator::Orchestrator;
use rvoip_core::session::SessionMedium;
use rvoip_core::stream::{MediaFrame, StreamKind};
use rvoip_core::DataMessage;
use rvoip_webrtc::signaling::auth::BearerStaticTokenAuth;
use rvoip_webrtc::signaling::whip::WhepServerMode;
use rvoip_webrtc::tls::TlsConfig;
use rvoip_webrtc::{
    StaticWebRtcBearerCredentialProvider, WebRtcAdapter, WebRtcBearerCredential, WebRtcConfig,
    WebRtcIceExchangePolicy, WebRtcOriginateContext, WebRtcServerBuilder, WebRtcSignalingMode,
    WebRtcTargetPolicy, WebRtcTlsClientTrust,
};

fn self_signed_pem() -> (Vec<u8>, Vec<u8>) {
    let certificate =
        generate_simple_self_signed(vec!["localhost".into()]).expect("test certificate");
    (
        certificate.cert.pem().into_bytes(),
        certificate.signing_key.serialize_pem().into_bytes(),
    )
}

fn provider() -> Arc<StaticWebRtcBearerCredentialProvider> {
    Arc::new(StaticWebRtcBearerCredentialProvider::new(
        WebRtcBearerCredential::new("secret").expect("test bearer"),
    ))
}

fn target_policy(port: u16) -> WebRtcTargetPolicy {
    WebRtcTargetPolicy::default()
        .allow_port(port)
        .allow_loopback(true)
        .with_credential_partition("secure-loopback")
        .expect("credential partition")
        .with_timeouts(Duration::from_secs(3), Duration::from_secs(10))
        .expect("bounded timeouts")
}

#[tokio::test]
async fn whips_client_completes_ice_dtls_opus_and_conditional_cleanup() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let (cert_pem, key_pem) = self_signed_pem();
    let trust = Arc::new(WebRtcTlsClientTrust::from_pem(&cert_pem).expect("client trust"));
    let tls = TlsConfig::from_pem_bytes(&cert_pem, &key_pem)
        .await
        .expect("server TLS");
    let mut server_config = WebRtcConfig::loopback();
    server_config.trickle_ice = true;
    let server = WebRtcServerBuilder::new(server_config)
        .with_whips("127.0.0.1:0", tls)
        .with_whip_auth(Arc::new(BearerStaticTokenAuth::new("secret")))
        .build()
        .await
        .expect("WHIPS server");
    let server_adapter = server.adapter();
    let mut server_events = server_adapter.subscribe_events();
    let address = server.whips_addr().expect("WHIPS address");

    let client_adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let mut client_events = client_adapter.subscribe_events();
    let endpoint = format!("https://localhost:{}/whip/secure", address.port());
    let context = WebRtcOriginateContext::new(
        &endpoint,
        WebRtcSignalingMode::Whip,
        WebRtcIceExchangePolicy::Trickle,
        target_policy(address.port()),
        Some(provider()),
    )
    .expect("WHIPS context")
    .with_tls_trust(trust);
    let client_connection = prepare(&client_adapter, endpoint, context).await;

    client_adapter
        .activate_outbound(client_connection.clone())
        .await
        .expect("WHIPS activation");
    let server_connection = wait_for_inbound_connection(&mut server_events).await;
    let (client_connected, server_connected) = tokio::join!(
        client_adapter.accept(client_connection.clone()),
        server_adapter.accept(server_connection.clone()),
    );
    client_connected.expect("client ICE/DTLS");
    server_connected.expect("server ICE/DTLS");
    exchange_bidirectional_audio(
        &client_adapter,
        &client_connection,
        &server_adapter,
        &server_connection,
    )
    .await;

    // Open arbitrary labeled channels only after ICE/DTLS is established.
    // Both directions must surface the exact transport-neutral envelope.
    let client_message = DataMessage::reliable(
        "late.client.context",
        "application/json",
        bytes::Bytes::from_static(br#"{"correlation_id":"late-client"}"#),
    );
    client_adapter
        .send_data_message(client_connection.clone(), client_message.clone())
        .await
        .expect("late client DataChannel message");
    assert_eq!(
        wait_for_data_message(&mut server_events, &server_connection).await,
        client_message
    );

    let server_message = DataMessage::reliable(
        "late.server.binary",
        "application/octet-stream",
        bytes::Bytes::from_static(b"\0\xfflate-server"),
    );
    server_adapter
        .send_data_message(server_connection.clone(), server_message.clone())
        .await
        .expect("late server DataChannel message");
    assert_eq!(
        wait_for_data_message(&mut client_events, &client_connection).await,
        server_message
    );

    client_adapter
        .end(client_connection.clone(), EndReason::Normal)
        .await
        .expect("WHIPS conditional delete");
    let cleaned = wait_until_result(Duration::from_secs(3), || {
        !server_adapter.is_connection_live(&server_connection)
            && client_adapter.outbound_signaling_task_count() == 0
            && client_adapter.metrics().peer_session_tasks == 0
            && client_adapter.metrics().media_tasks == 0
            && server_adapter.metrics().peer_session_tasks == 0
            && server_adapter.metrics().media_tasks == 0
            && server_adapter.metrics().active_http_resources == 0
    })
    .await;
    assert!(
        cleaned,
        "WHIPS cleanup did not converge: client={:?}, server={:?}, outbound_drivers={}",
        client_adapter.metrics(),
        server_adapter.metrics(),
        client_adapter.outbound_signaling_task_count(),
    );
    assert!(!client_adapter.is_connection_live(&client_connection));
    assert!(client_adapter.routes().is_empty());
    assert!(server_adapter.routes().is_empty());
    server.shutdown().await;
}

#[tokio::test]
async fn wheps_client_completes_canonical_playback_media_and_cleanup() {
    run_secure_whep_case(WhepServerMode::Draft04).await;
}

#[tokio::test]
async fn wheps_client_completes_exact_406_counter_offer_and_cleanup() {
    run_secure_whep_case(WhepServerMode::Draft04CounterOffer).await;
}

async fn run_secure_whep_case(mode: WhepServerMode) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let (cert_pem, key_pem) = self_signed_pem();
    let trust = Arc::new(WebRtcTlsClientTrust::from_pem(&cert_pem).expect("client trust"));
    let tls = TlsConfig::from_pem_bytes(&cert_pem, &key_pem)
        .await
        .expect("server TLS");
    let ice_policy = if mode == WhepServerMode::Draft04 {
        WebRtcIceExchangePolicy::Trickle
    } else {
        WebRtcIceExchangePolicy::FullGather
    };
    let mut server_config = WebRtcConfig::loopback();
    server_config.trickle_ice = ice_policy == WebRtcIceExchangePolicy::Trickle;
    let server = WebRtcServerBuilder::new(server_config)
        .with_whips("127.0.0.1:0", tls)
        .with_whip_auth(Arc::new(BearerStaticTokenAuth::new("secret")))
        .with_whep_server_mode(mode)
        .build()
        .await
        .expect("WHEPS server");
    let server_adapter = server.adapter();
    let mut server_events = server_adapter.subscribe_events();
    let address = server.whips_addr().expect("WHEPS address");

    let client_adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let endpoint = format!("https://localhost:{}/whep/secure", address.port());
    let context = WebRtcOriginateContext::new(
        &endpoint,
        WebRtcSignalingMode::Whep,
        ice_policy,
        target_policy(address.port()),
        Some(provider()),
    )
    .expect("WHEPS context")
    .with_tls_trust(trust);
    let client_connection = prepare(&client_adapter, endpoint, context).await;

    client_adapter
        .activate_outbound(client_connection.clone())
        .await
        .expect("WHEPS activation");
    let server_connection = wait_for_inbound_connection(&mut server_events).await;
    let (client_connected, server_connected) = tokio::join!(
        client_adapter.accept(client_connection.clone()),
        server_adapter.accept(server_connection.clone()),
    );
    client_connected.expect("WHEPS client ICE/DTLS");
    server_connected.expect("WHEPS origin ICE/DTLS");
    exchange_one_way_audio(
        &server_adapter,
        &server_connection,
        &client_adapter,
        &client_connection,
    )
    .await;

    client_adapter
        .end(client_connection.clone(), EndReason::Normal)
        .await
        .expect("WHEPS conditional delete");
    wait_until(Duration::from_secs(3), || {
        !server_adapter.is_connection_live(&server_connection)
            && client_adapter.outbound_signaling_task_count() == 0
            && client_adapter.metrics().peer_session_tasks == 0
            && client_adapter.metrics().media_tasks == 0
            && server_adapter.metrics().peer_session_tasks == 0
            && server_adapter.metrics().media_tasks == 0
            && server_adapter.metrics().active_http_resources == 0
            && server_adapter.metrics().http_resource_tasks == 0
    })
    .await;
    assert!(client_adapter.routes().is_empty());
    assert!(server_adapter.routes().is_empty());
    server.shutdown().await;
}

#[tokio::test]
async fn wss_client_requires_exact_subprotocol_and_cleans_hub_after_opus() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let (cert_pem, key_pem) = self_signed_pem();
    let trust = Arc::new(WebRtcTlsClientTrust::from_pem(&cert_pem).expect("client trust"));
    let tls = TlsConfig::from_pem_bytes(&cert_pem, &key_pem)
        .await
        .expect("server TLS");
    let mut server_config = WebRtcConfig::loopback();
    server_config.trickle_ice = true;
    let server = WebRtcServerBuilder::new(server_config)
        .with_wss("127.0.0.1:0", tls)
        .with_ws_auth(Arc::new(BearerStaticTokenAuth::new("secret")))
        .build()
        .await
        .expect("WSS server");
    let server_adapter = server.adapter();
    let mut server_events = server_adapter.subscribe_events();
    let address = server.wss_addr().expect("WSS address");

    let client_adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let endpoint = format!("wss://localhost:{}/signal", address.port());
    let context = WebRtcOriginateContext::websocket(&endpoint, target_policy(address.port()))
        .expect("WSS context")
        .with_bearer_provider(provider())
        .with_tls_trust(trust);
    let orchestrator = Orchestrator::new(CoreConfig::default());
    orchestrator
        .register(Arc::clone(&client_adapter) as Arc<dyn ConnectionAdapter>)
        .expect("register target-contacting adapter");
    let conversation_id = orchestrator
        .open_conversation(
            TenantId::new(),
            ConversationPolicy::default(),
            std::collections::HashMap::new(),
        )
        .await
        .expect("open outbound conversation");
    let session_id = orchestrator
        .start_session(conversation_id, SessionMedium::Voice, vec![])
        .await
        .expect("start outbound voice session");
    let prepared = orchestrator
        .prepare_outbound_connection(
            OriginateRequest::new(
                session_id,
                ParticipantId::new(),
                endpoint,
                Direction::Outbound,
                client_adapter.capabilities(),
            )
            .with_context(context),
        )
        .await
        .expect("prepare WSS route through Orchestrator");
    let client_connection = prepared.connection_id().clone();
    let commit = tokio::spawn(async move { prepared.commit().await });
    let server_connection = wait_for_inbound_connection(&mut server_events).await;
    server_adapter
        .accept(server_connection.clone())
        .await
        .expect("server ICE/DTLS");
    commit
        .await
        .expect("Orchestrator commit task")
        .expect("Orchestrator commit completes client ICE/DTLS");

    // No adapter-private `accept` call is required after the staged commit.
    // Media and arbitrary DataChannels are ready at this boundary.
    exchange_bidirectional_audio(
        &client_adapter,
        &client_connection,
        &server_adapter,
        &server_connection,
    )
    .await;
    let client_message = DataMessage::reliable(
        "orchestrator.client.binary",
        "application/octet-stream",
        bytes::Bytes::from_static(b"\0orchestrated-client"),
    );
    client_adapter
        .send_data_message(client_connection.clone(), client_message.clone())
        .await
        .expect("Orchestrator-ready client DataChannel");
    assert_eq!(
        wait_for_data_message(&mut server_events, &server_connection).await,
        client_message
    );

    // Activation and accept remain idempotent for legacy direct-adapter
    // callers and cannot publish a second Connected transition.
    let mut events = orchestrator.subscribe_events();
    client_adapter
        .activate_outbound(client_connection.clone())
        .await
        .expect("idempotent target activation");
    client_adapter
        .accept(client_connection.clone())
        .await
        .expect("idempotent target accept");
    let duplicate = tokio::time::timeout(Duration::from_millis(150), async {
        loop {
            match events.recv().await {
                Ok(Event::ConnectionConnected { connection_id, .. })
                    if connection_id == client_connection =>
                {
                    return true;
                }
                Ok(_) => {}
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(!duplicate, "repeated activation published Connected twice");

    client_adapter
        .end(client_connection.clone(), EndReason::Normal)
        .await
        .expect("WSS BYE");
    wait_until(Duration::from_secs(3), || {
        !server_adapter.is_connection_live(&server_connection)
            && client_adapter.outbound_signaling_task_count() == 0
            && client_adapter.outbound_ws_hub_task_count() == 0
            && client_adapter.metrics().peer_session_tasks == 0
            && client_adapter.metrics().media_tasks == 0
            && server_adapter.metrics().peer_session_tasks == 0
            && server_adapter.metrics().media_tasks == 0
            && server_adapter.metrics().inbound_ws_connection_tasks == 0
    })
    .await;
    assert!(client_adapter.routes().is_empty());
    assert!(server_adapter.routes().is_empty());
    server.shutdown().await;
}

#[tokio::test]
async fn wss_server_end_sends_route_scoped_bye_and_cleans_client() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let (cert_pem, key_pem) = self_signed_pem();
    let trust = Arc::new(WebRtcTlsClientTrust::from_pem(&cert_pem).expect("client trust"));
    let tls = TlsConfig::from_pem_bytes(&cert_pem, &key_pem)
        .await
        .expect("server TLS");
    let mut server_config = WebRtcConfig::loopback();
    server_config.trickle_ice = true;
    let server = WebRtcServerBuilder::new(server_config)
        .with_wss("127.0.0.1:0", tls)
        .with_ws_auth(Arc::new(BearerStaticTokenAuth::new("secret")))
        .build()
        .await
        .expect("WSS server");
    let server_adapter = server.adapter();
    let mut server_events = server_adapter.subscribe_events();
    let address = server.wss_addr().expect("WSS address");

    let client_adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let endpoint = format!("wss://localhost:{}/signal", address.port());
    let context = WebRtcOriginateContext::websocket(&endpoint, target_policy(address.port()))
        .expect("WSS context")
        .with_bearer_provider(provider())
        .with_tls_trust(trust);
    let client_connection = prepare(&client_adapter, endpoint, context).await;
    let activation = {
        let adapter = Arc::clone(&client_adapter);
        let connection = client_connection.clone();
        tokio::spawn(async move { adapter.activate_outbound(connection).await })
    };
    let server_connection = wait_for_inbound_connection(&mut server_events).await;
    server_adapter
        .accept(server_connection.clone())
        .await
        .expect("server ICE/DTLS");
    activation
        .await
        .expect("client activation task")
        .expect("client ICE/DTLS");

    server_adapter
        .end(server_connection.clone(), EndReason::Normal)
        .await
        .expect("server-originated WSS end");
    wait_until(Duration::from_secs(3), || {
        !client_adapter.is_connection_live(&client_connection)
            && !server_adapter.is_connection_live(&server_connection)
            && client_adapter.outbound_signaling_task_count() == 0
            && client_adapter.outbound_ws_hub_task_count() == 0
            && client_adapter.metrics().peer_session_tasks == 0
            && client_adapter.metrics().media_tasks == 0
            && server_adapter.metrics().peer_session_tasks == 0
            && server_adapter.metrics().media_tasks == 0
            && server_adapter.metrics().inbound_ws_connection_tasks == 0
    })
    .await;
    assert!(client_adapter.routes().is_empty());
    assert!(server_adapter.routes().is_empty());
    server.shutdown().await;
}

#[derive(Clone)]
struct RedirectState {
    location: String,
    requests: Arc<AtomicUsize>,
    saw_authorization: Arc<AtomicBool>,
}

async fn redirect_creation(State(state): State<RedirectState>, headers: HeaderMap) -> Response {
    state.requests.fetch_add(1, Ordering::SeqCst);
    state.saw_authorization.store(
        headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            == Some("Bearer secret"),
        Ordering::SeqCst,
    );
    let mut response = StatusCode::TEMPORARY_REDIRECT.into_response();
    response.headers_mut().insert(
        LOCATION,
        HeaderValue::from_str(&state.location).expect("redirect location"),
    );
    response
}

async fn count_redirect_target(State(requests): State<Arc<AtomicUsize>>) -> StatusCode {
    requests.fetch_add(1, Ordering::SeqCst);
    StatusCode::NO_CONTENT
}

#[tokio::test]
async fn whips_redirect_never_crosses_the_credential_origin() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let (cert_pem, key_pem) = self_signed_pem();
    let trust = Arc::new(WebRtcTlsClientTrust::from_pem(&cert_pem).expect("client trust"));
    let target_requests = Arc::new(AtomicUsize::new(0));
    let (target_address, target_task) = spawn_https(
        Router::new()
            .fallback(any(count_redirect_target))
            .with_state(Arc::clone(&target_requests)),
        &cert_pem,
        &key_pem,
    )
    .await;
    let origin_requests = Arc::new(AtomicUsize::new(0));
    let saw_authorization = Arc::new(AtomicBool::new(false));
    let origin_state = RedirectState {
        location: format!("https://localhost:{}/stolen", target_address.port()),
        requests: Arc::clone(&origin_requests),
        saw_authorization: Arc::clone(&saw_authorization),
    };
    let (origin_address, origin_task) = spawn_https(
        Router::new()
            .route("/whip/redirect", post(redirect_creation))
            .with_state(origin_state),
        &cert_pem,
        &key_pem,
    )
    .await;

    let adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let endpoint = format!("https://localhost:{}/whip/redirect", origin_address.port());
    let context = WebRtcOriginateContext::new(
        &endpoint,
        WebRtcSignalingMode::Whip,
        WebRtcIceExchangePolicy::FullGather,
        target_policy(origin_address.port()),
        Some(provider()),
    )
    .expect("redirect context")
    .with_tls_trust(trust);
    let connection = prepare(&adapter, endpoint, context).await;
    adapter
        .activate_outbound(connection.clone())
        .await
        .expect_err("redirect must be rejected");
    assert_eq!(origin_requests.load(Ordering::SeqCst), 1);
    assert!(saw_authorization.load(Ordering::SeqCst));
    assert_eq!(
        target_requests.load(Ordering::SeqCst),
        0,
        "redirect target must receive neither request nor credential"
    );
    adapter
        .end(connection.clone(), EndReason::Normal)
        .await
        .expect("failed-route cleanup");
    wait_until(Duration::from_secs(2), || {
        adapter.outbound_signaling_task_count() == 0
    })
    .await;
    assert!(!adapter.is_connection_live(&connection));
    origin_task.abort();
    target_task.abort();
}

async fn spawn_https(
    app: Router,
    cert_pem: &[u8],
    key_pem: &[u8],
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind HTTPS fixture");
    listener
        .set_nonblocking(true)
        .expect("nonblocking HTTPS fixture");
    let address = listener.local_addr().expect("HTTPS fixture address");
    let tls = axum_server::tls_rustls::RustlsConfig::from_pem(cert_pem.to_vec(), key_pem.to_vec())
        .await
        .expect("HTTPS fixture TLS");
    let task = tokio::spawn(async move {
        axum_server::from_tcp_rustls(listener, tls)
            .serve(app.into_make_service())
            .await
            .expect("serve HTTPS fixture");
    });
    (address, task)
}

async fn prepare(
    adapter: &Arc<WebRtcAdapter>,
    endpoint: String,
    context: WebRtcOriginateContext,
) -> ConnectionId {
    adapter
        .originate(
            OriginateRequest::new(
                SessionId::new(),
                ParticipantId::new(),
                endpoint,
                Direction::Outbound,
                adapter.capabilities(),
            )
            .with_context(context),
        )
        .await
        .expect("prepare outbound route")
        .connection
        .id
}

async fn wait_for_inbound_connection(
    events: &mut tokio::sync::mpsc::Receiver<AdapterEvent>,
) -> ConnectionId {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Some(AdapterEvent::InboundConnection { connection }) => return connection.id,
                Some(_) => {}
                None => panic!("server event stream closed"),
            }
        }
    })
    .await
    .expect("inbound connection")
}

async fn wait_for_data_message(
    events: &mut tokio::sync::mpsc::Receiver<AdapterEvent>,
    connection_id: &ConnectionId,
) -> DataMessage {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match events.recv().await {
                Some(AdapterEvent::DataMessage {
                    connection_id: received_connection,
                    message,
                }) if &received_connection == connection_id => return message,
                Some(_) => {}
                None => panic!("adapter event stream closed"),
            }
        }
    })
    .await
    .expect("late DataChannel message")
}

async fn exchange_bidirectional_audio(
    first_adapter: &Arc<WebRtcAdapter>,
    first_connection: &ConnectionId,
    second_adapter: &Arc<WebRtcAdapter>,
    second_connection: &ConnectionId,
) {
    let first = first_adapter
        .streams(first_connection.clone())
        .await
        .expect("first streams")
        .into_iter()
        .find(|stream| stream.kind() == StreamKind::Audio)
        .expect("first audio stream");
    let second = second_adapter
        .streams(second_connection.clone())
        .await
        .expect("second streams")
        .into_iter()
        .find(|stream| stream.kind() == StreamKind::Audio)
        .expect("second audio stream");
    assert_eq!(first.codec().name.to_ascii_lowercase(), "opus");
    assert_eq!(second.codec().name.to_ascii_lowercase(), "opus");
    let mut first_inbound = first.try_frames_in().expect("first inbound");
    let mut second_inbound = second.try_frames_in().expect("second inbound");
    send_audio_burst(&first, 10).await;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), second_inbound.recv())
            .await
            .expect("first-to-second timeout")
            .expect("first-to-second closed")
            .payload,
        rvoip_webrtc::media::silent_opus_payload()
    );
    send_audio_burst(&second, 30).await;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), first_inbound.recv())
            .await
            .expect("second-to-first timeout")
            .expect("second-to-first closed")
            .payload,
        rvoip_webrtc::media::silent_opus_payload()
    );
}

async fn exchange_one_way_audio(
    sender_adapter: &Arc<WebRtcAdapter>,
    sender_connection: &ConnectionId,
    receiver_adapter: &Arc<WebRtcAdapter>,
    receiver_connection: &ConnectionId,
) {
    let sender = sender_adapter
        .streams(sender_connection.clone())
        .await
        .expect("sender streams")
        .into_iter()
        .find(|stream| stream.kind() == StreamKind::Audio)
        .expect("sender audio stream");
    let receiver = receiver_adapter
        .streams(receiver_connection.clone())
        .await
        .expect("receiver streams")
        .into_iter()
        .find(|stream| stream.kind() == StreamKind::Audio)
        .expect("receiver audio stream");
    assert_eq!(sender.codec().name.to_ascii_lowercase(), "opus");
    assert_eq!(receiver.codec().name.to_ascii_lowercase(), "opus");
    let mut inbound = receiver.try_frames_in().expect("receiver inbound");
    send_audio_burst(&sender, 50).await;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), inbound.recv())
            .await
            .expect("WHEP media timeout")
            .expect("WHEP media closed")
            .payload,
        rvoip_webrtc::media::silent_opus_payload()
    );
}

async fn send_audio_burst(stream: &Arc<dyn rvoip_core::stream::MediaStream>, base: u32) {
    for sequence in 0..10u32 {
        stream
            .frames_out()
            .send(MediaFrame {
                stream_id: stream.id(),
                kind: StreamKind::Audio,
                payload: rvoip_webrtc::media::silent_opus_payload(),
                timestamp_rtp: (base + sequence) * 960,
                captured_at: chrono::Utc::now(),
                payload_type: None,
            })
            .await
            .expect("send Opus frame");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_until(mut remaining: Duration, mut predicate: impl FnMut() -> bool) {
    while !predicate() {
        assert!(!remaining.is_zero(), "cleanup deadline exceeded");
        let slice = remaining.min(Duration::from_millis(10));
        tokio::time::sleep(slice).await;
        remaining = remaining.saturating_sub(slice);
    }
}

async fn wait_until_result(mut remaining: Duration, mut predicate: impl FnMut() -> bool) -> bool {
    while !predicate() {
        if remaining.is_zero() {
            return false;
        }
        let slice = remaining.min(Duration::from_millis(10));
        tokio::time::sleep(slice).await;
        remaining = remaining.saturating_sub(slice);
    }
    true
}
