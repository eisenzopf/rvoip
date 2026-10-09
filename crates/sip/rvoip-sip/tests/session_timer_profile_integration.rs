//! RFC 4028 session timers as the deployment profiles configure them.
//!
//! Starts from `Config::carrier_trunk_udp` — outbound proxy, playout and
//! session timers on — and shortens only the interval. An in-process raw-UDP
//! trunk answers the INVITE and names the refresher:
//!
//! 1. `refresher=uac`: rvoip must refresh with UPDATE at half the interval,
//!    re-arm after each 2xx, and publish `Event::SessionRefreshed`.
//! 2. `refresher=uas`: the trunk promises to refresh and never does; rvoip
//!    must tear the call down at expiry with `BYE` carrying
//!    `Reason: SIP;cause=408` and publish `Event::CallEnded`.
//!
//! The refresher-side failure path (UPDATE and re-INVITE both time out) is
//! covered by `session_timer_failure_integration.rs`.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;
use tokio::time::timeout;

use rvoip_sip::{Config, Event, StreamPeer};
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::{HeaderAccess, HeaderValue};
use rvoip_sip_core::types::session_expires::{Refresher, SessionExpires};
use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

const SESSION_SECS: u32 = 4;

struct MockTrunk {
    refresher: Refresher,
    port: u16,
    rtp_port: u16,
    to_tag: String,
    invites: AtomicU32,
    updates: AtomicU32,
    update_session_expires: Mutex<Vec<Option<String>>>,
    bye: Mutex<Option<(Instant, Option<String>)>>,
    answered_at: Mutex<Option<Instant>>,
}

impl MockTrunk {
    fn new(refresher: Refresher, port: u16, rtp_port: u16) -> Self {
        Self {
            refresher,
            port,
            rtp_port,
            to_tag: format!("trunk-{}", rand::random::<u32>()),
            invites: AtomicU32::new(0),
            updates: AtomicU32::new(0),
            update_session_expires: Mutex::new(Vec::new()),
            bye: Mutex::new(None),
            answered_at: Mutex::new(None),
        }
    }

    fn ok_with_session_timer(&self, request: &Request, refresher: Refresher) -> Response {
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
        response
            .headers
            .push(TypedHeader::SessionExpires(SessionExpires::new(
                SESSION_SECS,
                Some(refresher),
            )));
        response.headers.push(TypedHeader::Other(
            HeaderName::Require,
            HeaderValue::Raw(b"timer".to_vec()),
        ));
        response.headers.push(TypedHeader::Other(
            HeaderName::Contact,
            HeaderValue::Raw(format!("<sip:trunk@127.0.0.1:{}>", self.port).into_bytes()),
        ));
        response
    }

    fn invite_answer(&self, request: &Request) -> Vec<u8> {
        let mut response = self.ok_with_session_timer(request, self.refresher);
        let sdp = format!(
            "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\n\
             m=audio {} RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\na=sendrecv\r\n",
            self.rtp_port
        );
        let body = sdp.into_bytes();
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
        Message::Response(response).to_bytes()
    }
}

async fn run_mock_trunk(sock: Arc<UdpSocket>, trunk: Arc<MockTrunk>) {
    let mut buf = vec![0u8; 8192];
    loop {
        let Ok((n, from)) = sock.recv_from(&mut buf).await else {
            return;
        };
        let Ok(Message::Request(request)) = parse_message(&buf[..n]) else {
            continue;
        };
        let reply = match request.method() {
            Method::Invite => {
                if trunk.invites.fetch_add(1, Ordering::SeqCst) == 0 {
                    *trunk.answered_at.lock().unwrap() = Some(Instant::now());
                    trunk.invite_answer(&request)
                } else {
                    // Refresh re-INVITE: confirm the interval, keep the role.
                    Message::Response(trunk.ok_with_session_timer(&request, Refresher::Uac))
                        .to_bytes()
                }
            }
            Method::Update => {
                trunk.updates.fetch_add(1, Ordering::SeqCst);
                trunk
                    .update_session_expires
                    .lock()
                    .unwrap()
                    .push(request.raw_header_value(&HeaderName::SessionExpires));
                Message::Response(trunk.ok_with_session_timer(&request, Refresher::Uac)).to_bytes()
            }
            Method::Bye => {
                trunk.bye.lock().unwrap().get_or_insert((
                    Instant::now(),
                    request.raw_header_value(&HeaderName::Reason),
                ));
                Message::Response(create_response(&request, StatusCode::Ok)).to_bytes()
            }
            _ => continue,
        };
        let _ = sock.send_to(&reply, from).await;
    }
}

struct Harness {
    peer: StreamPeer,
    trunk: Arc<MockTrunk>,
    call_id: rvoip_sip::api::events::CallId,
    _rtp_sink: UdpSocket,
    task: tokio::task::JoinHandle<()>,
}

async fn start_call(refresher: Refresher, media_ports: (u16, u16)) -> Harness {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();

    let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("trunk bind"));
    let port = sock.local_addr().unwrap().port();
    let rtp_sink = UdpSocket::bind("127.0.0.1:0").await.expect("rtp bind");
    let trunk = Arc::new(MockTrunk::new(
        refresher,
        port,
        rtp_sink.local_addr().unwrap().port(),
    ));
    let task = tokio::spawn(run_mock_trunk(sock, Arc::clone(&trunk)));

    let mut config = Config::carrier_trunk_udp(
        "pbx",
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1:5060".parse().unwrap(),
        format!("sip:127.0.0.1:{port};lr"),
    );
    assert_eq!(config.session_timer_secs, Some(1800), "profile default");
    // Loopback test: no public address, and a short interval.
    config.sip_advertised_addr = None;
    config.media_public_addr = None;
    config.session_timer_secs = Some(SESSION_SECS);
    config.session_timer_min_se = 2;
    config.media_port_start = media_ports.0;
    config.media_port_end = media_ports.1;

    let mut peer = StreamPeer::with_config(config).await.expect("peer");
    let call_id = peer
        .invite(format!("sip:+15551234567@127.0.0.1:{port}"))
        .send()
        .await
        .expect("invite");
    timeout(Duration::from_secs(10), peer.wait_for_answered(&call_id))
        .await
        .expect("answered in time")
        .expect("answered");
    Harness {
        peer,
        trunk,
        call_id,
        _rtp_sink: rtp_sink,
        task,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_refresher_refreshes_with_update_and_rearms() {
    let mut h = start_call(Refresher::Uac, (41400, 41500)).await;
    assert_eq!(h.trunk.invites.load(Ordering::SeqCst), 1);

    // Two refreshes at ~SESSION_SECS/2 each prove the timer re-arms.
    let mut refreshed = 0;
    let deadline = Duration::from_secs(u64::from(SESSION_SECS) * 2 + 3);
    let collected = timeout(deadline, async {
        while refreshed < 2 {
            match h.peer.next_event().await {
                Some(Event::SessionRefreshed {
                    call_id,
                    expires_secs,
                }) if call_id == h.call_id => {
                    assert_eq!(expires_secs, SESSION_SECS);
                    refreshed += 1;
                }
                Some(Event::CallEnded { call_id, reason }) if call_id == h.call_id => {
                    panic!("call ended instead of refreshing: {reason}")
                }
                Some(_) => {}
                None => panic!("event stream closed"),
            }
        }
    })
    .await;
    assert!(collected.is_ok(), "saw {refreshed} refreshes");

    let updates = h.trunk.updates.load(Ordering::SeqCst);
    let reinvites = h.trunk.invites.load(Ordering::SeqCst) - 1;
    assert!(
        updates >= 2,
        "refresh uses UPDATE first (updates={updates})"
    );
    assert_eq!(reinvites, 0, "no re-INVITE fallback when UPDATE succeeds");
    for header in h.trunk.update_session_expires.lock().unwrap().iter() {
        let header = header.as_deref().expect("UPDATE carries Session-Expires");
        assert!(header.starts_with(&SESSION_SECS.to_string()), "{header}");
    }
    assert!(h.trunk.bye.lock().unwrap().is_none(), "call stays up");

    h.peer.shutdown().await.expect("shutdown");
    h.task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn silent_peer_refresher_expires_session_with_408_bye() {
    let mut h = start_call(Refresher::Uas, (41500, 41600)).await;

    let ended = timeout(Duration::from_secs(u64::from(SESSION_SECS) + 6), async {
        loop {
            match h.peer.next_event().await {
                Some(Event::CallEnded { call_id, .. }) if call_id == h.call_id => return,
                Some(_) => {}
                None => panic!("event stream closed"),
            }
        }
    })
    .await;
    assert!(
        ended.is_ok(),
        "call must end once the peer misses its refresh"
    );

    let (bye_at, reason) = h.trunk.bye.lock().unwrap().clone().expect("rvoip sent BYE");
    let reason = reason.expect("BYE carries a Reason header");
    assert!(reason.contains("cause=408"), "Reason: {reason}");
    let answered_at = h.trunk.answered_at.lock().unwrap().expect("answered");
    let elapsed = bye_at.duration_since(answered_at);
    assert!(
        elapsed >= Duration::from_secs(u64::from(SESSION_SECS) - 1),
        "BYE must wait for the session interval, sent after {elapsed:?}"
    );
    assert_eq!(
        h.trunk.updates.load(Ordering::SeqCst),
        0,
        "the non-refresher never refreshes"
    );

    h.peer.shutdown().await.expect("shutdown");
    h.task.abort();
}
