//! RFC 4028 conformance of rvoip-sip as the called party (UAS).
//!
//! A raw-UDP UAC on loopback sends the INVITE, so the test controls exactly
//! which session-timer headers the caller advertises and sees every byte
//! rvoip answers with:
//!
//! 1. A caller that does not advertise `timer` (RFC 4028 §9, Table 2): the
//!    2xx still carries `Session-Expires;refresher=uas` but never
//!    `Require: timer`, rvoip refreshes on its own, and its refresh names
//!    itself with `refresher=uac` (§5/§7.4 — the role is relative to the
//!    refresh transaction). The 2xx to BYE carries no session-timer headers.
//! 2. A caller that advertises `timer` and refreshes itself: the 2xx carries
//!    `Require: timer` and `refresher=uac`, as before.
//! 3. The same caller never refreshes: rvoip, the non-refresher, sends
//!    `BYE` with `Reason: SIP;cause=408` at
//!    `interval - min(32, interval / 3)` (§10), not at the full interval.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;
use tokio::time::timeout;

use rvoip_sip::{Config, StreamPeer};
use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::{HeaderAccess, HeaderValue};
use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

const CALL_TIMEOUT: Duration = Duration::from_secs(10);

struct RawUac {
    sock: UdpSocket,
    peer_addr: SocketAddr,
    local_addr: SocketAddr,
    call_id: String,
    from_tag: String,
    _rtp: UdpSocket,
}

impl RawUac {
    async fn new(peer_addr: SocketAddr) -> Self {
        let sock = UdpSocket::bind("127.0.0.1:0").await.expect("uac bind");
        let local_addr = sock.local_addr().unwrap();
        let rtp = UdpSocket::bind("127.0.0.1:0").await.expect("rtp bind");
        Self {
            sock,
            peer_addr,
            local_addr,
            call_id: format!("rfc4028-uas-{}", rand::random::<u32>()),
            from_tag: format!("uac-{}", rand::random::<u32>()),
            _rtp: rtp,
        }
    }

    fn target(&self) -> String {
        format!("sip:bob@{}", self.peer_addr)
    }

    fn invite(&self, extra: &[(HeaderName, &str)]) -> Request {
        let sdp = format!(
            "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\n\
             m=audio {} RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\na=sendrecv\r\n",
            self._rtp.local_addr().unwrap().port()
        );
        let mut request = SimpleRequestBuilder::new(Method::Invite, &self.target())
            .unwrap()
            .from("Alice", "sip:alice@127.0.0.1", Some(&self.from_tag))
            .to("Bob", &self.target(), None)
            .call_id(&self.call_id)
            .cseq(1)
            .via(
                &self.local_addr.to_string(),
                "UDP",
                Some(&format!("z9hG4bK-inv-{}", rand::random::<u32>())),
            )
            .max_forwards(70)
            .contact(&format!("sip:alice@{}", self.local_addr), None)
            .content_type("application/sdp")
            .body(sdp.into_bytes())
            .build();
        for (name, value) in extra {
            request.headers.push(TypedHeader::Other(
                name.clone(),
                HeaderValue::Raw(value.as_bytes().to_vec()),
            ));
        }
        request
    }

    fn in_dialog(&self, method: Method, cseq: u32, to_tag: &str) -> Request {
        let branch = format!("z9hG4bK-{}-{}", method, rand::random::<u32>());
        SimpleRequestBuilder::new(method, &self.target())
            .unwrap()
            .from("Alice", "sip:alice@127.0.0.1", Some(&self.from_tag))
            .to("Bob", &self.target(), Some(to_tag))
            .call_id(&self.call_id)
            .cseq(cseq)
            .via(&self.local_addr.to_string(), "UDP", Some(&branch))
            .max_forwards(70)
            .build()
    }

    async fn send(&self, message: Message) {
        self.sock
            .send_to(&message.to_bytes(), self.peer_addr)
            .await
            .expect("send");
    }

    async fn recv(&self, wait: Duration) -> Option<Message> {
        let mut buf = vec![0u8; 16384];
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let (n, _) = timeout(remaining, self.sock.recv_from(&mut buf))
                .await
                .ok()?
                .ok()?;
            if let Ok(message) = parse_message(&buf[..n]) {
                return Some(message);
            }
        }
    }

    /// Wait for the final response to the transaction with `cseq_method`,
    /// skipping provisionals and anything else.
    async fn final_response(&self, cseq_method: Method) -> Response {
        let deadline = Instant::now() + CALL_TIMEOUT;
        while Instant::now() < deadline {
            if let Some(Message::Response(response)) = self.recv(Duration::from_secs(1)).await {
                let matches = response
                    .cseq()
                    .is_some_and(|cseq| cseq.method == cseq_method);
                if matches && response.status_code() >= 200 {
                    return response;
                }
            }
        }
        panic!("no final response to {cseq_method}");
    }

    /// Wait for an in-dialog request from rvoip, answering anything else
    /// with nothing (2xx retransmissions are ignored).
    async fn request(&self, method: Method, wait: Duration) -> Option<Request> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return None;
            }
            match self.recv(remaining).await? {
                Message::Request(request) if request.method() == method => return Some(request),
                _ => continue,
            }
        }
    }

    async fn ack(&self, ok: &Response) -> String {
        let to_tag = ok
            .to()
            .and_then(|to| to.tag().map(str::to_string))
            .expect("2xx To tag");
        let ack = self.in_dialog(Method::Ack, 1, &to_tag);
        self.send(Message::Request(ack)).await;
        to_tag
    }
}

async fn answering_peer(session_secs: u32) -> (StreamPeer, SocketAddr) {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();
    let probe = std::net::UdpSocket::bind("127.0.0.1:0").expect("port probe");
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let mut config = Config::local("bob", port);
    let media_base = 44000 + (port % 1000) * 2;
    config.media_port_start = media_base;
    config.media_port_end = media_base + 40;
    config.session_timer_secs = Some(session_secs);
    config.session_timer_min_se = 2;
    let peer = StreamPeer::with_config(config).await.expect("peer");
    (peer, format!("127.0.0.1:{port}").parse().unwrap())
}

async fn answer_one(peer: &mut StreamPeer) {
    let incoming = timeout(CALL_TIMEOUT, peer.wait_for_incoming())
        .await
        .expect("incoming call in time")
        .expect("incoming call");
    incoming.accept().await.expect("accept");
}

fn header(message_headers: &[TypedHeader], name: &HeaderName) -> Vec<String> {
    message_headers
        .iter()
        .filter(|h| &h.name() == name)
        .map(|h| {
            // `TypedHeader` displays as `Name: value`; keep only the value.
            let full = h.to_string();
            full.split_once(':')
                .map_or(full.clone(), |(_, value)| value.trim().to_string())
        })
        .collect()
}

fn requires_timer(response: &Response) -> bool {
    header(&response.headers, &HeaderName::Require)
        .iter()
        .any(|value| value.to_ascii_lowercase().contains("timer"))
}

fn session_expires(headers: &[TypedHeader]) -> Option<String> {
    header(headers, &HeaderName::SessionExpires)
        .into_iter()
        .next()
        .map(|value| value.to_ascii_lowercase().replace(' ', ""))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn caller_without_timer_support_gets_no_require_timer_and_rvoip_refreshes() {
    const SESSION_SECS: u32 = 4;
    let (mut peer, peer_addr) = answering_peer(SESSION_SECS).await;
    let uac = RawUac::new(peer_addr).await;

    // No `Supported: timer`, no Session-Expires.
    uac.send(Message::Request(uac.invite(&[]))).await;
    answer_one(&mut peer).await;
    let ok = uac.final_response(Method::Invite).await;
    assert_eq!(ok.status_code(), 200);
    assert!(
        !requires_timer(&ok),
        "RFC 4028 §9: no Require: timer for a caller without timer support: {:?}",
        header(&ok.headers, &HeaderName::Require)
    );
    let se = session_expires(&ok.headers).expect("2xx still carries Session-Expires");
    assert!(
        se.contains(&format!("{SESSION_SECS}")) && se.contains("refresher=uas"),
        "rvoip must be the refresher: {se}"
    );
    let to_tag = uac.ack(&ok).await;

    // rvoip refreshes at half the interval and names itself, the sender,
    // with refresher=uac.
    let started = Instant::now();
    let update = uac
        .request(
            Method::Update,
            Duration::from_secs(u64::from(SESSION_SECS) + 2),
        )
        .await
        .expect("rvoip refreshes the session");
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(1500) && elapsed < Duration::from_secs(3),
        "refresh at interval / 2, got {elapsed:?}"
    );
    let refresh_se = session_expires(&update.headers).expect("refresh carries Session-Expires");
    assert!(
        refresh_se.contains("refresher=uac"),
        "RFC 4028 §7.4: the refreshing side sends refresher=uac: {refresh_se}"
    );
    uac.send(Message::Response(create_response(&update, StatusCode::Ok)))
        .await;

    // The 2xx to BYE is not a session refresh.
    let bye = uac.in_dialog(Method::Bye, 2, &to_tag);
    uac.send(Message::Request(bye)).await;
    let bye_ok = uac.final_response(Method::Bye).await;
    assert_eq!(bye_ok.status_code(), 200);
    assert!(
        session_expires(&bye_ok.headers).is_none() && !requires_timer(&bye_ok),
        "2xx to BYE carries no session-timer headers: {bye_ok}"
    );

    peer.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn caller_with_timer_support_keeps_require_timer() {
    const SESSION_SECS: u32 = 30;
    let (mut peer, peer_addr) = answering_peer(SESSION_SECS).await;
    let uac = RawUac::new(peer_addr).await;

    uac.send(Message::Request(uac.invite(&[
        (HeaderName::Supported, "timer"),
        (HeaderName::SessionExpires, "30;refresher=uac"),
    ])))
    .await;
    answer_one(&mut peer).await;
    let ok = uac.final_response(Method::Invite).await;
    assert_eq!(ok.status_code(), 200);
    assert!(
        requires_timer(&ok),
        "Require: timer for a supporting caller"
    );
    let se = session_expires(&ok.headers).expect("Session-Expires");
    assert!(
        se.starts_with("30") && se.contains("refresher=uac"),
        "caller keeps its refresher role: {se}"
    );
    let to_tag = uac.ack(&ok).await;

    let bye = uac.in_dialog(Method::Bye, 2, &to_tag);
    uac.send(Message::Request(bye)).await;
    assert_eq!(uac.final_response(Method::Bye).await.status_code(), 200);
    peer.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn non_refresher_sends_408_bye_before_the_session_expires() {
    // 12 s interval: RFC 4028 §10 BYE at 12 - min(32, 4) = 8 s.
    const SESSION_SECS: u32 = 12;
    let (mut peer, peer_addr) = answering_peer(SESSION_SECS).await;
    let uac = RawUac::new(peer_addr).await;

    uac.send(Message::Request(uac.invite(&[
        (HeaderName::Supported, "timer"),
        (HeaderName::SessionExpires, "12;refresher=uac"),
    ])))
    .await;
    answer_one(&mut peer).await;
    let ok = uac.final_response(Method::Invite).await;
    assert_eq!(ok.status_code(), 200);
    uac.ack(&ok).await;
    let answered = Instant::now();

    // The caller promised to refresh and never does.
    let bye = uac
        .request(
            Method::Bye,
            Duration::from_secs(u64::from(SESSION_SECS) + 4),
        )
        .await
        .expect("rvoip tears the expired session down");
    let elapsed = answered.elapsed();
    uac.send(Message::Response(create_response(&bye, StatusCode::Ok)))
        .await;

    let reason = bye
        .raw_header_value(&HeaderName::Reason)
        .expect("BYE carries Reason");
    assert!(reason.contains("cause=408"), "Reason: {reason}");
    assert!(
        elapsed >= Duration::from_millis(7_000) && elapsed < Duration::from_millis(10_000),
        "RFC 4028 §10: BYE at interval - min(32, interval/3) = 8 s, sent after {elapsed:?}"
    );

    peer.shutdown().await.expect("shutdown");
}
