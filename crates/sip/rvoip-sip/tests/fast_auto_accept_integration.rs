use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use rvoip_sip::adapters::media_adapter::cleanup_session_diag;
use rvoip_sip::api::callback_peer::{CallHandler, CallHandlerDecision, CallbackPeer};
use rvoip_sip::api::events::{CallId, Event};
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::api::stream_peer::EventReceiver;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::state_table::{Action, EventType, Role, StateKey, YamlTableLoader};
use rvoip_sip::types::CallState;
use rvoip_sip::SipTraceConfig;
use tokio::net::UdpSocket;
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

#[derive(Clone, Copy)]
enum DirectResolutionMode {
    Accept,
    Reject,
    Redirect,
    DeferThenReject,
}

struct DirectResolutionHandler {
    mode: DirectResolutionMode,
}

#[async_trait::async_trait]
impl CallHandler for DirectResolutionHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        match self.mode {
            DirectResolutionMode::Accept => {
                let _ = call.accept().await;
            }
            DirectResolutionMode::Reject => {
                call.reject(486, "Busy Here");
            }
            DirectResolutionMode::Redirect => {
                let _ = call.redirect_to("sip:alternate@127.0.0.1").await;
            }
            DirectResolutionMode::DeferThenReject => {
                let guard = call.defer(Duration::from_millis(10));
                guard.reject(503, "Service Unavailable");
            }
        }
        CallHandlerDecision::Reject {
            status: 486,
            reason: "Busy Here".to_string(),
        }
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

fn run_on_two_mib_worker_stack<F>(scenario: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_stack_size(2 * 1024 * 1024)
        .enable_all()
        .build()
        .expect("two MiB SIP regression runtime");
    let task = runtime.spawn(scenario);
    runtime
        .block_on(task)
        .expect("SIP regression scenario completed on a two MiB worker stack");
}

async fn wait_for_answer_without_failure(
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
            Ok(Some(Event::CallFailed { call_id, .. })) if &call_id == target_call_id => {
                return false;
            }
            Ok(Some(Event::CallCancelled { call_id })) if &call_id == target_call_id => {
                return false;
            }
            Ok(Some(_)) => continue,
        }
    }
}

#[test]
fn incoming_call_auto_accept_transition_sends_200_without_180_or_accept_event() {
    let table = YamlTableLoader::load_embedded_default().expect("default state table loads");
    let key = StateKey {
        role: Role::UAS,
        state: CallState::Idle,
        event: EventType::IncomingCallAutoAccept {
            from: String::new(),
            sdp: None,
        },
    };

    let transition = table
        .get_transition(&key)
        .expect("UAS Idle + IncomingCallAutoAccept transition must exist");

    assert_eq!(transition.next_state, Some(CallState::Answering));
    assert!(transition.actions.contains(&Action::CreateMediaSession));
    assert!(transition.actions.contains(&Action::StoreRemoteSDP));
    assert!(transition.actions.contains(&Action::GenerateLocalSDP));
    assert!(transition.actions.contains(&Action::NegotiateSDPAsUAS));
    assert!(transition
        .actions
        .contains(&Action::SendSIPResponse(200, "OK".to_string())));
    assert!(!transition
        .actions
        .contains(&Action::SendSIPResponse(180, "Ringing".to_string())));
    assert!(
        transition.publish_events.is_empty(),
        "app observation is published by the handler only after the 200 OK path completes"
    );
}

#[test]
fn fast_auto_accept_answers_before_callback_decision_runs() {
    run_on_two_mib_worker_stack(async move {
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

        let client = UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("raw SIP client");
        let client_port = client.local_addr().expect("raw SIP client address").port();
        let invite = format!(
            "INVITE sip:bob@127.0.0.1:{bob_port} SIP/2.0\r\n\
             Via: SIP/2.0/UDP 127.0.0.1:{client_port};branch=z9hG4bK-two-mib;rport\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:alice@127.0.0.1:{client_port}>;tag=two-mib-client\r\n\
             To: <sip:bob@127.0.0.1:{bob_port}>\r\n\
             Call-ID: two-mib-inbound@127.0.0.1\r\n\
             CSeq: 1 INVITE\r\n\
             Contact: <sip:alice@127.0.0.1:{client_port}>\r\n\
             Content-Length: 0\r\n\r\n"
        );
        client
            .send_to(invite.as_bytes(), format!("127.0.0.1:{bob_port}"))
            .await
            .expect("send raw inbound INVITE");

        tokio::time::timeout(Duration::from_secs(8), started_rx.recv())
            .await
            .expect("callback should start")
            .expect("callback start channel should remain open");

        let mut packet = vec![0_u8; 65_535];
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                let (length, _) = client
                    .recv_from(&mut packet)
                    .await
                    .expect("receive raw SIP response");
                let response = String::from_utf8_lossy(&packet[..length]);
                if response.starts_with("SIP/2.0 200 ") {
                    return;
                }
                assert!(
                    !response.starts_with("SIP/2.0 4")
                        && !response.starts_with("SIP/2.0 5")
                        && !response.starts_with("SIP/2.0 6"),
                    "fast inbound auto-accept returned an error: {response}"
                );
            }
        })
        .await
        .expect("raw inbound INVITE received 200 OK on a two MiB worker stack");

        release_callback.notify_one();
        tokio::time::sleep(Duration::from_millis(200)).await;
        bob_shutdown.shutdown();
        let _ = tokio::time::timeout(Duration::from_secs(2), bob_task).await;
    });
}

#[test]
fn fast_auto_accept_direct_resolution_methods_are_observational_noops() {
    run_on_two_mib_worker_stack(async move {
        let cases = [
            DirectResolutionMode::Accept,
            DirectResolutionMode::Reject,
            DirectResolutionMode::Redirect,
            DirectResolutionMode::DeferThenReject,
        ];

        for (idx, mode) in cases.into_iter().enumerate() {
            let alice_port = 18030 + (idx as u16 * 2);
            let bob_port = alice_port + 1;

            let bob_config = traced_config(&format!("fast-bob-direct-{idx}"), bob_port)
                .with_auto_180_ringing(false)
                .with_incoming_call_channel_capacity(4)
                .with_global_event_channel_capacity(4)
                .with_fast_auto_accept_incoming_calls(true);
            let bob = CallbackPeer::new(DirectResolutionHandler { mode }, bob_config)
                .await
                .expect("bob CallbackPeer::new");
            let bob_shutdown = bob.shutdown_handle();
            let bob_task = tokio::spawn(async move {
                let _ = bob.run().await;
            });
            tokio::time::sleep(Duration::from_millis(200)).await;

            let alice = UnifiedCoordinator::new(traced_config(
                &format!("fast-alice-direct-{idx}"),
                alice_port,
            ))
            .await
            .expect("alice UnifiedCoordinator::new");
            let mut alice_events = alice.events().await.expect("alice events");
            tokio::time::sleep(Duration::from_millis(150)).await;

            let cleanup_before = cleanup_session_diag::cleaned_total();
            let call_id = alice
                .invite(
                    Some(format!("sip:alice@127.0.0.1:{alice_port}")),
                    format!("sip:bob@127.0.0.1:{bob_port}"),
                )
                .send()
                .await
                .expect("invite send");

            assert!(
                wait_for_answer_without_failure(
                    &mut alice_events,
                    &call_id,
                    Duration::from_secs(8)
                )
                .await,
                "fast auto-accept should win over direct app resolution method in case {idx}"
            );

            alice.bye(&call_id).send().await.expect("alice bye");
            let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
            while cleanup_session_diag::cleaned_total() <= cleanup_before
                && tokio::time::Instant::now() < deadline
            {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            assert!(
                cleanup_session_diag::cleaned_total() > cleanup_before,
                "fast auto-accept ACK/BYE cleanup should be delivered in case {idx}"
            );

            bob_shutdown.shutdown();
            let _ = tokio::time::timeout(Duration::from_secs(2), bob_task).await;
        }
    });
}
