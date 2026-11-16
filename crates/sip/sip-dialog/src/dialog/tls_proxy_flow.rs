//! Same-dialog reuse of a retained TLS flow for in-dialog requests.
//!
//! Two opt-in sources bind a dialog to one exact TLS flow:
//!
//! - an inbound dialog delivered by a trusted record-routing proxy, admitted
//!   by a [`SipTlsProxyFlowPolicy`]; the flow is the TLS connection the proxy
//!   opened to us;
//! - an outbound dialog whose initial INVITE went over a certificate-verified
//!   TLS connection, when [`DialogConfig::retain_outbound_tls_flow`] is set.
//!
//! A bound dialog's request uses the flow only while its next hop is exactly
//! the URI the binding was admitted for and resolves to the bound endpoint.
//! Any other next hop keeps normal routing. A closed flow never silently
//! falls back to a new connection: the only replacement is the explicit
//! certificate-verified renewal a policy's `renewal_authority` permits.
//!
//! [`DialogConfig::retain_outbound_tls_flow`]: crate::api::config::DialogConfig::retain_outbound_tls_flow

use crate::api::config::SipTlsProxyFlowPolicy;
use rvoip_core_traits::identity::AuthenticatedPrincipal;
use rvoip_sip_core::types::{param::Param, uri::Scheme};
use rvoip_sip_core::{Host, Uri};
use rvoip_sip_transport::{transport::TransportType, TransportAuthority, TransportRoute};
use std::net::SocketAddr;

/// Default SIP TLS port (RFC 3261 §19.1.2).
const DEFAULT_TLS_PORT: u16 = 5061;

#[derive(Clone, Debug, PartialEq, Eq)]
enum FlowOrigin {
    /// A TLS connection a trusted proxy opened to this endpoint.
    InboundTrustedProxy,
    /// The certificate-verified TLS connection of our own initial INVITE.
    OutboundVerifiedTls,
}

/// The exact TLS flow a dialog's in-dialog requests are sent on.
///
/// Opaque and process-local: [`Dialog`](crate::dialog::Dialog) never
/// serializes it, so a restored dialog never reuses a stale flow identity.
#[derive(Clone, Debug)]
pub struct TlsProxyFlowBinding {
    next_hop: Uri,
    endpoint: SocketAddr,
    route: TransportRoute,
    origin: FlowOrigin,
    renewal_authority: Option<TransportAuthority>,
}

/// Transport parameters of `uri`, in order.
fn transport_params(uri: &Uri) -> Vec<&str> {
    uri.parameters
        .iter()
        .filter_map(|parameter| match parameter {
            Param::Transport(value) => Some(value.as_str()),
            _ => None,
        })
        .collect()
}

/// `true` when `uri` explicitly selects TLS: a `sips:` URI without a
/// conflicting transport, or a `sip:` URI with exactly `transport=tls`.
fn selects_tls(uri: &Uri) -> bool {
    let transports = transport_params(uri);
    match uri.scheme {
        Scheme::Sips => {
            transports.len() <= 1
                && transports
                    .first()
                    .is_none_or(|value| value.eq_ignore_ascii_case("tls"))
        }
        Scheme::Sip => transports.len() == 1 && transports[0].eq_ignore_ascii_case("tls"),
        _ => false,
    }
}

fn is_loose_route(uri: &Uri) -> bool {
    uri.parameters
        .iter()
        .any(|parameter| matches!(parameter, Param::Lr))
}

/// Socket address of a loose-routing TLS proxy URI with an IP-literal host.
fn tls_proxy_endpoint(uri: &Uri) -> Option<SocketAddr> {
    if !is_loose_route(uri) || !selects_tls(uri) {
        return None;
    }
    let Host::Address(address) = uri.host else {
        return None;
    };
    Some(SocketAddr::new(
        address,
        uri.port.unwrap_or(DEFAULT_TLS_PORT),
    ))
}

/// `true` when TLS URI `uri` names the peer `route` reached: the same port,
/// and either the same IP literal or the DNS name the route's certificate
/// was verified against.
fn endpoint_matches(uri: &Uri, route: &TransportRoute) -> bool {
    if !selects_tls(uri) || uri.port.unwrap_or(DEFAULT_TLS_PORT) != route.destination.port() {
        return false;
    }
    match (&uri.host, &route.authority) {
        (Host::Address(address), _) => *address == route.destination.ip(),
        (Host::Domain(domain), Some(TransportAuthority::Dns(authority))) => {
            domain.trim_end_matches('.').eq_ignore_ascii_case(authority)
        }
        _ => false,
    }
}

/// The next hop of the in-dialog requests of a dialog formed by
/// `original_request`, our successful initial INVITE.
///
/// A learned route set's first entry wins. Without one, a Route the INVITE
/// itself carried (a configured outbound proxy, which need not record-route)
/// is kept when it is the INVITE's exact next hop; otherwise requests go to
/// the Contact-derived remote target. Strict routes (no `;lr`) rewrite the
/// Request-URI and are outside this mechanism, so they yield `None`.
pub(crate) fn verified_outbound_next_hop(
    route_set: &[Uri],
    remote_target: &Uri,
    original_request: &rvoip_sip_core::Request,
) -> Option<Uri> {
    use rvoip_sip_core::{HeaderName, Method};
    if original_request.method() != Method::Invite {
        return None;
    }
    let route = if let Some(route) = route_set.first() {
        route.clone()
    } else if original_request
        .headers
        .iter()
        .any(|header| header.name().wire_eq(&HeaderName::Route))
    {
        let exact = crate::transaction::transport::multiplexed::exact_next_hop_uri_for_request(
            original_request,
        )
        .ok()?;
        let top = crate::transaction::transport::multiplexed::top_route_uri(original_request)?;
        if exact != top {
            return None;
        }
        top
    } else {
        return Some(remote_target.clone());
    };
    is_loose_route(&route).then_some(route)
}

impl TlsProxyFlowBinding {
    /// Bind an inbound dialog to the TLS flow its INVITE arrived on, when
    /// `policy` trusts `principal`, `ingress` is an identified TLS flow from
    /// the policy's proxy address, and `next_hop` (the dialog's first route)
    /// is the policy's loose-routing TLS proxy URI.
    pub(crate) fn admit(
        policy: &SipTlsProxyFlowPolicy,
        principal: &AuthenticatedPrincipal,
        next_hop: &Uri,
        ingress: &TransportRoute,
    ) -> Option<Self> {
        if policy.validate().is_err()
            || principal.is_expired()
            || principal.issuer.as_deref() != Some(policy.issuer.as_str())
            || principal.tenant.as_deref() != Some(policy.tenant.as_str())
            || principal.subject != policy.subject
            || ingress.transport_type != Some(TransportType::Tls)
            || ingress.flow_id.is_none()
            || ingress.destination.ip() != policy.proxy_endpoint.ip()
            || tls_proxy_endpoint(next_hop) != Some(policy.proxy_endpoint)
        {
            return None;
        }
        Some(Self {
            next_hop: next_hop.clone(),
            endpoint: policy.proxy_endpoint,
            route: ingress.clone(),
            origin: FlowOrigin::InboundTrustedProxy,
            renewal_authority: policy
                .renewal_authority
                .as_ref()
                .and_then(|value| TransportAuthority::dns(value.clone()).ok()),
        })
    }

    /// Bind a dialog this endpoint initiated to the certificate-verified TLS
    /// flow `route` its successful initial INVITE used, when `next_hop`
    /// names that same peer.
    pub(crate) fn admit_verified_outbound(next_hop: &Uri, route: &TransportRoute) -> Option<Self> {
        (route.transport_type == Some(TransportType::Tls)
            && route.authority.is_some()
            && route.flow_id.is_some()
            && endpoint_matches(next_hop, route))
        .then(|| Self {
            next_hop: next_hop.clone(),
            endpoint: route.destination,
            route: route.clone(),
            origin: FlowOrigin::OutboundVerifiedTls,
            renewal_authority: None,
        })
    }

    /// The retained route for a request whose exact next hop is `next_hop`,
    /// resolved to `candidate` over `transport`; `None` for anything else.
    /// The caller must still find the flow live at the transport boundary.
    pub(crate) fn route_for(
        &self,
        next_hop: &Uri,
        candidate: SocketAddr,
        transport: TransportType,
    ) -> Option<TransportRoute> {
        (next_hop == &self.next_hop
            && candidate == self.endpoint
            && transport == TransportType::Tls)
            .then(|| self.route.clone())
    }

    /// When the policy allows renewal, the route of a new connection to the
    /// fixed proxy endpoint, verified against the configured DNS name. Never
    /// the peer's ephemeral source port or the dialog's Contact. The caller
    /// prepares (authenticates) it before any SIP write and only then calls
    /// [`Self::retain_authenticated_renewal`].
    pub(crate) fn renewal_route_for(
        &self,
        next_hop: &Uri,
        candidate: SocketAddr,
        transport: TransportType,
    ) -> Option<TransportRoute> {
        self.route_for(next_hop, candidate, transport)?;
        if self.origin != FlowOrigin::InboundTrustedProxy {
            return None;
        }
        Some(
            TransportRoute::new(self.endpoint)
                .with_transport_type(TransportType::Tls)
                .with_authority(self.renewal_authority.clone()?),
        )
    }

    /// Replace the retained flow with `prepared`, a route returned by TLS
    /// preparation of [`Self::renewal_route_for`]. `expected` is the binding
    /// the renewal started from; a concurrent change makes this refuse.
    pub(crate) fn retain_authenticated_renewal(
        &mut self,
        expected: &Self,
        prepared: &TransportRoute,
    ) -> bool {
        if self.origin != FlowOrigin::InboundTrustedProxy
            || self.next_hop != expected.next_hop
            || self.endpoint != expected.endpoint
            || self.renewal_authority != expected.renewal_authority
            || (self.route != expected.route && self.route != *prepared)
            || prepared.destination != self.endpoint
            || prepared.transport_type != Some(TransportType::Tls)
            || prepared.authority.is_none()
            || prepared.authority != self.renewal_authority
            || prepared.flow_id.is_none()
            || prepared.flow_id == expected.route.flow_id
        {
            return false;
        }
        self.route = prepared.clone();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_core_traits::identity::{AuthenticationMethod, IdentityAssurance};
    use rvoip_sip_transport::TransportFlowId;

    const ISSUER: &str = "listener-trusted-cidr";

    fn policy() -> SipTlsProxyFlowPolicy {
        SipTlsProxyFlowPolicy::new(ISSUER, "edge", "trunk", "192.0.2.10:5061".parse().unwrap())
    }

    fn principal() -> AuthenticatedPrincipal {
        AuthenticatedPrincipal {
            subject: "trunk".into(),
            tenant: Some("edge".into()),
            issuer: Some(ISSUER.into()),
            scopes: vec!["call:attach".into()],
            method: AuthenticationMethod::ApiKey,
            assurance: IdentityAssurance::Anonymous,
            expires_at: None,
        }
    }

    fn ingress(flow: u64) -> TransportRoute {
        TransportRoute::new("192.0.2.10:43000".parse().unwrap())
            .with_transport_type(TransportType::Tls)
            .with_flow_id(TransportFlowId::from_process_local_value(flow).unwrap())
    }

    #[test]
    fn inbound_admission_requires_exact_principal_tls_flow_source_and_loose_tls_route() {
        let policy = policy();
        let principal = principal();
        let route: Uri = "sip:192.0.2.10:5061;transport=tls;lr".parse().unwrap();
        let ingress = ingress(101);
        let binding = TlsProxyFlowBinding::admit(&policy, &principal, &route, &ingress).unwrap();
        let selected = binding
            .route_for(&route, policy.proxy_endpoint, TransportType::Tls)
            .unwrap();
        assert_eq!(selected.flow_id, ingress.flow_id);
        assert_eq!(selected.destination, ingress.destination);

        // Method and scopes are the operator's business; only identity counts.
        let mut other_method = principal.clone();
        other_method.method = AuthenticationMethod::SipDigest;
        other_method.scopes.clear();
        assert!(TlsProxyFlowBinding::admit(&policy, &other_method, &route, &ingress).is_some());

        for field in [
            "subject",
            "tenant",
            "no-tenant",
            "issuer",
            "no-issuer",
            "expiry",
        ] {
            let mut changed = principal.clone();
            match field {
                "subject" => changed.subject = "other".into(),
                "tenant" => changed.tenant = Some("other".into()),
                "no-tenant" => changed.tenant = None,
                "issuer" => changed.issuer = Some("other".into()),
                "no-issuer" => changed.issuer = None,
                "expiry" => changed.expires_at = Some("2000-01-01T00:00:00Z".parse().unwrap()),
                _ => unreachable!(),
            }
            assert!(
                TlsProxyFlowBinding::admit(&policy, &changed, &route, &ingress).is_none(),
                "{field}"
            );
        }
        for transport in [None, Some(TransportType::Tcp), Some(TransportType::Udp)] {
            let mut changed = ingress.clone();
            changed.transport_type = transport;
            assert!(TlsProxyFlowBinding::admit(&policy, &principal, &route, &changed).is_none());
        }
        let mut changed = ingress.clone();
        changed.flow_id = None;
        assert!(TlsProxyFlowBinding::admit(&policy, &principal, &route, &changed).is_none());
        changed = ingress.clone();
        changed.destination = "192.0.2.11:43000".parse().unwrap();
        assert!(TlsProxyFlowBinding::admit(&policy, &principal, &route, &changed).is_none());

        let mut invalid_policy = policy.clone();
        invalid_policy.subject = String::new();
        assert!(
            TlsProxyFlowBinding::admit(&invalid_policy, &principal, &route, &ingress).is_none()
        );

        for invalid in [
            "sip:192.0.2.10:5061;transport=tls",
            "sip:192.0.2.10:5061;transport=udp;lr",
            "sip:192.0.2.10:5061;lr",
            "sip:192.0.2.10:5061;transport=tls;transport=tls;lr",
            "sip:192.0.2.11:5061;transport=tls;lr",
            "sip:192.0.2.10:5062;transport=tls;lr",
            "sip:proxy.example:5061;transport=tls;lr",
        ] {
            let changed: Uri = invalid.parse().unwrap();
            assert!(
                TlsProxyFlowBinding::admit(&policy, &principal, &changed, &ingress).is_none(),
                "{invalid}"
            );
            assert!(binding
                .route_for(&changed, policy.proxy_endpoint, TransportType::Tls)
                .is_none());
        }
        // A sips: route without a port is the policy's 5061 listener.
        let sips: Uri = "sips:192.0.2.10;lr".parse().unwrap();
        assert!(TlsProxyFlowBinding::admit(&policy, &principal, &sips, &ingress).is_some());
    }

    #[test]
    fn inbound_route_selection_is_exact_and_per_dialog() {
        let policy = policy();
        let route: Uri = "sip:192.0.2.10:5061;transport=tls;lr".parse().unwrap();
        let first =
            TlsProxyFlowBinding::admit(&policy, &principal(), &route, &ingress(101)).unwrap();
        let second =
            TlsProxyFlowBinding::admit(&policy, &principal(), &route, &ingress(202)).unwrap();
        assert!(first
            .route_for(
                &route,
                "192.0.2.10:5062".parse().unwrap(),
                TransportType::Tls
            )
            .is_none());
        assert!(first
            .route_for(&route, policy.proxy_endpoint, TransportType::Udp)
            .is_none());
        assert_ne!(
            first
                .route_for(&route, policy.proxy_endpoint, TransportType::Tls)
                .unwrap()
                .flow_id,
            second
                .route_for(&route, policy.proxy_endpoint, TransportType::Tls)
                .unwrap()
                .flow_id
        );
        // Without a renewal authority there is no replacement route.
        assert!(first
            .renewal_route_for(&route, policy.proxy_endpoint, TransportType::Tls)
            .is_none());
    }

    #[test]
    fn renewal_uses_fixed_endpoint_and_configured_name_and_accepts_only_a_new_verified_flow() {
        let policy = policy().with_renewal_authority("Proxy.Example");
        let route: Uri = "sip:192.0.2.10:5061;transport=tls;lr".parse().unwrap();
        let original = ingress(101);
        let binding = TlsProxyFlowBinding::admit(&policy, &principal(), &route, &original).unwrap();
        let renewal = binding
            .renewal_route_for(&route, policy.proxy_endpoint, TransportType::Tls)
            .unwrap();
        assert_eq!(renewal.destination, policy.proxy_endpoint);
        assert_eq!(
            renewal.authority,
            Some(TransportAuthority::dns("proxy.example").unwrap())
        );
        assert!(renewal.flow_id.is_none());
        let other: Uri = "sip:192.0.2.10:5062;transport=tls;lr".parse().unwrap();
        assert!(binding
            .renewal_route_for(&other, policy.proxy_endpoint, TransportType::Tls)
            .is_none());

        let prepared = renewal
            .clone()
            .with_flow_id(TransportFlowId::from_process_local_value(303).unwrap());
        for bad in [
            renewal.clone(),
            prepared.clone().with_flow_id(original.flow_id.unwrap()),
            {
                let mut changed = prepared.clone();
                changed.authority = Some(TransportAuthority::dns("other.example").unwrap());
                changed
            },
            {
                let mut changed = prepared.clone();
                changed.destination = original.destination;
                changed
            },
        ] {
            let mut current = binding.clone();
            assert!(!current.retain_authenticated_renewal(&binding, &bad));
        }
        let mut current = binding.clone();
        assert!(current.retain_authenticated_renewal(&binding, &prepared));
        assert_eq!(
            current
                .route_for(&route, policy.proxy_endpoint, TransportType::Tls)
                .unwrap()
                .flow_id,
            prepared.flow_id
        );
        // A binding changed concurrently by another renewal is not overwritten
        // with a third flow.
        let third = renewal.with_flow_id(TransportFlowId::from_process_local_value(404).unwrap());
        assert!(!current.retain_authenticated_renewal(&binding, &third));
    }

    #[test]
    fn outbound_binding_keeps_verified_authority_for_ip_record_route_and_rejects_other_hops() {
        let route = TransportRoute::new("192.0.2.10:5061".parse().unwrap())
            .with_transport_type(TransportType::Tls)
            .with_authority(TransportAuthority::dns("carrier.example").unwrap())
            .with_flow_id(TransportFlowId::from_process_local_value(101).unwrap());
        let next: Uri = "sip:192.0.2.10:5061;transport=tls;lr".parse().unwrap();
        let binding = TlsProxyFlowBinding::admit_verified_outbound(&next, &route).unwrap();
        let retained = binding
            .route_for(&next, route.destination, TransportType::Tls)
            .unwrap();
        assert_eq!(retained.authority, route.authority);
        assert_eq!(retained.flow_id, route.flow_id);
        assert!(binding
            .renewal_route_for(&next, route.destination, TransportType::Tls)
            .is_none());
        for changed in [
            "sip:192.0.2.11:5061;transport=tls;lr",
            "sip:192.0.2.10:5062;transport=tls;lr",
            "sip:192.0.2.10:5061;transport=udp;lr",
            "sip:192.0.2.10:5061;lr",
            "sip:192.0.2.10:5061;transport=tls;transport=tls;lr",
            "sips:foreign.example:5061;lr",
        ] {
            let changed: Uri = changed.parse().unwrap();
            assert!(
                TlsProxyFlowBinding::admit_verified_outbound(&changed, &route).is_none(),
                "{changed}"
            );
            assert!(binding
                .route_for(&changed, route.destination, TransportType::Tls)
                .is_none());
        }
        assert!(binding
            .route_for(
                &next,
                "192.0.2.11:5061".parse().unwrap(),
                TransportType::Tls
            )
            .is_none());
        assert!(binding
            .route_for(&next, route.destination, TransportType::Tcp)
            .is_none());
        for kind in [TransportType::Tcp, TransportType::Udp] {
            let mut changed = route.clone();
            changed.transport_type = Some(kind);
            assert!(TlsProxyFlowBinding::admit_verified_outbound(&next, &changed).is_none());
        }
        let mut changed = route.clone();
        changed.flow_id = None;
        assert!(TlsProxyFlowBinding::admit_verified_outbound(&next, &changed).is_none());
        let mut changed = route.clone();
        changed.authority = None;
        assert!(TlsProxyFlowBinding::admit_verified_outbound(&next, &changed).is_none());
        // The DNS name the certificate was verified against also matches.
        assert!(TlsProxyFlowBinding::admit_verified_outbound(
            &"sips:carrier.example:5061".parse().unwrap(),
            &route
        )
        .is_some());
    }

    #[test]
    fn outbound_next_hop_prefers_learned_route_then_exact_preloaded_route_then_contact() {
        use rvoip_sip_core::types::route::Route;
        use rvoip_sip_core::{
            builder::SimpleRequestBuilder, HeaderName, HeaderValue, Method, TypedHeader,
        };
        let contact: Uri = "sips:callee@192.0.2.10:5061;transport=tls".parse().unwrap();
        let proxy: Uri = "sips:192.0.2.10:5061;lr".parse().unwrap();
        let mut original = SimpleRequestBuilder::new(Method::Invite, "sips:callee@192.0.2.10:5061")
            .unwrap()
            .build();
        assert_eq!(
            verified_outbound_next_hop(&[], &contact, &original),
            Some(contact.clone())
        );
        original
            .headers
            .push(TypedHeader::Route(Route::with_uri(proxy.clone())));
        assert_eq!(
            verified_outbound_next_hop(&[], &contact, &original),
            Some(proxy.clone())
        );
        let learned: Uri = "sips:192.0.2.20:5061;lr".parse().unwrap();
        assert_eq!(
            verified_outbound_next_hop(&[learned.clone()], &contact, &original),
            Some(learned)
        );
        let strict: Uri = "sips:192.0.2.10:5061".parse().unwrap();
        assert!(verified_outbound_next_hop(&[strict.clone()], &contact, &original).is_none());
        original.headers.clear();
        original
            .headers
            .push(TypedHeader::Route(Route::with_uri(strict)));
        assert!(verified_outbound_next_hop(&[], &contact, &original).is_none());
        original.headers.clear();
        original.headers.push(TypedHeader::Other(
            HeaderName::Route,
            HeaderValue::Raw(b"unusable".to_vec()),
        ));
        assert!(verified_outbound_next_hop(&[], &contact, &original).is_none());
        let not_invite = SimpleRequestBuilder::new(Method::Bye, "sips:callee@192.0.2.10:5061")
            .unwrap()
            .build();
        assert!(verified_outbound_next_hop(&[], &contact, &not_invite).is_none());
    }
}
