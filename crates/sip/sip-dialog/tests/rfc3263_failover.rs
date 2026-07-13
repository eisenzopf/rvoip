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

use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rvoip_sip_core::types::headers::HeaderAccess;
use rvoip_sip_core::{Method, Request, Uri};
use rvoip_sip_dialog::manager::transaction_integration::CandidateWirePlan;
use rvoip_sip_dialog::transaction::transport::multiplexed::MultiplexedTransport;
use rvoip_sip_dialog::transaction::TransactionManager;
use rvoip_sip_dialog::DialogManager;
use rvoip_sip_transport::error::Error as TransportError;
use rvoip_sip_transport::resolver::{ResolvedTarget, Resolver, ResolverError};
use rvoip_sip_transport::transport::TransportType;
use rvoip_sip_transport::Transport;
use tokio::sync::mpsc;

/// A mock transport whose `send_message` outcome can be programmed
/// per-destination. The default for any address not in `outcomes` is
/// `Ok(())`.
#[derive(Debug, Clone)]
struct ProgrammableTransport {
    local_addr: SocketAddr,
    outcomes: Arc<Mutex<HashMap<SocketAddr, Outcome>>>,
    sends: Arc<Mutex<Vec<SocketAddr>>>,
    branches: Arc<Mutex<Vec<Option<String>>>>,
    via_transports: Arc<Mutex<Vec<Option<String>>>>,
    via_sent_by: Arc<Mutex<Vec<Option<String>>>>,
    contacts: Arc<Mutex<Vec<Option<String>>>>,
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
            sends: Arc::new(Mutex::new(Vec::new())),
            branches: Arc::new(Mutex::new(Vec::new())),
            via_transports: Arc::new(Mutex::new(Vec::new())),
            via_sent_by: Arc::new(Mutex::new(Vec::new())),
            contacts: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn program(&self, addr: SocketAddr, outcome: Outcome) {
        self.outcomes.lock().unwrap().insert(addr, outcome);
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
}

#[async_trait]
impl Transport for ProgrammableTransport {
    async fn send_message(
        &self,
        message: rvoip_sip_core::Message,
        destination: SocketAddr,
    ) -> Result<(), TransportError> {
        self.sends.lock().unwrap().push(destination);
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
        let outcome = self.outcomes.lock().unwrap().get(&destination).cloned();
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
