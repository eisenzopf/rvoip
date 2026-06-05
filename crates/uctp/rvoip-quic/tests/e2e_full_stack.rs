//! Cross-crate end-to-end integration: **rvoip-core + rvoip-uctp +
//! rvoip-quic + rvoip-vcon**.
//!
//! Exercises the full v1 lifecycle path that no other test covers:
//!
//! 1. **rvoip-quic** bootstraps a real loopback QUIC endpoint.
//! 2. **rvoip-uctp** runs the auth handshake + envelope dispatch
//!    inside the per-peer `UctpCoordinator` the QUIC server spawns.
//! 3. **rvoip-core** owns the live Conversation/Session/Participant
//!    registries (P1) and the vCon auto-emission path (P3).
//! 4. **rvoip-vcon** is exercised side-by-side: a `Vcon` document is
//!    constructed via `VconBuilder` using the same Session
//!    participant data, stored in `rvoip_vcon::MemoryVconStore`, and
//!    round-trips through `get`.
//!
//! What this test proves end-to-end:
//! - Conversation + Session created via the new orchestrator methods.
//! - QUIC peer connects, UCTP auth handshake completes.
//! - `session.invite` over the wire surfaces as `Event::ConnectionInbound`.
//! - `route_inbound_connection(Accept{session_id, participant_id})`
//!   binds the live Connection into the pre-created Session.
//! - `end_session(sid)` triggers `Event::SessionEnded` and the
//!   automatic `Event::VconReady` emission.
//! - The vCon bytes in rvoip-core's `VconStore::get` are sha256-
//!   verifiable against the handle.
//! - A parallel rvoip-vcon document model populated from the same
//!   Session data round-trips cleanly through its own store.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use rvoip_auth_core::bearer_stub;
use rvoip_core::adapter::{ConnectionAdapter, EndReason};
use rvoip_core::commands::InboundAction;
use rvoip_core::conversation::ConversationPolicy;
use rvoip_core::events::Event;
use rvoip_core::ids::{ParticipantId, TenantId};
use rvoip_core::participant::{ParticipantKind, ParticipantRole};
use rvoip_core::session::SessionMedium;
use rvoip_core::{Config, Orchestrator};
use rvoip_quic::{UctpQuicAdapter, UctpQuicClient, UctpQuicConfig};
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::payloads::{auth, session::SessionInvite};
use rvoip_uctp::substrate::{dev_client_config_trusting, dispatch_by_alpn, self_signed_for_dev};
use rvoip_uctp::types::MessageType;
use rvoip_vcon::{MemoryVconStore as IetfMemoryVconStore, Party, VconBuilder, VconStore};

const ALPN_UCTP: &[u8] = b"uctp/1";

fn rand_hex() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!("{:016x}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

fn server_endpoint(
    addr: SocketAddr,
) -> (
    Arc<quinn::Endpoint>,
    rustls::pki_types::CertificateDer<'static>,
) {
    let (cert_der, key_der) = self_signed_for_dev(&["localhost".into()]).expect("self_signed");
    let mut tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der.clone()], key_der)
        .expect("server tls");
    tls.alpn_protocols = vec![ALPN_UCTP.to_vec()];
    let endpoint = rvoip_uctp::substrate::make_server_endpoint(
        addr,
        Arc::new(tls),
        quinn::TransportConfig::default(),
    )
    .expect("endpoint");
    (Arc::new(endpoint), cert_der)
}

fn client_endpoint() -> Arc<quinn::Endpoint> {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind");
    Arc::new(
        quinn::Endpoint::new(
            quinn::EndpointConfig::default(),
            None,
            socket,
            Arc::new(quinn::TokioRuntime),
        )
        .expect("client endpoint"),
    )
}

/// Dial + complete the UCTP auth handshake. Returns the connected
/// client ready to send signaling envelopes.
async fn dial_authenticated(
    client_ep: &quinn::Endpoint,
    server_addr: SocketAddr,
    cert: &rustls::pki_types::CertificateDer<'static>,
) -> (
    Arc<UctpQuicClient>,
    tokio::sync::mpsc::Receiver<UctpEnvelope>,
) {
    let client_cfg = dev_client_config_trusting(cert).expect("client cfg");
    let client = UctpQuicClient::connect(client_ep, server_addr, "localhost", Arc::new(client_cfg))
        .await
        .expect("client connect");

    let mut inbound = client.take_inbound().expect("take_inbound");
    // hello → challenge → response → session
    let hello = UctpEnvelope {
        v: 1,
        msg_type: MessageType::AuthHello,
        id: format!("env_{}", rand_hex()),
        ts: Utc::now(),
        cid: None,
        sid: None,
        connid: None,
        in_reply_to: None,
        payload: serde_json::to_value(auth::AuthHello {
            device: auth::Device {
                id: "dev_e2e".into(),
                kind: "desktop".into(),
                platform: "test-platform".into(),
                sdk_version: "test/0.1".into(),
            },
            auth_methods: vec!["bearer".into()],
            capabilities: serde_json::Value::Object(Default::default()),
        })
        .unwrap(),
        signature: None,
    };
    client.send(hello).await.expect("send hello");
    let challenge = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
        .await
        .expect("auth.challenge timeout")
        .expect("inbound closed");
    assert_eq!(challenge.msg_type, MessageType::AuthChallenge);

    let response = UctpEnvelope {
        v: 1,
        msg_type: MessageType::AuthResponse,
        id: format!("env_{}", rand_hex()),
        ts: Utc::now(),
        cid: None,
        sid: None,
        connid: None,
        in_reply_to: Some(challenge.id),
        payload: serde_json::to_value(auth::AuthResponse {
            method: "bearer".into(),
            credential: "e2e-token".into(),
            actor_token: None,
        })
        .unwrap(),
        signature: None,
    };
    client.send(response).await.expect("send response");
    let session_reply = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
        .await
        .expect("auth.session timeout")
        .expect("inbound closed");
    assert_eq!(session_reply.msg_type, MessageType::AuthSession);

    (client, inbound)
}

#[tokio::test]
async fn e2e_quic_session_lifecycle_emits_vcon_and_rvoip_vcon_doc_roundtrips() {
    let _ = tracing_subscriber::fmt::try_init();
    install_crypto_provider();

    // === 1. Server bootstrap: rvoip-core Orchestrator + rvoip-quic
    //    adapter (which spins per-peer rvoip-uctp coordinators). ===
    let (server_ep, cert_der) = server_endpoint("127.0.0.1:0".parse().unwrap());
    let server_addr = server_ep.local_addr().expect("local_addr");
    let mut routes = dispatch_by_alpn(Arc::clone(&server_ep), &[ALPN_UCTP]).expect("dispatcher");
    let accept_rx = routes.take(ALPN_UCTP).expect("uctp/1 channel");

    let cfg = UctpQuicConfig::new(Arc::clone(&server_ep), accept_rx, bearer_stub());
    let adapter = UctpQuicAdapter::new(cfg).await.expect("adapter");

    let orchestrator = Orchestrator::new(Config::default());
    orchestrator
        .register(adapter.clone() as Arc<dyn ConnectionAdapter>)
        .expect("register");

    // === 2. Pre-create a Conversation + Session on the server side.
    //    P1 surface — this is the new lifecycle layer that didn't
    //    exist before the gap-plan sweep. ===
    let tenant = TenantId::new();
    let cid = orchestrator
        .open_conversation(
            tenant.clone(),
            ConversationPolicy::default(),
            HashMap::new(),
        )
        .await
        .expect("open_conversation");
    let alice = ParticipantId::new();
    let bot = ParticipantId::new();
    let sid = orchestrator
        .start_session(cid.clone(), SessionMedium::Voice, vec![alice.clone()])
        .await
        .expect("start_session");

    // Subscribe to the orchestrator's event bus before any client
    // traffic so we capture every transition in order.
    let mut events = orchestrator.subscribe_events();

    // === 3. Client connects + auth handshake (rvoip-uctp wire). ===
    let client_ep = client_endpoint();
    let (client, mut client_inbound) = dial_authenticated(&client_ep, server_addr, &cert_der).await;

    // === 4. Client sends `session.invite` — server emits
    //    Event::ConnectionInbound. ===
    let invite_sid = format!("sess_e2e_{}", rand_hex());
    let invite = UctpEnvelope {
        v: 1,
        msg_type: MessageType::SessionInvite,
        id: format!("env_{}", rand_hex()),
        ts: Utc::now(),
        cid: Some(format!("conv_e2e_{}", rand_hex())),
        sid: Some(invite_sid),
        connid: None,
        in_reply_to: None,
        payload: serde_json::to_value(SessionInvite {
            from: "part_alice_wire".into(),
            to: vec!["part_bot".into()],
            medium: "voice".into(),
            intent: "synchronous-engagement".into(),
            capabilities_offer: serde_json::Value::Object(Default::default()),
        })
        .unwrap(),
        signature: None,
    };
    client.send(invite).await.expect("send session.invite");

    // === 5. Capture the ConnectionInbound event the orchestrator
    //    normalizes from the UCTP wire-side `session.invite`. ===
    let connection_id = loop {
        let ev = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("timed out waiting for ConnectionInbound")
            .expect("event bus closed");
        match ev {
            Event::ConnectionInbound { connection_id, .. } => break connection_id,
            // ConversationOpened / SessionStarted / ParticipantJoined
            // / ConnectionAuthenticated may all arrive first; ignore
            // them and wait for the inbound.
            _ => continue,
        }
    };

    // === 6. Route the inbound into our pre-created Session via the
    //    P1.8 binding path. ===
    orchestrator
        .route_inbound_connection(
            connection_id.clone(),
            InboundAction::Accept {
                session_id: sid.clone(),
                participant_id: bot.clone(),
            },
        )
        .await
        .expect("accept into pre-created Session");

    // Pre-bind Alice to the Session too (the wire invite arrived
    // under a different participant_id than the orchestrator-side
    // alice; in production the participant identity would be derived
    // from the auth handshake, here we wire it explicitly).
    orchestrator
        .join_session(
            sid.clone(),
            alice.clone(),
            ParticipantKind::Human,
            ParticipantRole::Customer,
        )
        .await
        .expect("alice join");
    orchestrator
        .join_session(
            sid.clone(),
            bot.clone(),
            ParticipantKind::Ai,
            ParticipantRole::Agent,
        )
        .await
        .expect("bot join");

    // === 7. Verify the Session is now Active and carries the live
    //    Connection. ===
    {
        let s = orchestrator.session(&sid).expect("session present");
        let s = s.read().unwrap();
        assert_eq!(
            s.state,
            rvoip_core::session::SessionState::Active,
            "Session must transition to Active after first attach"
        );
        assert!(
            s.connections.contains_key(&connection_id),
            "Session.connections must contain the bound QUIC connection"
        );
        assert!(s.participants.contains(&alice));
        assert!(s.participants.contains(&bot));
    }

    // === 8. End the Session — triggers SessionEnded + VconReady. ===
    orchestrator
        .end_session(sid.clone(), EndReason::Normal)
        .await
        .expect("end_session");

    let mut saw_session_ended = false;
    let mut vcon_handle: Option<rvoip_core::store::VconHandle> = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        let Ok(Ok(ev)) = tokio::time::timeout(Duration::from_millis(200), events.recv()).await
        else {
            continue;
        };
        match ev {
            Event::SessionEnded { session_id, .. } if session_id == sid => {
                saw_session_ended = true;
            }
            Event::VconReady {
                session_id, handle, ..
            } if session_id == sid => {
                vcon_handle = Some(handle);
            }
            _ => continue,
        }
        if saw_session_ended && vcon_handle.is_some() {
            break;
        }
    }
    assert!(
        saw_session_ended,
        "SessionEnded must fire after end_session"
    );
    let handle = vcon_handle.expect("VconReady must fire after SessionEnded");
    assert!(
        handle.url.starts_with("memory:vcon/"),
        "default MemoryVconStore URL shape"
    );
    assert!(
        handle.content_hash.starts_with("sha256:"),
        "content_hash uses sha256: prefix"
    );

    // === 9. Verify the bytes round-trip + hash-check. ===
    let store = orchestrator.config.vcon_store.clone();
    let bytes = store
        .get(&handle)
        .await
        .unwrap()
        .expect("vCon bytes resolve");
    assert!(!bytes.is_empty(), "vCon bytes are non-empty");
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(&bytes);
    let digest = h.finalize();
    let hex: String = digest.iter().map(|b| format!("{:02x}", b)).collect();
    assert_eq!(
        handle.content_hash,
        format!("sha256:{}", hex),
        "stored bytes must hash-match the VconHandle"
    );

    // === 10. Side-by-side: exercise rvoip-vcon's *own* document
    //    builder + store. Demonstrates that the IETF-shaped vCon
    //    model (rvoip-vcon) and rvoip-core's encoder-shape model
    //    are both functional. In production the consumer would
    //    wire rvoip-vcon as a custom VconStore impl bridging
    //    `rvoip_core::store::VconStore` to `rvoip_vcon::VconStore`. ===
    let ietf_store = IetfMemoryVconStore::new();
    let ietf_vcon = VconBuilder::new()
        .with_party(Party {
            name: Some("Alice".into()),
            uuid: Some(alice.to_string()),
            role: Some("customer".into()),
            ..Default::default()
        })
        .with_party(Party {
            name: Some("Bot".into()),
            uuid: Some(bot.to_string()),
            role: Some("agent".into()),
            ..Default::default()
        })
        .recording(Utc::now(), 1500, vec![0, 1], "audio/opus")
        .build();
    let stored_uuid = ietf_store.put(ietf_vcon.clone()).await.expect("ietf put");
    let fetched = ietf_store.get(&stored_uuid).await.expect("ietf get");
    assert_eq!(fetched.parties.len(), 2);
    assert_eq!(fetched.parties[0].name.as_deref(), Some("Alice"));
    assert_eq!(fetched.parties[1].name.as_deref(), Some("Bot"));
    assert_eq!(fetched.dialog.len(), 1);

    // Drain the client side of any lingering envelopes so the test
    // doesn't leave the QUIC connection in an awkward state.
    let _ = tokio::time::timeout(Duration::from_millis(100), client_inbound.recv()).await;

    // Close the conversation now that the session ended — exercises
    // the close-after-end path.
    orchestrator
        .close_conversation(cid, false)
        .await
        .expect("close_conversation");
}
