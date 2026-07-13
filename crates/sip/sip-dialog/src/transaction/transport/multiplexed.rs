//! URI-aware multiplexing wrapper around multiple sip-transport
//! `Transport` implementations.
//!
//! `TransactionManager` is built around a single `Arc<dyn Transport>`. To
//! support per-call transport selection — `sip:bob@host;transport=tcp`
//! must use TCP, `sips:bob@host` must use TLS, while plain
//! `sip:bob@host` defaults to UDP — we install a `MultiplexedTransport`
//! as that single transport. It implements `Transport` itself and
//! dispatches each `send_message` call to the appropriate underlying
//! transport based on the SIP request next hop: the top Route URI when
//! present, otherwise the Request-URI (RFC 3261 §8.1.2, §18.1.1).
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
use rvoip_sip_core::{
    types::uri::Host, HeaderName, HeaderValue, Message, Method, Request, TypedHeader, Uri,
};
use rvoip_sip_transport::transport::TransportType;
use rvoip_sip_transport::{
    error::{Error as TransportError, Result as TransportResult},
    Transport, TransportAuthority, TransportFlowId, TransportRoute,
};
use tracing::{debug, trace, warn};

use super::SipTraceRuntime;
use rvoip_infra_common::events::cross_crate::SipTraceDirection;

fn safe_method_label(method: &Method) -> &'static str {
    match method {
        Method::Invite => "INVITE",
        Method::Ack => "ACK",
        Method::Bye => "BYE",
        Method::Cancel => "CANCEL",
        Method::Register => "REGISTER",
        Method::Options => "OPTIONS",
        Method::Subscribe => "SUBSCRIBE",
        Method::Notify => "NOTIFY",
        Method::Update => "UPDATE",
        Method::Refer => "REFER",
        Method::Info => "INFO",
        Method::Message => "MESSAGE",
        Method::Prack => "PRACK",
        Method::Publish => "PUBLISH",
        Method::Extension(_) => "extension",
    }
}

/// Returns the URI that determines the next hop for an outbound request:
/// the first Route URI when present, otherwise the Request-URI. Route
/// address parameters are folded back into the URI so `;transport=tls`
/// on a name-addr Route still affects transport and default-port logic.
pub fn next_hop_uri_for_request(request: &Request) -> Uri {
    top_route_uri(request).unwrap_or_else(|| request.uri().clone())
}

/// Returns the exact URI that determines the next hop, rejecting a Route
/// header that is present but cannot be interpreted structurally. Callers
/// that allocate a transaction or resolve DNS must use this fallible form;
/// treating an unusable Route as absent can bypass an explicitly selected
/// proxy and send credentials or signaling to the Request-URI instead.
pub fn exact_next_hop_uri_for_request(request: &Request) -> TransportResult<Uri> {
    for header in &request.headers {
        if !header.name().wire_eq(&HeaderName::Route) {
            continue;
        }
        let next_hop = match header {
            TypedHeader::Route(route) => route.first().map(|entry| {
                let mut uri = entry.0.uri.clone();
                for param in &entry.0.params {
                    uri = uri.with_parameter(param.clone());
                }
                uri
            }),
            TypedHeader::Other(_, HeaderValue::Route(entries)) => entries.first().map(|entry| {
                let mut uri = entry.0.uri.clone();
                for param in &entry.0.params {
                    uri = uri.with_parameter(param.clone());
                }
                uri
            }),
            _ => None,
        };
        return next_hop.ok_or_else(|| {
            TransportError::UnsupportedTransport(
                "outbound request contains an unusable Route header".into(),
            )
        });
    }

    Ok(request.uri().clone())
}

/// Returns the top Route URI for an outbound request, when a route set is
/// present.
pub fn top_route_uri(request: &Request) -> Option<Uri> {
    request
        .headers
        .iter()
        .filter_map(|header| {
            if header.name().wire_eq(&HeaderName::Route) {
                match header {
                    TypedHeader::Route(route) => route.first().map(|entry| {
                        let mut uri = entry.0.uri.clone();
                        for param in &entry.0.params {
                            uri = uri.with_parameter(param.clone());
                        }
                        uri
                    }),
                    TypedHeader::Other(_, HeaderValue::Route(entries)) => {
                        entries.first().map(|entry| {
                            let mut uri = entry.0.uri.clone();
                            for param in &entry.0.params {
                                uri = uri.with_parameter(param.clone());
                            }
                            uri
                        })
                    }
                    _ => None,
                }
            } else {
                None
            }
        })
        .next()
}

/// Picks the SIP transport flavour to use for a request's next hop per
/// RFC 3261 §8.1.2 / §18.1.1, §26.2 (TLS is mandatory for `sips:` URIs)
/// and §19.1.5 (`;transport=` URI parameter).
pub fn select_transport_for_request(request: &Request) -> TransportType {
    select_transport_for_uri(&next_hop_uri_for_request(request))
}

fn validate_next_hop_transport(uri: &Uri) -> TransportResult<()> {
    use rvoip_sip_core::types::uri::Scheme;

    if matches!(uri.scheme(), Scheme::Sips)
        && uri.transport().is_some_and(|transport| {
            matches!(transport.to_ascii_lowercase().as_str(), "udp" | "ws")
        })
    {
        return Err(TransportError::UnsupportedTransport(
            "sips: next hop cannot use an insecure transport".into(),
        ));
    }
    Ok(())
}

/// Enforce the `sips:` confidentiality boundary at the final transport
/// dispatch seam. Resolvers and callers may supply an explicit route, but
/// that route is advisory and must never downgrade a secure next hop to a
/// plaintext transport.
fn validate_message_route_security(
    message: &Message,
    route: &TransportRoute,
) -> TransportResult<()> {
    let Message::Request(request) = message else {
        return Ok(());
    };
    validate_request_route_security(request, route)
}

/// Validate an explicitly supplied client route before transaction
/// allocation. This duplicates the final multiplexer guard intentionally:
/// callers may construct a `TransactionManager` over a concrete TCP transport
/// and must receive the same no-downgrade guarantee.
pub(crate) fn validate_request_route_security(
    request: &Request,
    route: &TransportRoute,
) -> TransportResult<()> {
    use rvoip_sip_core::types::uri::Scheme;

    let next_hop = exact_next_hop_uri_for_request(request)?;
    validate_next_hop_transport(&next_hop)?;
    let selected_transport = route
        .transport_type
        .unwrap_or_else(|| select_transport_for_uri(&next_hop));
    // A sips Request-URI is an end-to-end confidentiality requirement. A
    // plaintext `sip:` top Route must not downgrade its first hop.
    if (matches!(request.uri().scheme(), Scheme::Sips) || matches!(next_hop.scheme(), Scheme::Sips))
        && !matches!(selected_transport, TransportType::Tls | TransportType::Wss)
    {
        return Err(TransportError::UnsupportedTransport(
            "sips: request route cannot use a plaintext transport".into(),
        ));
    }
    Ok(())
}

/// Build the authority-bearing route selected for an outbound request.
pub fn transport_route_for_request(
    request: &Request,
    destination: SocketAddr,
) -> TransportResult<TransportRoute> {
    let next_hop = exact_next_hop_uri_for_request(request)?;
    validate_next_hop_transport(&next_hop)?;
    let authority = match &next_hop.host {
        Host::Domain(domain) => TransportAuthority::dns(domain.clone())?,
        Host::Address(address) => TransportAuthority::ip(*address),
    };
    Ok(TransportRoute::new(destination)
        .with_transport_type(select_transport_for_uri(&next_hop))
        .with_authority(authority))
}

/// Select transport from a URI alone.
///
/// Re-exported from
/// [`rvoip_sip_transport::resolver::select_transport_for_uri`] so the
/// dialog layer and the resolver share a single source of truth.
pub use rvoip_sip_transport::resolver::select_transport_for_uri;

/// `Transport` implementation that owns a registry of underlying
/// transports keyed by `TransportType` and dispatches `send_message`
/// calls to whichever one matches the request next-hop URI.
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
    /// Optional SIP trace publisher for outbound transport-boundary events.
    sip_trace: Option<Arc<SipTraceRuntime>>,
}

impl MultiplexedTransport {
    fn transport_for_kind(&self, kind: TransportType) -> Option<Arc<dyn Transport>> {
        self.transports.get(&kind).cloned().or_else(|| {
            (self.default.default_transport_type() == kind).then(|| self.default.clone())
        })
    }

    /// Build a multiplexer.
    ///
    /// `default` is what `local_addr()` reports and is used whenever a
    /// requested transport flavour isn't in the registry. `transports`
    /// is the per-flavour registry (typically UDP + TCP, plus TLS when
    /// configured). `default` does not have to appear in `transports` —
    /// the dispatcher will fall back to it explicitly.
    /// Build a multiplexer without the (crate-private) SIP-trace runtime.
    /// Convenience entry point for integration tests and external callers
    /// that don't need transport-boundary tracing.
    pub fn new_without_trace(
        default: Arc<dyn Transport>,
        transports: HashMap<TransportType, Arc<dyn Transport>>,
    ) -> TransportResult<Self> {
        Self::new(default, transports, None)
    }

    pub(crate) fn new(
        default: Arc<dyn Transport>,
        transports: HashMap<TransportType, Arc<dyn Transport>>,
        sip_trace: Option<Arc<SipTraceRuntime>>,
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
            sip_trace,
        })
    }

    /// Pick the underlying transport to use for a given outbound
    /// `Message` bound for `destination`.
    ///
    /// - **Requests** route by top Route URI when present, otherwise
    ///   Request-URI, then by scheme + `;transport=` parameter (RFC 3261
    ///   §8.1.2, §19.1.5, §26.2).
    /// - **Responses** on a connection-oriented transport must carry the
    ///   exact ingress [`TransportRoute`]. Socket addresses are not flow
    ///   identities: multiple authenticated, inbound, or outbound flows may
    ///   share an address. Callers must supply an explicit UDP route or an
    ///   exact stream route; this address-only path always fails closed.
    fn pick_transport(
        &self,
        message: &Message,
        destination: SocketAddr,
    ) -> TransportResult<(TransportType, Arc<dyn Transport>)> {
        match message {
            Message::Request(request) => {
                let next_hop = next_hop_uri_for_request(request);
                validate_next_hop_transport(&next_hop)?;
                let want = select_transport_for_uri(&next_hop);
                if let Some(transport) = self.transports.get(&want) {
                    trace!(
                        "MultiplexedTransport: routing {} to {} via URI selection",
                        safe_method_label(&request.method),
                        want
                    );
                    Ok((want, transport.clone()))
                } else if matches!(want, TransportType::Tls | TransportType::Wss) {
                    Err(TransportError::UnsupportedTransport(format!(
                        "{} requires TLS by next-hop URI {}, but no TLS transport is registered",
                        safe_method_label(&request.method),
                        next_hop_uri_for_request(request)
                    )))
                } else {
                    debug!(
                        "MultiplexedTransport: no {} transport registered for {}; falling back to default",
                        want,
                        safe_method_label(&request.method)
                    );
                    Ok((self.default.default_transport_type(), self.default.clone()))
                }
            }
            Message::Response(response) => {
                Err(TransportError::InvalidState(format!(
                    "{} response to {destination} requires an explicit UDP route or exact connection flow",
                    response.status_code()
                )))
            }
        }
    }

    /// RFC 3263 §4.3 multi-candidate failover send.
    ///
    /// Walks `candidates` in order. For each, dispatches the message via
    /// the normal [`Transport::send_message`] path (which honours the
    /// usual URI-based transport selection and the RFC 3261 §18.1.1
    /// UDP→TCP MTU failover). On a recoverable transport error
    /// (`is_recoverable()` — connect failures, timeouts, send failures,
    /// closed connections) it logs and advances to the next candidate.
    /// On a non-recoverable error (e.g. `MessageTooLarge`,
    /// `UnsupportedTransport`, `InvalidUri`) it returns immediately —
    /// retrying a different candidate won't help.
    ///
    /// Returns the `SocketAddr` of the candidate that succeeded, or the
    /// last error when every candidate failed. An empty input vec
    /// returns `Err(InvalidAddress("no candidates"))`.
    ///
    /// Note: UDP sockets are connectionless, so a UDP `send_message` to
    /// an unreachable host typically returns `Ok(())`. RFC 3263 §4.3
    /// failover is most useful on TCP/TLS, where `ConnectFailed` is
    /// surfaced synchronously.
    pub async fn send_message_with_failover(
        &self,
        message: Message,
        candidates: &[rvoip_sip_transport::resolver::ResolvedTarget],
    ) -> TransportResult<SocketAddr> {
        if candidates.is_empty() {
            return Err(TransportError::InvalidAddress(
                "send_message_with_failover called with no candidates".into(),
            ));
        }
        let total = candidates.len();
        let mut last_err: Option<TransportError> = None;
        for (idx, target) in candidates.iter().enumerate() {
            let attempt = idx + 1;
            let mut route = match &message {
                Message::Request(request) => transport_route_for_request(request, target.addr)?,
                Message::Response(_) => TransportRoute::new(target.addr),
            };
            route.transport_type = Some(target.transport);
            if let Some(authority) = &target.authority {
                route.authority = Some(authority.clone());
            }
            match self.send_message_via(message.clone(), route).await {
                Ok(()) => {
                    if attempt > 1 {
                        debug!(
                            "RFC 3263 §4.3: candidate {} of {} ({}/{}) succeeded after {} prior failures",
                            attempt,
                            total,
                            target.transport,
                            target.addr,
                            attempt - 1
                        );
                    }
                    return Ok(target.addr);
                }
                Err(e) if e.is_recoverable() => {
                    debug!(attempt, total, transport=%target.transport, destination=%target.addr, error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "Candidate failed recoverably; trying next");
                    last_err = Some(e);
                    continue;
                }
                Err(e) => {
                    debug!(attempt, total, transport=%target.transport, destination=%target.addr, error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "Candidate failed non-recoverably; aborting failover");
                    return Err(e);
                }
            }
        }
        Err(last_err
            .unwrap_or_else(|| TransportError::InvalidAddress("all candidates failed".into())))
    }
}

#[async_trait]
impl Transport for MultiplexedTransport {
    fn local_addr(&self) -> TransportResult<SocketAddr> {
        Ok(self.local_addr)
    }

    async fn send_message(&self, message: Message, destination: SocketAddr) -> TransportResult<()> {
        let route = match &message {
            Message::Request(request) => transport_route_for_request(request, destination)?,
            Message::Response(_) => TransportRoute::new(destination),
        };
        self.send_message_via(message, route).await
    }

    async fn send_message_via(
        &self,
        message: Message,
        route: TransportRoute,
    ) -> TransportResult<()> {
        self.send_message_on_route(message, route).await.map(|_| ())
    }

    async fn prepare_message_route(
        &self,
        message: &Message,
        mut route: TransportRoute,
    ) -> TransportResult<TransportRoute> {
        rvoip_sip_core::validation::validate_typed_outbound_message(message).map_err(|_| {
            TransportError::ProtocolError(
                "outbound typed SIP message failed wire-safety validation".into(),
            )
        })?;
        validate_message_route_security(message, &route)?;
        let kind = match route.transport_type {
            Some(kind) => kind,
            None => match message {
                Message::Request(request) => {
                    let next_hop = next_hop_uri_for_request(request);
                    validate_next_hop_transport(&next_hop)?;
                    select_transport_for_uri(&next_hop)
                }
                Message::Response(_) => {
                    return Err(TransportError::InvalidState(
                        "response route is missing its transport type".into(),
                    ));
                }
            },
        };
        let transport = self.transport_for_kind(kind).ok_or_else(|| {
            TransportError::UnsupportedTransport(format!(
                "prepared route transport {kind} is not registered"
            ))
        })?;
        route.transport_type = Some(kind);
        transport.prepare_message_route(message, route).await
    }

    async fn send_message_on_route(
        &self,
        mut message: Message,
        route: TransportRoute,
    ) -> TransportResult<TransportRoute> {
        let destination = route.destination;
        rvoip_sip_core::validation::validate_typed_outbound_message(&message).map_err(|_| {
            TransportError::ProtocolError(
                "outbound typed SIP message failed wire-safety validation".into(),
            )
        })?;
        validate_message_route_security(&message, &route)?;

        let (mut transport_type, mut transport) = if let Some(flow_id) = route.flow_id {
            let kind = route.transport_type.ok_or_else(|| {
                TransportError::InvalidState(format!(
                    "exact flow {} is missing its transport type",
                    flow_id.as_u64()
                ))
            })?;
            let transport = self.transport_for_kind(kind).ok_or_else(|| {
                TransportError::UnsupportedTransport(format!(
                    "exact response flow transport {kind} is not registered"
                ))
            })?;
            (kind, transport)
        } else if let (Message::Response(_), Some(kind)) = (&message, route.transport_type) {
            let transport = self.transport_for_kind(kind).ok_or_else(|| {
                TransportError::UnsupportedTransport(format!(
                    "response route transport {kind} is not registered"
                ))
            })?;
            (kind, transport)
        } else if let Some(kind) = route.transport_type {
            if let Some(transport) = self.transport_for_kind(kind) {
                (kind, transport)
            } else if matches!(kind, TransportType::Tls | TransportType::Wss) {
                return Err(TransportError::UnsupportedTransport(format!(
                    "secure request route transport {kind} is not registered"
                )));
            } else {
                self.pick_transport(&message, destination)?
            }
        } else {
            self.pick_transport(&message, destination)?
        };

        // RFC 3261 §18.1.1 — if the URI selected UDP but the request
        // would exceed UDP's safe size, fail over to TCP when a TCP
        // transport is registered. If none is, fail closed: this is a
        // protocol MUST.
        if transport_type == TransportType::Udp {
            if let Message::Request(ref req) = message {
                let size = message.to_bytes().len();
                let limit = transport.max_safe_message_size();
                if size > limit {
                    match self.transports.get(&TransportType::Tcp) {
                        Some(tcp) => {
                            debug!(
                                "MultiplexedTransport: {} is {} bytes (UDP limit {}), failing over to TCP per RFC 3261 §18.1.1",
                                safe_method_label(&req.method), size, limit
                            );
                            if let Message::Request(ref mut req_mut) = message {
                                crate::transaction::utils::set_top_via_protocol(req_mut, "TCP");
                            }
                            transport_type = TransportType::Tcp;
                            transport = tcp.clone();
                        }
                        None => {
                            warn!(
                                "MultiplexedTransport: {} is {} bytes (UDP limit {}) and no TCP transport is registered; refusing to send",
                                safe_method_label(&req.method), size, limit
                            );
                            return Err(TransportError::MessageTooLarge(size));
                        }
                    }
                }
            }
        }

        if let Some(trace) = &self.sip_trace {
            let local_addr = transport.local_addr().unwrap_or(self.local_addr);
            trace.publish(
                SipTraceDirection::Outbound,
                transport_type,
                local_addr,
                destination,
                &message,
            );
        }
        let mut selected_route = route;
        selected_route.transport_type = Some(transport_type);
        transport
            .send_message_on_route(message, selected_route)
            .await
    }

    async fn send_message_raw(&self, bytes: Bytes, destination: SocketAddr) -> TransportResult<()> {
        self.send_message_raw_via(bytes, TransportRoute::new(destination))
            .await
    }

    async fn send_message_raw_via(
        &self,
        bytes: Bytes,
        route: TransportRoute,
    ) -> TransportResult<()> {
        let destination = route.destination;
        if let Some(flow_id) = route.flow_id {
            let kind = route.transport_type.ok_or_else(|| {
                TransportError::InvalidState(format!(
                    "exact raw response flow {} is missing its transport type",
                    flow_id.as_u64()
                ))
            })?;
            let transport = self.transport_for_kind(kind).ok_or_else(|| {
                TransportError::UnsupportedTransport(format!(
                    "raw response flow transport {kind} is not registered"
                ))
            })?;
            return transport.send_message_raw_via(bytes, route).await;
        }
        if let Some(kind) = route.transport_type {
            if kind != TransportType::Udp && route.authority.is_none() {
                return Err(TransportError::InvalidState(format!(
                    "raw SIP on {kind} requires an outbound authority or exact flow"
                )));
            }
            let transport = self.transport_for_kind(kind).ok_or_else(|| {
                TransportError::UnsupportedTransport(format!(
                    "raw SIP route transport {kind} is not registered"
                ))
            })?;
            return transport.send_message_raw_via(bytes, route).await;
        }
        Err(TransportError::InvalidState(format!(
            "raw SIP to {destination} requires an explicit UDP route, outbound authority, or exact flow"
        )))
    }

    async fn close(&self) -> TransportResult<()> {
        let mut last_err: Option<TransportError> = None;
        for (kind, transport) in &self.transports {
            if let Err(e) = transport.close().await {
                warn!(transport=%kind, error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "Error closing transport");
                last_err = Some(e);
            }
        }
        if let Err(e) = self.default.close().await {
            warn!(error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "Error closing default transport");
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
        self.transports.contains_key(&TransportType::Tcp) || self.default.supports_tcp()
    }

    fn supports_tls(&self) -> bool {
        self.transports.contains_key(&TransportType::Tls) || self.default.supports_tls()
    }

    fn supports_ws(&self) -> bool {
        self.transports.contains_key(&TransportType::Ws) || self.default.supports_ws()
    }

    fn supports_wss(&self) -> bool {
        self.transports.contains_key(&TransportType::Wss) || self.default.supports_wss()
    }

    fn default_transport_type(&self) -> TransportType {
        self.default.default_transport_type()
    }

    fn max_safe_message_size(&self) -> usize {
        self.default.max_safe_message_size()
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
            if let Some(transport) = self.transport_for_kind(kind) {
                if transport.has_connection_to(remote_addr) {
                    return true;
                }
            }
        }
        false
    }

    fn flow_id_for_route(&self, route: &TransportRoute) -> Option<TransportFlowId> {
        if let Some(kind) = route.transport_type {
            return self
                .transport_for_kind(kind)
                .and_then(|transport| transport.flow_id_for_route(route));
        }
        let mut found = None;
        for transport in self.transports.values() {
            let Some(flow_id) = transport.flow_id_for_route(route) else {
                continue;
            };
            if found.replace(flow_id).is_some() {
                return None;
            }
        }
        if !self
            .transports
            .contains_key(&self.default.default_transport_type())
        {
            if let Some(flow_id) = self.default.flow_id_for_route(route) {
                if found.replace(flow_id).is_some() {
                    return None;
                }
            }
        }
        found
    }

    async fn resolve_flow_id_for_route(&self, route: &TransportRoute) -> Option<TransportFlowId> {
        if let Some(kind) = route.transport_type {
            return match self.transport_for_kind(kind) {
                Some(transport) => transport.resolve_flow_id_for_route(route).await,
                None => None,
            };
        }

        let mut found = None;
        for transport in self.transports.values() {
            let Some(flow_id) = transport.resolve_flow_id_for_route(route).await else {
                continue;
            };
            if found.replace(flow_id).is_some() {
                return None;
            }
        }
        if !self
            .transports
            .contains_key(&self.default.default_transport_type())
        {
            if let Some(flow_id) = self.default.resolve_flow_id_for_route(route).await {
                if found.replace(flow_id).is_some() {
                    return None;
                }
            }
        }
        found
    }

    async fn send_raw(&self, destination: SocketAddr, data: Bytes) -> TransportResult<()> {
        self.send_raw_via(TransportRoute::new(destination), data)
            .await
    }

    async fn send_raw_via(&self, route: TransportRoute, data: Bytes) -> TransportResult<()> {
        let flow_id = route.flow_id.ok_or_else(|| {
            TransportError::InvalidState(
                "raw lifecycle traffic requires an exact connection flow".into(),
            )
        })?;
        let kind = route.transport_type.ok_or_else(|| {
            TransportError::InvalidState(format!(
                "exact raw flow {} is missing its transport type",
                flow_id.as_u64()
            ))
        })?;
        let transport = self.transport_for_kind(kind).ok_or_else(|| {
            TransportError::UnsupportedTransport(format!(
                "raw flow transport {kind} is not registered"
            ))
        })?;
        transport.send_raw_via(route, data).await
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
    fn select_uri_sips_tcp_hint_still_requires_tls() {
        let uri = Uri::from_str("sips:bob@example.com;transport=tcp").unwrap();
        assert_eq!(select_transport_for_uri(&uri), TransportType::Tls);
    }

    #[test]
    fn secure_route_builder_rejects_insecure_uri_hints() {
        let destination = "127.0.0.1:5061".parse().unwrap();
        for target in [
            "sips:bob@example.com;transport=udp",
            "sips:bob@example.com;transport=ws",
        ] {
            let Message::Request(request) = make_invite(target) else {
                unreachable!();
            };
            assert!(matches!(
                transport_route_for_request(&request, destination),
                Err(TransportError::UnsupportedTransport(_))
            ));
        }
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
        raw_message_sends: AtomicUsize,
        /// Whether this transport reports as having a connection to any
        /// destination. Used to drive `send_raw` / response-path probes.
        has_conn: std::sync::atomic::AtomicBool,
        last_route: std::sync::Mutex<Option<TransportRoute>>,
    }

    impl CountingTransport {
        fn new(label: &'static str) -> Arc<Self> {
            Arc::new(Self {
                label,
                addr: "127.0.0.1:0".parse().unwrap(),
                sends: AtomicUsize::new(0),
                raw_sends: AtomicUsize::new(0),
                raw_message_sends: AtomicUsize::new(0),
                has_conn: std::sync::atomic::AtomicBool::new(false),
                last_route: std::sync::Mutex::new(None),
            })
        }

        fn count(&self) -> usize {
            self.sends.load(Ordering::SeqCst)
        }

        fn raw_count(&self) -> usize {
            self.raw_sends.load(Ordering::SeqCst)
        }

        fn raw_message_count(&self) -> usize {
            self.raw_message_sends.load(Ordering::SeqCst)
        }

        fn set_has_connection(&self, v: bool) {
            self.has_conn.store(v, Ordering::SeqCst);
        }

        fn last_route(&self) -> Option<TransportRoute> {
            self.last_route.lock().unwrap().clone()
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

        async fn send_message_via(
            &self,
            _message: Message,
            route: TransportRoute,
        ) -> TransportResult<()> {
            *self.last_route.lock().unwrap() = Some(route);
            self.sends.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn send_raw(&self, _destination: SocketAddr, _data: Bytes) -> TransportResult<()> {
            self.raw_sends.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn send_message_raw(
            &self,
            _bytes: Bytes,
            _destination: SocketAddr,
        ) -> TransportResult<()> {
            self.raw_message_sends.fetch_add(1, Ordering::SeqCst);
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

        fn supports_ws(&self) -> bool {
            self.label == "ws"
        }

        fn supports_wss(&self) -> bool {
            self.label == "wss"
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

        let mux =
            MultiplexedTransport::new(udp.clone() as Arc<dyn Transport>, by_flavour, None).unwrap();

        let dest: SocketAddr = "127.0.0.1:5061".parse().unwrap();
        let msg = make_invite("sips:bob@example.com");

        mux.send_message(msg, dest).await.unwrap();
        assert_eq!(tls.count(), 1, "sips: URI must route to TLS transport");
        assert_eq!(tcp.count(), 0);
        assert_eq!(udp.count(), 0);
    }

    #[tokio::test]
    async fn explicit_plaintext_route_cannot_downgrade_sips() {
        let udp = CountingTransport::new("udp");
        let tcp = CountingTransport::new("tcp");
        let tls = CountingTransport::new("tls");
        let mut by_flavour: HashMap<TransportType, Arc<dyn Transport>> = HashMap::new();
        by_flavour.insert(TransportType::Udp, udp.clone());
        by_flavour.insert(TransportType::Tcp, tcp.clone());
        by_flavour.insert(TransportType::Tls, tls.clone());
        let mux = MultiplexedTransport::new(udp.clone(), by_flavour, None).unwrap();
        let destination = "127.0.0.1:5061".parse().unwrap();
        let message = make_invite("sips:bob@example.com");

        let error = mux
            .send_message_via(
                message.clone(),
                TransportRoute::new(destination).with_transport_type(TransportType::Tcp),
            )
            .await
            .expect_err("explicit plaintext route must be rejected");
        assert!(matches!(error, TransportError::UnsupportedTransport(_)));

        let malicious_candidate = rvoip_sip_transport::resolver::ResolvedTarget::immediate(
            destination,
            TransportType::Tcp,
        );
        let error = mux
            .send_message_with_failover(message, &[malicious_candidate])
            .await
            .expect_err("resolver candidate must not bypass sips policy");
        assert!(matches!(error, TransportError::UnsupportedTransport(_)));
        assert_eq!(udp.count(), 0);
        assert_eq!(tcp.count(), 0);
        assert_eq!(tls.count(), 0);
    }

    #[tokio::test]
    async fn plaintext_top_route_cannot_downgrade_sips_request_uri() {
        use rvoip_sip_core::builder::SimpleRequestBuilder;
        use rvoip_sip_core::types::route::Route;

        let udp = CountingTransport::new("udp");
        let tls = CountingTransport::new("tls");
        let mut by_flavour: HashMap<TransportType, Arc<dyn Transport>> = HashMap::new();
        by_flavour.insert(TransportType::Udp, udp.clone());
        by_flavour.insert(TransportType::Tls, tls.clone());
        let mux = MultiplexedTransport::new(udp.clone(), by_flavour, None).unwrap();
        let proxy: Uri = "sip:proxy.example.com;lr;transport=udp".parse().unwrap();
        let request = SimpleRequestBuilder::new(Method::Invite, "sips:bob@example.com")
            .unwrap()
            .from("alice", "sips:alice@example.com", Some("tagA"))
            .to("bob", "sips:bob@example.com", None)
            .call_id("sips-route-downgrade")
            .cseq(1)
            .header(TypedHeader::Route(Route::with_uri(proxy)))
            .build();

        let error = mux
            .send_message_via(
                Message::Request(request),
                TransportRoute::new("127.0.0.1:5060".parse().unwrap())
                    .with_transport_type(TransportType::Udp),
            )
            .await
            .expect_err("sips Request-URI requires a secure first hop");
        assert!(matches!(error, TransportError::UnsupportedTransport(_)));
        assert_eq!(udp.count(), 0);
        assert_eq!(tls.count(), 0);
    }

    #[test]
    fn semantic_other_route_is_used_and_unstructured_route_is_rejected() {
        use rvoip_sip_core::builder::SimpleRequestBuilder;
        use rvoip_sip_core::types::route::Route;

        let route: Uri = "sips:proxy.example.com:5061;lr".parse().unwrap();
        let mut request = SimpleRequestBuilder::new(Method::Options, "sip:target.example.com")
            .unwrap()
            .from("alice", "sip:alice@example.com", Some("tagA"))
            .to("target", "sip:target.example.com", None)
            .call_id("semantic-other-route")
            .cseq(1)
            .build();
        request.headers.push(TypedHeader::Other(
            HeaderName::Other("rOuTe".into()),
            HeaderValue::Route(Route::with_uri(route.clone()).0),
        ));
        assert_eq!(top_route_uri(&request), Some(route));

        request.headers.pop();
        request.headers.push(TypedHeader::Other(
            HeaderName::Other("Route".into()),
            HeaderValue::Raw(b"<sip:unparsed.example.com;lr>".to_vec()),
        ));
        let error = validate_request_route_security(
            &request,
            &TransportRoute::new("127.0.0.1:5060".parse().unwrap())
                .with_transport_type(TransportType::Udp),
        )
        .expect_err("unstructured semantic Route must fail closed");
        assert!(matches!(error, TransportError::UnsupportedTransport(_)));
    }

    #[tokio::test]
    async fn dispatch_uses_top_route_before_request_uri() {
        use rvoip_sip_core::builder::SimpleRequestBuilder;
        use rvoip_sip_core::types::route::Route;
        use rvoip_sip_core::{Method, TypedHeader, Uri};

        let udp = CountingTransport::new("udp");
        let tls = CountingTransport::new("tls");

        let mut by_flavour: HashMap<TransportType, Arc<dyn Transport>> = HashMap::new();
        by_flavour.insert(TransportType::Udp, udp.clone() as Arc<dyn Transport>);
        by_flavour.insert(TransportType::Tls, tls.clone() as Arc<dyn Transport>);

        let mux =
            MultiplexedTransport::new(udp.clone() as Arc<dyn Transport>, by_flavour, None).unwrap();

        let route: Uri = "sips:proxy.example.com:5061;lr;transport=tls"
            .parse()
            .unwrap();
        let req = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
            .unwrap()
            .from("alice", "sip:alice@example.com", Some("tagA"))
            .to("bob", "sip:bob@example.com", None)
            .call_id("call-mux-route-test")
            .cseq(1)
            .header(TypedHeader::Route(Route::with_uri(route)))
            .build();

        let dest: SocketAddr = "127.0.0.1:5061".parse().unwrap();
        mux.send_message(Message::Request(req), dest).await.unwrap();
        assert_eq!(tls.count(), 1, "top Route must select TLS");
        assert_eq!(
            tls.last_route().and_then(|route| route.authority),
            Some(TransportAuthority::Dns("proxy.example.com".into())),
            "top Route authority must survive transport selection and DNS resolution"
        );
        assert_eq!(udp.count(), 0);
    }

    #[tokio::test]
    async fn dispatch_uses_transport_param_on_route_address() {
        use rvoip_sip_core::builder::SimpleRequestBuilder;
        use rvoip_sip_core::types::param::Param;
        use rvoip_sip_core::types::route::Route;
        use rvoip_sip_core::{Address, Method, TypedHeader, Uri};

        let udp = CountingTransport::new("udp");
        let tls = CountingTransport::new("tls");

        let mut by_flavour: HashMap<TransportType, Arc<dyn Transport>> = HashMap::new();
        by_flavour.insert(TransportType::Udp, udp.clone() as Arc<dyn Transport>);
        by_flavour.insert(TransportType::Tls, tls.clone() as Arc<dyn Transport>);

        let mux =
            MultiplexedTransport::new(udp.clone() as Arc<dyn Transport>, by_flavour, None).unwrap();

        let route_uri: Uri = "sip:proxy.example.com:5061".parse().unwrap();
        let mut route_address = Address::new(route_uri);
        route_address.params.push(Param::Lr);
        route_address.params.push(Param::transport("tls"));
        let route = Route::with_address(route_address);
        let req = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
            .unwrap()
            .from("alice", "sip:alice@example.com", Some("tagA"))
            .to("bob", "sip:bob@example.com", None)
            .call_id("call-mux-route-param-test")
            .cseq(1)
            .header(TypedHeader::Route(route))
            .build();

        let dest: SocketAddr = "127.0.0.1:5061".parse().unwrap();
        mux.send_message(Message::Request(req), dest).await.unwrap();
        assert_eq!(
            tls.count(),
            1,
            "Route address transport=tls must select TLS"
        );
        assert_eq!(udp.count(), 0);
    }

    #[tokio::test]
    async fn dispatch_routes_transport_tcp_param_to_tcp_transport() {
        let udp = CountingTransport::new("udp");
        let tcp = CountingTransport::new("tcp");

        let mut by_flavour: HashMap<TransportType, Arc<dyn Transport>> = HashMap::new();
        by_flavour.insert(TransportType::Udp, udp.clone() as Arc<dyn Transport>);
        by_flavour.insert(TransportType::Tcp, tcp.clone() as Arc<dyn Transport>);

        let mux =
            MultiplexedTransport::new(udp.clone() as Arc<dyn Transport>, by_flavour, None).unwrap();

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

        let mux =
            MultiplexedTransport::new(udp.clone() as Arc<dyn Transport>, by_flavour, None).unwrap();

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
    async fn typed_mux_rejects_invalid_fields_before_child_dispatch() {
        use rvoip_sip_core::types::call_info::CallInfoValue;
        use rvoip_sip_core::types::headers::HeaderValue;
        use rvoip_sip_core::types::param::Param;
        use rvoip_sip_core::{Response, StatusCode};

        let udp = CountingTransport::new("udp");
        let mut by_flavour: HashMap<TransportType, Arc<dyn Transport>> = HashMap::new();
        by_flavour.insert(TransportType::Udp, udp.clone() as Arc<dyn Transport>);
        let mux =
            MultiplexedTransport::new(udp.clone() as Arc<dyn Transport>, by_flavour, None).unwrap();
        let destination = "127.0.0.1:5060".parse().unwrap();
        let mut unsafe_structured = Request::new(Method::Options, Uri::sip("example.test"));
        unsafe_structured.headers.push(TypedHeader::Other(
            HeaderName::Other("X-Structured".into()),
            HeaderValue::CallInfo(vec![CallInfoValue::new(Uri::sip("example.test"))
                .with_param(Param::Other(
                    "purpose\r\nX-Injected: mux-structured-secret".into(),
                    None,
                ))]),
        ));
        let unsafe_uri = Request::new(
            Method::Options,
            Uri::custom("sip:bob@example.test\r\nX-Injected: mux-uri-secret"),
        );

        for message in [
            Message::Response(
                Response::new(StatusCode::Ok).with_reason("OK\r\nX-Injected: mux-reason-secret"),
            ),
            Message::Request(unsafe_structured),
            Message::Request(unsafe_uri),
        ] {
            let error = mux
                .send_message(message, destination)
                .await
                .expect_err("multiplexer must reject before child dispatch");
            assert!(matches!(error, TransportError::ProtocolError(_)));
            for secret in [
                "mux-reason-secret",
                "mux-structured-secret",
                "mux-uri-secret",
            ] {
                assert!(!error.to_string().contains(secret));
            }
        }
        assert_eq!(udp.count(), 0);
    }

    #[tokio::test]
    async fn send_raw_rejects_address_only_even_with_live_connection() {
        let udp = CountingTransport::new("udp");
        let tcp = CountingTransport::new("tcp");
        let tls = CountingTransport::new("tls");

        // Only TCP reports a live connection to the destination.
        tcp.set_has_connection(true);

        let mut by_flavour: HashMap<TransportType, Arc<dyn Transport>> = HashMap::new();
        by_flavour.insert(TransportType::Udp, udp.clone() as Arc<dyn Transport>);
        by_flavour.insert(TransportType::Tcp, tcp.clone() as Arc<dyn Transport>);
        by_flavour.insert(TransportType::Tls, tls.clone() as Arc<dyn Transport>);

        let mux =
            MultiplexedTransport::new(udp.clone() as Arc<dyn Transport>, by_flavour, None).unwrap();

        let dest: SocketAddr = "127.0.0.1:5060".parse().unwrap();
        let error = mux
            .send_raw(dest, Bytes::from_static(b"\r\n\r\n"))
            .await
            .expect_err("an address-only lifecycle send must fail closed");
        assert!(matches!(error, TransportError::InvalidState(_)));

        assert_eq!(tcp.raw_count(), 0);
        assert_eq!(tls.raw_count(), 0);
        assert_eq!(
            udp.raw_count(),
            0,
            "UDP is never used for send_raw (RFC 5626 UDP uses STUN)"
        );
    }

    #[tokio::test]
    async fn flowless_response_fails_without_probing_ws_or_wss_children() {
        use rvoip_sip_core::{Response, StatusCode};

        for (kind, label) in [(TransportType::Ws, "ws"), (TransportType::Wss, "wss")] {
            let udp = CountingTransport::new("udp");
            let websocket = CountingTransport::new(label);
            websocket.set_has_connection(true);
            let mut transports: HashMap<TransportType, Arc<dyn Transport>> = HashMap::new();
            transports.insert(TransportType::Udp, udp.clone());
            transports.insert(kind, websocket.clone());
            let mux = MultiplexedTransport::new_without_trace(udp.clone(), transports).unwrap();

            let error = mux
                .send_message(
                    Message::Response(Response::new(StatusCode::Ok)),
                    "127.0.0.1:5090".parse().unwrap(),
                )
                .await
                .expect_err("flowless response must fail closed");
            assert!(matches!(error, TransportError::InvalidState(_)));

            assert_eq!(
                websocket.count(),
                0,
                "{kind} must not be selected by address"
            );
            assert_eq!(
                udp.count(),
                0,
                "UDP also requires an explicit response route"
            );
        }
    }

    #[tokio::test]
    async fn explicit_udp_response_and_cached_bytes_ignore_coexisting_stream_flow() {
        use rvoip_sip_core::{Response, StatusCode};

        // `default` is deliberately not present in the registry. A TCP flow
        // to the same peer must not steal a response that ingress explicitly
        // bound to UDP.
        let udp = CountingTransport::new("udp");
        let tcp = CountingTransport::new("tcp");
        tcp.set_has_connection(true);
        let mut transports: HashMap<TransportType, Arc<dyn Transport>> = HashMap::new();
        transports.insert(TransportType::Tcp, tcp.clone());
        let mux = MultiplexedTransport::new_without_trace(udp.clone(), transports).unwrap();
        let destination = "127.0.0.1:5091".parse().unwrap();
        let route = TransportRoute::new(destination).with_transport_type(TransportType::Udp);

        mux.send_message_via(
            Message::Response(Response::new(StatusCode::Ok)),
            route.clone(),
        )
        .await
        .unwrap();
        mux.send_message_raw_via(Bytes::from_static(b"SIP/2.0 200 OK\r\n\r\n"), route)
            .await
            .unwrap();

        assert_eq!(udp.count(), 1);
        assert_eq!(udp.raw_message_count(), 1);
        assert_eq!(tcp.count(), 0);
        assert_eq!(tcp.raw_message_count(), 0);
    }

    #[tokio::test]
    async fn exact_websocket_flow_routes_structured_and_cached_raw_responses() {
        use rvoip_sip_core::builder::SimpleResponseBuilder;
        use rvoip_sip_core::StatusCode;
        use rvoip_sip_transport::WebSocketTransport;
        use tokio::time::{timeout, Duration};

        let (server_ws, mut server_events) =
            WebSocketTransport::bind("127.0.0.1:0".parse().unwrap(), false, None, None, None)
                .await
                .unwrap();
        let server_addr = server_ws.local_addr().unwrap();
        let server_ws = Arc::new(server_ws);
        let udp = CountingTransport::new("udp");
        let mut transports: HashMap<TransportType, Arc<dyn Transport>> = HashMap::new();
        transports.insert(TransportType::Udp, udp.clone());
        transports.insert(TransportType::Ws, server_ws.clone());
        let mux = MultiplexedTransport::new_without_trace(udp.clone(), transports).unwrap();

        let (client_ws, mut client_events) =
            WebSocketTransport::bind("127.0.0.1:0".parse().unwrap(), false, None, None, None)
                .await
                .unwrap();
        let request = match make_invite("sip:bob@ws-authority.example;transport=ws") {
            Message::Request(request) => request,
            Message::Response(_) => unreachable!(),
        };
        client_ws
            .send_message(Message::Request(request.clone()), server_addr)
            .await
            .unwrap();

        let (source, flow_id) = match timeout(Duration::from_secs(2), server_events.recv())
            .await
            .unwrap()
            .unwrap()
        {
            rvoip_sip_transport::TransportEvent::MessageReceived {
                source,
                flow_id: Some(flow_id),
                ..
            } => (source, flow_id),
            event => panic!("expected flow-bearing WS request, got {event:?}"),
        };
        assert!(server_ws.has_connection_to(source));
        let route = TransportRoute::new(source)
            .with_transport_type(TransportType::Ws)
            .with_flow_id(flow_id);
        let response =
            SimpleResponseBuilder::response_from_request(&request, StatusCode::Ok, Some("OK"))
                .build();

        mux.send_message_via(Message::Response(response.clone()), route.clone())
            .await
            .unwrap();
        assert!(matches!(
            timeout(Duration::from_secs(2), client_events.recv())
                .await
                .unwrap()
                .unwrap(),
            rvoip_sip_transport::TransportEvent::MessageReceived {
                message: Message::Response(_),
                ..
            }
        ));

        mux.send_message_raw_via(Bytes::from(Message::Response(response).to_bytes()), route)
            .await
            .unwrap();
        assert!(matches!(
            timeout(Duration::from_secs(2), client_events.recv())
                .await
                .unwrap()
                .unwrap(),
            rvoip_sip_transport::TransportEvent::MessageReceived {
                message: Message::Response(_),
                ..
            }
        ));
        assert_eq!(udp.count(), 0);

        client_ws.close().await.unwrap();
        server_ws.close().await.unwrap();
    }

    #[tokio::test]
    async fn send_raw_errors_when_no_live_connection_exists() {
        let udp = CountingTransport::new("udp");
        let tcp = CountingTransport::new("tcp");
        // No transport reports a live connection.

        let mut by_flavour: HashMap<TransportType, Arc<dyn Transport>> = HashMap::new();
        by_flavour.insert(TransportType::Udp, udp.clone() as Arc<dyn Transport>);
        by_flavour.insert(TransportType::Tcp, tcp.clone() as Arc<dyn Transport>);

        let mux =
            MultiplexedTransport::new(udp.clone() as Arc<dyn Transport>, by_flavour, None).unwrap();

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
    async fn send_raw_rejects_ambiguous_coaddressed_live_flows() {
        let udp = CountingTransport::new("udp");
        let tcp = CountingTransport::new("tcp");
        let tls = CountingTransport::new("tls");
        tcp.set_has_connection(true);
        tls.set_has_connection(true);

        let mut by_flavour: HashMap<TransportType, Arc<dyn Transport>> = HashMap::new();
        by_flavour.insert(TransportType::Udp, udp.clone() as Arc<dyn Transport>);
        by_flavour.insert(TransportType::Tcp, tcp.clone() as Arc<dyn Transport>);
        by_flavour.insert(TransportType::Tls, tls.clone() as Arc<dyn Transport>);

        let mux =
            MultiplexedTransport::new(udp.clone() as Arc<dyn Transport>, by_flavour, None).unwrap();

        let dest: SocketAddr = "127.0.0.1:5061".parse().unwrap();
        let error = mux
            .send_raw(dest, Bytes::from_static(b"\r\n\r\n"))
            .await
            .expect_err("co-addressed flows require an exact route");
        assert!(matches!(error, TransportError::InvalidState(_)));

        assert_eq!(tls.raw_count(), 0);
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

        let mux =
            MultiplexedTransport::new(udp.clone() as Arc<dyn Transport>, by_flavour, None).unwrap();

        let dest: SocketAddr = "127.0.0.1:5060".parse().unwrap();
        let msg = make_invite("sip:bob@example.com;transport=tcp");

        mux.send_message(msg, dest).await.unwrap();
        // Default (UDP) handled the send; counted twice would mean the
        // dispatcher delivered to both registry-UDP and default-UDP — we
        // expect exactly one delivery (registry hit OR default fallback).
        assert_eq!(udp.count(), 1);
    }

    #[tokio::test]
    async fn dispatch_fails_when_tls_required_but_unavailable() {
        let udp = CountingTransport::new("udp");

        let mut by_flavour: HashMap<TransportType, Arc<dyn Transport>> = HashMap::new();
        by_flavour.insert(TransportType::Udp, udp.clone() as Arc<dyn Transport>);

        let mux =
            MultiplexedTransport::new(udp.clone() as Arc<dyn Transport>, by_flavour, None).unwrap();

        let dest: SocketAddr = "127.0.0.1:5061".parse().unwrap();
        let result = mux
            .send_message(make_invite("sips:bob@example.com"), dest)
            .await;

        assert!(matches!(
            result,
            Err(TransportError::UnsupportedTransport(_))
        ));
        assert_eq!(udp.count(), 0, "sips: must not fall back to UDP");
    }
}
