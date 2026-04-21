//! End-to-end TLS call regression test.
//!
//! Stands up two `UnifiedCoordinator` instances, configures both for
//! TLS using a self-signed cert (with `tls_insecure_skip_verify = true`
//! so the dev cert is accepted), then has Alice place a call to Bob
//! over a `sips:` URI. The `MultiplexedTransport` routes the outbound
//! INVITE through the TLS transport; Bob's TLS listener accepts the
//! handshake and surfaces an `IncomingCall` event. We then accept,
//! observe `CallEstablished` on Alice, and hang up cleanly.
//!
//! This is the **Step 1C** regression check from
//! `crates/TLS_SIP_IMPLEMENTATION_PLAN.md`: end-to-end call setup over
//! `sips:` URIs through real session-core APIs, not just unit-level TLS
//! transport plumbing.

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use rvoip_session_core::api::events::Event;
use rvoip_session_core::api::stream_peer::EventReceiver;
use rvoip_session_core::api::unified::{Config, UnifiedCoordinator};

fn write_self_signed_localhost_cert() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let cert_path = dir.path().join("server.crt");
    let key_path = dir.path().join("server.key");

    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
        .expect("rcgen self-signed");
    let cert_pem = cert.serialize_pem().expect("cert PEM");
    let key_pem = cert.serialize_private_key_pem();

    std::fs::File::create(&cert_path)
        .and_then(|mut f| f.write_all(cert_pem.as_bytes()))
        .expect("write cert");
    std::fs::File::create(&key_path)
        .and_then(|mut f| f.write_all(key_pem.as_bytes()))
        .expect("write key");

    (dir, cert_path, key_path)
}

/// Wait for any event matching `pred` on `events`, up to `timeout`.
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sips_call_establishes_through_tls_transport() {
    let _ = tracing_subscriber::fmt::try_init();

    let (_cert_dir, cert_path, key_path) = write_self_signed_localhost_cert();

    // Use distinct ports per side; TLS is auto-bound to `sip_port + 1`
    // by `TransportManager::initialize` (mirrors the RFC 3261 5060→5061
    // convention). Spread the two peers far enough apart that no
    // port-pair collides.
    let alice_sip_port = 36061;
    let bob_sip_port = 36071;
    // Bob's TLS listener lands at `bob_sip_port + 1`; that's where the
    // sips: URI must point.
    let bob_tls_port = bob_sip_port + 1;

    let mut alice_cfg = Config::local("alice", alice_sip_port);
    alice_cfg.tls_cert_path = Some(cert_path.clone());
    alice_cfg.tls_key_path = Some(key_path.clone());
    // Self-signed cert isn't in the system trust store; allow it for
    // this test only.
    alice_cfg.tls_insecure_skip_verify = true;

    let mut bob_cfg = Config::local("bob", bob_sip_port);
    bob_cfg.tls_cert_path = Some(cert_path.clone());
    bob_cfg.tls_key_path = Some(key_path.clone());
    bob_cfg.tls_insecure_skip_verify = true;

    let alice = UnifiedCoordinator::new(alice_cfg)
        .await
        .expect("alice coordinator");
    let bob = UnifiedCoordinator::new(bob_cfg)
        .await
        .expect("bob coordinator");

    // Subscribe to events on both sides BEFORE placing the call so we
    // don't race the publisher.
    let mut alice_events = alice.events().await.expect("alice events");
    let mut bob_events = bob.events().await.expect("bob events");

    // Give both transports a moment to bind their TLS listeners.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Place a call from Alice → Bob using a `sips:` URI. The
    // MultiplexedTransport must observe the scheme and route through
    // the TLS listener rather than UDP.
    let target = format!("sips:bob@127.0.0.1:{}", bob_tls_port);
    let _alice_session = alice
        .make_call("sips:alice@127.0.0.1", &target)
        .await
        .expect("alice make_call");

    // Bob should see an IncomingCall event. The session_id field
    // identifies the new session for accept_call.
    let incoming = wait_for(&mut bob_events, Duration::from_secs(8), |ev| {
        matches!(ev, Event::IncomingCall { .. })
    })
    .await
    .expect("bob did not see IncomingCall over TLS");

    let bob_session_id = match incoming {
        Event::IncomingCall { call_id, .. } => call_id,
        _ => unreachable!(),
    };

    bob.accept_call(&bob_session_id)
        .await
        .expect("bob accept_call");

    // Alice should see her call answered (200 OK landed via the TLS
    // transport).
    let answered = wait_for(&mut alice_events, Duration::from_secs(8), |ev| {
        matches!(ev, Event::CallAnswered { .. })
    })
    .await;
    assert!(
        answered.is_some(),
        "alice did not observe CallAnswered after TLS sips: call setup"
    );

    // Hang up cleanly so we don't leak background tasks.
    bob.terminate_current_session().await.ok();
    alice.terminate_current_session().await.ok();

    // Drain any final events so background tasks can exit.
    tokio::time::sleep(Duration::from_millis(200)).await;
}
