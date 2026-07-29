//! Regression coverage for response-context dispatch and downstream startup
//! failures. These cases intentionally exercise failures before an
//! authoritative first write so the proxy must retain its aggregation state.

use std::collections::{HashMap, HashSet};
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
use rvoip_sip_core::{Message, Method, Request, Response};
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
struct ProgrammableTransport {
    local_addr: SocketAddr,
    sent: Arc<Mutex<Vec<(Message, SocketAddr)>>>,
    failed_destinations: Arc<Mutex<HashSet<SocketAddr>>>,
}

impl ProgrammableTransport {
    fn new(local_addr: SocketAddr) -> Self {
        Self {
            local_addr,
            sent: Arc::new(Mutex::new(Vec::new())),
            failed_destinations: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    async fn fail_for(&self, destination: SocketAddr) {
        self.failed_destinations.lock().await.insert(destination);
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
        self.sent.lock().await.push((message, destination));
        if self.failed_destinations.lock().await.contains(&destination) {
            return Err(rvoip_sip_transport::Error::ConnectFailed(
                destination,
                std::io::Error::new(
                    std::io::ErrorKind::ConnectionRefused,
                    "programmed destination failure",
                ),
            ));
        }
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
    transport: Arc<ProgrammableTransport>,
    tx: mpsc::Sender<TransportEvent>,
    _tm: Arc<TransactionManager>,
    _proxy_task: tokio::task::JoinHandle<()>,
}

impl Harness {
    async fn new(route: RouteDecision) -> Self {
        let proxy_addr: SocketAddr = PROXY_ADDR.parse().unwrap();
        let transport = Arc::new(ProgrammableTransport::new(proxy_addr));
        let (tx, rx) = mpsc::channel(64);
        let (tm, events) = TransactionManager::new(transport.clone(), rx, Some(32))
            .await
            .expect("TransactionManager::new");
        let tm = Arc::new(tm);
        let route_fn: RouteFn = Arc::new(move |_request| Some(route.clone()));
        let proxy = StatefulProxy::new(tm.clone(), route_fn);
        let proxy_task = proxy.run(events);
        Self {
            transport,
            tx,
            _tm: tm,
            _proxy_task: proxy_task,
        }
    }

    async fn inject(&self, message: Message, source: SocketAddr) {
        self.tx
            .send(TransportEvent::MessageReceived {
                message,
                source,
                destination: self.transport.local_addr,
                transport_type: TransportType::Udp,
                flow_id: None,
                raw_bytes: None,
                timing: None,
                connection_metadata: None,
            })
            .await
            .expect("inject");
    }

    async fn wait_for_invites(&self, destinations: &[SocketAddr]) -> HashMap<SocketAddr, Request> {
        self.wait_for(Duration::from_secs(2), |sent| {
            destinations.iter().all(|destination| {
                sent.iter().any(|(message, actual)| {
                    *actual == *destination
                        && matches!(message, Message::Request(request) if request.method() == Method::Invite)
                })
            })
        })
        .await;
        self.transport
            .sent()
            .await
            .into_iter()
            .filter_map(|(message, destination)| match message {
                Message::Request(request)
                    if request.method() == Method::Invite
                        && destinations.contains(&destination) =>
                {
                    Some((destination, request))
                }
                _ => None,
            })
            .collect()
    }

    async fn wait_for_request(&self, method: Method, destination: SocketAddr) -> Request {
        self.wait_for(Duration::from_secs(2), |sent| {
            sent.iter().any(|(message, actual)| {
                *actual == destination
                    && matches!(message, Message::Request(request) if request.method() == method)
            })
        })
        .await;
        self.transport
            .sent()
            .await
            .into_iter()
            .find_map(|(message, actual)| match message {
                Message::Request(request)
                    if actual == destination && request.method() == method =>
                {
                    Some(request)
                }
                _ => None,
            })
            .expect("matching request")
    }

    async fn wait_for_response(&self, status: StatusCode) {
        let uac: SocketAddr = UAC_ADDR.parse().unwrap();
        self.wait_for(Duration::from_secs(2), |sent| {
            sent.iter().any(|(message, destination)| {
                *destination == uac
                    && matches!(message, Message::Response(response) if response.status() == status)
            })
        })
        .await;
    }

    async fn wait_for<F>(&self, timeout: Duration, predicate: F)
    where
        F: Fn(&[(Message, SocketAddr)]) -> bool,
    {
        let started = std::time::Instant::now();
        loop {
            let sent = self.transport.sent().await;
            if predicate(&sent) {
                return;
            }
            assert!(
                started.elapsed() <= timeout,
                "timed out after {timeout:?}; sent: {:?}",
                sent.iter()
                    .map(|(message, destination)| (short(message), *destination))
                    .collect::<Vec<_>>()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

fn build_request(method: Method, call_id: &str) -> Request {
    SimpleRequestBuilder::new(method, "sip:bob@example.com")
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
                vec![Param::branch(format!("z9hG4bK-{call_id}"))],
            )
            .unwrap(),
        ))
        .max_forwards(70)
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build()
}

fn response_for(request: &Request, status: StatusCode) -> Response {
    SimpleResponseBuilder::response_from_request(request, status, None).build()
}

/// Leave only the proxy's top Via. The response still matches the downstream
/// transaction, but after the proxy pops its own Via the upstream response is
/// invalid. This injects a proven zero-wire upstream failure without pretending
/// a transport error is safe to retry after the write boundary.
fn remove_upstream_via(response: &mut Response) {
    let mut retained_proxy_via = false;
    response.headers.retain(|header| {
        if !matches!(header, TypedHeader::Via(_)) {
            return true;
        }
        if retained_proxy_via {
            false
        } else {
            retained_proxy_via = true;
            true
        }
    });
}

fn short(message: &Message) -> String {
    match message {
        Message::Request(request) => format!("REQ {}", request.method()),
        Message::Response(response) => format!("RESP {}", response.status()),
    }
}

#[tokio::test]
async fn failed_first_invite_2xx_does_not_pre_latch_winner_or_break_multiple_2xx() {
    let destinations = [
        UAS_A.parse().unwrap(),
        UAS_B.parse().unwrap(),
        UAS_C.parse().unwrap(),
    ];
    let harness = Harness::new(RouteDecision::parallel(destinations.to_vec())).await;
    harness
        .inject(
            Message::Request(build_request(Method::Invite, "first-final-zero-wire")),
            UAC_ADDR.parse().unwrap(),
        )
        .await;
    let invites = harness.wait_for_invites(&destinations).await;

    // Put C into Proceeding so the first successfully forwarded 2xx must
    // dispatch its sibling CANCEL immediately.
    harness
        .inject(
            Message::Response(response_for(
                invites.get(&destinations[2]).unwrap(),
                StatusCode::Ringing,
            )),
            destinations[2],
        )
        .await;

    let mut invalid_first = response_for(invites.get(&destinations[0]).unwrap(), StatusCode::Ok);
    remove_upstream_via(&mut invalid_first);
    harness
        .inject(Message::Response(invalid_first), destinations[0])
        .await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let valid_winner = response_for(invites.get(&destinations[1]).unwrap(), StatusCode::Ok);
    harness
        .inject(Message::Response(valid_winner), destinations[1])
        .await;
    harness.wait_for_response(StatusCode::Ok).await;
    harness
        .wait_for(Duration::from_secs(2), |sent| {
            sent.iter().any(|(message, destination)| {
                *destination == destinations[2]
                    && matches!(message, Message::Request(request) if request.method() == Method::Cancel)
            })
        })
        .await;

    // A racing 2xx from the CANCELed branch is still forwarded as an
    // additional INVITE 2xx under RFC 3261 §16.7.
    harness
        .inject(
            Message::Response(response_for(
                invites.get(&destinations[2]).unwrap(),
                StatusCode::Ok,
            )),
            destinations[2],
        )
        .await;
    harness
        .wait_for(Duration::from_secs(2), |sent| {
            sent.iter()
                .filter(|(message, destination)| {
                    *destination == UAC_ADDR.parse().unwrap()
                        && matches!(message, Message::Response(response) if response.status() == StatusCode::Ok)
                })
                .count()
                >= 2
        })
        .await;
}

#[tokio::test]
async fn non_invite_zero_wire_final_failure_leaves_later_branch_eligible() {
    let uas_a: SocketAddr = UAS_A.parse().unwrap();
    let uas_b: SocketAddr = UAS_B.parse().unwrap();
    let harness = Harness::new(RouteDecision::parallel(vec![uas_a, uas_b])).await;
    harness
        .inject(
            Message::Request(build_request(Method::Options, "options-zero-wire")),
            UAC_ADDR.parse().unwrap(),
        )
        .await;
    let request_a = harness.wait_for_request(Method::Options, uas_a).await;
    let request_b = harness.wait_for_request(Method::Options, uas_b).await;

    let mut invalid_first = response_for(&request_a, StatusCode::Ok);
    remove_upstream_via(&mut invalid_first);
    harness
        .inject(Message::Response(invalid_first), uas_a)
        .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    harness
        .inject(
            Message::Response(response_for(&request_b, StatusCode::Ok)),
            uas_b,
        )
        .await;

    harness.wait_for_response(StatusCode::Ok).await;
}

#[tokio::test]
async fn all_parallel_candidate_exhaustion_generates_upstream_final() {
    let uas_a: SocketAddr = UAS_A.parse().unwrap();
    let uas_b: SocketAddr = UAS_B.parse().unwrap();
    let harness = Harness::new(RouteDecision::parallel_with_failover(vec![
        vec![uas_a],
        vec![uas_b],
    ]))
    .await;
    harness.transport.fail_for(uas_a).await;
    harness.transport.fail_for(uas_b).await;

    harness
        .inject(
            Message::Request(build_request(Method::Invite, "parallel-exhausted")),
            UAC_ADDR.parse().unwrap(),
        )
        .await;

    harness
        .wait_for(Duration::from_secs(2), |sent| {
            [uas_a, uas_b].iter().all(|destination| {
                sent.iter().any(|(message, actual)| {
                    actual == destination
                        && matches!(message, Message::Request(request) if request.method() == Method::Invite)
                })
            })
        })
        .await;
    harness
        .wait_for_response(StatusCode::ServerInternalError)
        .await;
}

#[tokio::test]
async fn partial_parallel_start_failure_still_selects_live_branch_final() {
    let uas_a: SocketAddr = UAS_A.parse().unwrap();
    let uas_b: SocketAddr = UAS_B.parse().unwrap();
    let harness = Harness::new(RouteDecision::parallel(vec![uas_a, uas_b])).await;
    harness.transport.fail_for(uas_a).await;

    harness
        .inject(
            Message::Request(build_request(Method::Invite, "partial-parallel")),
            UAC_ADDR.parse().unwrap(),
        )
        .await;
    let live_request = harness.wait_for_request(Method::Invite, uas_b).await;
    harness
        .inject(
            Message::Response(response_for(&live_request, StatusCode::NotFound)),
            uas_b,
        )
        .await;

    harness.wait_for_response(StatusCode::NotFound).await;
}

#[tokio::test]
async fn sequential_start_failures_advance_to_later_live_leg() {
    let uas_a: SocketAddr = UAS_A.parse().unwrap();
    let uas_b: SocketAddr = UAS_B.parse().unwrap();
    let uas_c: SocketAddr = UAS_C.parse().unwrap();
    let harness = Harness::new(RouteDecision::sequential(vec![uas_a, uas_b, uas_c])).await;
    harness.transport.fail_for(uas_a).await;
    harness.transport.fail_for(uas_b).await;

    harness
        .inject(
            Message::Request(build_request(Method::Invite, "sequential-failover")),
            UAC_ADDR.parse().unwrap(),
        )
        .await;
    let live_request = harness.wait_for_request(Method::Invite, uas_c).await;
    harness
        .inject(
            Message::Response(response_for(&live_request, StatusCode::BusyHere)),
            uas_c,
        )
        .await;

    harness.wait_for_response(StatusCode::BusyHere).await;
}

#[tokio::test]
async fn sequential_candidate_exhaustion_generates_upstream_final() {
    let uas_a: SocketAddr = UAS_A.parse().unwrap();
    let uas_b: SocketAddr = UAS_B.parse().unwrap();
    let harness = Harness::new(RouteDecision::sequential(vec![uas_a, uas_b])).await;
    harness.transport.fail_for(uas_a).await;
    harness.transport.fail_for(uas_b).await;

    harness
        .inject(
            Message::Request(build_request(Method::Invite, "sequential-exhausted")),
            UAC_ADDR.parse().unwrap(),
        )
        .await;

    harness
        .wait_for_response(StatusCode::ServerInternalError)
        .await;
}
