//! End-to-end acceptance test for per-call outbound TLS client identity.
//!
//! Companion to `tls_call_integration.rs`, which proves a single sips:
//! call establishes over TLS. This test proves the actual ask behind the
//! per-call `OutboundTlsConfig` feature: **one process placing two
//! simultaneous calls to two different endpoints, each trusting only its
//! own peer's certificate** — with no TLS client identity baked into
//! `Config` at all (`SipTlsMode::ClientOnly`, no cert/key). Both calls
//! must reach `CallAnswered` concurrently, proving the per-call identity
//! override is what carries each dial, not any process-wide default.

#[cfg(feature = "dev-insecure-tls")]
use std::io::Write;
#[cfg(feature = "dev-insecure-tls")]
use std::path::PathBuf;
#[cfg(feature = "dev-insecure-tls")]
use std::time::Duration;

#[cfg(feature = "dev-insecure-tls")]
use rvoip_sip::api::events::Event;
#[cfg(feature = "dev-insecure-tls")]
use rvoip_sip::api::stream_peer::EventReceiver;
#[cfg(feature = "dev-insecure-tls")]
use rvoip_sip::api::unified::{Config, SipTlsMode, UnifiedCoordinator};
#[cfg(feature = "dev-insecure-tls")]
use rvoip_sip_transport::{OutboundTlsConfig, TlsClientConfig};

#[cfg(feature = "dev-insecure-tls")]
fn write_self_signed_cert_for_name(name: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let cert_path = dir.path().join("server.crt");
    let key_path = dir.path().join("server.key");

    let cert =
        rcgen::generate_simple_self_signed(vec![name.to_string()]).expect("rcgen self-signed");
    let cert_pem = cert.cert.pem();
    let key_pem = cert.signing_key.serialize_pem();

    std::fs::File::create(&cert_path)
        .and_then(|mut f| f.write_all(cert_pem.as_bytes()))
        .expect("write cert");
    std::fs::File::create(&key_path)
        .and_then(|mut f| f.write_all(key_pem.as_bytes()))
        .expect("write key");

    (dir, cert_path, key_path)
}

/// Wait for any event matching `pred` on `events`, up to `timeout`.
#[cfg(feature = "dev-insecure-tls")]
async fn wait_for<F>(events: &mut EventReceiver, timeout: Duration, mut pred: F) -> Option<Event>
where
    F: FnMut(&Event) -> bool,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let next = tokio::time::timeout(remaining, events.next()).await;
        match next {
            Err(_) => return None,
            Ok(None) => return None,
            Ok(Some(event)) => {
                if pred(&event) {
                    return Some(event);
                }
            }
        }
    }
}

#[cfg(feature = "dev-insecure-tls")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_concurrent_calls_use_distinct_per_call_tls_identities() {
    let _ = tracing_subscriber::fmt::try_init();

    // Two UAS endpoints ("bob" and "carol"), each with its own
    // self-signed identity and its own truststore — distinct hostnames
    // so a cert trusted for one is not valid for the other.
    let (_bob_dir, bob_cert, bob_key) = write_self_signed_cert_for_name("bob.example.test");
    let (_carol_dir, carol_cert, carol_key) = write_self_signed_cert_for_name("carol.example.test");

    let alice_sip_port = 36161;
    let bob_sip_port = 36171;
    let carol_sip_port = 36181;
    let bob_tls_port = bob_sip_port + 1;
    let carol_tls_port = carol_sip_port + 1;

    // Alice is ClientOnly with NO TLS cert/key/truststore configured at
    // all — every bit of trust needed to reach bob and carol rides the
    // per-call `OutboundCallBuilder::with_transport_security` override.
    let mut alice_cfg = Config::local("alice", alice_sip_port);
    alice_cfg.sip_tls_mode = SipTlsMode::ClientOnly;
    alice_cfg.contact_uri = Some(format!("sip:alice@127.0.0.1:{}", alice_sip_port));

    let mut bob_cfg = Config::local("bob", bob_sip_port);
    bob_cfg.sip_tls_mode = SipTlsMode::ServerOnly;
    bob_cfg.tls_bind_addr = Some(format!("127.0.0.1:{}", bob_tls_port).parse().unwrap());
    bob_cfg.tls_cert_path = Some(bob_cert.clone());
    bob_cfg.tls_key_path = Some(bob_key.clone());
    bob_cfg.contact_uri = Some(format!("sips:bob@127.0.0.1:{};transport=tls", bob_tls_port));

    let mut carol_cfg = Config::local("carol", carol_sip_port);
    carol_cfg.sip_tls_mode = SipTlsMode::ServerOnly;
    carol_cfg.tls_bind_addr = Some(format!("127.0.0.1:{}", carol_tls_port).parse().unwrap());
    carol_cfg.tls_cert_path = Some(carol_cert.clone());
    carol_cfg.tls_key_path = Some(carol_key.clone());
    carol_cfg.contact_uri = Some(format!(
        "sips:carol@127.0.0.1:{};transport=tls",
        carol_tls_port
    ));

    let alice = UnifiedCoordinator::new(alice_cfg)
        .await
        .expect("alice coordinator");
    let bob = UnifiedCoordinator::new(bob_cfg)
        .await
        .expect("bob coordinator");
    let carol = UnifiedCoordinator::new(carol_cfg)
        .await
        .expect("carol coordinator");

    let mut alice_events = alice.events().await.expect("alice events");
    let mut bob_events = bob.events().await.expect("bob events");
    let mut carol_events = carol.events().await.expect("carol events");

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Per-call identity for the bob leg: trusts ONLY bob's cert.
    let identity_for_bob = OutboundTlsConfig {
        client: TlsClientConfig {
            extra_ca_path: Some(bob_cert.clone()),
            insecure_skip_verify: false,
            ..Default::default()
        },
        server_name: Some("bob.example.test".to_string()),
    };
    // Per-call identity for the carol leg: trusts ONLY carol's cert.
    let identity_for_carol = OutboundTlsConfig {
        client: TlsClientConfig {
            extra_ca_path: Some(carol_cert.clone()),
            insecure_skip_verify: false,
            ..Default::default()
        },
        server_name: Some("carol.example.test".to_string()),
    };

    let bob_target = format!("sips:bob@127.0.0.1:{}", bob_tls_port);
    let carol_target = format!("sips:carol@127.0.0.1:{}", carol_tls_port);

    // Fire both INVITEs concurrently from the same Alice coordinator —
    // this is the literal ask: one process, two simultaneous calls,
    // distinct identities/truststores.
    let (bob_call, carol_call) = tokio::join!(
        alice
            .invite(Some("sips:alice@127.0.0.1".to_string()), &bob_target)
            .with_transport_security(identity_for_bob)
            .send(),
        alice
            .invite(Some("sips:alice@127.0.0.1".to_string()), &carol_target)
            .with_transport_security(identity_for_carol)
            .send(),
    );
    let alice_to_bob_session = bob_call.expect("alice invite to bob");
    let alice_to_carol_session = carol_call.expect("alice invite to carol");

    let bob_incoming = wait_for(&mut bob_events, Duration::from_secs(8), |ev| {
        matches!(ev, Event::IncomingCall { .. })
    })
    .await
    .expect("bob did not see IncomingCall over its own TLS identity");
    let bob_session_id = match bob_incoming {
        Event::IncomingCall { call_id, .. } => call_id,
        _ => unreachable!(),
    };

    let carol_incoming = wait_for(&mut carol_events, Duration::from_secs(8), |ev| {
        matches!(ev, Event::IncomingCall { .. })
    })
    .await
    .expect("carol did not see IncomingCall over its own TLS identity");
    let carol_session_id = match carol_incoming {
        Event::IncomingCall { call_id, .. } => call_id,
        _ => unreachable!(),
    };

    bob.accept_call(&bob_session_id)
        .await
        .expect("bob accept_call");
    carol
        .accept_call(&carol_session_id)
        .await
        .expect("carol accept_call");

    // Alice must observe BOTH calls reach CallAnswered — one leg used
    // bob's identity, the other carol's, from the same process.
    let mut bob_answered = false;
    let mut carol_answered = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    while !(bob_answered && carol_answered) {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, alice_events.next()).await {
            Ok(Some(Event::CallAnswered { call_id, .. })) => {
                if call_id == alice_to_bob_session {
                    bob_answered = true;
                } else if call_id == alice_to_carol_session {
                    carol_answered = true;
                }
            }
            Ok(Some(_)) => continue,
            _ => break,
        }
    }

    assert!(
        bob_answered,
        "alice did not observe CallAnswered for the bob leg (identity trusting bob's cert)"
    );
    assert!(
        carol_answered,
        "alice did not observe CallAnswered for the carol leg (identity trusting carol's cert)"
    );

    // Clean up.
    alice.hangup(&alice_to_bob_session).await.ok();
    alice.hangup(&alice_to_carol_session).await.ok();
    bob.terminate_current_session().await.ok();
    carol.terminate_current_session().await.ok();

    tokio::time::sleep(Duration::from_millis(200)).await;
}
