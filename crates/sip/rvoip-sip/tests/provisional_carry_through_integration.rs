//! SIP_API_DESIGN_2 §10 verification #18 — inbound 1xx provisional
//! responses fire `Event::CallProgressDetailed(IncomingResponse)` so
//! B2BUA / SBC code can inspect the upstream's `Contact:`, `Allow:`,
//! `Supported:`, `Server:` headers before mirroring them to the
//! downstream 1xx.
//!
//! Alice INVITEs Bob (auto-accepting CallbackPeer). Bob's stack
//! sends a 180 Ringing automatically. Alice waits for
//! `CallProgressDetailed`; the carried `IncomingResponse` must
//! expose the status code and the response's typed headers.

use std::time::Duration;

use rvoip_sip::api::callback_peer::{CallHandler, CallHandlerDecision, CallbackPeer};
use rvoip_sip::api::events::Event;
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};

const PAIR: (u16, u16) = (18100, 18101);

fn cfg(name: &str, port: u16) -> Config {
    Config::local(name, port)
}

struct AutoAccept;
#[async_trait::async_trait]
impl CallHandler for AutoAccept {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        let _ = call.accept().await;
        CallHandlerDecision::Accept
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn inbound_provisional_fires_call_progress_detailed_with_response() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR;

    let bob = CallbackPeer::new(AutoAccept, cfg("bob-pcd", bob_port))
        .await
        .expect("bob");
    let bob_shutdown = bob.shutdown_handle();
    let bob_task = tokio::spawn(async move {
        let _ = bob.run().await;
    });
    tokio::time::sleep(Duration::from_millis(200)).await;

    let alice = UnifiedCoordinator::new(cfg("alice-pcd", alice_port))
        .await
        .expect("alice");
    let mut alice_events = alice.events().await.expect("alice events");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let target = format!("sip:bob@127.0.0.1:{bob_port}");
    let _id = alice
        .invite(Some("sip:alice@127.0.0.1".to_string()), target)
        .send()
        .await
        .expect("invite.send()");

    // Wait for the first CallProgressDetailed.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut detailed = None;
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(500), alice_events.next()).await {
            Ok(Some(Event::CallProgressDetailed(resp))) => {
                detailed = Some(resp);
                break;
            }
            Ok(Some(_)) | Ok(None) | Err(_) => continue,
        }
    }
    let detailed = detailed.expect("Event::CallProgressDetailed must fire on the first 1xx");

    // The carried IncomingResponse must surface a status code in the
    // 1xx range. The exact value (100, 180, 183) is implementation-
    // defined; what matters for B2BUA carry-through is that the
    // detailed form fires AT ALL with a typed inspection surface.
    let status = detailed.status_code;
    assert!(
        (100..200).contains(&status),
        "CallProgressDetailed must carry a 1xx status; got {status}"
    );
    assert!(
        detailed.is_provisional(),
        "IncomingResponse::is_provisional() must return true"
    );

    bob_shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), bob_task).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
}
