//! Minimal executable boundary for the external stateful-proxy interop gate.
//!
//! This is intentionally a policy-free lab process. A release harness supplies
//! one transport, one exact next hop, and an advertised Via address. It keeps
//! all application routing outside the proxy implementation while exercising
//! the shipping transport, transaction manager, and `StatefulProxy`.

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rvoip_sip_core::types::uri::{Host, Scheme};
use rvoip_sip_core::{HeaderName, HeaderValue, Message, Method, Request, TypedHeader, Uri};
use rvoip_sip_dialog::transaction::TransactionManager;
use rvoip_sip_proxy::{
    ProxyConfig, ProxyRoutingPolicy, ProxyRuntimeOptions, ProxyTarget, RecordRoutePolicy,
    RouteDecision, RouteFn, StatefulProxy, UriRouteDecision, UriRouteFn,
};
use rvoip_sip_transport::resolver::{HickoryResolver, ResolvedTarget, Resolver, ResolverError};
use rvoip_sip_transport::transport::tls::{
    TlsClientConfig, TlsServerClientAuthConfig, TlsTransport,
};
use rvoip_sip_transport::transport::{
    TcpTransport, TransportAuthority, TransportType, UdpTransport,
};
use rvoip_sip_transport::{Transport, TransportConnectionMetadata, TransportEvent};
use tokio::sync::mpsc;

const INTEROP_SCENARIO_HEADER: &str = "X-Interop-Scenario";
const MAX_INTEROP_TARGETS: usize = 16;
const MIN_INTEROP_TIMER_C_MS: u64 = 10;
const MAX_INTEROP_TIMER_C_MS: u64 = 180_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WireTransport {
    Udp,
    Tcp,
    Tls,
}

impl WireTransport {
    fn parse(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "udp" => Ok(Self::Udp),
            "tcp" => Ok(Self::Tcp),
            "tls" | "sips" => Ok(Self::Tls),
            _ => Err(format!(
                "unsupported transport {value:?}; use udp, tcp, or tls"
            )),
        }
    }

    fn transport_type(self) -> TransportType {
        match self {
            Self::Udp => TransportType::Udp,
            Self::Tcp => TransportType::Tcp,
            Self::Tls => TransportType::Tls,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Udp => "udp",
            Self::Tcp => "tcp",
            Self::Tls => "tls",
        }
    }
}

#[derive(Debug)]
struct Args {
    listen: SocketAddr,
    advertised: SocketAddr,
    target: SocketAddr,
    aux_targets: Vec<SocketAddr>,
    failover_targets: Vec<SocketAddr>,
    dns_server: Option<SocketAddr>,
    rfc3263_uri: Option<Uri>,
    target_authority: Option<String>,
    transport: WireTransport,
    interop_test_mode: bool,
    timer_c_ms: Option<u64>,
    max_response_contexts: Option<usize>,
    max_downstream_transactions: Option<usize>,
    max_branches_per_context: Option<usize>,
    max_stateless_routes: Option<usize>,
    local_uris: Vec<Uri>,
    record_route_sip: Option<Uri>,
    record_route_sips: Option<Uri>,
    certificate: Option<PathBuf>,
    private_key: Option<PathBuf>,
    ca_certificate: Option<PathBuf>,
    client_certificate: Option<PathBuf>,
    client_private_key: Option<PathBuf>,
    client_ca_certificate: Option<PathBuf>,
}

fn usage() -> &'static str {
    concat!(
        "stateful_proxy_interop \\\n",
        "  --listen <ip:port> --advertised <ip:port> --target <ip:port> \\\n",
        "  --transport <udp|tcp|tls> \\\n",
        "  [--target-authority <dns-name>] \\\n",
        "  [--interop-test-mode \\\n",
        "   [--aux-target <ip:port>]... [--failover-target <ip:port>]... \\\n",
        "   [--dns-server <ip:port> --rfc3263-uri <sip-uri-without-port>] \\\n",
        "   [--timer-c-ms <10..180000>] \\\n",
        "   [--max-response-contexts <positive-integer>] \\\n",
        "   [--max-downstream-transactions <positive-integer>] \\\n",
        "   [--max-branches-per-context <positive-integer>] \\\n",
        "   [--max-stateless-routes <positive-integer>] \\\n",
        "   [--local-uri <sip-or-sips-uri>]... \\\n",
        "   [--record-route-sip <sip-uri;lr>] \\\n",
        "   [--record-route-sips <sips-uri;lr>]] \\\n",
        "  [--certificate <pem> --private-key <pem> --ca-certificate <pem> \\\n",
        "   --client-certificate <pem> --client-private-key <pem> \\\n",
        "   --client-ca-certificate <pem>]"
    )
}

fn validate_dns_authority(value: &str) -> Result<(), String> {
    let normalized = value.strip_suffix('.').unwrap_or(value);
    if normalized.is_empty() || normalized.len() > 253 {
        return Err("--target-authority must be a non-empty DNS name".into());
    }
    if matches!(normalized.parse::<Host>(), Ok(Host::Address(_)) | Err(_)) {
        return Err("--target-authority must be a DNS name, not an IP literal".into());
    }
    if !normalized.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            && label
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric)
            && label
                .as_bytes()
                .last()
                .is_some_and(u8::is_ascii_alphanumeric)
    }) {
        return Err("--target-authority is not a valid DNS name".into());
    }
    Ok(())
}

fn validate_uri_transport(uri: &Uri, secure: bool, option: &str) -> Result<(), String> {
    if secure != matches!(uri.scheme(), Scheme::Sips) {
        return Err(format!(
            "{option} requires a {} URI",
            if secure { "sips:" } else { "sip:" }
        ));
    }
    let transport = uri.transport().map(|value| value.to_ascii_lowercase());
    let compatible = if secure {
        matches!(transport.as_deref(), None | Some("tcp" | "tls" | "wss"))
    } else {
        matches!(transport.as_deref(), None | Some("udp" | "tcp" | "ws"))
    };
    if !compatible {
        return Err(format!(
            "{option} URI transport is inconsistent with its scheme"
        ));
    }
    Ok(())
}

fn validate_local_uri(uri: &Uri) -> Result<(), String> {
    match uri.scheme() {
        Scheme::Sip => {
            if !matches!(
                uri.transport()
                    .map(|value| value.to_ascii_lowercase())
                    .as_deref(),
                None | Some("udp" | "tcp" | "tls" | "ws" | "wss")
            ) {
                return Err("--local-uri contains an unsupported transport".into());
            }
        }
        Scheme::Sips => {
            if !matches!(
                uri.transport()
                    .map(|value| value.to_ascii_lowercase())
                    .as_deref(),
                None | Some("tcp" | "tls" | "wss")
            ) {
                return Err("--local-uri sips: identity cannot use an insecure transport".into());
            }
        }
        _ => return Err("--local-uri must use the sip: or sips: scheme".into()),
    }
    Ok(())
}

fn routing_policy(args: &Args) -> Result<ProxyRoutingPolicy, String> {
    let mut policy = ProxyRoutingPolicy::new(args.local_uris.clone())
        .map_err(|error| format!("invalid --local-uri: {error}"))?;
    match (&args.record_route_sip, &args.record_route_sips) {
        (None, None) => {}
        (Some(sip), Some(sips)) => {
            let record_route = RecordRoutePolicy::new(sip.clone(), sips.clone())
                .map_err(|error| format!("invalid Record-Route policy: {error}"))?;
            policy = policy.with_record_route(record_route);
        }
        _ => {
            return Err(
                "--record-route-sip and --record-route-sips must be configured together".into(),
            );
        }
    }
    Ok(policy)
}

fn has_test_controls(args: &Args) -> bool {
    !args.aux_targets.is_empty()
        || !args.failover_targets.is_empty()
        || args.dns_server.is_some()
        || args.rfc3263_uri.is_some()
        || args.timer_c_ms.is_some()
        || args.max_response_contexts.is_some()
        || args.max_downstream_transactions.is_some()
        || args.max_branches_per_context.is_some()
        || args.max_stateless_routes.is_some()
        || !args.local_uris.is_empty()
        || args.record_route_sip.is_some()
        || args.record_route_sips.is_some()
}

fn validate_args(parsed: &Args) -> Result<(), String> {
    if matches!(parsed.transport, WireTransport::Tls)
        && (parsed.certificate.is_none()
            || parsed.private_key.is_none()
            || parsed.ca_certificate.is_none()
            || parsed.client_certificate.is_none()
            || parsed.client_private_key.is_none()
            || parsed.client_ca_certificate.is_none()
            || parsed.target_authority.is_none())
    {
        return Err(format!(
            "TLS requires server identity, outbound trust, outbound client identity, \
             and inbound client trust\n{}",
            usage()
        ));
    }
    if let Some(authority) = parsed.target_authority.as_deref() {
        validate_dns_authority(authority)?;
    }
    if has_test_controls(parsed) && !parsed.interop_test_mode {
        return Err(
            "advanced routing, Timer C, and capacity overrides require --interop-test-mode".into(),
        );
    }
    if matches!(parsed.transport, WireTransport::Tls)
        && (!parsed.aux_targets.is_empty() || !parsed.failover_targets.is_empty())
    {
        return Err(
            "TLS interop keeps one exact target authority; aux/failover targets are unsupported"
                .into(),
        );
    }
    if parsed.aux_targets.len() + 1 > MAX_INTEROP_TARGETS {
        return Err(format!(
            "--aux-target exceeds the {MAX_INTEROP_TARGETS}-target interop limit"
        ));
    }
    if !parsed.failover_targets.is_empty()
        && !(2..=MAX_INTEROP_TARGETS).contains(&parsed.failover_targets.len())
    {
        return Err(format!(
            "--failover-target requires 2..={MAX_INTEROP_TARGETS} ordered candidates"
        ));
    }
    if parsed
        .aux_targets
        .iter()
        .any(|target| target.port() == 0 || target.ip().is_unspecified())
        || parsed
            .failover_targets
            .iter()
            .any(|target| target.port() == 0 || target.ip().is_unspecified())
    {
        return Err("interop targets require a concrete IP and nonzero port".into());
    }
    if parsed
        .aux_targets
        .iter()
        .enumerate()
        .any(|(index, target)| {
            *target == parsed.target || parsed.aux_targets[..index].contains(target)
        })
    {
        return Err("--aux-target values must be unique and differ from --target".into());
    }
    if parsed
        .failover_targets
        .iter()
        .enumerate()
        .any(|(index, target)| parsed.failover_targets[..index].contains(target))
    {
        return Err("--failover-target values must be unique".into());
    }
    match (&parsed.dns_server, &parsed.rfc3263_uri) {
        (None, None) => {}
        (Some(server), Some(uri)) => {
            if !matches!(parsed.transport, WireTransport::Tcp) {
                return Err("--dns-server/--rfc3263-uri currently require TCP interop".into());
            }
            if server.port() == 0 || server.ip().is_unspecified() {
                return Err("--dns-server requires a concrete IP and nonzero port".into());
            }
            if !matches!(uri.scheme(), Scheme::Sip)
                || uri.port.is_some()
                || !matches!(&uri.host, Host::Domain(_))
                || uri
                    .transport()
                    .is_none_or(|transport| !transport.eq_ignore_ascii_case("tcp"))
            {
                return Err(
                    "--rfc3263-uri must be a sip: domain URI without an explicit port and with transport=tcp"
                        .into(),
                );
            }
        }
        _ => {
            return Err("--dns-server and --rfc3263-uri must be configured together".into());
        }
    }
    if let Some(timer_c_ms) = parsed.timer_c_ms {
        if !(MIN_INTEROP_TIMER_C_MS..=MAX_INTEROP_TIMER_C_MS).contains(&timer_c_ms) {
            return Err(format!(
                "--timer-c-ms must be in {MIN_INTEROP_TIMER_C_MS}..={MAX_INTEROP_TIMER_C_MS}"
            ));
        }
    }
    for (name, capacity) in [
        ("--max-response-contexts", parsed.max_response_contexts),
        (
            "--max-downstream-transactions",
            parsed.max_downstream_transactions,
        ),
        (
            "--max-branches-per-context",
            parsed.max_branches_per_context,
        ),
        ("--max-stateless-routes", parsed.max_stateless_routes),
    ] {
        if capacity == Some(0) {
            return Err(format!("{name} must be greater than zero"));
        }
    }
    for uri in &parsed.local_uris {
        validate_local_uri(uri)?;
    }
    if let Some(uri) = &parsed.record_route_sip {
        validate_uri_transport(uri, false, "--record-route-sip")?;
    }
    if let Some(uri) = &parsed.record_route_sips {
        validate_uri_transport(uri, true, "--record-route-sips")?;
    }
    routing_policy(parsed)?;
    Ok(())
}

fn parse_args() -> Result<Args, String> {
    parse_args_from(env::args().skip(1))
}

fn parse_args_from(arguments: impl IntoIterator<Item = impl Into<String>>) -> Result<Args, String> {
    let mut listen = None;
    let mut advertised = None;
    let mut target = None;
    let mut aux_targets = Vec::new();
    let mut failover_targets = Vec::new();
    let mut dns_server = None;
    let mut rfc3263_uri = None;
    let mut target_authority = None;
    let mut transport = None;
    let mut interop_test_mode = false;
    let mut timer_c_ms = None;
    let mut max_response_contexts = None;
    let mut max_downstream_transactions = None;
    let mut max_branches_per_context = None;
    let mut max_stateless_routes = None;
    let mut local_uris = Vec::new();
    let mut record_route_sip = None;
    let mut record_route_sips = None;
    let mut certificate = None;
    let mut private_key = None;
    let mut ca_certificate = None;
    let mut client_certificate = None;
    let mut client_private_key = None;
    let mut client_ca_certificate = None;
    let mut args = arguments.into_iter().map(Into::into);

    while let Some(argument) = args.next() {
        let mut value = || {
            args.next()
                .ok_or_else(|| format!("{argument} requires a value"))
        };
        match argument.as_str() {
            "--listen" => {
                listen = Some(
                    value()?
                        .parse()
                        .map_err(|error| format!("invalid --listen: {error}"))?,
                );
            }
            "--advertised" => {
                advertised = Some(
                    value()?
                        .parse()
                        .map_err(|error| format!("invalid --advertised: {error}"))?,
                );
            }
            "--target" => {
                target = Some(
                    value()?
                        .parse()
                        .map_err(|error| format!("invalid --target: {error}"))?,
                );
            }
            "--aux-target" => {
                aux_targets.push(
                    value()?
                        .parse()
                        .map_err(|error| format!("invalid --aux-target: {error}"))?,
                );
            }
            "--failover-target" => {
                failover_targets.push(
                    value()?
                        .parse()
                        .map_err(|error| format!("invalid --failover-target: {error}"))?,
                );
            }
            "--dns-server" => {
                dns_server = Some(
                    value()?
                        .parse()
                        .map_err(|error| format!("invalid --dns-server: {error}"))?,
                );
            }
            "--rfc3263-uri" => {
                rfc3263_uri = Some(
                    value()?
                        .parse()
                        .map_err(|error| format!("invalid --rfc3263-uri: {error}"))?,
                );
            }
            "--target-authority" => target_authority = Some(value()?),
            "--transport" => transport = Some(WireTransport::parse(&value()?)?),
            "--interop-test-mode" => interop_test_mode = true,
            "--timer-c-ms" => {
                timer_c_ms = Some(
                    value()?
                        .parse()
                        .map_err(|error| format!("invalid --timer-c-ms: {error}"))?,
                );
            }
            "--max-response-contexts" => {
                max_response_contexts = Some(
                    value()?
                        .parse()
                        .map_err(|error| format!("invalid --max-response-contexts: {error}"))?,
                );
            }
            "--max-downstream-transactions" => {
                max_downstream_transactions =
                    Some(value()?.parse().map_err(|error| {
                        format!("invalid --max-downstream-transactions: {error}")
                    })?);
            }
            "--max-branches-per-context" => {
                max_branches_per_context = Some(
                    value()?
                        .parse()
                        .map_err(|error| format!("invalid --max-branches-per-context: {error}"))?,
                );
            }
            "--max-stateless-routes" => {
                max_stateless_routes = Some(
                    value()?
                        .parse()
                        .map_err(|error| format!("invalid --max-stateless-routes: {error}"))?,
                );
            }
            "--local-uri" => {
                local_uris.push(
                    value()?
                        .parse()
                        .map_err(|error| format!("invalid --local-uri: {error}"))?,
                );
            }
            "--record-route-sip" => {
                if record_route_sip.is_some() {
                    return Err("--record-route-sip may be specified only once".into());
                }
                record_route_sip = Some(
                    value()?
                        .parse()
                        .map_err(|error| format!("invalid --record-route-sip: {error}"))?,
                );
            }
            "--record-route-sips" => {
                if record_route_sips.is_some() {
                    return Err("--record-route-sips may be specified only once".into());
                }
                record_route_sips = Some(
                    value()?
                        .parse()
                        .map_err(|error| format!("invalid --record-route-sips: {error}"))?,
                );
            }
            "--certificate" => certificate = Some(PathBuf::from(value()?)),
            "--private-key" => private_key = Some(PathBuf::from(value()?)),
            "--ca-certificate" => ca_certificate = Some(PathBuf::from(value()?)),
            "--client-certificate" => client_certificate = Some(PathBuf::from(value()?)),
            "--client-private-key" => client_private_key = Some(PathBuf::from(value()?)),
            "--client-ca-certificate" => {
                client_ca_certificate = Some(PathBuf::from(value()?));
            }
            "--help" | "-h" => return Err(usage().to_owned()),
            _ => return Err(format!("unknown argument {argument:?}\n{}", usage())),
        }
    }

    let parsed = Args {
        listen: listen.ok_or_else(|| format!("missing --listen\n{}", usage()))?,
        advertised: advertised.ok_or_else(|| format!("missing --advertised\n{}", usage()))?,
        target: target.ok_or_else(|| format!("missing --target\n{}", usage()))?,
        aux_targets,
        failover_targets,
        dns_server,
        rfc3263_uri,
        target_authority,
        transport: transport.ok_or_else(|| format!("missing --transport\n{}", usage()))?,
        interop_test_mode,
        timer_c_ms,
        max_response_contexts,
        max_downstream_transactions,
        max_branches_per_context,
        max_stateless_routes,
        local_uris,
        record_route_sip,
        record_route_sips,
        certificate,
        private_key,
        ca_certificate,
        client_certificate,
        client_private_key,
        client_ca_certificate,
    };

    validate_args(&parsed)?;
    Ok(parsed)
}

fn interop_scenario(request: &Request) -> Option<&str> {
    let expected = HeaderName::Other(INTEROP_SCENARIO_HEADER.into());
    let mut values = request.headers.iter().filter_map(|header| match header {
        TypedHeader::Other(name, HeaderValue::Raw(value)) if name.wire_eq(&expected) => {
            Some(value.as_slice())
        }
        _ => None,
    });
    let raw = values.next()?;
    if values.next().is_some() || raw.len() > 64 {
        return None;
    }
    let value = std::str::from_utf8(raw).ok()?.trim();
    (!value.is_empty() && value.is_ascii()).then_some(value)
}

#[derive(Clone, Debug)]
struct InteropRoutes {
    enabled: bool,
    target: SocketAddr,
    aux_targets: Vec<SocketAddr>,
    failover_targets: Vec<SocketAddr>,
}

impl InteropRoutes {
    fn new(args: &Args) -> Self {
        Self {
            enabled: args.interop_test_mode,
            target: args.target,
            aux_targets: args.aux_targets.clone(),
            failover_targets: args.failover_targets.clone(),
        }
    }

    fn empty_sequential() -> RouteDecision {
        RouteDecision::sequential(Vec::new())
    }

    fn fork_targets(&self) -> Option<Vec<SocketAddr>> {
        (self.aux_targets.len() >= 2).then(|| {
            std::iter::once(self.target)
                .chain(self.aux_targets.iter().copied())
                .collect()
        })
    }

    fn request_uri_target(&self, request: &Request) -> Option<SocketAddr> {
        let Host::Address(address) = &request.uri().host else {
            return None;
        };
        let port = request.uri().port?;
        std::iter::once(self.target)
            .chain(self.aux_targets.iter().copied())
            .chain(self.failover_targets.iter().copied())
            .find(|target| target.ip() == *address && target.port() == port)
    }

    fn select(&self, request: &Request) -> RouteDecision {
        if !self.enabled {
            return RouteDecision::to(self.target);
        }
        // A 2xx ACK is a new transaction and is routed by its Contact-derived
        // Request-URI. Never reapply the original INVITE fork to an ACK.
        if request.method() == Method::Ack {
            return RouteDecision::to(self.request_uri_target(request).unwrap_or(self.target));
        }
        if request.method() != Method::Invite {
            return RouteDecision::to(self.target);
        }

        match interop_scenario(request) {
            Some("sequential-fork") => self
                .fork_targets()
                .map(RouteDecision::sequential)
                .unwrap_or_else(Self::empty_sequential),
            Some(
                "parallel-fork" | "multiple-2xx" | "late-2xx" | "sixxx-cancel" | "auth-aggregation",
            ) => self
                .fork_targets()
                .map(RouteDecision::parallel)
                .unwrap_or_else(Self::empty_sequential),
            Some("transport-failure") => self
                .failover_targets
                .first()
                .copied()
                .map(RouteDecision::to)
                .unwrap_or_else(Self::empty_sequential),
            // The real RFC 3263 scenario is routed through the URI callback
            // and production Hickory resolver. Never relabel an injected
            // SocketAddr list as DNS interoperability evidence.
            Some("rfc3263-failover") => Self::empty_sequential(),
            Some("route-strict" | "route-loose-record-route") => self
                .aux_targets
                .first()
                .copied()
                .map(RouteDecision::to)
                .unwrap_or_else(Self::empty_sequential),
            _ => RouteDecision::to(self.target),
        }
    }
}

fn apply_runtime_controls(
    args: &Args,
    mut options: ProxyRuntimeOptions,
) -> Result<(ProxyConfig, ProxyRuntimeOptions), String> {
    let mut config = ProxyConfig::default();
    if let Some(timer_c_ms) = args.timer_c_ms {
        config.timer_c = Duration::from_millis(timer_c_ms);
        options = options.with_short_timer_c_for_tests();
    }
    if let Some(capacity) = args.max_response_contexts {
        options = options.with_response_context_capacity(capacity);
    }
    if let Some(capacity) = args.max_downstream_transactions {
        options = options.with_downstream_transaction_capacity(capacity);
    }
    if let Some(capacity) = args.max_branches_per_context {
        options = options.with_branches_per_response_context(capacity);
    }
    if let Some(capacity) = args.max_stateless_routes {
        options = options.with_stateless_response_route_capacity(capacity);
    }
    if !args.local_uris.is_empty()
        || args.record_route_sip.is_some()
        || args.record_route_sips.is_some()
    {
        options = options.with_routing_policy(routing_policy(args)?);
    }
    Ok((config, options))
}

#[derive(Debug)]
struct ExactTlsResolver {
    target: SocketAddr,
    authority: TransportAuthority,
}

fn exact_tls_route_target(request: &Request) -> ProxyTarget {
    // The configured TLS socket and authenticated DNS identity are the
    // application-selected next hop, not a replacement for the SIP target.
    // In particular, RFC 3261 §12.2.1.1 requires in-dialog ACK/BYE requests
    // to retain the remote target learned from Contact after loose routes are
    // consumed.
    ProxyTarget::new(request.uri().clone())
}

#[async_trait]
impl Resolver for ExactTlsResolver {
    async fn resolve(&self, _uri: &Uri) -> Result<Vec<ResolvedTarget>, ResolverError> {
        // This executable models an administratively configured outbound
        // proxy. The fixed socket and TLS DNS authority select and authenticate
        // that next hop independently of the end-to-end Request-URI.
        Ok(vec![ResolvedTarget::immediate(
            self.target,
            TransportType::Tls,
        )
        .with_authority(self.authority.clone())])
    }
}

async fn bind_transport(
    args: &Args,
) -> Result<
    (
        Arc<dyn Transport>,
        mpsc::Receiver<TransportEvent>,
        SocketAddr,
    ),
    Box<dyn std::error::Error>,
> {
    match args.transport {
        WireTransport::Udp => {
            let (transport, events) = UdpTransport::bind(args.listen, None).await?;
            let local_addr = transport.local_addr()?;
            Ok((Arc::new(transport), events, local_addr))
        }
        WireTransport::Tcp => {
            let (transport, events) = TcpTransport::bind(args.listen, Some(256), None).await?;
            let local_addr = transport.local_addr()?;
            Ok((Arc::new(transport), events, local_addr))
        }
        WireTransport::Tls => {
            let certificate = args.certificate.as_ref().expect("validated certificate");
            let private_key = args.private_key.as_ref().expect("validated private key");
            let ca_certificate = args
                .ca_certificate
                .as_ref()
                .expect("validated CA certificate");
            let client_certificate = args
                .client_certificate
                .as_ref()
                .expect("validated client certificate");
            let client_private_key = args
                .client_private_key
                .as_ref()
                .expect("validated client private key");
            let client_ca_certificate = args
                .client_ca_certificate
                .as_ref()
                .expect("validated client CA certificate");
            let (transport, events) = TlsTransport::bind_with_configs(
                args.listen,
                certificate,
                private_key,
                None,
                TlsClientConfig {
                    extra_ca_path: Some(ca_certificate.clone()),
                    insecure_skip_verify: false,
                    client_cert_path: Some(client_certificate.clone()),
                    client_key_path: Some(client_private_key.clone()),
                    ..Default::default()
                },
                TlsServerClientAuthConfig::required(client_ca_certificate.clone()),
            )
            .await?;
            let local_addr = transport.local_addr()?;
            Ok((Arc::new(transport), events, local_addr))
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum InboundTlsRequestIdentity<'a> {
    Verified(&'a TransportConnectionMetadata),
    Missing,
}

fn inbound_tls_request_identity(event: &TransportEvent) -> Option<InboundTlsRequestIdentity<'_>> {
    let TransportEvent::MessageReceived {
        message: Message::Request(_),
        transport_type: TransportType::Tls,
        connection_metadata,
        ..
    } = event
    else {
        return None;
    };
    Some(match connection_metadata {
        Some(metadata) => InboundTlsRequestIdentity::Verified(metadata),
        None => InboundTlsRequestIdentity::Missing,
    })
}

fn audit_tls_transport_events(
    mut events: mpsc::Receiver<TransportEvent>,
) -> mpsc::Receiver<TransportEvent> {
    let (forward_tx, forward_rx) = mpsc::channel(4096);
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            match inbound_tls_request_identity(&event) {
                Some(InboundTlsRequestIdentity::Verified(metadata)) => {
                    println!(
                        "RVOIP_TLS_PEER_ACCEPTED direction=inbound transport=tls \
                         leaf_certificate_sha256={} presented_chain_len={}",
                        metadata.tls_peer_identity.leaf_certificate_sha256,
                        metadata.tls_peer_identity.presented_chain_len,
                    );
                }
                Some(InboundTlsRequestIdentity::Missing) => {
                    println!("RVOIP_TLS_PEER_METADATA_MISSING direction=inbound transport=tls");
                }
                None => {}
            }
            if forward_tx.send(event).await.is_err() {
                break;
            }
        }
    });
    forward_rx
}

fn print_retention_snapshot(
    phase: &str,
    proxy: &StatefulProxy,
    transaction_manager: &TransactionManager,
) {
    let proxy_counts = proxy.retention_snapshot();
    let transaction_counts = transaction_manager.retention_counts();
    let completion_counts = transaction_manager.client_completion_retention_counts();
    let retired_counts = transaction_manager.retired_client_retention_counts();
    println!(
        "RVOIP_PROXY_RETENTION \
         phase={phase} \
         response_contexts={} \
         downstream_invite_indexes={} \
         generated_cancel_transactions={} \
         timer_c_entries={} \
         timer_c_heap_entries={} \
         generated_cancel_retry_entries={} \
         generated_cancel_retry_heap_entries={} \
         response_context_deadlines={} \
         stateless_response_routes={} \
         known_branches={} \
         downstream_slot_reservations={} \
         response_context_deadline_heap_entries={} \
         stateless_response_route_deadlines={} \
         stateless_response_route_deadline_heap_entries={} \
         client_transactions={} \
         server_transactions={} \
         active_transactions_total={} \
         terminated_transactions={} \
         server_invite_dialog_index={} \
         server_invite_dialog_keys_by_tx={} \
         invite_2xx_response_cache={} \
         invite_2xx_response_due_queue={} \
         transaction_destinations={} \
         compact_non_invite_tombstones={} \
         compact_non_invite_deadlines={} \
         event_subscribers={} \
         subscriber_to_transactions={} \
         transaction_to_subscribers={} \
         pending_inbound_bytes={} \
         pending_inbound_transport={} \
         pending_inbound_timing={} \
         pending_inbound_principals={} \
         completion_active={} \
         completion_retained={} \
         completion_compact={} \
         completion_parsed_responses={} \
         completion_wire_responses={} \
         completion_wire_response_bytes={} \
         completion_deadlines={} \
         retired_client_transactions={} \
         retired_client_request_wire_bytes={} \
         retired_client_ack_template_allocations={} \
         retired_client_deadlines={}",
        proxy_counts.response_contexts,
        proxy_counts.downstream_invite_indexes,
        proxy_counts.generated_cancel_transactions,
        proxy_counts.timer_c_entries,
        proxy_counts.timer_c_heap_entries,
        proxy_counts.generated_cancel_retry_entries,
        proxy_counts.generated_cancel_retry_heap_entries,
        proxy_counts.response_context_deadlines,
        proxy_counts.stateless_response_routes,
        proxy_counts.known_branches,
        proxy_counts.downstream_slot_reservations,
        proxy_counts.response_context_deadline_heap_entries,
        proxy_counts.stateless_response_route_deadlines,
        proxy_counts.stateless_response_route_deadline_heap_entries,
        transaction_counts.client_transactions,
        transaction_counts.server_transactions,
        transaction_counts.active_transactions_total,
        transaction_counts.terminated_transactions,
        transaction_counts.server_invite_dialog_index,
        transaction_counts.server_invite_dialog_keys_by_tx,
        transaction_counts.invite_2xx_response_cache,
        transaction_counts.invite_2xx_response_due_queue,
        transaction_counts.transaction_destinations,
        transaction_counts.compact_non_invite_tombstones,
        transaction_counts.compact_non_invite_deadlines,
        transaction_counts.event_subscribers,
        transaction_counts.subscriber_to_transactions,
        transaction_counts.transaction_to_subscribers,
        transaction_counts.pending_inbound_bytes,
        transaction_counts.pending_inbound_transport,
        transaction_counts.pending_inbound_timing,
        transaction_counts.pending_inbound_principals,
        completion_counts.active,
        completion_counts.retained,
        completion_counts.compact,
        completion_counts.parsed_responses,
        completion_counts.wire_responses,
        completion_counts.wire_response_bytes,
        completion_counts.deadlines,
        retired_counts.transactions,
        retired_counts.request_wire_bytes,
        retired_counts.ack_template_allocations,
        retired_counts.deadlines,
    );
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args().map_err(|error| {
        if error == usage() {
            println!("{error}");
        } else {
            eprintln!("{error}");
        }
        std::io::Error::new(std::io::ErrorKind::InvalidInput, error)
    })?;

    let (transport, transport_events, actual_listen) = bind_transport(&args).await?;
    let transport_events = if matches!(args.transport, WireTransport::Tls) {
        audit_tls_transport_events(transport_events)
    } else {
        transport_events
    };
    let (transaction_manager, transaction_events) =
        TransactionManager::new(transport.clone(), transport_events, Some(4096)).await?;
    let transaction_manager = Arc::new(transaction_manager);
    let options = ProxyRuntimeOptions::default()
        .with_advertised_via(args.transport.transport_type(), args.advertised);
    let (proxy_config, options) = apply_runtime_controls(&args, options)?;
    let proxy = if matches!(args.transport, WireTransport::Tls) {
        let target_authority = args
            .target_authority
            .as_ref()
            .expect("validated TLS target authority");
        let authority = TransportAuthority::dns(target_authority.clone())?;
        let route: UriRouteFn =
            Arc::new(move |request| Some(UriRouteDecision::to(exact_tls_route_target(request))));
        let resolver = Arc::new(ExactTlsResolver {
            target: args.target,
            authority,
        });
        StatefulProxy::with_uri_routes(
            transaction_manager.clone(),
            route,
            proxy_config,
            options.with_resolver(resolver),
        )
    } else {
        let routes = InteropRoutes::new(&args);
        let socket_route: RouteFn = Arc::new(move |request| Some(routes.select(request)));
        if let (Some(dns_server), Some(rfc3263_uri)) = (args.dns_server, args.rfc3263_uri.clone()) {
            let uri_route: UriRouteFn = Arc::new(move |request| {
                (request.method() == Method::Invite
                    && interop_scenario(request) == Some("rfc3263-failover"))
                .then(|| UriRouteDecision::to(ProxyTarget::new(rfc3263_uri.clone())))
            });
            StatefulProxy::with_uri_routes_and_socket_fallback(
                transaction_manager.clone(),
                uri_route,
                socket_route,
                proxy_config,
                options.with_resolver(Arc::new(HickoryResolver::with_nameserver(dns_server))),
            )
        } else {
            StatefulProxy::with_options(
                transaction_manager.clone(),
                socket_route,
                proxy_config,
                options,
            )
        }
    };
    let proxy_task = proxy.clone().run(transaction_events);

    println!(
        "RVOIP_PROXY_READY transport={} listen={} advertised={} target={} \
         interop_test_mode={} aux_targets={} failover_targets={} dns_server={} \
         rfc3263_uri={} timer_c_ms={} max_response_contexts={} \
         local_uris={} record_route={} tls_dns_authority={}",
        args.transport.label(),
        actual_listen,
        args.advertised,
        args.target,
        args.interop_test_mode,
        args.aux_targets.len(),
        args.failover_targets.len(),
        args.dns_server
            .map_or_else(|| "none".to_owned(), |value| value.to_string()),
        args.rfc3263_uri
            .as_ref()
            .map_or_else(|| "none".to_owned(), ToString::to_string),
        args.timer_c_ms
            .map_or_else(|| "181000".to_owned(), |value| value.to_string()),
        args.max_response_contexts
            .map_or_else(|| "default".to_owned(), |value| value.to_string()),
        args.local_uris.len(),
        args.record_route_sip.is_some() && args.record_route_sips.is_some(),
        args.target_authority.is_some(),
    );

    print_retention_snapshot("pre_zero", &proxy, &transaction_manager);

    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let mut snapshot_signal = signal(SignalKind::user_defined1())?;
        let phases = ["activity", "cooldown", "post_retention"];
        let mut next_phase = 0usize;
        loop {
            tokio::select! {
                result = tokio::signal::ctrl_c() => {
                    result?;
                    break;
                }
                received = snapshot_signal.recv() => {
                    if received.is_none() {
                        break;
                    }
                    if let Some(phase) = phases.get(next_phase) {
                        print_retention_snapshot(phase, &proxy, &transaction_manager);
                        next_phase += 1;
                    } else {
                        eprintln!(
                            "ignoring extra SIGUSR1 retention snapshot request after post_retention"
                        );
                    }
                }
            }
        }
    }

    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await?;

    print_retention_snapshot("pre_shutdown", &proxy, &transaction_manager);
    proxy_task.abort();
    transaction_manager.shutdown().await;
    print_retention_snapshot("post_shutdown", &proxy, &transaction_manager);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_proxy::ForkMode;

    fn valid_udp_args() -> Args {
        Args {
            listen: "127.0.0.1:25060".parse().unwrap(),
            advertised: "127.0.0.1:25060".parse().unwrap(),
            target: "127.0.0.1:25080".parse().unwrap(),
            aux_targets: Vec::new(),
            failover_targets: Vec::new(),
            dns_server: None,
            rfc3263_uri: None,
            target_authority: None,
            transport: WireTransport::Udp,
            interop_test_mode: false,
            timer_c_ms: None,
            max_response_contexts: None,
            max_downstream_transactions: None,
            max_branches_per_context: None,
            max_stateless_routes: None,
            local_uris: Vec::new(),
            record_route_sip: None,
            record_route_sips: None,
            certificate: None,
            private_key: None,
            ca_certificate: None,
            client_certificate: None,
            client_private_key: None,
            client_ca_certificate: None,
        }
    }

    fn valid_tls_args() -> Args {
        Args {
            listen: "127.0.0.1:25060".parse().unwrap(),
            advertised: "127.0.0.1:25060".parse().unwrap(),
            target: "127.0.0.1:25070".parse().unwrap(),
            aux_targets: Vec::new(),
            failover_targets: Vec::new(),
            dns_server: None,
            rfc3263_uri: None,
            target_authority: Some("kamailio.proxy.test".into()),
            transport: WireTransport::Tls,
            interop_test_mode: false,
            timer_c_ms: None,
            max_response_contexts: None,
            max_downstream_transactions: None,
            max_branches_per_context: None,
            max_stateless_routes: None,
            local_uris: Vec::new(),
            record_route_sip: None,
            record_route_sips: None,
            certificate: Some("server.pem".into()),
            private_key: Some("server.key.pem".into()),
            ca_certificate: Some("ca.pem".into()),
            client_certificate: Some("client.pem".into()),
            client_private_key: Some("client.key.pem".into()),
            client_ca_certificate: Some("client-ca.pem".into()),
        }
    }

    fn scenario_request(method: Method, uri: &str, scenario: &str) -> Request {
        Request::new(method, uri.parse().unwrap()).with_header(TypedHeader::Other(
            HeaderName::Other(INTEROP_SCENARIO_HEADER.into()),
            HeaderValue::Raw(scenario.as_bytes().to_vec()),
        ))
    }

    #[test]
    fn tls_configuration_accepts_exact_dns_authority() {
        assert!(validate_args(&valid_tls_args()).is_ok());
    }

    #[test]
    fn tls_configuration_rejects_absent_target_authority() {
        let mut args = valid_tls_args();
        args.target_authority = None;
        assert!(validate_args(&args)
            .unwrap_err()
            .contains("TLS requires server identity"));
    }

    #[test]
    fn tls_configuration_rejects_ip_literal_target_authority() {
        let mut args = valid_tls_args();
        args.target_authority = Some("127.0.0.1".into());
        assert!(validate_args(&args)
            .unwrap_err()
            .contains("must be a DNS name"));
    }

    #[test]
    fn tls_configuration_rejects_malformed_dns_label() {
        let mut args = valid_tls_args();
        args.target_authority = Some("bad_label.proxy.test".into());
        assert!(validate_args(&args)
            .unwrap_err()
            .contains("not a valid DNS name"));
    }

    #[test]
    fn tls_configuration_rejects_partial_credentials() {
        let mut args = valid_tls_args();
        args.client_ca_certificate = None;
        assert!(validate_args(&args)
            .unwrap_err()
            .contains("TLS requires server identity"));
    }

    #[test]
    fn tls_dialog_ack_and_bye_preserve_the_contact_derived_request_uri() {
        for method in [Method::Ack, Method::Bye] {
            let contact_uri: Uri = format!(
                "sip:agent@192.0.2.80:{};transport=tcp",
                if method == Method::Ack { 5080 } else { 5081 }
            )
            .parse()
            .unwrap();
            let request = Request::new(method.clone(), contact_uri.clone());

            assert_eq!(
                exact_tls_route_target(&request).uri,
                contact_uri,
                "{method} must retain its distinct dialog remote target"
            );
        }
    }

    #[tokio::test]
    async fn exact_tls_resolver_keeps_configured_socket_and_dns_authority_for_dialog_target() {
        let target: SocketAddr = "127.0.0.1:25070".parse().unwrap();
        let authority = TransportAuthority::dns("sipp.proxy.test").unwrap();
        let resolver = ExactTlsResolver {
            target,
            authority: authority.clone(),
        };

        let resolved = resolver
            .resolve(&"sip:agent@192.0.2.80:5080;transport=tcp".parse().unwrap())
            .await
            .expect("the exact TLS egress route is independent of the dialog remote target");

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].addr, target);
        assert_eq!(resolved[0].transport, TransportType::Tls);
        assert_eq!(resolved[0].authority.as_ref(), Some(&authority));
    }

    #[test]
    fn tls_audit_requires_client_identity_only_for_received_requests() {
        let source = "192.0.2.10:5061".parse().unwrap();
        let destination = "192.0.2.20:5061".parse().unwrap();
        let metadata = TransportConnectionMetadata {
            tls_peer_identity: rvoip_sip_transport::TlsPeerIdentity {
                leaf_certificate_sha256: "ab".repeat(32),
                presented_chain_len: 2,
            },
        };
        let event = |message, connection_metadata| TransportEvent::MessageReceived {
            message,
            source,
            destination,
            transport_type: TransportType::Tls,
            flow_id: None,
            raw_bytes: None,
            timing: None,
            connection_metadata,
        };

        let verified_request = event(
            Message::Request(Request::new(
                Method::Invite,
                "sips:agent@example.test".parse().unwrap(),
            )),
            Some(metadata.clone()),
        );
        assert!(matches!(
            inbound_tls_request_identity(&verified_request),
            Some(InboundTlsRequestIdentity::Verified(observed))
                if observed == &metadata
        ));

        let missing_request = event(
            Message::Request(Request::new(
                Method::Bye,
                "sips:agent@example.test".parse().unwrap(),
            )),
            None,
        );
        assert_eq!(
            inbound_tls_request_identity(&missing_request),
            Some(InboundTlsRequestIdentity::Missing)
        );

        let verified_outbound_response = event(
            Message::Response(rvoip_sip_core::Response::new(
                rvoip_sip_core::StatusCode::Ok,
            )),
            None,
        );
        assert_eq!(
            inbound_tls_request_identity(&verified_outbound_response),
            None,
            "a response on an outbound TLS client flow is authenticated by \
             the configured server authority, not inbound mTLS client metadata",
        );
    }

    #[test]
    fn default_route_remains_the_exact_single_target() {
        let args = valid_udp_args();
        let routes = InteropRoutes::new(&args);
        for request in [
            Request::new(Method::Invite, "sip:service@example.test".parse().unwrap()),
            scenario_request(Method::Invite, "sip:service@example.test", "parallel-fork"),
        ] {
            let decision = routes.select(&request);
            assert_eq!(decision.mode, ForkMode::Sequential);
            assert_eq!(decision.targets, vec![args.target]);
            assert!(decision.leg_candidates.is_empty());
        }
        assert_eq!(ProxyConfig::default().timer_c, Duration::from_secs(181));
    }

    #[test]
    fn invite_scenarios_select_deterministic_multi_target_routes() {
        let mut args = valid_udp_args();
        args.interop_test_mode = true;
        args.aux_targets = vec![
            "127.0.0.1:25081".parse().unwrap(),
            "127.0.0.1:25082".parse().unwrap(),
        ];
        let expected = vec![args.target, args.aux_targets[0], args.aux_targets[1]];
        let routes = InteropRoutes::new(&args);

        let sequential = routes.select(&scenario_request(
            Method::Invite,
            "sip:service@example.test",
            "sequential-fork",
        ));
        assert_eq!(sequential.mode, ForkMode::Sequential);
        assert_eq!(sequential.targets, expected);

        for scenario in [
            "parallel-fork",
            "multiple-2xx",
            "late-2xx",
            "sixxx-cancel",
            "auth-aggregation",
        ] {
            let parallel = routes.select(&scenario_request(
                Method::Invite,
                "sip:service@example.test",
                scenario,
            ));
            assert_eq!(parallel.mode, ForkMode::Parallel, "{scenario}");
            assert_eq!(parallel.targets, expected, "{scenario}");
        }
    }

    #[test]
    fn recognized_fork_without_two_aux_targets_fails_closed() {
        let mut args = valid_udp_args();
        args.interop_test_mode = true;
        args.aux_targets = vec!["127.0.0.1:25081".parse().unwrap()];
        let decision = InteropRoutes::new(&args).select(&scenario_request(
            Method::Invite,
            "sip:service@example.test",
            "parallel-fork",
        ));
        assert!(decision.targets.is_empty());
        assert!(decision.leg_candidates.is_empty());
    }

    #[test]
    fn two_x_interop_scenario_headers_do_not_select_a_test_route() {
        let mut args = valid_udp_args();
        args.interop_test_mode = true;
        args.aux_targets = vec![
            "127.0.0.1:25081".parse().unwrap(),
            "127.0.0.1:25082".parse().unwrap(),
        ];
        let mut request =
            scenario_request(Method::Invite, "sip:service@example.test", "parallel-fork");
        request.headers.push(TypedHeader::Other(
            HeaderName::Other(INTEROP_SCENARIO_HEADER.into()),
            HeaderValue::Raw(b"sequential-fork".to_vec()),
        ));
        let decision = InteropRoutes::new(&args).select(&request);
        assert_eq!(decision.targets, vec![args.target]);
    }

    #[test]
    fn contact_derived_ack_routes_to_the_exact_aux_target_without_reforking() {
        let mut args = valid_udp_args();
        args.interop_test_mode = true;
        args.aux_targets = vec![
            "127.0.0.1:25081".parse().unwrap(),
            "127.0.0.1:25082".parse().unwrap(),
        ];
        let decision = InteropRoutes::new(&args).select(&scenario_request(
            Method::Ack,
            "sip:branch@127.0.0.1:25082",
            "multiple-2xx",
        ));
        assert_eq!(decision.mode, ForkMode::Sequential);
        assert_eq!(decision.targets, vec![args.aux_targets[1]]);
    }

    #[test]
    fn transport_failure_uses_one_exact_target_and_rfc3263_is_dns_only() {
        let mut args = valid_udp_args();
        args.interop_test_mode = true;
        args.failover_targets = vec![
            "127.0.0.1:25999".parse().unwrap(),
            "127.0.0.1:25081".parse().unwrap(),
        ];
        let routes = InteropRoutes::new(&args);

        let failure = routes.select(&scenario_request(
            Method::Invite,
            "sip:service@example.test;transport=tcp",
            "transport-failure",
        ));
        assert_eq!(failure.targets, vec![args.failover_targets[0]]);

        let failover = routes.select(&scenario_request(
            Method::Invite,
            "sip:service@example.test;transport=tcp",
            "rfc3263-failover",
        ));
        assert_eq!(failover.mode, ForkMode::Sequential);
        assert!(failover.targets.is_empty());
        assert!(failover.leg_candidates.is_empty());
    }

    #[test]
    fn rfc3263_dns_controls_are_paired_and_tcp_only() {
        let mut args = valid_udp_args();
        args.interop_test_mode = true;
        args.dns_server = Some("127.0.0.1:25353".parse().unwrap());
        assert!(validate_args(&args)
            .unwrap_err()
            .contains("must be configured together"));

        args.rfc3263_uri = Some(
            "sip:agent@failover.interop.test;transport=tcp"
                .parse()
                .unwrap(),
        );
        assert!(validate_args(&args)
            .unwrap_err()
            .contains("currently require TCP"));

        args.transport = WireTransport::Tcp;
        assert!(validate_args(&args).is_ok());
        args.rfc3263_uri = Some(
            "sip:agent@failover.interop.test:5060;transport=tcp"
                .parse()
                .unwrap(),
        );
        assert!(validate_args(&args)
            .unwrap_err()
            .contains("without an explicit port"));
    }

    #[test]
    fn strict_and_loose_route_markers_use_the_first_aux_hop() {
        let mut args = valid_udp_args();
        args.interop_test_mode = true;
        args.aux_targets = vec![
            "127.0.0.1:25081".parse().unwrap(),
            "127.0.0.1:25082".parse().unwrap(),
        ];
        let routes = InteropRoutes::new(&args);
        for scenario in ["route-strict", "route-loose-record-route"] {
            let decision = routes.select(&scenario_request(
                Method::Invite,
                "sip:service@example.test;transport=tcp",
                scenario,
            ));
            assert_eq!(decision.targets, vec![args.aux_targets[0]], "{scenario}");
        }
    }

    #[test]
    fn advanced_controls_require_explicit_interop_test_mode() {
        let mut args = valid_udp_args();
        args.aux_targets = vec![
            "127.0.0.1:25081".parse().unwrap(),
            "127.0.0.1:25082".parse().unwrap(),
        ];
        assert!(validate_args(&args)
            .unwrap_err()
            .contains("--interop-test-mode"));

        args.interop_test_mode = true;
        assert!(validate_args(&args).is_ok());
    }

    #[test]
    fn timer_c_override_is_test_only_and_bounded() {
        let mut args = valid_udp_args();
        args.timer_c_ms = Some(500);
        assert!(validate_args(&args).is_err());
        args.interop_test_mode = true;
        assert!(validate_args(&args).is_ok());
        let (config, options) =
            apply_runtime_controls(&args, ProxyRuntimeOptions::default()).unwrap();
        assert_eq!(config.timer_c, Duration::from_millis(500));
        assert!(format!("{options:?}").contains("allow_short_timer_c_for_tests: true"));

        args.timer_c_ms = Some(MIN_INTEROP_TIMER_C_MS - 1);
        assert!(validate_args(&args).unwrap_err().contains("--timer-c-ms"));
        args.timer_c_ms = Some(MAX_INTEROP_TIMER_C_MS + 1);
        assert!(validate_args(&args).unwrap_err().contains("--timer-c-ms"));
    }

    #[test]
    fn capacity_overrides_are_positive_and_applied_together() {
        let mut args = valid_udp_args();
        args.interop_test_mode = true;
        args.max_response_contexts = Some(1);
        args.max_downstream_transactions = Some(3);
        args.max_branches_per_context = Some(3);
        args.max_stateless_routes = Some(2);
        assert!(validate_args(&args).is_ok());
        let (_, options) = apply_runtime_controls(&args, ProxyRuntimeOptions::default()).unwrap();
        let debug = format!("{options:?}");
        assert!(debug.contains("response_context_capacity: 1"));
        assert!(debug.contains("downstream_transaction_capacity: 3"));
        assert!(debug.contains("branches_per_response_context: 3"));
        assert!(debug.contains("stateless_response_route_capacity: 2"));

        args.max_response_contexts = Some(0);
        assert!(validate_args(&args)
            .unwrap_err()
            .contains("--max-response-contexts"));
    }

    #[test]
    fn routing_policy_options_are_test_only_and_scheme_consistent() {
        let mut args = valid_udp_args();
        args.interop_test_mode = true;
        args.local_uris = vec![
            "sip:proxy.test:25060;transport=tcp;lr".parse().unwrap(),
            "sips:proxy.test:25061;transport=tls;lr".parse().unwrap(),
        ];
        args.record_route_sip = Some("sip:proxy.test:25060;transport=tcp;lr".parse().unwrap());
        args.record_route_sips = Some("sips:proxy.test:25061;transport=tls;lr".parse().unwrap());
        assert!(validate_args(&args).is_ok());
        assert!(routing_policy(&args).is_ok());

        args.record_route_sips = Some("sip:proxy.test:25061;transport=tcp;lr".parse().unwrap());
        assert!(validate_args(&args)
            .unwrap_err()
            .contains("requires a sips: URI"));
    }

    #[test]
    fn record_route_pair_and_single_value_cli_are_enforced() {
        let parsed = parse_args_from([
            "--listen",
            "127.0.0.1:25060",
            "--advertised",
            "127.0.0.1:25060",
            "--target",
            "127.0.0.1:25080",
            "--transport",
            "tcp",
            "--interop-test-mode",
            "--record-route-sip",
            "sip:proxy.test:25060;transport=tcp;lr",
        ]);
        assert!(parsed.unwrap_err().contains("must be configured together"));

        let duplicate = parse_args_from([
            "--listen",
            "127.0.0.1:25060",
            "--advertised",
            "127.0.0.1:25060",
            "--target",
            "127.0.0.1:25080",
            "--transport",
            "tcp",
            "--interop-test-mode",
            "--record-route-sip",
            "sip:proxy.test:25060;transport=tcp;lr",
            "--record-route-sip",
            "sip:other.test:25060;transport=tcp;lr",
        ]);
        assert!(duplicate
            .unwrap_err()
            .contains("may be specified only once"));
    }

    #[test]
    fn tls_rejects_multi_target_overrides_but_keeps_exact_authority_defaults() {
        let mut args = valid_tls_args();
        args.interop_test_mode = true;
        args.aux_targets = vec![
            "127.0.0.1:25081".parse().unwrap(),
            "127.0.0.1:25082".parse().unwrap(),
        ];
        assert!(validate_args(&args)
            .unwrap_err()
            .contains("one exact target authority"));

        args.aux_targets.clear();
        args.interop_test_mode = false;
        assert!(validate_args(&args).is_ok());
    }
}
