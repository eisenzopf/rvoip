//! Polish-item acceptance tests landing alongside the Phase 7
//! follow-ups documented in `STIR_SHAKEN_AND_PROXY_PLAN.md`:
//!
//! - Loop detection via `Via::detect_loop` (Phase 6 + 7 deferral).
//! - Timer C per-1xx reset per RFC 3261 §16.8 (Phase 6 + 7 deferral).
//! - `ProxyEvent::RedirectReceived` observability stream (Phase 7
//!   deferral).
//!
//! Each test stands up the same `MockTransport` + `TransactionManager`
//! + `StatefulProxy` harness used by `stateful_proxy_single_target.rs`
//! and `proxy_parallel_fork.rs`. We feed inbound traffic by pushing
//! synthetic `TransportEvent`s into the channel the manager consumes.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use rvoip_sip_core::types::content_length::ContentLength;
use rvoip_sip_core::types::param::Param;
use rvoip_sip_core::types::status::StatusCode;
use rvoip_sip_core::types::via::Via;
use rvoip_sip_core::types::TypedHeader;
use rvoip_sip_core::{Message, Method, Request};
use rvoip_sip_dialog::transaction::TransactionManager;
use rvoip_sip_proxy::{ProxyConfig, ProxyEvent, RouteDecision, RouteFn, StatefulProxy};
use rvoip_sip_transport::transport::TransportType;
use rvoip_sip_transport::TransportEvent;
use tokio::sync::{mpsc, Mutex};

const PROXY_ADDR: &str = "127.0.0.1:5060";
const UAC_ADDR: &str = "10.0.0.5:5060";
const UAS_ADDR: &str = "10.0.0.10:5060";

#[derive(Debug, Clone)]
struct MockTransport {
    local_addr: SocketAddr,
    sent: Arc<Mutex<Vec<(Message, SocketAddr)>>>,
}

impl MockTransport {
    fn new(local_addr: SocketAddr) -> Self {
        Self {
            local_addr,
            sent: Arc::new(Mutex::new(Vec::new())),
        }
    }

    async fn sent(&self) -> Vec<(Message, SocketAddr)> {
        self.sent.lock().await.clone()
    }
}

#[async_trait]
impl rvoip_sip_transport::Transport for MockTransport {
    async fn send_message(
        &self,
        message: Message,
        destination: SocketAddr,
    ) -> Result<(), rvoip_sip_transport::Error> {
        self.sent.lock().await.push((message, destination));
        Ok(())
    }

    fn local_addr(&self) -> Result<SocketAddr, rvoip_sip_transport::Error> {
        Ok(self.local_addr)
    }

    async fn close(&self) -> Result<(), rvoip_sip_transport::Error> {
        Ok(())
    }

    fn is_closed(&self) -> bool {
        false
    }
}

struct Harness {
    transport: Arc<MockTransport>,
    tx: mpsc::Sender<TransportEvent>,
    proxy: Arc<StatefulProxy>,
    _tm: Arc<TransactionManager>,
    _proxy_task: tokio::task::JoinHandle<()>,
}

impl Harness {
    async fn new_with_config(route: RouteDecision, config: ProxyConfig) -> Self {
        let _ = tracing_subscriber::fmt()
            .with_env_filter("rvoip_sip_proxy=debug,rvoip_sip_dialog=warn")
            .with_test_writer()
            .try_init();
        let proxy_addr: SocketAddr = PROXY_ADDR.parse().unwrap();
        let transport = Arc::new(MockTransport::new(proxy_addr));
        let (tx, rx) = mpsc::channel(64);
        let (tm, events) = TransactionManager::new(transport.clone(), rx, Some(32))
            .await
            .expect("TransactionManager::new");
        let tm = Arc::new(tm);

        let route_clone = route.clone();
        let route_fn: RouteFn = Arc::new(move |_req: &Request| Some(route_clone.clone()));
        let proxy = StatefulProxy::with_config(tm.clone(), route_fn, config);
        let proxy_task = proxy.clone().run(events);

        Harness {
            transport,
            tx,
            proxy,
            _tm: tm,
            _proxy_task: proxy_task,
        }
    }

    async fn new(route: RouteDecision) -> Self {
        Self::new_with_config(route, ProxyConfig::default()).await
    }

    async fn inject(&self, message: Message, source: SocketAddr) {
        let event = TransportEvent::MessageReceived {
            message,
            source,
            destination: self.transport.local_addr,
            transport_type: TransportType::Udp,
            flow_id: None,
            raw_bytes: None,
            timing: None,
            connection_metadata: None,
        };
        self.tx.send(event).await.expect("inject");
    }

    async fn wait_for<F>(&self, deadline_ms: u64, predicate: F) -> (Message, SocketAddr)
    where
        F: Fn(&Message, &SocketAddr) -> bool,
    {
        let start = std::time::Instant::now();
        loop {
            let sent = self.transport.sent().await;
            if let Some((m, d)) = sent.iter().find(|(m, d)| predicate(m, d)) {
                return (m.clone(), *d);
            }
            if start.elapsed() > Duration::from_millis(deadline_ms) {
                panic!(
                    "Timed out after {}ms; sent: {:?}",
                    deadline_ms,
                    sent.iter()
                        .map(|(m, d)| format!("{} -> {}", short(m), d))
                        .collect::<Vec<_>>()
                );
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

fn short(m: &Message) -> String {
    match m {
        Message::Request(r) => format!("REQ {}", r.method()),
        Message::Response(r) => format!("RESP {}", r.status()),
    }
}

fn build_uac_invite(call_id: &str) -> Request {
    SimpleRequestBuilder::new(Method::Invite, "sip:bob@10.0.0.10:5060")
        .unwrap()
        .from("Alice", "sip:alice@uac.example.com", Some("alicetag"))
        .to("Bob", "sip:bob@10.0.0.10:5060", None)
        .call_id(call_id)
        .cseq(1)
        .contact("sip:alice@10.0.0.5:5060", None)
        .header(TypedHeader::Via(
            Via::new(
                "SIP",
                "2.0",
                "UDP",
                "10.0.0.5",
                Some(5060),
                vec![Param::branch("z9hG4bK-uac-polish")],
            )
            .unwrap(),
        ))
        .max_forwards(70)
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build()
}

// ───────────────────── 1) Loop detection ─────────────────────

#[tokio::test]
async fn inbound_with_our_branch_in_via_stack_returns_482() {
    let uas_addr: SocketAddr = UAS_ADDR.parse().unwrap();
    let harness = Harness::new(RouteDecision::to(uas_addr)).await;

    // First inbound INVITE — proxy forwards it and stamps a fresh
    // proxy branch we can inspect.
    let invite_a = build_uac_invite("loop-a");
    harness
        .inject(Message::Request(invite_a), UAC_ADDR.parse().unwrap())
        .await;
    let (forwarded, _) = harness
        .wait_for(1500, |m, d| {
            matches!(m, Message::Request(r) if r.method() == Method::Invite) && *d == uas_addr
        })
        .await;
    let Message::Request(forwarded) = forwarded else {
        unreachable!();
    };
    let proxy_branch = forwarded
        .via_headers()
        .first()
        .and_then(|v| v.branch().map(|b| b.to_string()))
        .expect("proxy branch present");
    assert!(proxy_branch.starts_with("z9hG4bK-proxy-"));

    // Second inbound INVITE — but its Via stack carries the proxy's
    // own branch on top. RFC 3261 §16.6 step 4 → 482 Loop Detected.
    let looped_invite = SimpleRequestBuilder::new(Method::Invite, "sip:bob@10.0.0.10:5060")
        .unwrap()
        .from("Mallory", "sip:m@evil.example.com", Some("mtag"))
        .to("Bob", "sip:bob@10.0.0.10:5060", None)
        .call_id("loop-b")
        .cseq(1)
        .contact("sip:m@10.0.0.99:5060", None)
        .header(TypedHeader::Via(
            Via::new(
                "SIP",
                "2.0",
                "UDP",
                "10.0.0.99",
                Some(5060),
                vec![Param::branch(proxy_branch.clone())],
            )
            .unwrap(),
        ))
        .max_forwards(70)
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build();

    harness
        .inject(
            Message::Request(looped_invite),
            "10.0.0.99:5060".parse().unwrap(),
        )
        .await;

    // 482 Loop Detected must come back upstream. The proxy MUST NOT
    // forward this looped INVITE downstream — check by counting UAS
    // forwards.
    let _ = harness
        .wait_for(
            1500,
            |m, _| matches!(m, Message::Response(r) if r.status() == StatusCode::LoopDetected),
        )
        .await;
    let uas_invites = harness
        .transport
        .sent()
        .await
        .iter()
        .filter(|(m, d)| {
            matches!(m, Message::Request(r) if r.method() == Method::Invite) && *d == uas_addr
        })
        .count();
    assert_eq!(
        uas_invites, 1,
        "looped INVITE must not produce a second forward — only the original INVITE should hit UAS"
    );
}

// ───────────────────── 2) Timer C reset on 1xx ─────────────────────

#[tokio::test]
async fn timer_c_resets_on_1xx_and_does_not_fire_408() {
    // Tight Timer C — 200ms. Without the reset behaviour, a leg that
    // emits a 1xx every 100ms would still fire 408 after 200ms. With
    // the reset, the timer restarts on each 1xx and 408 never comes.
    let config = ProxyConfig {
        timer_c: Duration::from_millis(200),
        enforce_max_forwards: true,
    };
    let uas_addr: SocketAddr = UAS_ADDR.parse().unwrap();
    let harness = Harness::new_with_config(RouteDecision::to(uas_addr), config).await;
    let invite = build_uac_invite("timer-c-reset");
    harness
        .inject(Message::Request(invite), UAC_ADDR.parse().unwrap())
        .await;

    // Capture the forwarded INVITE so we can build matching 180s.
    let (forwarded, _) = harness
        .wait_for(1500, |m, d| {
            matches!(m, Message::Request(r) if r.method() == Method::Invite) && *d == uas_addr
        })
        .await;
    let Message::Request(forwarded) = forwarded else {
        unreachable!();
    };

    // Inject 4 × 180 Ringing spaced 100ms apart (total 400ms,
    // exceeding the 200ms Timer C). With per-1xx reset, no 408.
    for _ in 0..4 {
        let ringing =
            SimpleResponseBuilder::response_from_request(&forwarded, StatusCode::Ringing, None)
                .build();
        harness.inject(Message::Response(ringing), uas_addr).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Look at the sent log — there must be no 408 Request Timeout
    // anywhere.
    let sent = harness.transport.sent().await;
    let timeouts = sent
        .iter()
        .filter(
            |(m, _)| matches!(m, Message::Response(r) if r.status() == StatusCode::RequestTimeout),
        )
        .count();
    assert_eq!(
        timeouts, 0,
        "Timer C must reset on every 1xx; saw {} 408s after 4×180 Ringing within the timer window",
        timeouts
    );
}

// ───────────────────── 3) RedirectReceived event ─────────────────────

#[tokio::test]
async fn redirect_3xx_emits_event_with_contact_uris() {
    let uas_addr: SocketAddr = UAS_ADDR.parse().unwrap();
    let harness = Harness::new(RouteDecision::to(uas_addr)).await;
    let mut events = harness.proxy.subscribe_events();
    let invite = build_uac_invite("redirect-event");
    harness
        .inject(Message::Request(invite), UAC_ADDR.parse().unwrap())
        .await;

    // Capture forwarded INVITE so we can copy its Via stack onto
    // the redirect response.
    let (forwarded, _) = harness
        .wait_for(1500, |m, d| {
            matches!(m, Message::Request(r) if r.method() == Method::Invite) && *d == uas_addr
        })
        .await;
    let Message::Request(forwarded) = forwarded else {
        unreachable!();
    };

    // UAS returns 302 Moved Temporarily with a Contact: redirect URI.
    let redirect = SimpleResponseBuilder::response_from_request(
        &forwarded,
        StatusCode::MovedTemporarily,
        None,
    )
    .contact("sip:bob@new-location.example.com:5060", None)
    .build();
    harness.inject(Message::Response(redirect), uas_addr).await;

    // Subscriber sees the event.
    let evt = tokio::time::timeout(Duration::from_millis(1500), events.recv())
        .await
        .expect("event arrived")
        .expect("event channel still open");
    let ProxyEvent::RedirectReceived {
        status, contacts, ..
    } = evt;
    assert_eq!(status, StatusCode::MovedTemporarily);
    assert_eq!(contacts.len(), 1);
    assert_eq!(
        contacts[0].to_string(),
        "sip:bob@new-location.example.com:5060"
    );

    // The 302 still forwards upstream to the UAC (observability-only).
    let _ = harness
        .wait_for(
            1500,
            |m, _| matches!(m, Message::Response(r) if r.status() == StatusCode::MovedTemporarily),
        )
        .await;
}

// ─────────────────── 4) RedirectInterceptor re-fork ───────────────────

#[tokio::test]
async fn redirect_interceptor_refork_swallows_3xx_and_spawns_new_leg() {
    use rvoip_sip_proxy::{RedirectDecision, RedirectInfo, RedirectInterceptor};

    struct ReForkToBackup {
        backup: SocketAddr,
    }

    #[async_trait]
    impl RedirectInterceptor for ReForkToBackup {
        async fn on_redirect(&self, _info: RedirectInfo) -> Option<RedirectDecision> {
            Some(RedirectDecision::ReFork {
                mode: rvoip_sip_proxy::ForkMode::Sequential,
                targets: vec![self.backup],
            })
        }
    }

    let uas_addr: SocketAddr = UAS_ADDR.parse().unwrap();
    let backup_addr: SocketAddr = "10.0.0.99:5060".parse().unwrap();
    let harness = Harness::new(RouteDecision::to(uas_addr)).await;
    harness
        .proxy
        .set_redirect_interceptor(Some(Arc::new(ReForkToBackup {
            backup: backup_addr,
        })));

    harness
        .inject(
            Message::Request(build_uac_invite("redirect-refork")),
            UAC_ADDR.parse().unwrap(),
        )
        .await;

    // First INVITE goes to the original UAS.
    let (forwarded, _) = harness
        .wait_for(1500, |m, d| {
            matches!(m, Message::Request(r) if r.method() == Method::Invite) && *d == uas_addr
        })
        .await;
    let Message::Request(forwarded) = forwarded else {
        unreachable!();
    };

    // UAS returns 302 — interceptor swallows it and re-forks to backup.
    let redirect = SimpleResponseBuilder::response_from_request(
        &forwarded,
        StatusCode::MovedTemporarily,
        None,
    )
    .contact("sip:bob@elsewhere.example.com:5060", None)
    .build();
    harness.inject(Message::Response(redirect), uas_addr).await;

    // Re-fork INVITE lands on the backup.
    let _ = harness
        .wait_for(1500, |m, d| {
            matches!(m, Message::Request(r) if r.method() == Method::Invite) && *d == backup_addr
        })
        .await;

    // Settle briefly, then verify the 302 did NOT reach the UAC.
    tokio::time::sleep(Duration::from_millis(150)).await;
    let saw_302_upstream = harness.transport.sent().await.iter().any(|(m, d)| {
        matches!(m, Message::Response(r) if r.status() == StatusCode::MovedTemporarily)
            && *d == UAC_ADDR.parse::<SocketAddr>().unwrap()
    });
    assert!(
        !saw_302_upstream,
        "interceptor returned ReFork — 302 must not propagate upstream"
    );
}
