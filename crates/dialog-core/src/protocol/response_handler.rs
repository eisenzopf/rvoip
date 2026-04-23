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

use tracing::{debug, info, warn};

use rvoip_sip_core::Response;
use crate::transaction::TransactionKey;
use crate::dialog::{DialogId, DialogState};
use crate::errors::DialogResult;
use crate::events::SessionCoordinationEvent;
use crate::manager::{DialogManager, SessionCoordinator, MessageExtensions};

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
    async fn handle_response_message(&self, response: Response, transaction_id: TransactionKey) -> DialogResult<()> {
        debug!("Processing response {} for transaction {}", response.status_code(), transaction_id);

        // RFC 3581 NAT discovery — peek at the top Via for
        // `received=`/`rport=` and update our cache. This applies to
        // ALL inbound responses (dialog-bound or not), including
        // REGISTER 2xx/4xx and INVITE provisionals — so the
        // discovered address can populate before the first dialog is
        // established.
        record_nat_discovery_from_response(self, &response).await;

        // RFC 3608 — capture Service-Route from a REGISTER 2xx into the
        // per-AoR cache. Caller-facing surface is
        // `DialogManager::service_route_for_aor`.
        record_service_route_from_response(self, &response).await;

        // Find associated dialog
        if let Ok(dialog_id) = self.find_dialog_for_transaction(&transaction_id) {
            self.process_response_in_dialog(response, transaction_id, dialog_id).await
        } else {
            debug!("Response for transaction {} has no associated dialog", transaction_id);
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
    use crate::manager::MessageExtensions;

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
async fn record_nat_discovery_from_response(
    manager: &DialogManager,
    response: &Response,
) {
    let local_addr = manager.local_address;
    let Some(new_addr) = extract_nat_discovery(local_addr, response) else { return };

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
async fn record_service_route_from_response(
    manager: &DialogManager,
    response: &Response,
) {
    let Some((aor_key, uris)) = extract_service_route(response) else { return };

    let mut guard = manager.service_route_by_aor.write().await;
    let prev = guard.insert(aor_key.clone(), uris.clone());
    if prev.as_ref() != Some(&uris) {
        if uris.is_empty() {
            info!(
                "RFC 3608: registrar cleared Service-Route for AoR {}",
                aor_key
            );
        } else {
            info!(
                "RFC 3608: Service-Route learned for AoR {} ({} hop(s))",
                aor_key,
                uris.len()
            );
        }
    }
}

#[cfg(test)]
mod nat_discovery_tests {
    use super::extract_nat_discovery;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use rvoip_sip_core::types::{
        param::Param, status::StatusCode, via::{Via, ViaHeader}, TypedHeader, headers::HeaderAccess,
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
            sent_by_host: rvoip_sip_core::types::uri::Host::Address(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10))),
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
            Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7)), 54321))
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
        address::Address,
        cseq::CSeq,
        from::From as FromHdr,
        method::Method,
        param::Param,
        service_route::ServiceRoute,
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
        service_routes: Option<Vec<&str>>,
    ) -> Response {
        let mut response = Response::new(status);
        response
            .headers
            .push(TypedHeader::CSeq(CSeq::new(1, cseq_method)));
        let to_addr = Address::new(Uri::from_str(to_uri).unwrap());
        response
            .headers
            .push(TypedHeader::To(To::new(to_addr)));
        // Also need From to satisfy a typical response shape (not consulted
        // by the helper but keeps the fixture realistic).
        let from_addr = Address::new(Uri::from_str(to_uri).unwrap())
            .with_tag("abcd");
        response.headers.push(TypedHeader::From(FromHdr::new(from_addr)));
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
            Some(vec![
                "sip:orig1.example.com;lr",
                "sip:orig2.example.com;lr",
            ]),
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

    // Silence unused-import warnings when Param isn't needed in this mod.
    #[allow(dead_code)]
    fn _use_param(_: Param) {}
}

/// Response-specific helper methods for DialogManager
impl DialogManager {
    /// Process response within a dialog
    pub async fn process_response_in_dialog(
        &self,
        response: Response,
        _transaction_id: TransactionKey,
        dialog_id: DialogId,
    ) -> DialogResult<()> {
        debug!("Processing response {} for dialog {}", response.status_code(), dialog_id);
        
        // Update dialog state based on response
        {
            let mut dialog = self.get_dialog_mut(&dialog_id)?;

            if response.status_code() >= 200 && response.status_code() < 300 {
                // 2xx response - confirm dialog if in Early state
                if dialog.state == DialogState::Early {
                    if let Some(to_tag) = response.to().and_then(|to| to.tag()) {
                        dialog.confirm_with_tag(to_tag.to_string());
                        debug!("Confirmed dialog {} with 2xx response", dialog_id);
                    }
                }

                // RFC 4028 UAC: capture the negotiated Session-Expires from
                // a 2xx to INVITE. If the peer echoed `refresher=uac` (or
                // omitted it — RFC 4028 §7 default is uac when the UAC
                // originally requested uac) we are the refresher.
                if response.status_code() == 200 {
                    use rvoip_sip_core::types::TypedHeader;
                    use rvoip_sip_core::types::session_expires::Refresher;
                    if let Some(se) = response.headers.iter().find_map(|h| {
                        if let TypedHeader::SessionExpires(se) = h { Some(se) } else { None }
                    }) {
                        dialog.session_expires_secs = Some(se.delta_seconds);
                        dialog.is_session_refresher = matches!(
                            se.refresher,
                            None | Some(Refresher::Uac)
                        );
                    }
                }
            } else if response.status_code() >= 300 {
                // 3xx+ response - terminate dialog
                dialog.terminate();
                debug!("Terminated dialog {} due to final non-2xx response", dialog_id);
            }
        }

        // RFC 4028 UAC: dialog is now confirmed and we've captured the
        // negotiated interval. Spawn the refresh task if we're refresher.
        if response.status_code() == 200 {
            if let Ok(dlg) = self.get_dialog(&dialog_id) {
                if let Some(secs) = dlg.session_expires_secs {
                    let is_refresher = dlg.is_session_refresher;
                    drop(dlg);
                    crate::manager::session_timer::spawn_refresh_task(
                        self.clone(),
                        dialog_id.clone(),
                        secs,
                        is_refresher,
                    );
                }
            }
        }
        
        // Send appropriate session coordination event
        let event = if response.status_code() >= 200 && response.status_code() < 300 {
            SessionCoordinationEvent::CallAnswered {
                dialog_id: dialog_id.clone(),
                session_answer: response.body_string().unwrap_or_default(),
            }
        } else if response.status_code() >= 300 {
            SessionCoordinationEvent::CallTerminated {
                dialog_id: dialog_id.clone(),
                reason: format!("{} {}", response.status_code(), response.reason_phrase()),
            }
        } else {
            SessionCoordinationEvent::CallProgress {
                dialog_id: dialog_id.clone(),
                status_code: response.status_code(),
                reason_phrase: response.reason_phrase().to_string(),
            }
        };
        
        self.notify_session_layer(event).await?;
        debug!("Response processed for dialog {}", dialog_id);
        Ok(())
    }
    
    /// Handle provisional responses (1xx)
    pub async fn handle_provisional_response(
        &self,
        response: Response,
        _transaction_id: TransactionKey,
        dialog_id: DialogId,
    ) -> DialogResult<()> {
        debug!("Processing provisional response {} for dialog {}", response.status_code(), dialog_id);
        
        // Update dialog state for early dialogs
        let dialog_created = {
            let mut dialog = self.get_dialog_mut(&dialog_id)?;
            let old_state = dialog.state.clone();
            
            // For provisional responses with to-tag, create early dialog
            if let Some(to_header) = response.to() {
                if let Some(to_tag) = to_header.tag() {
                    if dialog.remote_tag.is_none() {
                        dialog.set_remote_tag(to_tag.to_string());
                        if dialog.state == DialogState::Initial {
                            dialog.state = DialogState::Early;
                            Some((old_state, dialog.state.clone()))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        };
        
        // Emit dialog state change if early dialog was created
        if let Some((old_state, new_state)) = dialog_created {
            self.emit_dialog_event(crate::events::DialogEvent::StateChanged {
                dialog_id: dialog_id.clone(),
                old_state,
                new_state,
            }).await;
        }
        
        // Handle specific provisional responses and emit session coordination events
        match response.status_code() {
            180 => {
                info!("Call ringing for dialog {}", dialog_id);
                
                self.notify_session_layer(SessionCoordinationEvent::CallRinging {
                    dialog_id: dialog_id.clone(),
                }).await?;
            },
            
            183 => {
                info!("Session progress for dialog {}", dialog_id);
                
                // Check for early media (SDP in 183)
                if !response.body().is_empty() {
                    let sdp = String::from_utf8_lossy(response.body()).to_string();
                    self.notify_session_layer(SessionCoordinationEvent::EarlyMedia {
                        dialog_id: dialog_id.clone(),
                        sdp,
                    }).await?;
                } else {
                    self.notify_session_layer(SessionCoordinationEvent::CallProgress {
                        dialog_id: dialog_id.clone(),
                        status_code: response.status_code(),
                        reason_phrase: response.reason_phrase().to_string(),
                    }).await?;
                }
            },
            
            _ => {
                debug!("Other provisional response {} for dialog {}", response.status_code(), dialog_id);
                
                // Emit general call progress event
                self.notify_session_layer(SessionCoordinationEvent::CallProgress {
                    dialog_id: dialog_id.clone(),
                    status_code: response.status_code(),
                    reason_phrase: response.reason_phrase().to_string(),
                }).await?;
            }
        }
        
        Ok(())
    }
    
    /// Handle successful responses (2xx)
    pub async fn handle_success_response(
        &self,
        response: Response,
        transaction_id: TransactionKey,
        dialog_id: DialogId,
    ) -> DialogResult<()> {
        info!("Processing success response {} for dialog {}", response.status_code(), dialog_id);
        
        // Update dialog state based on successful response
        let dialog_state_changed = {
            let mut dialog = self.get_dialog_mut(&dialog_id)?;
            let old_state = dialog.state.clone();
            
            // Update dialog with response information (remote tag, etc.)
            if let Some(to_header) = response.to() {
                if let Some(to_tag) = to_header.tag() {
                    dialog.set_remote_tag(to_tag.to_string());
                }
            }
            
            // Update dialog state based on response status and current state
            let state_changed = match response.status_code() {
                200 => {
                    if dialog.state == DialogState::Early {
                        dialog.state = DialogState::Confirmed;
                        
                        // CRITICAL FIX: Update dialog lookup now that we have both tags
                        if let Some(tuple) = dialog.dialog_id_tuple() {
                            let key = crate::manager::utils::DialogUtils::create_lookup_key(&tuple.0, &tuple.1, &tuple.2);
                            self.dialog_lookup.insert(key, dialog_id.clone());
                            info!("Updated dialog lookup for confirmed dialog {}", dialog_id);
                        }
                        
                        true
                    } else {
                        false
                    }
                },
                _ => false
            };
            
            if state_changed {
                Some((old_state, dialog.state.clone()))
            } else {
                None
            }
        };
        
        // Emit dialog events for session-core
        if let Some((old_state, new_state)) = dialog_state_changed {
            self.emit_dialog_event(crate::events::DialogEvent::StateChanged {
                dialog_id: dialog_id.clone(),
                old_state,
                new_state,
            }).await;
        }
        
        // Emit session coordination events for session-core
        self.notify_session_layer(SessionCoordinationEvent::ResponseReceived {
            dialog_id: dialog_id.clone(),
            response: response.clone(),
            transaction_id: transaction_id.clone(),
        }).await?;
        
        // Handle specific successful response types
        match response.status_code() {
            200 => {
                println!("🎯 RESPONSE HANDLER: Processing 200 OK, checking if INVITE response needs ACK");
                
                // For 200 OK responses to INVITE, automatically send ACK
                // Check if this is a response to an INVITE by looking at the transaction
                if let Some(original_request_method) = self.get_transaction_method(&transaction_id) {
                    if original_request_method == rvoip_sip_core::Method::Invite {
                        println!("🚀 RESPONSE HANDLER: This is a 200 OK to INVITE - sending automatic ACK");
                        
                        // Create and send ACK for this 2xx response
                        if let Err(e) = self.send_automatic_ack_for_2xx(&transaction_id, &response, &dialog_id).await {
                            warn!("Failed to send automatic ACK for 200 OK to INVITE: {}", e);
                        } else {
                            info!("Successfully sent automatic ACK for 200 OK to INVITE");

                            // Notify session-core that ACK was sent (for state machine transition)
                            // Extract SDP if present for final negotiation
                            let negotiated_sdp = if !response.body().is_empty() {
                                Some(String::from_utf8_lossy(response.body()).to_string())
                            } else {
                                None
                            };

                            if let Err(e) = self.notify_session_layer(SessionCoordinationEvent::AckSent {
                                dialog_id: dialog_id.clone(),
                                transaction_id: transaction_id.clone(),
                                negotiated_sdp,
                            }).await {
                                warn!("Failed to notify session layer of ACK sent: {}", e);
                            }
                        }
                    }
                }
                
                // Successful completion - could be call answered, request completed, etc.
                if !response.body().is_empty() {
                    let sdp = String::from_utf8_lossy(response.body()).to_string();
                    self.notify_session_layer(SessionCoordinationEvent::CallAnswered {
                        dialog_id: dialog_id.clone(),
                        session_answer: sdp,
                    }).await?;
                }
            },
            _ => {
                debug!("Other successful response {} for dialog {}", response.status_code(), dialog_id);
            }
        }
        
        Ok(())
    }
    
    /// Handle failure responses (4xx, 5xx, 6xx)
    pub async fn handle_failure_response(
        &self,
        response: Response,
        transaction_id: TransactionKey,
        dialog_id: DialogId,
    ) -> DialogResult<()> {
        warn!("Processing failure response {} for dialog {}", response.status_code(), dialog_id);
        
        // Handle specific failure cases and emit appropriate events
        match response.status_code() {
            487 => {
                // Request Terminated (CANCEL received)
                info!("Call cancelled for dialog {}", dialog_id);
                
                // Emit dialog event
                self.emit_dialog_event(crate::events::DialogEvent::Terminated {
                    dialog_id: dialog_id.clone(),
                    reason: "Request terminated".to_string(),
                }).await;
                
                // Emit session coordination event
                self.notify_session_layer(SessionCoordinationEvent::CallCancelled {
                    dialog_id: dialog_id.clone(),
                    reason: "Request terminated".to_string(),
                }).await?;
            },
            
            status if status >= 400 && status < 500 => {
                // Client error - may require dialog termination
                warn!("Client error {} for dialog {} - considering termination", status, dialog_id);
                
                // Emit session coordination event for failed requests
                self.notify_session_layer(SessionCoordinationEvent::RequestFailed {
                    dialog_id: Some(dialog_id.clone()),
                    transaction_id: transaction_id.clone(),
                    status_code: status,
                    reason_phrase: response.reason_phrase().to_string(),
                    method: "Unknown".to_string(), // TODO: Extract from transaction context
                }).await?;
            },
            
            status if status >= 500 => {
                // Server error - may require retry or termination
                warn!("Server error {} for dialog {} - considering retry", status, dialog_id);
                
                // Emit session coordination event for server errors
                self.notify_session_layer(SessionCoordinationEvent::RequestFailed {
                    dialog_id: Some(dialog_id.clone()),
                    transaction_id: transaction_id.clone(),
                    status_code: status,
                    reason_phrase: response.reason_phrase().to_string(),
                    method: "Unknown".to_string(), // TODO: Extract from transaction context
                }).await?;
            },
            
            _ => {
                debug!("Other failure response {} for dialog {}", response.status_code(), dialog_id);
            }
        }
        
        // Always emit the response received event for session-core to handle
        self.notify_session_layer(SessionCoordinationEvent::ResponseReceived {
            dialog_id: dialog_id.clone(),
            response: response.clone(),
            transaction_id: transaction_id.clone(),
        }).await?;
        
        Ok(())
    }
    
    /// Get the original request method for a transaction
    /// 
    /// This is a simplified implementation - in a real system this would
    /// query the transaction manager for the original request method.
    fn get_transaction_method(&self, transaction_id: &TransactionKey) -> Option<rvoip_sip_core::Method> {
        // Extract method from transaction key (simplified approach)
        // The transaction key typically contains the method information
        if transaction_id.to_string().contains("INVITE") {
            Some(rvoip_sip_core::Method::Invite)
        } else if transaction_id.to_string().contains("BYE") {
            Some(rvoip_sip_core::Method::Bye)
        } else {
            // For now, assume it's INVITE if we can't determine
            // In a real implementation, this would query the transaction manager
            Some(rvoip_sip_core::Method::Invite)
        }
    }
    
    /// Send automatic ACK for 2xx response to INVITE
    /// 
    /// Uses the existing dialog-core → transaction-core → transport architecture
    /// to properly send ACKs according to RFC 3261 while maintaining separation of concerns.
    async fn send_automatic_ack_for_2xx(
        &self,
        original_invite_tx_id: &TransactionKey,
        response: &Response,
        dialog_id: &DialogId,
    ) -> DialogResult<()> {
        debug!("Sending automatic ACK for 2xx response to INVITE using proper architecture");
        
        println!("📧 RESPONSE HANDLER: Using existing send_ack_for_2xx_response method");
        
        // Use the existing dialog-core method that properly delegates to transaction-core
        // This maintains separation of concerns: dialog-core → transaction-core → transport
        self.send_ack_for_2xx_response(dialog_id, original_invite_tx_id, response).await?;
        
        println!("✅ RESPONSE HANDLER: Successfully sent ACK for 2xx response via proper channels");
        Ok(())
    }
} 