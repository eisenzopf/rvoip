//! In-dialog requests follow the dialog route set learned from Record-Route
//! (RFC 3261 §12.1.1, §12.1.2, §12.2.1.1), not the configured outbound proxy.
//!
//! A mock UDP proxy record-routes a call in each direction and captures the
//! BYE the rvoip peer sends when it hangs up.

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use rvoip_sip::api::callback_peer::{CallHandler, CallHandlerDecision, CallbackPeer};
use rvoip_sip::api::events::CallId;
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_dialog::transaction::utils::response_builders::{
    create_ok_response_with_contact_uri, create_response,
};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout};

const SDP: &[u8] = b"v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 35299 RTP/AVP 0\r\n";

async fn reserve_port() -> u16 {
    loop {
        let tcp = TcpListener::bind("127.0.0.1:0").await.expect("TCP bind");
        let addr = tcp.local_addr().expect("TCP address");
        if UdpSocket::bind(addr).await.is_ok() {
            return addr.port();
        }
    }
}

fn outbound_proxy_uri(proxy_port: u16) -> String {
    format!("sip:127.0.0.1:{proxy_port};lr;hop=outbound")
}

fn route_values(request: &Request) -> Vec<String> {
    request
        .headers
        .iter()
        .filter_map(|header| match header {
            TypedHeader::Route(route) => Some(route.to_string()),
            _ => None,
        })
        .flat_map(|value| {
            value
                .split(',')
                .map(|entry| entry.trim().to_string())
                .collect::<Vec<_>>()
        })
        .collect()
}

fn with_sdp(mut response: Response) -> Response {
    response.body = bytes::Bytes::from_static(SDP);
    response.headers.push(TypedHeader::ContentType(
        rvoip_sip_core::types::content_type::ContentType::from_type_subtype("application", "sdp"),
    ));
    response
        .headers
        .retain(|header| !matches!(header, TypedHeader::ContentLength(_)));
    response.headers.push(TypedHeader::ContentLength(
        rvoip_sip_core::types::content_length::ContentLength::new(SDP.len() as u32),
    ));
    response
}

async fn wait_for_bye(rx: &mut mpsc::Receiver<Request>) -> Request {
    timeout(Duration::from_secs(8), rx.recv())
        .await
        .expect("BYE reaches the proxy")
        .expect("proxy task alive")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn uac_bye_uses_the_reversed_record_route_of_the_2xx() {
    let (proxy_port, bye) = uac_call_and_hangup(true).await;
    assert_eq!(
        route_values(&bye),
        vec![
            format!("<sip:127.0.0.1:{proxy_port};lr;hop=caller-side>"),
            format!("<sip:127.0.0.1:{proxy_port};lr;hop=callee-side>"),
        ],
        "BYE must carry the reversed 2xx Record-Route and nothing else"
    );
    assert_eq!(bye.uri().to_string(), "sip:bob@127.0.0.1:9");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn uac_bye_keeps_the_outbound_proxy_without_record_route() {
    let (proxy_port, bye) = uac_call_and_hangup(false).await;
    assert_eq!(
        route_values(&bye),
        vec![format!("<{}>", outbound_proxy_uri(proxy_port))],
        "a dialog without route set still preloads the outbound proxy"
    );
}

async fn uac_call_and_hangup(record_route: bool) -> (u16, Request) {
    let proxy_port = reserve_port().await;
    let alice_port = reserve_port().await;
    let proxy = Arc::new(
        UdpSocket::bind(format!("127.0.0.1:{proxy_port}"))
            .await
            .expect("proxy bind"),
    );
    let (bye_tx, mut bye_rx) = mpsc::channel(1);
    let (ack_tx, mut ack_rx) = mpsc::channel(1);

    let proxy_task = {
        let sock = proxy.clone();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 8192];
            loop {
                let Ok((n, from)) = sock.recv_from(&mut buf).await else {
                    return;
                };
                let Ok(Message::Request(request)) = parse_message(&buf[..n]) else {
                    continue;
                };
                let response = match request.method() {
                    Method::Invite => {
                        let Ok(mut ok) =
                            create_ok_response_with_contact_uri(&request, "sip:bob@127.0.0.1:9")
                        else {
                            continue;
                        };
                        if record_route {
                            ok.headers.push(TypedHeader::RecordRoute(
                                RecordRoute::from_str(&format!(
                                    "<sip:127.0.0.1:{proxy_port};lr;hop=callee-side>, \
                                     <sip:127.0.0.1:{proxy_port};lr;hop=caller-side>"
                                ))
                                .expect("Record-Route"),
                            ));
                        }
                        with_sdp(ok)
                    }
                    Method::Ack => {
                        let _ = ack_tx.try_send(());
                        continue;
                    }
                    Method::Bye => {
                        let _ = bye_tx.try_send(request.clone());
                        create_response(&request, StatusCode::Ok)
                    }
                    _ => create_response(&request, StatusCode::Ok),
                };
                let _ = sock
                    .send_to(&Message::Response(response).to_bytes(), from)
                    .await;
            }
        })
    };

    let mut config = Config::local("alice", alice_port);
    config.outbound_proxy_uri = Some(outbound_proxy_uri(proxy_port));
    let alice = UnifiedCoordinator::new(config)
        .await
        .expect("alice coordinator");
    sleep(Duration::from_millis(150)).await;

    let call_id = alice
        .invite(
            Some("sip:alice@127.0.0.1".to_string()),
            format!("sip:bob@127.0.0.1:{proxy_port}"),
        )
        .send()
        .await
        .expect("INVITE");
    // Without a route set the ACK goes straight to the Contact, not the proxy.
    if record_route {
        timeout(Duration::from_secs(8), ack_rx.recv())
            .await
            .expect("ACK reaches the proxy");
    } else {
        sleep(Duration::from_millis(300)).await;
    }
    sleep(Duration::from_millis(200)).await;

    alice.hangup(&call_id).await.expect("hangup");
    let bye = wait_for_bye(&mut bye_rx).await;
    proxy_task.abort();
    (proxy_port, bye)
}

struct AcceptAndReport(mpsc::Sender<CallId>);

#[async_trait::async_trait]
impl CallHandler for AcceptAndReport {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        let _ = self.0.send(call.call_id.clone()).await;
        CallHandlerDecision::Accept
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn uas_bye_uses_the_record_route_of_the_invite_in_order() {
    let proxy_port = reserve_port().await;
    let bob_port = reserve_port().await;
    let proxy = Arc::new(
        UdpSocket::bind(format!("127.0.0.1:{proxy_port}"))
            .await
            .expect("proxy bind"),
    );

    let (call_tx, mut call_rx) = mpsc::channel(1);
    let mut config = Config::local("bob", bob_port).with_auto_180_ringing(false);
    config.outbound_proxy_uri = Some(outbound_proxy_uri(proxy_port));
    let bob = CallbackPeer::new(AcceptAndReport(call_tx), config)
        .await
        .expect("bob CallbackPeer");
    let coordinator = bob.coordinator().clone();
    let bob_shutdown = bob.shutdown_handle();
    let bob_task = tokio::spawn(async move {
        let _ = bob.run().await;
    });
    sleep(Duration::from_millis(200)).await;

    let (bye_tx, mut bye_rx) = mpsc::channel(1);
    let proxy_task = {
        let sock = proxy.clone();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 8192];
            loop {
                let Ok((n, from)) = sock.recv_from(&mut buf).await else {
                    return;
                };
                match parse_message(&buf[..n]) {
                    Ok(Message::Response(response))
                        if response.status_code() == 200
                            && response.cseq().is_some_and(|c| c.method == Method::Invite) =>
                    {
                        let to_tag = response
                            .to()
                            .and_then(|to| to.tag())
                            .unwrap_or_default()
                            .to_string();
                        let ack = format!(
                            "ACK sip:bob@127.0.0.1:{bob_port} SIP/2.0\r\n\
                             Via: SIP/2.0/UDP 127.0.0.1:{proxy_port};branch=z9hG4bK-rr-uas-ack\r\n\
                             Max-Forwards: 70\r\n\
                             From: <sip:carol@127.0.0.1>;tag=carol-tag\r\n\
                             To: <sip:bob@127.0.0.1:{bob_port}>;tag={to_tag}\r\n\
                             Call-ID: rr-uas@127.0.0.1\r\n\
                             CSeq: 1 ACK\r\n\
                             Content-Length: 0\r\n\r\n"
                        );
                        let _ = sock.send_to(ack.as_bytes(), from).await;
                    }
                    Ok(Message::Request(request)) if request.method() == Method::Bye => {
                        let _ = bye_tx.try_send(request.clone());
                        let ok = create_response(&request, StatusCode::Ok);
                        let _ = sock.send_to(&Message::Response(ok).to_bytes(), from).await;
                    }
                    _ => {}
                }
            }
        })
    };

    let invite = format!(
        "INVITE sip:bob@127.0.0.1:{bob_port} SIP/2.0\r\n\
         Via: SIP/2.0/UDP 127.0.0.1:{proxy_port};branch=z9hG4bK-rr-uas\r\n\
         Max-Forwards: 70\r\n\
         Record-Route: <sip:127.0.0.1:{proxy_port};lr;hop=callee-side>\r\n\
         Record-Route: <sip:127.0.0.1:{proxy_port};lr;hop=caller-side>\r\n\
         From: <sip:carol@127.0.0.1>;tag=carol-tag\r\n\
         To: <sip:bob@127.0.0.1:{bob_port}>\r\n\
         Call-ID: rr-uas@127.0.0.1\r\n\
         CSeq: 1 INVITE\r\n\
         Contact: <sip:carol@127.0.0.1:9>\r\n\
         Content-Type: application/sdp\r\n\
         Content-Length: {}\r\n\r\n{}",
        SDP.len(),
        String::from_utf8_lossy(SDP)
    );
    proxy
        .send_to(invite.as_bytes(), format!("127.0.0.1:{bob_port}"))
        .await
        .expect("INVITE");

    let call_id = timeout(Duration::from_secs(8), call_rx.recv())
        .await
        .expect("incoming call")
        .expect("handler alive");
    sleep(Duration::from_millis(500)).await;

    coordinator.hangup(&call_id).await.expect("hangup");
    let bye = wait_for_bye(&mut bye_rx).await;

    assert_eq!(
        route_values(&bye),
        vec![
            format!("<sip:127.0.0.1:{proxy_port};lr;hop=callee-side>"),
            format!("<sip:127.0.0.1:{proxy_port};lr;hop=caller-side>"),
        ],
        "BYE must carry the INVITE Record-Route in order and nothing else"
    );
    assert_eq!(bye.uri().to_string(), "sip:carol@127.0.0.1:9");

    proxy_task.abort();
    bob_shutdown.shutdown();
    let _ = timeout(Duration::from_secs(2), bob_task).await;
}
