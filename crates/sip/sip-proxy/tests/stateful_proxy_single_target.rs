//! Acceptance tests for Phase 6 — single-target stateful proxy.
//!
//! Validates the UAC → proxy → UAS round trip at the
//! `StatefulProxy` + `TransactionManager` boundary, and the Timer C
//! (RFC 3261 §16.8) timeout path.
//!
//! Both legs run on a single `MockTransport` that captures everything
//! the proxy sends. Inbound traffic is injected by pushing
//! `TransportEvent`s into the channel that `TransactionManager`
//! consumes — the same path real UDP / TCP transports use, just with
//! synthetic packets.

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
use rvoip_sip_proxy::{ProxyConfig, RouteDecision, RouteFn, StatefulProxy};
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
    #[allow(dead_code)] // held to keep the TransactionManager alive
    tm: Arc<TransactionManager>,
    _proxy_task: tokio::task::JoinHandle<()>,
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_sip_proxy=trace,rvoip_sip_dialog=warn")
        .with_test_writer()
        .try_init();
}

impl Harness {
    async fn new_with_config(config: ProxyConfig) -> Self {
        init_tracing();
        let proxy_addr: SocketAddr = PROXY_ADDR.parse().unwrap();
        let uas_addr: SocketAddr = UAS_ADDR.parse().unwrap();

        let transport = Arc::new(MockTransport::new(proxy_addr));
        let (tx, rx) = mpsc::channel(32);
        let (tm, events) = TransactionManager::new(transport.clone(), rx, Some(16))
            .await
            .expect("TransactionManager::new");
        let tm = Arc::new(tm);

        let route_fn: RouteFn = Arc::new(move |_req: &Request| Some(RouteDecision::to(uas_addr)));
        let proxy = StatefulProxy::with_config(tm.clone(), route_fn, config);
        let proxy_task = proxy.run(events);

        Harness {
            transport,
            tx,
            tm,
            _proxy_task: proxy_task,
        }
    }

    async fn new() -> Self {
        Self::new_with_config(ProxyConfig::default()).await
    }

    async fn inject(&self, message: Message, source: SocketAddr) {
        let event = TransportEvent::MessageReceived {
            message,
            source,
            destination: self.transport.local_addr,
            transport_type: TransportType::Udp,
            raw_bytes: None,
            timing: None,
        };
        self.tx.send(event).await.expect("inject transport event");
    }

    /// Poll until a sent message matches `predicate` or the deadline
    /// passes. Returns the matching `(message, destination)`.
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
                    "Timed out waiting for matching message after {}ms; sent so far ({}): {:#?}",
                    deadline_ms,
                    sent.len(),
                    sent.iter()
                        .map(|(m, d)| format!("{} -> {}", short(m), d))
                        .collect::<Vec<_>>(),
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
                vec![Param::branch("z9hG4bK-uac-original")],
            )
            .unwrap(),
        ))
        .max_forwards(70)
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build()
}

#[tokio::test]
async fn uac_invite_is_forwarded_to_uas_with_proxy_via_pushed() {
    let harness = Harness::new().await;
    let invite = build_uac_invite("uac-to-uas-forward");

    harness
        .inject(Message::Request(invite.clone()), UAC_ADDR.parse().unwrap())
        .await;

    let uas_addr: SocketAddr = UAS_ADDR.parse().unwrap();
    let (msg, _) = harness
        .wait_for(1000, |m, d| {
            matches!(m, Message::Request(r) if r.method() == Method::Invite) && *d == uas_addr
        })
        .await;
    let Message::Request(forwarded) = msg else {
        unreachable!();
    };

    // RFC 3261 §16.6 step 3 — Max-Forwards decremented from 70 → 69.
    let max_fwd = forwarded
        .headers
        .iter()
        .find_map(|h| match h {
            TypedHeader::MaxForwards(mf) => Some(mf.0),
            _ => None,
        })
        .expect("Max-Forwards present");
    assert_eq!(max_fwd, 69);

    // The proxy pushes its Via as a NEW typed-header above the UAC's,
    // so we expect two Via typed-headers: proxy first, UAC second.
    let vias = forwarded.via_headers();
    assert!(
        vias.len() >= 2,
        "forwarded INVITE should carry proxy + UAC Via headers, got {}",
        vias.len()
    );
    let proxy_branch = vias[0]
        .branch()
        .expect("proxy Via must carry branch")
        .to_string();
    assert!(
        proxy_branch.starts_with("z9hG4bK-proxy-"),
        "proxy branch should start with z9hG4bK-proxy-, got {}",
        proxy_branch
    );

    let uac_branch = vias[1].branch().expect("UAC Via branch survives");
    assert_eq!(uac_branch, "z9hG4bK-uac-original");
}

#[tokio::test]
async fn uas_200_ok_is_forwarded_upstream_with_proxy_via_popped() {
    let harness = Harness::new().await;
    let invite = build_uac_invite("uac-to-uas-200ok");

    harness
        .inject(Message::Request(invite.clone()), UAC_ADDR.parse().unwrap())
        .await;
    let uas_addr: SocketAddr = UAS_ADDR.parse().unwrap();
    let (forwarded_msg, _) = harness
        .wait_for(1000, |m, d| {
            matches!(m, Message::Request(r) if r.method() == Method::Invite) && *d == uas_addr
        })
        .await;
    let Message::Request(forwarded) = forwarded_msg else {
        unreachable!();
    };

    // Build a 200 OK as a real UAS would: copy the request's Via stack
    // verbatim onto the response (RFC 3261 §8.2.6.2).
    let response =
        SimpleResponseBuilder::response_from_request(&forwarded, StatusCode::Ok, Some("OK"))
            .build();

    // Inject the 200 OK from the UAS-facing side.
    harness
        .inject(Message::Response(response), UAS_ADDR.parse().unwrap())
        .await;

    // Look for the upstream 200 OK — addressed to the UAC, not the UAS.
    let (msg, dest) = harness
        .wait_for(1000, |m, d| {
            matches!(m, Message::Response(r) if r.status() == StatusCode::Ok) && *d != uas_addr
        })
        .await;
    let Message::Response(upstream_resp) = msg else {
        unreachable!();
    };
    // Server-tx response routing uses the top-Via sent-by; for a
    // mock transport with no rport handling, the destination defaults
    // to the UAC's declared sent-by address.
    assert!(
        dest.to_string().starts_with("10.0.0.5") || dest.to_string().starts_with("127.0.0.1"),
        "200 OK should route towards the UAC's Via sent-by, got {}",
        dest
    );

    // Proxy Via popped — top Via on the response is the UAC's
    // original.
    let top_via = upstream_resp.first_via().expect("Via present on response");
    let top_branch = top_via.branch().expect("top branch present");
    assert_eq!(
        top_branch, "z9hG4bK-uac-original",
        "proxy must pop its own Via — top should be UAC's"
    );
}

#[tokio::test]
async fn timer_c_fires_408_upstream_on_stalled_invite() {
    let config = ProxyConfig {
        timer_c: Duration::from_millis(150),
        enforce_max_forwards: true,
    };
    let harness = Harness::new_with_config(config).await;
    let invite = build_uac_invite("timer-c-stall");

    harness
        .inject(Message::Request(invite), UAC_ADDR.parse().unwrap())
        .await;

    // No 1xx / final from UAS — Timer C fires, proxy injects 408
    // upstream. Look for the 408 in the sent log.
    harness
        .wait_for(
            2000,
            |m, _| matches!(m, Message::Response(r) if r.status() == StatusCode::RequestTimeout),
        )
        .await;
}

#[tokio::test]
async fn max_forwards_zero_returns_483_too_many_hops() {
    let harness = Harness::new().await;
    let mut invite = build_uac_invite("max-forwards-zero");
    // Replace the existing Max-Forwards:70 with a 0.
    for header in &mut invite.headers {
        if let TypedHeader::MaxForwards(mf) = header {
            mf.0 = 0;
        }
    }

    harness
        .inject(Message::Request(invite), UAC_ADDR.parse().unwrap())
        .await;

    harness
        .wait_for(
            1000,
            |m, _| matches!(m, Message::Response(r) if r.status() == StatusCode::TooManyHops),
        )
        .await;
}

#[tokio::test]
async fn route_fn_none_returns_404_upstream() {
    let proxy_addr: SocketAddr = PROXY_ADDR.parse().unwrap();
    let transport = Arc::new(MockTransport::new(proxy_addr));
    let (tx, rx) = mpsc::channel(32);
    let (tm, events) = TransactionManager::new(transport.clone(), rx, Some(16))
        .await
        .expect("TransactionManager::new");
    let tm = Arc::new(tm);

    // Routing function rejects everything.
    let route_fn: RouteFn = Arc::new(|_req: &Request| None);
    let proxy = StatefulProxy::new(tm, route_fn);
    let _task = proxy.run(events);

    let invite = build_uac_invite("no-route");
    let event = TransportEvent::MessageReceived {
        message: Message::Request(invite),
        source: UAC_ADDR.parse().unwrap(),
        destination: proxy_addr,
        transport_type: TransportType::Udp,
        raw_bytes: None,
        timing: None,
    };
    tx.send(event).await.unwrap();

    let start = std::time::Instant::now();
    loop {
        let sent = transport.sent().await;
        if let Some(_) = sent
            .iter()
            .find(|(m, _)| matches!(m, Message::Response(r) if r.status() == StatusCode::NotFound))
        {
            return;
        }
        if start.elapsed() > Duration::from_millis(1000) {
            panic!(
                "timed out waiting for 404; sent: {:?}",
                sent.iter().map(|(m, _)| short(m)).collect::<Vec<_>>()
            );
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
