//! Panic-safety test for `IncomingCall::Drop`.
//!
//! Verifies that when a `CallHandler::on_incoming_call` implementation panics
//! before resolving the incoming call, the panic-aware `Drop` fires and
//! sends a `500 Server Internal Error` response via the state machine's
//! `SendRejectResponse` action.
//!
//! ## What this test verifies
//!
//! - `IncomingCall::drop` correctly detects panic unwinding.
//! - The spawned `reject_call` completes without error — the state machine
//!   executes the rejection, builds a 500 response, and hands it to dialog-core.
//! - The UAC receives the 500 as `Event::CallFailed { status_code: 500, .. }`
//!   within a few seconds (not waiting ~3 min for Timer C).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::time::sleep;

use rvoip_session_core_v3::{
    CallHandler, CallHandlerDecision, CallbackPeer, Config, Event, IncomingCall, StreamPeer,
};
use tokio::time::timeout;

/// Handler that panics as soon as it receives an incoming call, without
/// resolving it. Drop's panic-aware path must send a 500 in its place.
struct PanickingHandler {
    called: Arc<AtomicBool>,
}

#[async_trait]
impl CallHandler for PanickingHandler {
    async fn on_incoming_call(&self, _call: IncomingCall) -> CallHandlerDecision {
        self.called.store(true, Ordering::SeqCst);
        panic!("intentional panic — panic-safety test");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn panicking_handler_triggers_drop_safety_net() {
    // Unique ports for this test.
    const SERVER_PORT: u16 = 15310;
    const CLIENT_PORT: u16 = 15311;

    let called = Arc::new(AtomicBool::new(false));
    let handler = PanickingHandler { called: called.clone() };

    let server = CallbackPeer::new(handler, Config::local("srv", SERVER_PORT))
        .await
        .expect("build server");
    let stop = server.shutdown_handle();

    let server_task = tokio::spawn(async move {
        let _ = server.run().await;
    });

    sleep(Duration::from_millis(500)).await;

    // Subscribe to UAC events before placing the call so we don't race
    // against the 500 arriving.
    let mut caller = StreamPeer::with_config(Config::local("caller", CLIENT_PORT))
        .await
        .expect("build caller");
    let mut events = caller.control().subscribe_events().await.expect("subscribe");
    let handle = caller
        .call(&format!("sip:srv@127.0.0.1:{}", SERVER_PORT))
        .await
        .expect("send INVITE");

    // Wait up to 5 seconds for CallFailed — well under Timer C's 3 minutes.
    let status = timeout(Duration::from_secs(5), async {
        loop {
            match events.next().await {
                Some(Event::CallFailed { call_id, status_code, .. }) if call_id == *handle.id() => {
                    return Some(status_code);
                }
                Some(_) => continue,
                None => return None,
            }
        }
    })
    .await
    .expect("timed out waiting for CallFailed — panic safety net did not propagate to UAC")
    .expect("event stream closed unexpectedly");

    assert!(
        called.load(Ordering::SeqCst),
        "handler never ran — INVITE did not reach the UAS"
    );
    assert_eq!(status, 500, "expected 500 Server Internal Error, got {}", status);

    // Clean shutdown
    stop.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), server_task).await;
}

/// Sanity: confirm Drop is a no-op in the happy path (no panic, handler
/// consumed the call via accept()). If Drop spuriously rejected here, the
/// test would fail because accept wouldn't have completed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn normal_path_drop_is_noop() {
    const SERVER_PORT: u16 = 15312;
    const CLIENT_PORT: u16 = 15313;

    struct AcceptHandler;
    #[async_trait]
    impl CallHandler for AcceptHandler {
        async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
            // Consume the IncomingCall explicitly — sets resolved=true.
            let _ = call.accept().await;
            CallHandlerDecision::Accept
        }
    }

    let server = CallbackPeer::new(AcceptHandler, Config::local("srv", SERVER_PORT))
        .await
        .expect("build server");
    let stop = server.shutdown_handle();
    let server_task = tokio::spawn(async move { let _ = server.run().await; });
    sleep(Duration::from_millis(500)).await;

    let mut caller = StreamPeer::with_config(Config::local("caller", CLIENT_PORT))
        .await
        .expect("build caller");
    let handle = caller
        .call(&format!("sip:srv@127.0.0.1:{}", SERVER_PORT))
        .await
        .expect("send INVITE");

    // If Drop had spuriously fired, the call would have been rejected and
    // wait_for_answered would fail/time out.
    tokio::time::timeout(Duration::from_secs(3), caller.wait_for_answered(handle.id()))
        .await
        .expect("wait_for_answered timed out — Drop may have rejected the call")
        .expect("accept failed");

    let _ = handle.hangup().await;
    stop.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), server_task).await;
}
