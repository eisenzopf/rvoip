//! RFC 4028 conformance of rvoip-sip as the calling party (UAC) once the
//! session timer is running.
//!
//! An in-process raw-UDP peer answers the INVITE and names the refresher:
//!
//! 1. `refresher=uac`, and the peer rejects UPDATE with 405: rvoip must fall
//!    back to a refresh re-INVITE (§7.4: carrying an offer, one
//!    `Session-Expires;refresher=uac`) and keep the call up. Before the fix
//!    the re-INVITE carried a duplicate `Session-Expires`/`Min-SE`, failed
//!    to build, and the call was torn down with a 408 BYE.
//! 2. `refresher=uas`: an application re-INVITE (hold) must keep the peer
//!    as refresher (`refresher=uas`) with the negotiated interval, not
//!    re-propose the configured interval with `refresher=uac`.

// Second-scale session timers need `Config::session_timer_allow_short_intervals_for_testing`,
// which exists only with the `test-hooks` feature (`cargo test -p rvoip-sip
// --features test-hooks`; every rvoip-sip CI lane enables it).
#![cfg(feature = "test-hooks")]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::time::timeout;

use rvoip_sip::{Config, Event, StreamPeer};
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::HeaderValue;
use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

const SESSION_SECS: u32 = 4;

#[derive(Clone, Debug)]
struct SeenRequest {
    method: Method,
    body_len: usize,
    session_expires: Vec<String>,
    min_se: usize,
}

struct MockPeer {
    refresher: &'static str,
    port: u16,
    rtp_port: u16,
    to_tag: String,
    seen: Mutex<Vec<SeenRequest>>,
}

impl MockPeer {
    fn ok(&self, request: &Request, with_sdp: bool) -> Vec<u8> {
        let mut response = create_response(request, StatusCode::Ok);
        if let Some(TypedHeader::To(to)) = response
            .headers
            .iter_mut()
            .find(|header| matches!(header, TypedHeader::To(_)))
        {
            if to.tag().is_none() {
                to.set_tag(&self.to_tag);
            }
        }
        if matches!(request.method(), Method::Invite | Method::Update) {
            response.headers.push(TypedHeader::Other(
                HeaderName::SessionExpires,
                HeaderValue::Raw(
                    format!("{SESSION_SECS};refresher={}", self.refresher).into_bytes(),
                ),
            ));
            response.headers.push(TypedHeader::Other(
                HeaderName::Require,
                HeaderValue::Raw(b"timer".to_vec()),
            ));
            response.headers.push(TypedHeader::Other(
                HeaderName::Contact,
                HeaderValue::Raw(format!("<sip:peer@127.0.0.1:{}>", self.port).into_bytes()),
            ));
            // No `Allow` header: rvoip cannot tell whether UPDATE works, so
            // it tries UPDATE first and must fall back on the 405. (A peer
            // whose `Allow` omits UPDATE is refreshed with re-INVITE from
            // the start; see session_timer_refresh_outcomes.rs.)
        }
        if with_sdp {
            let body = format!(
                "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\n\
                 m=audio {} RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\na=sendrecv\r\n",
                self.rtp_port
            )
            .into_bytes();
            response
                .headers
                .retain(|header| !matches!(header, TypedHeader::ContentLength(_)));
            response.headers.push(TypedHeader::ContentLength(
                rvoip_sip_core::types::ContentLength::new(body.len() as u32),
            ));
            response.headers.push(TypedHeader::ContentType(
                rvoip_sip_core::types::ContentType::from_type_subtype("application", "sdp"),
            ));
            response.body = body.into();
        }
        Message::Response(response).to_bytes()
    }

    fn seen(&self) -> Vec<SeenRequest> {
        self.seen.lock().unwrap().clone()
    }
}

async fn run_mock_peer(sock: Arc<UdpSocket>, peer: Arc<MockPeer>) {
    let mut buf = vec![0u8; 16384];
    loop {
        let Ok((n, from)) = sock.recv_from(&mut buf).await else {
            return;
        };
        let Ok(Message::Request(request)) = parse_message(&buf[..n]) else {
            continue;
        };
        peer.seen.lock().unwrap().push(SeenRequest {
            method: request.method(),
            body_len: request.body().len(),
            session_expires: request
                .headers
                .iter()
                .filter(|h| h.name() == HeaderName::SessionExpires)
                .map(|h| {
                    let full = h.to_string().to_ascii_lowercase().replace(' ', "");
                    full.split_once(':')
                        .map_or(full.clone(), |(_, value)| value.to_string())
                })
                .collect(),
            min_se: request
                .headers
                .iter()
                .filter(|h| h.name() == HeaderName::MinSE)
                .count(),
        });
        let reply = match request.method() {
            Method::Invite => peer.ok(&request, true),
            Method::Update => {
                Message::Response(create_response(&request, StatusCode::MethodNotAllowed))
                    .to_bytes()
            }
            Method::Bye => peer.ok(&request, false),
            _ => continue,
        };
        let _ = sock.send_to(&reply, from).await;
    }
}

struct Harness {
    peer: StreamPeer,
    mock: Arc<MockPeer>,
    call_id: rvoip_sip::api::events::CallId,
    handle: rvoip_sip::SessionHandle,
    _rtp_sink: UdpSocket,
    task: tokio::task::JoinHandle<()>,
}

async fn start_call(refresher: &'static str, media_ports: (u16, u16)) -> Harness {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();

    let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("peer bind"));
    let port = sock.local_addr().unwrap().port();
    let rtp_sink = UdpSocket::bind("127.0.0.1:0").await.expect("rtp bind");
    let mock = Arc::new(MockPeer {
        refresher,
        port,
        rtp_port: rtp_sink.local_addr().unwrap().port(),
        to_tag: format!("peer-{}", rand::random::<u32>()),
        seen: Mutex::new(Vec::new()),
    });
    let task = tokio::spawn(run_mock_peer(sock, Arc::clone(&mock)));

    let mut config = Config::local("alice", 0);
    config.media_port_start = media_ports.0;
    config.media_port_end = media_ports.1;
    config.session_timer_secs = Some(SESSION_SECS);
    config.session_timer_min_se = 2;
    // Seconds-scale intervals keep the test fast; RFC 4028 §5 forbids them in
    // production, so `Config::validate` needs the test escape hatch.
    config.session_timer_allow_short_intervals_for_testing = true;
    let mut peer = StreamPeer::with_config(config).await.expect("peer");
    let call_id = peer
        .invite(format!("sip:bob@127.0.0.1:{port}"))
        .send()
        .await
        .expect("invite");
    let handle = timeout(Duration::from_secs(10), peer.wait_for_answered(&call_id))
        .await
        .expect("answered in time")
        .expect("answered");
    Harness {
        peer,
        mock,
        call_id,
        handle,
        _rtp_sink: rtp_sink,
        task,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refresher_falls_back_to_reinvite_with_offer_when_peer_rejects_update() {
    let mut h = start_call("uac", (45600, 45700)).await;

    // Two refreshes at ~SESSION_SECS / 2 each: UPDATE (405) then re-INVITE.
    let mut refreshed = 0;
    let outcome = timeout(
        Duration::from_secs(u64::from(SESSION_SECS) * 2 + 3),
        async {
            while refreshed < 2 {
                match h.peer.next_event().await {
                    Some(Event::SessionRefreshed { call_id, .. }) if call_id == h.call_id => {
                        refreshed += 1;
                    }
                    Some(Event::SessionRefreshFailed { call_id, reason })
                        if call_id == h.call_id =>
                    {
                        panic!("refresh failed instead of falling back to re-INVITE: {reason}")
                    }
                    Some(Event::CallEnded { call_id, reason }) if call_id == h.call_id => {
                        panic!("call ended instead of refreshing: {reason}")
                    }
                    Some(_) => {}
                    None => panic!("event stream closed"),
                }
            }
        },
    )
    .await;
    assert!(
        outcome.is_ok(),
        "saw {refreshed} refreshes: {:?}",
        h.mock.seen()
    );

    let seen = h.mock.seen();
    let reinvites: Vec<_> = seen
        .iter()
        .filter(|request| request.method == Method::Invite)
        .skip(1)
        .collect();
    assert!(reinvites.len() >= 2, "re-INVITE fallback: {seen:?}");
    for reinvite in reinvites {
        assert!(
            reinvite.body_len > 0,
            "§7.4: refresh re-INVITE carries an offer"
        );
        assert_eq!(
            reinvite.session_expires.len(),
            1,
            "exactly one Session-Expires: {reinvite:?}"
        );
        assert_eq!(reinvite.min_se, 1, "exactly one Min-SE: {reinvite:?}");
        assert!(
            reinvite.session_expires[0].starts_with(&SESSION_SECS.to_string())
                && reinvite.session_expires[0].contains("refresher=uac"),
            "{reinvite:?}"
        );
    }
    assert!(
        !seen.iter().any(|request| request.method == Method::Bye),
        "call stays up: {seen:?}"
    );

    h.peer.shutdown().await.expect("shutdown");
    h.task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn application_reinvite_keeps_the_peer_as_refresher() {
    let h = start_call("uas", (45700, 45800)).await;

    h.handle.hold().await.expect("hold");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let reinvite = loop {
        if let Some(reinvite) = h
            .mock
            .seen()
            .into_iter()
            .filter(|request| request.method == Method::Invite)
            .nth(1)
        {
            break reinvite;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "hold re-INVITE not sent: {:?}",
            h.mock.seen()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(reinvite.session_expires.len(), 1, "{reinvite:?}");
    assert!(
        reinvite.session_expires[0].starts_with(&SESSION_SECS.to_string())
            && reinvite.session_expires[0].contains("refresher=uas"),
        "§7.4: the peer stays the refresher: {reinvite:?}"
    );

    h.peer.shutdown().await.expect("shutdown");
    h.task.abort();
}
