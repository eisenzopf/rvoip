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
use rvoip_sip_proxy::{ProxyConfig, ProxyRuntimeOptions, RouteDecision, RouteFn, StatefulProxy};
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
    proxy: Arc<StatefulProxy>,
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
        let uas_addr: SocketAddr = UAS_ADDR.parse().unwrap();
        let route_fn: RouteFn = Arc::new(move |_req: &Request| Some(RouteDecision::to(uas_addr)));
        Self::new_with_options_and_route(
            config,
            ProxyRuntimeOptions::default().with_short_timer_c_for_tests(),
            route_fn,
        )
        .await
    }

    async fn new_with_options_and_route(
        config: ProxyConfig,
        options: ProxyRuntimeOptions,
        route_fn: RouteFn,
    ) -> Self {
        init_tracing();
        let proxy_addr: SocketAddr = PROXY_ADDR.parse().unwrap();
        let transport = Arc::new(MockTransport::new(proxy_addr));
        let (tx, rx) = mpsc::channel(32);
        let (tm, events) = TransactionManager::new(transport.clone(), rx, Some(16))
            .await
            .expect("TransactionManager::new");
        let tm = Arc::new(tm);

        let proxy = StatefulProxy::with_options(tm.clone(), route_fn, config, options);
        let proxy_task = proxy.clone().run(events);

        Harness {
            transport,
            tx,
            tm,
            proxy,
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
            flow_id: None,
            raw_bytes: None,
            timing: None,
            connection_metadata: None,
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
    build_uac_invite_with_branch(call_id, "z9hG4bK-uac-original")
}

fn build_uac_invite_with_branch(call_id: &str, branch: &str) -> Request {
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
                vec![Param::branch(branch)],
            )
            .unwrap(),
        ))
        .max_forwards(70)
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build()
}

fn build_uac_cancel(call_id: &str, branch: &str) -> Request {
    SimpleRequestBuilder::new(Method::Cancel, "sip:bob@10.0.0.10:5060")
        .unwrap()
        .from("Alice", "sip:alice@uac.example.com", Some("alicetag"))
        .to("Bob", "sip:bob@10.0.0.10:5060", None)
        .call_id(call_id)
        .cseq(1)
        .header(TypedHeader::Via(
            Via::new(
                "SIP",
                "2.0",
                "UDP",
                "10.0.0.5",
                Some(5060),
                vec![Param::branch(branch)],
            )
            .unwrap(),
        ))
        .max_forwards(70)
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build()
}

fn build_uac_options(call_id: &str, branch: &str) -> Request {
    SimpleRequestBuilder::new(Method::Options, "sip:bob@10.0.0.10:5060")
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
                vec![Param::branch(branch)],
            )
            .unwrap(),
        ))
        .max_forwards(70)
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build()
}

fn build_uac_ack_with_branch(call_id: &str, branch: &str) -> Request {
    SimpleRequestBuilder::new(Method::Ack, "sip:bob@10.0.0.10:5060")
        .unwrap()
        .from("Alice", "sip:alice@uac.example.com", Some("alicetag"))
        .to("Bob", "sip:bob@10.0.0.10:5060", Some("bobtag"))
        .call_id(call_id)
        .cseq(1)
        .header(TypedHeader::Via(
            Via::new(
                "SIP",
                "2.0",
                "UDP",
                "10.0.0.5",
                Some(5060),
                vec![Param::branch(branch)],
            )
            .unwrap(),
        ))
        .max_forwards(70)
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build()
}

fn build_uac_ack(call_id: &str) -> Request {
    build_uac_ack_with_branch(call_id, "z9hG4bK-2xx-ack")
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
    assert_eq!(
        harness.proxy.retention_snapshot().known_branches,
        0,
        "the default-disabled legacy detector must not retain stateful branches"
    );
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
async fn downstream_100_is_suppressed_but_180_is_forwarded() {
    let harness = Harness::new().await;
    harness
        .inject(
            Message::Request(build_uac_invite("provisional-filter")),
            UAC_ADDR.parse().unwrap(),
        )
        .await;
    let uas_addr: SocketAddr = UAS_ADDR.parse().unwrap();
    let (forwarded, _) = harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Request(request) if request.method() == Method::Invite)
                && *destination == uas_addr
        })
        .await;
    let Message::Request(forwarded) = forwarded else {
        unreachable!();
    };

    let trying = SimpleResponseBuilder::response_from_request(
        &forwarded,
        StatusCode::Trying,
        Some("Trying"),
    )
    .build();
    harness.inject(Message::Response(trying), uas_addr).await;
    tokio::time::sleep(Duration::from_millis(75)).await;
    let forwarded_trying = harness.transport.sent().await.iter().any(
        |(message, destination)| {
            matches!(message, Message::Response(response) if response.status() == StatusCode::Trying)
                && *destination != uas_addr
        },
    );
    assert!(!forwarded_trying, "a proxy MUST suppress downstream 100");

    let ringing = SimpleResponseBuilder::response_from_request(
        &forwarded,
        StatusCode::Ringing,
        Some("Ringing"),
    )
    .build();
    harness.inject(Message::Response(ringing), uas_addr).await;
    harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Response(response)
                if response.status() == StatusCode::Ringing)
                && *destination != uas_addr
        })
        .await;
}

#[test]
fn production_timer_c_default_is_strictly_greater_than_three_minutes() {
    assert!(ProxyConfig::default().timer_c > Duration::from_secs(180));
}

#[tokio::test]
async fn timer_c_fires_408_upstream_on_stalled_invite() {
    let config = ProxyConfig {
        timer_c: Duration::from_millis(150),
        ..ProxyConfig::default()
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

#[tokio::test(start_paused = true)]
async fn timer_c_calling_expiry_records_408_and_terminates_exact_client_transaction() {
    let config = ProxyConfig {
        timer_c: Duration::from_millis(200),
        ..ProxyConfig::default()
    };
    let harness = Harness::new_with_config(config).await;
    harness
        .inject(
            Message::Request(build_uac_invite("timer-c-exact-termination")),
            UAC_ADDR.parse().unwrap(),
        )
        .await;
    let uas_addr: SocketAddr = UAS_ADDR.parse().unwrap();
    harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Request(request) if request.method() == Method::Invite)
                && *destination == uas_addr
        })
        .await;
    assert_eq!(
        harness.tm.retention_counts().client_transactions,
        1,
        "the downstream INVITE client transaction must be active before Timer C"
    );

    tokio::time::advance(Duration::from_millis(201)).await;
    for _ in 0..64 {
        tokio::task::yield_now().await;
        if harness.transport.sent().await.iter().any(|(message, _)| {
            matches!(message, Message::Response(response)
                    if response.status() == StatusCode::RequestTimeout)
        }) && harness.tm.retention_counts().client_transactions == 0
        {
            break;
        }
    }

    assert!(
        harness.transport.sent().await.iter().any(|(message, _)| {
            matches!(message, Message::Response(response)
                    if response.status() == StatusCode::RequestTimeout)
        }),
        "Timer C Calling expiry must record and select a branch-local 408"
    );
    assert_eq!(
        harness.tm.retention_counts().client_transactions,
        0,
        "Timer C must explicitly terminate the exact downstream client transaction"
    );
    let snapshot = harness.proxy.retention_snapshot();
    assert_eq!(snapshot.timer_c_entries, 0);
    assert_eq!(snapshot.timer_c_heap_entries, 0);
}

#[tokio::test]
async fn timer_c_cancels_a_proceeding_branch_and_forwards_its_final() {
    let config = ProxyConfig {
        timer_c: Duration::from_millis(125),
        ..ProxyConfig::default()
    };
    let harness = Harness::new_with_config(config).await;
    harness
        .inject(
            Message::Request(build_uac_invite("timer-c-proceeding")),
            UAC_ADDR.parse().unwrap(),
        )
        .await;
    let uas_addr: SocketAddr = UAS_ADDR.parse().unwrap();
    let (forwarded, _) = harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Request(request) if request.method() == Method::Invite)
                && *destination == uas_addr
        })
        .await;
    let Message::Request(forwarded) = forwarded else {
        unreachable!();
    };
    let ringing =
        SimpleResponseBuilder::response_from_request(&forwarded, StatusCode::Ringing, None).build();
    harness.inject(Message::Response(ringing), uas_addr).await;

    harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Request(request) if request.method() == Method::Cancel)
                && *destination == uas_addr
        })
        .await;

    let terminated = SimpleResponseBuilder::response_from_request(
        &forwarded,
        StatusCode::RequestTerminated,
        None,
    )
    .build();
    harness
        .inject(Message::Response(terminated), uas_addr)
        .await;
    harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Response(response)
                if response.status() == StatusCode::RequestTerminated)
                && *destination != uas_addr
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn timer_c_resets_only_for_101_through_199_and_dispatches_one_cancel() {
    let config = ProxyConfig {
        timer_c: Duration::from_millis(200),
        ..ProxyConfig::default()
    };
    let harness = Harness::new_with_config(config).await;
    harness
        .inject(
            Message::Request(build_uac_invite("timer-c-paused-reset")),
            UAC_ADDR.parse().unwrap(),
        )
        .await;
    let uas_addr: SocketAddr = UAS_ADDR.parse().unwrap();
    let (forwarded, _) = harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Request(request) if request.method() == Method::Invite)
                && *destination == uas_addr
        })
        .await;
    let Message::Request(forwarded) = forwarded else {
        unreachable!();
    };

    tokio::time::advance(Duration::from_millis(150)).await;
    let ringing =
        SimpleResponseBuilder::response_from_request(&forwarded, StatusCode::Ringing, None).build();
    harness.inject(Message::Response(ringing), uas_addr).await;
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }

    tokio::time::advance(Duration::from_millis(100)).await;
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        harness
            .transport
            .sent()
            .await
            .iter()
            .filter(|(message, destination)| {
                matches!(message, Message::Request(request) if request.method() == Method::Cancel)
                    && *destination == uas_addr
            })
            .count(),
        0,
        "a 101-199 response must reset Timer C"
    );
    assert_eq!(harness.proxy.retention_snapshot().timer_c_entries, 1);

    tokio::time::advance(Duration::from_millis(101)).await;
    for _ in 0..32 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        harness
            .transport
            .sent()
            .await
            .iter()
            .filter(|(message, destination)| {
                matches!(message, Message::Request(request) if request.method() == Method::Cancel)
                    && *destination == uas_addr
            })
            .count(),
        1,
        "one Timer C expiry must dispatch exactly one generated CANCEL transaction"
    );
    assert_eq!(harness.proxy.retention_snapshot().timer_c_entries, 0);
}

#[tokio::test(start_paused = true)]
async fn downstream_100_does_not_reset_timer_c() {
    let config = ProxyConfig {
        timer_c: Duration::from_millis(200),
        ..ProxyConfig::default()
    };
    let harness = Harness::new_with_config(config).await;
    harness
        .inject(
            Message::Request(build_uac_invite("timer-c-100-no-reset")),
            UAC_ADDR.parse().unwrap(),
        )
        .await;
    let uas_addr: SocketAddr = UAS_ADDR.parse().unwrap();
    let (forwarded, _) = harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Request(request) if request.method() == Method::Invite)
                && *destination == uas_addr
        })
        .await;
    let Message::Request(forwarded) = forwarded else {
        unreachable!();
    };

    tokio::time::advance(Duration::from_millis(100)).await;
    let trying =
        SimpleResponseBuilder::response_from_request(&forwarded, StatusCode::Trying, None).build();
    harness.inject(Message::Response(trying), uas_addr).await;
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
    tokio::time::advance(Duration::from_millis(101)).await;
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }

    assert!(
        harness
            .transport
            .sent()
            .await
            .iter()
            .any(|(message, destination)| {
                matches!(message, Message::Request(request) if request.method() == Method::Cancel)
                    && *destination == uas_addr
            }),
        "Timer C must retain its original deadline after a downstream 100"
    );
}

#[tokio::test(start_paused = true)]
async fn response_context_capacity_returns_503_without_evicting_and_releases_after_drain() {
    let uas_addr: SocketAddr = UAS_ADDR.parse().unwrap();
    let route_fn: RouteFn = Arc::new(move |_request| Some(RouteDecision::to(uas_addr)));
    let options = ProxyRuntimeOptions::default()
        .with_response_context_capacity(1)
        .with_downstream_transaction_capacity(1)
        .with_branches_per_response_context(1);
    let harness =
        Harness::new_with_options_and_route(ProxyConfig::default(), options, route_fn).await;

    harness
        .inject(
            Message::Request(build_uac_options(
                "capacity-live-context",
                "z9hG4bK-capacity-live",
            )),
            UAC_ADDR.parse().unwrap(),
        )
        .await;
    let (first_forwarded, _) = harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Request(request) if request.method() == Method::Options)
                && *destination == uas_addr
        })
        .await;
    let Message::Request(first_forwarded) = first_forwarded else {
        unreachable!();
    };

    harness
        .inject(
            Message::Request(build_uac_options(
                "capacity-overload",
                "z9hG4bK-capacity-overload",
            )),
            UAC_ADDR.parse().unwrap(),
        )
        .await;
    harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Response(response)
                if response.status() == StatusCode::ServiceUnavailable)
                && *destination != uas_addr
        })
        .await;

    let retained = harness.proxy.retention_snapshot();
    assert_eq!(retained.response_contexts, 1);
    assert_eq!(retained.downstream_invite_indexes, 1);
    assert_eq!(retained.downstream_slot_reservations, 1);
    assert_eq!(
        harness
            .transport
            .sent()
            .await
            .iter()
            .filter(|(message, destination)| {
                matches!(message, Message::Request(request) if request.method() == Method::Options)
                    && *destination == uas_addr
            })
            .count(),
        1,
        "overload admission must not create a downstream transaction"
    );

    let ok =
        SimpleResponseBuilder::response_from_request(&first_forwarded, StatusCode::Ok, Some("OK"))
            .build();
    harness.inject(Message::Response(ok), uas_addr).await;
    harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Response(response) if response.status() == StatusCode::Ok)
                && *destination != uas_addr
        })
        .await;

    tokio::time::advance(Duration::from_secs(33)).await;
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
    let awaiting_retention = harness.proxy.retention_snapshot();
    assert_eq!(awaiting_retention.response_contexts, 1);
    assert_eq!(awaiting_retention.response_context_deadlines, 1);
    assert_eq!(awaiting_retention.response_context_deadline_heap_entries, 1);

    tokio::time::advance(Duration::from_secs(65)).await;
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
    let drained = harness.proxy.retention_snapshot();
    assert_eq!(drained.response_contexts, 0);
    assert_eq!(drained.downstream_invite_indexes, 0);
    assert_eq!(drained.downstream_slot_reservations, 0);
    assert_eq!(drained.response_context_deadlines, 0);
    assert_eq!(drained.response_context_deadline_heap_entries, 0);
}

#[tokio::test]
async fn per_context_branch_capacity_rejects_before_creating_any_leg() {
    let first: SocketAddr = UAS_ADDR.parse().unwrap();
    let second: SocketAddr = "10.0.0.11:5060".parse().unwrap();
    let route_fn: RouteFn =
        Arc::new(move |_request| Some(RouteDecision::parallel(vec![first, second])));
    let options = ProxyRuntimeOptions::default()
        .with_response_context_capacity(4)
        .with_downstream_transaction_capacity(4)
        .with_branches_per_response_context(1);
    let harness =
        Harness::new_with_options_and_route(ProxyConfig::default(), options, route_fn).await;

    harness
        .inject(
            Message::Request(build_uac_options(
                "per-context-capacity",
                "z9hG4bK-per-context-capacity",
            )),
            UAC_ADDR.parse().unwrap(),
        )
        .await;
    harness
        .wait_for(1000, |message, _| {
            matches!(message, Message::Response(response)
                if response.status() == StatusCode::ServiceUnavailable)
        })
        .await;

    assert!(
        harness
            .transport
            .sent()
            .await
            .iter()
            .all(|(message, _)| !matches!(message, Message::Request(_))),
        "an oversized fork must be rejected atomically before any branch starts"
    );
    let snapshot = harness.proxy.retention_snapshot();
    assert_eq!(snapshot.response_contexts, 0);
    assert_eq!(snapshot.downstream_invite_indexes, 0);
    assert_eq!(snapshot.downstream_slot_reservations, 0);
}

#[tokio::test]
async fn downstream_transaction_capacity_rejects_an_unreservable_fork_atomically() {
    let first: SocketAddr = UAS_ADDR.parse().unwrap();
    let second: SocketAddr = "10.0.0.11:5060".parse().unwrap();
    let route_fn: RouteFn =
        Arc::new(move |_request| Some(RouteDecision::parallel(vec![first, second])));
    let options = ProxyRuntimeOptions::default()
        .with_response_context_capacity(4)
        .with_downstream_transaction_capacity(1)
        .with_branches_per_response_context(2);
    let harness =
        Harness::new_with_options_and_route(ProxyConfig::default(), options, route_fn).await;

    harness
        .inject(
            Message::Request(build_uac_options(
                "global-downstream-capacity",
                "z9hG4bK-global-downstream-capacity",
            )),
            UAC_ADDR.parse().unwrap(),
        )
        .await;
    harness
        .wait_for(1000, |message, _| {
            matches!(message, Message::Response(response)
                if response.status() == StatusCode::ServiceUnavailable)
        })
        .await;

    assert!(
        harness
            .transport
            .sent()
            .await
            .iter()
            .all(|(message, _)| !matches!(message, Message::Request(_))),
        "a fork that cannot reserve its full downstream budget must start no branch"
    );
    let snapshot = harness.proxy.retention_snapshot();
    assert_eq!(snapshot.response_contexts, 0);
    assert_eq!(snapshot.downstream_invite_indexes, 0);
    assert_eq!(snapshot.downstream_slot_reservations, 0);
}

#[tokio::test(start_paused = true)]
async fn stateless_response_capacity_never_evicts_a_live_correlation_and_heap_drains() {
    let uas_addr: SocketAddr = UAS_ADDR.parse().unwrap();
    let route_fn: RouteFn = Arc::new(move |_request| Some(RouteDecision::to(uas_addr)));
    let options = ProxyRuntimeOptions::default().with_stateless_response_route_capacity(1);
    let harness =
        Harness::new_with_options_and_route(ProxyConfig::default(), options, route_fn).await;

    harness
        .inject(
            Message::Request(build_uac_cancel(
                "stateless-capacity-live",
                "z9hG4bK-stateless-live",
            )),
            UAC_ADDR.parse().unwrap(),
        )
        .await;
    let (first_forwarded, _) = harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Request(request) if request.method() == Method::Cancel)
                && *destination == uas_addr
        })
        .await;
    let Message::Request(first_forwarded) = first_forwarded else {
        unreachable!();
    };

    harness
        .inject(
            Message::Request(build_uac_cancel(
                "stateless-capacity-rejected",
                "z9hG4bK-stateless-rejected",
            )),
            UAC_ADDR.parse().unwrap(),
        )
        .await;
    for _ in 0..32 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        harness
            .transport
            .sent()
            .await
            .iter()
            .filter(|(message, destination)| {
                matches!(message, Message::Request(request) if request.method() == Method::Cancel)
                    && *destination == uas_addr
            })
            .count(),
        1,
        "a new stateless correlation must not evict and replace the live route"
    );

    let response =
        SimpleResponseBuilder::response_from_request(&first_forwarded, StatusCode::Ok, None)
            .build();
    harness.inject(Message::Response(response), uas_addr).await;
    harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Response(response) if response.status() == StatusCode::Ok)
                && *destination != uas_addr
        })
        .await;
    let retained = harness.proxy.retention_snapshot();
    assert_eq!(retained.stateless_response_routes, 1);
    assert_eq!(retained.stateless_response_route_deadlines, 1);
    assert_eq!(retained.stateless_response_route_deadline_heap_entries, 1);

    tokio::time::advance(Duration::from_secs(65)).await;
    for _ in 0..32 {
        tokio::task::yield_now().await;
    }
    let drained = harness.proxy.retention_snapshot();
    assert_eq!(drained.stateless_response_routes, 0);
    assert_eq!(drained.stateless_response_route_deadlines, 0);
    assert_eq!(drained.stateless_response_route_deadline_heap_entries, 0);
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
async fn absent_max_forwards_is_added_as_70_without_decrement() {
    let harness = Harness::new().await;
    let mut invite = build_uac_invite("max-forwards-absent");
    invite
        .headers
        .retain(|header| !matches!(header, TypedHeader::MaxForwards(_)));
    harness
        .inject(Message::Request(invite), UAC_ADDR.parse().unwrap())
        .await;
    let uas_addr: SocketAddr = UAS_ADDR.parse().unwrap();
    let (message, _) = harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Request(request) if request.method() == Method::Invite)
                && *destination == uas_addr
        })
        .await;
    let Message::Request(forwarded) = message else {
        unreachable!();
    };
    assert_eq!(
        forwarded.headers.iter().find_map(|header| match header {
            TypedHeader::MaxForwards(value) => Some(value.0),
            _ => None,
        }),
        Some(70)
    );
}

#[tokio::test]
async fn production_constructor_rejects_short_timer_c() {
    let proxy_addr: SocketAddr = PROXY_ADDR.parse().unwrap();
    let transport = Arc::new(MockTransport::new(proxy_addr));
    let (_tx, rx) = mpsc::channel(4);
    let (tm, _events) = TransactionManager::new(transport, rx, Some(4))
        .await
        .expect("TransactionManager::new");
    let tm = Arc::new(tm);
    let route_fn: RouteFn = Arc::new(|_| Some(RouteDecision::to(UAS_ADDR.parse().unwrap())));
    let mut config = ProxyConfig::default();
    config.timer_c = Duration::from_secs(180);
    assert!(StatefulProxy::try_with_config(tm, route_fn, config).is_err());
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
        flow_id: None,
        raw_bytes: None,
        timing: None,
        connection_metadata: None,
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

#[tokio::test]
async fn unmatched_cancel_is_forwarded_statelessly_without_local_481() {
    let harness = Harness::new().await;
    let uas_addr: SocketAddr = UAS_ADDR.parse().unwrap();
    harness
        .inject(
            Message::Request(build_uac_cancel(
                "unmatched-cancel",
                "z9hG4bK-unmatched-cancel",
            )),
            UAC_ADDR.parse().unwrap(),
        )
        .await;

    harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Request(request) if request.method() == Method::Cancel)
                && *destination == uas_addr
        })
        .await;
    tokio::time::sleep(Duration::from_millis(75)).await;
    let local_481 = harness
        .transport
        .sent()
        .await
        .iter()
        .any(|(message, destination)| {
            matches!(message, Message::Response(response)
                if response.status() == StatusCode::CallOrTransactionDoesNotExist)
                && *destination != uas_addr
        });
    assert!(
        !local_481,
        "an unmatched CANCEL must be forwarded statelessly, not answered as a UAS"
    );
}

#[tokio::test]
async fn failure_response_creates_exactly_one_downstream_transaction_ack() {
    let harness = Harness::new().await;
    let uas_addr: SocketAddr = UAS_ADDR.parse().unwrap();
    harness
        .inject(
            Message::Request(build_uac_invite("non-2xx-transaction-ack")),
            UAC_ADDR.parse().unwrap(),
        )
        .await;
    let (forwarded, _) = harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Request(request) if request.method() == Method::Invite)
                && *destination == uas_addr
        })
        .await;
    let Message::Request(forwarded) = forwarded else {
        unreachable!();
    };

    let response = SimpleResponseBuilder::response_from_request(
        &forwarded,
        StatusCode::BusyHere,
        Some("Busy Here"),
    )
    .build();
    harness.inject(Message::Response(response), uas_addr).await;

    let (ack, _) = harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Request(request) if request.method() == Method::Ack)
                && *destination == uas_addr
        })
        .await;
    let Message::Request(ack) = ack else {
        unreachable!();
    };
    assert_eq!(
        ack.first_via()
            .and_then(|via| via.branch().map(str::to_owned)),
        forwarded
            .first_via()
            .and_then(|via| via.branch().map(str::to_owned)),
        "the transaction-owned non-2xx ACK must reuse the downstream INVITE branch"
    );

    tokio::time::sleep(Duration::from_millis(75)).await;
    let ack_count = harness
        .transport
        .sent()
        .await
        .iter()
        .filter(|(message, destination)| {
            matches!(message, Message::Request(request) if request.method() == Method::Ack)
                && *destination == uas_addr
        })
        .count();
    assert_eq!(
        ack_count, 1,
        "one downstream failure response must produce exactly one transaction ACK"
    );
}

#[tokio::test]
async fn upstream_non_2xx_ack_is_consumed_and_not_forwarded() {
    let harness = Harness::new().await;
    let uas_addr: SocketAddr = UAS_ADDR.parse().unwrap();
    let call_id = "upstream-non-2xx-ack";
    harness
        .inject(
            Message::Request(build_uac_invite(call_id)),
            UAC_ADDR.parse().unwrap(),
        )
        .await;
    let (forwarded, _) = harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Request(request) if request.method() == Method::Invite)
                && *destination == uas_addr
        })
        .await;
    let Message::Request(forwarded) = forwarded else {
        unreachable!();
    };

    let response = SimpleResponseBuilder::response_from_request(
        &forwarded,
        StatusCode::BusyHere,
        Some("Busy Here"),
    )
    .build();
    harness.inject(Message::Response(response), uas_addr).await;
    harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Request(request) if request.method() == Method::Ack)
                && *destination == uas_addr
        })
        .await;

    // This ACK reuses the upstream INVITE branch and belongs to that server
    // transaction. The proxy observes it but must not send another downstream
    // ACK: the downstream client transaction already generated one.
    harness
        .inject(
            Message::Request(build_uac_ack_with_branch(call_id, "z9hG4bK-uac-original")),
            UAC_ADDR.parse().unwrap(),
        )
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let downstream_acks = harness
        .transport
        .sent()
        .await
        .iter()
        .filter(|(message, destination)| {
            matches!(message, Message::Request(request) if request.method() == Method::Ack)
                && *destination == uas_addr
        })
        .count();
    assert_eq!(
        downstream_acks, 1,
        "the upstream non-2xx ACK must not duplicate the downstream transaction ACK"
    );
}

#[tokio::test]
async fn different_branch_2xx_ack_is_forwarded_without_a_client_transaction() {
    let harness = Harness::new().await;
    let uas_addr: SocketAddr = UAS_ADDR.parse().unwrap();
    let call_id = "different-branch-2xx-ack";
    harness
        .inject(
            Message::Request(build_uac_invite(call_id)),
            UAC_ADDR.parse().unwrap(),
        )
        .await;
    let (forwarded_invite, _) = harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Request(request) if request.method() == Method::Invite)
                && *destination == uas_addr
        })
        .await;
    let Message::Request(forwarded_invite) = forwarded_invite else {
        unreachable!();
    };

    let response =
        SimpleResponseBuilder::response_from_request(&forwarded_invite, StatusCode::Ok, Some("OK"))
            .build();
    harness.inject(Message::Response(response), uas_addr).await;
    harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Response(response) if response.status() == StatusCode::Ok)
                && *destination != uas_addr
        })
        .await;

    harness
        .inject(
            Message::Request(build_uac_ack(call_id)),
            UAC_ADDR.parse().unwrap(),
        )
        .await;

    let (message, _) = harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Request(request) if request.method() == Method::Ack)
                && *destination == uas_addr
        })
        .await;
    let Message::Request(forwarded) = message else {
        unreachable!();
    };
    assert_eq!(
        forwarded.headers.iter().find_map(|header| match header {
            TypedHeader::MaxForwards(value) => Some(value.0),
            _ => None,
        }),
        Some(69)
    );
    assert_eq!(
        forwarded.via_headers().last().and_then(|via| via.branch()),
        Some("z9hG4bK-2xx-ack"),
        "the different end-to-end ACK branch must survive proxy forwarding"
    );

    let (client_transactions, _) = harness.tm.active_transactions().await;
    assert!(
        client_transactions
            .iter()
            .all(|transaction| transaction.method() != &Method::Ack),
        "a 2xx ACK must be forwarded statelessly without a client transaction"
    );
}
