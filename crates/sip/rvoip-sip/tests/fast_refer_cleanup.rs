//! Regression for a peer that terminates the original dialog while a
//! successful REFER dispatch is still unwinding.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rvoip_sip::adapters::media_adapter::cleanup_session_diag;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::{CallState, Event};
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::{Message, Method, StatusCode};
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::{HeaderAccess, HeaderValue, TypedHeader};
use rvoip_sip_dialog::transaction::utils::response_builders::create_response;
use serial_test::serial;
use tokio::net::UdpSocket;
use tokio::sync::oneshot;

const CALLER_PORT: u16 = 17_603;

fn caller_config() -> Config {
    let mut config = Config::local("fast-refer-caller", CALLER_PORT);
    config.media_port_start = 27_700;
    config.media_port_end = 27_800;
    config
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn fast_remote_bye_keeps_successful_refer_dispatch_from_resurrecting_session() {
    let uas = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("capture UAS bind");
    let uas_addr = uas.local_addr().expect("capture UAS address");
    let uas_port = uas_addr.port();
    let (bye_response_tx, bye_response_rx) = oneshot::channel();
    let refer_count = Arc::new(AtomicUsize::new(0));
    let task_refer_count = Arc::clone(&refer_count);

    let uas_task = tokio::spawn(async move {
        let mut bye_response_tx = Some(bye_response_tx);
        let mut packet = vec![0u8; 8_192];
        loop {
            let (bytes, peer) = uas.recv_from(&mut packet).await.expect("UAS receive");
            match parse_message(&packet[..bytes]).expect("parse captured message") {
                Message::Request(request) => match request.method() {
                    Method::Invite => {
                        let mut response = create_response(&request, StatusCode::Ok);
                        if let Some(TypedHeader::To(to)) = response
                            .headers
                            .iter_mut()
                            .find(|header| matches!(header, TypedHeader::To(_)))
                        {
                            to.set_tag("fast-refer-uas");
                        }
                        response.headers.push(TypedHeader::Other(
                            HeaderName::Contact,
                            HeaderValue::Raw(
                                format!("<sip:callee@127.0.0.1:{uas_port}>").into_bytes(),
                            ),
                        ));
                        uas.send_to(&Message::Response(response).to_bytes(), peer)
                            .await
                            .expect("send INVITE 200");
                    }
                    Method::Ack => {}
                    Method::Refer => {
                        task_refer_count.fetch_add(1, Ordering::Relaxed);
                        let from = request
                            .raw_header_value(&HeaderName::From)
                            .expect("REFER From");
                        let to = request.raw_header_value(&HeaderName::To).expect("REFER To");
                        let call_id = request
                            .raw_header_value(&HeaderName::CallId)
                            .expect("REFER Call-ID");
                        let bye = format!(
                            "BYE sip:fast-refer-caller@{peer} SIP/2.0\r\n\
                             Via: SIP/2.0/UDP {uas_addr};branch=z9hG4bK-fast-refer-bye;rport\r\n\
                             From: {to}\r\n\
                             To: {from}\r\n\
                             Call-ID: {call_id}\r\n\
                             CSeq: 2 BYE\r\n\
                             Max-Forwards: 70\r\n\
                             Content-Length: 0\r\n\r\n"
                        );
                        uas.send_to(bye.as_bytes(), peer)
                            .await
                            .expect("send immediate remote BYE");

                        // Keep REFER's transaction open long enough for the
                        // inbound BYE to retire the exact session first.
                        tokio::time::sleep(Duration::from_millis(25)).await;
                        let response = create_response(&request, StatusCode::Accepted);
                        uas.send_to(&Message::Response(response).to_bytes(), peer)
                            .await
                            .expect("send REFER 202");
                    }
                    Method::Bye | Method::Cancel => {
                        let response = create_response(&request, StatusCode::Ok);
                        uas.send_to(&Message::Response(response).to_bytes(), peer)
                            .await
                            .expect("send teardown response");
                    }
                    _ => {}
                },
                Message::Response(response)
                    if response
                        .cseq()
                        .is_some_and(|cseq| cseq.method == Method::Bye) =>
                {
                    if let Some(sender) = bye_response_tx.take() {
                        let _ = sender.send(());
                    }
                    return;
                }
                Message::Response(_) => {}
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
    tokio::time::timeout(
        Duration::from_secs(5),
        caller
            .refer(&session_id, "sip:replacement@example.test")
            .send(),
    )
    .await
    .expect("REFER dispatch timed out")
    .expect("terminal peer teardown must not turn successful REFER into an error");
    tokio::time::timeout(Duration::from_secs(5), bye_response_rx)
        .await
        .expect("UAS never received the BYE response")
        .expect("UAS task ended before the BYE response");

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
        "fast remote BYE published the wrong terminal event: {terminal:?}"
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
        "fast remote BYE published a duplicate terminal event: {duplicate:?}"
    );
    assert_eq!(
        cleanup_session_diag::cleaned_total(),
        cleanup_before + 1,
        "media cleanup must be exact-once"
    );

    uas_task.await.expect("capture UAS task");
    assert_eq!(
        refer_count.load(Ordering::Relaxed),
        1,
        "REFER must be emitted exactly once"
    );
    caller
        .shutdown_gracefully(Some(Duration::from_secs(1)))
        .await
        .expect("terminal BYE must retire the retained initial-INVITE owner");
}
