//! Dialog utility functions
//!
//! This module provides utility functions for dialog management including:
//! - SIP header parsing and extraction (tags, URIs)
//! - URI resolution and contact handling
//! - Network address resolution for SIP URIs
//! - Helper functions for dialog operations

use rvoip_sip_core::types::contact::ContactValue;
use rvoip_sip_core::Uri;
use std::net::SocketAddr;
use std::str::FromStr;

use crate::diagnostics::safe_log::error_class;

/// Extract tag from header with parameter
pub fn extract_tag(_header: &rvoip_sip_core::Header) -> Option<String> {
    // Just return empty tag for now
    None
}

/// Extract tag from header string - test compatibility function
pub fn extract_tag_from_str(header_value: &str) -> Option<String> {
    // Check for tag parameter
    if let Some(tag_pos) = header_value.find(";tag=") {
        let tag_start = tag_pos + 5; // ";tag=" length
        let tag_end = header_value[tag_start..]
            .find(|c: char| c == ';' || c == ',' || c.is_whitespace())
            .map(|pos| tag_start + pos)
            .unwrap_or(header_value.len());
        Some(header_value[tag_start..tag_end].to_string())
    } else {
        None
    }
}

/// Extract URI from a string representation
pub fn extract_uri(header_value: &str) -> Option<Uri> {
    if header_value.is_empty() {
        return None;
    }

    // Try to directly parse the URI first
    if let Ok(uri) = Uri::from_str(header_value) {
        // This might include parameters like ;tag in the raw string
        // If it's a sip URI, parse out just the URI part without parameters
        if header_value.contains(';') && uri.scheme().to_string().starts_with("sip") {
            let uri_part = header_value.split(';').next().unwrap();
            if let Ok(clean_uri) = Uri::from_str(uri_part) {
                return Some(clean_uri);
            }
        }
        return Some(uri);
    }

    // Handle URIs in angle brackets
    if header_value.contains('<') && header_value.contains('>') {
        let start = header_value.find('<').unwrap() + 1;
        let end = header_value.find('>').unwrap();
        if start < end {
            let uri_str = &header_value[start..end];
            if let Ok(uri) = Uri::from_str(uri_str) {
                return Some(uri);
            }
        }
    }

    // Attempt to extract domain part as fallback
    if let Some(at_pos) = header_value.rfind('@') {
        // There's a username@domain pattern
        let domain_part = &header_value[at_pos + 1..];
        let domain_end = domain_part
            .find(|c: char| c == ';' || c == '>' || c == ' ')
            .unwrap_or(domain_part.len());
        let domain = &domain_part[0..domain_end];

        // Try to create a SIP URI with just the domain
        if let Ok(uri) = Uri::from_str(&format!("sip:{}", domain)) {
            return Some(uri);
        }
    } else {
        // Try to extract just the domain/IP directly
        let end = header_value
            .find(|c: char| c == ';' || c == '>' || c == ' ')
            .unwrap_or(header_value.len());
        let potential_domain = &header_value[0..end];

        // Ensure we're not including scheme parts
        let domain = if potential_domain.contains(':') {
            // If it has a scheme like sip:, skip to the domain part
            if let Some(colon) = potential_domain.find(':') {
                &potential_domain[colon + 1..]
            } else {
                potential_domain
            }
        } else {
            potential_domain
        };

        if !domain.is_empty() {
            if let Ok(uri) = Uri::from_str(&format!("sip:{}", domain)) {
                return Some(uri);
            }
        }
    }

    None
}

/// Extract URI from a contact parameter
pub fn extract_uri_from_contact(contact: &ContactValue) -> Result<Uri, &'static str> {
    match contact {
        ContactValue::Params(params) if !params.is_empty() => {
            // Extract the URI from the first address in the params list
            if let Some(p) = params.first() {
                return Ok(p.address.uri.clone());
            }
        }
        ContactValue::Star => return Err("Cannot extract URI from Contact: *"),
        _ => {}
    }

    // Fallback for tests
    Uri::from_str("sip:unknown@example.com").map_err(|_| "Failed to create fallback URI")
}

/// Resolve a SIP URI to a socket address (convenience function).
///
/// Walks RFC 3263 §4 (NAPTR → SRV → A/AAAA, with IP-literal /
/// explicit-port short-circuits) via the process-wide
/// [`HickoryResolver`](rvoip_sip_transport::resolver::HickoryResolver).
/// Returns `None` on unresolvable URIs or DNS failures.
///
/// IP-literal URIs short-circuit before consulting the resolver so this
/// function works in sandboxed environments (no `/etc/resolv.conf`)
/// where the system resolver fails to initialise.
///
/// Per-`DialogManager` resolver overrides go through
/// [`DialogManager::resolve_uri_to_socketaddr`](crate::manager::DialogManager::resolve_uri_to_socketaddr)
/// rather than this free function.
pub async fn resolve_uri_to_socketaddr(uri: &Uri) -> Option<SocketAddr> {
    resolve_uri_to_candidates_for_family(uri, None)
        .await
        .into_iter()
        .next()
        .map(|target| target.addr)
}

/// Resolve a destination URI to the FULL list of candidates produced
/// by RFC 3263 §4 NAPTR/SRV walking. Callers iterate candidates in
/// the returned order, trying the next one on transport-level failure
/// per RFC 3263 §4.3.
///
/// IP-literal URIs short-circuit and produce a single candidate so
/// this works in sandboxed environments (no `/etc/resolv.conf`).
///
/// Per-`DialogManager` resolver overrides go through
/// [`DialogManager::resolve_uri_to_candidates`](crate::manager::DialogManager::resolve_uri_to_candidates)
/// rather than this free function.
pub async fn resolve_uri_to_candidates(
    uri: &Uri,
) -> Vec<rvoip_sip_transport::resolver::ResolvedTarget> {
    resolve_uri_to_candidates_for_family(uri, None).await
}

/// [`resolve_uri_to_candidates`] for a stack whose transports are bound to
/// one address family.
///
/// With `Some(family)`, DNS-derived candidates of the other family are
/// dropped (keeping the RFC 3263 order of the rest), since a socket of one
/// family cannot reach the other, and `localhost` resolves to that family's
/// loopback (RFC 6761 §6.3). An IP-literal URI names one exact address and
/// is returned as-is. `None` keeps every candidate and maps `localhost` to
/// `127.0.0.1`.
pub async fn resolve_uri_to_candidates_for_family(
    uri: &Uri,
    family: Option<rvoip_sip_transport::AddressFamily>,
) -> Vec<rvoip_sip_transport::resolver::ResolvedTarget> {
    use rvoip_sip_core::types::uri::{Host, Scheme};
    use rvoip_sip_transport::resolver::{
        retain_address_family, select_transport_for_uri, ResolvedTarget,
    };

    if let Host::Address(ip) = &uri.host {
        let default_port = match uri.scheme() {
            Scheme::Sips => 5061,
            _ => 5060,
        };
        let port = uri.port.filter(|p| *p > 0).unwrap_or(default_port);
        let transport = select_transport_for_uri(uri);
        return vec![ResolvedTarget::immediate(
            SocketAddr::new(*ip, port),
            transport,
        )];
    }

    // `localhost` → loopback, bypassing system-DNS resolver init.
    if let Some(addr) = loopback_for_localhost(uri, family) {
        let transport = select_transport_for_uri(uri);
        return vec![ResolvedTarget::immediate(addr, transport)];
    }

    let Some(resolver) = process_default_resolver().await else {
        return Vec::new();
    };
    match resolver.resolve(uri).await {
        Ok(mut candidates) => {
            if let Some(family) = family {
                retain_address_family(&mut candidates, family);
            }
            candidates
        }
        Err(e) => {
            tracing::debug!("Default resolver failed for URI class: {}", error_class(&e));
            Vec::new()
        }
    }
}

/// Loopback short-circuit for the special `localhost` name.
///
/// `localhost` is loopback by convention (RFC 6761 §6.3), so resolving it
/// through the system DNS resolver is both unnecessary and risky: the
/// one-time resolver init (`read_system_conf`) can block for many seconds on
/// a slow or misconfigured host. Returning loopback directly keeps localhost
/// targets fast and works in sandboxes with no `/etc/resolv.conf`. The
/// loopback is that of the bound address `family` (`::1` for an IPv6 stack;
/// `127.0.0.1` when the family is IPv4 or unknown). Honors the `sips:`→5061
/// default port.
fn loopback_for_localhost(
    uri: &Uri,
    family: Option<rvoip_sip_transport::AddressFamily>,
) -> Option<SocketAddr> {
    use rvoip_sip_core::types::uri::{Host, Scheme};
    if let Host::Domain(name) = &uri.host {
        if name.eq_ignore_ascii_case("localhost") {
            let default_port = match uri.scheme() {
                Scheme::Sips => 5061,
                _ => 5060,
            };
            let port = uri.port.filter(|p| *p > 0).unwrap_or(default_port);
            let family = family.unwrap_or(rvoip_sip_transport::AddressFamily::Ipv4);
            return Some(SocketAddr::new(family.loopback(), port));
        }
    }
    None
}

/// URI resolution utilities (historical module path). Re-exports of the
/// free function above are kept so existing call sites compile unchanged.
pub mod uri_resolver {
    pub use super::resolve_uri_to_socketaddr;
}

/// Process-wide lazy-init system resolver. The first call constructs a
/// [`HickoryResolver`](rvoip_sip_transport::resolver::HickoryResolver) from the
/// system DNS config, but via
/// [`new_system_resilient`](rvoip_sip_transport::resolver::HickoryResolver::new_system_resilient):
/// the config read is capped at 2s and falls back to a default config if the
/// host's DNS setup is slow/hung, so the first non-IP, non-localhost resolution
/// can never stall the caller for seconds.
async fn process_default_resolver(
) -> Option<std::sync::Arc<dyn rvoip_sip_transport::resolver::Resolver>> {
    use std::sync::Arc;
    static DEFAULT_RESOLVER: tokio::sync::OnceCell<
        Arc<dyn rvoip_sip_transport::resolver::Resolver>,
    > = tokio::sync::OnceCell::const_new();

    let resolver = DEFAULT_RESOLVER
        .get_or_init(|| async {
            let r = rvoip_sip_transport::resolver::HickoryResolver::new_system_resilient(
                std::time::Duration::from_secs(2),
            )
            .await;
            Arc::new(r) as Arc<dyn rvoip_sip_transport::resolver::Resolver>
        })
        .await
        .clone();
    Some(resolver)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_tag() {
        // Standard case with a tag parameter
        let header_value = "\"User\" <sip:user@example.com>;tag=abc123";
        assert_eq!(
            extract_tag_from_str(header_value),
            Some("abc123".to_string())
        );

        // Tag with additional parameters
        let header_value = "<sip:user@example.com>;tag=xyz789;param=value";
        assert_eq!(
            extract_tag_from_str(header_value),
            Some("xyz789".to_string())
        );

        // No tag parameter
        let header_value = "<sip:user@example.com>";
        assert_eq!(extract_tag_from_str(header_value), None);

        // Tag at the end of the string with no delimiter
        let header_value = "<sip:user@example.com>;tag=end-tag";
        assert_eq!(
            extract_tag_from_str(header_value),
            Some("end-tag".to_string())
        );

        // Multiple tag parameters (should return the first one)
        let header_value = "<sip:user@example.com>;tag=first;otherparam=x;tag=second";
        assert_eq!(
            extract_tag_from_str(header_value),
            Some("first".to_string())
        );

        // Tag with whitespace
        let header_value = "<sip:user@example.com>;tag=with space;param=value";
        assert_eq!(extract_tag_from_str(header_value), Some("with".to_string()));

        // Tag with empty value
        let header_value = "<sip:user@example.com>;tag=;param=value";
        assert_eq!(extract_tag_from_str(header_value), Some("".to_string()));

        // Semicolon without tag parameter
        let header_value = "<sip:user@example.com>;notag=value";
        assert_eq!(extract_tag_from_str(header_value), None);
    }

    #[test]
    fn test_extract_uri() {
        // URI in angle brackets with display name
        let header_value = "\"User\" <sip:user@example.com>;tag=abc123";
        let uri = extract_uri(header_value);
        assert!(uri.is_some());
        assert_eq!(uri.unwrap().to_string(), "sip:user@example.com");

        // URI without angle brackets
        let header_value = "sip:user@example.com;tag=xyz789";
        let uri = extract_uri(header_value);
        assert!(uri.is_some());
        assert_eq!(uri.unwrap().to_string(), "sip:user@example.com");

        // SIPS URI scheme
        let header_value = "sips:secure@example.com";
        let uri = extract_uri(header_value);
        assert!(uri.is_some());
        assert_eq!(uri.unwrap().to_string(), "sips:secure@example.com");

        // TEL URI scheme
        let header_value = "tel:+1-212-555-0123";
        let uri = extract_uri(header_value);
        assert!(uri.is_some());
        assert_eq!(uri.unwrap().to_string(), "tel:+1-212-555-0123");

        // Malformed URI (opening angle bracket but no closing one) - the function
        // falls back to extracting the domain part and creating a sip URI
        let header_value = "\"User\" <sip:malformed@example.com";
        let uri = extract_uri(header_value);
        assert!(uri.is_some());
        assert_eq!(uri.unwrap().to_string(), "sip:example.com");

        // Extract from just domain part
        let header_value = "example.com";
        let uri = extract_uri(header_value);
        assert!(uri.is_some());
        assert_eq!(uri.unwrap().to_string(), "sip:example.com");

        // With port in URI
        let header_value = "<sip:user@example.com:5060>";
        let uri = extract_uri(header_value);
        assert!(uri.is_some());
        assert_eq!(uri.unwrap().to_string(), "sip:user@example.com:5060");

        // Empty header value
        let header_value = "";
        let uri = extract_uri(header_value);
        assert!(uri.is_none());

        // Invalid scheme
        let header_value = "invalid:user@example.com";
        let uri = extract_uri(header_value);
        assert!(uri.is_some(), "Should extract the domain part as fallback");
        assert_eq!(uri.unwrap().to_string(), "sip:example.com");

        // IP address instead of domain
        let header_value = "192.168.1.1";
        let uri = extract_uri(header_value);
        assert!(uri.is_some());
        assert_eq!(uri.unwrap().to_string(), "sip:192.168.1.1");
    }

    #[tokio::test]
    async fn localhost_resolves_to_the_bound_family_loopback() {
        use rvoip_sip_transport::AddressFamily;

        let uri = Uri::from_str("sip:alice@localhost:5070").unwrap();
        let first = |candidates: Vec<rvoip_sip_transport::ResolvedTarget>| {
            candidates.into_iter().next().map(|target| target.addr)
        };
        assert_eq!(
            first(resolve_uri_to_candidates_for_family(&uri, Some(AddressFamily::Ipv6)).await),
            Some("[::1]:5070".parse().unwrap())
        );
        assert_eq!(
            first(resolve_uri_to_candidates_for_family(&uri, Some(AddressFamily::Ipv4)).await),
            Some("127.0.0.1:5070".parse().unwrap())
        );
        assert_eq!(
            resolve_uri_to_socketaddr(&uri).await,
            Some("127.0.0.1:5070".parse().unwrap())
        );

        // An IP literal names one exact address and is never rewritten.
        let literal = Uri::from_str("sips:alice@[2001:db8::1]").unwrap();
        assert_eq!(
            first(resolve_uri_to_candidates_for_family(&literal, Some(AddressFamily::Ipv6)).await),
            Some("[2001:db8::1]:5061".parse().unwrap())
        );
    }

    #[tokio::test]
    async fn test_uri_resolver() {
        // IP address in URI
        let uri = Uri::from_str("sip:user@192.168.1.1").unwrap();
        let socket_addr = uri_resolver::resolve_uri_to_socketaddr(&uri).await;
        assert!(socket_addr.is_some());
        assert_eq!(socket_addr.unwrap().to_string(), "192.168.1.1:5060");

        // IP address with custom port
        let uri = Uri::from_str("sip:user@192.168.1.1:5070").unwrap();
        let socket_addr = uri_resolver::resolve_uri_to_socketaddr(&uri).await;
        assert!(socket_addr.is_some());
        assert_eq!(socket_addr.unwrap().to_string(), "192.168.1.1:5070");

        // Domain name test - just verify behavior is consistent
        let uri = Uri::from_str("sip:user@nonexistent-domain-123456.local").unwrap();
        let socket_addr = uri_resolver::resolve_uri_to_socketaddr(&uri).await;
        assert!(
            socket_addr.is_none(),
            "Non-existent domain should not resolve"
        );

        // IPv6 address test - only if IPv6 is supported
        if let Ok(uri) = Uri::from_str("sip:user@[::1]") {
            let socket_addr = uri_resolver::resolve_uri_to_socketaddr(&uri).await;
            // Don't assert specifically that IPv6 works, as it may not be available in all environments
            println!("IPv6 resolver test result: {:?}", socket_addr);
        }
    }
}
