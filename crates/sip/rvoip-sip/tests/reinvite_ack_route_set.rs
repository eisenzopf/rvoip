//! RFC 3261 §§12.2 and 13.2.2.4: the ACK for a re-INVITE's 2xx follows the
//! dialog's established route set.
//!
//! A raw-UDP "proxy" socket record-routes the initial INVITE and answers it
//! with a Contact on a second socket, so the dialog's route set is the proxy
//! and its remote target is the Contact. A hold re-INVITE then goes through
//! the proxy and is answered with a 2xx that carries no Record-Route (a
//! re-INVITE cannot change the route set). The ACK for that 2xx must still
//! carry the proxy Route and reach the proxy; it must not be sent straight
//! to the Contact as if the dialog had no route set.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::time::timeout;

use rvoip_sip::api::unified::Config;
use rvoip_sip::{CallState, SessionHandle, StreamPeer};
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::{HeaderAccess, HeaderValue};
use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

const STEP: Duration = Duration::from_secs(5);
const UAS_TAG: &str = "reinvite-ack-uas";

async fn next_request(socket: &UdpSocket, within: Duration) -> Option<(Request, SocketAddr)> {
    timeout(within, async {
        let mut packet = vec![0u8; 16_384];
        loop {
            let (size, from) = socket.recv_from(&mut packet).await.expect("recv");
            if let Ok(Message::Request(request)) = parse_message(&packet[..size]) {
                return (request, from);
            }
        }
    })
    .await
    .ok()
}

/// Next request of `method` on `socket`, skipping any other method.
async fn request(socket: &UdpSocket, method: Method, step: &str) -> (Request, SocketAddr) {
    loop {
        let (request, from) = next_request(socket, STEP)
            .await
            .unwrap_or_else(|| panic!("{step}: {method} did not reach the proxy"));
        if request.method() == method {
            return (request, from);
        }
    }
}

fn answer(
    request: &Request,
    contact: SocketAddr,
    record_route: Option<SocketAddr>,
    media_port: u16,
) -> Vec<u8> {
    let mut response = create_response(request, StatusCode::Ok);
    if let Some(TypedHeader::To(to)) = response
        .headers
        .iter_mut()
        .find(|header| matches!(header, TypedHeader::To(_)))
    {
        to.set_tag(UAS_TAG);
    }
    response.headers.push(TypedHeader::Other(
        HeaderName::Contact,
        HeaderValue::Raw(format!("<sip:target@{contact}>").into_bytes()),
    ));
    if let Some(proxy) = record_route {
        response.headers.push(TypedHeader::Other(
            HeaderName::RecordRoute,
            HeaderValue::Raw(format!("<sip:{proxy};lr>").into_bytes()),
        ));
    }
    let direction = if String::from_utf8_lossy(&request.body).contains("a=sendonly") {
        "recvonly"
    } else {
        "sendrecv"
    };
    let version = request.cseq().expect("CSeq").sequence();
    response.headers.push(TypedHeader::ContentType(
        rvoip_sip_core::types::ContentType::sdp(),
    ));
    response.body = bytes::Bytes::from(format!(
        "v=0\r\no=capture 1 {version} IN IP4 127.0.0.1\r\ns=capture\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio {media_port} RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\na={direction}\r\n"
    ));
    response
        .headers
        .retain(|header| !matches!(header, TypedHeader::ContentLength(_)));
    response.headers.push(TypedHeader::ContentLength(
        rvoip_sip_core::types::ContentLength::new(response.body.len() as u32),
    ));
    Message::Response(response).to_bytes().to_vec()
}

fn routes(request: &Request) -> Vec<String> {
    request
        .headers
        .iter()
        .filter_map(|header| match header {
            TypedHeader::Route(route) => Some(route.to_string()),
            _ => None,
        })
        .collect()
}

async fn wait_for_state(call: &SessionHandle, expected: CallState) {
    let reached = timeout(STEP, async {
        loop {
            if matches!(call.state().await, Ok(state) if state == expected) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(
        reached.is_ok(),
        "call did not reach {expected:?}; last state: {:?}",
        call.state().await
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reinvite_ack_keeps_the_established_proxy_route() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();
    let proxy = UdpSocket::bind("127.0.0.1:0").await.expect("proxy socket");
    let contact = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("contact socket");
    let media = UdpSocket::bind("127.0.0.1:0").await.expect("media socket");
    let proxy_addr = proxy.local_addr().unwrap();
    let contact_addr = contact.local_addr().unwrap();
    let media_port = media.local_addr().unwrap().port();
    let expected_route = format!("<sip:{proxy_addr};lr>");

    let uac = StreamPeer::with_config(Config::local("reinvite-ack-uac", 0))
        .await
        .expect("UAC");
    let send = tokio::spawn(uac.invite(format!("sip:target@{proxy_addr}")).send());
    let (invite, from) = request(&proxy, Method::Invite, "initial INVITE").await;
    proxy
        .send_to(
            &answer(&invite, contact_addr, Some(proxy_addr), media_port),
            from,
        )
        .await
        .unwrap();
    let call_id = timeout(STEP, send)
        .await
        .expect("INVITE deadline")
        .expect("INVITE task")
        .expect("INVITE");
    let call = uac.session(&call_id);
    call.wait_for_answered(Some(STEP)).await.expect("answered");
    let (initial_ack, _) = request(&proxy, Method::Ack, "initial ACK").await;
    assert_eq!(routes(&initial_ack), vec![expected_route.clone()]);
    wait_for_state(&call, CallState::Active).await;

    call.hold().await.expect("hold re-INVITE");
    let (reinvite, from) = request(&proxy, Method::Invite, "hold re-INVITE").await;
    assert_eq!(
        reinvite.uri().to_string(),
        format!("sip:target@{contact_addr}"),
        "the re-INVITE targets the remote Contact"
    );
    assert_eq!(routes(&reinvite), vec![expected_route.clone()]);
    // The 2xx to the re-INVITE carries no Record-Route.
    proxy
        .send_to(&answer(&reinvite, contact_addr, None, media_port), from)
        .await
        .unwrap();

    let (ack, _) = request(&proxy, Method::Ack, "re-INVITE ACK").await;
    assert_eq!(
        ack.cseq().expect("ACK CSeq").sequence(),
        reinvite.cseq().expect("re-INVITE CSeq").sequence()
    );
    assert_eq!(ack.uri().to_string(), format!("sip:target@{contact_addr}"));
    assert_eq!(
        routes(&ack),
        vec![expected_route],
        "the re-INVITE ACK keeps the dialog's route set"
    );
    assert!(ack.raw_header_value(&HeaderName::RecordRoute).is_none());
    assert!(
        next_request(&contact, Duration::from_millis(500))
            .await
            .is_none(),
        "nothing bypasses the proxy to reach the Contact directly"
    );
    wait_for_state(&call, CallState::OnHold).await;

    let _ = timeout(STEP, uac.shutdown()).await;
}
