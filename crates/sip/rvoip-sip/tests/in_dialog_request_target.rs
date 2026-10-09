//! RFC 3261 §12.2.1.1: every request inside a dialog uses the dialog's
//! remote target (the peer's `Contact`) as its Request-URI, never the
//! peer's From/To address.
//!
//! A raw-UDP caller whose From address points at one socket and whose
//! Contact points at another calls rvoip. rvoip, as the called party, then
//! transfers the call (blind and attended REFER, RFC 3515 / RFC 3891) and
//! sends INFO and NOTIFY. Every one of them must arrive at the Contact
//! socket with the Contact URI as Request-URI; nothing may reach the From
//! address. Before the fix REFER, INFO and NOTIFY were addressed to the From
//! URI, so a transfer from a carrier-facing leg went to the wrong host.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;
use tokio::time::timeout;

use rvoip_sip::{Config, StreamPeer};
use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

const WAIT: Duration = Duration::from_secs(10);

async fn recv_message(sock: &UdpSocket, within: Duration) -> Option<Message> {
    let mut buf = vec![0u8; 16384];
    let deadline = tokio::time::Instant::now() + within;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let (n, _) = timeout(remaining, sock.recv_from(&mut buf))
            .await
            .ok()?
            .ok()?;
        if let Ok(message) = parse_message(&buf[..n]) {
            return Some(message);
        }
    }
}

/// Record every request that reaches `sock` and answer it (202 for REFER,
/// 200 otherwise) so rvoip's transactions complete.
async fn record_and_answer(sock: Arc<UdpSocket>, seen: Arc<Mutex<Vec<Request>>>) {
    let mut buf = vec![0u8; 16384];
    loop {
        let Ok((n, from)) = sock.recv_from(&mut buf).await else {
            return;
        };
        let Ok(Message::Request(request)) = parse_message(&buf[..n]) else {
            continue;
        };
        if request.method() == Method::Ack {
            continue;
        }
        let status = if request.method() == Method::Refer {
            StatusCode::Accepted
        } else {
            StatusCode::Ok
        };
        let response = create_response(&request, status);
        seen.lock().unwrap().push(request);
        let _ = sock
            .send_to(&Message::Response(response).to_bytes(), from)
            .await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transfer_info_and_notify_target_the_peer_contact_not_its_from_address() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();

    // rvoip answers.
    let probe = std::net::UdpSocket::bind("127.0.0.1:0").expect("port probe");
    let rvoip_port = probe.local_addr().unwrap().port();
    drop(probe);
    let mut config = Config::local("bob", rvoip_port);
    config.media_port_start = 47400;
    config.media_port_end = 47500;
    let mut peer = StreamPeer::with_config(config).await.expect("peer");
    let rvoip_addr: SocketAddr = format!("127.0.0.1:{rvoip_port}").parse().unwrap();

    // The caller's Contact socket (where it really is) and the host its
    // From address names (where in-dialog requests must NOT go).
    let contact_sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let contact_addr = contact_sock.local_addr().unwrap();
    let aor_sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let aor_addr = aor_sock.local_addr().unwrap();
    let rtp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let contact_uri = format!("sip:alice@{contact_addr}");
    let from_uri = format!("sip:alice@{aor_addr}");

    let sdp = format!(
        "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\n\
         m=audio {} RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\na=sendrecv\r\n",
        rtp.local_addr().unwrap().port()
    );
    let target = format!("sip:bob@{rvoip_addr}");
    let call_id = format!("remote-target-{}", rand::random::<u32>());
    let invite = SimpleRequestBuilder::new(Method::Invite, &target)
        .unwrap()
        .from("Alice", &from_uri, Some("alice-tag"))
        .to("Bob", &target, None)
        .call_id(&call_id)
        .cseq(1)
        .via(
            &contact_addr.to_string(),
            "UDP",
            Some("z9hG4bK-remote-target"),
        )
        .max_forwards(70)
        .contact(&contact_uri, None)
        .content_type("application/sdp")
        .body(sdp.into_bytes())
        .build();
    contact_sock
        .send_to(&Message::Request(invite).to_bytes(), rvoip_addr)
        .await
        .unwrap();

    let incoming = timeout(WAIT, peer.wait_for_incoming())
        .await
        .expect("incoming call in time")
        .expect("incoming call");
    let call = incoming.accept().await.expect("accept");

    let ok = loop {
        match recv_message(&contact_sock, WAIT)
            .await
            .expect("final response")
        {
            Message::Response(response) if response.status_code() >= 200 => break response,
            _ => continue,
        }
    };
    assert_eq!(ok.status_code(), 200);
    let to_tag = ok
        .to()
        .and_then(|to| to.tag().map(str::to_string))
        .expect("2xx To tag");
    let ack = SimpleRequestBuilder::new(Method::Ack, &target)
        .unwrap()
        .from("Alice", &from_uri, Some("alice-tag"))
        .to("Bob", &target, Some(&to_tag))
        .call_id(&call_id)
        .cseq(1)
        .via(
            &contact_addr.to_string(),
            "UDP",
            Some("z9hG4bK-remote-target-ack"),
        )
        .max_forwards(70)
        .build();
    contact_sock
        .send_to(&Message::Request(ack).to_bytes(), rvoip_addr)
        .await
        .unwrap();

    let at_contact = Arc::new(Mutex::new(Vec::new()));
    let at_aor = Arc::new(Mutex::new(Vec::new()));
    let contact_task = tokio::spawn(record_and_answer(
        Arc::clone(&contact_sock),
        Arc::clone(&at_contact),
    ));
    let aor_task = tokio::spawn(record_and_answer(
        Arc::clone(&aor_sock),
        Arc::clone(&at_aor),
    ));

    // The ACK confirms the call before the callee sends anything.
    let deadline = Instant::now() + WAIT;
    while !call.is_active().await {
        assert!(Instant::now() < deadline, "call never became active");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Blind transfer, INFO, NOTIFY and attended transfer from the callee.
    let blind = timeout(WAIT, call.transfer_blind("sip:carol@127.0.0.1:5999")).await;
    let info = timeout(
        WAIT,
        call.send_info("application/dtmf-relay", b"Signal=5\r\nDuration=160\r\n"),
    )
    .await;
    let notify = timeout(
        WAIT,
        call.send_notify(
            "talk",
            Some("talk".to_string()),
            Some("active;expires=60".to_string()),
        ),
    )
    .await;
    let attended = timeout(
        WAIT,
        call.transfer_attended(
            "sip:dave@127.0.0.1:5998",
            "other-call;to-tag=dave-tag;from-tag=carol-tag",
        ),
    )
    .await;

    for (what, result) in [
        ("blind transfer", blind),
        ("INFO", info),
        ("NOTIFY", notify),
        ("attended transfer", attended),
    ] {
        assert!(
            matches!(result, Ok(Ok(()))),
            "{what} was not sent: {result:?}"
        );
    }
    let deadline = Instant::now() + WAIT;
    let wanted = [Method::Refer, Method::Info, Method::Notify, Method::Refer];
    loop {
        let methods: Vec<Method> = at_contact
            .lock()
            .unwrap()
            .iter()
            .map(Request::method)
            .collect();
        let have = |method: &Method, count: usize| {
            methods.iter().filter(|seen| *seen == method).count() >= count
        };
        if have(&Method::Refer, 2) && have(&Method::Info, 1) && have(&Method::Notify, 1) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "expected {wanted:?} at the Contact; got {methods:?} there and {:?} at the From host",
            at_aor
                .lock()
                .unwrap()
                .iter()
                .map(|request| (request.method(), request.uri().to_string()))
                .collect::<Vec<_>>()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    for request in at_contact.lock().unwrap().iter() {
        assert_eq!(
            request.uri().to_string(),
            contact_uri,
            "{} Request-URI is the remote target",
            request.method()
        );
        assert_eq!(
            request.to().map(|to| to.address().uri.to_string()),
            Some(from_uri.clone()),
            "{} keeps the peer's address in To",
            request.method()
        );
    }
    let refer_to: Vec<String> = at_contact
        .lock()
        .unwrap()
        .iter()
        .filter(|request| request.method() == Method::Refer)
        .filter_map(|request| {
            request
                .headers
                .iter()
                .find(|h| h.name() == HeaderName::ReferTo)
                .map(ToString::to_string)
        })
        .collect();
    assert!(
        refer_to.iter().any(|value| value.contains("carol"))
            && refer_to.iter().any(|value| value.contains("dave")),
        "both transfers reached the Contact: {refer_to:?}"
    );
    assert!(
        at_aor.lock().unwrap().is_empty(),
        "nothing may be sent to the From address: {:?}",
        at_aor
            .lock()
            .unwrap()
            .iter()
            .map(Request::method)
            .collect::<Vec<_>>()
    );

    contact_task.abort();
    aor_task.abort();
    peer.shutdown().await.expect("shutdown");
}
