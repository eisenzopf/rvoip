//! Endpoint softphone walkthrough — SIP_API_DESIGN_2 §11.1.
//!
//! Run with:
//!
//!   cargo run --example endpoint_softphone
//!
//! Demonstrates the canonical UAC lifecycle:
//!
//! 1. Register with a (mock) registrar via
//!    `coord.register(uri, user, pass).with_expires(s).send()`.
//! 2. Place an outbound INVITE via `coord.invite(from, target).send()`.
//! 3. Mid-dialog `hold()` → sleep → `resume()` (RFC 3264 re-INVITEs).
//! 4. Send a DTMF digit via `coord.send_dtmf(session, '5')` (RFC 2833
//!    over RTP, or SIP INFO depending on negotiation).
//! 5. Hang up via `coord.hangup(session)`.
//!
//! The mock registrar is a 30-line raw-UDP responder embedded below so
//! the example runs without external infrastructure. The callee
//! ("bob") is a `CallbackPeer<AutoAccept>` on a local port.

use std::sync::Arc;
use std::time::Duration;

use rvoip_sip::api::callback_peer::{CallHandler, CallHandlerDecision, CallbackPeer};
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::{CallState, Config, UnifiedCoordinator};

use tokio::net::UdpSocket;

use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::HeaderValue;
use rvoip_sip_core::{Message, Method, StatusCode, TypedHeader};

use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

const REGISTRAR_PORT: u16 = 37200;
const ALICE_PORT: u16 = 37201;
const BOB_PORT: u16 = 37202;

struct AutoAccept;

#[async_trait::async_trait]
impl CallHandler for AutoAccept {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        let _ = call.accept().await;
        CallHandlerDecision::Accept
    }
}

/// Bind a raw-UDP socket and answer every inbound REGISTER with a 200
/// OK + the Contact echo. Spawns a background task; returns the
/// `JoinHandle` so the example can abort it on exit.
async fn spawn_mock_registrar(port: u16) -> tokio::task::JoinHandle<()> {
    let sock = Arc::new(
        UdpSocket::bind(format!("127.0.0.1:{port}"))
            .await
            .expect("registrar bind"),
    );
    tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        loop {
            let (n, from) = match sock.recv_from(&mut buf).await {
                Ok(p) => p,
                Err(_) => return,
            };
            let parsed = match parse_message(&buf[..n]) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let request = match parsed {
                Message::Request(r) if r.method() == Method::Register => r,
                _ => continue,
            };
            let mut resp = create_response(&request, StatusCode::Ok);
            if let Some(c) = request.header(&HeaderName::Contact) {
                resp.headers.push(c.clone());
            }
            resp.headers.push(TypedHeader::Other(
                HeaderName::Expires,
                HeaderValue::Raw(b"3600".to_vec()),
            ));
            let _ = sock
                .send_to(&Message::Response(resp).to_bytes(), from)
                .await;
        }
    })
}

async fn wait_state(
    coord: &UnifiedCoordinator,
    sid: &rvoip_sip::SessionId,
    target: CallState,
    deadline: Duration,
) -> bool {
    let end = tokio::time::Instant::now() + deadline;
    loop {
        if let Ok(state) = coord.get_state(sid).await {
            if state == target {
                return true;
            }
        }
        if tokio::time::Instant::now() >= end {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}

#[tokio::main]
async fn main() -> rvoip_sip::Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    // ── Mock registrar ─────────────────────────────────────────────
    let registrar = spawn_mock_registrar(REGISTRAR_PORT).await;
    tokio::time::sleep(Duration::from_millis(120)).await;
    println!("[reg ] mock registrar listening on 127.0.0.1:{REGISTRAR_PORT}");

    // ── Bob (auto-accept callee) ───────────────────────────────────
    let bob = CallbackPeer::new(AutoAccept, Config::local("bob", BOB_PORT)).await?;
    let bob_shutdown = bob.shutdown_handle();
    let bob_task = tokio::spawn(async move { bob.run().await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    println!("[bob ] callback peer running on 127.0.0.1:{BOB_PORT}");

    // ── Alice (softphone) ──────────────────────────────────────────
    let alice = UnifiedCoordinator::new(Config::local("alice", ALICE_PORT)).await?;
    tokio::time::sleep(Duration::from_millis(150)).await;

    println!("[alice] registering with sip:127.0.0.1:{REGISTRAR_PORT}");
    let _reg = alice
        .register(
            format!("sip:127.0.0.1:{REGISTRAR_PORT}"),
            "alice",
            "supersecret",
        )
        .with_expires(3600)
        .send()
        .await?;
    println!("[alice] register OK");

    println!("[alice] placing INVITE to sip:bob@127.0.0.1:{BOB_PORT}");
    let call_id = alice
        .invite(
            Some(format!("sip:alice@127.0.0.1:{ALICE_PORT}")),
            format!("sip:bob@127.0.0.1:{BOB_PORT}"),
        )
        .send()
        .await?;

    if !wait_state(&alice, &call_id, CallState::Active, Duration::from_secs(10)).await {
        eprintln!("[alice] call never reached Active");
        return Ok(());
    }
    println!("[alice] call active as {}", call_id.0);

    tokio::time::sleep(Duration::from_secs(1)).await;
    println!("[alice] hold()");
    alice.hold(&call_id).await?;
    tokio::time::sleep(Duration::from_millis(800)).await;

    println!("[alice] resume()");
    alice.resume(&call_id).await?;
    tokio::time::sleep(Duration::from_millis(800)).await;

    println!("[alice] sending DTMF '5'");
    alice.send_dtmf(&call_id, '5').await?;
    tokio::time::sleep(Duration::from_millis(500)).await;

    println!("[alice] hangup");
    alice.hangup(&call_id).await?;
    tokio::time::sleep(Duration::from_millis(400)).await;

    bob_shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), bob_task).await;
    registrar.abort();
    println!("[done] softphone walkthrough complete.");
    Ok(())
}
