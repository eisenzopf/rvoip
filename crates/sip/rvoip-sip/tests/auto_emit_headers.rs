//! SIP_API_DESIGN_2 §10 verification #30 — `Config.auto_emit_extra_headers`
//! is applied to internally-emitted teardown messages without
//! application code.
//!
//! Today the test exercises the BYE side: Alice configures
//! `auto_emit_extra_headers = [X-AutoEmit: trace]`, establishes a
//! call with Bob, and then calls `coord.hangup(...)` (the legacy
//! teardown path, which routes through `Action::SendBYE` rather than
//! `Action::SendBYEWithOptions`). That action consults
//! `dialog_adapter.auto_emit_extra_headers` when the
//! `pending_bye_options` stash is empty, builds a synthetic
//! `ByeRequestOptions`, and dispatches via the same wire path as
//! application-staged BYEs.

use std::time::Duration;

use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::CallState;
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::{Message, Method, StatusCode};
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::{HeaderAccess, HeaderValue, TypedHeader};
use rvoip_sip_dialog::transaction::utils::response_builders::create_response;
use tokio::net::UdpSocket;
use tokio::sync::oneshot;

const PAIR: (u16, u16) = (17600, 17601);
const AUTO_HEADER_NAME: &str = "X-AutoEmit";
const AUTO_HEADER_VALUE: &str = "operator-trace";

fn cfg_with_auto_emit(name: &str, port: u16) -> Config {
    let mut c = Config::local(name, port);
    c.auto_emit_extra_headers = vec![TypedHeader::Other(
        HeaderName::Other(AUTO_HEADER_NAME.to_string()),
        HeaderValue::Raw(AUTO_HEADER_VALUE.as_bytes().to_vec()),
    )];
    c
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn config_auto_emit_extra_headers_stamps_legacy_bye() {
    let _ = tracing_subscriber::fmt::try_init();
    let (alice_port, bob_port) = PAIR;

    // A raw capture UAS keeps the assertion independent of trace redaction.
    // It establishes one dialog, then reports the parsed BYE header value from
    // the actual datagram before answering 200.
    let uas = UdpSocket::bind(("127.0.0.1", bob_port))
        .await
        .expect("capture UAS bind");
    let (bye_tx, bye_rx) = oneshot::channel();
    let uas_task = tokio::spawn(async move {
        let mut bye_tx = Some(bye_tx);
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
                        to.set_tag("auto-emit-capture-uas");
                    }
                    response.headers.push(TypedHeader::Other(
                        HeaderName::Contact,
                        HeaderValue::Raw(format!("<sip:bob@127.0.0.1:{bob_port}>").into_bytes()),
                    ));
                    uas.send_to(&Message::Response(response).to_bytes(), peer)
                        .await
                        .expect("send INVITE response");
                }
                Method::Ack => {}
                Method::Bye => {
                    if let Some(sender) = bye_tx.take() {
                        let _ = sender
                            .send(request.raw_header_value(&HeaderName::Other(
                                AUTO_HEADER_NAME.to_string(),
                            )));
                    }
                    let response = create_response(&request, StatusCode::Ok);
                    uas.send_to(&Message::Response(response).to_bytes(), peer)
                        .await
                        .expect("send BYE response");
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

    // Alice carries the auto-emit header.
    let alice = UnifiedCoordinator::new(cfg_with_auto_emit("alice-ae", alice_port))
        .await
        .expect("alice");
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Establish a call.
    let target = format!("sip:bob@127.0.0.1:{bob_port}");
    let session_id = alice
        .invite(Some("sip:alice@127.0.0.1".to_string()), &target)
        .send()
        .await
        .expect("invite");

    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if matches!(alice.get_state(&session_id).await, Ok(CallState::Active)) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("alice never reached Active");

    // Trigger BYE via the legacy hangup path (no pending_bye_options
    // staged — exactly the path the auto-emit fallback targets).
    alice.hangup(&session_id).await.expect("hangup");

    let captured_value = tokio::time::timeout(Duration::from_secs(8), bye_rx)
        .await
        .expect("capture UAS did not receive BYE")
        .expect("capture UAS sender ended");
    assert_eq!(captured_value.as_deref(), Some(AUTO_HEADER_VALUE));

    uas_task.await.expect("capture UAS task");
    alice
        .shutdown_gracefully(Some(Duration::from_secs(1)))
        .await
        .expect("alice shutdown");
}
