//! Fail-closed WebRTC protocol admission tests.

#![cfg(feature = "signaling-whip")]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
#[cfg(feature = "signaling-ws")]
use futures::{SinkExt, StreamExt};
use rvoip_core::adapter::{ConnectionAdapter, EndReason, OriginateRequest, RejectReason};
use rvoip_core::config::Config;
use rvoip_core::connection::Direction;
use rvoip_core::events::Event;
use rvoip_core::identity::{
    AuthenticatedPrincipal, AuthenticationMethod, CredentialKind, IdentityAssurance,
};
use rvoip_core::ids::{ParticipantId, SessionId};
use rvoip_core::orchestrator::Orchestrator;
use rvoip_webrtc::peer::{PeerRole, RvoipPeerConnection};
#[cfg(feature = "signaling-ws")]
use rvoip_webrtc::signaling::auth::WsAuthHook;
use rvoip_webrtc::signaling::auth::{AuthContext, AuthRejection, WhipAuthHook};
#[cfg(feature = "signaling-ws")]
use rvoip_webrtc::signaling::websocket::SignalingMessage;
use rvoip_webrtc::{WebRtcAdapter, WebRtcConfig, WebRtcServer, WebRtcServerBuilder};

struct CompleteAuth;

fn complete_auth_context(session_hint: Option<&str>) -> AuthContext {
    AuthContext {
        subject: "secure-webrtc-user".into(),
        scopes: vec!["whip:publish".into(), "whep:subscribe".into()],
        session_hint: session_hint.map(str::to_owned),
        principal: Some(AuthenticatedPrincipal {
            subject: "secure-webrtc-user".into(),
            tenant: Some("tenant-a".into()),
            scopes: vec!["whip:publish".into(), "whep:subscribe".into()],
            issuer: Some("https://issuer.example".into()),
            expires_at: None,
            method: AuthenticationMethod::Jwt,
            assurance: IdentityAssurance::Identified {
                credential_kind: CredentialKind::Oidc,
            },
        }),
    }
}

#[async_trait]
impl WhipAuthHook for CompleteAuth {
    async fn authenticate(
        &self,
        _method: &str,
        _path: &str,
        _bearer: Option<&str>,
        _peer_addr: SocketAddr,
    ) -> Result<AuthContext, AuthRejection> {
        Ok(complete_auth_context(None))
    }
}

#[cfg(feature = "signaling-ws")]
#[async_trait]
impl WsAuthHook for CompleteAuth {
    async fn authenticate(
        &self,
        _subprotocols: &[String],
        _query_token: Option<&str>,
        _peer_addr: SocketAddr,
    ) -> Result<AuthContext, AuthRejection> {
        Ok(complete_auth_context(Some("ws-attachment")))
    }
}

async fn secure_server(timeout: Duration, max_sessions: usize) -> WebRtcServer {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut config = WebRtcConfig::loopback();
    config.max_concurrent_sessions = max_sessions;
    WebRtcServerBuilder::new(config)
        .with_inbound_admission_confirmation(timeout)
        .with_whip_auth(Arc::new(CompleteAuth))
        .with_whip("127.0.0.1:0")
        .build()
        .await
        .expect("secure WebRTC server")
}

async fn offer() -> (Arc<WebRtcAdapter>, String) {
    let publisher = WebRtcAdapter::new(WebRtcConfig::loopback());
    let handle = publisher
        .originate(OriginateRequest {
            session_id: SessionId::new(),
            participant_id: ParticipantId::new(),
            target: String::new(),
            direction: Direction::Outbound,
            capabilities: publisher.capabilities(),
            transport: None,
            context: Default::default(),
        })
        .await
        .expect("create test offer");
    let sdp = publisher
        .local_sdp(&handle.connection.id)
        .expect("test offer SDP");
    (publisher, sdp)
}

async fn whep_offer() -> String {
    let player = RvoipPeerConnection::new(&WebRtcConfig::loopback(), PeerRole::Offerer)
        .await
        .expect("WHEP player");
    player
        .prepare_receive_only_offer()
        .await
        .expect("receive-only WHEP media");
    player
        .create_offer_and_gather()
        .await
        .expect("WHEP player offer")
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("HTTP client")
}

fn whip_url(server: &WebRtcServer, tag: &str) -> String {
    format!(
        "http://{}/whip/{tag}",
        server.whip_addr().expect("WHIP address")
    )
}

fn whep_url(server: &WebRtcServer, tag: &str) -> String {
    format!(
        "http://{}/whep/{tag}",
        server.whip_addr().expect("WHEP address")
    )
}

async fn post_offer(client: reqwest::Client, url: String, offer: String) -> reqwest::Response {
    client
        .post(url)
        .header("authorization", "Bearer ignored-by-test-hook")
        .header("content-type", "application/sdp")
        .body(offer)
        .send()
        .await
        .expect("WHIP POST")
}

#[tokio::test]
async fn whip_does_not_return_201_before_core_accepts() {
    let server = secure_server(Duration::from_secs(2), 8).await;
    assert!(server.adapter().supports_inbound_admission_confirmation());
    let orchestrator = Orchestrator::new(Config::default());
    let mut admissions = orchestrator
        .install_inbound_admission_gate(2, Duration::from_secs(2))
        .expect("install gate before adapter");
    orchestrator
        .register(server.adapter() as Arc<dyn ConnectionAdapter>)
        .expect("register adapter");

    let (_publisher, offer) = offer().await;
    let request = tokio::spawn(post_offer(http(), whip_url(&server, "accept"), offer));
    let admission = tokio::time::timeout(Duration::from_secs(5), admissions.recv())
        .await
        .expect("admission timeout")
        .expect("admission ticket");
    tokio::time::sleep(Duration::from_millis(75)).await;
    assert!(
        !request.is_finished(),
        "WHIP must not expose a 201 while policy admission is unresolved"
    );

    admission.accept().await.expect("accept admission");
    let response = request.await.expect("request task");
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    assert!(response.text().await.expect("answer").contains("m=audio"));
    server.shutdown().await;
}

#[cfg(feature = "signaling-ws")]
#[tokio::test]
async fn websocket_does_not_send_answer_before_core_accepts() {
    let server = WebRtcServerBuilder::new(WebRtcConfig::loopback())
        .with_inbound_admission_confirmation(Duration::from_secs(2))
        .with_ws_auth(Arc::new(CompleteAuth))
        .with_ws("127.0.0.1:0")
        .build()
        .await
        .expect("secure WebSocket server");
    let orchestrator = Orchestrator::new(Config::default());
    let mut admissions = orchestrator
        .install_inbound_admission_gate(1, Duration::from_secs(2))
        .expect("install gate");
    orchestrator
        .register(server.adapter() as Arc<dyn ConnectionAdapter>)
        .expect("register adapter");
    let (_publisher, offer) = offer().await;

    let (mut socket, _) = tokio_tungstenite::connect_async(format!(
        "ws://{}/?access_token=test",
        server.ws_addr().expect("WS address")
    ))
    .await
    .expect("WebSocket connect");
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::to_string(&SignalingMessage {
                msg_type: "offer".into(),
                sdp: offer,
                connection_id: String::new(),
                candidate: String::new(),
                request_id: String::new(),
            })
            .expect("offer JSON")
            .into(),
        ))
        .await
        .expect("send offer");
    let admission = admissions.recv().await.expect("admission");
    assert!(
        tokio::time::timeout(Duration::from_millis(75), socket.next())
            .await
            .is_err(),
        "WebSocket answer escaped before admission"
    );

    admission.accept().await.expect("accept admission");
    let answer = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("answer timeout")
        .expect("answer frame")
        .expect("valid answer frame");
    let answer: SignalingMessage =
        serde_json::from_str(answer.to_text().expect("text answer")).expect("answer JSON");
    assert_eq!(answer.msg_type, "answer");
    assert!(!answer.connection_id.is_empty());
    server.shutdown().await;
}

#[tokio::test]
async fn explicit_reject_is_redacted_and_releases_capacity() {
    let server = secure_server(Duration::from_secs(2), 1).await;
    let adapter = server.adapter();
    let orchestrator = Orchestrator::new(Config::default());
    let mut admissions = orchestrator
        .install_inbound_admission_gate(1, Duration::from_secs(2))
        .expect("install gate");
    orchestrator
        .register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register adapter");

    let (_publisher, offer) = offer().await;
    let request = tokio::spawn(post_offer(http(), whip_url(&server, "reject"), offer));
    let admission = admissions.recv().await.expect("admission");
    admission
        .reject(RejectReason::Forbidden)
        .await
        .expect("reject admission");
    let response = request.await.expect("request task");
    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
    assert_eq!(
        response.text().await.expect("body"),
        "inbound signaling was not admitted"
    );
    assert_eq!(adapter.metrics().active_sessions, 0);
    server.shutdown().await;
}

#[tokio::test]
async fn missing_gate_times_out_without_201_and_capacity_is_reusable() {
    let server = secure_server(Duration::from_millis(100), 1).await;
    let adapter = server.adapter();
    let orchestrator = Orchestrator::new(Config::default());
    orchestrator
        .register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register without gate");
    let mut events = orchestrator.subscribe_events();
    let (_publisher, offer) = offer().await;

    for tag in ["missing-gate-a", "missing-gate-b"] {
        let started = Instant::now();
        let response = post_offer(http(), whip_url(&server, tag), offer.clone()).await;
        assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
        assert!(
            started.elapsed() >= Duration::from_millis(75),
            "listener returned before its configured confirmation wait"
        );
        assert_eq!(adapter.metrics().active_sessions, 0);
        let connection_id = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(Event::ConnectionInbound { connection_id, .. }) = events.recv().await {
                    break connection_id;
                }
            }
        })
        .await
        .expect("legacy publication was observable before fail-closed timeout");
        tokio::time::timeout(Duration::from_secs(2), async {
            while orchestrator.connection_transport(&connection_id).is_ok() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("timeout cleanup removed core ownership");
    }
    server.shutdown().await;
}

#[tokio::test]
async fn local_route_end_cancels_pending_protocol_response() {
    let server = secure_server(Duration::from_secs(2), 1).await;
    let adapter = server.adapter();
    let orchestrator = Orchestrator::new(Config::default());
    let mut admissions = orchestrator
        .install_inbound_admission_gate(1, Duration::from_secs(2))
        .expect("install gate");
    orchestrator
        .register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register adapter");
    let (_publisher, offer) = offer().await;

    let request = tokio::spawn(post_offer(http(), whip_url(&server, "local-end"), offer));
    let admission = admissions.recv().await.expect("admission");
    let connection_id = admission.connection_id().clone();
    adapter
        .end(connection_id, EndReason::Cancelled)
        .await
        .expect("local end");
    assert!(admission.accept().await.is_err());
    let response = request.await.expect("request task");
    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
    assert_eq!(adapter.metrics().active_sessions, 0);
    server.shutdown().await;
}

#[tokio::test]
async fn accept_terminal_race_never_leaves_a_successful_orphan() {
    let server = secure_server(Duration::from_secs(2), 2).await;
    let adapter = server.adapter();
    let orchestrator = Orchestrator::new(Config::default());
    let mut admissions = orchestrator
        .install_inbound_admission_gate(2, Duration::from_secs(2))
        .expect("install gate");
    orchestrator
        .register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register adapter");
    let (_publisher, offer) = offer().await;

    for iteration in 0..8 {
        let request = tokio::spawn(post_offer(
            http(),
            whip_url(&server, &format!("race-{iteration}")),
            offer.clone(),
        ));
        let admission = admissions.recv().await.expect("admission");
        let connection_id = admission.connection_id().clone();
        let end = async {
            tokio::task::yield_now().await;
            adapter
                .end(connection_id.clone(), EndReason::Cancelled)
                .await
        };
        let (_accepted, ended) = tokio::join!(admission.accept(), end);
        ended.expect("terminal cleanup");
        let response = request.await.expect("request task");
        assert!(matches!(
            response.status(),
            reqwest::StatusCode::CREATED | reqwest::StatusCode::FORBIDDEN
        ));

        let forgotten = tokio::time::timeout(Duration::from_secs(2), async {
            while orchestrator.connection_transport(&connection_id).is_ok() {
                tokio::task::yield_now().await;
            }
        })
        .await;
        assert!(
            forgotten.is_ok(),
            "terminal race left core ownership behind"
        );
        assert!(!adapter.routes().contains_key(&connection_id));
    }
    server.shutdown().await;
}

#[tokio::test]
async fn canonical_whep_waits_for_principal_bound_core_admission() {
    let server = secure_server(Duration::from_secs(2), 1).await;
    let adapter = server.adapter();
    let orchestrator = Orchestrator::new(Config::default());
    let mut admissions = orchestrator
        .install_inbound_admission_gate(1, Duration::from_secs(2))
        .expect("install WHEP admission gate");
    orchestrator
        .register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register adapter");
    let request = tokio::spawn(
        http()
            .post(format!(
                "http://{}/whep/attachment-proof",
                server.whip_addr().expect("WHIP address")
            ))
            .header("authorization", "Bearer ignored-by-test-hook")
            .header("content-type", "application/sdp")
            .body(whep_offer().await)
            .send(),
    );
    let mut admission = admissions.recv().await.expect("WHEP admission");
    let principal = admission
        .authenticated_principal()
        .expect("principal is retained before admission");
    assert_eq!(principal.tenant.as_deref(), Some("tenant-a"));
    let context = admission
        .take_inbound_context()
        .expect("take attachment context")
        .expect("WHEP attachment context");
    assert_eq!(
        context
            .routing_hint()
            .expect("attachment token")
            .expose_secret(),
        "attachment-proof"
    );
    assert!(admission
        .take_inbound_context()
        .expect("second context read")
        .is_none());
    tokio::time::sleep(Duration::from_millis(75)).await;
    assert!(!request.is_finished(), "WHEP 201 escaped before admission");

    admission.accept().await.expect("accept WHEP admission");
    let response = request
        .await
        .expect("WHEP request task")
        .expect("WHEP POST");
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    assert!(response
        .text()
        .await
        .expect("WHEP answer")
        .contains("a=sendonly"));
    server.shutdown().await;
}

#[tokio::test]
async fn concurrent_replay_is_exposed_as_two_exact_admissions_for_atomic_policy() {
    let server = secure_server(Duration::from_secs(2), 2).await;
    let adapter = server.adapter();
    let orchestrator = Orchestrator::new(Config::default());
    let mut admissions = orchestrator
        .install_inbound_admission_gate(2, Duration::from_secs(2))
        .expect("install WHEP admission gate");
    orchestrator
        .register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register adapter");
    let offer = whep_offer().await;
    let post = |offer: String| {
        http()
            .post(whep_url(&server, "single-use-token"))
            .header("authorization", "Bearer ignored-by-test-hook")
            .header("content-type", "application/sdp")
            .body(offer)
            .send()
    };
    let first_request = tokio::spawn(post(offer.clone()));
    let second_request = tokio::spawn(post(offer));
    let mut first = admissions.recv().await.expect("first WHEP admission");
    let mut second = admissions.recv().await.expect("second WHEP admission");
    assert_ne!(first.connection_id(), second.connection_id());
    for admission in [&mut first, &mut second] {
        assert_eq!(
            admission
                .authenticated_principal()
                .expect("retained principal")
                .tenant
                .as_deref(),
            Some("tenant-a")
        );
        assert_eq!(
            admission
                .take_inbound_context()
                .expect("take replay context")
                .expect("replay context")
                .routing_hint()
                .expect("replayed attachment token")
                .expose_secret(),
            "single-use-token"
        );
    }
    let accepted_id = first.connection_id().clone();
    first.accept().await.expect("atomic first-use acceptance");
    second
        .reject(RejectReason::Forbidden)
        .await
        .expect("atomic replay rejection");

    let first_response = first_request
        .await
        .expect("first request task")
        .expect("first response");
    let second_response = second_request
        .await
        .expect("second request task")
        .expect("second response");
    let statuses = [first_response.status(), second_response.status()];
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == reqwest::StatusCode::CREATED)
            .count(),
        1
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == reqwest::StatusCode::FORBIDDEN)
            .count(),
        1
    );
    assert_eq!(adapter.metrics().active_sessions, 1);
    assert_eq!(adapter.metrics().active_http_resources, 1);
    adapter
        .end(accepted_id, EndReason::Cancelled)
        .await
        .expect("end accepted route");
    assert_eq!(adapter.metrics().active_sessions, 0);
    assert_eq!(adapter.metrics().active_http_resources, 0);
    server.shutdown().await;
}

#[tokio::test]
async fn direct_legacy_adapter_remains_compatible_without_confirmation() {
    let (_publisher, offer) = offer().await;
    let adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let connection_id = adapter
        .apply_remote_offer(&offer)
        .await
        .expect("legacy direct offer");
    assert!(adapter
        .local_sdp(&connection_id)
        .expect("legacy direct answer")
        .contains("m=audio"));
    assert!(!adapter.supports_inbound_admission_confirmation());
}

#[tokio::test]
async fn builder_rejects_invalid_secure_timeout_before_starting_listeners() {
    let result = WebRtcServerBuilder::new(WebRtcConfig::loopback())
        .with_inbound_admission_confirmation(Duration::ZERO)
        .with_whip_auth(Arc::new(CompleteAuth))
        .with_whip("127.0.0.1:0")
        .build()
        .await;
    assert!(result.is_err());
}
