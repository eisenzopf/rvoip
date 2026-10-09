//! RFC 4028 refresh outcomes for rvoip-sip as the refresher.
//!
//! rvoip calls an in-process raw-UDP peer that answers the INVITE with
//! `Session-Expires: N;refresher=uac`, so rvoip refreshes every N/2 seconds.
//! The peer answers each in-dialog INVITE/UPDATE from a script, which lets
//! the tests drive every refresh outcome RFC 4028 distinguishes:
//!
//! - §10: only a refresh that times out or draws 408/481 ends the session
//!   (BYE with `Reason: SIP;cause=408`). 491 is retried after the RFC 3261
//!   §14.1 backoff, 422 is retried with the peer's `Min-SE` (§7.4), and any
//!   other rejection (488, 403, 5xx) keeps the call until the session
//!   expires unless something refreshes it first.
//! - §7.2/§7.4: the 2xx to a refresh renegotiates the interval and the
//!   refresher; a 2xx without `Session-Expires` turns the timer off.
//! - RFC 3261 §20.5: a peer whose `Allow` omits UPDATE is refreshed with
//!   re-INVITE only.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;
use tokio::time::timeout;

use rvoip_sip::{Config, Event, StreamPeer};
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::{HeaderAccess, HeaderValue};
use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

/// Allow without UPDATE: RFC 3261 §20.5 says UPDATE will not work.
const ALLOW_NO_UPDATE: &str = "INVITE, ACK, BYE, CANCEL, OPTIONS";

#[derive(Clone, Debug)]
enum Reply {
    /// 200 OK carrying `Session-Expires: <value>` (and `Require: timer`), or
    /// no session-timer headers at all for `None`.
    Ok(Option<String>),
    /// A final failure response with extra `(header, value)` pairs.
    Status(u16, Vec<(HeaderName, String)>),
    /// Never answer: the refresh transaction times out.
    Drop,
}

#[derive(Clone, Debug)]
struct Seen {
    method: Method,
    at: Instant,
    session_expires: Option<String>,
    min_se: Option<String>,
    reason: Option<String>,
}

struct MockPeer {
    session_secs: u32,
    allow: Option<&'static str>,
    port: u16,
    rtp_port: u16,
    to_tag: String,
    script: Mutex<VecDeque<Reply>>,
    default_reply: Reply,
    seen: Mutex<Vec<Seen>>,
}

impl MockPeer {
    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    /// In-dialog INVITEs and UPDATEs, i.e. everything after the call setup.
    fn refreshes(&self) -> Vec<Seen> {
        self.seen()
            .into_iter()
            .filter(|seen| matches!(seen.method, Method::Invite | Method::Update))
            .skip(1)
            .collect()
    }

    fn byes(&self) -> Vec<Seen> {
        self.seen()
            .into_iter()
            .filter(|seen| seen.method == Method::Bye)
            .collect()
    }

    fn answer(&self, request: &Request, reply: &Reply) -> Option<Vec<u8>> {
        let mut response = match reply {
            Reply::Drop => return None,
            Reply::Ok(_) => create_response(request, StatusCode::Ok),
            Reply::Status(code, _) => {
                create_response(request, StatusCode::from_u16(*code).expect("status code"))
            }
        };
        if let Some(TypedHeader::To(to)) = response
            .headers
            .iter_mut()
            .find(|header| matches!(header, TypedHeader::To(_)))
        {
            if to.tag().is_none() {
                to.set_tag(&self.to_tag);
            }
        }
        let raw = |name: HeaderName, value: &str| {
            TypedHeader::Other(name, HeaderValue::Raw(value.as_bytes().to_vec()))
        };
        match reply {
            Reply::Ok(session_expires) => {
                if let Some(value) = session_expires {
                    response
                        .headers
                        .push(raw(HeaderName::SessionExpires, value));
                    response.headers.push(raw(HeaderName::Require, "timer"));
                }
                response.headers.push(raw(
                    HeaderName::Contact,
                    &format!("<sip:peer@127.0.0.1:{}>", self.port),
                ));
                if let Some(allow) = self.allow {
                    response.headers.push(raw(HeaderName::Allow, allow));
                }
                if request.method() == Method::Invite {
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
            }
            Reply::Status(_, extra) => {
                for (name, value) in extra {
                    response.headers.push(raw(name.clone(), value));
                }
            }
            Reply::Drop => unreachable!(),
        }
        Some(Message::Response(response).to_bytes())
    }
}

fn header_value(request: &Request, name: HeaderName) -> Option<String> {
    request.headers.iter().find(|h| h.name() == name).map(|h| {
        let full = h.to_string().to_ascii_lowercase().replace(' ', "");
        full.split_once(':')
            .map_or(full.clone(), |(_, value)| value.to_string())
    })
}

async fn run_mock_peer(sock: Arc<UdpSocket>, peer: Arc<MockPeer>) {
    let mut buf = vec![0u8; 16384];
    let mut answered_initial = false;
    // One decision per transaction: a UDP retransmission (same Via branch)
    // gets the same answer, or none for `Drop`.
    let mut decided: HashMap<String, Reply> = HashMap::new();
    loop {
        let Ok((n, from)) = sock.recv_from(&mut buf).await else {
            return;
        };
        let Ok(Message::Request(request)) = parse_message(&buf[..n]) else {
            continue;
        };
        let method = request.method();
        if method == Method::Ack {
            continue;
        }
        let branch = request
            .first_via()
            .and_then(|via| via.branch().map(str::to_string))
            .unwrap_or_default();
        if let Some(reply) = decided.get(&branch) {
            if let Some(bytes) = peer.answer(&request, reply) {
                let _ = sock.send_to(&bytes, from).await;
            }
            continue;
        }
        peer.seen.lock().unwrap().push(Seen {
            method: method.clone(),
            at: Instant::now(),
            session_expires: header_value(&request, HeaderName::SessionExpires),
            min_se: header_value(&request, HeaderName::MinSE),
            reason: request.raw_header_value(&HeaderName::Reason),
        });
        let reply = match method {
            Method::Invite if !answered_initial => {
                answered_initial = true;
                Reply::Ok(Some(format!("{};refresher=uac", peer.session_secs)))
            }
            Method::Invite | Method::Update => peer
                .script
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| peer.default_reply.clone()),
            Method::Bye => Reply::Ok(None),
            _ => continue,
        };
        decided.insert(branch, reply.clone());
        if let Some(bytes) = peer.answer(&request, &reply) {
            let _ = sock.send_to(&bytes, from).await;
        }
    }
}

struct Harness {
    peer: StreamPeer,
    mock: Arc<MockPeer>,
    call_id: rvoip_sip::api::events::CallId,
    handle: rvoip_sip::SessionHandle,
    answered: Instant,
    _rtp_sink: UdpSocket,
    task: tokio::task::JoinHandle<()>,
}

impl Harness {
    async fn shutdown(self) {
        self.peer.shutdown().await.expect("shutdown");
        self.task.abort();
    }

    /// Wait until `ready` holds for the peer's view, polling every 50 ms.
    async fn wait_until(&self, within: Duration, what: &str, ready: impl Fn(&MockPeer) -> bool) {
        let deadline = Instant::now() + within;
        while !ready(&self.mock) {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}: {:#?}",
                self.mock.seen()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Drain application events until `matches` accepts one or `within`
    /// elapses.
    async fn event(&mut self, within: Duration, matches: impl Fn(&Event) -> bool) -> Option<Event> {
        let call_id = self.call_id.clone();
        timeout(within, async {
            loop {
                match self.peer.next_event().await {
                    Some(event) if event.call_id() == Some(&call_id) && matches(&event) => {
                        return Some(event)
                    }
                    Some(_) => {}
                    None => return None,
                }
            }
        })
        .await
        .ok()
        .flatten()
    }
}

static TEST_TIMEOUTS: Once = Once::new();

async fn start_call(
    session_secs: u32,
    allow: Option<&'static str>,
    script: Vec<Reply>,
    default_reply: Reply,
    media_base: u16,
) -> Harness {
    TEST_TIMEOUTS.call_once(|| {
        // Shorten Timer B/F so the timeout test does not wait 32 s. Set once,
        // before any peer in this binary builds its transaction layer.
        std::env::set_var("RVOIP_TEST_TRANSACTION_TIMEOUT_MS", "2000");
        let _ = tracing_subscriber::fmt()
            .with_test_writer()
            .with_max_level(tracing::Level::WARN)
            .try_init();
    });

    let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("peer bind"));
    let port = sock.local_addr().unwrap().port();
    let rtp_sink = UdpSocket::bind("127.0.0.1:0").await.expect("rtp bind");
    let mock = Arc::new(MockPeer {
        session_secs,
        allow,
        port,
        rtp_port: rtp_sink.local_addr().unwrap().port(),
        to_tag: format!("peer-{}", rand::random::<u32>()),
        script: Mutex::new(script.into()),
        default_reply,
        seen: Mutex::new(Vec::new()),
    });
    let task = tokio::spawn(run_mock_peer(sock, Arc::clone(&mock)));

    let mut config = Config::local("alice", 0);
    config.media_port_start = media_base;
    config.media_port_end = media_base + 100;
    config.session_timer_secs = Some(session_secs);
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
        answered: Instant::now(),
        _rtp_sink: rtp_sink,
        task,
    }
}

fn ok_with(session_expires: &str) -> Reply {
    Reply::Ok(Some(session_expires.to_string()))
}

fn default_ok(session_secs: u32) -> Reply {
    ok_with(&format!("{session_secs};refresher=uac"))
}

// ---------------------------------------------------------------------------
// (b) RFC 4028 §10: which refresh failures end the session
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reinvite_refresh_answered_491_is_retried_after_glare_backoff() {
    let h = start_call(
        4,
        Some(ALLOW_NO_UPDATE),
        vec![Reply::Status(491, vec![])],
        default_ok(4),
        46000,
    )
    .await;

    // Refresh at 2 s draws 491; RFC 3261 §14.1 has the Call-ID owner (rvoip
    // placed the call) retry after 2.1-4 s.
    h.wait_until(Duration::from_secs(10), "the retried re-INVITE", |mock| {
        mock.refreshes().len() >= 2
    })
    .await;
    let refreshes = h.mock.refreshes();
    assert!(
        refreshes.iter().all(|seen| seen.method == Method::Invite),
        "{refreshes:#?}"
    );
    let backoff = refreshes[1].at - refreshes[0].at;
    assert!(
        backoff >= Duration::from_millis(2_000) && backoff < Duration::from_millis(4_800),
        "glare retry after the 2.1-4 s backoff, got {backoff:?}"
    );
    assert!(
        h.mock.byes().is_empty(),
        "491 must not end the call: {:#?}",
        h.mock.seen()
    );
    assert!(h.handle.is_active().await, "call is still up");
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_refresh_answered_491_is_retried_with_update() {
    let h = start_call(
        4,
        None,
        vec![Reply::Status(491, vec![])],
        default_ok(4),
        46100,
    )
    .await;

    h.wait_until(Duration::from_secs(10), "the retried UPDATE", |mock| {
        mock.refreshes().len() >= 2
    })
    .await;
    let refreshes = h.mock.refreshes();
    assert_eq!(
        (refreshes[0].method.clone(), refreshes[1].method.clone()),
        (Method::Update, Method::Update),
        "491 is glare, not a reason to switch to re-INVITE: {refreshes:#?}"
    );
    assert!(
        refreshes[1].at - refreshes[0].at >= Duration::from_millis(2_000),
        "{refreshes:#?}"
    );
    assert!(h.mock.byes().is_empty(), "{:#?}", h.mock.seen());
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reinvite_refresh_answered_488_keeps_the_call_until_the_session_expires() {
    // 12 s interval: refresh at 6 s; if nothing refreshes the session it
    // expires and rvoip sends BYE at 12 - min(32, 12 / 3) = 8 s.
    let mut h = start_call(
        12,
        Some(ALLOW_NO_UPDATE),
        vec![],
        Reply::Status(488, vec![]),
        46200,
    )
    .await;

    h.wait_until(Duration::from_secs(9), "the refresh re-INVITE", |mock| {
        !mock.refreshes().is_empty()
    })
    .await;
    let rejected_at = h.mock.refreshes()[0].at;
    tokio::time::sleep(Duration::from_millis(1_000)).await;
    assert!(
        h.mock.byes().is_empty(),
        "a 488 to a refresh must not end the call: {:#?}",
        h.mock.seen()
    );
    assert!(h.handle.is_active().await, "call is still up after the 488");

    let failed = h
        .event(Duration::from_secs(6), |event| {
            matches!(event, Event::SessionRefreshFailed { .. })
        })
        .await;
    assert!(failed.is_some(), "the session eventually expires");
    h.wait_until(Duration::from_secs(3), "the expiry BYE", |mock| {
        !mock.byes().is_empty()
    })
    .await;
    let bye = &h.mock.byes()[0];
    let after_answer = bye.at - h.answered;
    assert!(
        after_answer >= Duration::from_millis(7_400)
            && after_answer < Duration::from_millis(10_000),
        "BYE at session expiry (~8 s), got {after_answer:?} ({:?} after the 488)",
        bye.at - rejected_at
    );
    assert!(
        bye.reason
            .as_deref()
            .is_some_and(|reason| reason.contains("cause=408")),
        "{bye:?}"
    );
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejected_refresh_is_rescued_by_a_later_successful_reinvite() {
    // 12 s interval: the 6 s refresh draws 488. A hold re-INVITE then
    // succeeds, which refreshes the session (RFC 4028 §7.4), so there is no
    // expiry BYE at 8 s and refreshing resumes 6 s after the hold.
    let h = start_call(
        12,
        Some(ALLOW_NO_UPDATE),
        vec![Reply::Status(488, vec![])],
        default_ok(12),
        46300,
    )
    .await;

    h.wait_until(Duration::from_secs(9), "the rejected refresh", |mock| {
        !mock.refreshes().is_empty()
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    h.handle.hold().await.expect("hold");
    let held_at = Instant::now();
    h.wait_until(
        Duration::from_secs(10),
        "a refresh after the hold",
        |mock| {
            mock.refreshes()
                .iter()
                .filter(|seen| seen.at > held_at + Duration::from_secs(1))
                .count()
                >= 1
        },
    )
    .await;
    assert!(
        h.mock.byes().is_empty(),
        "the successful re-INVITE refreshed the session: {:#?}",
        h.mock.seen()
    );
    h.shutdown().await;
}

async fn refresh_answer_ends_the_call(reply: Reply, media_base: u16) {
    let mut h = start_call(4, None, vec![reply.clone()], default_ok(4), media_base).await;
    h.wait_until(Duration::from_secs(8), "the BYE", |mock| {
        !mock.byes().is_empty()
    })
    .await;
    let refreshes = h.mock.refreshes();
    let bye = &h.mock.byes()[0];
    assert_eq!(
        refreshes.len(),
        1,
        "{reply:?} ends the session without a re-INVITE fallback: {:#?}",
        h.mock.seen()
    );
    assert_eq!(refreshes[0].method, Method::Update);
    assert!(
        bye.reason
            .as_deref()
            .is_some_and(|reason| reason.contains("cause=408")),
        "{bye:?}"
    );
    assert!(h
        .event(Duration::from_secs(3), |event| {
            matches!(event, Event::SessionRefreshFailed { .. })
        })
        .await
        .is_some());
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refresh_answered_481_ends_the_call() {
    refresh_answer_ends_the_call(Reply::Status(481, vec![]), 46400).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refresh_answered_408_ends_the_call() {
    refresh_answer_ends_the_call(Reply::Status(408, vec![]), 46500).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refresh_that_times_out_ends_the_call() {
    // RVOIP_TEST_TRANSACTION_TIMEOUT_MS=2000 makes the UPDATE time out ~2 s
    // after it is sent.
    refresh_answer_ends_the_call(Reply::Drop, 46600).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refresh_answered_422_is_retried_with_the_peers_min_se() {
    let h = start_call(
        4,
        Some(ALLOW_NO_UPDATE),
        vec![
            Reply::Status(422, vec![(HeaderName::MinSE, "8".to_string())]),
            ok_with("8;refresher=uac"),
        ],
        ok_with("8;refresher=uac"),
        46700,
    )
    .await;

    h.wait_until(Duration::from_secs(8), "the 422 retry", |mock| {
        mock.refreshes().len() >= 2
    })
    .await;
    let refreshes = h.mock.refreshes();
    let retry = &refreshes[1];
    assert!(
        retry.at - refreshes[0].at < Duration::from_millis(1_500),
        "422 is retried at once: {refreshes:#?}"
    );
    assert!(
        retry
            .session_expires
            .as_deref()
            .is_some_and(|se| se.starts_with('8')),
        "§7.4: the retry's interval is at least the 422 Min-SE: {retry:?}"
    );
    assert_eq!(retry.min_se.as_deref(), Some("8"), "{retry:?}");
    assert!(h.mock.byes().is_empty(), "{:#?}", h.mock.seen());
    h.shutdown().await;
}

// ---------------------------------------------------------------------------
// (a) RFC 4028 §7.2/§7.4: refresh responses renegotiate the timer
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refresh_response_interval_is_applied() {
    // Interval 4 (refresh every 2 s) until the first refresh's 2xx says 8:
    // the next refresh then comes 4 s later.
    let h = start_call(
        4,
        None,
        vec![ok_with("8;refresher=uac")],
        default_ok(8),
        46800,
    )
    .await;

    h.wait_until(Duration::from_secs(10), "two refreshes", |mock| {
        mock.refreshes().len() >= 2
    })
    .await;
    let refreshes = h.mock.refreshes();
    let gap = refreshes[1].at - refreshes[0].at;
    assert!(
        gap >= Duration::from_millis(3_300),
        "the renegotiated 8 s interval is refreshed at 4 s, got {gap:?}"
    );
    assert!(
        refreshes[1]
            .session_expires
            .as_deref()
            .is_some_and(|se| se.starts_with('8')),
        "{refreshes:#?}"
    );
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refresh_response_can_make_the_peer_the_refresher() {
    // The first refresh's 2xx names the peer (refresher=uas). rvoip stops
    // refreshing and, as the non-refresher, sends BYE when no refresh arrives:
    // 6 - min(32, 6 / 3) = 4 s after that 2xx.
    let h = start_call(
        4,
        None,
        vec![ok_with("6;refresher=uas")],
        default_ok(4),
        46900,
    )
    .await;

    h.wait_until(Duration::from_secs(8), "the expiry BYE", |mock| {
        !mock.byes().is_empty()
    })
    .await;
    let refreshes = h.mock.refreshes();
    let bye = &h.mock.byes()[0];
    assert_eq!(
        refreshes.len(),
        1,
        "rvoip no longer refreshes: {refreshes:#?}"
    );
    let after = bye.at - refreshes[0].at;
    assert!(
        after >= Duration::from_millis(3_500) && after < Duration::from_millis(5_500),
        "non-refresher BYE ~4 s after the renegotiation, got {after:?}"
    );
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refresh_response_without_session_expires_turns_the_timer_off() {
    let h = start_call(4, None, vec![Reply::Ok(None)], default_ok(4), 47000).await;

    h.wait_until(Duration::from_secs(5), "the first refresh", |mock| {
        !mock.refreshes().is_empty()
    })
    .await;
    // Without the fix, refreshes keep coming every 2 s.
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert_eq!(
        h.mock.refreshes().len(),
        1,
        "§7.2: no Session-Expires in the 2xx means no session expiration: {:#?}",
        h.mock.seen()
    );
    assert!(h.mock.byes().is_empty(), "{:#?}", h.mock.seen());
    assert!(h.handle.is_active().await);
    h.shutdown().await;
}

// ---------------------------------------------------------------------------
// (c) RFC 3261 §20.5 / RFC 4028 §7.4: honor the peer's Allow
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn peer_without_update_in_allow_is_refreshed_with_reinvite_only() {
    let h = start_call(4, Some(ALLOW_NO_UPDATE), vec![], default_ok(4), 47100).await;

    h.wait_until(Duration::from_secs(8), "two refreshes", |mock| {
        mock.refreshes().len() >= 2
    })
    .await;
    let refreshes = h.mock.refreshes();
    assert!(
        refreshes.iter().all(|seen| seen.method == Method::Invite),
        "no UPDATE to a peer whose Allow omits it: {refreshes:#?}"
    );
    assert!(h.mock.byes().is_empty());
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn peer_with_update_in_allow_is_refreshed_with_update() {
    let h = start_call(
        4,
        Some("INVITE, ACK, BYE, CANCEL, OPTIONS, UPDATE"),
        vec![],
        default_ok(4),
        47200,
    )
    .await;

    h.wait_until(Duration::from_secs(8), "two refreshes", |mock| {
        mock.refreshes().len() >= 2
    })
    .await;
    assert!(
        h.mock
            .refreshes()
            .iter()
            .all(|seen| seen.method == Method::Update),
        "{:#?}",
        h.mock.seen()
    );
    h.shutdown().await;
}
