//! Gate 7 target-contacting WebSocket origination.

#![cfg(feature = "signaling-ws")]

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use rvoip_core::adapter::{
    AdapterEvent, ConnectionAdapter, EndReason, OriginateRequest, RejectReason,
};
use rvoip_core::config::Config as CoreConfig;
use rvoip_core::connection::Direction;
use rvoip_core::identity::{
    AuthenticatedPrincipal, AuthenticationMethod, CredentialKind, IdentityAssurance,
};
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId};
use rvoip_core::{DataMessage, DataReliability, Orchestrator, StagedInboundDataPolicy};
use rvoip_webrtc::signaling::auth::{AnonymousAuth, AuthContext, AuthRejection, WsAuthHook};
use rvoip_webrtc::signaling::websocket::{serve_listener_with_auth, SignalingMessage};
use rvoip_webrtc::{
    StaticWebRtcBearerCredentialProvider, WebRtcAdapter, WebRtcBearerCredential, WebRtcConfig,
    WebRtcOriginateContext, WebRtcTargetPolicy,
};
use tokio_tungstenite::accept_hdr_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::http::{header, HeaderValue};
use tokio_tungstenite::tungstenite::Message;

#[derive(Clone)]
struct DelayedBearerAuth {
    calls: Arc<AtomicUsize>,
    delay: Duration,
}

impl DelayedBearerAuth {
    fn new(delay: Duration) -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
            delay,
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::Acquire)
    }
}

#[async_trait]
impl WsAuthHook for DelayedBearerAuth {
    async fn authenticate(
        &self,
        subprotocols: &[String],
        _query_token: Option<&str>,
        _peer_addr: SocketAddr,
    ) -> std::result::Result<AuthContext, AuthRejection> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        if !subprotocols.iter().any(|value| value == "rvoip.webrtc.v1")
            || !subprotocols.iter().any(|value| value == "token.secret")
        {
            return Err(AuthRejection::Unauthorized {
                www_authenticate: "Bearer realm=\"test\"".into(),
            });
        }
        // Keep the first activation waiter pending long enough to abort it.
        // The adapter-owned activation driver must continue independently.
        tokio::time::sleep(self.delay).await;
        Ok(AuthContext {
            subject: "outbound-loopback".into(),
            scopes: vec!["webrtc:answer".into()],
            session_hint: None,
            principal: None,
        })
    }
}

struct AdmissionBearerAuth;

#[async_trait]
impl WsAuthHook for AdmissionBearerAuth {
    async fn authenticate(
        &self,
        subprotocols: &[String],
        _query_token: Option<&str>,
        _peer_addr: SocketAddr,
    ) -> std::result::Result<AuthContext, AuthRejection> {
        if !subprotocols.iter().any(|value| value == "rvoip.webrtc.v1")
            || !subprotocols.iter().any(|value| value == "token.secret")
        {
            return Err(AuthRejection::Unauthorized {
                www_authenticate: "Bearer realm=\"test\"".into(),
            });
        }
        Ok(AuthContext {
            subject: "outbound-admission".into(),
            scopes: vec!["webrtc:answer".into()],
            session_hint: Some("attachment-proof".into()),
            principal: Some(AuthenticatedPrincipal {
                subject: "outbound-admission".into(),
                tenant: Some("tenant-a".into()),
                scopes: vec!["webrtc:answer".into()],
                issuer: Some("https://issuer.example".into()),
                expires_at: None,
                method: AuthenticationMethod::Jwt,
                assurance: IdentityAssurance::Identified {
                    credential_kind: CredentialKind::Oidc,
                },
            }),
        })
    }
}

struct ExpiringAdmissionBearerAuth {
    expires_after: chrono::Duration,
}

#[async_trait]
impl WsAuthHook for ExpiringAdmissionBearerAuth {
    async fn authenticate(
        &self,
        subprotocols: &[String],
        _query_token: Option<&str>,
        _peer_addr: SocketAddr,
    ) -> std::result::Result<AuthContext, AuthRejection> {
        if !subprotocols.iter().any(|value| value == "rvoip.webrtc.v1")
            || !subprotocols.iter().any(|value| value == "token.secret")
        {
            return Err(AuthRejection::Unauthorized {
                www_authenticate: "Bearer realm=\"test\"".into(),
            });
        }
        Ok(AuthContext {
            subject: "expiring-admission".into(),
            scopes: vec!["webrtc:answer".into()],
            session_hint: Some("expiring-attachment-proof".into()),
            principal: Some(AuthenticatedPrincipal {
                subject: "expiring-admission".into(),
                tenant: Some("tenant-a".into()),
                scopes: vec!["webrtc:answer".into()],
                issuer: Some("https://issuer.example".into()),
                expires_at: Some(chrono::Utc::now() + self.expires_after),
                method: AuthenticationMethod::Jwt,
                assurance: IdentityAssurance::Identified {
                    credential_kind: CredentialKind::Oidc,
                },
            }),
        })
    }
}

async fn prepare_required_admission_client(
    address: SocketAddr,
) -> (Arc<WebRtcAdapter>, ConnectionId) {
    let adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let endpoint = format!("ws://{address}/signal");
    let context = WebRtcOriginateContext::websocket(
        &endpoint,
        WebRtcTargetPolicy::default()
            .allow_port(address.port())
            .allow_insecure(true)
            .allow_loopback(true)
            .with_timeouts(Duration::from_secs(3), Duration::from_secs(10))
            .expect("bounded timeouts"),
    )
    .expect("WebSocket context")
    .with_bearer_provider(Arc::new(StaticWebRtcBearerCredentialProvider::new(
        WebRtcBearerCredential::new("secret").expect("bearer"),
    )))
    .require_remote_admission_ready()
    .expect("remote admission policy");
    let connection = adapter
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
        .id;
    (adapter, connection)
}

async fn wait_for_admission_route_cleanup(adapter: &WebRtcAdapter, connection_id: &ConnectionId) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let metrics = adapter.metrics();
            if !adapter.is_connection_live(connection_id)
                && metrics.active_sessions == 0
                && metrics.inbound_admission_tasks == 0
                && metrics.peer_session_tasks == 0
                && metrics.media_tasks == 0
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("provisional admission route/task cleanup");
}

#[tokio::test]
async fn preparation_is_network_free_and_cancelled_waiter_does_not_cancel_session() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut server_config = WebRtcConfig::loopback();
    server_config.trickle_ice = true;
    let server_adapter = WebRtcAdapter::new(server_config);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind WS loopback");
    let address = listener.local_addr().expect("listener address");
    let auth = DelayedBearerAuth::new(Duration::from_millis(100));
    let server_task = {
        let adapter = Arc::clone(&server_adapter);
        let auth = auth.clone();
        tokio::spawn(async move {
            serve_listener_with_auth(listener, adapter, Arc::new(auth))
                .await
                .expect("serve loopback")
        })
    };
    let mut server_events = server_adapter.subscribe_events();

    // The adapter-level config deliberately differs. The typed context owns
    // this exchange's trickle policy and must override it per route.
    let mut client_config = WebRtcConfig::loopback();
    client_config.trickle_ice = false;
    let client_adapter = WebRtcAdapter::new(client_config);
    let endpoint = format!("ws://{address}/signal");
    let policy = WebRtcTargetPolicy::default()
        .allow_port(address.port())
        .allow_insecure(true)
        .allow_loopback(true)
        .with_timeouts(Duration::from_secs(3), Duration::from_secs(10))
        .expect("bounded timeouts");
    let provider = Arc::new(StaticWebRtcBearerCredentialProvider::new(
        WebRtcBearerCredential::new("secret").expect("bearer"),
    ));
    let context = WebRtcOriginateContext::websocket(&endpoint, policy)
        .expect("validated context")
        .with_bearer_provider(provider);
    let request = OriginateRequest::new(
        SessionId::new(),
        ParticipantId::new(),
        endpoint,
        Direction::Outbound,
        client_adapter.capabilities(),
    )
    .with_context(context);

    let handle = client_adapter
        .originate(request)
        .await
        .expect("prepare outbound route");
    let client_connection = handle.connection.id.clone();
    assert!(
        tokio::time::timeout(Duration::from_millis(50), server_events.recv())
            .await
            .is_err(),
        "originate preparation must not contact the target"
    );

    let first_waiter = {
        let adapter = Arc::clone(&client_adapter);
        let connection = client_connection.clone();
        tokio::spawn(async move { adapter.activate_outbound(connection).await })
    };
    tokio::time::sleep(Duration::from_millis(20)).await;
    first_waiter.abort();

    // A second waiter observes the one retained driver; it must not open a
    // second logical route or restart authentication after the first waiter
    // is cancelled.
    client_adapter
        .activate_outbound(client_connection.clone())
        .await
        .expect("retained activation succeeds");
    assert_eq!(
        auth.calls(),
        1,
        "single-flight activation authenticates once"
    );

    let server_connection = wait_for_inbound_connection(&mut server_events).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(100), async {
            loop {
                if let Some(AdapterEvent::InboundConnection { .. }) = server_events.recv().await {
                    return;
                }
            }
        })
        .await
        .is_err(),
        "single-flight activation must create exactly one remote route"
    );

    let (client_accept, server_accept) = tokio::join!(
        client_adapter.accept(client_connection.clone()),
        server_adapter.accept(server_connection.clone()),
    );
    client_accept.expect("client ICE/DTLS connected");
    server_accept.expect("server ICE/DTLS connected");

    client_adapter
        .end(client_connection.clone(), EndReason::Normal)
        .await
        .expect("local end sends one BYE");
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if !server_adapter.is_connection_live(&server_connection) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("remote BYE cleanup");
    assert!(!client_adapter.is_connection_live(&client_connection));

    server_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn same_security_partition_multiplexes_routes_and_tears_them_down_independently() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut server_config = WebRtcConfig::loopback();
    server_config.trickle_ice = true;
    let server_adapter = WebRtcAdapter::new(server_config);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind WS loopback");
    let address = listener.local_addr().expect("listener address");
    let auth = DelayedBearerAuth::new(Duration::from_millis(25));
    let server_task = {
        let adapter = Arc::clone(&server_adapter);
        let auth = auth.clone();
        tokio::spawn(async move {
            serve_listener_with_auth(listener, adapter, Arc::new(auth))
                .await
                .expect("serve loopback")
        })
    };
    let mut server_events = server_adapter.subscribe_events();

    let client_adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let endpoint = format!("ws://{address}/signal");
    let policy = WebRtcTargetPolicy::default()
        .allow_port(address.port())
        .allow_insecure(true)
        .allow_loopback(true)
        .with_credential_partition("tenant-a")
        .expect("credential partition")
        .with_timeouts(Duration::from_secs(3), Duration::from_secs(10))
        .expect("bounded timeouts");
    let provider = Arc::new(StaticWebRtcBearerCredentialProvider::new(
        WebRtcBearerCredential::new("secret").expect("bearer"),
    ));
    let context = WebRtcOriginateContext::websocket(&endpoint, policy)
        .expect("validated context")
        .with_bearer_provider(provider);

    let first = client_adapter
        .originate(
            OriginateRequest::new(
                SessionId::new(),
                ParticipantId::new(),
                endpoint.clone(),
                Direction::Outbound,
                client_adapter.capabilities(),
            )
            .with_context(context.clone()),
        )
        .await
        .expect("prepare first route")
        .connection
        .id;
    let second = client_adapter
        .originate(
            OriginateRequest::new(
                SessionId::new(),
                ParticipantId::new(),
                endpoint,
                Direction::Outbound,
                client_adapter.capabilities(),
            )
            .with_context(context),
        )
        .await
        .expect("prepare second route")
        .connection
        .id;

    let (first_activation, second_activation) = tokio::join!(
        client_adapter.activate_outbound(first.clone()),
        client_adapter.activate_outbound(second.clone()),
    );
    first_activation.expect("activate first multiplexed route");
    second_activation.expect("activate second multiplexed route");
    assert_eq!(
        auth.calls(),
        1,
        "one credential-partition hub must perform one WebSocket upgrade"
    );

    let first_server = wait_for_inbound_connection(&mut server_events).await;
    let second_server = wait_for_inbound_connection(&mut server_events).await;
    assert_ne!(first_server, second_server);

    let (first_accept, second_accept, first_server_accept, second_server_accept) = tokio::join!(
        client_adapter.accept(first.clone()),
        client_adapter.accept(second.clone()),
        server_adapter.accept(first_server.clone()),
        server_adapter.accept(second_server.clone()),
    );
    first_accept.expect("first client ICE/DTLS connected");
    second_accept.expect("second client ICE/DTLS connected");
    first_server_accept.expect("first server ICE/DTLS connected");
    second_server_accept.expect("second server ICE/DTLS connected");

    client_adapter
        .end(first.clone(), EndReason::Normal)
        .await
        .expect("end first route");
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if !server_adapter.is_connection_live(&first_server)
                || !server_adapter.is_connection_live(&second_server)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("first remote route cleanup");
    assert!(
        server_adapter.is_connection_live(&first_server)
            ^ server_adapter.is_connection_live(&second_server),
        "one and only one server route survives the first BYE"
    );
    assert!(
        client_adapter.is_connection_live(&second),
        "ending one logical route must not close its sibling"
    );
    assert_eq!(auth.calls(), 1, "sibling route remains on the same socket");

    client_adapter
        .end(second.clone(), EndReason::Normal)
        .await
        .expect("end second route");
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if !server_adapter.is_connection_live(&first_server)
                && !server_adapter.is_connection_live(&second_server)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("second remote route cleanup");
    assert!(!client_adapter.is_connection_live(&first));
    assert!(!client_adapter.is_connection_live(&second));

    server_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn required_remote_admission_keeps_activation_pending_until_application_accepts() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let server_adapter = WebRtcAdapter::new_with_inbound_admission_confirmation(
        WebRtcConfig::loopback(),
        Duration::from_secs(2),
    )
    .expect("confirmed-admission adapter");
    let orchestrator = Orchestrator::new(CoreConfig::default());
    let mut admissions = orchestrator
        .install_inbound_admission_gate(1, Duration::from_secs(2))
        .expect("install inbound gate");
    orchestrator
        .register(Arc::clone(&server_adapter) as Arc<dyn ConnectionAdapter>)
        .expect("register confirmed-admission adapter");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind WS admission target");
    let address = listener.local_addr().expect("listener address");
    let server_task = {
        let adapter = Arc::clone(&server_adapter);
        tokio::spawn(async move {
            serve_listener_with_auth(listener, adapter, Arc::new(AdmissionBearerAuth))
                .await
                .expect("serve admission target")
        })
    };

    let client_adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let mut client_events = client_adapter.subscribe_events();
    let endpoint = format!("ws://{address}/signal");
    let context = WebRtcOriginateContext::websocket(
        &endpoint,
        WebRtcTargetPolicy::default()
            .allow_port(address.port())
            .allow_insecure(true)
            .allow_loopback(true)
            .with_timeouts(Duration::from_secs(3), Duration::from_secs(10))
            .expect("bounded timeouts"),
    )
    .expect("WebSocket context")
    .with_bearer_provider(Arc::new(StaticWebRtcBearerCredentialProvider::new(
        WebRtcBearerCredential::new("secret").expect("bearer"),
    )))
    .with_preopened_data_channel("bridgefu.context.v1", DataReliability::ReliableOrdered)
    .expect("preopen context channel")
    .require_remote_admission_ready()
    .expect("remote admission policy");
    let connection = client_adapter
        .originate(
            OriginateRequest::new(
                SessionId::new(),
                ParticipantId::new(),
                endpoint,
                Direction::Outbound,
                client_adapter.capabilities(),
            )
            .with_context(context),
        )
        .await
        .expect("prepare outbound route")
        .connection
        .id;
    let prepared_channels = client_adapter
        .routes()
        .get(&connection)
        .expect("prepared route")
        .data_channel
        .iter()
        .map(|entry| Arc::clone(entry.value()))
        .collect::<Vec<_>>();
    assert_eq!(
        prepared_channels.len(),
        2,
        "custom channel extends rather than replaces the legacy bootstrap"
    );
    let mut saw_context_channel = false;
    for channel in prepared_channels {
        saw_context_channel |=
            channel.label().await.expect("prepared channel label") == "bridgefu.context.v1";
    }
    assert!(saw_context_channel, "context channel was not preopened");
    let activation = {
        let adapter = Arc::clone(&client_adapter);
        let connection = connection.clone();
        tokio::spawn(async move { adapter.activate_outbound(connection).await })
    };
    let mut admission = tokio::time::timeout(Duration::from_secs(5), admissions.recv())
        .await
        .expect("admission timeout")
        .expect("admission ticket");
    let server_connection = admission.connection_id().clone();
    assert!(
        server_adapter
            .accept(server_connection.clone())
            .await
            .is_err(),
        "adapter accept bypassed pending core admission"
    );
    assert!(
        server_adapter
            .streams(server_connection.clone())
            .await
            .is_err(),
        "media streams escaped pending core admission"
    );
    assert_eq!(
        server_adapter.metrics().media_tasks,
        0,
        "pre-admission DataChannel startup created media tasks"
    );

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let answer_received = client_adapter
                .routes()
                .get(&connection)
                .is_some_and(|route| route.remote_sdp.is_some());
            if answer_received {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("provisional answer arrived before application admission");
    assert!(
        !activation.is_finished(),
        "provisional SDP answer must not imply application admission"
    );

    let (_, mut staged_context) = admission
        .open_staged_data_channel(StagedInboundDataPolicy::new(
            std::iter::empty::<&str>(),
            ["bridgefu.context.v1"],
            4,
        ))
        .expect("open pre-admission context channel")
        .split();

    let context_message = DataMessage::reliable(
        "bridgefu.context.v1",
        "application/vnd.bridgefu.context.v1+json",
        bytes::Bytes::from_static(br#"{"version":1}"#),
    );
    let early_send = {
        let adapter = Arc::clone(&client_adapter);
        let connection = connection.clone();
        let message = context_message.clone();
        tokio::spawn(async move { adapter.send_data_message(connection, message).await })
    };
    tokio::time::timeout(Duration::from_secs(5), early_send)
        .await
        .expect("pre-admission DataChannel send timeout")
        .expect("pre-admission send task")
        .expect("pre-admission DataChannel send");
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), staged_context.recv())
            .await
            .expect("staged context receive timeout")
            .expect("staged context channel closed"),
        context_message,
        "preopened context did not reach the pending core admission"
    );

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !activation.is_finished(),
        "SDP answer and ICE/DTLS must not imply application admission"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(50), client_events.recv())
            .await
            .is_err(),
        "Connected escaped before the remote application accepted"
    );

    admission.accept().await.expect("core admission accept");
    assert!(
        !activation.is_finished(),
        "core policy acceptance alone must not bypass adapter accept"
    );
    server_adapter
        .accept(server_connection.clone())
        .await
        .expect("remote application accept");
    tokio::time::timeout(Duration::from_secs(5), activation)
        .await
        .expect("admitted activation timeout")
        .expect("activation task")
        .expect("admitted activation");
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), client_events.recv())
            .await
            .expect("Connected timeout"),
        Some(AdapterEvent::Connected { connection_id }) if connection_id == connection
    ));

    client_adapter
        .end(connection, EndReason::Normal)
        .await
        .expect("end admitted route");
    tokio::time::timeout(Duration::from_secs(2), async {
        while server_adapter.is_connection_live(&server_connection)
            || server_adapter.metrics().inbound_admission_tasks != 0
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("accepted route cleanup");
    server_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn immediate_core_accept_is_retained_until_ready_and_connected() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let server_adapter = WebRtcAdapter::new_with_inbound_admission_confirmation(
        WebRtcConfig::loopback(),
        Duration::from_secs(2),
    )
    .expect("confirmed-admission adapter");
    let orchestrator = Orchestrator::new(CoreConfig::default());
    let mut admissions = orchestrator
        .install_inbound_admission_gate(1, Duration::from_secs(2))
        .expect("install inbound gate");
    orchestrator
        .register(Arc::clone(&server_adapter) as Arc<dyn ConnectionAdapter>)
        .expect("register confirmed-admission adapter");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind WS immediate-accept target");
    let address = listener.local_addr().expect("listener address");
    let server_task = {
        let adapter = Arc::clone(&server_adapter);
        tokio::spawn(async move {
            serve_listener_with_auth(listener, adapter, Arc::new(AdmissionBearerAuth))
                .await
                .expect("serve immediate-accept target")
        })
    };

    let (client_adapter, connection) = prepare_required_admission_client(address).await;
    let mut client_events = client_adapter.subscribe_events();
    let activation = {
        let adapter = Arc::clone(&client_adapter);
        let connection = connection.clone();
        tokio::spawn(async move { adapter.activate_outbound(connection).await })
    };
    let admission = tokio::time::timeout(Duration::from_secs(5), admissions.recv())
        .await
        .expect("immediate admission timeout")
        .expect("immediate admission ticket");
    let server_connection = admission.connection_id().clone();
    admission.accept().await.expect("immediate core accept");
    server_adapter
        .accept(server_connection.clone())
        .await
        .expect("immediate adapter accept");

    tokio::time::timeout(Duration::from_secs(5), activation)
        .await
        .expect("immediate accepted activation timeout")
        .expect("immediate accepted activation task")
        .expect("immediate accepted activation");
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), client_events.recv())
            .await
            .expect("immediate Connected timeout"),
        Some(AdapterEvent::Connected { connection_id }) if connection_id == connection
    ));

    client_adapter
        .end(connection, EndReason::Normal)
        .await
        .expect("end immediately accepted route");
    wait_for_admission_route_cleanup(&server_adapter, &server_connection).await;
    server_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn immediate_core_reject_fails_activation_and_releases_exact_route() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let server_adapter = WebRtcAdapter::new_with_inbound_admission_confirmation(
        WebRtcConfig::loopback(),
        Duration::from_secs(2),
    )
    .expect("confirmed-admission adapter");
    let orchestrator = Orchestrator::new(CoreConfig::default());
    let mut admissions = orchestrator
        .install_inbound_admission_gate(1, Duration::from_secs(2))
        .expect("install inbound gate");
    orchestrator
        .register(Arc::clone(&server_adapter) as Arc<dyn ConnectionAdapter>)
        .expect("register confirmed-admission adapter");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind WS immediate-reject target");
    let address = listener.local_addr().expect("listener address");
    let server_task = {
        let adapter = Arc::clone(&server_adapter);
        tokio::spawn(async move {
            serve_listener_with_auth(listener, adapter, Arc::new(AdmissionBearerAuth))
                .await
                .expect("serve immediate-reject target")
        })
    };

    let (client_adapter, connection) = prepare_required_admission_client(address).await;
    let mut client_events = client_adapter.subscribe_events();
    let activation = {
        let adapter = Arc::clone(&client_adapter);
        let connection = connection.clone();
        tokio::spawn(async move { adapter.activate_outbound(connection).await })
    };
    let admission = tokio::time::timeout(Duration::from_secs(5), admissions.recv())
        .await
        .expect("immediate rejection timeout")
        .expect("immediate rejection ticket");
    let server_connection = admission.connection_id().clone();
    admission
        .reject(RejectReason::Forbidden)
        .await
        .expect("immediate core reject");

    assert!(
        tokio::time::timeout(Duration::from_secs(5), activation)
            .await
            .expect("rejected activation timeout")
            .expect("activation task")
            .is_err(),
        "remote rejection must fail outbound activation"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), client_events.recv())
            .await
            .is_err(),
        "rejected route published a lifecycle event before commit"
    );
    wait_for_admission_route_cleanup(&server_adapter, &server_connection).await;

    client_adapter
        .end(connection, EndReason::Normal)
        .await
        .expect("cleanup rejected route");
    server_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn core_admission_timeout_rejects_activation_and_releases_all_tasks() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let server_adapter = WebRtcAdapter::new_with_inbound_admission_confirmation(
        WebRtcConfig::loopback(),
        Duration::from_secs(1),
    )
    .expect("confirmed-admission adapter");
    let orchestrator = Orchestrator::new(CoreConfig::default());
    let mut admissions = orchestrator
        .install_inbound_admission_gate(1, Duration::from_secs(3))
        .expect("install inbound gate");
    orchestrator
        .register(Arc::clone(&server_adapter) as Arc<dyn ConnectionAdapter>)
        .expect("register confirmed-admission adapter");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind WS admission-timeout target");
    let address = listener.local_addr().expect("listener address");
    let server_task = {
        let adapter = Arc::clone(&server_adapter);
        tokio::spawn(async move {
            serve_listener_with_auth(listener, adapter, Arc::new(AdmissionBearerAuth))
                .await
                .expect("serve admission-timeout target")
        })
    };

    let (client_adapter, connection) = prepare_required_admission_client(address).await;
    let mut client_events = client_adapter.subscribe_events();
    let activation = {
        let adapter = Arc::clone(&client_adapter);
        let connection = connection.clone();
        tokio::spawn(async move { adapter.activate_outbound(connection).await })
    };
    let admission = tokio::time::timeout(Duration::from_secs(5), admissions.recv())
        .await
        .expect("pending admission timeout")
        .expect("pending admission ticket");
    let server_connection = admission.connection_id().clone();

    tokio::time::timeout(Duration::from_millis(750), async {
        loop {
            if client_adapter
                .routes()
                .get(&connection)
                .is_some_and(|route| route.remote_sdp.is_some())
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("provisional answer before admission timeout");
    assert!(
        !activation.is_finished(),
        "provisional answer bypassed the unresolved core admission"
    );

    assert!(
        tokio::time::timeout(Duration::from_secs(5), activation)
            .await
            .expect("timed-out activation deadline")
            .expect("timed-out activation task")
            .is_err(),
        "unresolved core admission must fail activation"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), client_events.recv())
            .await
            .is_err(),
        "timed-out admission published Connected"
    );
    wait_for_admission_route_cleanup(&server_adapter, &server_connection).await;
    assert!(
        admission.accept().await.is_err(),
        "expired admission ticket remained authoritative"
    );

    client_adapter
        .end(connection, EndReason::Normal)
        .await
        .expect("cleanup timed-out client route");
    server_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn leased_socket_close_cancels_pending_admission_and_exact_route() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let server_adapter = WebRtcAdapter::new_with_inbound_admission_confirmation(
        WebRtcConfig::loopback(),
        Duration::from_secs(3),
    )
    .expect("confirmed-admission adapter");
    let orchestrator = Orchestrator::new(CoreConfig::default());
    let mut admissions = orchestrator
        .install_inbound_admission_gate(1, Duration::from_secs(3))
        .expect("install inbound gate");
    orchestrator
        .register(Arc::clone(&server_adapter) as Arc<dyn ConnectionAdapter>)
        .expect("register confirmed-admission adapter");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind WS close target");
    let address = listener.local_addr().expect("listener address");
    let server_task = {
        let adapter = Arc::clone(&server_adapter);
        tokio::spawn(async move {
            serve_listener_with_auth(listener, adapter, Arc::new(AdmissionBearerAuth))
                .await
                .expect("serve close target")
        })
    };

    let (client_adapter, connection) = prepare_required_admission_client(address).await;
    let activation = {
        let adapter = Arc::clone(&client_adapter);
        let connection = connection.clone();
        tokio::spawn(async move { adapter.activate_outbound(connection).await })
    };
    let admission = tokio::time::timeout(Duration::from_secs(5), admissions.recv())
        .await
        .expect("socket-close admission timeout")
        .expect("socket-close admission ticket");
    let server_connection = admission.connection_id().clone();
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if client_adapter
                .routes()
                .get(&connection)
                .is_some_and(|route| route.remote_sdp.is_some())
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("provisional answer before socket close");

    client_adapter
        .end(connection.clone(), EndReason::Normal)
        .await
        .expect("close sole leased client route");
    assert!(
        tokio::time::timeout(Duration::from_secs(3), activation)
            .await
            .expect("closed-socket activation deadline")
            .expect("closed-socket activation task")
            .is_err(),
        "closed signaling socket admitted the route"
    );
    wait_for_admission_route_cleanup(&server_adapter, &server_connection).await;
    assert!(!client_adapter.is_connection_live(&connection));
    drop(admission);
    server_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn expired_principal_cannot_block_exact_leased_socket_cleanup() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut server_config = WebRtcConfig::loopback();
    server_config.trickle_ice = true;
    let server_adapter = WebRtcAdapter::new_with_inbound_admission_confirmation(
        server_config,
        Duration::from_secs(6),
    )
    .expect("confirmed-admission adapter");
    let orchestrator = Orchestrator::new(CoreConfig::default());
    let mut admissions = orchestrator
        .install_inbound_admission_gate(1, Duration::from_secs(6))
        .expect("install inbound gate");
    orchestrator
        .register(Arc::clone(&server_adapter) as Arc<dyn ConnectionAdapter>)
        .expect("register confirmed-admission adapter");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind expiring WS target");
    let address = listener.local_addr().expect("listener address");
    let server_task = {
        let adapter = Arc::clone(&server_adapter);
        tokio::spawn(async move {
            serve_listener_with_auth(
                listener,
                adapter,
                Arc::new(ExpiringAdmissionBearerAuth {
                    expires_after: chrono::Duration::seconds(2),
                }),
            )
            .await
            .expect("serve expiring WS target")
        })
    };

    let (client_adapter, connection) = prepare_required_admission_client(address).await;
    let activation = {
        let adapter = Arc::clone(&client_adapter);
        let connection = connection.clone();
        tokio::spawn(async move { adapter.activate_outbound(connection).await })
    };
    let admission = tokio::time::timeout(Duration::from_secs(5), admissions.recv())
        .await
        .expect("expiring admission timeout")
        .expect("expiring admission ticket");
    let server_connection = admission.connection_id().clone();
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if client_adapter
                .routes()
                .get(&connection)
                .is_some_and(|route| route.remote_sdp.is_some())
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("provisional answer before principal expiry");

    tokio::time::sleep(Duration::from_millis(2_100)).await;
    assert!(
        server_adapter
            .authenticated_principal(&server_connection)
            .expect("retained route principal")
            .expect("authenticated route")
            .is_expired(),
        "test principal did not expire before leased cleanup"
    );
    client_adapter
        .end(connection.clone(), EndReason::Normal)
        .await
        .expect("close expired-principal leased route");
    assert!(
        tokio::time::timeout(Duration::from_secs(3), activation)
            .await
            .expect("expired-principal activation deadline")
            .expect("expired-principal activation task")
            .is_err(),
        "expired-principal route was admitted during shutdown"
    );
    wait_for_admission_route_cleanup(&server_adapter, &server_connection).await;
    assert!(!client_adapter.is_connection_live(&connection));
    drop(admission);
    server_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn non_leased_offer_ready_fails_before_route_allocation() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let server_adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind anonymous WS target");
    let address = listener.local_addr().expect("listener address");
    let server_task = {
        let adapter = Arc::clone(&server_adapter);
        tokio::spawn(async move {
            serve_listener_with_auth(listener, adapter, Arc::new(AnonymousAuth))
                .await
                .expect("serve anonymous WS target")
        })
    };

    let offerer = WebRtcAdapter::new(WebRtcConfig::loopback());
    let offerer_connection = offerer
        .originate(OriginateRequest::new(
            SessionId::new(),
            ParticipantId::new(),
            String::new(),
            Direction::Outbound,
            offerer.capabilities(),
        ))
        .await
        .expect("prepare local offer")
        .connection
        .id;
    let offer = offerer
        .local_sdp(&offerer_connection)
        .expect("local SDP offer");
    let (mut socket, response) = tokio_tungstenite::connect_async(format!("ws://{address}/signal"))
        .await
        .expect("connect anonymous non-leased socket");
    assert!(
        response.headers().get("sec-websocket-protocol").is_none(),
        "anonymous socket unexpectedly acquired a route lease"
    );
    socket
        .send(Message::Text(
            serde_json::to_string(&SignalingMessage {
                msg_type: "offer-ready".into(),
                sdp: offer,
                connection_id: String::new(),
                candidate: String::new(),
                request_id: "anonymous-offer-ready".into(),
            })
            .expect("serialize anonymous offer-ready")
            .into(),
        ))
        .await
        .expect("send anonymous offer-ready");

    match tokio::time::timeout(Duration::from_secs(3), socket.next())
        .await
        .expect("anonymous offer-ready socket did not fail closed")
    {
        None | Some(Err(_)) | Some(Ok(Message::Close(_))) => {}
        Some(Ok(frame)) => panic!("non-leased offer-ready received unexpected frame: {frame:?}"),
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let metrics = server_adapter.metrics();
            if server_adapter.routes().is_empty()
                && metrics.active_sessions == 0
                && metrics.inbound_admission_tasks == 0
                && metrics.peer_session_tasks == 0
                && metrics.inbound_ws_connection_tasks == 0
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("anonymous offer-ready retained route/task state");
    assert_eq!(
        server_adapter.metrics().inbound_total,
        0,
        "non-leased offer-ready reached route preparation"
    );

    offerer
        .end(offerer_connection, EndReason::Normal)
        .await
        .expect("cleanup local offerer");
    server_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn anonymous_leased_offer_ready_fails_before_route_allocation() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let server_adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind anonymous leased WS target");
    let address = listener.local_addr().expect("listener address");
    let server_task = {
        let adapter = Arc::clone(&server_adapter);
        tokio::spawn(async move {
            serve_listener_with_auth(listener, adapter, Arc::new(AnonymousAuth))
                .await
                .expect("serve anonymous leased WS target")
        })
    };

    let offerer = WebRtcAdapter::new(WebRtcConfig::loopback());
    let offerer_connection = offerer
        .originate(OriginateRequest::new(
            SessionId::new(),
            ParticipantId::new(),
            String::new(),
            Direction::Outbound,
            offerer.capabilities(),
        ))
        .await
        .expect("prepare local offer")
        .connection
        .id;
    let offer = offerer
        .local_sdp(&offerer_connection)
        .expect("local SDP offer");
    let mut request = format!("ws://{address}/signal")
        .into_client_request()
        .expect("anonymous leased request");
    request.headers_mut().insert(
        header::SEC_WEBSOCKET_PROTOCOL,
        HeaderValue::from_static("rvoip.webrtc.v1"),
    );
    let (mut socket, response) = tokio_tungstenite::connect_async(request)
        .await
        .expect("connect anonymous leased socket");
    assert_eq!(
        response
            .headers()
            .get(header::SEC_WEBSOCKET_PROTOCOL)
            .and_then(|value| value.to_str().ok()),
        Some("rvoip.webrtc.v1"),
        "test socket did not negotiate the leased subprotocol"
    );
    socket
        .send(Message::Text(
            serde_json::to_string(&SignalingMessage {
                msg_type: "offer-ready".into(),
                sdp: offer,
                connection_id: String::new(),
                candidate: String::new(),
                request_id: "anonymous-leased-offer-ready".into(),
            })
            .expect("serialize anonymous leased offer-ready")
            .into(),
        ))
        .await
        .expect("send anonymous leased offer-ready");

    match tokio::time::timeout(Duration::from_secs(3), socket.next())
        .await
        .expect("anonymous leased offer-ready socket did not fail closed")
    {
        None | Some(Err(_)) | Some(Ok(Message::Close(_))) => {}
        Some(Ok(frame)) => {
            panic!("anonymous leased offer-ready received unexpected frame: {frame:?}")
        }
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let metrics = server_adapter.metrics();
            if server_adapter.routes().is_empty()
                && metrics.active_sessions == 0
                && metrics.inbound_admission_tasks == 0
                && metrics.peer_session_tasks == 0
                && metrics.inbound_ws_connection_tasks == 0
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("anonymous leased offer-ready retained route/task state");
    assert_eq!(
        server_adapter.metrics().inbound_total,
        0,
        "anonymous leased offer-ready reached route preparation"
    );

    offerer
        .end(offerer_connection, EndReason::Normal)
        .await
        .expect("cleanup local offerer");
    server_task.abort();
}

#[tokio::test]
async fn unanswered_socket_route_is_aborted_and_closes_its_shared_hub() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind stalled WS origin");
    let address = listener.local_addr().expect("listener address");
    let upgraded = Arc::new(tokio::sync::Notify::new());
    let closed = Arc::new(tokio::sync::Notify::new());
    let server_task = {
        let upgraded = Arc::clone(&upgraded);
        let closed = Arc::clone(&closed);
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept WS client");
            let callback = |_request: &Request, mut response: Response| {
                response.headers_mut().insert(
                    header::SEC_WEBSOCKET_PROTOCOL,
                    HeaderValue::from_static("rvoip.webrtc.v1"),
                );
                Ok(response)
            };
            let mut socket = accept_hdr_async(stream, callback)
                .await
                .expect("upgrade WS client");
            upgraded.notify_one();
            while let Some(frame) = socket.next().await {
                match frame {
                    Ok(Message::Close(_)) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            closed.notify_one();
        })
    };

    let adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let endpoint = format!("ws://{address}/signal");
    let context = WebRtcOriginateContext::websocket(
        &endpoint,
        WebRtcTargetPolicy::default()
            .allow_port(address.port())
            .allow_insecure(true)
            .allow_loopback(true)
            .with_timeouts(Duration::from_secs(3), Duration::from_secs(20))
            .expect("bounded timeouts"),
    )
    .expect("validated WS context");
    let connection = adapter
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
        .expect("prepare stalled WS route")
        .connection
        .id;
    let activation = {
        let adapter = Arc::clone(&adapter);
        let connection = connection.clone();
        tokio::spawn(async move { adapter.activate_outbound(connection).await })
    };
    tokio::time::timeout(Duration::from_secs(5), upgraded.notified())
        .await
        .expect("WS target was contacted");
    assert_eq!(adapter.outbound_signaling_task_count(), 1);

    tokio::time::timeout(
        Duration::from_secs(5),
        adapter.end(connection.clone(), EndReason::Normal),
    )
    .await
    .expect("WS route shutdown exceeded its abort deadline")
    .expect("WS route shutdown");
    assert!(tokio::time::timeout(Duration::from_secs(1), activation)
        .await
        .expect("activation waiter remained blocked after forced shutdown")
        .expect("activation task")
        .is_err());
    tokio::time::timeout(Duration::from_secs(2), closed.notified())
        .await
        .expect("shared WS hub did not close after its last route");
    assert_eq!(adapter.outbound_signaling_task_count(), 0);
    assert!(!adapter.is_connection_live(&connection));

    server_task.await.expect("stalled WS origin task");
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
    .expect("inbound connection event")
}
