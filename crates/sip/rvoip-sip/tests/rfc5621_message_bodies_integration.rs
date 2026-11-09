//! RFC 5621 message bodies end to end: a running endpoint takes the session
//! description out of a multipart INVITE body, answers it, and rejects a body
//! with a required part it does not understand.
//!
//! Requests are written by hand and sent over UDP; their top Via carries the
//! test socket's port, so responses come back to it.

use std::time::Duration;

use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip_core::types::headers::TypedHeader;
use rvoip_sip_core::{parse_message_with_mode, Message, ParseMode, Response};
use tokio::net::UdpSocket;

const PORT: u16 = 16440;

const SDP: &str = "v=0\r\n\
o=carrier 2890844526 2890844526 IN IP4 127.0.0.1\r\n\
s=-\r\n\
c=IN IP4 127.0.0.1\r\n\
t=0 0\r\n\
m=audio 40000 RTP/AVP 0\r\n\
a=rtpmap:0 PCMU/8000\r\n";

/// An ISUP IAM-like payload: binary, not UTF-8.
const ISUP: &[u8] = &[0x01, 0x00, 0x60, 0x01, 0x0a, 0x00, 0x02, 0xff, 0xfe, 0x80];

fn invite(port: u16, local: u16, call_id: &str, content_type: &str, body: &[u8]) -> Vec<u8> {
    let head = format!(
        "INVITE sip:uas@127.0.0.1:{port} SIP/2.0\r\n\
         Via: SIP/2.0/UDP 127.0.0.1:{local};branch=z9hG4bK-{call_id}\r\n\
         Max-Forwards: 70\r\n\
         From: <sip:carrier@127.0.0.1:{local}>;tag=c-{call_id}\r\n\
         To: <sip:uas@127.0.0.1:{port}>\r\n\
         Call-ID: {call_id}@rfc5621\r\n\
         CSeq: 1 INVITE\r\n\
         Contact: <sip:carrier@127.0.0.1:{local}>\r\n\
         MIME-Version: 1.0\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {}\r\n\r\n",
        body.len()
    );
    [head.as_bytes(), body].concat()
}

/// SDP plus ISUP in `multipart/mixed`, the ISUP part with `isup_disposition`.
fn sdp_and_isup(isup_disposition: &str) -> Vec<u8> {
    [
        b"--unique-boundary-1\r\nContent-Type: application/sdp\r\n\r\n",
        SDP.as_bytes(),
        b"\r\n--unique-boundary-1\r\nContent-Type: application/isup;version=itu-t92+\r\n",
        isup_disposition.as_bytes(),
        b"\r\n",
        ISUP,
        b"\r\n--unique-boundary-1--\r\n",
    ]
    .concat()
}

/// The first final response to `call_id`, which must be valid SIP.
async fn final_response(socket: &UdpSocket, call_id: &str) -> Response {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut datagram = vec![0u8; 65535];
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let (n, _) = tokio::time::timeout(remaining, socket.recv_from(&mut datagram))
            .await
            .unwrap_or_else(|_| panic!("no final response to {call_id}"))
            .expect("receive");
        let message = parse_message_with_mode(&datagram[..n], ParseMode::Strict)
            .unwrap_or_else(|error| panic!("response is not valid SIP ({error})"));
        if let Message::Response(response) = message {
            let ours = response
                .call_id()
                .is_some_and(|id| id.value().starts_with(call_id));
            if ours && response.status_code() >= 200 {
                return response;
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multipart_invite_bodies_follow_rfc5621() {
    let coordinator = UnifiedCoordinator::new(Config::local("rfc5621", PORT))
        .await
        .expect("coordinator");
    let mut events = coordinator.events().await.expect("events");
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let local = socket.local_addr().unwrap().port();
    tokio::time::sleep(Duration::from_millis(200)).await;

    // SDP + ISUP marked optional: the call is offered with the SDP part only,
    // and the answer is SDP.
    socket
        .send_to(
            &invite(
                PORT,
                local,
                "mixed",
                "multipart/mixed;boundary=unique-boundary-1",
                &sdp_and_isup("Content-Disposition: signal;handling=optional\r\n"),
            ),
            ("127.0.0.1", PORT),
        )
        .await
        .unwrap();
    let incoming = tokio::time::timeout(
        Duration::from_secs(10),
        coordinator.next_incoming_call(&mut events),
    )
    .await
    .expect("incoming call")
    .expect("events")
    .expect("an incoming call");
    assert_eq!(
        incoming.sdp.as_deref(),
        Some(SDP),
        "the offer is the SDP part"
    );
    let accepted = tokio::spawn(incoming.accept());
    let answer = final_response(&socket, "mixed").await;
    assert_eq!(answer.status_code(), 200);
    let to_tag = answer
        .to()
        .and_then(|to| to.tag())
        .expect("To tag")
        .to_string();
    let ack = format!(
        "ACK sip:uas@127.0.0.1:{PORT} SIP/2.0\r\n\
         Via: SIP/2.0/UDP 127.0.0.1:{local};branch=z9hG4bK-mixed-ack\r\n\
         Max-Forwards: 70\r\n\
         From: <sip:carrier@127.0.0.1:{local}>;tag=c-mixed\r\n\
         To: <sip:uas@127.0.0.1:{PORT}>;tag={to_tag}\r\n\
         Call-ID: mixed@rfc5621\r\n\
         CSeq: 1 ACK\r\n\
         Content-Length: 0\r\n\r\n"
    );
    socket
        .send_to(ack.as_bytes(), ("127.0.0.1", PORT))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), accepted)
        .await
        .expect("accept completes")
        .expect("accept task")
        .expect("accept");
    assert!(answer
        .headers
        .iter()
        .any(|h| matches!(h, TypedHeader::ContentType(ct) if ct.to_string() == "application/sdp")));
    let answer_sdp = std::str::from_utf8(answer.body()).expect("SDP answer");
    assert!(answer_sdp.contains("m=audio "), "{answer_sdp}");
    assert!(
        !answer_sdp.contains("m=audio 0 "),
        "the audio stream is accepted: {answer_sdp}"
    );

    // SDP + ISUP whose handling is required (the default): 415, listing the
    // multipart types the UA accepts.
    socket
        .send_to(
            &invite(
                PORT,
                local,
                "required",
                "multipart/mixed;boundary=unique-boundary-1",
                &sdp_and_isup("Content-Disposition: signal\r\n"),
            ),
            ("127.0.0.1", PORT),
        )
        .await
        .unwrap();
    let rejected = final_response(&socket, "required").await;
    assert_eq!(rejected.status_code(), 415);
    let accept = rejected
        .headers
        .iter()
        .find(|h| matches!(h, TypedHeader::Accept(_)))
        .map(ToString::to_string)
        .expect("415 carries Accept");
    assert!(
        accept.contains("application/sdp") && accept.contains("multipart/mixed"),
        "{accept}"
    );

    // multipart/alternative: an alternative the UA does not understand, then
    // SDP. The offer is the SDP alternative.
    let alternative = [
        &b"--alt\r\nContent-Type: application/x-unknown-session\r\n\r\nunknown\r\n"[..],
        b"--alt\r\nContent-Type: application/sdp\r\n\r\n",
        SDP.as_bytes(),
        b"\r\n--alt--\r\n",
    ]
    .concat();
    socket
        .send_to(
            &invite(
                PORT,
                local,
                "alternative",
                "multipart/alternative;boundary=alt",
                &alternative,
            ),
            ("127.0.0.1", PORT),
        )
        .await
        .unwrap();
    let incoming = tokio::time::timeout(
        Duration::from_secs(10),
        coordinator.next_incoming_call(&mut events),
    )
    .await
    .expect("incoming call")
    .expect("events")
    .expect("an incoming call");
    assert_eq!(incoming.sdp.as_deref(), Some(SDP));

    coordinator.shutdown();
}
