//! Exact ownership tests for proxy-generated CANCEL transactions.
//!
//! The transport faults are injected at the conservative first-write boundary:
//! `prepare_message_route` failures prove zero wire, while `send_message`
//! failures are wire-unknown. These tests exercise the real transaction
//! manager and proxy event path without allocating per-transaction observers.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
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
use rvoip_sip_dialog::transaction::{
    CancelInviteTransactionDispatch, TransactionKey, TransactionManager,
};
use rvoip_sip_proxy::{ProxyConfig, ProxyRuntimeOptions, RouteDecision, RouteFn, StatefulProxy};
use rvoip_sip_transport::transport::{TransportRoute, TransportType};
use rvoip_sip_transport::{Error as TransportError, TransportEvent};
use tokio::sync::{mpsc, Mutex};

const PROXY_ADDR: &str = "127.0.0.1:5060";
const UAC_ADDR: &str = "10.0.0.5:5060";
const UAS_ADDR: &str = "10.0.0.20:5060";

const CANCEL_SUCCESS: u8 = 0;
const CANCEL_ZERO_WIRE_ONCE: u8 = 1;
const CANCEL_WIRE_UNKNOWN: u8 = 2;
const CANCEL_WIRE_UNKNOWN_WITH_IMMEDIATE_RESPONSE: u8 = 3;
const CANCEL_ZERO_WIRE_ALWAYS: u8 = 4;

#[derive(Debug)]
struct ClassifiedCancelTransport {
    local_addr: SocketAddr,
    ingress: mpsc::Sender<TransportEvent>,
    sent: Mutex<Vec<(Message, SocketAddr)>>,
    cancel_mode: AtomicU8,
    zero_wire_attempts: AtomicUsize,
}

impl ClassifiedCancelTransport {
    fn new(local_addr: SocketAddr, ingress: mpsc::Sender<TransportEvent>) -> Self {
        Self {
            local_addr,
            ingress,
            sent: Mutex::new(Vec::new()),
            cancel_mode: AtomicU8::new(CANCEL_SUCCESS),
            zero_wire_attempts: AtomicUsize::new(0),
        }
    }

    fn set_cancel_mode(&self, mode: u8) {
        self.cancel_mode.store(mode, Ordering::Release);
    }

    async fn sent(&self) -> Vec<(Message, SocketAddr)> {
        self.sent.lock().await.clone()
    }

    async fn cancel_requests(&self) -> Vec<(Request, SocketAddr)> {
        self.sent()
            .await
            .into_iter()
            .filter_map(|(message, destination)| match message {
                Message::Request(request) if request.method() == Method::Cancel => {
                    Some((request, destination))
                }
                _ => None,
            })
            .collect()
    }
}

#[async_trait]
impl rvoip_sip_transport::Transport for ClassifiedCancelTransport {
    async fn prepare_message_route(
        &self,
        message: &Message,
        route: TransportRoute,
    ) -> Result<TransportRoute, TransportError> {
        if matches!(message, Message::Request(request) if request.method() == Method::Cancel) {
            let mode = self.cancel_mode.load(Ordering::Acquire);
            if mode == CANCEL_ZERO_WIRE_ALWAYS {
                self.zero_wire_attempts.fetch_add(1, Ordering::AcqRel);
                return Err(TransportError::InvalidState(
                    "injected persistent pre-wire CANCEL route failure".into(),
                ));
            }
            if mode == CANCEL_ZERO_WIRE_ONCE
                && self.zero_wire_attempts.fetch_add(1, Ordering::AcqRel) == 0
            {
                self.cancel_mode.store(CANCEL_SUCCESS, Ordering::Release);
                return Err(TransportError::InvalidState(
                    "injected pre-wire CANCEL route failure".into(),
                ));
            }
        }
        Ok(route)
    }

    async fn send_message(
        &self,
        message: Message,
        destination: SocketAddr,
    ) -> Result<(), TransportError> {
        self.sent.lock().await.push((message.clone(), destination));

        if let Message::Request(request) = &message {
            if request.method() == Method::Cancel {
                let mode = self.cancel_mode.load(Ordering::Acquire);
                if mode == CANCEL_WIRE_UNKNOWN_WITH_IMMEDIATE_RESPONSE {
                    // Queue a loopback response before returning the injected
                    // write error. The proxy must already own the exact CANCEL
                    // generation when this response reaches its event stream.
                    let response =
                        SimpleResponseBuilder::response_from_request(request, StatusCode::Ok, None)
                            .build();
                    self.ingress
                        .send(TransportEvent::MessageReceived {
                            message: Message::Response(response),
                            source: destination,
                            destination: self.local_addr,
                            transport_type: TransportType::Udp,
                            flow_id: None,
                            raw_bytes: None,
                            timing: None,
                            connection_metadata: None,
                        })
                        .await
                        .expect("loopback CANCEL response");
                    tokio::task::yield_now().await;
                }
                if matches!(
                    mode,
                    CANCEL_WIRE_UNKNOWN | CANCEL_WIRE_UNKNOWN_WITH_IMMEDIATE_RESPONSE
                ) {
                    return Err(TransportError::ProtocolError(
                        "injected post-boundary CANCEL write failure".into(),
                    ));
                }
            }
        }
        Ok(())
    }

    fn local_addr(&self) -> Result<SocketAddr, TransportError> {
        Ok(self.local_addr)
    }

    async fn close(&self) -> Result<(), TransportError> {
        Ok(())
    }

    fn is_closed(&self) -> bool {
        false
    }
}

struct Harness {
    transport: Arc<ClassifiedCancelTransport>,
    ingress: mpsc::Sender<TransportEvent>,
    _tm: Arc<TransactionManager>,
    proxy: Arc<StatefulProxy>,
    _proxy_task: tokio::task::JoinHandle<()>,
}

impl Harness {
    async fn new(config: ProxyConfig) -> Self {
        let proxy_addr: SocketAddr = PROXY_ADDR.parse().unwrap();
        let uas: SocketAddr = UAS_ADDR.parse().unwrap();
        let (ingress, receiver) = mpsc::channel(128);
        let transport = Arc::new(ClassifiedCancelTransport::new(proxy_addr, ingress.clone()));
        let (tm, events) = TransactionManager::new(transport.clone(), receiver, Some(64))
            .await
            .expect("transaction manager");
        let tm = Arc::new(tm);
        let route_fn: RouteFn = Arc::new(move |_request| Some(RouteDecision::to(uas)));
        let proxy = StatefulProxy::with_options(
            tm.clone(),
            route_fn,
            config,
            ProxyRuntimeOptions::default().with_short_timer_c_for_tests(),
        );
        let proxy_task = proxy.clone().run(events);
        Self {
            transport,
            ingress,
            _tm: tm,
            proxy,
            _proxy_task: proxy_task,
        }
    }

    async fn inject(&self, message: Message, source: SocketAddr) {
        self.ingress
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
            .expect("transport injection");
    }

    async fn wait_for_request(&self, method: Method, destination: SocketAddr) -> Request {
        for _ in 0..2_048 {
            if let Some(request) = self.transport.sent().await.into_iter().find_map(
                |(message, actual)| match message {
                    Message::Request(request)
                        if request.method() == method && actual == destination =>
                    {
                        Some(request)
                    }
                    _ => None,
                },
            ) {
                return request;
            }
            tokio::task::yield_now().await;
        }
        panic!("timed out waiting for {method} to {destination}");
    }

    async fn start_proceeding_invite(&self, call_id: &str) -> Request {
        let uac: SocketAddr = UAC_ADDR.parse().unwrap();
        let uas: SocketAddr = UAS_ADDR.parse().unwrap();
        self.inject(Message::Request(build_invite(call_id)), uac)
            .await;
        let invite = self.wait_for_request(Method::Invite, uas).await;
        self.inject(response_for(&invite, StatusCode::Ringing), uas)
            .await;
        for _ in 0..32 {
            tokio::task::yield_now().await;
        }
        invite
    }
}

fn build_request(method: Method, call_id: &str) -> Request {
    SimpleRequestBuilder::new(method, "sip:bob@10.0.0.20:5060")
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

fn build_invite(call_id: &str) -> Request {
    build_request(Method::Invite, call_id)
}

fn build_cancel(call_id: &str) -> Request {
    build_request(Method::Cancel, call_id)
}

fn build_ack(call_id: &str) -> Request {
    build_request(Method::Ack, call_id)
}

fn response_for(request: &Request, status: StatusCode) -> Message {
    Message::Response(SimpleResponseBuilder::response_from_request(request, status, None).build())
}

async fn wait_for_snapshot(
    proxy: &StatefulProxy,
    predicate: impl Fn(rvoip_sip_proxy::ProxyRetentionSnapshot) -> bool,
) -> rvoip_sip_proxy::ProxyRetentionSnapshot {
    for _ in 0..2_048 {
        let snapshot = proxy.retention_snapshot();
        if predicate(snapshot) {
            return snapshot;
        }
        tokio::task::yield_now().await;
    }
    panic!(
        "timed out waiting for retention snapshot; current={:?}",
        proxy.retention_snapshot()
    );
}

async fn create_outbound_invite(
    tm: &TransactionManager,
    destination: SocketAddr,
    call_id: &str,
) -> TransactionKey {
    let transaction_id = tm
        .create_client_transaction(build_invite(call_id), destination)
        .await
        .expect("create INVITE client transaction");
    tm.send_request(&transaction_id).await.expect("send INVITE");
    transaction_id
}

#[tokio::test]
async fn classified_cancel_api_preserves_all_first_write_outcomes() {
    let proxy_addr: SocketAddr = PROXY_ADDR.parse().unwrap();
    let uas: SocketAddr = UAS_ADDR.parse().unwrap();

    // Successful first write returns the exact RFC CANCEL key.
    let (success_ingress, success_receiver) = mpsc::channel(32);
    let success_transport = Arc::new(ClassifiedCancelTransport::new(proxy_addr, success_ingress));
    let (success_tm, _events) =
        TransactionManager::new(success_transport.clone(), success_receiver, Some(16))
            .await
            .unwrap();
    let invite = create_outbound_invite(&success_tm, uas, "classified-success").await;
    match success_tm
        .cancel_invite_transaction_classified(&invite)
        .await
    {
        CancelInviteTransactionDispatch::Success { transaction_id } => {
            assert_eq!(transaction_id, invite.with_method(Method::Cancel));
        }
        other => panic!("expected successful CANCEL dispatch, got {other:?}"),
    }
    match success_tm
        .cancel_invite_transaction_classified(&invite.with_method(Method::Options))
        .await
    {
        CancelInviteTransactionDispatch::ZeroWire {
            retired_transaction_id,
            ..
        } => assert_eq!(retired_transaction_id, None),
        other => panic!("expected pre-admission zero-wire failure, got {other:?}"),
    }

    // A preparation failure is proven zero wire and permits an exact-key retry.
    let (zero_ingress, zero_receiver) = mpsc::channel(32);
    let zero_transport = Arc::new(ClassifiedCancelTransport::new(proxy_addr, zero_ingress));
    let (zero_tm, _events) =
        TransactionManager::new(zero_transport.clone(), zero_receiver, Some(16))
            .await
            .unwrap();
    let invite = create_outbound_invite(&zero_tm, uas, "classified-zero-wire").await;
    zero_transport.set_cancel_mode(CANCEL_ZERO_WIRE_ONCE);
    match zero_tm.cancel_invite_transaction_classified(&invite).await {
        CancelInviteTransactionDispatch::ZeroWire {
            retired_transaction_id,
            ..
        } => {
            assert_eq!(
                retired_transaction_id,
                Some(invite.with_method(Method::Cancel))
            );
        }
        other => panic!("expected exact retired zero-wire generation, got {other:?}"),
    }
    assert!(zero_transport.cancel_requests().await.is_empty());
    match zero_tm.cancel_invite_transaction_classified(&invite).await {
        CancelInviteTransactionDispatch::Success { transaction_id } => {
            assert_eq!(transaction_id, invite.with_method(Method::Cancel));
        }
        other => panic!("expected safe retry success, got {other:?}"),
    }

    // A post-boundary failure retains the exact ambiguous generation.
    let (unknown_ingress, unknown_receiver) = mpsc::channel(32);
    let unknown_transport = Arc::new(ClassifiedCancelTransport::new(proxy_addr, unknown_ingress));
    let (unknown_tm, _events) =
        TransactionManager::new(unknown_transport.clone(), unknown_receiver, Some(16))
            .await
            .unwrap();
    let invite = create_outbound_invite(&unknown_tm, uas, "classified-wire-unknown").await;
    unknown_transport.set_cancel_mode(CANCEL_WIRE_UNKNOWN);
    match unknown_tm
        .cancel_invite_transaction_classified(&invite)
        .await
    {
        CancelInviteTransactionDispatch::WireUnknown { transaction_id, .. } => {
            assert_eq!(transaction_id, invite.with_method(Method::Cancel));
        }
        other => panic!("expected wire-unknown dispatch, got {other:?}"),
    }
    assert_eq!(unknown_transport.cancel_requests().await.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn zero_wire_cancel_retries_at_t1_without_duplicate_wire_generation() {
    let uac: SocketAddr = UAC_ADDR.parse().unwrap();
    let uas: SocketAddr = UAS_ADDR.parse().unwrap();
    let harness = Harness::new(ProxyConfig {
        timer_c: Duration::from_secs(5),
        ..ProxyConfig::default()
    })
    .await;
    let call_id = "proxy-cancel-zero-wire-retry";
    harness.start_proceeding_invite(call_id).await;
    harness.transport.set_cancel_mode(CANCEL_ZERO_WIRE_ONCE);

    harness
        .inject(Message::Request(build_cancel(call_id)), uac)
        .await;
    let pending = wait_for_snapshot(&harness.proxy, |snapshot| {
        snapshot.generated_cancel_transactions == 0 && snapshot.generated_cancel_retry_entries == 1
    })
    .await;
    assert_eq!(pending.generated_cancel_transactions, 0);
    assert!(harness.transport.cancel_requests().await.is_empty());

    tokio::time::advance(Duration::from_millis(499)).await;
    for _ in 0..32 {
        tokio::task::yield_now().await;
    }
    assert!(harness.transport.cancel_requests().await.is_empty());

    tokio::time::advance(Duration::from_millis(2)).await;
    let cancel = harness.wait_for_request(Method::Cancel, uas).await;
    assert_eq!(
        TransactionKey::from_request(&cancel)
            .expect("CANCEL key")
            .method(),
        &Method::Cancel
    );
    let owned = wait_for_snapshot(&harness.proxy, |snapshot| {
        snapshot.generated_cancel_transactions == 1 && snapshot.generated_cancel_retry_entries == 0
    })
    .await;
    assert_eq!(owned.generated_cancel_transactions, 1);
    harness
        .inject(response_for(&cancel, StatusCode::Ok), uas)
        .await;
    for _ in 0..32 {
        tokio::task::yield_now().await;
    }

    // A retransmitted upstream CANCEL is absorbed by its server transaction.
    harness
        .inject(Message::Request(build_cancel(call_id)), uac)
        .await;
    tokio::time::advance(Duration::from_secs(5)).await;
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
    assert_eq!(harness.transport.cancel_requests().await.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn persistent_zero_wire_cancel_uses_bounded_backoff_and_final_clears_retry() {
    let uac: SocketAddr = UAC_ADDR.parse().unwrap();
    let uas: SocketAddr = UAS_ADDR.parse().unwrap();
    let harness = Harness::new(ProxyConfig {
        timer_c: Duration::from_secs(20),
        ..ProxyConfig::default()
    })
    .await;
    let call_id = "proxy-cancel-zero-wire-backoff";
    let invite = harness.start_proceeding_invite(call_id).await;
    harness.transport.set_cancel_mode(CANCEL_ZERO_WIRE_ALWAYS);

    harness
        .inject(Message::Request(build_cancel(call_id)), uac)
        .await;
    wait_for_snapshot(&harness.proxy, |snapshot| {
        snapshot.generated_cancel_transactions == 0 && snapshot.generated_cancel_retry_entries == 1
    })
    .await;
    assert_eq!(
        harness.transport.zero_wire_attempts.load(Ordering::Acquire),
        1
    );

    // Retry intervals are T1, 2*T1, 4*T1, then capped at T2.
    for (advance, expected_attempts) in [
        (Duration::from_millis(500), 2),
        (Duration::from_secs(1), 3),
        (Duration::from_secs(2), 4),
        (Duration::from_secs(4), 5),
        (Duration::from_secs(4), 6),
    ] {
        tokio::time::advance(advance).await;
        for _ in 0..64 {
            tokio::task::yield_now().await;
        }
        wait_for_snapshot(&harness.proxy, |snapshot| {
            snapshot.generated_cancel_transactions == 0
                && snapshot.generated_cancel_retry_entries == 1
        })
        .await;
        assert_eq!(
            harness.transport.zero_wire_attempts.load(Ordering::Acquire),
            expected_attempts
        );
    }
    assert!(
        harness.transport.cancel_requests().await.is_empty(),
        "proven pre-wire failures must not be represented as wire sends"
    );

    harness
        .inject(response_for(&invite, StatusCode::RequestTerminated), uas)
        .await;
    wait_for_snapshot(&harness.proxy, |snapshot| {
        snapshot.generated_cancel_retry_entries == 0
    })
    .await;
    let attempts_after_final = harness.transport.zero_wire_attempts.load(Ordering::Acquire);
    tokio::time::advance(Duration::from_secs(8)).await;
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        harness.transport.zero_wire_attempts.load(Ordering::Acquire),
        attempts_after_final,
        "a final branch response must cancel every pending retry deadline"
    );
}

#[tokio::test(start_paused = true)]
async fn wire_unknown_cancel_is_preowned_consumed_and_cleaned_exactly_once() {
    let uac: SocketAddr = UAC_ADDR.parse().unwrap();
    let uas: SocketAddr = UAS_ADDR.parse().unwrap();
    let harness = Harness::new(ProxyConfig {
        timer_c: Duration::from_secs(5),
        ..ProxyConfig::default()
    })
    .await;
    let call_id = "proxy-cancel-wire-unknown";
    let invite = harness.start_proceeding_invite(call_id).await;
    harness
        .transport
        .set_cancel_mode(CANCEL_WIRE_UNKNOWN_WITH_IMMEDIATE_RESPONSE);

    harness
        .inject(Message::Request(build_cancel(call_id)), uac)
        .await;
    harness.wait_for_request(Method::Cancel, uas).await;
    let owned = wait_for_snapshot(&harness.proxy, |snapshot| {
        snapshot.generated_cancel_transactions == 1
    })
    .await;
    assert_eq!(owned.generated_cancel_retry_entries, 0);

    // The immediate downstream 200 to the generated CANCEL must be consumed;
    // only the independent upstream CANCEL transaction gets a 200.
    for _ in 0..128 {
        tokio::task::yield_now().await;
    }
    let upstream_cancel_ok = harness
        .transport
        .sent()
        .await
        .iter()
        .filter(|(message, destination)| {
            *destination == uac
                && matches!(
                    message,
                    Message::Response(response)
                        if response.status() == StatusCode::Ok
                            && response.cseq().is_some_and(|cseq| cseq.method == Method::Cancel)
                )
        })
        .count();
    assert_eq!(upstream_cancel_ok, 1);
    assert_eq!(harness.transport.cancel_requests().await.len(), 1);

    // Settle the INVITE legs and prove the exact generated generation remains
    // part of the response-context drain fence, then expires with that context.
    harness
        .inject(response_for(&invite, StatusCode::RequestTerminated), uas)
        .await;
    for _ in 0..128 {
        tokio::task::yield_now().await;
    }
    harness
        .inject(Message::Request(build_ack(call_id)), uac)
        .await;

    for _ in 0..12 {
        tokio::time::advance(Duration::from_secs(5)).await;
        for _ in 0..32 {
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
    let draining = harness.proxy.retention_snapshot();
    assert_eq!(draining.response_contexts, 1);
    assert_eq!(draining.generated_cancel_transactions, 1);
    assert_eq!(draining.response_context_deadlines, 1);

    tokio::time::advance(Duration::from_secs(65)).await;
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
    let drained = harness.proxy.retention_snapshot();
    assert_eq!(drained.response_contexts, 0);
    assert_eq!(drained.downstream_invite_indexes, 0);
    assert_eq!(drained.generated_cancel_transactions, 0);
    assert_eq!(drained.generated_cancel_retry_entries, 0);
    assert_eq!(drained.generated_cancel_retry_heap_entries, 0);
    assert_eq!(drained.response_context_deadlines, 0);

    // No later retry may appear after a wire-unknown generation.
    tokio::time::advance(Duration::from_secs(10)).await;
    for _ in 0..32 {
        tokio::task::yield_now().await;
    }
    assert_eq!(harness.transport.cancel_requests().await.len(), 1);
}
