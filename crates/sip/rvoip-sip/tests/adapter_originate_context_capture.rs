//! End-to-end capture-UAS coverage for the SIP adapter's staged originate
//! context. The same ordered application headers and per-call From identity
//! must survive the initial INVITE and its authenticated retry.

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use rvoip_core::adapter::{AdapterEvent, ConnectionAdapter, OriginateRequest};
use rvoip_core::capability::CapabilityDescriptor;
use rvoip_core::connection::{Direction, Transport};
use rvoip_core::ids::{ParticipantId, SessionId};
use rvoip_sip::api::unified::{Config as SipConfig, UnifiedCoordinator};
use rvoip_sip::{SipAdapter, SipInitialHeaders, SipOriginateContext};
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::{HeaderAccess, HeaderValue};
use rvoip_sip_dialog::transaction::utils::response_builders::create_response;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

#[derive(Debug)]
struct CapturedInvite {
    raw: String,
    call_id: String,
    from: String,
    has_authorization: bool,
}

#[derive(Debug)]
struct CapturedDialogRequest {
    method: Method,
    request_uri: String,
    call_id: String,
    to: String,
}

fn ordered_application_headers(raw: &str) -> Vec<(String, String)> {
    raw.lines()
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            (name.eq_ignore_ascii_case("X-Context-Order")
                || name.eq_ignore_ascii_case("X-Context-Middle"))
            .then(|| (name.to_ascii_lowercase(), value.trim().to_string()))
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn staged_context_survives_initial_and_authenticated_retry_on_wire() {
    let uas = Arc::new(
        UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("capture UAS bind"),
    );
    let uas_addr = uas.local_addr().expect("capture UAS address");
    // Keep signaling and media on distinct sockets. Once the 200 response
    // carries valid SDP the UAC may emit RTP immediately; advertising the SIP
    // capture port as the RTP target would feed binary RTP into the SIP parser
    // and terminate the fixture before teardown can be observed.
    let _media_sink = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("capture media sink bind");
    let media_port = _media_sink
        .local_addr()
        .expect("capture media sink address")
        .port();
    let (captured_tx, mut captured_rx) = mpsc::unbounded_channel();
    let (dialog_tx, mut dialog_rx) = mpsc::unbounded_channel();
    let uas_task_socket = Arc::clone(&uas);
    let uas_task = tokio::spawn(async move {
        const UAS_TO_TAG: &str = "context-capture-uas";
        let mut packet = vec![0u8; 16_384];
        let mut invite_count = 0usize;
        loop {
            let (bytes, peer) = uas_task_socket
                .recv_from(&mut packet)
                .await
                .expect("capture UAS receive");
            let Message::Request(request) =
                parse_message(&packet[..bytes]).expect("parse captured request")
            else {
                continue;
            };
            match request.method() {
                Method::Invite => {
                    let raw = String::from_utf8(packet[..bytes].to_vec())
                        .expect("captured INVITE is UTF-8");
                    captured_tx
                        .send(CapturedInvite {
                            raw,
                            call_id: request
                                .call_id()
                                .map(|call_id| call_id.value())
                                .expect("INVITE Call-ID"),
                            from: request
                                .raw_header_value(&HeaderName::From)
                                .expect("INVITE From"),
                            has_authorization: request
                                .raw_header_value(&HeaderName::Authorization)
                                .is_some(),
                        })
                        .expect("capture receiver");

                    let responses = if invite_count == 0 {
                        let mut response = create_response(&request, StatusCode::Unauthorized);
                        if let Some(TypedHeader::To(to)) = response
                            .headers
                            .iter_mut()
                            .find(|header| matches!(header, TypedHeader::To(_)))
                        {
                            to.set_tag(UAS_TO_TAG);
                        }
                        response.headers.push(TypedHeader::Other(
                            HeaderName::WwwAuthenticate,
                            HeaderValue::Raw(
                                br#"Digest realm="context.test", nonce="context-nonce", algorithm=MD5, qop="auth""#
                                    .to_vec(),
                            ),
                        ));
                        vec![response]
                    } else {
                        let mut ringing = create_response(&request, StatusCode::Ringing);
                        if let Some(TypedHeader::To(to)) = ringing
                            .headers
                            .iter_mut()
                            .find(|header| matches!(header, TypedHeader::To(_)))
                        {
                            to.set_tag(UAS_TO_TAG);
                        }

                        let mut response = create_response(&request, StatusCode::Ok);
                        if let Some(TypedHeader::To(to)) = response
                            .headers
                            .iter_mut()
                            .find(|header| matches!(header, TypedHeader::To(_)))
                        {
                            to.set_tag(UAS_TO_TAG);
                        }
                        response.headers.push(TypedHeader::Contact(
                            Contact::from_str(&format!("<sip:capture@{uas_addr}>"))
                                .expect("valid capture Contact"),
                        ));
                        response.headers.push(TypedHeader::ContentType(
                            rvoip_sip_core::types::ContentType::sdp(),
                        ));
                        response.body = bytes::Bytes::from(format!(
                            "v=0\r\no=capture 1 1 IN IP4 127.0.0.1\r\ns=context-capture\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio {} RTP/AVP 0 101\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:101 telephone-event/8000\r\na=fmtp:101 0-15\r\na=sendrecv\r\n",
                            media_port
                        ));
                        response
                            .headers
                            .retain(|header| !matches!(header, TypedHeader::ContentLength(_)));
                        response.headers.push(TypedHeader::ContentLength(
                            rvoip_sip_core::types::ContentLength::new(response.body.len() as u32),
                        ));
                        vec![ringing, response]
                    };
                    invite_count += 1;
                    for response in responses {
                        uas_task_socket
                            .send_to(&Message::Response(response).to_bytes(), peer)
                            .await
                            .expect("capture UAS response");
                    }
                }
                Method::Bye | Method::Cancel => {
                    dialog_tx
                        .send(CapturedDialogRequest {
                            method: request.method().clone(),
                            request_uri: request.uri().to_string(),
                            call_id: request
                                .call_id()
                                .map(|call_id| call_id.value())
                                .expect("teardown Call-ID"),
                            to: request
                                .raw_header_value(&HeaderName::To)
                                .expect("teardown To"),
                        })
                        .expect("dialog capture receiver");
                    let response = create_response(&request, StatusCode::Ok);
                    uas_task_socket
                        .send_to(&Message::Response(response).to_bytes(), peer)
                        .await
                        .expect("capture UAS teardown response");
                }
                Method::Ack => {
                    dialog_tx
                        .send(CapturedDialogRequest {
                            method: Method::Ack,
                            request_uri: request.uri().to_string(),
                            call_id: request
                                .call_id()
                                .map(|call_id| call_id.value())
                                .expect("ACK Call-ID"),
                            to: request.raw_header_value(&HeaderName::To).expect("ACK To"),
                        })
                        .expect("dialog capture receiver");
                }
                _ => {}
            }
        }
    });

    let coordinator = UnifiedCoordinator::new(SipConfig::local("context-capture-uac", 0))
        .await
        .expect("coordinator");
    let adapter = SipAdapter::new(Arc::clone(&coordinator))
        .await
        .expect("adapter");
    let mut adapter_events = ConnectionAdapter::subscribe_events(adapter.as_ref());
    let context = SipOriginateContext::new()
        .with_from_uri("sip:private-caller@context.test")
        .expect("valid per-call From")
        .with_auth(rvoip_sip::auth::SipClientAuth::digest(
            "context-user",
            "context-password",
        ))
        .expect("bounded Digest auth")
        .with_outbound_proxy(format!("sip:{uas_addr};lr"))
        .expect("bounded per-call proxy")
        .with_initial_headers(
            SipInitialHeaders::new([
                ("X-Context-Order", "first"),
                ("X-Context-Middle", "middle"),
                ("x-context-order", "second"),
            ])
            .expect("ordered application headers"),
        );
    let prepared = <SipAdapter as ConnectionAdapter>::originate(
        adapter.as_ref(),
        OriginateRequest::new(
            SessionId::new(),
            ParticipantId::new(),
            format!("sip:callee@{uas_addr}"),
            Direction::Outbound,
            CapabilityDescriptor::default(),
        )
        .with_transport(Transport::Sip)
        .with_context(context),
    )
    .await
    .expect("dormant route");
    let connection_id = prepared.connection.id.clone();
    let receipt = tokio::time::timeout(
        Duration::from_secs(5),
        <SipAdapter as ConnectionAdapter>::activate_outbound_with_receipt(
            adapter.as_ref(),
            connection_id.clone(),
        ),
    )
    .await
    .expect("activation deadline")
    .expect("activation receipt");
    let initial = tokio::time::timeout(Duration::from_secs(8), captured_rx.recv())
        .await
        .expect("initial INVITE deadline")
        .expect("initial INVITE");
    let retry = tokio::time::timeout(Duration::from_secs(8), captured_rx.recv())
        .await
        .expect("retry INVITE deadline")
        .expect("retry INVITE");
    assert!(
        tokio::time::timeout(Duration::from_millis(1_100), captured_rx.recv())
            .await
            .is_err(),
        "one challenge must produce exactly one authenticated retry beyond SIP T1"
    );
    for invite in [&initial, &retry] {
        assert!(invite.from.contains("sip:private-caller@context.test"));
        assert!(
            invite
                .raw
                .to_ascii_lowercase()
                .contains(&format!("route: <sip:{uas_addr};lr>").to_ascii_lowercase()),
            "initial and authenticated retry retain the exact per-call proxy Route"
        );
        assert_eq!(
            ordered_application_headers(&invite.raw),
            vec![
                ("x-context-order".to_string(), "first".to_string()),
                ("x-context-middle".to_string(), "middle".to_string()),
                ("x-context-order".to_string(), "second".to_string()),
            ]
        );
    }
    assert_eq!(
        initial.from, retry.from,
        "authenticated retry must retain the exact From URI and tag"
    );
    assert!(!initial.has_authorization);
    assert!(retry.has_authorization);
    assert_eq!(initial.call_id, retry.call_id);
    let actual_call_id = receipt
        .external_references()
        .iter()
        .find(|reference| reference.kind() == "sip.call-id")
        .expect("SIP Call-ID receipt")
        .expose_secret();
    assert_eq!(actual_call_id, initial.call_id);

    let challenge_ack = tokio::time::timeout(Duration::from_secs(5), dialog_rx.recv())
        .await
        .expect("challenge ACK deadline")
        .expect("challenge ACK capture");
    assert_eq!(challenge_ack.method, Method::Ack);
    assert_eq!(challenge_ack.call_id, initial.call_id);
    assert_eq!(challenge_ack.request_uri, format!("sip:callee@{uas_addr}"));
    assert!(challenge_ack.to.contains("tag=context-capture-uas"));

    let success_ack = tokio::time::timeout(Duration::from_secs(5), dialog_rx.recv())
        .await
        .expect("success ACK deadline")
        .expect("success ACK capture");
    assert_eq!(success_ack.method, Method::Ack);
    assert_eq!(success_ack.call_id, initial.call_id);
    assert_eq!(success_ack.request_uri, format!("sip:capture@{uas_addr}"));
    assert!(success_ack.to.contains("tag=context-capture-uas"));

    tokio::time::timeout(Duration::from_secs(5), async {
        let mut saw_ringing = false;
        loop {
            match adapter_events.recv().await {
                Some(AdapterEvent::Progress {
                    connection_id: id,
                    status_code: 180,
                    reason,
                    early_media: false,
                }) if id == connection_id && reason == "Ringing" => {
                    saw_ringing = true;
                }
                Some(AdapterEvent::Connected { connection_id: id }) if id == connection_id => {
                    assert!(
                        saw_ringing,
                        "staged 180 progress must be delivered before Connected"
                    );
                    break;
                }
                Some(_) => {}
                None => panic!("adapter event stream closed before Connected"),
            }
        }
    })
    .await
    .expect("Connected event deadline");
    tokio::time::timeout(
        Duration::from_secs(5),
        <SipAdapter as ConnectionAdapter>::end(
            adapter.as_ref(),
            connection_id,
            rvoip_core::adapter::EndReason::Normal,
        ),
    )
    .await
    .expect("teardown deadline")
    .expect("teardown");
    let bye = tokio::time::timeout(Duration::from_secs(5), dialog_rx.recv())
        .await
        .expect("BYE deadline")
        .expect("BYE capture");
    assert_eq!(bye.method, Method::Bye);
    assert_eq!(bye.call_id, initial.call_id);
    assert_eq!(bye.request_uri, format!("sip:capture@{uas_addr}"));
    assert!(bye.to.contains("tag=context-capture-uas"));
    adapter.drain().await.expect("adapter drain");
    coordinator
        .shutdown_gracefully(Some(Duration::from_secs(1)))
        .await
        .expect("coordinator shutdown");
    uas_task.abort();
}
