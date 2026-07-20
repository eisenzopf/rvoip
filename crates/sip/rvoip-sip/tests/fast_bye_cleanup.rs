//! Regression for a peer that answers BYE before the initiating state-machine
//! action has unwound.
//!
//! Cleanup is response-driven (with the coordinator's retained exact-release
//! fallback), never a second action in the transition that sends BYE.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rvoip_sip::adapters::media_adapter::cleanup_session_diag;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::{CallState, Event};
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::{Message, Method, StatusCode};
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::{HeaderValue, TypedHeader};
use rvoip_sip_dialog::transaction::utils::response_builders::create_response;
use serial_test::serial;
use tokio::net::UdpSocket;
use tokio::sync::oneshot;

const CALLER_PORT: u16 = 17_602;
const CONCURRENT_CALLER_PORT: u16 = 17_603;

fn caller_config() -> Config {
    let mut config = Config::local("fast-bye-caller", CALLER_PORT);
    config.media_port_start = 27_600;
    config.media_port_end = 27_700;
    config
}

fn concurrent_caller_config() -> Config {
    let mut config = Config::local("concurrent-bye-caller", CONCURRENT_CALLER_PORT);
    config.media_port_start = 27_701;
    config.media_port_end = 27_800;
    config
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn fast_bye_200_keeps_hangup_successful_and_cleans_media_once() {
    let uas = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("capture UAS bind");
    let uas_port = uas.local_addr().expect("capture UAS address").port();
    let (bye_seen_tx, bye_seen_rx) = oneshot::channel();

    let uas_task = tokio::spawn(async move {
        let mut bye_seen_tx = Some(bye_seen_tx);
        let mut packet = vec![0u8; 8_192];
        loop {
            let (bytes, peer) = uas.recv_from(&mut packet).await.expect("UAS receive");
            let Message::Request(request) =
                parse_message(&packet[..bytes]).expect("parse captured request")
            else {
                continue;
            };

            match request.method() {
                Method::Invite => {
                    let mut response = create_response(&request, StatusCode::Ok);
                    if let Some(TypedHeader::To(to)) = response
                        .headers
                        .iter_mut()
                        .find(|header| matches!(header, TypedHeader::To(_)))
                    {
                        to.set_tag("fast-bye-uas");
                    }
                    response.headers.push(TypedHeader::Other(
                        HeaderName::Contact,
                        HeaderValue::Raw(format!("<sip:callee@127.0.0.1:{uas_port}>").into_bytes()),
                    ));
                    uas.send_to(&Message::Response(response).to_bytes(), peer)
                        .await
                        .expect("send INVITE 200");
                }
                Method::Ack => {}
                Method::Bye => {
                    // Send the final response before notifying the test task.
                    // Dialog-core can therefore publish DialogTerminated while
                    // the caller is still awaiting SendBYE.
                    let response = create_response(&request, StatusCode::Ok);
                    uas.send_to(&Message::Response(response).to_bytes(), peer)
                        .await
                        .expect("send immediate BYE 200");
                    if let Some(sender) = bye_seen_tx.take() {
                        let _ = sender.send(());
                    }
                    return;
                }
                _ => {
                    let response = create_response(&request, StatusCode::Ok);
                    uas.send_to(&Message::Response(response).to_bytes(), peer)
                        .await
                        .expect("send generic response");
                }
            }
        }
    });

    let cleanup_before = cleanup_session_diag::cleaned_total();
    let caller = UnifiedCoordinator::new(caller_config())
        .await
        .expect("caller coordinator");
    let target = format!("sip:callee@127.0.0.1:{uas_port}");
    let session_id = caller
        .invite(Some("sip:caller@127.0.0.1".to_string()), &target)
        .send()
        .await
        .expect("INVITE dispatch");

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if matches!(caller.get_state(&session_id).await, Ok(CallState::Active)) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("caller never became active");

    let mut events = caller
        .events_for_session(&session_id)
        .await
        .expect("terminal event receiver");
    tokio::time::timeout(Duration::from_secs(5), caller.hangup(&session_id))
        .await
        .expect("hangup timed out")
        .expect("fast BYE response must not turn hangup into an error");
    tokio::time::timeout(Duration::from_secs(5), bye_seen_rx)
        .await
        .expect("UAS never saw BYE")
        .expect("UAS task ended before BYE");

    let cleanup_wait = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if caller.list_sessions().await.is_empty()
                && cleanup_session_diag::cleaned_total() == cleanup_before + 1
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(
        cleanup_wait.is_ok(),
        "exact terminal cleanup did not complete (sessions={}, cleanup_before={}, cleanup_now={})",
        caller.list_sessions().await.len(),
        cleanup_before,
        cleanup_session_diag::cleaned_total()
    );

    let terminal = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.next().await {
                Some(event @ Event::CallEnded { .. })
                | Some(event @ Event::CallFailed { .. })
                | Some(event @ Event::CallCancelled { .. }) => return event,
                Some(_) => {}
                None => panic!("terminal event stream closed"),
            }
        }
    })
    .await
    .expect("terminal event was not delivered");
    assert!(
        matches!(terminal, Event::CallEnded { .. }),
        "fast local BYE published the wrong terminal event: {terminal:?}"
    );

    let duplicate = tokio::time::timeout(Duration::from_millis(250), async {
        loop {
            match events.next().await {
                Some(event @ Event::CallEnded { .. })
                | Some(event @ Event::CallFailed { .. })
                | Some(event @ Event::CallCancelled { .. }) => return event,
                Some(_) => {}
                None => panic!("terminal event stream closed while checking duplicates"),
            }
        }
    })
    .await;
    assert!(
        duplicate.is_err(),
        "fast local BYE published a duplicate terminal event: {duplicate:?}"
    );

    // Give any duplicate terminal task a scheduling opportunity. A second
    // cleanup would increment this process-local counter even if lower media
    // resources were already absent.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        cleanup_session_diag::cleaned_total(),
        cleanup_before + 1,
        "media cleanup must be exact-once"
    );

    uas_task.await.expect("capture UAS task");
    caller
        .shutdown_gracefully(Some(Duration::from_secs(1)))
        .await
        .expect("terminal BYE must retire the retained initial-INVITE owner");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn aborted_hangup_waiter_does_not_duplicate_exact_bye() {
    let uas = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("concurrent UAS bind");
    let uas_port = uas.local_addr().expect("concurrent UAS address").port();
    let (bye_seen_tx, bye_seen_rx) = oneshot::channel();
    let (release_bye_tx, release_bye_rx) = oneshot::channel();
    let bye_count = Arc::new(AtomicUsize::new(0));
    let uas_bye_count = Arc::clone(&bye_count);

    let uas_task = tokio::spawn(async move {
        let mut bye_seen_tx = Some(bye_seen_tx);
        let mut release_bye_rx = Some(release_bye_rx);
        let mut observe_until = None;
        let mut packet = vec![0u8; 8_192];
        loop {
            let received = match observe_until {
                Some(deadline) => {
                    match tokio::time::timeout_at(deadline, uas.recv_from(&mut packet)).await {
                        Ok(received) => received.expect("concurrent UAS receive"),
                        Err(_) => return,
                    }
                }
                None => uas
                    .recv_from(&mut packet)
                    .await
                    .expect("concurrent UAS receive"),
            };
            let (bytes, peer) = received;
            let Message::Request(request) =
                parse_message(&packet[..bytes]).expect("parse concurrent request")
            else {
                continue;
            };

            match request.method() {
                Method::Invite => {
                    let mut response = create_response(&request, StatusCode::Ok);
                    if let Some(TypedHeader::To(to)) = response
                        .headers
                        .iter_mut()
                        .find(|header| matches!(header, TypedHeader::To(_)))
                    {
                        to.set_tag("concurrent-bye-uas");
                    }
                    response.headers.push(TypedHeader::Other(
                        HeaderName::Contact,
                        HeaderValue::Raw(format!("<sip:callee@127.0.0.1:{uas_port}>").into_bytes()),
                    ));
                    uas.send_to(&Message::Response(response).to_bytes(), peer)
                        .await
                        .expect("send concurrent INVITE 200");
                }
                Method::Ack => {}
                Method::Bye => {
                    let observed = uas_bye_count.fetch_add(1, Ordering::SeqCst) + 1;
                    if observed == 1 {
                        if let Some(sender) = bye_seen_tx.take() {
                            let _ = sender.send(());
                        }
                        release_bye_rx
                            .take()
                            .expect("first BYE release receiver")
                            .await
                            .expect("test must release first BYE response");
                    }
                    let response = create_response(&request, StatusCode::Ok);
                    uas.send_to(&Message::Response(response).to_bytes(), peer)
                        .await
                        .expect("send concurrent BYE 200");
                    observe_until = Some(tokio::time::Instant::now() + Duration::from_millis(250));
                }
                _ => {
                    let response = create_response(&request, StatusCode::Ok);
                    uas.send_to(&Message::Response(response).to_bytes(), peer)
                        .await
                        .expect("send concurrent generic response");
                }
            }
        }
    });

    let caller = UnifiedCoordinator::new(concurrent_caller_config())
        .await
        .expect("concurrent caller coordinator");
    let target = format!("sip:callee@127.0.0.1:{uas_port}");
    let session_id = caller
        .invite(Some("sip:caller@127.0.0.1".to_string()), &target)
        .send()
        .await
        .expect("concurrent INVITE dispatch");

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if matches!(caller.get_state(&session_id).await, Ok(CallState::Active)) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("concurrent caller never became active");

    let first_caller = Arc::clone(&caller);
    let first_session = session_id.clone();
    let first = tokio::spawn(async move { first_caller.hangup(&first_session).await });
    tokio::time::timeout(Duration::from_secs(5), bye_seen_rx)
        .await
        .expect("UAS never saw the owned BYE")
        .expect("concurrent UAS ended before BYE");
    first.abort();
    let first_join = first
        .await
        .expect_err("aborted public hangup waiter completed");
    assert!(
        first_join.is_cancelled(),
        "first public hangup waiter must be cancellation evidence"
    );

    let second_caller = Arc::clone(&caller);
    let second_session = session_id.clone();
    let (second_started_tx, second_started_rx) = oneshot::channel();
    let mut second = tokio::spawn(async move {
        let _ = second_started_tx.send(());
        second_caller.hangup(&second_session).await
    });
    second_started_rx
        .await
        .expect("second hangup task stopped before dispatch");
    assert!(
        tokio::time::timeout(Duration::from_millis(75), &mut second)
            .await
            .is_err(),
        "second hangup crossed the first hangup's exact ownership interval"
    );

    release_bye_tx.send(()).expect("release first BYE response");
    let second_result = tokio::time::timeout(Duration::from_secs(5), second)
        .await
        .expect("second hangup timed out")
        .expect("second hangup task panicked");
    assert!(
        second_result.is_ok(),
        "queued hangup must share the exact owner's successful completion: {second_result:?}"
    );

    uas_task.await.expect("concurrent UAS task");
    assert_eq!(
        bye_count.load(Ordering::SeqCst),
        1,
        "concurrent hangups must emit exactly one BYE"
    );
    assert!(
        caller.list_sessions().await.is_empty(),
        "retained exact hangup task must complete session cleanup"
    );
    #[cfg(feature = "perf-tests")]
    assert_eq!(
        caller.perf_diagnostic_snapshot().await["dialog_adapter"]["outgoing_bye_tx"].as_u64(),
        Some(0),
        "retained exact hangup task must reclaim its outgoing BYE transaction"
    );
    caller
        .shutdown_gracefully(Some(Duration::from_secs(1)))
        .await
        .expect("concurrent BYE must retire the exact session");
}
