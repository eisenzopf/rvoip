//! Call-center agent walkthrough — SIP_API_DESIGN_2 §11.2.
//!
//! Run with:
//!
//!   cargo run --example call_center_agent
//!
//! Demonstrates the call-center agent lifecycle:
//!
//! 1. Agent registers with the (mock) registrar via the canonical
//!    `coord.register(uri, user, pass).send()` builder.
//! 2. Agent runs as a `CallbackPeer` so inbound calls are routed
//!    through its `on_incoming` hook.
//! 3. On `on_incoming`, the agent accepts the customer's INVITE and
//!    immediately blind-transfers via `session.refer(target).send()`
//!    — the SessionHandle is captured from `call.accept().await?`.
//! 4. The customer's UA follows the REFER and ends up talking to the
//!    colleague.
//!
//! Wired in-process: mock registrar (raw UDP) + customer
//! (`UnifiedCoordinator`) + agent (`CallbackPeerBuilder`) + colleague
//! (`CallbackPeer<AutoAccept>`). The handler closure no longer needs
//! to close over the agent's coordinator — the `SessionHandle`
//! returned by `accept()` carries everything the closure needs to
//! drive the in-dialog REFER.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rvoip_sip::api::callback_peer::{
    CallHandler, CallHandlerDecision, CallbackPeer, CallbackPeerBuilder,
};
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::{CallState, Config, UnifiedCoordinator};

use tokio::net::UdpSocket;

use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::HeaderValue;
use rvoip_sip_core::{Message, Method, StatusCode, TypedHeader};

use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

const REGISTRAR_PORT: u16 = 37400;
const CUSTOMER_PORT: u16 = 37401;
const AGENT_PORT: u16 = 37402;
const COLLEAGUE_PORT: u16 = 37403;

struct AutoAccept(&'static str);

#[async_trait::async_trait]
impl CallHandler for AutoAccept {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        println!("[{}] auto-accepting inbound from {}", self.0, call.from);
        let _ = call.accept().await;
        CallHandlerDecision::Accept
    }
}

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

#[tokio::main]
async fn main() -> rvoip_sip::Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    // ── Mock registrar ─────────────────────────────────────────────
    let registrar = spawn_mock_registrar(REGISTRAR_PORT).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    println!("[reg ] mock registrar on 127.0.0.1:{REGISTRAR_PORT}");

    // ── Colleague ──────────────────────────────────────────────────
    let colleague = CallbackPeer::new(
        AutoAccept("colleague"),
        Config::local("colleague", COLLEAGUE_PORT),
    )
    .await?;
    let colleague_shutdown = colleague.shutdown_handle();
    let colleague_task = tokio::spawn(async move { colleague.run().await });
    tokio::time::sleep(Duration::from_millis(150)).await;
    println!("[col ] colleague on 127.0.0.1:{COLLEAGUE_PORT}");

    // ── Agent (CallbackPeerBuilder) ────────────────────────────────
    //
    // The handler captures only the colleague URI and a done-flag.
    // The SessionHandle returned by `call.accept().await?` carries
    // everything else the closure needs to drive the in-dialog REFER
    // — no need to back-pipe the coordinator into the closure.
    let done = Arc::new(AtomicBool::new(false));
    let done_for_handler = done.clone();
    let colleague_uri = format!("sip:colleague@127.0.0.1:{COLLEAGUE_PORT}");
    let colleague_uri_for_handler = colleague_uri.clone();

    let agent_peer = CallbackPeerBuilder::new(Config::local("agent", AGENT_PORT))
        .on_incoming(move |call| {
            let done = done_for_handler.clone();
            let target = colleague_uri_for_handler.clone();
            async move {
                if done.swap(true, Ordering::SeqCst) {
                    return CallHandlerDecision::Accept;
                }
                let call_id_display = call.call_id.0.clone();
                println!("[agent] accepting customer call {}", call_id_display);
                let session = match call.accept().await {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("[agent] accept failed: {e:?}");
                        return CallHandlerDecision::Accept;
                    }
                };

                tokio::spawn(async move {
                    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
                    loop {
                        if let Ok(state) = session.state().await {
                            if state == CallState::Active {
                                break;
                            }
                        }
                        if tokio::time::Instant::now() >= deadline {
                            eprintln!("[agent] call never reached Active; abandoning REFER");
                            return;
                        }
                        tokio::time::sleep(Duration::from_millis(40)).await;
                    }
                    println!("[agent] blind-transferring to {}", target);
                    // NEW ergonomic — REFER straight off the SessionHandle.
                    if let Err(e) = session.refer(target).send().await {
                        eprintln!("[agent] REFER failed: {e:?}");
                    }
                });
                CallHandlerDecision::Accept
            }
        })
        .build()
        .await?;
    let agent_coord = agent_peer.coordinator().clone();

    println!("[agent] registering with sip:127.0.0.1:{REGISTRAR_PORT}");
    let _reg_handle = agent_coord
        .register(format!("sip:127.0.0.1:{REGISTRAR_PORT}"), "agent", "secret")
        .with_expires(3600)
        .send()
        .await?;
    println!("[agent] register OK");

    let agent_shutdown = agent_peer.shutdown_handle();
    let agent_task = tokio::spawn(async move { agent_peer.run().await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    println!("[agent] callback peer on 127.0.0.1:{AGENT_PORT}");

    // ── Customer (UAC) ─────────────────────────────────────────────
    let customer = UnifiedCoordinator::new(Config::local("customer", CUSTOMER_PORT)).await?;
    tokio::time::sleep(Duration::from_millis(150)).await;

    println!("[cust] INVITE to sip:agent@127.0.0.1:{AGENT_PORT}");
    let call_id = customer
        .invite(
            Some(format!("sip:customer@127.0.0.1:{CUSTOMER_PORT}")),
            format!("sip:agent@127.0.0.1:{AGENT_PORT}"),
        )
        .send()
        .await?;

    // Give the agent time to accept and dispatch the REFER. The
    // customer's state machine handles the inbound REFER and emits
    // a fresh INVITE to the colleague URI automatically.
    tokio::time::sleep(Duration::from_secs(3)).await;
    println!("[cust] customer session is {}", call_id.0);

    println!("[cust] hangup");
    let _ = customer.hangup(&call_id).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    agent_shutdown.shutdown();
    colleague_shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), agent_task).await;
    let _ = tokio::time::timeout(Duration::from_secs(2), colleague_task).await;
    registrar.abort();
    println!("[done] call-center agent walkthrough complete.");
    Ok(())
}
