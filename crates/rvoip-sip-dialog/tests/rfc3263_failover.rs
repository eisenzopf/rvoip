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
use rvoip_sip_core::{Method, Request, Uri};
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
}

#[derive(Debug, Clone)]
enum Outcome {
    Ok,
    RecoverableFail(String),
    FatalFail(String),
}

impl ProgrammableTransport {
    fn new(local_addr: SocketAddr) -> Self {
        Self {
            local_addr,
            outcomes: Arc::new(Mutex::new(HashMap::new())),
            sends: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn program(&self, addr: SocketAddr, outcome: Outcome) {
        self.outcomes.lock().unwrap().insert(addr, outcome);
    }

    fn sends(&self) -> Vec<SocketAddr> {
        self.sends.lock().unwrap().clone()
    }
}

#[async_trait]
impl Transport for ProgrammableTransport {
    async fn send_message(
        &self,
        _message: rvoip_sip_core::Message,
        destination: SocketAddr,
    ) -> Result<(), TransportError> {
        self.sends.lock().unwrap().push(destination);
        match self.outcomes.lock().unwrap().get(&destination).cloned() {
            None | Some(Outcome::Ok) => Ok(()),
            Some(Outcome::RecoverableFail(msg)) => Err(TransportError::ConnectFailed(
                destination,
                std::io::Error::new(std::io::ErrorKind::ConnectionRefused, msg),
            )),
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
    let (transaction_manager, _event_rx) =
        TransactionManager::new(transport.clone(), transport_rx, Some(16))
            .await
            .expect("TransactionManager::new");
    Arc::new(
        DialogManager::new(Arc::new(transaction_manager), local_addr)
            .await
            .expect("DialogManager::new"),
    )
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
