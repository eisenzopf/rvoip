//! Acceptance tests for RFC 3263 §4.3 per-leg failover in the
//! stateful proxy. Validates that `RouteDecision::parallel_with_failover`
//! / `RouteDecision::sequential_with_failover` walk per-leg candidate
//! lists on transport-level send failures.

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
use rvoip_sip_proxy::{ProxyConfig, ProxyRuntimeOptions, RouteDecision, RouteFn, StatefulProxy};
use rvoip_sip_transport::transport::TransportType;
use rvoip_sip_transport::TransportEvent;
use tokio::sync::{mpsc, Mutex};

const PROXY_ADDR: &str = "127.0.0.1:5060";
const UAC_ADDR: &str = "10.0.0.5:5060";

/// MockTransport whose `send_message` outcome is programmable
/// per-destination. The default for any unprogrammed destination is
/// `Ok(())`.
#[derive(Debug, Clone)]
struct ProgrammableTransport {
    local_addr: SocketAddr,
    sent: Arc<Mutex<Vec<(Message, SocketAddr)>>>,
    fail_addrs: Arc<Mutex<HashMap<SocketAddr, ()>>>,
}

impl ProgrammableTransport {
    fn new(local_addr: SocketAddr) -> Self {
        Self {
            local_addr,
            sent: Arc::new(Mutex::new(Vec::new())),
            fail_addrs: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn fail_for(&self, addr: SocketAddr) {
        self.fail_addrs.lock().await.insert(addr, ());
    }

    async fn sent(&self) -> Vec<(Message, SocketAddr)> {
        self.sent.lock().await.clone()
    }
}

#[async_trait]
impl rvoip_sip_transport::Transport for ProgrammableTransport {
    async fn send_message(
        &self,
        message: Message,
        destination: SocketAddr,
    ) -> Result<(), rvoip_sip_transport::Error> {
        let fails = self.fail_addrs.lock().await.contains_key(&destination);
        self.sent.lock().await.push((message, destination));
        if fails {
            Err(rvoip_sip_transport::Error::ConnectFailed(
                destination,
                std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "programmed fail"),
            ))
        } else {
            Ok(())
        }
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
    transport: Arc<ProgrammableTransport>,
    tx: mpsc::Sender<TransportEvent>,
    _tm: Arc<TransactionManager>,
    proxy: Arc<StatefulProxy>,
    _proxy_task: tokio::task::JoinHandle<()>,
}

impl Harness {
    async fn new(route_fn: RouteFn) -> Self {
        Self::new_with_config(route_fn, ProxyConfig::default()).await
    }

    async fn new_with_config(route_fn: RouteFn, config: ProxyConfig) -> Self {
        Self::new_with_options(route_fn, config, ProxyRuntimeOptions::default()).await
    }

    async fn new_with_options(
        route_fn: RouteFn,
        config: ProxyConfig,
        options: ProxyRuntimeOptions,
    ) -> Self {
        let _ = tracing_subscriber::fmt()
            .with_env_filter("rvoip_sip_proxy=trace,rvoip_sip_dialog=warn")
            .with_test_writer()
            .try_init();
        let proxy_addr: SocketAddr = PROXY_ADDR.parse().unwrap();
        let transport = Arc::new(ProgrammableTransport::new(proxy_addr));
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
            _tm: tm,
            proxy,
            _proxy_task: proxy_task,
        }
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

    async fn wait_for_send_to(&self, addr: SocketAddr, deadline_ms: u64) {
        let start = std::time::Instant::now();
        loop {
            if self.transport.sent().await.iter().any(|(_, d)| *d == addr) {
                return;
            }
            if start.elapsed() > Duration::from_millis(deadline_ms) {
                panic!(
                    "Timed out waiting for send to {}; sent so far: {:?}",
                    addr,
                    self.transport
                        .sent()
                        .await
                        .iter()
                        .map(|(_, d)| *d)
                        .collect::<Vec<_>>()
                );
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn wait_for_invite_to(&self, addr: SocketAddr) -> Request {
        let start = std::time::Instant::now();
        loop {
            if let Some(request) = self.transport.sent().await.iter().find_map(
                |(message, destination)| match message {
                    Message::Request(request)
                        if request.method() == Method::Invite && *destination == addr =>
                    {
                        Some(request.clone())
                    }
                    _ => None,
                },
            ) {
                return request;
            }
            if start.elapsed() > Duration::from_millis(1500) {
                panic!("timed out waiting for INVITE to {addr}");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn wait_for_upstream_final(&self, status: StatusCode) {
        let uac: SocketAddr = UAC_ADDR.parse().unwrap();
        let start = std::time::Instant::now();
        loop {
            if self
                .transport
                .sent()
                .await
                .iter()
                .any(|(message, destination)| {
                    matches!(
                        message,
                        Message::Response(response) if response.status() == status
                    ) && *destination == uac
                })
            {
                return;
            }
            if start.elapsed() > Duration::from_millis(1500) {
                panic!("timed out waiting for upstream {status}");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn wait_for_upstream_server_failure(&self) -> StatusCode {
        let uac: SocketAddr = UAC_ADDR.parse().unwrap();
        let start = std::time::Instant::now();
        loop {
            if let Some(status) = self.transport.sent().await.iter().find_map(
                |(message, destination)| match message {
                    Message::Response(response)
                        if response.status().as_u16() / 100 == 5 && *destination == uac =>
                    {
                        Some(response.status())
                    }
                    _ => None,
                },
            ) {
                return status;
            }
            if start.elapsed() > Duration::from_millis(1500) {
                panic!("timed out waiting for upstream 5xx");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn invite_count_to(&self, addr: SocketAddr) -> usize {
        self.transport
            .sent()
            .await
            .iter()
            .filter(|(message, destination)| {
                matches!(
                    message,
                    Message::Request(request) if request.method() == Method::Invite
                ) && *destination == addr
            })
            .count()
    }
}

fn build_uac_invite(call_id: &str) -> Request {
    SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
        .unwrap()
        .from("Alice", "sip:alice@uac.example.com", Some("alicetag"))
        .to("Bob", "sip:bob@example.com", None)
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
                vec![Param::branch("z9hG4bK-uac-failover")],
            )
            .unwrap(),
        ))
        .max_forwards(70)
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build()
}

fn response_for(request: &Request, status: StatusCode) -> Message {
    Message::Response(SimpleResponseBuilder::response_from_request(request, status, None).build())
}

fn proxy_branch(request: &Request) -> String {
    request
        .via_headers()
        .first()
        .and_then(|via| via.branch())
        .expect("forwarded request has proxy Via branch")
        .to_owned()
}

async fn settle_proxy_tasks() {
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test(start_paused = true)]
async fn failed_leg_does_not_retain_loop_branch_or_timer_c_entry() {
    let destination: SocketAddr = "10.0.0.50:5060".parse().unwrap();
    let route_fn: RouteFn = Arc::new(move |_req: &Request| Some(RouteDecision::to(destination)));
    let harness = Harness::new_with_options(
        route_fn,
        ProxyConfig::default(),
        ProxyRuntimeOptions::default().with_legacy_loop_detection_for_tests(),
    )
    .await;
    harness.transport.fail_for(destination).await;

    harness
        .inject(
            Message::Request(build_uac_invite("failed-branch-retention")),
            UAC_ADDR.parse().unwrap(),
        )
        .await;
    harness.wait_for_send_to(destination, 1500).await;
    for _ in 0..64 {
        let snapshot = harness.proxy.retention_snapshot();
        if snapshot.known_branches == 0
            && snapshot.timer_c_entries == 0
            && snapshot.timer_c_heap_entries == 0
            && snapshot.downstream_invite_indexes == 0
        {
            break;
        }
        tokio::task::yield_now().await;
    }

    let snapshot = harness.proxy.retention_snapshot();
    assert_eq!(snapshot.known_branches, 0);
    assert_eq!(snapshot.timer_c_entries, 0);
    assert_eq!(snapshot.timer_c_heap_entries, 0);
    assert_eq!(snapshot.downstream_invite_indexes, 0);
}

#[tokio::test]
async fn per_leg_failover_advances_on_send_failure_to_next_candidate() {
    // Leg 0 has two candidates: first fails, second succeeds.
    let primary: SocketAddr = SocketAddr::from_str("10.0.0.10:5060").unwrap();
    let backup: SocketAddr = SocketAddr::from_str("10.0.0.20:5060").unwrap();

    let route_fn: RouteFn = Arc::new(move |_req: &Request| {
        Some(RouteDecision::sequential_with_failover(vec![vec![
            primary, backup,
        ]]))
    });
    let harness = Harness::new(route_fn).await;
    // Program the primary to fail; backup will succeed by default.
    harness.transport.fail_for(primary).await;

    harness
        .inject(
            Message::Request(build_uac_invite("per-leg-failover")),
            UAC_ADDR.parse().unwrap(),
        )
        .await;

    // The proxy must end up sending to the backup. Both candidates
    // should be touched (the proxy first attempts the primary, sees
    // it fail, then advances to the backup within the same leg).
    harness.wait_for_send_to(backup, 1500).await;

    let dests: Vec<SocketAddr> = harness
        .transport
        .sent()
        .await
        .iter()
        .filter_map(|(m, d)| match m {
            Message::Request(r) if r.method() == Method::Invite => Some(*d),
            _ => None,
        })
        .collect();
    assert!(
        dests.contains(&primary),
        "primary candidate must have been attempted; got {:?}",
        dests
    );
    assert!(
        dests.contains(&backup),
        "backup candidate must have been attempted; got {:?}",
        dests
    );
}

#[tokio::test]
async fn parallel_with_failover_fires_first_candidate_per_leg() {
    // Two legs, each with two candidates. Default outcome: first
    // candidate of each leg succeeds, second is never tried.
    let leg_a: Vec<SocketAddr> = vec![
        "10.0.0.10:5060".parse().unwrap(),
        "10.0.0.11:5060".parse().unwrap(),
    ];
    let leg_b: Vec<SocketAddr> = vec![
        "10.0.0.20:5060".parse().unwrap(),
        "10.0.0.21:5060".parse().unwrap(),
    ];

    let leg_a_clone = leg_a.clone();
    let leg_b_clone = leg_b.clone();
    let route_fn: RouteFn = Arc::new(move |_req: &Request| {
        Some(RouteDecision::parallel_with_failover(vec![
            leg_a_clone.clone(),
            leg_b_clone.clone(),
        ]))
    });
    let harness = Harness::new(route_fn).await;

    harness
        .inject(
            Message::Request(build_uac_invite("parallel-failover")),
            UAC_ADDR.parse().unwrap(),
        )
        .await;

    harness.wait_for_send_to(leg_a[0], 1500).await;
    harness.wait_for_send_to(leg_b[0], 1500).await;

    let dests: Vec<SocketAddr> = harness
        .transport
        .sent()
        .await
        .iter()
        .filter_map(|(m, d)| match m {
            Message::Request(r) if r.method() == Method::Invite => Some(*d),
            _ => None,
        })
        .collect();
    // Backup candidates must NOT have been touched (primaries succeed).
    assert!(
        !dests.contains(&leg_a[1]),
        "leg A backup should not be tried"
    );
    assert!(
        !dests.contains(&leg_b[1]),
        "leg B backup should not be tried"
    );
}

#[tokio::test]
async fn downstream_503_advances_same_logical_leg_with_fresh_branch() {
    let primary: SocketAddr = "10.0.0.30:5060".parse().unwrap();
    let backup: SocketAddr = "10.0.0.31:5060".parse().unwrap();
    let route_fn: RouteFn = Arc::new(move |_request: &Request| {
        Some(RouteDecision::sequential_with_failover(vec![vec![
            primary, backup,
        ]]))
    });
    let harness = Harness::new(route_fn).await;

    harness
        .inject(
            Message::Request(build_uac_invite("candidate-503-advance")),
            UAC_ADDR.parse().unwrap(),
        )
        .await;
    let primary_invite = harness.wait_for_invite_to(primary).await;
    let unavailable = response_for(&primary_invite, StatusCode::ServiceUnavailable);
    harness.inject(unavailable.clone(), primary).await;
    harness.inject(unavailable, primary).await;

    let backup_invite = harness.wait_for_invite_to(backup).await;
    assert_ne!(
        proxy_branch(&primary_invite),
        proxy_branch(&backup_invite),
        "each candidate attempt must use a fresh Via branch"
    );
    let uac: SocketAddr = UAC_ADDR.parse().unwrap();
    assert!(
        !harness
            .transport
            .sent()
            .await
            .iter()
            .any(|(message, destination)| {
                matches!(
                    message,
                    Message::Response(response)
                        if response.status() == StatusCode::ServiceUnavailable
                ) && *destination == uac
            }),
        "candidate-local 503 must not be aggregated before backup settles"
    );
    assert_eq!(
        harness.invite_count_to(backup).await,
        1,
        "duplicate candidate failure must not start the replacement twice"
    );

    harness
        .inject(response_for(&backup_invite, StatusCode::BusyHere), backup)
        .await;
    harness.wait_for_upstream_final(StatusCode::BusyHere).await;
}

#[tokio::test(start_paused = true)]
async fn timer_c_calling_expiry_advances_candidate_without_upstream_408() {
    let primary: SocketAddr = "10.0.0.40:5060".parse().unwrap();
    let backup: SocketAddr = "10.0.0.41:5060".parse().unwrap();
    let route_fn: RouteFn = Arc::new(move |_request: &Request| {
        Some(RouteDecision::sequential_with_failover(vec![vec![
            primary, backup,
        ]]))
    });
    let mut config = ProxyConfig::default();
    config.timer_c = Duration::from_millis(100);
    let harness = Harness::new_with_options(
        route_fn,
        config,
        ProxyRuntimeOptions::default().with_short_timer_c_for_tests(),
    )
    .await;

    harness
        .inject(
            Message::Request(build_uac_invite("candidate-timer-c-advance")),
            UAC_ADDR.parse().unwrap(),
        )
        .await;
    let primary_invite = harness.wait_for_invite_to(primary).await;
    tokio::time::advance(Duration::from_millis(101)).await;
    settle_proxy_tasks().await;
    let backup_invite = harness.wait_for_invite_to(backup).await;

    assert_ne!(
        proxy_branch(&primary_invite),
        proxy_branch(&backup_invite),
        "Timer C candidate advancement must create a fresh transaction branch"
    );
    let uac: SocketAddr = UAC_ADDR.parse().unwrap();
    assert!(
        !harness
            .transport
            .sent()
            .await
            .iter()
            .any(|(message, destination)| {
                matches!(
                    message,
                    Message::Response(response)
                        if response.status() == StatusCode::RequestTimeout
                ) && *destination == uac
            }),
        "Timer C must not aggregate 408 while another candidate is available"
    );

    harness
        .inject(response_for(&backup_invite, StatusCode::BusyHere), backup)
        .await;
    harness.wait_for_upstream_final(StatusCode::BusyHere).await;
}

#[tokio::test(start_paused = true)]
async fn invite_transaction_timeout_advances_candidate_before_aggregation() {
    let primary: SocketAddr = "10.0.0.42:5060".parse().unwrap();
    let backup: SocketAddr = "10.0.0.43:5060".parse().unwrap();
    let route_fn: RouteFn = Arc::new(move |_request: &Request| {
        Some(RouteDecision::sequential_with_failover(vec![vec![
            primary, backup,
        ]]))
    });
    let harness = Harness::new(route_fn).await;

    harness
        .inject(
            Message::Request(build_uac_invite("candidate-transaction-timeout")),
            UAC_ADDR.parse().unwrap(),
        )
        .await;
    let primary_invite = harness.wait_for_invite_to(primary).await;
    tokio::time::advance(Duration::from_secs(33)).await;
    settle_proxy_tasks().await;
    let backup_invite = harness.wait_for_invite_to(backup).await;

    assert_ne!(
        proxy_branch(&primary_invite),
        proxy_branch(&backup_invite),
        "transaction-timeout advancement must create a fresh branch"
    );
    let uac: SocketAddr = UAC_ADDR.parse().unwrap();
    assert!(
        !harness
            .transport
            .sent()
            .await
            .iter()
            .any(|(message, destination)| {
                matches!(
                    message,
                    Message::Response(response)
                        if response.status() == StatusCode::RequestTimeout
                ) && *destination == uac
            }),
        "a no-response timeout must try the remaining candidate before 408"
    );

    harness
        .inject(response_for(&backup_invite, StatusCode::BusyHere), backup)
        .await;
    harness.wait_for_upstream_final(StatusCode::BusyHere).await;
}

#[tokio::test]
async fn received_408_is_a_real_final_and_does_not_advance_candidate() {
    let primary: SocketAddr = "10.0.0.50:5060".parse().unwrap();
    let backup: SocketAddr = "10.0.0.51:5060".parse().unwrap();
    let route_fn: RouteFn = Arc::new(move |_request: &Request| {
        Some(RouteDecision::sequential_with_failover(vec![vec![
            primary, backup,
        ]]))
    });
    let harness = Harness::new(route_fn).await;

    harness
        .inject(
            Message::Request(build_uac_invite("received-408-no-failover")),
            UAC_ADDR.parse().unwrap(),
        )
        .await;
    let primary_invite = harness.wait_for_invite_to(primary).await;
    harness
        .inject(
            response_for(&primary_invite, StatusCode::RequestTimeout),
            primary,
        )
        .await;
    harness
        .wait_for_upstream_final(StatusCode::RequestTimeout)
        .await;
    settle_proxy_tasks().await;

    assert_eq!(
        harness.invite_count_to(backup).await,
        0,
        "an actual downstream 408 is not an RFC 3263 candidate timeout"
    );
}

#[tokio::test]
async fn repeated_503_exhausts_candidate_set_without_restarting_it() {
    let primary: SocketAddr = "10.0.0.60:5060".parse().unwrap();
    let backup: SocketAddr = "10.0.0.61:5060".parse().unwrap();
    let route_fn: RouteFn = Arc::new(move |_request: &Request| {
        Some(RouteDecision::sequential_with_failover(vec![vec![
            primary, backup,
        ]]))
    });
    let harness = Harness::new(route_fn).await;

    harness
        .inject(
            Message::Request(build_uac_invite("candidate-503-exhaustion")),
            UAC_ADDR.parse().unwrap(),
        )
        .await;
    let primary_invite = harness.wait_for_invite_to(primary).await;
    harness
        .inject(
            response_for(&primary_invite, StatusCode::ServiceUnavailable),
            primary,
        )
        .await;
    let backup_invite = harness.wait_for_invite_to(backup).await;
    harness
        .inject(
            response_for(&backup_invite, StatusCode::ServiceUnavailable),
            backup,
        )
        .await;

    let final_status = harness.wait_for_upstream_server_failure().await;
    assert!(
        matches!(
            final_status,
            StatusCode::ServiceUnavailable | StatusCode::ServerInternalError
        ),
        "exhausted all-503 branch produced unexpected final {final_status}"
    );
    settle_proxy_tasks().await;
    assert_eq!(
        harness.invite_count_to(primary).await,
        1,
        "candidate exhaustion must not restart the candidate list"
    );
    assert_eq!(
        harness.invite_count_to(backup).await,
        1,
        "the final candidate must be attempted exactly once"
    );
}

#[tokio::test]
async fn parallel_candidate_advancement_does_not_cross_logical_legs() {
    let leg_a_primary: SocketAddr = "10.0.0.70:5060".parse().unwrap();
    let leg_a_backup: SocketAddr = "10.0.0.71:5060".parse().unwrap();
    let leg_b_primary: SocketAddr = "10.0.0.80:5060".parse().unwrap();
    let leg_b_backup: SocketAddr = "10.0.0.81:5060".parse().unwrap();
    let route_fn: RouteFn = Arc::new(move |_request: &Request| {
        Some(RouteDecision::parallel_with_failover(vec![
            vec![leg_a_primary, leg_a_backup],
            vec![leg_b_primary, leg_b_backup],
        ]))
    });
    let harness = Harness::new(route_fn).await;

    harness
        .inject(
            Message::Request(build_uac_invite("parallel-candidate-isolation")),
            UAC_ADDR.parse().unwrap(),
        )
        .await;
    let leg_a_first = harness.wait_for_invite_to(leg_a_primary).await;
    let leg_b_first = harness.wait_for_invite_to(leg_b_primary).await;
    harness
        .inject(
            response_for(&leg_a_first, StatusCode::ServiceUnavailable),
            leg_a_primary,
        )
        .await;
    let leg_a_second = harness.wait_for_invite_to(leg_a_backup).await;

    assert_eq!(
        harness.invite_count_to(leg_b_backup).await,
        0,
        "leg A failure must not advance leg B's candidate list"
    );
    assert_ne!(proxy_branch(&leg_a_first), proxy_branch(&leg_a_second));

    harness
        .inject(
            response_for(&leg_a_second, StatusCode::BusyHere),
            leg_a_backup,
        )
        .await;
    harness
        .inject(
            response_for(&leg_b_first, StatusCode::NotFound),
            leg_b_primary,
        )
        .await;
    harness.wait_for_upstream_final(StatusCode::NotFound).await;
    assert_eq!(harness.invite_count_to(leg_b_backup).await, 0);
}
