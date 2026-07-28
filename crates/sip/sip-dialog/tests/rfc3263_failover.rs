//! Acceptance tests for RFC 3263 §4.3 multi-candidate failover.
//!
//! Covers two layers:
//!
//! 1. **The primitive** — `MultiplexedTransport::send_message_with_failover`
//!    walks a candidate list, advancing on recoverable transport errors
//!    and failing fast on non-recoverable ones.
//!
//! 2. **The resolve API** — `DialogManager::resolve_uri_to_candidates`
//!    returns the FULL candidate list (not just the first) so callers
//!    can do §4.3 failover.

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rvoip_sip_core::types::headers::HeaderAccess;
use rvoip_sip_core::{Message, Method, Request, Uri};
use rvoip_sip_dialog::manager::transaction_integration::CandidateWirePlan;
use rvoip_sip_dialog::transaction::timer::TimerSettings;
use rvoip_sip_dialog::transaction::transport::multiplexed::MultiplexedTransport;
use rvoip_sip_dialog::transaction::{
    ClientTransactionFailure, ClientTransactionOutcome, TransactionKey, TransactionManager,
};
use rvoip_sip_dialog::DialogManager;
use rvoip_sip_transport::error::Error as TransportError;
use rvoip_sip_transport::resolver::{ResolvedTarget, Resolver, ResolverError};
use rvoip_sip_transport::transport::{TransportEvent, TransportType};
use rvoip_sip_transport::Transport;
use tokio::sync::mpsc;

/// A mock transport whose `send_message` outcome can be programmed
/// per-destination. The default for any address not in `outcomes` is
/// `Ok(())`.
#[derive(Debug, Clone)]
struct ProgrammableTransport {
    local_addr: SocketAddr,
    outcomes: Arc<Mutex<HashMap<SocketAddr, Outcome>>>,
    method_outcomes: Arc<Mutex<HashMap<(SocketAddr, Method), VecDeque<Outcome>>>>,
    sends: Arc<Mutex<Vec<SocketAddr>>>,
    branches: Arc<Mutex<Vec<Option<String>>>>,
    via_transports: Arc<Mutex<Vec<Option<String>>>>,
    via_sent_by: Arc<Mutex<Vec<Option<String>>>>,
    contacts: Arc<Mutex<Vec<Option<String>>>>,
    messages: Arc<Mutex<Vec<(Message, SocketAddr)>>>,
}

/// Programmable per-attempt outcome for the failover tests. `Ok` is
/// kept alongside the failure variants for symmetry; today the tests
/// only schedule failures, but mixing in an `Ok` is essential when
/// extending coverage to recovery-after-success scenarios.
#[allow(dead_code)]
#[derive(Debug, Clone)]
enum Outcome {
    Ok,
    RecoverableFail(String),
    DelayedRecoverableFail(std::time::Duration, String),
    FatalFail(String),
}

impl ProgrammableTransport {
    fn new(local_addr: SocketAddr) -> Self {
        Self {
            local_addr,
            outcomes: Arc::new(Mutex::new(HashMap::new())),
            method_outcomes: Arc::new(Mutex::new(HashMap::new())),
            sends: Arc::new(Mutex::new(Vec::new())),
            branches: Arc::new(Mutex::new(Vec::new())),
            via_transports: Arc::new(Mutex::new(Vec::new())),
            via_sent_by: Arc::new(Mutex::new(Vec::new())),
            contacts: Arc::new(Mutex::new(Vec::new())),
            messages: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn program(&self, addr: SocketAddr, outcome: Outcome) {
        self.outcomes.lock().unwrap().insert(addr, outcome);
    }

    fn program_method_sequence(
        &self,
        addr: SocketAddr,
        method: Method,
        outcomes: impl IntoIterator<Item = Outcome>,
    ) {
        self.method_outcomes
            .lock()
            .unwrap()
            .insert((addr, method), outcomes.into_iter().collect());
    }

    fn sends(&self) -> Vec<SocketAddr> {
        self.sends.lock().unwrap().clone()
    }

    fn branches(&self) -> Vec<Option<String>> {
        self.branches.lock().unwrap().clone()
    }

    fn via_transports(&self) -> Vec<Option<String>> {
        self.via_transports.lock().unwrap().clone()
    }

    fn via_sent_by(&self) -> Vec<Option<String>> {
        self.via_sent_by.lock().unwrap().clone()
    }

    fn contacts(&self) -> Vec<Option<String>> {
        self.contacts.lock().unwrap().clone()
    }

    fn messages(&self) -> Vec<(Message, SocketAddr)> {
        self.messages.lock().unwrap().clone()
    }
}

#[async_trait]
impl Transport for ProgrammableTransport {
    async fn send_message(
        &self,
        message: rvoip_sip_core::Message,
        destination: SocketAddr,
    ) -> Result<(), TransportError> {
        self.sends.lock().unwrap().push(destination);
        self.messages
            .lock()
            .unwrap()
            .push((message.clone(), destination));
        let branch = match &message {
            rvoip_sip_core::Message::Request(request) => request
                .first_via()
                .and_then(|via| via.branch().map(str::to_string)),
            rvoip_sip_core::Message::Response(_) => None,
        };
        self.branches.lock().unwrap().push(branch);
        let (via_transport, via_sent_by, contact) = match &message {
            rvoip_sip_core::Message::Request(request) => {
                let via = request.first_via();
                let top_via = via.as_ref().and_then(|via| via.0.first());
                (
                    top_via.map(|via| via.transport().to_string()),
                    top_via.map(|via| format!("{}:{}", via.host(), via.port().unwrap_or(5060))),
                    request.raw_header_value(&rvoip_sip_core::HeaderName::Contact),
                )
            }
            rvoip_sip_core::Message::Response(_) => (None, None, None),
        };
        self.via_transports.lock().unwrap().push(via_transport);
        self.via_sent_by.lock().unwrap().push(via_sent_by);
        self.contacts.lock().unwrap().push(contact);
        let method = message.method();
        let method_outcome = method.and_then(|method| {
            self.method_outcomes
                .lock()
                .unwrap()
                .get_mut(&(destination, method))
                .and_then(VecDeque::pop_front)
        });
        let outcome =
            method_outcome.or_else(|| self.outcomes.lock().unwrap().get(&destination).cloned());
        match outcome {
            None | Some(Outcome::Ok) => Ok(()),
            Some(Outcome::RecoverableFail(msg)) => Err(TransportError::ConnectFailed(
                destination,
                std::io::Error::new(std::io::ErrorKind::ConnectionRefused, msg),
            )),
            Some(Outcome::DelayedRecoverableFail(delay, msg)) => {
                tokio::time::sleep(delay).await;
                Err(TransportError::ConnectFailed(
                    destination,
                    std::io::Error::new(std::io::ErrorKind::ConnectionRefused, msg),
                ))
            }
            Some(Outcome::FatalFail(_msg)) => {
                // MessageTooLarge is marked non-recoverable by Error::is_recoverable().
                Err(TransportError::MessageTooLarge(99999))
            }
        }
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

fn mux(transport: Arc<dyn Transport>) -> MultiplexedTransport {
    MultiplexedTransport::new_without_trace(transport, HashMap::new()).expect("mux build")
}

fn target(addr: &str, transport: TransportType) -> ResolvedTarget {
    ResolvedTarget::immediate(addr.parse().unwrap(), transport)
}

fn make_invite() -> rvoip_sip_core::Message {
    rvoip_sip_core::Message::Request(Request::new(
        Method::Invite,
        Uri::from_str("sip:bob@example.com").unwrap(),
    ))
}

// ---------- 1. Primitive: send_message_with_failover ----------

#[tokio::test]
async fn empty_candidate_list_yields_invalid_address() {
    let inner = Arc::new(ProgrammableTransport::new(
        "127.0.0.1:5060".parse().unwrap(),
    ));
    let mux = mux(inner.clone());

    let err = mux
        .send_message_with_failover(make_invite(), &[])
        .await
        .unwrap_err();
    match err {
        TransportError::InvalidAddress(msg) => assert!(msg.contains("no candidates")),
        other => panic!("expected InvalidAddress, got {:?}", other),
    }
    assert!(inner.sends().is_empty(), "no sends should have been issued");
}

#[tokio::test]
async fn single_candidate_succeeds_yields_addr() {
    let inner = Arc::new(ProgrammableTransport::new(
        "127.0.0.1:5060".parse().unwrap(),
    ));
    let mux = mux(inner.clone());

    let candidates = vec![target("10.0.0.1:5060", TransportType::Udp)];
    let addr = mux
        .send_message_with_failover(make_invite(), &candidates)
        .await
        .expect("send ok");
    assert_eq!(addr.to_string(), "10.0.0.1:5060");
    assert_eq!(inner.sends(), vec![candidates[0].addr]);
}

#[tokio::test]
async fn first_candidate_recoverable_failure_falls_over_to_second() {
    let inner = Arc::new(ProgrammableTransport::new(
        "127.0.0.1:5060".parse().unwrap(),
    ));
    let mux = mux(inner.clone());
    inner.program(
        "10.0.0.1:5060".parse().unwrap(),
        Outcome::RecoverableFail("connection refused".into()),
    );

    let candidates = vec![
        target("10.0.0.1:5060", TransportType::Tcp),
        target("10.0.0.2:5060", TransportType::Udp),
    ];
    let addr = mux
        .send_message_with_failover(make_invite(), &candidates)
        .await
        .expect("second candidate ok");
    assert_eq!(addr.to_string(), "10.0.0.2:5060");
    // Both candidates were attempted (in order).
    assert_eq!(inner.sends().len(), 2);
    assert_eq!(inner.sends()[0].to_string(), "10.0.0.1:5060");
    assert_eq!(inner.sends()[1].to_string(), "10.0.0.2:5060");
}

#[tokio::test]
async fn all_candidates_fail_recoverably_returns_last_error() {
    let inner = Arc::new(ProgrammableTransport::new(
        "127.0.0.1:5060".parse().unwrap(),
    ));
    let mux = mux(inner.clone());
    inner.program(
        "10.0.0.1:5060".parse().unwrap(),
        Outcome::RecoverableFail("refused".into()),
    );
    inner.program(
        "10.0.0.2:5060".parse().unwrap(),
        Outcome::RecoverableFail("timeout".into()),
    );

    let candidates = vec![
        target("10.0.0.1:5060", TransportType::Tcp),
        target("10.0.0.2:5060", TransportType::Tcp),
    ];
    let err = mux
        .send_message_with_failover(make_invite(), &candidates)
        .await
        .unwrap_err();
    assert!(
        matches!(err, TransportError::ConnectFailed(_, _)),
        "expected ConnectFailed, got {:?}",
        err
    );
    assert_eq!(inner.sends().len(), 2);
}

#[tokio::test]
async fn non_recoverable_failure_aborts_immediately() {
    let inner = Arc::new(ProgrammableTransport::new(
        "127.0.0.1:5060".parse().unwrap(),
    ));
    let mux = mux(inner.clone());
    // MessageTooLarge is non-recoverable per Error::is_recoverable.
    inner.program(
        "10.0.0.1:5060".parse().unwrap(),
        Outcome::FatalFail("too big".into()),
    );

    let candidates = vec![
        target("10.0.0.1:5060", TransportType::Tcp),
        target("10.0.0.2:5060", TransportType::Udp),
    ];
    let err = mux
        .send_message_with_failover(make_invite(), &candidates)
        .await
        .unwrap_err();
    assert!(
        matches!(err, TransportError::MessageTooLarge(_)),
        "expected fail-fast MessageTooLarge, got {:?}",
        err
    );
    // Second candidate must NOT be tried.
    assert_eq!(inner.sends().len(), 1);
    assert_eq!(inner.sends()[0].to_string(), "10.0.0.1:5060");
}

// ---------- 2. resolve_uri_to_candidates ----------

#[derive(Default)]
struct MultiCandResolver {
    response: Mutex<Vec<ResolvedTarget>>,
    err: Mutex<Option<ResolverError>>,
}

impl MultiCandResolver {
    fn with(targets: Vec<ResolvedTarget>) -> Self {
        Self {
            response: Mutex::new(targets),
            err: Mutex::new(None),
        }
    }
    fn with_err(e: ResolverError) -> Self {
        Self {
            response: Mutex::new(Vec::new()),
            err: Mutex::new(Some(e)),
        }
    }
}

#[async_trait]
impl Resolver for MultiCandResolver {
    async fn resolve(&self, _uri: &Uri) -> Result<Vec<ResolvedTarget>, ResolverError> {
        if let Some(e) = self.err.lock().unwrap().take() {
            return Err(e);
        }
        Ok(self.response.lock().unwrap().clone())
    }
}

async fn build_manager() -> Arc<DialogManager> {
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    let transport = Arc::new(ProgrammableTransport::new(local_addr));
    let (_tx, transport_rx) = mpsc::channel(8);
    let (transaction_manager, mut event_rx) =
        TransactionManager::new(transport.clone(), transport_rx, Some(16))
            .await
            .expect("TransactionManager::new");
    tokio::spawn(async move { while event_rx.recv().await.is_some() {} });
    Arc::new(
        DialogManager::new(Arc::new(transaction_manager), local_addr)
            .await
            .expect("DialogManager::new"),
    )
}

async fn build_manager_with_transport() -> (Arc<DialogManager>, Arc<ProgrammableTransport>) {
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    let transport = Arc::new(ProgrammableTransport::new(local_addr));
    let (_tx, transport_rx) = mpsc::channel(8);
    let (transaction_manager, mut event_rx) =
        TransactionManager::new(transport.clone(), transport_rx, Some(16))
            .await
            .expect("TransactionManager::new");
    tokio::spawn(async move { while event_rx.recv().await.is_some() {} });
    let manager = Arc::new(
        DialogManager::new(Arc::new(transaction_manager), local_addr)
            .await
            .expect("DialogManager::new"),
    );
    (manager, transport)
}

async fn build_event_manager_with_transport() -> (
    Arc<DialogManager>,
    Arc<ProgrammableTransport>,
    mpsc::Sender<TransportEvent>,
) {
    build_event_manager_with_transport_and_timers(None).await
}

async fn build_event_manager_with_transport_and_timers(
    timer_settings: Option<TimerSettings>,
) -> (
    Arc<DialogManager>,
    Arc<ProgrammableTransport>,
    mpsc::Sender<TransportEvent>,
) {
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    let transport = Arc::new(ProgrammableTransport::new(local_addr));
    let (transport_tx, transport_rx) = mpsc::channel(64);
    let (transaction_manager, event_rx) = TransactionManager::new_with_config(
        transport.clone(),
        transport_rx,
        Some(64),
        timer_settings,
    )
    .await
    .expect("TransactionManager::new_with_config");
    let manager = Arc::new(
        DialogManager::with_global_events(Arc::new(transaction_manager), event_rx, local_addr)
            .await
            .expect("DialogManager::with_global_events"),
    );
    (manager, transport, transport_tx)
}

fn requests_sent_as(
    transport: &ProgrammableTransport,
    method: Method,
) -> Vec<(Request, SocketAddr)> {
    transport
        .messages()
        .into_iter()
        .filter_map(|(message, destination)| match message {
            Message::Request(request) if request.method() == method => Some((request, destination)),
            _ => None,
        })
        .collect()
}

async fn wait_for_request_count(
    transport: &ProgrammableTransport,
    method: Method,
    count: usize,
) -> Vec<(Request, SocketAddr)> {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let requests = requests_sent_as(transport, method.clone());
            if requests.len() >= count {
                return requests;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "timed out waiting for {count} {} request(s); messages={:?}",
            method,
            transport.messages()
        )
    })
}

fn client_transaction_key_for_request(request: &Request) -> TransactionKey {
    let branch = request
        .first_via()
        .and_then(|via| via.branch().map(str::to_owned))
        .expect("client request Via branch");
    TransactionKey::new(branch, request.method().clone(), false)
}

fn unique_requests_sent_as(
    transport: &ProgrammableTransport,
    method: Method,
) -> Vec<(Request, SocketAddr)> {
    let mut branches = HashSet::new();
    requests_sent_as(transport, method)
        .into_iter()
        .filter(|(request, _)| {
            request
                .first_via()
                .and_then(|via| via.branch().map(str::to_owned))
                .is_some_and(|branch| branches.insert(branch))
        })
        .collect()
}

async fn wait_for_unique_request_count(
    transport: &ProgrammableTransport,
    method: Method,
    count: usize,
) -> Vec<(Request, SocketAddr)> {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let requests = unique_requests_sent_as(transport, method.clone());
            if requests.len() >= count {
                return requests;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "timed out waiting for {count} unique {} request(s); messages={:?}",
            method,
            transport.messages()
        )
    })
}

async fn send_test_initial_invite(
    manager: &DialogManager,
    candidates: Vec<ResolvedTarget>,
    call_id: &str,
) -> (
    rvoip_sip_dialog::dialog::DialogId,
    rvoip_sip_dialog::transaction::TransactionKey,
) {
    use rvoip_sip_core::builder::SimpleRequestBuilder;

    let local_uri: Uri = "sip:alice@127.0.0.1:5060".parse().unwrap();
    let remote_uri: Uri = "sip:bob@service.example.com".parse().unwrap();
    let dialog_id = manager
        .create_outgoing_dialog(local_uri.clone(), remote_uri.clone(), Some(call_id.into()))
        .await
        .expect("create outgoing dialog");
    let local_tag = format!(
        "local-{}",
        call_id.replace(|character: char| !character.is_ascii_alphanumeric(), "")
    );
    manager
        .get_dialog_mut(&dialog_id)
        .expect("dialog")
        .local_tag = Some(local_tag.clone());
    let request = SimpleRequestBuilder::new(Method::Invite, &remote_uri.to_string())
        .unwrap()
        .from("alice", &local_uri.to_string(), Some(&local_tag))
        .to("bob", &remote_uri.to_string(), None)
        .contact("sip:alice@127.0.0.1:5060", None)
        .call_id(call_id)
        .cseq(1)
        .max_forwards(70)
        .via("127.0.0.1:5060", "UDP", Some("z9hG4bK-pre-plan-template"))
        .build();
    let (transaction_id, _) = manager
        .send_request_with_candidate_wire_plan(
            request,
            candidates,
            Some(&dialog_id),
            CandidateWirePlan::default(),
        )
        .await
        .expect("send initial invite");
    (dialog_id, transaction_id)
}

fn response_for_invite(
    request: &Request,
    status: rvoip_sip_core::StatusCode,
    to_tag: &str,
    contact: Option<&str>,
) -> rvoip_sip_core::Response {
    use rvoip_sip_core::builder::SimpleResponseBuilder;

    let to_uri = request.to().expect("INVITE To").address().uri.to_string();
    let mut builder = SimpleResponseBuilder::response_from_request(request, status, None).to(
        "bob",
        &to_uri,
        Some(to_tag),
    );
    if let Some(contact) = contact {
        builder = builder.contact(contact, None);
    }
    builder.build()
}

fn response_for_request(
    request: &Request,
    status: rvoip_sip_core::StatusCode,
) -> rvoip_sip_core::Response {
    use rvoip_sip_core::builder::SimpleResponseBuilder;

    SimpleResponseBuilder::response_from_request(request, status, None).build()
}

async fn inject_response(
    transport_tx: &mpsc::Sender<TransportEvent>,
    response: rvoip_sip_core::Response,
    source: SocketAddr,
) {
    transport_tx
        .send(TransportEvent::MessageReceived {
            message: Message::Response(response),
            source,
            destination: "127.0.0.1:5060".parse().unwrap(),
            transport_type: TransportType::Udp,
            flow_id: None,
            raw_bytes: None,
            timing: None,
            connection_metadata: None,
        })
        .await
        .expect("inject response");
}

#[tokio::test]
async fn retained_invite_plan_advances_on_503_then_accepts_second_candidate() {
    use rvoip_sip_core::StatusCode;
    use rvoip_sip_dialog::dialog::DialogState;

    let (manager, transport, transport_tx) = build_event_manager_with_transport().await;
    let first: SocketAddr = "10.0.1.1:5060".parse().unwrap();
    let second: SocketAddr = "10.0.1.2:5060".parse().unwrap();
    let (dialog_id, _) = send_test_initial_invite(
        &manager,
        vec![
            ResolvedTarget::immediate(first, TransportType::Udp),
            ResolvedTarget::immediate(second, TransportType::Udp),
        ],
        "retained-plan-503-then-200",
    )
    .await;
    let first_invite = wait_for_request_count(&transport, Method::Invite, 1).await[0]
        .0
        .clone();

    inject_response(
        &transport_tx,
        response_for_invite(
            &first_invite,
            StatusCode::ServiceUnavailable,
            "first-503",
            None,
        ),
        first,
    )
    .await;
    let invites = wait_for_request_count(&transport, Method::Invite, 2).await;
    assert_eq!(invites[0].1, first);
    assert_eq!(invites[1].1, second);
    assert_ne!(
        invites[0]
            .0
            .first_via()
            .and_then(|via| via.branch().map(str::to_owned)),
        invites[1]
            .0
            .first_via()
            .and_then(|via| via.branch().map(str::to_owned)),
        "a failover attempt must be a fresh client transaction"
    );

    let acknowledgements_before = requests_sent_as(&transport, Method::Ack).len();
    inject_response(
        &transport_tx,
        response_for_invite(
            &invites[1].0,
            StatusCode::Ok,
            "selected-second",
            Some("sip:bob@10.0.1.2:5060"),
        ),
        second,
    )
    .await;
    wait_for_request_count(&transport, Method::Ack, acknowledgements_before + 1).await;
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if matches!(
                manager.get_dialog_state(&dialog_id),
                Ok(DialogState::Confirmed)
            ) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("second candidate confirms dialog");
}

#[tokio::test]
async fn retained_invite_plan_reports_503_after_candidates_are_exhausted() {
    use rvoip_sip_core::StatusCode;

    let (manager, transport, transport_tx) = build_event_manager_with_transport().await;
    let only: SocketAddr = "10.0.1.9:5060".parse().unwrap();
    send_test_initial_invite(
        &manager,
        vec![ResolvedTarget::immediate(only, TransportType::Udp)],
        "retained-plan-503-exhausted",
    )
    .await;
    let invite = wait_for_request_count(&transport, Method::Invite, 1).await[0]
        .0
        .clone();
    inject_response(
        &transport_tx,
        response_for_invite(&invite, StatusCode::ServiceUnavailable, "only-503", None),
        only,
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(requests_sent_as(&transport, Method::Invite).len(), 1);
}

#[tokio::test]
async fn duplicate_selected_invite_success_is_reacked_without_fork_bye() {
    use rvoip_sip_core::StatusCode;

    let (manager, transport, transport_tx) = build_event_manager_with_transport().await;
    let destination: SocketAddr = "10.0.1.10:5060".parse().unwrap();
    send_test_initial_invite(
        &manager,
        vec![ResolvedTarget::immediate(destination, TransportType::Udp)],
        "retained-plan-selected-duplicate",
    )
    .await;
    let invite = wait_for_request_count(&transport, Method::Invite, 1).await[0]
        .0
        .clone();
    let success = response_for_invite(
        &invite,
        StatusCode::Ok,
        "selected-duplicate",
        Some("sip:bob@10.0.1.10:5060"),
    );
    inject_response(&transport_tx, success.clone(), destination).await;
    let first_ack_count = wait_for_request_count(&transport, Method::Ack, 1)
        .await
        .len();
    inject_response(&transport_tx, success, destination).await;
    wait_for_request_count(&transport, Method::Ack, first_ack_count + 1).await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(requests_sent_as(&transport, Method::Bye).is_empty());
}

#[tokio::test]
async fn retained_invite_plan_does_not_advance_after_provisional_response() {
    use rvoip_sip_core::StatusCode;

    let (manager, transport, transport_tx) = build_event_manager_with_transport().await;
    let first: SocketAddr = "10.0.2.1:5060".parse().unwrap();
    let second: SocketAddr = "10.0.2.2:5060".parse().unwrap();
    send_test_initial_invite(
        &manager,
        vec![
            ResolvedTarget::immediate(first, TransportType::Udp),
            ResolvedTarget::immediate(second, TransportType::Udp),
        ],
        "retained-plan-provisional-no-failover",
    )
    .await;
    let first_invite = wait_for_request_count(&transport, Method::Invite, 1).await[0]
        .0
        .clone();

    inject_response(
        &transport_tx,
        response_for_invite(&first_invite, StatusCode::Ringing, "early", None),
        first,
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    inject_response(
        &transport_tx,
        response_for_invite(&first_invite, StatusCode::ServiceUnavailable, "early", None),
        first,
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    assert_eq!(
        requests_sent_as(&transport, Method::Invite).len(),
        1,
        "a provisional response pins the INVITE to its current candidate"
    );
}

#[tokio::test]
async fn retained_invite_plan_does_not_treat_auth_redirect_or_422_as_candidate_failure() {
    use rvoip_sip_core::StatusCode;

    for (case, status, port) in [
        ("auth", StatusCode::Unauthorized, 5101),
        ("redirect", StatusCode::MovedTemporarily, 5102),
        ("session-timer", StatusCode::SessionIntervalTooSmall, 5103),
    ] {
        let (manager, transport, transport_tx) = build_event_manager_with_transport().await;
        let first = SocketAddr::from(([10, 0, 5, 1], port));
        let second = SocketAddr::from(([10, 0, 5, 2], port));
        send_test_initial_invite(
            &manager,
            vec![
                ResolvedTarget::immediate(first, TransportType::Udp),
                ResolvedTarget::immediate(second, TransportType::Udp),
            ],
            &format!("retained-plan-no-candidate-retry-{case}"),
        )
        .await;
        let first_invite = wait_for_request_count(&transport, Method::Invite, 1).await[0]
            .0
            .clone();
        inject_response(
            &transport_tx,
            response_for_invite(&first_invite, status, case, None),
            first,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(75)).await;
        assert_eq!(
            requests_sent_as(&transport, Method::Invite).len(),
            1,
            "{case} must be handled semantically, not as RFC 3263 candidate failover"
        );
    }
}

#[tokio::test]
async fn late_success_from_superseded_invite_is_reacked_and_byeed_once() {
    use rvoip_sip_core::StatusCode;

    let (manager, transport, transport_tx) = build_event_manager_with_transport().await;
    let first: SocketAddr = "10.0.3.1:5060".parse().unwrap();
    let second: SocketAddr = "10.0.3.2:5060".parse().unwrap();
    send_test_initial_invite(
        &manager,
        vec![
            ResolvedTarget::immediate(first, TransportType::Udp),
            ResolvedTarget::immediate(second, TransportType::Udp),
        ],
        "retained-plan-late-fork-cleanup",
    )
    .await;
    let first_invite = wait_for_request_count(&transport, Method::Invite, 1).await[0]
        .0
        .clone();
    inject_response(
        &transport_tx,
        response_for_invite(
            &first_invite,
            StatusCode::ServiceUnavailable,
            "first-503",
            None,
        ),
        first,
    )
    .await;
    wait_for_request_count(&transport, Method::Invite, 2).await;

    let late_success = response_for_invite(
        &first_invite,
        StatusCode::Ok,
        "late-fork",
        Some("sip:bob@10.0.3.1:5060"),
    );
    let acknowledgements_before = requests_sent_as(&transport, Method::Ack).len();
    inject_response(&transport_tx, late_success.clone(), first).await;
    let byes = wait_for_request_count(&transport, Method::Bye, 1).await;
    let acknowledgements =
        wait_for_request_count(&transport, Method::Ack, acknowledgements_before + 1).await;
    assert_eq!(byes[0].1, first);
    assert_eq!(byes[0].0.to().and_then(|to| to.tag()), Some("late-fork"));
    let bye_transaction = client_transaction_key_for_request(&byes[0].0);
    inject_response(
        &transport_tx,
        response_for_request(&byes[0].0, StatusCode::Ok),
        first,
    )
    .await;
    assert!(matches!(
        manager
            .transaction_manager()
            .wait_for_client_transaction_outcome(
                &bye_transaction,
                std::time::Duration::from_secs(1),
            )
            .await
            .expect("wait for late-fork BYE completion"),
        Some(ClientTransactionOutcome::FinalResponse(response))
            if response.status().as_u16() == 200
    ));

    inject_response(&transport_tx, late_success, first).await;
    wait_for_request_count(&transport, Method::Ack, acknowledgements.len() + 1).await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(
        requests_sent_as(&transport, Method::Bye).len(),
        1,
        "duplicate late 2xx must be ACKed again without emitting a second BYE"
    );
    assert_eq!(
        manager
            .transaction_manager()
            .retention_counts()
            .event_subscribers,
        0,
        "late-fork cleanup must use exact completion, not a global event subscription"
    );
}

#[tokio::test]
async fn retransmitted_late_2xx_while_fork_bye_is_pending_is_reacked_without_parallel_bye() {
    use rvoip_sip_core::StatusCode;

    let (manager, transport, transport_tx) = build_event_manager_with_transport().await;
    let first: SocketAddr = "10.0.3.41:5060".parse().unwrap();
    let second: SocketAddr = "10.0.3.42:5060".parse().unwrap();
    send_test_initial_invite(
        &manager,
        vec![
            ResolvedTarget::immediate(first, TransportType::Udp),
            ResolvedTarget::immediate(second, TransportType::Udp),
        ],
        "retained-plan-late-fork-pending-bye",
    )
    .await;
    let first_invite = wait_for_request_count(&transport, Method::Invite, 1).await[0]
        .0
        .clone();
    inject_response(
        &transport_tx,
        response_for_invite(
            &first_invite,
            StatusCode::ServiceUnavailable,
            "first-503",
            None,
        ),
        first,
    )
    .await;
    wait_for_request_count(&transport, Method::Invite, 2).await;

    let late_success = response_for_invite(
        &first_invite,
        StatusCode::Ok,
        "late-fork-pending-bye",
        Some("sip:bob@10.0.3.41:5060"),
    );
    let acknowledgements_before = requests_sent_as(&transport, Method::Ack).len();
    inject_response(&transport_tx, late_success.clone(), first).await;
    let byes = wait_for_unique_request_count(&transport, Method::Bye, 1).await;
    wait_for_request_count(&transport, Method::Ack, acknowledgements_before + 1).await;

    // The fork BYE has crossed the wire but has no final response. A
    // retransmitted INVITE 2xx still needs an immediate ACK; it must not race
    // a second BYE against the pending authoritative transaction.
    inject_response(&transport_tx, late_success.clone(), first).await;
    wait_for_request_count(&transport, Method::Ack, acknowledgements_before + 2).await;
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    assert_eq!(unique_requests_sent_as(&transport, Method::Bye).len(), 1);

    inject_response(
        &transport_tx,
        response_for_request(&byes[0].0, StatusCode::Ok),
        first,
    )
    .await;
    let bye_transaction = client_transaction_key_for_request(&byes[0].0);
    assert!(matches!(
        manager
            .transaction_manager()
            .wait_for_client_transaction_outcome(
                &bye_transaction,
                std::time::Duration::from_secs(1),
            )
            .await
            .expect("wait for pending late-fork BYE"),
        Some(ClientTransactionOutcome::FinalResponse(response))
            if response.status().as_u16() == 200
    ));
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;

    inject_response(&transport_tx, late_success, first).await;
    wait_for_request_count(&transport, Method::Ack, acknowledgements_before + 3).await;
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    assert_eq!(
        unique_requests_sent_as(&transport, Method::Bye).len(),
        1,
        "confirmed cleanup must suppress any later BYE while preserving re-ACK"
    );
    assert_eq!(
        manager
            .transaction_manager()
            .retention_counts()
            .event_subscribers,
        0
    );
}

#[tokio::test]
async fn rejected_late_fork_bye_remains_retryable_until_a_bye_2xx() {
    use rvoip_sip_core::StatusCode;

    let (manager, transport, transport_tx) = build_event_manager_with_transport().await;
    let first: SocketAddr = "10.0.3.51:5060".parse().unwrap();
    let second: SocketAddr = "10.0.3.52:5060".parse().unwrap();
    send_test_initial_invite(
        &manager,
        vec![
            ResolvedTarget::immediate(first, TransportType::Udp),
            ResolvedTarget::immediate(second, TransportType::Udp),
        ],
        "retained-plan-late-fork-bye-rejection",
    )
    .await;
    let first_invite = wait_for_request_count(&transport, Method::Invite, 1).await[0]
        .0
        .clone();
    inject_response(
        &transport_tx,
        response_for_invite(
            &first_invite,
            StatusCode::ServiceUnavailable,
            "first-503",
            None,
        ),
        first,
    )
    .await;
    wait_for_request_count(&transport, Method::Invite, 2).await;

    let late_success = response_for_invite(
        &first_invite,
        StatusCode::Ok,
        "late-fork-bye-rejection",
        Some("sip:bob@10.0.3.51:5060"),
    );
    let acknowledgements_before = requests_sent_as(&transport, Method::Ack).len();
    inject_response(&transport_tx, late_success.clone(), first).await;
    let first_byes = wait_for_unique_request_count(&transport, Method::Bye, 1).await;
    let first_bye_transaction = client_transaction_key_for_request(&first_byes[0].0);
    inject_response(
        &transport_tx,
        response_for_request(&first_byes[0].0, StatusCode::ServerInternalError),
        first,
    )
    .await;
    assert!(matches!(
        manager
            .transaction_manager()
            .wait_for_client_transaction_outcome(
                &first_bye_transaction,
                std::time::Duration::from_secs(1),
            )
            .await
            .expect("wait for rejected late-fork BYE"),
        Some(ClientTransactionOutcome::FinalResponse(response))
            if response.status().as_u16() == 500
    ));
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;

    inject_response(&transport_tx, late_success.clone(), first).await;
    wait_for_request_count(&transport, Method::Ack, acknowledgements_before + 2).await;
    let retry_byes = wait_for_unique_request_count(&transport, Method::Bye, 2).await;
    assert_ne!(
        retry_byes[0]
            .0
            .first_via()
            .and_then(|via| via.branch().map(str::to_owned)),
        retry_byes[1]
            .0
            .first_via()
            .and_then(|via| via.branch().map(str::to_owned)),
        "a rejected cleanup must retry with a new BYE transaction"
    );
    inject_response(
        &transport_tx,
        response_for_request(&retry_byes[1].0, StatusCode::Ok),
        first,
    )
    .await;
    let retry_transaction = client_transaction_key_for_request(&retry_byes[1].0);
    assert!(matches!(
        manager
            .transaction_manager()
            .wait_for_client_transaction_outcome(
                &retry_transaction,
                std::time::Duration::from_secs(1),
            )
            .await
            .expect("wait for successful retry BYE"),
        Some(ClientTransactionOutcome::FinalResponse(response))
            if response.status().as_u16() == 200
    ));
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;

    inject_response(&transport_tx, late_success, first).await;
    wait_for_request_count(&transport, Method::Ack, acknowledgements_before + 3).await;
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    assert_eq!(unique_requests_sent_as(&transport, Method::Bye).len(), 2);
    assert_eq!(
        manager
            .transaction_manager()
            .retention_counts()
            .event_subscribers,
        0
    );
}

#[tokio::test]
async fn timed_out_late_fork_bye_remains_retryable() {
    use rvoip_sip_core::StatusCode;

    let timer_settings = TimerSettings {
        t1: std::time::Duration::from_millis(25),
        t2: std::time::Duration::from_millis(50),
        t4: std::time::Duration::from_millis(25),
        transaction_timeout: std::time::Duration::from_millis(250),
        wait_time_k: std::time::Duration::from_millis(25),
        ..TimerSettings::default()
    };
    let (manager, transport, transport_tx) =
        build_event_manager_with_transport_and_timers(Some(timer_settings)).await;
    let first: SocketAddr = "10.0.3.61:5060".parse().unwrap();
    let second: SocketAddr = "10.0.3.62:5060".parse().unwrap();
    send_test_initial_invite(
        &manager,
        vec![
            ResolvedTarget::immediate(first, TransportType::Udp),
            ResolvedTarget::immediate(second, TransportType::Udp),
        ],
        "retained-plan-late-fork-bye-timeout",
    )
    .await;
    let first_invite = wait_for_request_count(&transport, Method::Invite, 1).await[0]
        .0
        .clone();
    inject_response(
        &transport_tx,
        response_for_invite(
            &first_invite,
            StatusCode::ServiceUnavailable,
            "first-503",
            None,
        ),
        first,
    )
    .await;
    wait_for_request_count(&transport, Method::Invite, 2).await;

    let late_success = response_for_invite(
        &first_invite,
        StatusCode::Ok,
        "late-fork-bye-timeout",
        Some("sip:bob@10.0.3.61:5060"),
    );
    let acknowledgements_before = requests_sent_as(&transport, Method::Ack).len();
    inject_response(&transport_tx, late_success.clone(), first).await;
    let first_byes = wait_for_unique_request_count(&transport, Method::Bye, 1).await;
    let first_bye_transaction = client_transaction_key_for_request(&first_byes[0].0);

    // Exercise the response-versus-timeout race: the duplicate 2xx arrives
    // while the first BYE is pending. It is ACKed without creating BYE #2
    // until the exact timeout has released the cleanup claim.
    inject_response(&transport_tx, late_success.clone(), first).await;
    wait_for_request_count(&transport, Method::Ack, acknowledgements_before + 2).await;
    assert_eq!(unique_requests_sent_as(&transport, Method::Bye).len(), 1);
    assert!(matches!(
        manager
            .transaction_manager()
            .wait_for_client_transaction_outcome(
                &first_bye_transaction,
                std::time::Duration::from_secs(1),
            )
            .await
            .expect("wait for late-fork BYE timeout"),
        Some(ClientTransactionOutcome::Failure(
            ClientTransactionFailure::Timeout
        ))
    ));
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;

    inject_response(&transport_tx, late_success, first).await;
    wait_for_request_count(&transport, Method::Ack, acknowledgements_before + 3).await;
    let retry_byes = wait_for_unique_request_count(&transport, Method::Bye, 2).await;
    inject_response(
        &transport_tx,
        response_for_request(&retry_byes[1].0, StatusCode::Ok),
        first,
    )
    .await;
    let retry_transaction = client_transaction_key_for_request(&retry_byes[1].0);
    assert!(matches!(
        manager
            .transaction_manager()
            .wait_for_client_transaction_outcome(
                &retry_transaction,
                std::time::Duration::from_secs(1),
            )
            .await
            .expect("wait for timeout retry BYE"),
        Some(ClientTransactionOutcome::FinalResponse(response))
            if response.status().as_u16() == 200
    ));
    assert_eq!(
        manager
            .transaction_manager()
            .retention_counts()
            .event_subscribers,
        0
    );
}

#[tokio::test]
async fn late_fork_bye_resolves_a_contact_with_a_different_dns_authority() {
    use rvoip_sip_core::StatusCode;

    let (manager, transport, transport_tx) = build_event_manager_with_transport().await;
    let first: SocketAddr = "10.0.3.31:5060".parse().unwrap();
    let second: SocketAddr = "10.0.3.32:5060".parse().unwrap();
    let contact_target: SocketAddr = "10.0.3.33:5090".parse().unwrap();
    manager.set_resolver(Some(Arc::new(MultiCandResolver::with(vec![
        ResolvedTarget::immediate(contact_target, TransportType::Udp),
    ]))));
    send_test_initial_invite(
        &manager,
        vec![
            ResolvedTarget::immediate(first, TransportType::Udp),
            ResolvedTarget::immediate(second, TransportType::Udp),
        ],
        "retained-plan-late-fork-domain-contact",
    )
    .await;
    let first_invite = wait_for_request_count(&transport, Method::Invite, 1).await[0]
        .0
        .clone();
    inject_response(
        &transport_tx,
        response_for_invite(
            &first_invite,
            StatusCode::ServiceUnavailable,
            "first-503",
            None,
        ),
        first,
    )
    .await;
    wait_for_request_count(&transport, Method::Invite, 2).await;

    inject_response(
        &transport_tx,
        response_for_invite(
            &first_invite,
            StatusCode::Ok,
            "late-domain-fork",
            Some("sip:bob@late-fork.example.com:5090"),
        ),
        first,
    )
    .await;
    let byes = wait_for_request_count(&transport, Method::Bye, 1).await;

    assert_eq!(byes[0].1, contact_target);
    assert_eq!(
        byes[0].0.uri().to_string(),
        "sip:bob@late-fork.example.com:5090"
    );
    inject_response(
        &transport_tx,
        response_for_request(&byes[0].0, StatusCode::Ok),
        contact_target,
    )
    .await;
}

#[tokio::test]
async fn late_success_with_selected_dialog_tag_is_reacked_without_bye() {
    use rvoip_sip_core::StatusCode;

    let (manager, transport, transport_tx) = build_event_manager_with_transport().await;
    let first: SocketAddr = "10.0.3.21:5060".parse().unwrap();
    let second: SocketAddr = "10.0.3.22:5060".parse().unwrap();
    send_test_initial_invite(
        &manager,
        vec![
            ResolvedTarget::immediate(first, TransportType::Udp),
            ResolvedTarget::immediate(second, TransportType::Udp),
        ],
        "retained-plan-late-selected-dialog",
    )
    .await;
    let first_invite = wait_for_request_count(&transport, Method::Invite, 1).await[0]
        .0
        .clone();
    inject_response(
        &transport_tx,
        response_for_invite(
            &first_invite,
            StatusCode::ServiceUnavailable,
            "first-503",
            None,
        ),
        first,
    )
    .await;
    let invites = wait_for_request_count(&transport, Method::Invite, 2).await;
    let selected_success = response_for_invite(
        &invites[1].0,
        StatusCode::Ok,
        "same-selected-dialog",
        Some("sip:bob@10.0.3.22:5060"),
    );
    inject_response(&transport_tx, selected_success, second).await;
    let selected_ack_count = wait_for_request_count(&transport, Method::Ack, 1)
        .await
        .len();

    let late_same_dialog = response_for_invite(
        &first_invite,
        StatusCode::Ok,
        "same-selected-dialog",
        Some("sip:bob@10.0.3.21:5060"),
    );
    inject_response(&transport_tx, late_same_dialog, first).await;
    wait_for_request_count(&transport, Method::Ack, selected_ack_count + 1).await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    assert!(
        requests_sent_as(&transport, Method::Bye).is_empty(),
        "the remote To tag identifies the selected dialog even when its 2xx arrives on an older transaction"
    );
}

#[tokio::test]
async fn failed_late_fork_ack_blocks_bye_until_retransmitted_success_retries_cleanup() {
    use rvoip_sip_core::StatusCode;

    let (manager, transport, transport_tx) = build_event_manager_with_transport().await;
    let first: SocketAddr = "10.0.3.11:5060".parse().unwrap();
    let second: SocketAddr = "10.0.3.12:5060".parse().unwrap();
    send_test_initial_invite(
        &manager,
        vec![
            ResolvedTarget::immediate(first, TransportType::Udp),
            ResolvedTarget::immediate(second, TransportType::Udp),
        ],
        "retained-plan-late-fork-ack-retry",
    )
    .await;
    let first_invite = wait_for_request_count(&transport, Method::Invite, 1).await[0]
        .0
        .clone();
    inject_response(
        &transport_tx,
        response_for_invite(
            &first_invite,
            StatusCode::ServiceUnavailable,
            "first-503",
            None,
        ),
        first,
    )
    .await;
    wait_for_request_count(&transport, Method::Invite, 2).await;
    transport.program_method_sequence(
        first,
        Method::Ack,
        [
            Outcome::RecoverableFail("first late ACK fails".into()),
            Outcome::Ok,
        ],
    );

    let late_success = response_for_invite(
        &first_invite,
        StatusCode::Ok,
        "late-fork-ack-retry",
        Some("sip:bob@10.0.3.11:5060"),
    );
    inject_response(&transport_tx, late_success.clone(), first).await;
    tokio::time::sleep(std::time::Duration::from_millis(75)).await;
    assert!(
        requests_sent_as(&transport, Method::Bye).is_empty(),
        "a BYE cannot precede the required late-fork ACK"
    );

    inject_response(&transport_tx, late_success, first).await;
    let byes = wait_for_request_count(&transport, Method::Bye, 1).await;
    assert_eq!(byes[0].1, first);
    assert_eq!(
        byes[0].0.to().and_then(|to| to.tag()),
        Some("late-fork-ack-retry")
    );
    inject_response(
        &transport_tx,
        response_for_request(&byes[0].0, StatusCode::Ok),
        first,
    )
    .await;
}

#[tokio::test]
async fn cancel_targets_the_exact_current_failover_attempt() {
    use rvoip_sip_core::StatusCode;

    let (manager, transport, transport_tx) = build_event_manager_with_transport().await;
    let first: SocketAddr = "10.0.4.1:5060".parse().unwrap();
    let second: SocketAddr = "10.0.4.2:5060".parse().unwrap();
    let (_, original_transaction) = send_test_initial_invite(
        &manager,
        vec![
            ResolvedTarget::immediate(first, TransportType::Udp),
            ResolvedTarget::immediate(second, TransportType::Udp),
        ],
        "retained-plan-cancel-current",
    )
    .await;
    let first_invite = wait_for_request_count(&transport, Method::Invite, 1).await[0]
        .0
        .clone();
    inject_response(
        &transport_tx,
        response_for_invite(
            &first_invite,
            StatusCode::ServiceUnavailable,
            "first-503",
            None,
        ),
        first,
    )
    .await;
    let invites = wait_for_request_count(&transport, Method::Invite, 2).await;
    let current_branch = invites[1]
        .0
        .first_via()
        .and_then(|via| via.branch().map(str::to_owned));

    manager
        .cancel_invite_transaction_with_dialog(&original_transaction)
        .await
        .expect("cancel current failover attempt");
    let cancels = wait_for_request_count(&transport, Method::Cancel, 1).await;

    assert_eq!(cancels[0].1, second);
    assert_eq!(
        cancels[0]
            .0
            .first_via()
            .and_then(|via| via.branch().map(str::to_owned)),
        current_branch,
        "CANCEL must use the branch of the serialized current attempt"
    );
    assert_eq!(
        cancels[0].0.cseq().map(|cseq| cseq.seq),
        invites[1].0.cseq().map(|cseq| cseq.seq)
    );
}

#[tokio::test]
async fn transaction_candidate_failover_waits_for_slow_initial_send_failure() {
    use rvoip_sip_core::builder::SimpleRequestBuilder;

    let (manager, transport) = build_manager_with_transport().await;
    let first: SocketAddr = "10.0.0.1:5060".parse().unwrap();
    let second: SocketAddr = "10.0.0.2:5060".parse().unwrap();
    let delay = std::time::Duration::from_millis(75);
    transport.program(
        first,
        Outcome::DelayedRecoverableFail(delay, "Transaction terminated after timeout".into()),
    );
    let request = SimpleRequestBuilder::new(Method::Options, "sip:service@example.com")
        .unwrap()
        .from("alice", "sip:alice@example.com", Some("tag"))
        .to("service", "sip:service@example.com", None)
        .contact("sip:alice@127.0.0.1:5060", None)
        .call_id("transaction-candidate-slow-fail")
        .cseq(1)
        .max_forwards(70)
        .via("127.0.0.1:5060", "UDP", Some("z9hG4bK-original-candidate"))
        .build();
    let started = tokio::time::Instant::now();

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        manager.send_request_with_candidate_wire_plan(
            request,
            vec![
                ResolvedTarget::immediate(first, TransportType::Tcp),
                ResolvedTarget::immediate(second, TransportType::Udp),
            ],
            None,
            CandidateWirePlan {
                regenerate_stack_default_contact: true,
            },
        ),
    )
    .await;
    let elapsed = started.elapsed();
    let sends = transport.sends();
    let (_transaction, selected) = result
        .unwrap_or_else(|_| panic!("candidate failover hung; sends={sends:?}; elapsed={elapsed:?}"))
        .unwrap_or_else(|error| {
        panic!(
            "second candidate should succeed after actual first send fails: {error:?}; sends={sends:?}; elapsed={elapsed:?}"
        )
    });

    assert_eq!(selected, second);
    assert!(elapsed >= delay);
    assert_eq!(sends, vec![first, second]);
    let branches = transport.branches();
    assert_eq!(branches.len(), 2);
    assert!(branches.iter().all(Option::is_some));
    assert!(branches
        .iter()
        .all(|branch| branch.as_deref() != Some("z9hG4bK-original-candidate")));
    assert_ne!(
        branches[0], branches[1],
        "each failover candidate must use a distinct client transaction branch"
    );
    assert_eq!(
        transport.via_transports(),
        vec![Some("TCP".into()), Some("UDP".into())],
        "Via transport must match the resolver-selected route on every attempt"
    );
    assert_eq!(
        transport.via_sent_by(),
        vec![Some("127.0.0.1:5060".into()), Some("127.0.0.1:5060".into())]
    );
    let contacts = transport.contacts();
    assert!(contacts[0]
        .as_deref()
        .is_some_and(|contact| contact.contains(";transport=tcp")));
    assert!(contacts[1]
        .as_deref()
        .is_some_and(|contact| !contact.contains(";transport=")));
}

#[tokio::test]
async fn candidate_failover_preserves_application_contact() {
    use rvoip_sip_core::builder::SimpleRequestBuilder;

    let (manager, transport) = build_manager_with_transport().await;
    let destination: SocketAddr = "10.0.0.9:5060".parse().unwrap();
    let request = SimpleRequestBuilder::new(Method::Options, "sip:service@example.com")
        .unwrap()
        .from("alice", "sip:alice@example.com", Some("tag"))
        .to("service", "sip:service@example.com", None)
        .contact("sip:explicit@203.0.113.9:5099;transport=tcp", None)
        .call_id("candidate-explicit-contact")
        .cseq(1)
        .max_forwards(70)
        .via("127.0.0.1:5060", "UDP", Some("z9hG4bK-contact"))
        .build();

    manager
        .send_request_with_candidate_failover(
            request,
            vec![ResolvedTarget::immediate(destination, TransportType::Tcp)],
            None,
        )
        .await
        .expect("candidate send");

    assert!(transport.contacts()[0]
        .as_deref()
        .is_some_and(|contact| contact.contains("explicit@203.0.113.9:5099;transport=tcp")));
}

#[tokio::test]
async fn transaction_candidate_failover_fails_closed_without_exact_candidates() {
    use rvoip_sip_core::builder::SimpleRequestBuilder;

    let (manager, transport) = build_manager_with_transport().await;
    let request = SimpleRequestBuilder::new(Method::Options, "sip:target@example.com")
        .unwrap()
        .from("alice", "sip:alice@example.com", Some("tag"))
        .to("target", "sip:target@example.com", None)
        .call_id("empty-exact-next-hop")
        .cseq(1)
        .build();

    let error = manager
        .send_request_with_candidate_failover(request, Vec::new(), None)
        .await
        .expect_err("an unresolved exact next hop must not use a target fallback");
    assert_eq!(error.diagnostic_class(), "routing");
    assert!(transport.sends().is_empty());
}

#[tokio::test]
async fn resolve_uri_to_candidates_returns_full_list() {
    let manager = build_manager().await;
    let mock = Arc::new(MultiCandResolver::with(vec![
        target("10.0.0.1:5061", TransportType::Tls),
        target("10.0.0.2:5060", TransportType::Tcp),
        target("10.0.0.3:5060", TransportType::Udp),
    ]));
    manager.set_resolver(Some(mock));

    let uri = Uri::from_str("sip:bob@example.com").unwrap();
    let candidates = manager.resolve_uri_to_candidates(&uri).await;
    assert_eq!(candidates.len(), 3);
    assert_eq!(candidates[0].addr.to_string(), "10.0.0.1:5061");
    assert_eq!(candidates[1].addr.to_string(), "10.0.0.2:5060");
    assert_eq!(candidates[2].addr.to_string(), "10.0.0.3:5060");
}

#[tokio::test]
async fn resolve_uri_to_candidates_short_circuits_ip_literals() {
    let manager = build_manager().await;
    // No resolver installed → falls back to free function which
    // short-circuits IP literals.
    let uri = Uri::from_str("sip:bob@127.0.0.1:5060").unwrap();
    let candidates = manager.resolve_uri_to_candidates(&uri).await;
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].addr.to_string(), "127.0.0.1:5060");
    assert_eq!(candidates[0].transport, TransportType::Udp);
}

#[tokio::test]
async fn resolve_uri_to_candidates_returns_empty_on_resolver_error() {
    let manager = build_manager().await;
    let mock = Arc::new(MultiCandResolver::with_err(ResolverError::NoCandidates));
    manager.set_resolver(Some(mock));

    let uri = Uri::from_str("sip:bob@example.com").unwrap();
    let candidates = manager.resolve_uri_to_candidates(&uri).await;
    assert!(candidates.is_empty());
}
