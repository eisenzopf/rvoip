//! Acceptance tests for Phase 5 — RFC 3263 NAPTR/SRV/A resolution.
//!
//! Covers the `rvoip-sip-dialog` slice of the resolver wiring:
//!
//! 1. A `Resolver` can be installed on a `DialogManager` via
//!    `set_resolver` and read back via `resolver()`.
//! 2. `DialogManager::resolve_uri_to_socketaddr` consults the
//!    configured resolver when one is installed.
//! 3. With no resolver configured, IP-literal URIs still resolve via
//!    the process-wide fallback path (so the function works in
//!    sandboxed environments without `/etc/resolv.conf`).
//!
//! Algorithm-level coverage of the NAPTR ladder lives in
//! `crates/rvoip-sip-transport/tests/resolver_hickory_e2e.rs`. Full
//! INVITE-flow wire tests depend on the broader transaction harness
//! and run as part of the PBX matrix.

use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rvoip_sip_core::Uri;
use rvoip_sip_dialog::transaction::TransactionManager;
use rvoip_sip_dialog::DialogManager;
use rvoip_sip_transport::resolver::{ResolvedTarget, Resolver, ResolverError};
use rvoip_sip_transport::transport::TransportType;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
struct MockTransport {
    local_addr: SocketAddr,
}

#[async_trait]
impl rvoip_sip_transport::Transport for MockTransport {
    async fn send_message(
        &self,
        _message: rvoip_sip_core::Message,
        _destination: SocketAddr,
    ) -> Result<(), rvoip_sip_transport::Error> {
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

#[derive(Default)]
struct MockResolver {
    calls: Mutex<Vec<String>>,
    response: Mutex<Vec<ResolvedTarget>>,
}

impl MockResolver {
    fn with_response(self, candidates: Vec<ResolvedTarget>) -> Self {
        *self.response.lock().unwrap() = candidates;
        self
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl Resolver for MockResolver {
    async fn resolve(&self, uri: &Uri) -> Result<Vec<ResolvedTarget>, ResolverError> {
        self.calls.lock().unwrap().push(uri.to_string());
        Ok(self.response.lock().unwrap().clone())
    }
}

async fn build_manager() -> Arc<DialogManager> {
    let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
    let transport = Arc::new(MockTransport { local_addr });
    let (_tx, transport_rx) = mpsc::channel(8);
    let (transaction_manager, _event_rx) =
        TransactionManager::new(transport.clone(), transport_rx, Some(16))
            .await
            .expect("TransactionManager::new");
    let transaction_manager = Arc::new(transaction_manager);
    Arc::new(
        DialogManager::new(transaction_manager, local_addr)
            .await
            .expect("DialogManager::new"),
    )
}

fn target(addr: &str, transport: TransportType) -> ResolvedTarget {
    ResolvedTarget::immediate(addr.parse().unwrap(), transport)
}

#[tokio::test]
async fn set_resolver_round_trips() {
    let manager = build_manager().await;
    assert!(manager.resolver().is_none());

    let mock: Arc<dyn Resolver> = Arc::new(MockResolver::default());
    manager.set_resolver(Some(mock.clone()));
    assert!(manager.resolver().is_some());

    manager.set_resolver(None);
    assert!(manager.resolver().is_none());
}

#[tokio::test]
async fn manager_uses_configured_resolver_for_invite_destination() {
    let manager = build_manager().await;
    let mock = Arc::new(
        MockResolver::default().with_response(vec![target("10.0.0.42:5061", TransportType::Tls)]),
    );
    manager.set_resolver(Some(mock.clone()));

    let uri = Uri::from_str("sip:bob@example.com").unwrap();
    let addr = manager
        .resolve_uri_to_socketaddr(&uri)
        .await
        .expect("resolver returned candidate");
    assert_eq!(addr.to_string(), "10.0.0.42:5061");

    // Mock was consulted exactly once with the expected URI.
    let calls = mock.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0], "sip:bob@example.com");
}

#[tokio::test]
async fn manager_returns_first_candidate_when_resolver_offers_multiple() {
    let manager = build_manager().await;
    let mock = Arc::new(MockResolver::default().with_response(vec![
        target("10.0.0.42:5061", TransportType::Tls),
        target("10.0.0.43:5060", TransportType::Udp),
    ]));
    manager.set_resolver(Some(mock));

    let uri = Uri::from_str("sip:bob@example.com").unwrap();
    let addr = manager.resolve_uri_to_socketaddr(&uri).await.unwrap();
    // First candidate wins on the manager's `next()` projection.
    assert_eq!(addr.to_string(), "10.0.0.42:5061");
}

#[tokio::test]
async fn manager_falls_back_to_default_resolver_when_unset() {
    let manager = build_manager().await;
    assert!(manager.resolver().is_none());

    // IP literal must resolve even without a configured resolver and
    // even when the process-wide hickory init fails (sandboxed CI).
    let uri = Uri::from_str("sip:bob@127.0.0.1:5060").unwrap();
    let addr = manager.resolve_uri_to_socketaddr(&uri).await.unwrap();
    assert_eq!(addr.to_string(), "127.0.0.1:5060");

    // Sips IP literal picks the SIPS default port.
    let uri = Uri::from_str("sips:bob@127.0.0.1").unwrap();
    let addr = manager.resolve_uri_to_socketaddr(&uri).await.unwrap();
    assert_eq!(addr.to_string(), "127.0.0.1:5061");
}

#[tokio::test]
async fn configured_resolver_overrides_default_for_ip_literal_uri_resolution_path() {
    // When a resolver is configured, the manager defers to it even
    // for URIs that the free function would short-circuit. The
    // configured resolver is authoritative for that DialogManager.
    let manager = build_manager().await;
    let mock = Arc::new(
        MockResolver::default().with_response(vec![target("203.0.113.7:5060", TransportType::Udp)]),
    );
    manager.set_resolver(Some(mock.clone()));

    let uri = Uri::from_str("sip:bob@10.10.10.10:5060").unwrap();
    let addr = manager.resolve_uri_to_socketaddr(&uri).await.unwrap();
    // The mock won — manager did NOT fall back to its own short-circuit.
    assert_eq!(addr.to_string(), "203.0.113.7:5060");
    assert_eq!(mock.calls().len(), 1);
}
