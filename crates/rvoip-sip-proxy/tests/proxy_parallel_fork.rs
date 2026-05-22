//! Acceptance tests for Phase 7 — forking + 3xx-aware response
//! aggregation.
//!
//! Each test fans a single inbound INVITE to multiple UAS destinations
//! and verifies the proxy's §16.7 response context: the first 2xx
//! wins, siblings get CANCELed, sequential mode advances on failure,
//! and parallel mode picks the "best" failure when all legs return
//! errors.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;
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
use rvoip_sip_proxy::{RouteDecision, RouteFn, StatefulProxy};
use rvoip_sip_transport::transport::TransportType;
use rvoip_sip_transport::TransportEvent;
use tokio::sync::{mpsc, Mutex};

const PROXY_ADDR: &str = "127.0.0.1:5060";
const UAC_ADDR: &str = "10.0.0.5:5060";
const UAS_A: &str = "10.0.0.20:5060";
const UAS_B: &str = "10.0.0.30:5060";
const UAS_C: &str = "10.0.0.40:5060";

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
    _tm: Arc<TransactionManager>,
    _proxy_task: tokio::task::JoinHandle<()>,
}

impl Harness {
    async fn new(route: RouteDecision) -> Self {
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
        let proxy = StatefulProxy::new(tm.clone(), route_fn);
        let proxy_task = proxy.run(events);

        Harness {
            transport,
            tx,
            _tm: tm,
            _proxy_task: proxy_task,
        }
    }

    async fn inject(&self, message: Message, source: SocketAddr) {
        let event = TransportEvent::MessageReceived {
            message,
            source,
            destination: self.transport.local_addr,
            transport_type: TransportType::Udp,
            raw_bytes: None,
        };
        self.tx.send(event).await.expect("inject");
    }

    /// Block until the captured send log matches `predicate`. Returns
    /// the matching `(message, destination)`.
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
                    "Timed out after {}ms; sent {}: {:?}",
                    deadline_ms,
                    sent.len(),
                    sent.iter()
                        .map(|(m, d)| format!("{} -> {}", short(m), d))
                        .collect::<Vec<_>>()
                );
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Wait until every destination in `dests` has received at least
    /// one INVITE, then return a map `dest → forwarded INVITE`.
    async fn wait_for_invites(&self, dests: &[SocketAddr]) -> HashMap<SocketAddr, Request> {
        let start = std::time::Instant::now();
        loop {
            let sent = self.transport.sent().await;
            let mut found: HashMap<SocketAddr, Request> = HashMap::new();
            for (msg, dest) in &sent {
                if let Message::Request(req) = msg {
                    if req.method() == Method::Invite && dests.contains(dest) {
                        found.insert(*dest, req.clone());
                    }
                }
            }
            if dests.iter().all(|d| found.contains_key(d)) {
                return found;
            }
            if start.elapsed() > Duration::from_millis(2000) {
                panic!(
                    "Timed out waiting for INVITEs to {:?}; got: {:?}",
                    dests,
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
                vec![Param::branch("z9hG4bK-uac-fork-test")],
            )
            .unwrap(),
        ))
        .max_forwards(70)
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build()
}

#[tokio::test]
async fn parallel_fork_fans_out_to_every_target() {
    let route = RouteDecision::parallel(vec![
        UAS_A.parse().unwrap(),
        UAS_B.parse().unwrap(),
        UAS_C.parse().unwrap(),
    ]);
    let harness = Harness::new(route).await;
    let invite = build_uac_invite("parallel-fanout");

    harness
        .inject(Message::Request(invite), UAC_ADDR.parse().unwrap())
        .await;

    let targets = vec![
        UAS_A.parse().unwrap(),
        UAS_B.parse().unwrap(),
        UAS_C.parse().unwrap(),
    ];
    let invites = harness.wait_for_invites(&targets).await;
    // Each leg carries a distinct proxy branch.
    let proxy_branches: std::collections::HashSet<String> = invites
        .values()
        .filter_map(|req| {
            let vias = req.via_headers();
            vias.first().and_then(|v| v.branch().map(|b| b.to_string()))
        })
        .collect();
    assert_eq!(
        proxy_branches.len(),
        3,
        "each parallel leg must have its own proxy branch; got {:?}",
        proxy_branches
    );
    for branch in &proxy_branches {
        assert!(branch.starts_with("z9hG4bK-proxy-"));
    }
}

#[tokio::test]
async fn first_200_wins_and_cancels_siblings() {
    let uas_a: SocketAddr = UAS_A.parse().unwrap();
    let uas_b: SocketAddr = UAS_B.parse().unwrap();
    let uas_c: SocketAddr = UAS_C.parse().unwrap();
    let route = RouteDecision::parallel(vec![uas_a, uas_b, uas_c]);
    let harness = Harness::new(route).await;
    let invite = build_uac_invite("first-200-wins");

    harness
        .inject(Message::Request(invite), UAC_ADDR.parse().unwrap())
        .await;

    // Wait for all 3 INVITEs.
    let invites = harness.wait_for_invites(&[uas_a, uas_b, uas_c]).await;

    // UAS B answers 200 OK.
    let uas_b_invite = &invites[&uas_b];
    let resp =
        SimpleResponseBuilder::response_from_request(uas_b_invite, StatusCode::Ok, Some("OK"))
            .build();
    harness.inject(Message::Response(resp), uas_b).await;

    // Upstream 200 OK forwarded back towards UAC.
    let (msg, dest) = harness
        .wait_for(1500, |m, d| {
            matches!(m, Message::Response(r) if r.status() == StatusCode::Ok)
                && *d != uas_a
                && *d != uas_b
                && *d != uas_c
        })
        .await;
    let Message::Response(_) = msg else {
        unreachable!();
    };
    assert!(
        dest.to_string().starts_with("10.0.0.5") || dest.to_string().starts_with("127.0.0.1"),
        "200 OK should route towards UAC, got {}",
        dest
    );

    // CANCEL fanned out to UAS A and UAS C.
    harness
        .wait_for(1500, |m, d| {
            matches!(m, Message::Request(r) if r.method() == Method::Cancel) && *d == uas_a
        })
        .await;
    harness
        .wait_for(1500, |m, d| {
            matches!(m, Message::Request(r) if r.method() == Method::Cancel) && *d == uas_c
        })
        .await;
}

#[tokio::test]
async fn sequential_fork_advances_on_failure_and_succeeds_on_later_target() {
    let uas_a: SocketAddr = UAS_A.parse().unwrap();
    let uas_b: SocketAddr = UAS_B.parse().unwrap();
    let uas_c: SocketAddr = UAS_C.parse().unwrap();
    let route = RouteDecision::sequential(vec![uas_a, uas_b, uas_c]);
    let harness = Harness::new(route).await;
    let invite = build_uac_invite("sequential-fail-then-succeed");

    harness
        .inject(Message::Request(invite), UAC_ADDR.parse().unwrap())
        .await;

    // Wait for first leg (to UAS A) only.
    let (msg, _) = harness
        .wait_for(1500, |m, d| {
            matches!(m, Message::Request(r) if r.method() == Method::Invite) && *d == uas_a
        })
        .await;
    let Message::Request(leg_a) = msg else {
        unreachable!();
    };
    // UAS A returns 404.
    let resp =
        SimpleResponseBuilder::response_from_request(&leg_a, StatusCode::NotFound, None).build();
    harness.inject(Message::Response(resp), uas_a).await;

    // Proxy advances to UAS B.
    let (msg, _) = harness
        .wait_for(2000, |m, d| {
            matches!(m, Message::Request(r) if r.method() == Method::Invite) && *d == uas_b
        })
        .await;
    let Message::Request(leg_b) = msg else {
        unreachable!();
    };
    // UAS B answers 200 OK.
    let resp =
        SimpleResponseBuilder::response_from_request(&leg_b, StatusCode::Ok, Some("OK")).build();
    harness.inject(Message::Response(resp), uas_b).await;

    // 200 OK forwarded upstream.
    let _ = harness
        .wait_for(1500, |m, d| {
            matches!(m, Message::Response(r) if r.status() == StatusCode::Ok)
                && *d != uas_a
                && *d != uas_b
                && *d != uas_c
        })
        .await;

    // UAS C must never have been tried.
    let sent = harness.transport.sent().await;
    let c_invites = sent
        .iter()
        .filter(|(m, d)| {
            matches!(m, Message::Request(r) if r.method() == Method::Invite) && *d == uas_c
        })
        .count();
    assert_eq!(
        c_invites, 0,
        "sequential fork must NOT have forwarded to UAS C after UAS B 2xx"
    );
}

#[tokio::test]
async fn all_legs_fail_picks_lowest_status_upstream() {
    let uas_a: SocketAddr = UAS_A.parse().unwrap();
    let uas_b: SocketAddr = UAS_B.parse().unwrap();
    let route = RouteDecision::parallel(vec![uas_a, uas_b]);
    let harness = Harness::new(route).await;
    let invite = build_uac_invite("all-fail-best-failure");

    harness
        .inject(Message::Request(invite), UAC_ADDR.parse().unwrap())
        .await;

    let invites = harness.wait_for_invites(&[uas_a, uas_b]).await;

    // UAS A returns 503 Service Unavailable, UAS B returns 404 Not Found.
    // §16.7 step 6: pick the lowest-class final (404 beats 503).
    let resp_a = SimpleResponseBuilder::response_from_request(
        &invites[&uas_a],
        StatusCode::ServiceUnavailable,
        None,
    )
    .build();
    let resp_b =
        SimpleResponseBuilder::response_from_request(&invites[&uas_b], StatusCode::NotFound, None)
            .build();
    harness.inject(Message::Response(resp_a), uas_a).await;
    harness.inject(Message::Response(resp_b), uas_b).await;

    let (msg, _) = harness
        .wait_for(1500, |m, d| {
            matches!(m, Message::Response(r) if r.status() == StatusCode::NotFound)
                && *d != uas_a
                && *d != uas_b
        })
        .await;
    let Message::Response(_) = msg else {
        unreachable!();
    };
}

#[tokio::test]
async fn global_6xx_wins_over_lower_class_failures() {
    let uas_a: SocketAddr = UAS_A.parse().unwrap();
    let uas_b: SocketAddr = UAS_B.parse().unwrap();
    let route = RouteDecision::parallel(vec![uas_a, uas_b]);
    let harness = Harness::new(route).await;
    let invite = build_uac_invite("6xx-wins");

    harness
        .inject(Message::Request(invite), UAC_ADDR.parse().unwrap())
        .await;
    let invites = harness.wait_for_invites(&[uas_a, uas_b]).await;

    // UAS A returns 404, UAS B returns 603 Decline (global failure).
    let resp_a =
        SimpleResponseBuilder::response_from_request(&invites[&uas_a], StatusCode::NotFound, None)
            .build();
    let resp_b =
        SimpleResponseBuilder::response_from_request(&invites[&uas_b], StatusCode::Decline, None)
            .build();
    harness.inject(Message::Response(resp_a), uas_a).await;
    harness.inject(Message::Response(resp_b), uas_b).await;

    // 6xx wins.
    let (_, _) = harness
        .wait_for(1500, |m, d| {
            matches!(m, Message::Response(r) if r.status() == StatusCode::Decline)
                && *d != uas_a
                && *d != uas_b
        })
        .await;
}
