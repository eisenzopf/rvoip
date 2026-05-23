use std::sync::Arc;
use std::time::Duration;

use rvoip_sip::api::callback_peer::{CallHandler, CallHandlerDecision, CallbackPeer};
use rvoip_sip::api::events::{CallId, Event};
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::api::stream_peer::EventReceiver;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::SipTraceConfig;
use tokio::sync::{mpsc, Notify};

struct BlockingDecisionHandler {
    started_tx: mpsc::Sender<()>,
    release: Arc<Notify>,
}

#[async_trait::async_trait]
impl CallHandler for BlockingDecisionHandler {
    async fn on_incoming_call(&self, _call: IncomingCall) -> CallHandlerDecision {
        let _ = self.started_tx.send(()).await;
        self.release.notified().await;
        CallHandlerDecision::Accept
    }
}

fn traced_config(name: &str, port: u16) -> Config {
    let mut config = Config::local(name, port);
    config.sip_trace = SipTraceConfig {
        enabled: true,
        redact_sensitive_headers: false,
        include_body: true,
        ..SipTraceConfig::default()
    };
    config
}

async fn wait_for_call_answered(
    events: &mut EventReceiver,
    target_call_id: &CallId,
    timeout: Duration,
) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return false;
        }
        match tokio::time::timeout(remaining, events.next()).await {
            Err(_) | Ok(None) => return false,
            Ok(Some(Event::CallAnswered { call_id, .. })) if &call_id == target_call_id => {
                return true;
            }
            Ok(Some(_)) => continue,
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fast_auto_accept_answers_before_callback_decision_runs() {
    let alice_port = 18020;
    let bob_port = 18021;

    let (started_tx, mut started_rx) = mpsc::channel(1);
    let release_callback = Arc::new(Notify::new());

    let bob_config = traced_config("fast-bob", bob_port)
        .with_auto_180_ringing(false)
        .with_fast_auto_accept_incoming_calls(true);
    let bob = CallbackPeer::new(
        BlockingDecisionHandler {
            started_tx,
            release: Arc::clone(&release_callback),
        },
        bob_config,
    )
    .await
    .expect("bob CallbackPeer::new");
    let bob_shutdown = bob.shutdown_handle();
    let bob_task = tokio::spawn(async move {
        let _ = bob.run().await;
    });
    tokio::time::sleep(Duration::from_millis(200)).await;

    let alice = UnifiedCoordinator::new(traced_config("fast-alice", alice_port))
        .await
        .expect("alice UnifiedCoordinator::new");
    let mut alice_events = alice.events().await.expect("alice events");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let call_id = alice
        .invite(
            Some(format!("sip:alice@127.0.0.1:{alice_port}")),
            format!("sip:bob@127.0.0.1:{bob_port}"),
        )
        .send()
        .await
        .expect("invite send");

    tokio::time::timeout(Duration::from_secs(8), started_rx.recv())
        .await
        .expect("callback should start")
        .expect("callback start channel should remain open");

    assert!(
        wait_for_call_answered(&mut alice_events, &call_id, Duration::from_secs(8)).await,
        "alice did not observe CallAnswered while callback decision was blocked"
    );

    release_callback.notify_one();
    let _ = alice.bye(&call_id).send().await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    bob_shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), bob_task).await;
}
