//! RFC 3261 proxy request-validation and route-processing primitives.
//!
//! This module deliberately contains no network I/O.  It turns an inbound
//! request plus an application-selected URI target into the exact request and
//! next-hop URI that the resolver/transport layer must use.  Keeping the
//! transformations pure makes the strict-routing and SIPS invariants directly
//! packet-testable.

use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use rvoip_sip_core::parser::headers::route::RouteEntry;
use rvoip_sip_core::types::content_length::ContentLength;
use rvoip_sip_core::types::param::Param;
use rvoip_sip_core::types::record_route::{RecordRoute, RecordRouteEntry};
use rvoip_sip_core::types::route::Route;
use rvoip_sip_core::types::unsupported::Unsupported;
use rvoip_sip_core::types::uri::{Host, Scheme};
use rvoip_sip_core::{Address, HeaderName, Method, Request, TypedHeader, Uri};
use rvoip_sip_transport::resolver::{HickoryResolver, ResolvedTarget, Resolver, ResolverError};
use rvoip_sip_transport::transport::TransportType;

/// A URI target selected by the proxy's location/routing policy.
///
/// `route_set` is ordered from the first proxy to visit through the last.  It
/// may contain a strict router for interoperability; local policy routes added
/// by a modern proxy should carry `;lr`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyTarget {
    pub uri: Uri,
    pub route_set: Vec<Uri>,
}

impl ProxyTarget {
    pub fn new(uri: Uri) -> Self {
        Self {
            uri,
            route_set: Vec::new(),
        }
    }

    pub fn with_route_set(mut self, route_set: Vec<Uri>) -> Self {
        self.route_set = route_set;
        self
    }
}

/// Record-Route identities advertised on the insecure and secure sides of a
/// proxy.  Both URIs must resolve back to this proxy and contain `;lr`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordRoutePolicy {
    pub sip_uri: Uri,
    pub sips_uri: Uri,
}

impl RecordRoutePolicy {
    pub fn new(sip_uri: Uri, sips_uri: Uri) -> Result<Self, RoutingPolicyError> {
        validate_record_route_uri(&sip_uri, false)?;
        validate_record_route_uri(&sips_uri, true)?;
        Ok(Self { sip_uri, sips_uri })
    }
}

/// Static routing policy owned by one proxy process.
#[derive(Debug, Clone, Default)]
pub struct ProxyRoutingPolicy {
    /// SIP/SIPS URIs that identify this proxy in Request-URI or Route values.
    pub local_uris: Vec<Uri>,
    /// Proxy-sensitive option tags implemented by this proxy.
    pub supported_proxy_require: HashSet<String>,
    /// Optional policy for remaining on the dialog route.
    pub record_route: Option<RecordRoutePolicy>,
}

impl ProxyRoutingPolicy {
    pub fn new(local_uris: Vec<Uri>) -> Result<Self, RoutingPolicyError> {
        if local_uris
            .iter()
            .any(|uri| !matches!(uri.scheme(), Scheme::Sip | Scheme::Sips))
        {
            return Err(RoutingPolicyError::InvalidLocalUri);
        }
        Ok(Self {
            local_uris,
            supported_proxy_require: HashSet::new(),
            record_route: None,
        })
    }

    pub fn with_supported_proxy_require(
        mut self,
        tags: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.supported_proxy_require = tags
            .into_iter()
            .map(|tag| tag.into().to_ascii_lowercase())
            .collect();
        self
    }

    pub fn with_record_route(mut self, record_route: RecordRoutePolicy) -> Self {
        self.record_route = Some(record_route);
        self
    }

    /// Revalidate a policy after deserialization or mutation of its public
    /// configuration fields.
    ///
    /// Constructors validate their immediate inputs, but [`ProxyRoutingPolicy`]
    /// is deliberately configuration-friendly.  The live proxy calls this
    /// method before accepting a policy so invalid Record-Route identities can
    /// never reach the wire.
    pub fn validate(&self) -> Result<(), RoutingPolicyError> {
        if self
            .local_uris
            .iter()
            .any(|uri| !matches!(uri.scheme(), Scheme::Sip | Scheme::Sips))
        {
            return Err(RoutingPolicyError::InvalidLocalUri);
        }
        if let Some(record_route) = &self.record_route {
            validate_record_route_uri(&record_route.sip_uri, false)?;
            validate_record_route_uri(&record_route.sips_uri, true)?;
        }
        Ok(())
    }

    fn is_local_uri(&self, uri: &Uri) -> bool {
        self.local_uris
            .iter()
            .any(|local| proxy_uri_matches(local, uri))
    }

    fn is_record_route_uri(&self, uri: &Uri) -> bool {
        self.record_route.as_ref().is_some_and(|record_route| {
            proxy_uri_matches(&record_route.sip_uri, uri)
                || proxy_uri_matches(&record_route.sips_uri, uri)
        })
    }
}

/// Lazy process-local RFC 3263 resolver used by the compatibility
/// constructors on [`crate::StatefulProxy`].
///
/// Creating a proxy that only uses the legacy pre-resolved `SocketAddr` API
/// must not touch DNS.  The system resolver is therefore initialized on the
/// first URI target, with a bounded configuration-read delay.  Applications
/// that need deterministic routing should inject their own [`Resolver`].
#[derive(Default)]
pub struct DefaultProxyResolver {
    inner: tokio::sync::OnceCell<Arc<HickoryResolver>>,
}

impl std::fmt::Debug for DefaultProxyResolver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DefaultProxyResolver")
            .field("initialized", &self.inner.initialized())
            .finish()
    }
}

#[async_trait::async_trait]
impl Resolver for DefaultProxyResolver {
    async fn resolve(&self, uri: &Uri) -> Result<Vec<ResolvedTarget>, ResolverError> {
        // Keep literal and localhost targets independent of host DNS
        // configuration.  These paths are especially important for local
        // conformance suites and disposable container deployments.
        if let Host::Address(address) = &uri.host {
            let transport = rvoip_sip_transport::resolver::select_transport_for_uri(uri);
            let port = uri.port.unwrap_or_else(|| default_port(transport));
            return Ok(vec![ResolvedTarget::immediate(
                SocketAddr::new(*address, port),
                transport,
            )]);
        }
        if matches!(&uri.host, Host::Domain(domain) if domain.eq_ignore_ascii_case("localhost")) {
            let transport = rvoip_sip_transport::resolver::select_transport_for_uri(uri);
            let port = uri.port.unwrap_or_else(|| default_port(transport));
            return Ok(vec![ResolvedTarget::immediate(
                SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
                transport,
            )]);
        }

        let resolver = self
            .inner
            .get_or_init(|| async {
                Arc::new(HickoryResolver::new_system_resilient(Duration::from_millis(250)).await)
            })
            .await;
        resolver.resolve(uri).await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingPolicyError {
    InvalidLocalUri,
    InvalidRecordRouteUri(&'static str),
}

impl std::fmt::Display for RoutingPolicyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidLocalUri => {
                formatter.write_str("local proxy identities must use the sip or sips URI scheme")
            }
            Self::InvalidRecordRouteUri(reason) => {
                write!(formatter, "invalid Record-Route URI: {reason}")
            }
        }
    }
}

impl std::error::Error for RoutingPolicyError {}

/// A local RFC 3261 §16.3 rejection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestRejection {
    UnsupportedUriScheme,
    UnsupportedProxyRequire(Vec<String>),
    MalformedProxyRequire,
    MalformedRoute,
}

/// Output of target-specific §16.6 request construction.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedTarget {
    pub request: Request,
    /// URI used as input to RFC 3263 resolution after strict-routing
    /// postprocessing.
    pub next_hop_uri: Uri,
}

/// Validate the URI scheme and Proxy-Require option tags required by RFC 3261
/// §16.3.  CANCEL is exempt from Proxy-Require processing by §8.2.2.3.
pub fn validate_request(
    request: &Request,
    policy: &ProxyRoutingPolicy,
) -> Result<(), RequestRejection> {
    if !matches!(request.uri().scheme(), Scheme::Sip | Scheme::Sips) {
        return Err(RequestRejection::UnsupportedUriScheme);
    }

    if request.method() == Method::Cancel {
        return Ok(());
    }

    let mut unsupported = Vec::new();
    let mut seen = HashSet::new();
    for header in &request.headers {
        if !header.name().wire_eq(&HeaderName::ProxyRequire) {
            continue;
        }
        let TypedHeader::ProxyRequire(required) = header else {
            return Err(RequestRejection::MalformedProxyRequire);
        };
        for tag in required.options() {
            let normalized = tag.to_ascii_lowercase();
            if !policy.supported_proxy_require.contains(&normalized) && seen.insert(normalized) {
                unsupported.push(tag.clone());
            }
        }
    }

    if unsupported.is_empty() {
        Ok(())
    } else {
        Err(RequestRejection::UnsupportedProxyRequire(unsupported))
    }
}

/// Build the `Unsupported` header required on a 420 rejection.
pub fn unsupported_header(tags: Vec<String>) -> TypedHeader {
    TypedHeader::Unsupported(Unsupported::with_tags(tags))
}

/// Apply RFC 3261 §16.4 route-information preprocessing.
///
/// This handles the strict-router compatibility case first, then removes a
/// top Route value that identifies this proxy.  `maddr` processing is kept at
/// the transport-aware caller because stripping it also depends on the exact
/// ingress port and transport.
pub fn preprocess_inbound_route(
    request: &mut Request,
    policy: &ProxyRoutingPolicy,
) -> Result<(), RequestRejection> {
    if policy.is_record_route_uri(request.uri()) {
        if let Some(last) = pop_last_route_uri(request)? {
            request.uri = last;
        }
    }

    if first_route_uri(request)?
        .as_ref()
        .is_some_and(|uri| policy.is_local_uri(uri))
    {
        let _ = pop_first_route_uri(request)?;
    }
    Ok(())
}

/// Apply the transport-dependent `maddr` rule from RFC 3261 §16.4.
///
/// The parameter is stripped only when it identifies this proxy *and* the
/// request arrived on the port and transport selected by the Request-URI.
/// Otherwise it must remain so normal target selection forwards the request to
/// that explicitly requested hop.
pub fn preprocess_local_maddr(
    request: &mut Request,
    policy: &ProxyRoutingPolicy,
    ingress_local_addr: SocketAddr,
    ingress_transport: TransportType,
) -> bool {
    let Some(maddr) = request.uri().parameters.iter().find_map(|parameter| {
        if let Param::Maddr(value) = parameter {
            Some(value.clone())
        } else if let Param::Other(name, Some(value)) = parameter {
            name.eq_ignore_ascii_case("maddr")
                .then(|| value.to_string())
        } else {
            None
        }
    }) else {
        return false;
    };

    let local_host = normalize_host_text(&maddr);
    let owns_maddr = policy
        .local_uris
        .iter()
        .any(|uri| normalized_host(&uri.host) == local_host)
        || normalize_host_text(&ingress_local_addr.ip().to_string()) == local_host;
    let indicated_transport =
        rvoip_sip_transport::resolver::select_transport_for_uri(request.uri());
    let indicated_port = request
        .uri()
        .port
        .unwrap_or_else(|| default_port(indicated_transport));
    if !owns_maddr
        || indicated_transport != ingress_transport
        || indicated_port != ingress_local_addr.port()
    {
        return false;
    }

    request.uri.parameters.retain(|parameter| {
        !matches!(parameter, Param::Maddr(_))
            && !matches!(
                parameter,
                Param::Other(name, _) if name.eq_ignore_ascii_case("maddr")
            )
            && !matches!(parameter, Param::Transport(_))
    });
    request.uri.port = None;
    true
}

/// Construct one target-specific forwarded request following RFC 3261 §16.6
/// steps 1, 2, 4, 6, and 7.  Max-Forwards, Via, and Content-Length are applied
/// by the live transport path because Via depends on the selected concrete
/// transport.
pub fn prepare_target(
    received: &Request,
    target: &ProxyTarget,
    policy: &ProxyRoutingPolicy,
) -> Result<PreparedTarget, RequestRejection> {
    if !matches!(target.uri.scheme(), Scheme::Sip | Scheme::Sips) {
        return Err(RequestRejection::UnsupportedUriScheme);
    }
    let mut request = received.clone();
    request.uri = request_uri_from_target(&target.uri);

    if !target.route_set.is_empty() {
        prepend_route_set(&mut request, &target.route_set);
    }

    if let Some(record_route) = &policy.record_route {
        let secure = request_requires_secure_routing(&request)?;
        let uri = if secure {
            record_route.sips_uri.clone()
        } else {
            record_route.sip_uri.clone()
        };
        prepend_record_route(&mut request, uri);
    }

    let strict_routed = apply_strict_routing(&mut request)?;
    let mut next_hop_uri = if strict_routed {
        request.uri().clone()
    } else {
        first_route_uri(&request)?.unwrap_or_else(|| request.uri().clone())
    };

    // RFC 3261 §16.6 step 7: a SIPS Request-URI remains an end-to-end
    // security requirement even when a loose `sip:` Route identifies the
    // immediate hop.
    if matches!(request.uri().scheme(), Scheme::Sips) || matches!(target.uri.scheme(), Scheme::Sips)
    {
        next_hop_uri.scheme = Scheme::Sips;
    }

    ensure_content_length(&mut request);

    Ok(PreparedTarget {
        request,
        next_hop_uri,
    })
}

/// Whether the effective target or top Route requires a SIPS Record-Route.
pub fn request_requires_secure_routing(request: &Request) -> Result<bool, RequestRejection> {
    Ok(matches!(request.uri().scheme(), Scheme::Sips)
        || first_route_uri(request)?
            .as_ref()
            .is_some_and(|uri| matches!(uri.scheme(), Scheme::Sips)))
}

/// Recreate the shared branch base for a recursive redirect without resetting
/// request processing or weakening a SIPS requirement established by the
/// redirecting leg.
pub(crate) fn prepare_redirect_request(
    forwarding_request: &Request,
    redirecting_leg: &Request,
) -> Request {
    let mut request = forwarding_request.clone();
    if request_requires_secure_routing(redirecting_leg).unwrap_or(true) {
        request.uri.scheme = Scheme::Sips;
    }
    request
}

fn validate_record_route_uri(uri: &Uri, secure: bool) -> Result<(), RoutingPolicyError> {
    if secure != matches!(uri.scheme(), Scheme::Sips) {
        return Err(RoutingPolicyError::InvalidRecordRouteUri(
            "scheme does not match the secure side",
        ));
    }
    if !uri
        .parameters
        .iter()
        .any(|param| matches!(param, Param::Lr))
    {
        return Err(RoutingPolicyError::InvalidRecordRouteUri(
            "the lr parameter is required",
        ));
    }
    if !uri.headers.is_empty() {
        return Err(RoutingPolicyError::InvalidRecordRouteUri(
            "URI headers are not allowed",
        ));
    }
    if uri
        .parameters
        .iter()
        .any(|parameter| matches!(parameter, Param::Method(_) | Param::Ttl(_)))
    {
        return Err(RoutingPolicyError::InvalidRecordRouteUri(
            "method and ttl parameters are not allowed",
        ));
    }
    Ok(())
}

fn proxy_uri_matches(local: &Uri, candidate: &Uri) -> bool {
    if local.scheme() != candidate.scheme()
        || local.user != candidate.user
        || local.password != candidate.password
        || normalized_host(&local.host) != normalized_host(&candidate.host)
        // RFC 3261 section 19.1.4 deliberately distinguishes an omitted
        // port from an explicitly supplied default port.
        || local.port != candidate.port
        || normalized_uri_headers(local) != normalized_uri_headers(candidate)
    {
        return false;
    }

    uri_parameters_match(local, candidate)
}

fn normalized_host(host: &Host) -> String {
    match host {
        Host::Domain(domain) => domain.trim_end_matches('.').to_ascii_lowercase(),
        Host::Address(address) => address.to_string(),
    }
}

fn normalize_host_text(host: &str) -> String {
    host.trim_matches(['[', ']'])
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

fn default_port(transport: TransportType) -> u16 {
    match transport {
        TransportType::Tls | TransportType::Wss => 5061,
        TransportType::Udp | TransportType::Tcp | TransportType::Ws => 5060,
    }
}

fn normalized_uri_headers(uri: &Uri) -> Vec<(String, String)> {
    let mut headers: Vec<_> = uri
        .headers
        .iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
        .collect();
    headers.sort();
    headers
}

fn normalized_uri_parameters(uri: &Uri) -> std::collections::HashMap<String, String> {
    uri.parameters
        .iter()
        .map(|parameter| {
            let wire = parameter.to_string();
            let (name, value) = wire
                .split_once('=')
                .map_or((wire.as_str(), ""), |(name, value)| (name, value));
            (name.to_ascii_lowercase(), value.to_ascii_lowercase())
        })
        .collect()
}

fn uri_parameters_match(local: &Uri, candidate: &Uri) -> bool {
    let local = normalized_uri_parameters(local);
    let candidate = normalized_uri_parameters(candidate);

    for (name, local_value) in &local {
        if let Some(candidate_value) = candidate.get(name) {
            if local_value != candidate_value {
                return false;
            }
        } else if parameter_must_appear_in_both(name) {
            return false;
        }
    }
    for name in candidate.keys() {
        if !local.contains_key(name) && parameter_must_appear_in_both(name) {
            return false;
        }
    }
    true
}

fn parameter_must_appear_in_both(name: &str) -> bool {
    matches!(name, "transport" | "user" | "ttl" | "method" | "maddr")
}

/// Form the Request-URI from a policy-selected target.
///
/// RFC 3261 section 16.6 step 2 and Table 1 prohibit the `method`
/// parameter and URI header component in a Request-URI. They are target
/// construction instructions, not routing identity that may be copied onto
/// the request line. Bridgefu/rvoip routing policy owns the SIP method and
/// message headers, so the proxy deliberately ignores those instructions.
fn request_uri_from_target(target: &Uri) -> Uri {
    let mut uri = target.clone();
    uri.parameters.retain(|parameter| {
        !matches!(parameter, Param::Method(_))
            && !matches!(
                parameter,
                Param::Other(name, _) if name.eq_ignore_ascii_case("method")
            )
    });
    uri.headers.clear();
    uri
}

fn ensure_content_length(request: &mut Request) {
    if request
        .headers
        .iter()
        .any(|header| header.name().wire_eq(&HeaderName::ContentLength))
    {
        return;
    }
    request
        .headers
        .push(TypedHeader::ContentLength(ContentLength::new(
            request.body().len() as u32,
        )));
}

fn first_route_uri(request: &Request) -> Result<Option<Uri>, RequestRejection> {
    for header in &request.headers {
        if !header.name().wire_eq(&HeaderName::Route) {
            continue;
        }
        let TypedHeader::Route(route) = header else {
            return Err(RequestRejection::MalformedRoute);
        };
        return route
            .first()
            .map(|entry| route_entry_uri(entry))
            .transpose();
    }
    Ok(None)
}

fn route_entry_uri(entry: &RouteEntry) -> Result<Uri, RequestRejection> {
    let mut uri = entry.0.uri.clone();
    for param in &entry.0.params {
        uri.parameters.push(param.clone());
    }
    if matches!(uri.scheme(), Scheme::Sip | Scheme::Sips) {
        Ok(uri)
    } else {
        Err(RequestRejection::UnsupportedUriScheme)
    }
}

fn pop_first_route_uri(request: &mut Request) -> Result<Option<Uri>, RequestRejection> {
    let Some(header_index) = request
        .headers
        .iter()
        .position(|header| header.name().wire_eq(&HeaderName::Route))
    else {
        return Ok(None);
    };
    let TypedHeader::Route(route) = &mut request.headers[header_index] else {
        return Err(RequestRejection::MalformedRoute);
    };
    if route.0.is_empty() {
        return Err(RequestRejection::MalformedRoute);
    }
    let entry = route.0.remove(0);
    if route.0.is_empty() {
        request.headers.remove(header_index);
    }
    route_entry_uri(&entry).map(Some)
}

fn pop_last_route_uri(request: &mut Request) -> Result<Option<Uri>, RequestRejection> {
    let Some(header_index) = request
        .headers
        .iter()
        .rposition(|header| header.name().wire_eq(&HeaderName::Route))
    else {
        return Ok(None);
    };
    let TypedHeader::Route(route) = &mut request.headers[header_index] else {
        return Err(RequestRejection::MalformedRoute);
    };
    let Some(entry) = route.0.pop() else {
        return Err(RequestRejection::MalformedRoute);
    };
    if route.0.is_empty() {
        request.headers.remove(header_index);
    }
    route_entry_uri(&entry).map(Some)
}

fn append_route_uri(request: &mut Request, uri: Uri) {
    if let Some(TypedHeader::Route(route)) = request
        .headers
        .iter_mut()
        .rfind(|header| header.name().wire_eq(&HeaderName::Route))
    {
        route.add_uri(uri);
    } else {
        request
            .headers
            .push(TypedHeader::Route(Route::with_uri(uri)));
    }
}

fn prepend_route_set(request: &mut Request, route_set: &[Uri]) {
    let route = Route::new(
        route_set
            .iter()
            .cloned()
            .map(|uri| RouteEntry(Address::new(uri)))
            .collect(),
    );
    let index = request
        .headers
        .iter()
        .position(|header| header.name().wire_eq(&HeaderName::Route))
        .unwrap_or(request.headers.len());
    request.headers.insert(index, TypedHeader::Route(route));
}

fn prepend_record_route(request: &mut Request, uri: Uri) {
    let record_route = RecordRoute::new(vec![RecordRouteEntry::new(Address::new(uri))]);
    let index = request
        .headers
        .iter()
        .position(|header| header.name().wire_eq(&HeaderName::RecordRoute))
        .unwrap_or(request.headers.len());
    request
        .headers
        .insert(index, TypedHeader::RecordRoute(record_route));
}

fn apply_strict_routing(request: &mut Request) -> Result<bool, RequestRejection> {
    let Some(first) = first_route_uri(request)? else {
        return Ok(false);
    };
    if first
        .parameters
        .iter()
        .any(|param| matches!(param, Param::Lr))
    {
        return Ok(false);
    }

    let original_uri = request.uri().clone();
    request.uri = pop_first_route_uri(request)?.ok_or(RequestRejection::MalformedRoute)?;
    append_route_uri(request, original_uri);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use rvoip_sip_core::types::headers::HeaderValue;
    use rvoip_sip_core::types::proxy_require::ProxyRequire;

    use super::*;

    fn uri(value: &str) -> Uri {
        Uri::from_str(value).expect("valid URI")
    }

    fn policy() -> ProxyRoutingPolicy {
        ProxyRoutingPolicy::new(vec![
            uri("sip:proxy.example.com:5060;transport=udp;lr"),
            uri("sips:proxy.example.com:5061;transport=tls;lr"),
        ])
        .unwrap()
    }

    #[test]
    fn rejects_unknown_scheme_and_unsupported_proxy_require() {
        let mut request = Request::new(Method::Invite, uri("tel:+12065550100"));
        assert_eq!(
            validate_request(&request, &policy()),
            Err(RequestRejection::UnsupportedUriScheme)
        );

        request.uri = uri("sip:bob@example.net");
        request
            .headers
            .push(TypedHeader::ProxyRequire(ProxyRequire::with_options(&[
                "timer", "Foo", "foo",
            ])));
        let policy = policy().with_supported_proxy_require(["timer"]);
        assert_eq!(
            validate_request(&request, &policy),
            Err(RequestRejection::UnsupportedProxyRequire(vec![
                "Foo".to_string()
            ]))
        );
    }

    #[test]
    fn cancel_ignores_proxy_require() {
        let request = Request::new(Method::Cancel, uri("sip:bob@example.net"))
            .with_header(TypedHeader::ProxyRequire(ProxyRequire::single("unknown")));
        assert_eq!(validate_request(&request, &policy()), Ok(()));
    }

    #[test]
    fn preprocesses_strict_router_request_and_removes_own_route() {
        let policy = policy().with_record_route(
            RecordRoutePolicy::new(
                uri("sip:proxy.example.com:5060;transport=udp;lr"),
                uri("sips:proxy.example.com:5061;transport=tls;lr"),
            )
            .unwrap(),
        );
        let mut request = Request::new(
            Method::Invite,
            uri("sip:proxy.example.com:5060;transport=udp"),
        )
        .with_header(TypedHeader::Route(
            Route::from_str("<sip:next.example.net;lr>, <sip:bob@destination.example.net>")
                .unwrap(),
        ));

        preprocess_inbound_route(&mut request, &policy).unwrap();
        assert_eq!(request.uri(), &uri("sip:bob@destination.example.net"));
        assert_eq!(
            first_route_uri(&request).unwrap(),
            Some(uri("sip:next.example.net;lr"))
        );

        let mut request =
            Request::new(Method::Bye, uri("sip:bob@example.net")).with_header(TypedHeader::Route(
                Route::from_str(
                    "<SIP:PROXY.EXAMPLE.COM:5060;transport=udp;lr>, <sip:next.example.net;lr>",
                )
                .unwrap(),
            ));
        preprocess_inbound_route(&mut request, &policy).unwrap();
        assert_eq!(
            first_route_uri(&request).unwrap(),
            Some(uri("sip:next.example.net;lr"))
        );
    }

    #[test]
    fn postprocesses_strict_route_without_losing_original_target() {
        let request = Request::new(Method::Invite, uri("sip:original@example.org"));
        let target = ProxyTarget::new(uri("sip:bob@example.net")).with_route_set(vec![
            uri("sip:strict.example.com"),
            uri("sip:loose.example.com;lr"),
        ]);

        let prepared = prepare_target(&request, &target, &policy()).unwrap();
        assert_eq!(prepared.request.uri(), &uri("sip:strict.example.com"));
        assert_eq!(prepared.next_hop_uri, uri("sip:strict.example.com"));
        let route = prepared
            .request
            .headers
            .iter()
            .find_map(|header| match header {
                TypedHeader::Route(route) => Some(route),
                _ => None,
            })
            .unwrap();
        assert_eq!(route.len(), 2);
        assert_eq!(route.0[0].0.uri, uri("sip:loose.example.com;lr"));
        assert_eq!(route.0[1].0.uri, uri("sip:bob@example.net"));
    }

    #[test]
    fn secure_target_forces_secure_resolution_through_plain_loose_route() {
        let request = Request::new(Method::Invite, uri("sips:alice@example.org"));
        let target = ProxyTarget::new(uri("sips:bob@example.net"))
            .with_route_set(vec![uri("sip:edge.example.net;lr")]);

        let prepared = prepare_target(&request, &target, &policy()).unwrap();
        assert_eq!(prepared.request.uri(), &uri("sips:bob@example.net"));
        assert_eq!(prepared.next_hop_uri, uri("sips:edge.example.net;lr"));
    }

    #[test]
    fn record_route_is_validated_selected_and_prepended() {
        assert!(RecordRoutePolicy::new(
            uri("sip:proxy.example.com"),
            uri("sips:proxy.example.com;lr")
        )
        .is_err());

        let policy = policy().with_record_route(
            RecordRoutePolicy::new(
                uri("sip:proxy.example.com;lr"),
                uri("sips:proxy.example.com;lr"),
            )
            .unwrap(),
        );
        let request = Request::new(Method::Invite, uri("sips:alice@example.org")).with_header(
            TypedHeader::RecordRoute(RecordRoute::new(vec![RecordRouteEntry::new(Address::new(
                uri("sips:old.example.net;lr"),
            ))])),
        );
        let prepared = prepare_target(
            &request,
            &ProxyTarget::new(uri("sips:bob@example.net")),
            &policy,
        )
        .unwrap();
        let record_routes: Vec<_> = prepared
            .request
            .headers
            .iter()
            .filter_map(|header| match header {
                TypedHeader::RecordRoute(value) => Some(value),
                _ => None,
            })
            .collect();
        assert_eq!(record_routes.len(), 2);
        assert_eq!(
            record_routes[0].first().unwrap().uri(),
            &uri("sips:proxy.example.com;lr")
        );
        assert_eq!(
            record_routes[1].first().unwrap().uri(),
            &uri("sips:old.example.net;lr")
        );

        assert!(RecordRoutePolicy::new(
            uri("sip:proxy.example.com;lr;method=INVITE"),
            uri("sips:proxy.example.com;lr"),
        )
        .is_err());
        assert!(RecordRoutePolicy::new(
            uri("sip:proxy.example.com;lr"),
            uri("sips:proxy.example.com;lr;ttl=1"),
        )
        .is_err());
    }

    #[test]
    fn target_request_uri_drops_method_and_uri_headers_only() {
        let request = Request::new(Method::Invite, uri("sip:original@example.org"));
        let target = ProxyTarget::new(uri(
            "sip:+12065550100@example.net;user=phone;method=BYE?Subject=ignored",
        ));

        let prepared = prepare_target(&request, &target, &policy()).unwrap();

        assert_eq!(
            prepared.request.uri(),
            &uri("sip:+12065550100@example.net;user=phone")
        );
        assert_eq!(
            prepared.next_hop_uri,
            uri("sip:+12065550100@example.net;user=phone")
        );
    }

    #[test]
    fn target_copy_preserves_body_and_header_value_order_and_adds_content_length() {
        const EXTENSION: &str = "X-Rvoip-Order";
        const BODY: &[u8] = b"\0binary\r\nbody\xff";

        let mut request = Request::new(Method::Message, uri("sip:original@example.org"))
            .with_header(TypedHeader::Other(
                HeaderName::Other(EXTENSION.into()),
                HeaderValue::Raw(b"first".to_vec()),
            ));
        request.headers.push(TypedHeader::Other(
            HeaderName::Other(EXTENSION.into()),
            HeaderValue::Raw(b"second".to_vec()),
        ));
        request = request.with_body(BODY);
        request
            .headers
            .retain(|header| !header.name().wire_eq(&HeaderName::ContentLength));

        let prepared = prepare_target(
            &request,
            &ProxyTarget::new(uri("sip:agent@example.net")),
            &policy(),
        )
        .unwrap();

        assert_eq!(prepared.request.body(), BODY);
        let extension_values: Vec<_> = prepared
            .request
            .headers
            .iter()
            .filter_map(|header| match header {
                TypedHeader::Other(name, HeaderValue::Raw(value))
                    if name.wire_eq(&HeaderName::Other(EXTENSION.into())) =>
                {
                    Some(value.as_slice())
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            extension_values,
            vec![b"first".as_slice(), b"second".as_slice()]
        );
        assert!(prepared.request.headers.iter().any(
            |header| matches!(header, TypedHeader::ContentLength(value) if value.0 == BODY.len() as u32)
        ));
    }

    #[test]
    fn recursive_redirect_reuses_processed_request_and_preserves_sips() {
        let mut forwarding = Request::new(Method::Invite, uri("sip:service@example.net"))
            .with_header(TypedHeader::MaxForwards(
                rvoip_sip_core::types::max_forwards::MaxForwards::new(69),
            ));
        forwarding.headers.push(TypedHeader::Other(
            HeaderName::Other("X-Preserved".into()),
            HeaderValue::Raw(b"value".to_vec()),
        ));
        let secure_leg = Request::new(Method::Invite, uri("sips:agent@secure.example"));

        let redirected = prepare_redirect_request(&forwarding, &secure_leg);

        assert_eq!(redirected.uri().scheme(), &Scheme::Sips);
        assert_eq!(
            redirected.headers.iter().find_map(|header| match header {
                TypedHeader::MaxForwards(value) => Some(value.0),
                _ => None,
            }),
            Some(69)
        );
        assert!(redirected.headers.iter().any(|header| {
            matches!(
                header,
                TypedHeader::Other(name, HeaderValue::Raw(value))
                    if name.wire_eq(&HeaderName::Other("X-Preserved".into()))
                        && value == b"value"
            )
        }));
    }

    #[test]
    fn local_uri_recognition_uses_rfc_uri_comparison_rules() {
        let policy = ProxyRoutingPolicy::new(vec![uri("sip:proxy.example.com:5060;lr")]).unwrap();

        let mut explicit_default_differs = Request::new(Method::Bye, uri("sip:agent@example.net"))
            .with_header(TypedHeader::Route(
                Route::from_str("<sip:proxy.example.com;lr>, <sip:next.example.net;lr>").unwrap(),
            ));
        preprocess_inbound_route(&mut explicit_default_differs, &policy).unwrap();
        assert_eq!(
            first_route_uri(&explicit_default_differs).unwrap(),
            Some(uri("sip:proxy.example.com;lr")),
            "an omitted port must not equal an explicitly supplied default port"
        );

        let mut ignored_extension_parameter =
            Request::new(Method::Bye, uri("sip:agent@example.net")).with_header(
                TypedHeader::Route(
                    Route::from_str(
                        "<sip:proxy.example.com:5060;lr;extension=one>, <sip:next.example.net;lr>",
                    )
                    .unwrap(),
                ),
            );
        preprocess_inbound_route(&mut ignored_extension_parameter, &policy).unwrap();
        assert_eq!(
            first_route_uri(&ignored_extension_parameter).unwrap(),
            Some(uri("sip:next.example.net;lr")),
            "an extension parameter present in only one URI is ignored for equality"
        );
    }

    #[test]
    fn strips_maddr_only_on_the_matching_local_transport_and_port() {
        let mut request = Request::new(
            Method::Options,
            uri("sip:service@example.net:5070;maddr=proxy.example.com;transport=tcp"),
        );
        assert!(!preprocess_local_maddr(
            &mut request,
            &policy(),
            "192.0.2.10:5070".parse().unwrap(),
            TransportType::Udp,
        ));
        assert!(request.uri().to_string().contains("maddr"));

        assert!(preprocess_local_maddr(
            &mut request,
            &policy(),
            "192.0.2.10:5070".parse().unwrap(),
            TransportType::Tcp,
        ));
        assert_eq!(request.uri(), &uri("sip:service@example.net"));
    }
}
