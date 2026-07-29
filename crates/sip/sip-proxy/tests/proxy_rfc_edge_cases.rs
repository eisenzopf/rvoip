//! Packet-level regression tests for RFC 3261 proxy edge cases.
//!
//! These tests intentionally exercise the proxy through
//! `TransportEvent` and the real `TransactionManager`, while capturing
//! network output with a mock transport.  They cover interactions that
//! are easy to get wrong when CANCEL and fork aggregation race.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use rvoip_sip_core::types::content_length::ContentLength;
use rvoip_sip_core::types::headers::{HeaderName, HeaderValue};
use rvoip_sip_core::types::param::Param;
use rvoip_sip_core::types::status::StatusCode;
use rvoip_sip_core::types::via::Via;
use rvoip_sip_core::types::TypedHeader;
use rvoip_sip_core::{Message, Method, Request};
use rvoip_sip_dialog::transaction::{timer::TimerSettings, TransactionManager};
use rvoip_sip_proxy::{ProxyConfig, ProxyRuntimeOptions, RouteDecision, RouteFn, StatefulProxy};
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
    proxy: Arc<StatefulProxy>,
    _proxy_task: tokio::task::JoinHandle<()>,
}

impl Harness {
    async fn new(route: RouteDecision) -> Self {
        Self::new_with_config(route, ProxyConfig::default()).await
    }

    async fn new_with_config(route: RouteDecision, config: ProxyConfig) -> Self {
        Self::new_with_options(route, config, ProxyRuntimeOptions::default()).await
    }

    async fn new_with_options(
        route: RouteDecision,
        config: ProxyConfig,
        options: ProxyRuntimeOptions,
    ) -> Self {
        Self::new_with_options_and_timers(route, config, options, None).await
    }

    async fn new_with_timer_settings(route: RouteDecision, timer_settings: TimerSettings) -> Self {
        Self::new_with_options_and_timers(
            route,
            ProxyConfig::default(),
            ProxyRuntimeOptions::default(),
            Some(timer_settings),
        )
        .await
    }

    async fn new_with_options_and_timers(
        route: RouteDecision,
        config: ProxyConfig,
        options: ProxyRuntimeOptions,
        timer_settings: Option<TimerSettings>,
    ) -> Self {
        let proxy_addr: SocketAddr = PROXY_ADDR.parse().unwrap();
        let transport = Arc::new(MockTransport::new(proxy_addr));
        let (tx, rx) = mpsc::channel(64);
        let (tm, events) =
            TransactionManager::new_with_config(transport.clone(), rx, Some(32), timer_settings)
                .await
                .expect("TransactionManager::new_with_config");
        let tm = Arc::new(tm);
        let route_fn: RouteFn = Arc::new(move |_request: &Request| Some(route.clone()));
        let proxy = StatefulProxy::with_options(tm.clone(), route_fn, config, options);
        let proxy_task = proxy.clone().run(events);

        Self {
            transport,
            tx,
            _tm: tm,
            proxy,
            _proxy_task: proxy_task,
        }
    }

    async fn inject(&self, message: Message, source: SocketAddr) {
        self.inject_on(message, source, TransportType::Udp).await;
    }

    async fn inject_on(&self, message: Message, source: SocketAddr, transport_type: TransportType) {
        self.tx
            .send(TransportEvent::MessageReceived {
                message,
                source,
                destination: self.transport.local_addr,
                transport_type,
                flow_id: None,
                raw_bytes: None,
                timing: None,
                connection_metadata: None,
            })
            .await
            .expect("inject transport event");
    }

    async fn wait_for<F>(&self, deadline_ms: u64, predicate: F) -> (Message, SocketAddr)
    where
        F: Fn(&Message, &SocketAddr) -> bool,
    {
        let start = std::time::Instant::now();
        loop {
            let sent = self.transport.sent().await;
            if let Some((message, destination)) = sent
                .iter()
                .find(|(message, destination)| predicate(message, destination))
            {
                return (message.clone(), *destination);
            }
            if start.elapsed() > Duration::from_millis(deadline_ms) {
                panic!(
                    "timed out after {deadline_ms}ms; sent: {:?}",
                    sent.iter()
                        .map(|(message, destination)| format!(
                            "{} -> {destination}",
                            short(message)
                        ))
                        .collect::<Vec<_>>()
                );
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn wait_for_invites(&self, destinations: &[SocketAddr]) -> Vec<(SocketAddr, Request)> {
        let start = std::time::Instant::now();
        loop {
            let sent = self.transport.sent().await;
            let invites = destinations
                .iter()
                .filter_map(|destination| {
                    sent.iter()
                        .find_map(|(message, actual_destination)| match message {
                            Message::Request(request)
                                if request.method() == Method::Invite
                                    && actual_destination == destination =>
                            {
                                Some((*destination, request.clone()))
                            }
                            _ => None,
                        })
                })
                .collect::<Vec<_>>();
            if invites.len() == destinations.len() {
                return invites;
            }
            if start.elapsed() > Duration::from_millis(1500) {
                panic!(
                    "timed out waiting for INVITEs to {destinations:?}; sent: {:?}",
                    sent.iter()
                        .map(|(message, destination)| format!(
                            "{} -> {destination}",
                            short(message)
                        ))
                        .collect::<Vec<_>>()
                );
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

fn short(message: &Message) -> String {
    match message {
        Message::Request(request) => format!("REQ {}", request.method()),
        Message::Response(response) => format!("RESP {}", response.status()),
    }
}

fn build_invite(call_id: &str) -> Request {
    SimpleRequestBuilder::new(Method::Invite, "sip:bob@10.0.0.20:5060")
        .unwrap()
        .from("Alice", "sip:alice@uac.example.com", Some("alice-tag"))
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

fn build_cancel(call_id: &str) -> Request {
    SimpleRequestBuilder::new(Method::Cancel, "sip:bob@10.0.0.20:5060")
        .unwrap()
        .from("Alice", "sip:alice@uac.example.com", Some("alice-tag"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id(call_id)
        .cseq(1)
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

fn build_ack(call_id: &str) -> Request {
    SimpleRequestBuilder::new(Method::Ack, "sip:bob@10.0.0.20:5060")
        .unwrap()
        .from("Alice", "sip:alice@uac.example.com", Some("alice-tag"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id(call_id)
        .cseq(1)
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

fn build_options(call_id: &str) -> Request {
    build_non_invite(Method::Options, call_id)
}

fn build_non_invite(method: Method, call_id: &str) -> Request {
    SimpleRequestBuilder::new(method, "sip:bob@10.0.0.20:5060")
        .unwrap()
        .from("Alice", "sip:alice@uac.example.com", Some("alice-tag"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id(call_id)
        .cseq(1)
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

fn response_for(request: &Request, status: StatusCode) -> Message {
    Message::Response(SimpleResponseBuilder::response_from_request(request, status, None).build())
}

fn is_response_for_method(message: &Message, status: StatusCode, method: Method) -> bool {
    matches!(
        message,
        Message::Response(response)
            if response.status() == status
                && response.cseq().is_some_and(|cseq| cseq.method == method)
    )
}

async fn settle_proxy_tasks() {
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
}

fn fast_non_invite_timers() -> TimerSettings {
    TimerSettings {
        t1: Duration::from_millis(5),
        t2: Duration::from_millis(10),
        transaction_timeout: Duration::from_millis(80),
        wait_time_k: Duration::from_millis(5),
        ..TimerSettings::default()
    }
}

#[tokio::test(start_paused = true)]
async fn stateless_response_route_forwards_duplicate_final_responses_until_expiry() {
    let uas: SocketAddr = UAS_A.parse().unwrap();
    let uac: SocketAddr = UAC_ADDR.parse().unwrap();
    let harness = Harness::new(RouteDecision::to(uas)).await;

    harness
        .inject(
            Message::Request(build_cancel("stateless-response-retransmit")),
            uac,
        )
        .await;
    let (message, _) = harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Request(request) if request.method() == Method::Cancel)
                && *destination == uas
        })
        .await;
    let Message::Request(forwarded_cancel) = message else {
        unreachable!();
    };
    assert_eq!(
        harness.proxy.retention_snapshot().stateless_response_routes,
        1
    );
    assert_eq!(
        harness.proxy.retention_snapshot().known_branches,
        0,
        "the disabled legacy detector must not retain stateless branches"
    );

    let response = response_for(&forwarded_cancel, StatusCode::Ok);
    harness.inject(response.clone(), uas).await;
    harness.inject(response, uas).await;
    settle_proxy_tasks().await;

    let forwarded_count = harness
        .transport
        .sent()
        .await
        .iter()
        .filter(|(message, destination)| {
            is_response_for_method(message, StatusCode::Ok, Method::Cancel) && *destination == uac
        })
        .count();
    assert_eq!(
        forwarded_count, 2,
        "a stateless response route must remain available for final-response retransmissions"
    );
    assert_eq!(
        harness.proxy.retention_snapshot().stateless_response_routes,
        1,
        "forwarding a response must not consume its bounded route"
    );
}

#[tokio::test(start_paused = true)]
async fn stateless_response_route_rejects_wrong_source_and_cseq_method() {
    let uas: SocketAddr = UAS_A.parse().unwrap();
    let wrong_uas: SocketAddr = UAS_B.parse().unwrap();
    let uac: SocketAddr = UAC_ADDR.parse().unwrap();
    let harness = Harness::new(RouteDecision::to(uas)).await;

    harness
        .inject(
            Message::Request(build_cancel("stateless-response-authentication")),
            uac,
        )
        .await;
    let (message, _) = harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Request(request) if request.method() == Method::Cancel)
                && *destination == uas
        })
        .await;
    let Message::Request(forwarded_cancel) = message else {
        unreachable!();
    };

    let valid_response = response_for(&forwarded_cancel, StatusCode::Ok);
    harness.inject(valid_response.clone(), wrong_uas).await;
    harness
        .inject_on(valid_response.clone(), uas, TransportType::Tcp)
        .await;

    let Message::Response(mut wrong_method) = valid_response.clone() else {
        unreachable!();
    };
    for header in &mut wrong_method.headers {
        if let TypedHeader::CSeq(cseq) = header {
            cseq.method = Method::Options;
        }
    }
    harness.inject(Message::Response(wrong_method), uas).await;
    settle_proxy_tasks().await;

    let rejected_count = harness
        .transport
        .sent()
        .await
        .iter()
        .filter(|(message, destination)| {
            matches!(message, Message::Response(_)) && *destination == uac
        })
        .count();
    assert_eq!(
        rejected_count, 0,
        "branch-only correlation must not authorize a response from the wrong peer or method"
    );
    assert_eq!(
        harness.proxy.retention_snapshot().stateless_response_routes,
        1,
        "a mismatched response must not consume the valid route"
    );

    harness.inject(valid_response, uas).await;
    harness
        .wait_for(1000, |message, destination| {
            is_response_for_method(message, StatusCode::Ok, Method::Cancel) && *destination == uac
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn stateless_response_route_expires_while_proxy_is_otherwise_idle() {
    let uas: SocketAddr = UAS_A.parse().unwrap();
    let uac: SocketAddr = UAC_ADDR.parse().unwrap();
    let harness = Harness::new_with_options(
        RouteDecision::to(uas),
        ProxyConfig::default(),
        ProxyRuntimeOptions::default().with_legacy_loop_detection_for_tests(),
    )
    .await;

    harness
        .inject(
            Message::Request(build_cancel("stateless-response-idle-expiry")),
            uac,
        )
        .await;
    let (message, _) = harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Request(request) if request.method() == Method::Cancel)
                && *destination == uas
        })
        .await;
    let Message::Request(forwarded_cancel) = message else {
        unreachable!();
    };
    assert_eq!(
        harness.proxy.retention_snapshot().stateless_response_routes,
        1
    );
    assert_eq!(harness.proxy.retention_snapshot().known_branches, 1);

    tokio::time::advance(Duration::from_secs(63)).await;
    settle_proxy_tasks().await;
    assert_eq!(
        harness.proxy.retention_snapshot().stateless_response_routes,
        1
    );

    tokio::time::advance(Duration::from_secs(2)).await;
    settle_proxy_tasks().await;
    assert_eq!(
        harness.proxy.retention_snapshot().stateless_response_routes,
        0,
        "the stateless route deadline must wake an otherwise idle proxy"
    );
    assert_eq!(
        harness.proxy.retention_snapshot().known_branches,
        0,
        "stateless expiry must remove its opt-in retained loop-detection branch"
    );

    harness
        .inject(response_for(&forwarded_cancel, StatusCode::Ok), uas)
        .await;
    settle_proxy_tasks().await;
    let forwarded_after_expiry =
        harness
            .transport
            .sent()
            .await
            .iter()
            .any(|(message, destination)| {
                is_response_for_method(message, StatusCode::Ok, Method::Cancel)
                    && *destination == uac
            });
    assert!(
        !forwarded_after_expiry,
        "a final response must not use an expired correlation"
    );
}

fn extension_values(headers: &[TypedHeader], name: &str) -> Vec<Vec<u8>> {
    headers
        .iter()
        .filter_map(|header| match header {
            TypedHeader::Other(HeaderName::Other(actual), HeaderValue::Raw(value))
                if actual.eq_ignore_ascii_case(name) =>
            {
                Some(value.clone())
            }
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn duplicate_matched_cancel_dispatches_one_downstream_cancel() {
    let uas: SocketAddr = UAS_A.parse().unwrap();
    let uac: SocketAddr = UAC_ADDR.parse().unwrap();
    let harness = Harness::new(RouteDecision::to(uas)).await;
    let call_id = "duplicate-matched-cancel";

    harness
        .inject(Message::Request(build_invite(call_id)), uac)
        .await;
    let invite = harness.wait_for_invites(&[uas]).await.remove(0).1;
    harness
        .inject(response_for(&invite, StatusCode::Ringing), uas)
        .await;

    let cancel = build_cancel(call_id);
    harness.inject(Message::Request(cancel.clone()), uac).await;
    harness.inject(Message::Request(cancel), uac).await;

    harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Request(request) if request.method() == Method::Cancel)
                && *destination == uas
        })
        .await;
    tokio::time::sleep(Duration::from_millis(150)).await;

    let downstream_cancel_count = harness
        .transport
        .sent()
        .await
        .iter()
        .filter(|(message, destination)| {
            matches!(message, Message::Request(request) if request.method() == Method::Cancel)
                && *destination == uas
        })
        .count();
    assert_eq!(
        downstream_cancel_count, 1,
        "a CANCEL retransmission must reuse the CANCEL server transaction and not fan out twice"
    );
}

#[tokio::test]
async fn generated_cancel_response_is_consumed_and_does_not_aggregate_as_invite() {
    let uas_a: SocketAddr = UAS_A.parse().unwrap();
    let uas_b: SocketAddr = UAS_B.parse().unwrap();
    let uac: SocketAddr = UAC_ADDR.parse().unwrap();
    let harness = Harness::new(RouteDecision::parallel(vec![uas_a, uas_b])).await;
    let call_id = "generated-cancel-response";

    harness
        .inject(Message::Request(build_invite(call_id)), uac)
        .await;
    let invites = harness.wait_for_invites(&[uas_a, uas_b]).await;
    for (destination, invite) in &invites {
        harness
            .inject(response_for(invite, StatusCode::Ringing), *destination)
            .await;
    }
    harness
        .inject(Message::Request(build_cancel(call_id)), uac)
        .await;

    let (generated_cancel_a, _) = harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Request(request) if request.method() == Method::Cancel)
                && *destination == uas_a
        })
        .await;
    let Message::Request(generated_cancel_a) = generated_cancel_a else {
        unreachable!();
    };
    let (generated_cancel_b, _) = harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Request(request) if request.method() == Method::Cancel)
                && *destination == uas_b
        })
        .await;
    let Message::Request(generated_cancel_b) = generated_cancel_b else {
        unreachable!();
    };
    assert_eq!(
        generated_cancel_a.via_headers().len(),
        1,
        "a proxy-generated CANCEL carries only the proxy's own Via"
    );
    assert_eq!(
        generated_cancel_b.via_headers().len(),
        1,
        "a proxy-generated CANCEL carries only the proxy's own Via"
    );

    harness
        .inject(response_for(&generated_cancel_a, StatusCode::Ok), uas_a)
        .await;
    harness
        .inject(
            response_for(
                &generated_cancel_b,
                StatusCode::CallOrTransactionDoesNotExist,
            ),
            uas_b,
        )
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let cancel_ok_count = harness
        .transport
        .sent()
        .await
        .iter()
        .filter(|(message, destination)| {
            is_response_for_method(message, StatusCode::Ok, Method::Cancel) && *destination == uac
        })
        .count();
    assert_eq!(
        cancel_ok_count, 1,
        "the downstream response to the proxy-generated CANCEL must be consumed locally"
    );
    assert!(
        !harness
            .transport
            .sent()
            .await
            .iter()
            .any(|(message, destination)| {
                is_response_for_method(
                    message,
                    StatusCode::CallOrTransactionDoesNotExist,
                    Method::Cancel,
                ) && *destination == uac
            }),
        "a failure response to a proxy-generated CANCEL must also be consumed locally"
    );

    for (destination, invite) in invites {
        harness
            .inject(
                response_for(&invite, StatusCode::RequestTerminated),
                destination,
            )
            .await;
    }
    harness
        .wait_for(1000, |message, destination| {
            is_response_for_method(message, StatusCode::RequestTerminated, Method::Invite)
                && *destination == uac
        })
        .await;
}

#[tokio::test]
async fn non_invite_extra_2xx_is_not_forwarded_after_the_selected_final() {
    let uas: SocketAddr = UAS_A.parse().unwrap();
    let uac: SocketAddr = UAC_ADDR.parse().unwrap();
    let harness = Harness::new(RouteDecision::to(uas)).await;
    let request = build_options("non-invite-extra-2xx");

    harness.inject(Message::Request(request), uac).await;
    let (forwarded, _) = harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Request(request) if request.method() == Method::Options)
                && *destination == uas
        })
        .await;
    let Message::Request(forwarded) = forwarded else {
        unreachable!();
    };

    harness
        .inject(response_for(&forwarded, StatusCode::Ok), uas)
        .await;
    harness
        .wait_for(1000, |message, destination| {
            is_response_for_method(message, StatusCode::Ok, Method::Options) && *destination == uac
        })
        .await;
    harness
        .inject(response_for(&forwarded, StatusCode::Ok), uas)
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let upstream_successes = harness
        .transport
        .sent()
        .await
        .iter()
        .filter(|(message, destination)| {
            is_response_for_method(message, StatusCode::Ok, Method::Options) && *destination == uac
        })
        .count();
    assert_eq!(
        upstream_successes, 1,
        "only INVITE permits multiple forked 2xx responses upstream"
    );
}

#[tokio::test]
async fn non_invite_provisional_responses_are_consumed_locally() {
    let uas: SocketAddr = UAS_A.parse().unwrap();
    let uac: SocketAddr = UAC_ADDR.parse().unwrap();
    for (method, call_id) in [
        (Method::Options, "options-provisional"),
        (Method::Message, "message-provisional"),
    ] {
        let harness = Harness::new(RouteDecision::to(uas)).await;
        harness
            .inject(
                Message::Request(build_non_invite(method.clone(), call_id)),
                uac,
            )
            .await;
        let (forwarded, _) = harness
            .wait_for(1000, |message, destination| {
                matches!(message, Message::Request(request) if request.method() == method)
                    && *destination == uas
            })
            .await;
        let Message::Request(forwarded) = forwarded else {
            unreachable!();
        };

        for status in [
            StatusCode::Ringing,
            StatusCode::CallIsBeingForwarded,
            StatusCode::Queued,
            StatusCode::SessionProgress,
        ] {
            harness.inject(response_for(&forwarded, status), uas).await;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;

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
                            if (101..=199).contains(&response.status().as_u16())
                                && response.cseq().is_some_and(|cseq| cseq.method == method)
                    ) && *destination == uac
                }),
            "RFC 4320 forbids a proxy from forwarding any non-INVITE 101-199 response"
        );
    }
}

#[tokio::test]
async fn sequential_non_invite_timeout_advances_to_a_real_final_response() {
    let uas_a: SocketAddr = UAS_A.parse().unwrap();
    let uas_b: SocketAddr = UAS_B.parse().unwrap();
    let uac: SocketAddr = UAC_ADDR.parse().unwrap();
    let harness = Harness::new_with_timer_settings(
        RouteDecision::sequential(vec![uas_a, uas_b]),
        fast_non_invite_timers(),
    )
    .await;

    harness
        .inject(
            Message::Request(build_options("non-invite-sequential-timeout")),
            uac,
        )
        .await;
    harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Request(request) if request.method() == Method::Options)
                && *destination == uas_a
        })
        .await;

    let (second, _) = harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Request(request) if request.method() == Method::Options)
                && *destination == uas_b
        })
        .await;
    let Message::Request(second) = second else {
        unreachable!();
    };
    harness
        .inject(response_for(&second, StatusCode::NotFound), uas_b)
        .await;

    harness
        .wait_for(1000, |message, destination| {
            is_response_for_method(message, StatusCode::NotFound, Method::Options)
                && *destination == uac
        })
        .await;
    assert!(
        !harness
            .transport
            .sent()
            .await
            .iter()
            .any(|(message, destination)| {
                is_response_for_method(message, StatusCode::RequestTimeout, Method::Options)
                    && *destination == uac
            }),
        "a non-INVITE branch timeout must not synthesize an upstream 408"
    );
}

#[tokio::test]
async fn parallel_non_invite_timeout_allows_another_branch_real_final_to_win() {
    let uas_a: SocketAddr = UAS_A.parse().unwrap();
    let uas_b: SocketAddr = UAS_B.parse().unwrap();
    let uac: SocketAddr = UAC_ADDR.parse().unwrap();
    let harness = Harness::new_with_timer_settings(
        RouteDecision::parallel(vec![uas_a, uas_b]),
        fast_non_invite_timers(),
    )
    .await;

    harness
        .inject(
            Message::Request(build_options("non-invite-parallel-timeout")),
            uac,
        )
        .await;
    harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Request(request) if request.method() == Method::Options)
                && *destination == uas_a
        })
        .await;
    let (second, _) = harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Request(request) if request.method() == Method::Options)
                && *destination == uas_b
        })
        .await;
    let Message::Request(second) = second else {
        unreachable!();
    };
    harness
        .inject(response_for(&second, StatusCode::NotFound), uas_b)
        .await;

    harness
        .wait_for(1000, |message, destination| {
            is_response_for_method(message, StatusCode::NotFound, Method::Options)
                && *destination == uac
        })
        .await;
    assert!(
        !harness
            .transport
            .sent()
            .await
            .iter()
            .any(|(message, destination)| {
                is_response_for_method(message, StatusCode::RequestTimeout, Method::Options)
                    && *destination == uac
            }),
        "the timed-out parallel branch must remain silent while a received final wins"
    );
}

#[tokio::test]
async fn all_non_invite_timeouts_and_a_late_response_remain_silent() {
    let uas_a: SocketAddr = UAS_A.parse().unwrap();
    let uas_b: SocketAddr = UAS_B.parse().unwrap();
    let uac: SocketAddr = UAC_ADDR.parse().unwrap();
    let harness = Harness::new_with_timer_settings(
        RouteDecision::parallel(vec![uas_a, uas_b]),
        fast_non_invite_timers(),
    )
    .await;

    harness
        .inject(
            Message::Request(build_options("non-invite-all-timeout")),
            uac,
        )
        .await;
    let (first, _) = harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Request(request) if request.method() == Method::Options)
                && *destination == uas_a
        })
        .await;
    let Message::Request(first) = first else {
        unreachable!();
    };
    harness
        .wait_for(1000, |message, destination| {
            matches!(message, Message::Request(request) if request.method() == Method::Options)
                && *destination == uas_b
        })
        .await;

    tokio::time::sleep(Duration::from_millis(150)).await;
    settle_proxy_tasks().await;
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
                        if response.cseq().is_some_and(|cseq| cseq.method == Method::Options)
                            && response.status().as_u16() >= 200
                ) && *destination == uac
            }),
        "RFC 4320 requires an all-timeout non-INVITE fork to send no final response"
    );

    harness
        .inject(response_for(&first, StatusCode::Ok), uas_a)
        .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !harness
            .transport
            .sent()
            .await
            .iter()
            .any(|(message, destination)| {
                is_response_for_method(message, StatusCode::Ok, Method::Options)
                    && *destination == uac
            }),
        "a late non-INVITE response without a live matching transaction must be dropped"
    );
}

#[tokio::test(start_paused = true)]
async fn response_context_waits_for_generated_cancel_transactions_then_drains() {
    let uas_a: SocketAddr = UAS_A.parse().unwrap();
    let uas_b: SocketAddr = UAS_B.parse().unwrap();
    let uac: SocketAddr = UAC_ADDR.parse().unwrap();
    let harness = Harness::new_with_options(
        RouteDecision::parallel(vec![uas_a, uas_b]),
        ProxyConfig::default(),
        ProxyRuntimeOptions::default().with_legacy_loop_detection_for_tests(),
    )
    .await;
    let call_id = "generated-cancel-retention";

    harness
        .inject(Message::Request(build_invite(call_id)), uac)
        .await;
    let invites = harness.wait_for_invites(&[uas_a, uas_b]).await;
    for (destination, invite) in &invites {
        harness
            .inject(response_for(invite, StatusCode::Ringing), *destination)
            .await;
    }
    harness
        .inject(Message::Request(build_cancel(call_id)), uac)
        .await;

    let mut generated_cancels = Vec::new();
    for destination in [uas_a, uas_b] {
        let (message, _) = harness
            .wait_for(1000, |message, actual_destination| {
                matches!(message, Message::Request(request) if request.method() == Method::Cancel)
                    && *actual_destination == destination
            })
            .await;
        let Message::Request(cancel) = message else {
            unreachable!();
        };
        generated_cancels.push((destination, cancel));
    }
    let active = harness.proxy.retention_snapshot();
    assert_eq!(active.response_contexts, 1);
    assert_eq!(active.generated_cancel_transactions, 2);
    assert_eq!(active.known_branches, 2);

    for (destination, cancel) in generated_cancels {
        harness
            .inject(response_for(&cancel, StatusCode::Ok), destination)
            .await;
    }
    for (destination, invite) in invites {
        harness
            .inject(
                response_for(&invite, StatusCode::RequestTerminated),
                destination,
            )
            .await;
    }
    harness
        .wait_for(1000, |message, destination| {
            is_response_for_method(message, StatusCode::RequestTerminated, Method::Invite)
                && *destination == uac
        })
        .await;
    harness
        .inject(Message::Request(build_ack(call_id)), uac)
        .await;

    // UDP Timer D/K and server-side ACK processing must finish before the
    // response-context retention horizon starts.
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    for _ in 0..12 {
        tokio::time::advance(Duration::from_secs(5)).await;
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        if harness
            .proxy
            .retention_snapshot()
            .response_context_deadlines
            == 1
        {
            break;
        }
    }

    let drained_transactions = harness.proxy.retention_snapshot();
    assert_eq!(drained_transactions.response_contexts, 1);
    assert_eq!(drained_transactions.generated_cancel_transactions, 2);
    assert_eq!(
        drained_transactions.response_context_deadlines, 1,
        "the context becomes expiry-eligible only after INVITE and generated-CANCEL transactions terminate"
    );

    tokio::time::advance(Duration::from_secs(63)).await;
    tokio::task::yield_now().await;
    assert_eq!(harness.proxy.retention_snapshot().response_contexts, 1);

    tokio::time::advance(Duration::from_secs(2)).await;
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    let expired = harness.proxy.retention_snapshot();
    assert_eq!(expired.response_contexts, 0);
    assert_eq!(expired.downstream_invite_indexes, 0);
    assert_eq!(expired.generated_cancel_transactions, 0);
    assert_eq!(expired.response_context_deadlines, 0);
    assert_eq!(expired.known_branches, 0);
}

#[tokio::test]
async fn upstream_cancel_racing_with_invite_2xx_forwards_2xx_without_downstream_cancel() {
    let uas: SocketAddr = UAS_A.parse().unwrap();
    let uac: SocketAddr = UAC_ADDR.parse().unwrap();
    let harness = Harness::new(RouteDecision::to(uas)).await;
    let call_id = "cancel-races-2xx";

    harness
        .inject(Message::Request(build_invite(call_id)), uac)
        .await;
    let invite = harness.wait_for_invites(&[uas]).await.remove(0).1;
    harness
        .inject(Message::Request(build_cancel(call_id)), uac)
        .await;
    harness
        .wait_for(1000, |message, destination| {
            is_response_for_method(message, StatusCode::Ok, Method::Cancel) && *destination == uac
        })
        .await;

    harness
        .inject(response_for(&invite, StatusCode::Ok), uas)
        .await;
    harness
        .wait_for(1000, |message, destination| {
            is_response_for_method(message, StatusCode::Ok, Method::Invite) && *destination == uac
        })
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert!(
        !harness
            .transport
            .sent()
            .await
            .iter()
            .any(|(message, destination)| {
                matches!(message, Message::Request(request) if request.method() == Method::Cancel)
                    && *destination == uas
            }),
        "the final INVITE 2xx wins before the latched downstream CANCEL becomes eligible"
    );
}

#[tokio::test]
async fn best_response_uses_rfc_class_then_within_class_preference() {
    let uas_a: SocketAddr = UAS_A.parse().unwrap();
    let uas_b: SocketAddr = UAS_B.parse().unwrap();
    let uas_c: SocketAddr = UAS_C.parse().unwrap();
    let uac: SocketAddr = UAC_ADDR.parse().unwrap();

    // A 3xx is selected ahead of every 4xx and 5xx response.
    let class_harness = Harness::new(RouteDecision::parallel(vec![uas_a, uas_b, uas_c])).await;
    class_harness
        .inject(Message::Request(build_invite("best-response-class")), uac)
        .await;
    let invites = class_harness.wait_for_invites(&[uas_a, uas_b, uas_c]).await;
    for ((destination, invite), status) in invites.into_iter().zip([
        StatusCode::MovedTemporarily,
        StatusCode::Unauthorized,
        StatusCode::ServiceUnavailable,
    ]) {
        class_harness
            .inject(response_for(&invite, status), destination)
            .await;
    }
    class_harness
        .wait_for(1000, |message, destination| {
            is_response_for_method(message, StatusCode::MovedTemporarily, Method::Invite)
                && *destination == uac
        })
        .await;

    // Within 4xx, RFC 3261 gives 401 special resubmission preference
    // over an otherwise lower numeric 400.
    let preference_harness = Harness::new(RouteDecision::parallel(vec![uas_a, uas_b])).await;
    preference_harness
        .inject(
            Message::Request(build_invite("best-response-within-class")),
            uac,
        )
        .await;
    let invites = preference_harness.wait_for_invites(&[uas_a, uas_b]).await;
    for ((destination, invite), status) in invites
        .into_iter()
        .zip([StatusCode::BadRequest, StatusCode::Unauthorized])
    {
        preference_harness
            .inject(response_for(&invite, status), destination)
            .await;
    }
    preference_harness
        .wait_for(1000, |message, destination| {
            is_response_for_method(message, StatusCode::Unauthorized, Method::Invite)
                && *destination == uac
        })
        .await;

    // A global 6xx has preference over the otherwise best lower-class
    // response after the response context has settled.
    let global_harness = Harness::new(RouteDecision::parallel(vec![uas_a, uas_b, uas_c])).await;
    global_harness
        .inject(
            Message::Request(build_invite("best-response-global-class")),
            uac,
        )
        .await;
    let invites = global_harness
        .wait_for_invites(&[uas_a, uas_b, uas_c])
        .await;
    for ((destination, invite), status) in invites.into_iter().zip([
        StatusCode::MovedTemporarily,
        StatusCode::Unauthorized,
        StatusCode::Decline,
    ]) {
        global_harness
            .inject(response_for(&invite, status), destination)
            .await;
    }
    global_harness
        .wait_for(1000, |message, destination| {
            is_response_for_method(message, StatusCode::Decline, Method::Invite)
                && *destination == uac
        })
        .await;
}

#[tokio::test]
async fn only_503_failures_are_normalized_to_generated_500() {
    let uas_a: SocketAddr = UAS_A.parse().unwrap();
    let uas_b: SocketAddr = UAS_B.parse().unwrap();
    let uac: SocketAddr = UAC_ADDR.parse().unwrap();
    let harness = Harness::new(RouteDecision::parallel(vec![uas_a, uas_b])).await;

    harness
        .inject(
            Message::Request(build_invite("only-503-normalization")),
            uac,
        )
        .await;
    let invites = harness.wait_for_invites(&[uas_a, uas_b]).await;
    for (destination, invite) in invites {
        harness
            .inject(
                response_for(&invite, StatusCode::ServiceUnavailable),
                destination,
            )
            .await;
    }

    harness
        .wait_for(1000, |message, destination| {
            is_response_for_method(message, StatusCode::ServerInternalError, Method::Invite)
                && *destination == uac
        })
        .await;
    assert!(
        !harness
            .transport
            .sent()
            .await
            .iter()
            .any(|(message, destination)| {
                is_response_for_method(message, StatusCode::ServiceUnavailable, Method::Invite)
                    && *destination == uac
            }),
        "RFC 3261 §16.7 recommends generating 500 instead of forwarding an aggregate 503"
    );
}

#[tokio::test]
async fn received_503_never_outranks_another_received_final_response() {
    let uas_a: SocketAddr = UAS_A.parse().unwrap();
    let uas_b: SocketAddr = UAS_B.parse().unwrap();
    let uac: SocketAddr = UAC_ADDR.parse().unwrap();
    for (other_status, call_id) in [
        (StatusCode::ServerInternalError, "503-versus-500"),
        (StatusCode::ServerTimeout, "503-versus-504"),
    ] {
        let harness = Harness::new(RouteDecision::parallel(vec![uas_a, uas_b])).await;
        harness
            .inject(Message::Request(build_invite(call_id)), uac)
            .await;
        let invites = harness.wait_for_invites(&[uas_a, uas_b]).await;
        harness
            .inject(
                response_for(&invites[0].1, StatusCode::ServiceUnavailable),
                invites[0].0,
            )
            .await;
        harness
            .inject(response_for(&invites[1].1, other_status), invites[1].0)
            .await;

        harness
            .wait_for(1000, |message, destination| {
                is_response_for_method(message, other_status, Method::Invite) && *destination == uac
            })
            .await;
        assert!(
            !harness
                .transport
                .sent()
                .await
                .iter()
                .any(|(message, destination)| {
                    is_response_for_method(message, StatusCode::ServiceUnavailable, Method::Invite)
                        && *destination == uac
                }),
            "RFC 3261 section 16.7 makes 503 a last-resort response"
        );
    }
}

#[tokio::test]
async fn forwarding_preserves_request_and_response_bodies_and_repeated_headers() {
    const REQUEST_BODY: &[u8] = b"v=0\r\ns=request-body\r\n";
    const RESPONSE_BODY: &[u8] = b"v=0\r\ns=response-body\r\n";
    const HEADER: &str = "X-Rvoip-Trace";

    let uas: SocketAddr = UAS_A.parse().unwrap();
    let uac: SocketAddr = UAC_ADDR.parse().unwrap();
    let harness = Harness::new(RouteDecision::to(uas)).await;

    let mut invite = SimpleRequestBuilder::new(Method::Invite, "sip:bob@10.0.0.20:5060")
        .unwrap()
        .from("Alice", "sip:alice@uac.example.com", Some("alice-tag"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id("body-and-repeated-headers")
        .cseq(1)
        .contact("sip:alice@10.0.0.5:5060", None)
        .header(TypedHeader::Via(
            Via::new(
                "SIP",
                "2.0",
                "UDP",
                "10.0.0.5",
                Some(5060),
                vec![Param::branch("z9hG4bK-body-and-repeated-headers")],
            )
            .unwrap(),
        ))
        .max_forwards(70)
        .content_type("application/sdp")
        .body(REQUEST_BODY)
        .build();
    invite.headers.push(TypedHeader::Other(
        HeaderName::Other(HEADER.into()),
        HeaderValue::Raw(b"request-a".to_vec()),
    ));
    invite.headers.push(TypedHeader::Other(
        HeaderName::Other(HEADER.into()),
        HeaderValue::Raw(b"request-b".to_vec()),
    ));
    harness.inject(Message::Request(invite), uac).await;

    let forwarded = harness.wait_for_invites(&[uas]).await.remove(0).1;
    assert_eq!(forwarded.body(), REQUEST_BODY);
    assert_eq!(
        extension_values(&forwarded.headers, HEADER),
        vec![b"request-a".to_vec(), b"request-b".to_vec()]
    );

    let mut response =
        SimpleResponseBuilder::response_from_request(&forwarded, StatusCode::Ok, Some("OK"))
            .to(
                "Bob",
                "sip:bob@example.com",
                Some("downstream-response-tag"),
            )
            .content_type("application/sdp")
            .body(RESPONSE_BODY)
            .build();
    response.headers.push(TypedHeader::Other(
        HeaderName::Other(HEADER.into()),
        HeaderValue::Raw(b"response-a".to_vec()),
    ));
    response.headers.push(TypedHeader::Other(
        HeaderName::Other(HEADER.into()),
        HeaderValue::Raw(b"response-b".to_vec()),
    ));
    let expected_to = response.to().expect("downstream To").to_string();
    let expected_to_tag = response.to_tag();
    harness.inject(Message::Response(response), uas).await;

    let (forwarded_response, _) = harness
        .wait_for(1000, |message, destination| {
            is_response_for_method(message, StatusCode::Ok, Method::Invite) && *destination == uac
        })
        .await;
    let Message::Response(forwarded_response) = forwarded_response else {
        unreachable!();
    };
    assert_eq!(forwarded_response.body(), RESPONSE_BODY);
    assert_eq!(
        forwarded_response
            .to()
            .expect("forwarded response To")
            .to_string(),
        expected_to,
        "a proxy must not rewrite the downstream To value"
    );
    assert_eq!(
        forwarded_response.to_tag(),
        expected_to_tag,
        "a proxy must preserve the downstream To tag exactly"
    );
    let content_lengths = forwarded_response
        .headers
        .iter()
        .filter_map(|header| match header {
            TypedHeader::ContentLength(value) => Some(value.0),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        content_lengths,
        vec![RESPONSE_BODY.len() as u32],
        "the forwarded response must retain one correct Content-Length"
    );
    assert_eq!(
        extension_values(&forwarded_response.headers, HEADER),
        vec![b"response-a".to_vec(), b"response-b".to_vec()]
    );
    assert_eq!(
        forwarded_response.via_headers().len(),
        1,
        "the proxy must pop exactly its own Via and retain the upstream Via"
    );
}

#[tokio::test]
async fn forwarded_provisional_and_success_preserve_the_existing_to_tag() {
    const TO_TAG: &str = "existing-dialog-tag";

    let uas: SocketAddr = UAS_A.parse().unwrap();
    let uac: SocketAddr = UAC_ADDR.parse().unwrap();
    let harness = Harness::new(RouteDecision::to(uas)).await;
    let mut request = build_invite("forwarded-to-tag-invariance");
    for header in &mut request.headers {
        if let TypedHeader::To(to) = header {
            to.set_tag(TO_TAG);
        }
    }

    harness.inject(Message::Request(request), uac).await;
    let forwarded = harness.wait_for_invites(&[uas]).await.remove(0).1;
    assert_eq!(
        forwarded.to_tag().as_deref(),
        Some(TO_TAG),
        "the forwarded request must retain its existing To tag"
    );

    for status in [StatusCode::Ringing, StatusCode::Ok] {
        let response = response_for(&forwarded, status);
        harness.inject(response, uas).await;
        let (forwarded_response, _) = harness
            .wait_for(1000, |message, destination| {
                is_response_for_method(message, status, Method::Invite) && *destination == uac
            })
            .await;
        let Message::Response(forwarded_response) = forwarded_response else {
            unreachable!();
        };
        assert_eq!(
            forwarded_response.to_tag().as_deref(),
            Some(TO_TAG),
            "the proxy must not insert or rewrite the To tag on a forwarded response"
        );
    }
}

#[tokio::test]
async fn true_stray_invite_response_is_dropped() {
    let uas: SocketAddr = UAS_A.parse().unwrap();
    let harness = Harness::new(RouteDecision::to(uas)).await;
    let never_forwarded = build_invite("true-stray-response");
    let stray = response_for(&never_forwarded, StatusCode::Ok);

    harness.inject(stray, uas).await;
    tokio::time::sleep(Duration::from_millis(150)).await;

    assert!(
        harness.transport.sent().await.is_empty(),
        "RFC 6026 requires a true stray response with no matching transaction or retained route to be dropped"
    );
}

#[tokio::test]
async fn true_stray_non_invite_response_is_dropped() {
    let uas: SocketAddr = UAS_A.parse().unwrap();
    let harness = Harness::new(RouteDecision::to(uas)).await;
    let never_forwarded = build_options("true-stray-non-invite-response");
    let stray = response_for(&never_forwarded, StatusCode::Ok);

    harness.inject(stray, uas).await;
    tokio::time::sleep(Duration::from_millis(150)).await;

    assert!(
        harness.transport.sent().await.is_empty(),
        "RFC 4320 and RFC 6026 require a true stray non-INVITE response to be dropped"
    );
}
