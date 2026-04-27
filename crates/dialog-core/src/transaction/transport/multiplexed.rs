//! URI-aware multiplexing wrapper around multiple sip-transport
//! `Transport` implementations.
//!
//! `TransactionManager` is built around a single `Arc<dyn Transport>`. To
//! support per-call transport selection — `sip:bob@host;transport=tcp`
//! must use TCP, `sips:bob@host` must use TLS, while plain
//! `sip:bob@host` defaults to UDP — we install a `MultiplexedTransport`
//! as that single transport. It implements `Transport` itself and
//! dispatches each `send_message` call to the appropriate underlying
//! transport based on the SIP message's Request-URI (RFC 3261 §18.1.1).
//!
//! Responses are dispatched on whichever transport the underlying
//! request arrived on; for outbound responses (server-side replies) we
//! fall through to the default transport since the per-call transport
//! was already chosen on the inbound side.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use rvoip_sip_core::{Message, Request};
use rvoip_sip_transport::transport::TransportType;
use rvoip_sip_transport::{
    error::{Error as TransportError, Result as TransportResult},
    Transport,
};
use tracing::{debug, trace, warn};

/// Picks the SIP transport flavour to use for a given Request-URI per
/// RFC 3261 §26.2 (TLS is mandatory for `sips:` URIs) and §19.1.5
/// (`;transport=` URI parameter).
pub fn select_transport_for_request(request: &Request) -> TransportType {
    select_transport_for_uri(request.uri())
}

/// Select transport from a URI alone.
///
/// Precedence (highest first):
/// 1. URI `;transport=` parameter (`udp` / `tcp` / `tls` / `ws` / `wss`).
/// 2. Scheme: `sips:` → TLS.
/// 3. Default: UDP.
pub fn select_transport_for_uri(uri: &rvoip_sip_core::Uri) -> TransportType {
    use rvoip_sip_core::types::uri::Scheme;

    if let Some(transport_param) = uri.transport() {
        match transport_param.to_ascii_lowercase().as_str() {
            "udp" => return TransportType::Udp,
            "tcp" => return TransportType::Tcp,
            "tls" => return TransportType::Tls,
            "ws" => return TransportType::Ws,
            "wss" => return TransportType::Wss,
            other => {
                warn!(
                    "Unrecognised transport= URI parameter '{}'; falling back to scheme-based selection",
                    other
                );
            }
        }
    }

    match uri.scheme() {
        Scheme::Sips => TransportType::Tls,
        _ => TransportType::Udp,
    }
}

/// `Transport` implementation that owns a registry of underlying
/// transports keyed by `TransportType` and dispatches `send_message`
/// calls to whichever one matches the Request-URI.
#[derive(Debug)]
pub struct MultiplexedTransport {
    /// All registered transports keyed by their flavour. Populated at
    /// construction time from the `TransportManager`'s active
    /// transports.
    transports: HashMap<TransportType, Arc<dyn Transport>>,
    /// Fallback used when the requested flavour isn't registered (no
    /// listener bound for that protocol). Always required — typically
    /// UDP.
    default: Arc<dyn Transport>,
    /// Local address reported via the `Transport::local_addr()` method.
    /// Mirrors the default transport's bind address.
    local_addr: SocketAddr,
}

impl MultiplexedTransport {
    /// Build a multiplexer.
    ///
    /// `default` is what `local_addr()` reports and is used whenever a
    /// requested transport flavour isn't in the registry. `transports`
    /// is the per-flavour registry (typically UDP + TCP, plus TLS when
    /// configured). `default` does not have to appear in `transports` —
    /// the dispatcher will fall back to it explicitly.
    pub fn new(
        default: Arc<dyn Transport>,
        transports: HashMap<TransportType, Arc<dyn Transport>>,
    ) -> TransportResult<Self> {
        let local_addr = default.local_addr().map_err(|e| {
            TransportError::Other(format!(
                "MultiplexedTransport: default transport has no local addr: {}",
                e
            ))
        })?;
        Ok(Self {
            transports,
            default,
            local_addr,
        })
    }

    /// Pick the underlying transport to use for a given outbound
    /// `Message` bound for `destination`.
    ///
    /// - **Requests** route by Request-URI scheme + `;transport=`
    ///   parameter (RFC 3261 §19.1.5, §26.2).
    /// - **Responses** must go back over the same connection-oriented
    ///   transport that received the matching request (RFC 3261 §17.2,
    ///   §18.2.2). The transport layer doesn't have the request
    ///   transport stamped on the response, so we approximate: ask each
    ///   connection-oriented transport whether it currently has a live
    ///   connection to `destination`. The first match wins; otherwise
    ///   fall through to the default (UDP — connectionless, can send
    ///   anywhere).
    fn pick_transport(&self, message: &Message, destination: SocketAddr) -> Arc<dyn Transport> {
        match message {
            Message::Request(request) => {
                let want = select_transport_for_request(request);
                if let Some(transport) = self.transports.get(&want) {
                    trace!(
                        "MultiplexedTransport: routing {} to {} via URI selection",
                        request.method(),
                        want
                    );
                    transport.clone()
                } else {
                    debug!(
                        "MultiplexedTransport: no {} transport registered for {}; falling back to default",
                        want,
                        request.method()
                    );
                    self.default.clone()
                }
            }
            Message::Response(response) => {
                // Probe connection-oriented transports first. We do
                // *not* probe UDP because it always reports false (the
                // default-impl `has_connection_to` is a no-op) and it's
                // the fallback anyway.
                for kind in [
                    TransportType::Tls,
                    TransportType::Tcp,
                    TransportType::Wss,
                    TransportType::Ws,
                ] {
                    if let Some(transport) = self.transports.get(&kind) {
                        if transport.has_connection_to(destination) {
                            trace!(
                                "MultiplexedTransport: routing {} response to {} via {} (existing connection)",
                                response.status_code(),
                                destination,
                                kind
                            );
                            return transport.clone();
                        }
                    }
                }
                trace!(
                    "MultiplexedTransport: no connection-oriented transport has {}; routing response via default",
                    destination
                );
                self.default.clone()
            }
        }
    }
}

#[async_trait]
impl Transport for MultiplexedTransport {
    fn local_addr(&self) -> TransportResult<SocketAddr> {
        Ok(self.local_addr)
    }

    async fn send_message(&self, message: Message, destination: SocketAddr) -> TransportResult<()> {
        let transport = self.pick_transport(&message, destination);
        transport.send_message(message, destination).await
    }

    async fn close(&self) -> TransportResult<()> {
        let mut last_err: Option<TransportError> = None;
        for (kind, transport) in &self.transports {
            if let Err(e) = transport.close().await {
                warn!(
                    "MultiplexedTransport: error closing {} transport: {}",
                    kind, e
                );
                last_err = Some(e);
            }
        }
        if let Err(e) = self.default.close().await {
            warn!(
                "MultiplexedTransport: error closing default transport: {}",
                e
            );
            last_err = Some(e);
        }
        match last_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    fn is_closed(&self) -> bool {
        self.default.is_closed()
    }

    fn supports_udp(&self) -> bool {
        self.transports.contains_key(&TransportType::Udp) || self.default.supports_udp()
    }

    fn supports_tcp(&self) -> bool {
        self.transports.contains_key(&TransportType::Tcp)
    }

    fn supports_tls(&self) -> bool {
        self.transports.contains_key(&TransportType::Tls)
    }

    fn supports_ws(&self) -> bool {
        self.transports.contains_key(&TransportType::Ws)
    }

    fn supports_wss(&self) -> bool {
        self.transports.contains_key(&TransportType::Wss)
    }

    fn default_transport_type(&self) -> TransportType {
        self.default.default_transport_type()
    }

    fn has_connection_to(&self, remote_addr: SocketAddr) -> bool {
        // A multiplexed transport "has a connection" if any of its
        // connection-oriented children does.
        for kind in [
            TransportType::Tls,
            TransportType::Tcp,
            TransportType::Wss,
            TransportType::Ws,
        ] {
            if let Some(transport) = self.transports.get(&kind) {
                if transport.has_connection_to(remote_addr) {
                    return true;
                }
            }
        }
        false
    }

    async fn send_raw(&self, destination: SocketAddr, data: Bytes) -> TransportResult<()> {
        // RFC 5626 §3.5.1 keep-alive: probe connection-oriented
        // transports for an existing flow to `destination` and dispatch
        // bare bytes on the first that matches. UDP is never asked —
        // RFC 5626 UDP keep-alive uses STUN, out of scope here.
        for kind in [
            TransportType::Tls,
            TransportType::Tcp,
            TransportType::Wss,
            TransportType::Ws,
        ] {
            if let Some(transport) = self.transports.get(&kind) {
                if transport.has_connection_to(destination) {
                    trace!(
                        "MultiplexedTransport::send_raw routing {} bytes to {} via {}",
                        data.len(),
                        destination,
                        kind
                    );
                    return transport.send_raw(destination, data).await;
                }
            }
        }
        Err(TransportError::InvalidState(format!(
            "No connection-oriented transport has a live connection to {} for send_raw",
            destination
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::Uri;
    use std::str::FromStr;

    #[test]
    fn select_uri_default_is_udp() {
        let uri = Uri::from_str("sip:bob@example.com").unwrap();
        assert_eq!(select_transport_for_uri(&uri), TransportType::Udp);
    }

    #[test]
    fn select_uri_sips_is_tls() {
        let uri = Uri::from_str("sips:bob@example.com").unwrap();
        assert_eq!(select_transport_for_uri(&uri), TransportType::Tls);
    }

    #[test]
    fn select_uri_transport_param_wins_over_scheme() {
        // `sips:` would imply TLS, but explicit `;transport=tcp` overrides.
        // (Not a real-world combo, but verifies parameter precedence.)
        let uri = Uri::from_str("sips:bob@example.com;transport=tcp").unwrap();
        assert_eq!(select_transport_for_uri(&uri), TransportType::Tcp);
    }

    #[test]
    fn select_uri_explicit_tcp_param() {
        let uri = Uri::from_str("sip:bob@example.com;transport=tcp").unwrap();
        assert_eq!(select_transport_for_uri(&uri), TransportType::Tcp);
    }

    #[test]
    fn select_uri_explicit_udp_param() {
        let uri = Uri::from_str("sip:bob@example.com;transport=udp").unwrap();
        assert_eq!(select_transport_for_uri(&uri), TransportType::Udp);
    }

    #[test]
    fn select_uri_unknown_param_falls_through_to_scheme() {
        let uri = Uri::from_str("sips:bob@example.com;transport=sctp").unwrap();
        // Unknown transport param → fall through to scheme-based default.
        // For sips: that means TLS.
        assert_eq!(select_transport_for_uri(&uri), TransportType::Tls);
    }

    // ---- dispatch tests using a recording mock Transport -------------

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Counts each `send_message` invocation. Distinguishes call sites
    /// by an injected label so we can assert which mock transport the
    /// multiplexer dispatched to. Also counts `send_raw` so we can
    /// assert RFC 5626 keep-alive dispatch.
    #[derive(Debug)]
    struct CountingTransport {
        label: &'static str,
        addr: SocketAddr,
        sends: AtomicUsize,
        raw_sends: AtomicUsize,
        /// Whether this transport reports as having a connection to any
        /// destination. Used to drive `send_raw` / response-path probes.
        has_conn: std::sync::atomic::AtomicBool,
    }

    impl CountingTransport {
        fn new(label: &'static str) -> Arc<Self> {
            Arc::new(Self {
                label,
                addr: "127.0.0.1:0".parse().unwrap(),
                sends: AtomicUsize::new(0),
                raw_sends: AtomicUsize::new(0),
                has_conn: std::sync::atomic::AtomicBool::new(false),
            })
        }

        fn count(&self) -> usize {
            self.sends.load(Ordering::SeqCst)
        }

        fn raw_count(&self) -> usize {
            self.raw_sends.load(Ordering::SeqCst)
        }

        fn set_has_connection(&self, v: bool) {
            self.has_conn.store(v, Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl Transport for CountingTransport {
        fn local_addr(&self) -> TransportResult<SocketAddr> {
            Ok(self.addr)
        }

        async fn send_message(
            &self,
            _message: Message,
            _destination: SocketAddr,
        ) -> TransportResult<()> {
            self.sends.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn send_raw(&self, _destination: SocketAddr, _data: Bytes) -> TransportResult<()> {
            self.raw_sends.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn close(&self) -> TransportResult<()> {
            Ok(())
        }

        fn is_closed(&self) -> bool {
            false
        }

        fn supports_udp(&self) -> bool {
            self.label == "udp"
        }

        fn supports_tcp(&self) -> bool {
            self.label == "tcp"
        }

        fn supports_tls(&self) -> bool {
            self.label == "tls"
        }

        fn has_connection_to(&self, _remote_addr: SocketAddr) -> bool {
            self.has_conn.load(Ordering::SeqCst)
        }
    }

    fn make_invite(target: &str) -> Message {
        use rvoip_sip_core::builder::SimpleRequestBuilder;
        use rvoip_sip_core::Method;
        let req = SimpleRequestBuilder::new(Method::Invite, target)
            .unwrap()
            .from("alice", "sip:alice@example.com", Some("tagA"))
            .to("bob", target, None)
            .call_id("call-mux-test")
            .cseq(1)
            .build();
        Message::Request(req)
    }

    #[tokio::test]
    async fn dispatch_routes_sips_uri_to_tls_transport() {
        let udp = CountingTransport::new("udp");
        let tcp = CountingTransport::new("tcp");
        let tls = CountingTransport::new("tls");

        let mut by_flavour: HashMap<TransportType, Arc<dyn Transport>> = HashMap::new();
        by_flavour.insert(TransportType::Udp, udp.clone() as Arc<dyn Transport>);
        by_flavour.insert(TransportType::Tcp, tcp.clone() as Arc<dyn Transport>);
        by_flavour.insert(TransportType::Tls, tls.clone() as Arc<dyn Transport>);

        let mux = MultiplexedTransport::new(udp.clone() as Arc<dyn Transport>, by_flavour).unwrap();

        let dest: SocketAddr = "127.0.0.1:5061".parse().unwrap();
        let msg = make_invite("sips:bob@example.com");

        mux.send_message(msg, dest).await.unwrap();
        assert_eq!(tls.count(), 1, "sips: URI must route to TLS transport");
        assert_eq!(tcp.count(), 0);
        assert_eq!(udp.count(), 0);
    }

    #[tokio::test]
    async fn dispatch_routes_transport_tcp_param_to_tcp_transport() {
        let udp = CountingTransport::new("udp");
        let tcp = CountingTransport::new("tcp");

        let mut by_flavour: HashMap<TransportType, Arc<dyn Transport>> = HashMap::new();
        by_flavour.insert(TransportType::Udp, udp.clone() as Arc<dyn Transport>);
        by_flavour.insert(TransportType::Tcp, tcp.clone() as Arc<dyn Transport>);

        let mux = MultiplexedTransport::new(udp.clone() as Arc<dyn Transport>, by_flavour).unwrap();

        let dest: SocketAddr = "127.0.0.1:5060".parse().unwrap();
        let msg = make_invite("sip:bob@example.com;transport=tcp");

        mux.send_message(msg, dest).await.unwrap();
        assert_eq!(tcp.count(), 1, ";transport=tcp must route to TCP transport");
        assert_eq!(udp.count(), 0);
    }

    #[tokio::test]
    async fn dispatch_default_sip_uri_routes_to_udp() {
        let udp = CountingTransport::new("udp");
        let tcp = CountingTransport::new("tcp");

        let mut by_flavour: HashMap<TransportType, Arc<dyn Transport>> = HashMap::new();
        by_flavour.insert(TransportType::Udp, udp.clone() as Arc<dyn Transport>);
        by_flavour.insert(TransportType::Tcp, tcp.clone() as Arc<dyn Transport>);

        let mux = MultiplexedTransport::new(udp.clone() as Arc<dyn Transport>, by_flavour).unwrap();

        let dest: SocketAddr = "127.0.0.1:5060".parse().unwrap();
        let msg = make_invite("sip:bob@example.com");

        mux.send_message(msg, dest).await.unwrap();
        assert_eq!(
            udp.count(),
            1,
            "sip: URI with no transport= must default to UDP"
        );
        assert_eq!(tcp.count(), 0);
    }

    #[tokio::test]
    async fn send_raw_routes_to_transport_with_live_connection() {
        let udp = CountingTransport::new("udp");
        let tcp = CountingTransport::new("tcp");
        let tls = CountingTransport::new("tls");

        // Only TCP reports a live connection to the destination.
        tcp.set_has_connection(true);

        let mut by_flavour: HashMap<TransportType, Arc<dyn Transport>> = HashMap::new();
        by_flavour.insert(TransportType::Udp, udp.clone() as Arc<dyn Transport>);
        by_flavour.insert(TransportType::Tcp, tcp.clone() as Arc<dyn Transport>);
        by_flavour.insert(TransportType::Tls, tls.clone() as Arc<dyn Transport>);

        let mux = MultiplexedTransport::new(udp.clone() as Arc<dyn Transport>, by_flavour).unwrap();

        let dest: SocketAddr = "127.0.0.1:5060".parse().unwrap();
        mux.send_raw(dest, Bytes::from_static(b"\r\n\r\n"))
            .await
            .unwrap();

        assert_eq!(
            tcp.raw_count(),
            1,
            "send_raw must route to TCP (live connection)"
        );
        assert_eq!(tls.raw_count(), 0);
        assert_eq!(
            udp.raw_count(),
            0,
            "UDP is never used for send_raw (RFC 5626 UDP uses STUN)"
        );
    }

    #[tokio::test]
    async fn send_raw_errors_when_no_live_connection_exists() {
        let udp = CountingTransport::new("udp");
        let tcp = CountingTransport::new("tcp");
        // No transport reports a live connection.

        let mut by_flavour: HashMap<TransportType, Arc<dyn Transport>> = HashMap::new();
        by_flavour.insert(TransportType::Udp, udp.clone() as Arc<dyn Transport>);
        by_flavour.insert(TransportType::Tcp, tcp.clone() as Arc<dyn Transport>);

        let mux = MultiplexedTransport::new(udp.clone() as Arc<dyn Transport>, by_flavour).unwrap();

        let dest: SocketAddr = "127.0.0.1:5060".parse().unwrap();
        let result = mux.send_raw(dest, Bytes::from_static(b"\r\n\r\n")).await;
        assert!(
            result.is_err(),
            "send_raw must error when no connection-oriented transport has a live flow"
        );
        assert_eq!(tcp.raw_count(), 0);
        assert_eq!(udp.raw_count(), 0);
    }

    #[tokio::test]
    async fn send_raw_prefers_tls_over_tcp_when_both_live() {
        let udp = CountingTransport::new("udp");
        let tcp = CountingTransport::new("tcp");
        let tls = CountingTransport::new("tls");
        tcp.set_has_connection(true);
        tls.set_has_connection(true);

        let mut by_flavour: HashMap<TransportType, Arc<dyn Transport>> = HashMap::new();
        by_flavour.insert(TransportType::Udp, udp.clone() as Arc<dyn Transport>);
        by_flavour.insert(TransportType::Tcp, tcp.clone() as Arc<dyn Transport>);
        by_flavour.insert(TransportType::Tls, tls.clone() as Arc<dyn Transport>);

        let mux = MultiplexedTransport::new(udp.clone() as Arc<dyn Transport>, by_flavour).unwrap();

        let dest: SocketAddr = "127.0.0.1:5061".parse().unwrap();
        mux.send_raw(dest, Bytes::from_static(b"\r\n\r\n"))
            .await
            .unwrap();

        // TLS is tried first in the probe order — matches the
        // response-routing order in `pick_transport`.
        assert_eq!(tls.raw_count(), 1);
        assert_eq!(tcp.raw_count(), 0);
    }

    #[tokio::test]
    async fn dispatch_falls_back_to_default_when_flavour_missing() {
        // Caller asks for ;transport=tcp but the multiplexer was built
        // with only UDP — must fall through to the default transport
        // rather than refusing to send.
        let udp = CountingTransport::new("udp");

        let mut by_flavour: HashMap<TransportType, Arc<dyn Transport>> = HashMap::new();
        by_flavour.insert(TransportType::Udp, udp.clone() as Arc<dyn Transport>);

        let mux = MultiplexedTransport::new(udp.clone() as Arc<dyn Transport>, by_flavour).unwrap();

        let dest: SocketAddr = "127.0.0.1:5060".parse().unwrap();
        let msg = make_invite("sip:bob@example.com;transport=tcp");

        mux.send_message(msg, dest).await.unwrap();
        // Default (UDP) handled the send; counted twice would mean the
        // dispatcher delivered to both registry-UDP and default-UDP — we
        // expect exactly one delivery (registry hit OR default fallback).
        assert_eq!(udp.count(), 1);
    }
}
