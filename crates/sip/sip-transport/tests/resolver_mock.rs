//! Trait-surface tests for `Resolver`.
//!
//! These tests verify that the `Resolver` trait can be implemented by
//! application code, used as a trait object, and that the candidate
//! ordering / error propagation contract holds. Algorithm-level
//! coverage of [`HickoryResolver`] lives in `resolver_hickory_e2e.rs`.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rvoip_sip_core::Uri;
use rvoip_sip_transport::resolver::{ResolvedTarget, Resolver, ResolverError};
use rvoip_sip_transport::transport::TransportType;

/// In-memory mock that returns canned responses keyed by stringified
/// URI and records every call for assertions.
#[derive(Default)]
struct MockResolver {
    canned: HashMap<String, Result<Vec<ResolvedTarget>, ResolverError>>,
    calls: Mutex<Vec<String>>,
}

impl MockResolver {
    fn with(mut self, uri: &str, result: Result<Vec<ResolvedTarget>, ResolverError>) -> Self {
        self.canned.insert(uri.to_string(), result);
        self
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl Resolver for MockResolver {
    async fn resolve(&self, uri: &Uri) -> Result<Vec<ResolvedTarget>, ResolverError> {
        let key = uri.to_string();
        self.calls.lock().unwrap().push(key.clone());
        match self.canned.get(&key) {
            Some(Ok(v)) => Ok(v.clone()),
            Some(Err(e)) => Err(clone_error(e)),
            None => Err(ResolverError::NoCandidates),
        }
    }
}

fn clone_error(e: &ResolverError) -> ResolverError {
    match e {
        ResolverError::Dns(msg) => ResolverError::Dns(msg.clone()),
        ResolverError::Forbidden(reason) => ResolverError::Forbidden(reason),
        ResolverError::NoCandidates => ResolverError::NoCandidates,
        ResolverError::InvalidHost(msg) => ResolverError::InvalidHost(msg.clone()),
    }
}

fn addr(s: &str) -> SocketAddr {
    s.parse().expect("valid socket addr")
}

#[tokio::test]
async fn trait_object_dispatch_returns_canned_candidates() {
    let resolver: Arc<dyn Resolver> = Arc::new(MockResolver::default().with(
        "sip:bob@example.com",
        Ok(vec![
            ResolvedTarget::immediate(addr("10.0.0.1:5060"), TransportType::Udp),
            ResolvedTarget::immediate(addr("10.0.0.2:5060"), TransportType::Udp),
        ]),
    ));

    let uri = Uri::from_str("sip:bob@example.com").unwrap();
    let candidates = resolver.resolve(&uri).await.unwrap();
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0].addr.to_string(), "10.0.0.1:5060");
    assert_eq!(candidates[1].addr.to_string(), "10.0.0.2:5060");
}

#[tokio::test]
async fn candidates_preserve_caller_ordering() {
    // Caller iterates in the returned order on transport failure — the
    // mock returns three priority groups; resolver must NOT reorder.
    let resolver = MockResolver::default().with(
        "sip:bob@example.com",
        Ok(vec![
            ResolvedTarget::immediate(addr("10.0.0.10:5061"), TransportType::Tls),
            ResolvedTarget::immediate(addr("10.0.0.20:5060"), TransportType::Tcp),
            ResolvedTarget::immediate(addr("10.0.0.30:5060"), TransportType::Udp),
        ]),
    );

    let uri = Uri::from_str("sip:bob@example.com").unwrap();
    let out = resolver.resolve(&uri).await.unwrap();
    let order: Vec<TransportType> = out.iter().map(|t| t.transport).collect();
    assert_eq!(
        order,
        vec![TransportType::Tls, TransportType::Tcp, TransportType::Udp]
    );
}

#[tokio::test]
async fn dns_error_propagates() {
    let resolver = MockResolver::default().with(
        "sip:bob@example.com",
        Err(ResolverError::Dns("upstream timeout".to_string())),
    );
    let uri = Uri::from_str("sip:bob@example.com").unwrap();
    let err = resolver.resolve(&uri).await.unwrap_err();
    assert!(matches!(err, ResolverError::Dns(_)));
    assert!(err.to_string().contains("upstream timeout"));
}

#[tokio::test]
async fn forbidden_error_propagates() {
    let resolver = MockResolver::default().with(
        "sips:bob@example.com;transport=udp",
        Err(ResolverError::Forbidden(
            "sips: scheme cannot use transport=udp",
        )),
    );
    let uri = Uri::from_str("sips:bob@example.com;transport=udp").unwrap();
    let err = resolver.resolve(&uri).await.unwrap_err();
    assert!(matches!(err, ResolverError::Forbidden(_)));
}

#[tokio::test]
async fn no_candidates_error_propagates() {
    // An unmapped URI returns NoCandidates from our mock — verifies the
    // hard-failure path callers must handle.
    let resolver = MockResolver::default();
    let uri = Uri::from_str("sip:bob@example.com").unwrap();
    let err = resolver.resolve(&uri).await.unwrap_err();
    assert!(matches!(err, ResolverError::NoCandidates));
}

#[tokio::test]
async fn calls_are_recorded() {
    let resolver = MockResolver::default()
        .with(
            "sip:a@example.com",
            Ok(vec![ResolvedTarget::immediate(
                addr("10.0.0.1:5060"),
                TransportType::Udp,
            )]),
        )
        .with(
            "sip:b@example.com",
            Ok(vec![ResolvedTarget::immediate(
                addr("10.0.0.2:5060"),
                TransportType::Udp,
            )]),
        );

    let _ = resolver
        .resolve(&Uri::from_str("sip:a@example.com").unwrap())
        .await
        .unwrap();
    let _ = resolver
        .resolve(&Uri::from_str("sip:b@example.com").unwrap())
        .await
        .unwrap();
    let calls = resolver.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0], "sip:a@example.com");
    assert_eq!(calls[1], "sip:b@example.com");
}

#[tokio::test]
async fn ttl_expires_optional_is_carried() {
    let now = std::time::Instant::now();
    let target = ResolvedTarget {
        addr: addr("10.0.0.1:5060"),
        transport: TransportType::Tls,
        expires: Some(now + std::time::Duration::from_secs(300)),
    };
    let resolver = MockResolver::default().with("sip:bob@example.com", Ok(vec![target.clone()]));
    let uri = Uri::from_str("sip:bob@example.com").unwrap();
    let candidates = resolver.resolve(&uri).await.unwrap();
    assert_eq!(candidates[0].expires, target.expires);
}

#[tokio::test]
async fn from_resolver_error_into_transport_error_maps_dns() {
    let err: rvoip_sip_transport::error::Error = ResolverError::Dns("timeout".to_string()).into();
    assert!(matches!(
        err,
        rvoip_sip_transport::error::Error::DnsResolutionFailed(_)
    ));
}

#[tokio::test]
async fn from_resolver_error_into_transport_error_maps_forbidden() {
    let err: rvoip_sip_transport::error::Error = ResolverError::Forbidden("sips:+udp").into();
    assert!(matches!(
        err,
        rvoip_sip_transport::error::Error::UnsupportedTransport(_)
    ));
}

#[tokio::test]
async fn from_resolver_error_into_transport_error_maps_no_candidates() {
    let err: rvoip_sip_transport::error::Error = ResolverError::NoCandidates.into();
    assert!(matches!(
        err,
        rvoip_sip_transport::error::Error::DnsResolutionFailed(_)
    ));
}
