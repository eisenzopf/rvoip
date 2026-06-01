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
use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::types::content_length::ContentLength;
use rvoip_sip_core::types::param::Param;
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
    _proxy_task: tokio::task::JoinHandle<()>,
}

impl Harness {
    async fn new(route_fn: RouteFn) -> Self {
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

        let proxy = StatefulProxy::with_config(tm.clone(), route_fn, ProxyConfig::default());
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
            timing: None,
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
