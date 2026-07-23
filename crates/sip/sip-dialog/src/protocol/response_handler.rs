//! SIP Response Handler for Dialog-Core
//!
//! This module handles processing of SIP responses within dialogs according to RFC 3261.
//! It manages dialog state transitions based on response status codes and coordinates
//! with the session layer for proper call management.
//!
//! ## Response Categories Handled
//!
//! - **1xx Provisional**: Call progress, ringing, session progress
//! - **2xx Success**: Call answered, request completed successfully
//! - **3xx Redirection**: Call forwarding and redirect scenarios
//! - **4xx Client Error**: Authentication, not found, bad request
//! - **5xx Server Error**: Server failures and overload conditions
//! - **6xx Global Failure**: Permanent failures and rejections
//!
//! ## Dialog State Management
//!
//! - **180 Ringing**: May create early dialog with To-tag
//! - **200 OK INVITE**: Confirms dialog, transitions Early→Confirmed
//! - **4xx-6xx INVITE**: Terminates early dialogs
//! - **200 OK BYE**: Completes dialog termination

use tracing::{debug, info};

use crate::errors::DialogResult;
use crate::manager::DialogManager;
use crate::transaction::{TransactionEvent, TransactionKey};
use rvoip_sip_core::Response;

/// Returns true when a 401/407 carries the matching auth challenge header.
///
/// RFC 3261 §22.2 says these final responses are not terminal failures for a
/// UAC that can retry with credentials. The session layer must see them as
/// `AuthRequired`, not as a completed failed call.
pub(crate) fn response_has_auth_challenge(response: &Response) -> bool {
    use rvoip_sip_core::types::header::HeaderName;
    use rvoip_sip_core::types::headers::HeaderAccess;

    let header_name = match response.status_code() {
        401 => HeaderName::WwwAuthenticate,
        407 => HeaderName::ProxyAuthenticate,
        _ => return false,
    };

    response.raw_header_value(&header_name).is_some()
}

#[cfg(test)]
mod auth_challenge_classification_tests {
    use super::response_has_auth_challenge;
    use rvoip_sip_core::types::{
        auth::{ProxyAuthenticate, WwwAuthenticate},
        status::StatusCode,
        TypedHeader,
    };
    use rvoip_sip_core::Response;

    #[test]
    fn detects_401_with_www_authenticate_as_retryable_challenge() {
        let mut response = Response::new(StatusCode::Unauthorized);
        response
            .headers
            .push(TypedHeader::WwwAuthenticate(WwwAuthenticate::new(
                "asterisk", "nonce",
            )));

        assert!(response_has_auth_challenge(&response));
    }

    #[test]
    fn detects_407_with_proxy_authenticate_as_retryable_challenge() {
        let mut response = Response::new(StatusCode::ProxyAuthenticationRequired);
        response
            .headers
            .push(TypedHeader::ProxyAuthenticate(ProxyAuthenticate::new(
                "proxy", "nonce",
            )));

        assert!(response_has_auth_challenge(&response));
    }

    #[test]
    fn rejects_401_without_matching_challenge() {
        let response = Response::new(StatusCode::Unauthorized);

        assert!(!response_has_auth_challenge(&response));
    }

    #[test]
    fn rejects_non_auth_failure_with_auth_like_header() {
        let mut response = Response::new(StatusCode::Forbidden);
        response
            .headers
            .push(TypedHeader::WwwAuthenticate(WwwAuthenticate::new(
                "asterisk", "nonce",
            )));

        assert!(!response_has_auth_challenge(&response));
    }
}

/// Response-specific handling operations
pub trait ResponseHandler {
    /// Handle responses to client transactions
    fn handle_response_message(
        &self,
        response: Response,
        transaction_id: TransactionKey,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
}

/// Implementation of response handling for DialogManager
impl ResponseHandler for DialogManager {
    /// Handle responses to client transactions
    ///
    /// Processes responses and updates dialog state accordingly.
    async fn handle_response_message(
        &self,
        response: Response,
        transaction_id: TransactionKey,
    ) -> DialogResult<()> {
        debug!(
            "Processing response {} for transaction",
            response.status_code()
        );

        apply_response_protocol_metadata(self, &response, &transaction_id).await;

        // This public compatibility entry point delegates to the same
        // transaction-event state owner used by transport ingress. It does
        // not maintain a second response state machine.
        if let Ok(dialog_id) = self.find_dialog_for_transaction(&transaction_id) {
            let event = match response.status_code() {
                100..=199 => TransactionEvent::ProvisionalResponse {
                    transaction_id: transaction_id.clone(),
                    response,
                },
                200..=299 => TransactionEvent::SuccessResponse {
                    transaction_id: transaction_id.clone(),
                    response,
                    need_ack: transaction_id.method() == &rvoip_sip_core::Method::Invite,
                    // The canonical response processor does not use this
                    // compatibility-only source field. Real transport ingress
                    // supplies the actual peer address.
                    source: std::net::SocketAddr::from(([0, 0, 0, 0], 0)),
                },
                _ => TransactionEvent::FailureResponse {
                    transaction_id: transaction_id.clone(),
                    response,
                },
            };
            self.process_transaction_event(&transaction_id, &dialog_id, event)
                .await
        } else {
            debug!("Response for transaction has no associated dialog");
            Ok(())
        }
    }
}

/// Pure RFC 3581 §4 extraction — read `received=`/`rport=` from the
/// top `Via` of an inbound response and return the discovered public
/// `SocketAddr` if (a) both are present *and* (b) it differs from
/// the local bind address. Returns `None` otherwise (no NAT signal,
/// or NAT is a no-op).
///
/// Pure / sync so it's trivially unit-testable without standing up a
/// full DialogManager.
pub(crate) fn extract_nat_discovery(
    local_addr: std::net::SocketAddr,
    response: &Response,
) -> Option<std::net::SocketAddr> {
    let via = response.first_via()?;
    let received_ip = via.received()?;
    let rport = via.rport()??;

    if received_ip == local_addr.ip() && rport == local_addr.port() {
        // No NAT — discovered address matches what we already know.
        return None;
    }

    Some(std::net::SocketAddr::new(received_ip, rport))
}

/// Inspect the top `Via` header of an inbound response. If it carries
/// both `received=<ip>` and a populated `rport=<port>` (the carrier or
/// NAT echoed our externally-visible address per RFC 3581 §4), update
/// `DialogManager::nat_discovered_addr` with that observation.
///
/// Most-recent observation wins (single global slot — see field doc).
/// Free function rather than `DialogManager` method so it stays close
/// to the call site and doesn't pollute the manager's public surface.
async fn record_nat_discovery_from_response(manager: &DialogManager, response: &Response) {
    let local_addr = manager.local_address;
    let Some(new_addr) = extract_nat_discovery(local_addr, response) else {
        return;
    };

    let mut guard = manager.nat_discovered_addr.write().await;
    let prev = guard.replace(new_addr);
    if prev != Some(new_addr) {
        info!(
            "RFC 3581 NAT discovery: external address learned {} (local bind {})",
            new_addr, local_addr
        );
    }
}

/// RFC 3608 §5.1 extraction: for a 2xx response to a REGISTER, return
/// `(aor_key, service_route_uris)` where the AoR key is the To URI and
/// `service_route_uris` is the ordered list the registrar echoed on
/// `Service-Route:` headers (possibly empty if the registrar set no
/// route).
///
/// Returns `None` for any non-REGISTER response and for non-2xx.
/// Pure/sync so it's unit-testable without spinning up a manager.
pub(crate) fn extract_service_route(
    response: &Response,
) -> Option<(String, Vec<rvoip_sip_core::types::uri::Uri>)> {
    use rvoip_sip_core::types::{method::Method, TypedHeader};

    if !(200..300).contains(&response.status_code()) {
        return None;
    }

    // Only REGISTER responses carry Service-Route meaningfully (RFC 3608 §2).
    let is_register = response.headers.iter().any(|h| match h {
        TypedHeader::CSeq(cseq) => *cseq.method() == Method::Register,
        _ => false,
    });
    if !is_register {
        return None;
    }

    let aor_uri = response.to()?.uri().clone();
    let aor_key = aor_uri.to_string();

    // Collect every Service-Route header in order; a single logical list
    // MAY be split across multiple header instances per RFC 3261 §7.3.
    let uris: Vec<_> = response
        .headers
        .iter()
        .filter_map(|h| {
            if let TypedHeader::ServiceRoute(sr) = h {
                Some(sr.uris())
            } else {
                None
            }
        })
        .flatten()
        .collect();

    Some((aor_key, uris))
}

/// Inspect the response. If it's a 2xx to a REGISTER, capture the
/// registrar-returned Service-Route set into `DialogManager::service_route_by_aor`,
/// keyed by the AoR (To URI).
async fn record_service_route_from_response(manager: &DialogManager, response: &Response) {
    let Some((aor_key, uris)) = extract_service_route(response) else {
        return;
    };

    let mut guard = manager.service_route_by_aor.write().await;
    let prev = guard.insert(aor_key.clone(), uris.clone());
    if prev.as_ref() != Some(&uris) {
        if uris.is_empty() {
            info!("RFC 3608: registrar cleared Service-Route for AoR");
        } else {
            info!(
                "RFC 3608: Service-Route learned for AoR ({} hop(s))",
                uris.len()
            );
        }
    }
}

/// RFC 5627 §5.3 extraction: for a 2xx response to REGISTER, scan the
/// echoed `Contact:` headers for one that carries `pub-gruu` and/or
/// `temp-gruu` parameters. Returns `(aor, GruuContactParams)` where
/// `aor` is the To URI string. The struct's two fields are independent
/// — a registrar may set only one.
///
/// Returns `None` for non-REGISTER, non-2xx, no Contact, or a Contact
/// with neither GRUU parameter present (legacy registration).
/// Pure/sync for unit-testability.
pub(crate) fn extract_gruu(
    response: &Response,
) -> Option<(String, rvoip_sip_core::types::outbound::GruuContactParams)> {
    use rvoip_sip_core::types::{method::Method, TypedHeader};

    if !(200..300).contains(&response.status_code()) {
        return None;
    }

    let is_register = response.headers.iter().any(|h| match h {
        TypedHeader::CSeq(cseq) => *cseq.method() == Method::Register,
        _ => false,
    });
    if !is_register {
        return None;
    }

    let aor_key = response.to()?.uri().clone().to_string();

    // Any Contact entry that carries at least one GRUU param wins.
    // RFC 5627 §5.3 echoes the registrar-supplied URIs back on the
    // UA's own Contact, so we walk Contacts in order.
    for contact in response.headers.iter().filter_map(|h| match h {
        TypedHeader::Contact(c) => Some(c),
        _ => None,
    }) {
        for address in contact.addresses() {
            let params = rvoip_sip_core::types::outbound::read_gruu_contact_params(address);
            if params.pub_gruu.is_some() || params.temp_gruu.is_some() {
                return Some((aor_key, params));
            }
        }
    }

    None
}

/// Inspect the response. If it's a 2xx to a REGISTER and the echoed
/// Contact carries pub-gruu/temp-gruu, capture into
/// `DialogManager::gruu_by_aor` keyed by the AoR (To URI).
async fn record_gruu_from_response(manager: &DialogManager, response: &Response) {
    let Some((aor_key, params)) = extract_gruu(response) else {
        return;
    };

    let mut guard = manager.gruu_by_aor.write().await;
    let prev = guard.insert(aor_key.clone(), params.clone());
    if prev.as_ref() != Some(&params) {
        info!(
            "RFC 5627: GRUU learned for AoR (pub_gruu_present={}, temp_gruu_present={})",
            params.pub_gruu.is_some(),
            params.temp_gruu.is_some(),
        );
    }
}

/// RFC 5626 §4 extraction: for a 2xx response to REGISTER, scan the
/// echoed `Contact:` headers for one that carries both `+sip.instance`
/// and `reg-id` (the outbound-flow pair). Returns
/// `(aor, OutboundContactParams)` where `aor` is the To URI string.
///
/// Returns `None` for non-REGISTER or non-2xx responses, and for
/// REGISTER 2xx that didn't echo outbound params (legacy / non-5626
/// registrations). Pure/sync for unit-testability.
pub(crate) fn extract_outbound_flow(
    response: &Response,
) -> Option<(
    String,
    rvoip_sip_core::types::outbound::OutboundContactParams,
)> {
    use rvoip_sip_core::types::{method::Method, TypedHeader};

    if !(200..300).contains(&response.status_code()) {
        return None;
    }

    let is_register = response.headers.iter().any(|h| match h {
        TypedHeader::CSeq(cseq) => *cseq.method() == Method::Register,
        _ => false,
    });
    if !is_register {
        return None;
    }

    let aor_key = response.to()?.uri().clone().to_string();

    // Any Contact entry with both outbound params wins. RFC 5626 echoes
    // the UA-supplied Contact back verbatim, so the params the UA sent
    // are what we'll read here.
    for contact in response.headers.iter().filter_map(|h| match h {
        TypedHeader::Contact(c) => Some(c),
        _ => None,
    }) {
        for address in contact.addresses() {
            if let Some(params) =
                rvoip_sip_core::types::outbound::read_outbound_contact_params(address)
            {
                return Some((aor_key, params));
            }
        }
    }

    None
}

/// On REGISTER 2xx with outbound params echoed back, spawn (or refresh)
/// the RFC 5626 §3.5.1 CRLFCRLF keep-alive task targeting the
/// transaction's destination. No-op for non-outbound REGISTER
/// responses and when keep-alive is disabled in config.
async fn record_outbound_flow_from_response(
    manager: &DialogManager,
    response: &Response,
    transaction_id: &TransactionKey,
) {
    let Some((aor, params)) = extract_outbound_flow(response) else {
        return;
    };

    // Destination to ping is wherever we sent the REGISTER. The
    // transaction-destinations map in TransactionManager captured that
    // at send time.
    let Some(route) = manager
        .transaction_manager
        .transaction_route(transaction_id)
        .await
    else {
        debug!(
            "RFC 5626: REGISTER 2xx for AoR but no stored destination for transaction; skipping keep-alive"
        );
        return;
    };

    let key = (aor.clone(), params.reg_id, params.instance_urn.clone());
    let dest = route.destination;
    if manager.start_outbound_ping_on_route(key, route) {
        info!(
            "RFC 5626: keep-alive ping started for AoR (reg-id={}) → {}",
            params.reg_id, dest
        );
    }
}

/// Apply response-derived protocol metadata once, independently of whether a
/// dialog mapping exists. Transport ingress and the public response facade
/// both call this helper before entering the one transaction-event response
/// state machine.
pub(crate) async fn apply_response_protocol_metadata(
    manager: &DialogManager,
    response: &Response,
    transaction_id: &TransactionKey,
) {
    record_nat_discovery_from_response(manager, response).await;
    record_service_route_from_response(manager, response).await;
    record_gruu_from_response(manager, response).await;
    record_outbound_flow_from_response(manager, response, transaction_id).await;
}

#[cfg(test)]
mod nat_discovery_tests {
    use super::extract_nat_discovery;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use rvoip_sip_core::types::{
        param::Param,
        status::StatusCode,
        via::{Via, ViaHeader},
        TypedHeader,
    };
    use rvoip_sip_core::Response;

    fn local() -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)), 5060)
    }

    /// Build a 200 OK with a single `Via:` carrying the supplied
    /// params. Doesn't bother with any other headers — the discovery
    /// path only inspects Via.
    fn response_with_via(via_params: Vec<Param>) -> Response {
        let via = Via(vec![ViaHeader {
            sent_protocol: rvoip_sip_core::types::via::SentProtocol {
                name: "SIP".to_string(),
                version: "2.0".to_string(),
                transport: "UDP".to_string(),
            },
            sent_by_host: rvoip_sip_core::types::uri::Host::Address(IpAddr::V4(Ipv4Addr::new(
                192, 168, 1, 10,
            ))),
            sent_by_port: Some(5060),
            params: via_params,
        }]);
        let mut response = Response::new(StatusCode::Ok);
        response.headers.push(TypedHeader::Via(via));
        response
    }

    #[test]
    fn returns_some_when_received_and_rport_differ_from_local() {
        // `Via::received()` / `Via::rport()` only recognise the typed
        // variants (`Param::Received` / `Param::Rport`), not the
        // generic `Param::Other("received", …)` form.
        let response = response_with_via(vec![
            Param::Received(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7))),
            Param::Rport(Some(54321)),
        ]);
        let discovered = extract_nat_discovery(local(), &response);
        assert_eq!(
            discovered,
            Some(SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7)),
                54321
            ))
        );
    }

    #[test]
    fn returns_none_when_no_via() {
        let response = Response::new(StatusCode::Ok);
        assert_eq!(extract_nat_discovery(local(), &response), None);
    }

    #[test]
    fn returns_none_when_via_lacks_received() {
        let response = response_with_via(vec![Param::Rport(Some(54321))]);
        assert_eq!(extract_nat_discovery(local(), &response), None);
    }

    #[test]
    fn returns_none_when_via_lacks_rport_value() {
        // RFC 3581 — the response MUST echo `rport=<port>` (not just
        // a flag) for us to treat the discovery as actionable. A
        // `;rport` with no value (the request-side request flag) is
        // not enough.
        let response = response_with_via(vec![
            Param::Received(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7))),
            Param::Rport(None),
        ]);
        assert_eq!(extract_nat_discovery(local(), &response), None);
    }

    #[test]
    fn suppresses_update_when_nat_is_noop() {
        // Discovered address equals local bind → no NAT in path,
        // suppress the update to avoid log churn.
        let response = response_with_via(vec![
            Param::Received(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10))),
            Param::Rport(Some(5060)),
        ]);
        assert_eq!(extract_nat_discovery(local(), &response), None);
    }
}

#[cfg(test)]
mod service_route_tests {
    use super::extract_service_route;
    use rvoip_sip_core::types::{
        address::Address, cseq::CSeq, from::From as FromHdr, method::Method,
        service_route::ServiceRoute, status::StatusCode, to::To, uri::Uri, TypedHeader,
    };
    use rvoip_sip_core::Response;
    use std::str::FromStr;

    fn make_response(
        status: StatusCode,
        cseq_method: Method,
        to_uri: &str,
        service_routes: Option<Vec<&str>>,
    ) -> Response {
        let mut response = Response::new(status);
        response
            .headers
            .push(TypedHeader::CSeq(CSeq::new(1, cseq_method)));
        let to_addr = Address::new(Uri::from_str(to_uri).unwrap());
        response.headers.push(TypedHeader::To(To::new(to_addr)));
        // Also need From to satisfy a typical response shape (not consulted
        // by the helper but keeps the fixture realistic).
        let from_addr = Address::new(Uri::from_str(to_uri).unwrap()).with_tag("abcd");
        response
            .headers
            .push(TypedHeader::From(FromHdr::new(from_addr)));
        if let Some(uris) = service_routes {
            let mut sr = ServiceRoute::empty();
            for u in uris {
                sr.add_uri(Uri::from_str(u).unwrap());
            }
            response.headers.push(TypedHeader::ServiceRoute(sr));
        }
        response
    }

    #[test]
    fn extracts_service_route_on_register_200() {
        let response = make_response(
            StatusCode::Ok,
            Method::Register,
            "sip:alice@example.com",
            Some(vec!["sip:orig1.example.com;lr", "sip:orig2.example.com;lr"]),
        );
        let extracted = extract_service_route(&response).unwrap();
        assert_eq!(extracted.0, "sip:alice@example.com");
        assert_eq!(extracted.1.len(), 2);
        assert_eq!(extracted.1[0].to_string(), "sip:orig1.example.com;lr");
        assert_eq!(extracted.1[1].to_string(), "sip:orig2.example.com;lr");
    }

    #[test]
    fn returns_empty_vec_when_register_200_has_no_service_route() {
        // RFC 3608: registrar declined to set a Service-Route. Distinct
        // from "no registration yet" — callers use `Some(empty)` vs
        // `None` on the manager to tell these apart.
        let response = make_response(
            StatusCode::Ok,
            Method::Register,
            "sip:alice@example.com",
            None,
        );
        let extracted = extract_service_route(&response).unwrap();
        assert!(extracted.1.is_empty());
    }

    #[test]
    fn ignores_non_2xx_register_responses() {
        let response = make_response(
            StatusCode::Unauthorized,
            Method::Register,
            "sip:alice@example.com",
            Some(vec!["sip:orig.example.com;lr"]),
        );
        assert!(extract_service_route(&response).is_none());
    }

    #[test]
    fn ignores_non_register_responses() {
        // Service-Route carried on an INVITE 200 is out-of-spec; we
        // should not cache it as if it were a registrar-supplied set.
        let response = make_response(
            StatusCode::Ok,
            Method::Invite,
            "sip:bob@example.com",
            Some(vec!["sip:orig.example.com;lr"]),
        );
        assert!(extract_service_route(&response).is_none());
    }

    #[test]
    fn concatenates_multiple_service_route_headers() {
        // RFC 3261 §7.3 allows a logical list to be split across
        // multiple header instances. Concatenate in order.
        let mut response = make_response(
            StatusCode::Ok,
            Method::Register,
            "sip:alice@example.com",
            Some(vec!["sip:orig1.example.com;lr"]),
        );
        let mut sr2 = ServiceRoute::empty();
        sr2.add_uri(Uri::from_str("sip:orig2.example.com;lr").unwrap());
        response.headers.push(TypedHeader::ServiceRoute(sr2));

        let extracted = extract_service_route(&response).unwrap();
        assert_eq!(extracted.1.len(), 2);
        assert_eq!(extracted.1[0].to_string(), "sip:orig1.example.com;lr");
        assert_eq!(extracted.1[1].to_string(), "sip:orig2.example.com;lr");
    }
}

#[cfg(test)]
mod outbound_flow_tests {
    use super::extract_outbound_flow;
    use rvoip_sip_core::types::{
        address::Address,
        contact::{Contact, ContactParamInfo, ContactValue},
        cseq::CSeq,
        method::Method,
        outbound::{mark_uri_as_outbound, set_outbound_contact_params, OutboundContactParams},
        status::StatusCode,
        to::To,
        uri::Uri,
        TypedHeader,
    };
    use rvoip_sip_core::Response;
    use std::str::FromStr;

    fn make_response(
        status: StatusCode,
        cseq_method: Method,
        to_uri: &str,
        contact_addr: Option<Address>,
    ) -> Response {
        let mut response = Response::new(status);
        response
            .headers
            .push(TypedHeader::CSeq(CSeq::new(1, cseq_method)));
        response.headers.push(TypedHeader::To(To::new(Address::new(
            Uri::from_str(to_uri).unwrap(),
        ))));
        if let Some(addr) = contact_addr {
            let contact = Contact(vec![ContactValue::Params(vec![ContactParamInfo {
                address: addr,
            }])]);
            response.headers.push(TypedHeader::Contact(contact));
        }
        response
    }

    fn outbound_address(user_host: &str, instance_urn: &str, reg_id: u32) -> Address {
        let mut addr = Address::new(Uri::from_str(user_host).unwrap());
        mark_uri_as_outbound(&mut addr);
        set_outbound_contact_params(
            &mut addr,
            &OutboundContactParams {
                instance_urn: instance_urn.to_string(),
                reg_id,
            },
        );
        addr
    }

    #[test]
    fn extracts_outbound_flow_on_register_200_with_outbound_contact() {
        let response = make_response(
            StatusCode::Ok,
            Method::Register,
            "sip:alice@example.com",
            Some(outbound_address(
                "sip:alice@192.168.1.10:5060",
                "urn:uuid:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
                1,
            )),
        );
        let (aor, params) = extract_outbound_flow(&response).expect("outbound flow");
        assert_eq!(aor, "sip:alice@example.com");
        assert_eq!(params.reg_id, 1);
        assert_eq!(
            params.instance_urn,
            "urn:uuid:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
        );
    }

    #[test]
    fn returns_none_when_register_200_contact_lacks_outbound_params() {
        // Legacy (pre-5626) REGISTER — Contact without +sip.instance / reg-id.
        let contact = Address::new(Uri::from_str("sip:alice@192.168.1.10:5060").unwrap());
        let response = make_response(
            StatusCode::Ok,
            Method::Register,
            "sip:alice@example.com",
            Some(contact),
        );
        assert!(extract_outbound_flow(&response).is_none());
    }

    #[test]
    fn returns_none_for_non_register_responses() {
        let response = make_response(
            StatusCode::Ok,
            Method::Invite,
            "sip:bob@example.com",
            Some(outbound_address(
                "sip:bob@192.168.1.20:5060",
                "urn:uuid:bbbbbbbb",
                1,
            )),
        );
        assert!(extract_outbound_flow(&response).is_none());
    }

    #[test]
    fn returns_none_for_non_2xx_register() {
        let response = make_response(
            StatusCode::Unauthorized,
            Method::Register,
            "sip:alice@example.com",
            Some(outbound_address(
                "sip:alice@192.168.1.10:5060",
                "urn:uuid:a",
                1,
            )),
        );
        assert!(extract_outbound_flow(&response).is_none());
    }

    #[test]
    fn returns_none_when_no_contact_header() {
        let response = make_response(
            StatusCode::Ok,
            Method::Register,
            "sip:alice@example.com",
            None,
        );
        assert!(extract_outbound_flow(&response).is_none());
    }
}

#[cfg(test)]
mod gruu_tests {
    use super::extract_gruu;
    use rvoip_sip_core::types::{
        address::Address,
        contact::{Contact, ContactParamInfo, ContactValue},
        cseq::CSeq,
        method::Method,
        outbound::{set_gruu_contact_params, GruuContactParams},
        status::StatusCode,
        to::To,
        uri::Uri,
        TypedHeader,
    };
    use rvoip_sip_core::Response;
    use std::str::FromStr;

    fn make_response(
        status: StatusCode,
        cseq_method: Method,
        to_uri: &str,
        contact_addr: Option<Address>,
    ) -> Response {
        let mut response = Response::new(status);
        response
            .headers
            .push(TypedHeader::CSeq(CSeq::new(1, cseq_method)));
        response.headers.push(TypedHeader::To(To::new(Address::new(
            Uri::from_str(to_uri).unwrap(),
        ))));
        if let Some(addr) = contact_addr {
            let contact = Contact(vec![ContactValue::Params(vec![ContactParamInfo {
                address: addr,
            }])]);
            response.headers.push(TypedHeader::Contact(contact));
        }
        response
    }

    fn gruu_address(user_host: &str, pub_gruu: Option<&str>, temp_gruu: Option<&str>) -> Address {
        let mut addr = Address::new(Uri::from_str(user_host).unwrap());
        set_gruu_contact_params(
            &mut addr,
            &GruuContactParams {
                pub_gruu: pub_gruu.map(str::to_string),
                temp_gruu: temp_gruu.map(str::to_string),
            },
        );
        addr
    }

    #[test]
    fn extracts_both_gruu_on_register_200() {
        let response = make_response(
            StatusCode::Ok,
            Method::Register,
            "sip:alice@example.com",
            Some(gruu_address(
                "sip:alice@192.168.1.10:5060",
                Some("sip:alice@example.com;gr=urn:uuid:abc"),
                Some("sip:tgruu.7hs43@example.com;gr"),
            )),
        );
        let (aor, params) = extract_gruu(&response).expect("gruu extraction");
        assert_eq!(aor, "sip:alice@example.com");
        assert_eq!(
            params.pub_gruu.as_deref(),
            Some("sip:alice@example.com;gr=urn:uuid:abc")
        );
        assert_eq!(
            params.temp_gruu.as_deref(),
            Some("sip:tgruu.7hs43@example.com;gr")
        );
    }

    #[test]
    fn extracts_pub_only_when_temp_absent() {
        // Registrars MAY assign only pub-gruu — temp-gruu is independent
        // and a UA that didn't request privacy may not get one.
        let response = make_response(
            StatusCode::Ok,
            Method::Register,
            "sip:alice@example.com",
            Some(gruu_address(
                "sip:alice@192.168.1.10:5060",
                Some("sip:alice@example.com;gr=urn:uuid:abc"),
                None,
            )),
        );
        let (_, params) = extract_gruu(&response).expect("gruu extraction");
        assert!(params.pub_gruu.is_some());
        assert!(params.temp_gruu.is_none());
    }

    #[test]
    fn returns_none_when_contact_lacks_gruu() {
        // Pre-RFC-5627 Contact with no GRUU params — distinct from
        // "no Contact at all".
        let contact = Address::new(Uri::from_str("sip:alice@192.168.1.10:5060").unwrap());
        let response = make_response(
            StatusCode::Ok,
            Method::Register,
            "sip:alice@example.com",
            Some(contact),
        );
        assert!(extract_gruu(&response).is_none());
    }

    #[test]
    fn returns_none_for_non_register_responses() {
        let response = make_response(
            StatusCode::Ok,
            Method::Invite,
            "sip:bob@example.com",
            Some(gruu_address(
                "sip:bob@192.168.1.20:5060",
                Some("sip:bob@example.com;gr=urn:uuid:xyz"),
                None,
            )),
        );
        assert!(extract_gruu(&response).is_none());
    }

    #[test]
    fn returns_none_for_non_2xx_register() {
        let response = make_response(
            StatusCode::Unauthorized,
            Method::Register,
            "sip:alice@example.com",
            Some(gruu_address(
                "sip:alice@192.168.1.10:5060",
                Some("sip:alice@example.com;gr=urn:uuid:abc"),
                None,
            )),
        );
        assert!(extract_gruu(&response).is_none());
    }

    #[test]
    fn returns_none_when_no_contact_header() {
        let response = make_response(
            StatusCode::Ok,
            Method::Register,
            "sip:alice@example.com",
            None,
        );
        assert!(extract_gruu(&response).is_none());
    }
}

#[cfg(test)]
mod single_response_authority_tests {
    #[test]
    fn compatibility_response_entry_uses_transaction_event_authority() {
        let source = include_str!("response_handler.rs");
        let facade = source
            .split("impl ResponseHandler for DialogManager")
            .nth(1)
            .and_then(|tail| tail.split("/// Pure RFC 3581").next())
            .expect("response compatibility facade");

        assert!(facade.contains("self.process_transaction_event("));
        for duplicate in [
            "process_response_in_dialog",
            "handle_provisional_response",
            "handle_success_response",
            "handle_failure_response",
            "send_automatic_ack_for_2xx",
        ] {
            assert!(
                source.matches(duplicate).count() == 1,
                "duplicate response state writer returned: {duplicate}"
            );
        }
    }
}
