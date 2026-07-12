//! Transaction Integration for Dialog Management
//!
//! This module handles the integration between dialog-core and transaction-core,
//! managing transaction lifecycle events, request/response routing, and event processing.
//! It provides the bridge between SIP transaction reliability and dialog state management.
//!
//! ## Key Responsibilities
//!
//! - Processing transaction events and routing to appropriate dialogs
//! - Managing transaction-to-dialog associations
//! - Handling transaction completion and cleanup
//! - Converting between transaction and dialog abstractions
//! - Coordinating request sending through transaction layer

use super::core::DialogManager;
use crate::api::config::RelUsage;
use crate::dialog::DialogId;
use crate::errors::DialogResult;
use crate::events::{DialogEvent, SessionCoordinationEvent};
use crate::protocol::response_handler::response_has_auth_challenge;
use crate::transaction::builders::dialog_quick;
use crate::transaction::dialog::{request_builder_from_dialog_template, DialogRequestTemplate};
use crate::transaction::{TransactionEvent, TransactionKey, TransactionState};
use rvoip_sip_core::{Method, Request, Response};
use std::net::SocketAddr;
use tracing::{debug, error, info, warn};

/// Detect a reliable provisional response per RFC 3262.
///
/// Returns `Some(rseq)` when the response carries both `Require: 100rel`
/// and an `RSeq` header — meaning the UAC must PRACK it. Returns `None`
/// for unreliable provisionals.
pub fn detect_reliable_provisional(response: &Response) -> Option<u32> {
    use rvoip_sip_core::types::TypedHeader;

    let mut requires_100rel = false;
    let mut rseq_value: Option<u32> = None;

    for header in &response.headers {
        match header {
            TypedHeader::Require(req) if req.requires("100rel") => {
                requires_100rel = true;
            }
            TypedHeader::RSeq(rseq) => {
                rseq_value = Some(rseq.value);
            }
            _ => {}
        }
    }

    if requires_100rel {
        rseq_value
    } else {
        None
    }
}

/// Inspect a request's `Supported`/`Require` headers for the `100rel`
/// option tag. Returns `(supports, requires)` — `supports` is true when the
/// tag appears in either header (i.e., the peer has indicated 100rel
/// capability at minimum); `requires` is true only when the peer listed it
/// in `Require` (i.e., insists on it per RFC 3262 §4).
pub fn detect_peer_100rel_support(request: &Request) -> (bool, bool) {
    use rvoip_sip_core::types::TypedHeader;

    let mut supports = false;
    let mut requires = false;
    for header in &request.headers {
        match header {
            TypedHeader::Supported(sup) if sup.option_tags.iter().any(|t| t == "100rel") => {
                supports = true;
            }
            TypedHeader::Require(req) if req.requires("100rel") => {
                supports = true;
                requires = true;
            }
            _ => {}
        }
    }
    (supports, requires)
}

/// Inject the configured `100rel` option tag into an outgoing INVITE
/// (adds to existing `Supported`/`Require` headers if present).
///
/// `NotSupported` is a no-op — no header is added. `Supported` appends
/// `100rel` to any existing `Supported` header or creates one. `Required`
/// does the same for `Require`.
pub fn inject_100rel_policy(request: &mut Request, policy: RelUsage) {
    use rvoip_sip_core::types::{Require, Supported, TypedHeader};

    match policy {
        RelUsage::NotSupported => {}
        RelUsage::Supported => {
            let mut updated = false;
            for header in request.headers.iter_mut() {
                if let TypedHeader::Supported(ref mut sup) = header {
                    if !sup.option_tags.iter().any(|t| t == "100rel") {
                        sup.option_tags.push("100rel".to_string());
                    }
                    updated = true;
                    break;
                }
            }
            if !updated {
                request
                    .headers
                    .push(TypedHeader::Supported(Supported::new(vec![
                        "100rel".to_string()
                    ])));
            }
        }
        RelUsage::Required => {
            let mut updated = false;
            for header in request.headers.iter_mut() {
                if let TypedHeader::Require(ref mut req) = header {
                    if !req.requires("100rel") {
                        req.add_tag("100rel");
                    }
                    updated = true;
                    break;
                }
            }
            if !updated {
                request
                    .headers
                    .push(TypedHeader::Require(Require::with_tag("100rel")));
            }
        }
    }
}

/// Trait for transaction integration operations
pub trait TransactionIntegration {
    /// Send a request within a dialog using transaction-core
    fn send_request_in_dialog(
        &self,
        dialog_id: &DialogId,
        method: Method,
        body: Option<bytes::Bytes>,
    ) -> impl std::future::Future<Output = DialogResult<TransactionKey>> + Send;

    /// Send a response using transaction-core
    fn send_transaction_response(
        &self,
        transaction_id: &TransactionKey,
        response: Response,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
}

/// Trait for transaction helper operations
pub trait TransactionHelpers {
    /// Associate a transaction with a dialog
    fn link_transaction_to_dialog(&self, transaction_id: &TransactionKey, dialog_id: &DialogId);

    /// Create ACK for 2xx response using transaction-core helpers
    fn create_ack_for_success_response(
        &self,
        original_invite_tx_id: &TransactionKey,
        response: &Response,
    ) -> impl std::future::Future<Output = DialogResult<Request>> + Send;
}

// Actual implementations for DialogManager
impl TransactionIntegration for DialogManager {
    /// Send a request within a dialog using transaction-core
    ///
    /// Implements proper request creation within dialogs using Phase 3 dialog functions
    /// for significantly simplified and more maintainable code.
    async fn send_request_in_dialog(
        &self,
        dialog_id: &DialogId,
        method: Method,
        body: Option<bytes::Bytes>,
    ) -> DialogResult<TransactionKey> {
        self.send_request_in_dialog_with_extras(dialog_id, method, body, Vec::new())
            .await
    }

    /// Send a response using transaction-core
    async fn send_transaction_response(
        &self,
        transaction_id: &TransactionKey,
        response: Response,
    ) -> DialogResult<()> {
        Self::send_transaction_response_impl(self, transaction_id, response).await
    }
}

impl DialogManager {
    /// SIP_API_DESIGN_2 §7.2 — in-dialog request dispatch with
    /// application-staged `extra_headers` appended after the
    /// stack-managed slice. Used by every `send_*_with_options` path on
    /// `UnifiedDialogApi`. The legacy `send_request_in_dialog` (no
    /// extras) forwards to this with an empty Vec.
    pub async fn send_request_in_dialog_with_extras(
        &self,
        dialog_id: &DialogId,
        method: Method,
        body: Option<bytes::Bytes>,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> DialogResult<TransactionKey> {
        debug!(method=%crate::transaction::safe_diagnostics::SafeMethod::new(&method), dialog=%dialog_id, "Sending request using dialog functions");

        // Get dialog context and build the request. Destination is resolved
        // from the final request next hop after Route headers are present.
        let (fallback_destination, candidates, request) = {
            let mut dialog = self.get_dialog_mut(dialog_id)?;

            let fallback_destination =
                dialog.get_remote_target_address().await.ok_or_else(|| {
                    crate::errors::DialogError::routing_error("No remote target address available")
                })?;

            // Convert body to String if provided
            let body_string = body.map(|b| String::from_utf8_lossy(&b).to_string());

            // Create dialog template using the proper dialog method
            let template = dialog.create_request_template(method.clone());

            // Capture INVITE CSeq for later use by RAck (RFC 3262 §7.2). Applies
            // to both initial INVITE and re-INVITE — a re-INVITE can also produce
            // reliable provisionals, so the most recent INVITE CSeq is what counts.
            if method == Method::Invite {
                dialog.invite_cseq = Some(template.cseq_number);
            }

            // Read dialog-scoped fields needed by per-method request builders
            // BEFORE entering the match — the DashMap write lock held by
            // `dialog` would otherwise deadlock on any `self.get_dialog()` call
            // inside an arm (hit us on NOTIFY, which reads event_package +
            // subscription_state).
            let notify_event_package = dialog
                .event_package
                .clone()
                .unwrap_or_else(|| "dialog".to_string());
            let notify_subscription_state = dialog
                .subscription_state
                .as_ref()
                .map(|s| s.to_header_value());

            // Generate local tag if missing (for outgoing requests we should always have a local tag)
            let local_tag = match template.local_tag {
                Some(tag) if !tag.is_empty() => tag,
                _ => {
                    let new_tag = dialog.generate_local_tag();
                    dialog.local_tag = Some(new_tag.clone());
                    new_tag
                }
            };

            // Handle remote tag based on dialog state and method
            let remote_tag = match (&template.remote_tag, dialog.state.clone()) {
                // If we have a valid remote tag, use it
                (Some(tag), _) if !tag.is_empty() => Some(tag.clone()),

                // For certain methods in confirmed dialogs, remote tag is required
                (_, crate::dialog::DialogState::Confirmed) => {
                    error!(
                        dialog=%dialog_id,
                        method=%crate::transaction::safe_diagnostics::SafeMethod::new(&method),
                        has_local_tag=dialog.local_tag.is_some(),
                        has_remote_tag=dialog.remote_tag.is_some(),
                        "Confirmed dialog is missing remote tag for request"
                    );
                    return Err(crate::errors::DialogError::protocol_error(&format!(
                        "{} request in confirmed dialog missing remote tag",
                        method
                    )));
                }

                // For early/initial dialogs, remote tag may be None (will be set to None, not empty string)
                _ => None,
            };

            let local_address =
                self.local_address_for_target_and_routes(&template.target_uri, &template.route_set);

            // SIP_API_DESIGN_2 §7.2 — applications stage headers on the
            // builder; the dialog stack stamps Call-ID/CSeq/Via/From-tag
            // and appends application extras after that fixed prefix.
            // Empty Vec means legacy path (no extras to stamp).
            let extras_opt: Option<Vec<rvoip_sip_core::types::TypedHeader>> =
                if extra_headers.is_empty() {
                    None
                } else {
                    Some(extra_headers.clone())
                };

            // Build request using Phase 3 dialog quick functions (MUCH simpler!)
            let request = match method {
                Method::Invite => {
                    // Distinguish between initial INVITE and re-INVITE based on remote tag
                    match remote_tag {
                        Some(remote_tag) => {
                            // re-INVITE: We have a remote tag, so this is for an established dialog
                            // re-INVITE requires SDP content for session modification
                            let sdp_content = body_string.ok_or_else(|| {
                                crate::errors::DialogError::protocol_error("re-INVITE request requires SDP content for session modification")
                            })?;

                            dialog_quick::reinvite_for_dialog_with_extras(
                                &template.call_id,
                                &template.local_uri.to_string(),
                                &local_tag,
                                &template.remote_uri.to_string(),
                                &remote_tag,
                                &sdp_content,
                                template.cseq_number,
                                local_address,
                                if template.route_set.is_empty() { None } else { Some(template.route_set.clone()) },
                                self.local_contact_uri(),
                                extras_opt.clone(),
                            )
                        },
                        None => {
                            // Initial INVITE: No remote tag yet, creating new dialog
                            use crate::transaction::client::builders::InviteBuilder;

                            let mut invite_builder = InviteBuilder::new()
                                .from_detailed(
                                    Some("User"), // Display name
                                    template.local_uri.to_string(),
                                    Some(&local_tag)
                                )
                                .to_detailed(
                                    Some("User"), // Display name
                                    template.remote_uri.to_string(),
                                    None // No remote tag for initial INVITE
                                )
                                .call_id(&template.call_id)
                                .cseq(template.cseq_number)
                                .request_uri(template.target_uri.to_string())
                                .local_address(local_address);

                            // Add route set if present
                            for route in &template.route_set {
                                invite_builder = invite_builder.add_route(route.clone());
                            }

                            if let Some(contact) = self.local_contact_uri() {
                                invite_builder = invite_builder.contact(contact);
                            }

                            // Add SDP content if provided
                            if let Some(sdp_content) = body_string {
                                invite_builder = invite_builder.with_sdp(sdp_content);
                            }

                            // SIP_API_DESIGN_2 §5.2 — extras after stack-managed prefix.
                            invite_builder.build().map(|mut request| {
                                for hdr in extra_headers.iter().cloned() {
                                    request.headers.push(hdr);
                                }
                                request
                            })
                        }
                    }
                },

                Method::Bye => {
                    // BYE requires both tags in established dialogs
                    let remote_tag = remote_tag.ok_or_else(|| {
                        crate::errors::DialogError::protocol_error("BYE request requires remote tag in established dialog")
                    })?;

                    dialog_quick::bye_for_dialog_with_request_uri(
                        &template.call_id,
                        &template.local_uri.to_string(),
                        &local_tag,
                        &template.remote_uri.to_string(),
                        &remote_tag,
                        &template.target_uri.to_string(),
                        template.cseq_number,
                        local_address,
                        if template.route_set.is_empty() { None } else { Some(template.route_set.clone()) },
                        extras_opt.clone(),
                    )
                },

                Method::Refer => {
                    // REFER requires both tags in established dialogs
                    let remote_tag = remote_tag.ok_or_else(|| {
                        crate::errors::DialogError::protocol_error("REFER request requires remote tag in established dialog")
                    })?;

                    // Extract the target URI from the body if it's in the old format ("Refer-To: <uri>")
                    // Otherwise use it directly as the target URI
                    let target_uri = if let Some(body) = body_string.clone() {
                        // Check if it's in the old format with "Refer-To: " prefix
                        if body.starts_with("Refer-To: ") {
                            body.trim_start_matches("Refer-To: ").trim_end_matches("\r\n").to_string()
                        } else {
                            body
                        }
                    } else {
                        "sip:unknown".to_string()
                    };

                    dialog_quick::refer_for_dialog_with_extras(
                        &template.call_id,
                        &template.local_uri.to_string(),
                        &local_tag,
                        &template.remote_uri.to_string(),
                        &remote_tag,
                        &target_uri,
                        template.cseq_number,
                        local_address,
                        if template.route_set.is_empty() { None } else { Some(template.route_set.clone()) },
                        self.local_contact_uri(),
                        extras_opt.clone(),
                    )
                },

                Method::Update => {
                    // UPDATE requires both tags in established dialogs
                    let remote_tag = remote_tag.ok_or_else(|| {
                        crate::errors::DialogError::protocol_error("UPDATE request requires remote tag in established dialog")
                    })?;

                    dialog_quick::update_for_dialog_with_extras(
                        &template.call_id,
                        &template.local_uri.to_string(),
                        &local_tag,
                        &template.remote_uri.to_string(),
                        &remote_tag,
                        body_string.clone(),
                        template.cseq_number,
                        local_address,
                        if template.route_set.is_empty() { None } else { Some(template.route_set.clone()) },
                        self.local_contact_uri(),
                        extras_opt.clone(),
                    )
                },

                Method::Info => {
                    // INFO requires both tags in established dialogs
                    let remote_tag = remote_tag.ok_or_else(|| {
                        crate::errors::DialogError::protocol_error("INFO request requires remote tag in established dialog")
                    })?;

                    let content = body_string.unwrap_or_else(|| "Application info".to_string());
                    dialog_quick::info_for_dialog_with_extras(
                        &template.call_id,
                        &template.local_uri.to_string(),
                        &local_tag,
                        &template.remote_uri.to_string(),
                        &remote_tag,
                        &content,
                        Some("application/info".to_string()),
                        template.cseq_number,
                        local_address,
                        if template.route_set.is_empty() { None } else { Some(template.route_set.clone()) },
                        extras_opt.clone(),
                    )
                },

                Method::Notify => {
                    // NOTIFY requires both tags in established dialogs
                    let remote_tag = remote_tag.ok_or_else(|| {
                        crate::errors::DialogError::protocol_error("NOTIFY request requires remote tag in established dialog")
                    })?;

                    dialog_quick::notify_for_dialog_with_extras(
                        &template.call_id,
                        &template.local_uri.to_string(),
                        &local_tag,
                        &template.remote_uri.to_string(),
                        &remote_tag,
                        &notify_event_package,
                        body_string,
                        notify_subscription_state.clone(),
                        None, // content_type — legacy path infers from event package
                        template.cseq_number,
                        local_address,
                        if template.route_set.is_empty() { None } else { Some(template.route_set.clone()) },
                        extras_opt.clone(),
                    )
                },

                Method::Message => {
                    // MESSAGE requires both tags in established dialogs
                    let remote_tag = remote_tag.ok_or_else(|| {
                        crate::errors::DialogError::protocol_error("MESSAGE request requires remote tag in established dialog")
                    })?;

                    let content = body_string.unwrap_or_else(|| "".to_string());
                    dialog_quick::message_for_dialog_with_extras(
                        &template.call_id,
                        &template.local_uri.to_string(),
                        &local_tag,
                        &template.remote_uri.to_string(),
                        &remote_tag,
                        &content,
                        Some("text/plain".to_string()),
                        template.cseq_number,
                        local_address,
                        if template.route_set.is_empty() { None } else { Some(template.route_set.clone()) },
                        extras_opt.clone(),
                    )
                },

                _ => {
                    // For any other method, require established dialog
                    let remote_tag = remote_tag.ok_or_else(|| {
                        crate::errors::DialogError::protocol_error(&format!("{} request requires remote tag in established dialog", method))
                    })?;

                    let contact = if matches!(method, Method::Update | Method::Refer | Method::Subscribe | Method::Notify) {
                        self.local_contact_uri()
                    } else {
                        None
                    };

                    // Use dialog template + utility function
                    let template_struct = DialogRequestTemplate {
                        call_id: template.call_id,
                        from_uri: template.local_uri.to_string(),
                        from_tag: local_tag,
                        to_uri: template.remote_uri.to_string(),
                        to_tag: remote_tag,
                        request_uri: template.target_uri.to_string(),
                        cseq: template.cseq_number,
                        local_address,
                        route_set: template.route_set.clone(),
                        contact,
                    };

                    request_builder_from_dialog_template(
                        &template_struct,
                        method.clone(),
                        body_string,
                        None, // Auto-detect content type
                        extras_opt.clone(),
                    )
                }
            }.map_err(|e| crate::errors::DialogError::InternalError {
                message: format!("Failed to build {} request using Phase 3 dialog functions: {}", method, e),
                context: None,
            })?;

            let mut request = request;
            // RFC 3262: advertise or demand the `100rel` extension on outgoing
            // INVITEs per dialog config. Applies to both initial and re-INVITE.
            if method == Method::Invite {
                inject_100rel_policy(&mut request, self.config_100rel_policy());
                // RFC 4028: advertise session timers. Only emitted when the
                // config has `session_timer_secs = Some(_)`.
                if let Some((secs, min_se)) = self.config_session_timer_settings() {
                    inject_session_timer_headers(&mut request, secs, min_se);
                }
            }

            let candidates = self
                .resolve_uri_to_candidates(
                    &crate::transaction::transport::multiplexed::next_hop_uri_for_request(&request),
                )
                .await;

            (fallback_destination, candidates, request)
        };

        // RFC 3263 §4.3 multi-candidate failover. STIR/SHAKEN signing
        // (`pre_send_request`) and the RFC 3261 §17.1.1.3 benign-
        // terminate-after-2xx handling both live in the helper — only
        // INVITE gets the benign-terminate suppression so non-INVITE
        // methods (BYE / REFER / UPDATE / etc.) still surface real
        // transport failures.
        let (transaction_id, _addr) = self
            .send_request_with_candidate_failover(
                request,
                candidates,
                fallback_destination,
                Some(dialog_id),
            )
            .await?;

        debug!(
            method=%crate::transaction::safe_diagnostics::SafeMethod::new(&method),
            dialog=%dialog_id,
            transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id),
            "Sent request via candidate failover path"
        );

        Ok(transaction_id)
    }

    /// Send a response using transaction-core
    ///
    /// Delegates response sending to transaction-core while maintaining dialog state.
    /// Reliable-provisional wrapping (RFC 3262 §3) is applied here: a 1xx
    /// response with a body on a dialog whose peer advertised `100rel` is
    /// rewritten with `Require: 100rel` + `RSeq: <n>` and retransmitted with
    /// T1 backoff until PRACK acknowledges it.
    pub async fn send_transaction_response_impl(
        &self,
        transaction_id: &TransactionKey,
        mut response: Response,
    ) -> DialogResult<()> {
        debug!(status=response.status_code(), transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id), "Sending response");

        // RFC 4028: echo Session-Expires on 2xx to INVITE so the UAC learns
        // the negotiated interval + refresher assignment.
        if response.status_code() == 200 {
            if let Some(dialog_id_ref) = self.transaction_to_dialog.get(transaction_id) {
                let dialog_id = dialog_id_ref.clone();
                drop(dialog_id_ref);
                if let Ok(dialog) = self.get_dialog(&dialog_id) {
                    if let Some(secs) = dialog.session_expires_secs {
                        let refresher = if dialog.is_session_refresher {
                            rvoip_sip_core::types::session_expires::Refresher::Uas
                        } else {
                            rvoip_sip_core::types::session_expires::Refresher::Uac
                        };
                        let already_has = response.headers.iter().any(|h| {
                            matches!(h, rvoip_sip_core::types::TypedHeader::SessionExpires(_))
                        });
                        if !already_has {
                            response.headers.push(
                                rvoip_sip_core::types::TypedHeader::SessionExpires(
                                    rvoip_sip_core::types::session_expires::SessionExpires::new(
                                        secs,
                                        Some(refresher),
                                    ),
                                ),
                            );
                        }
                        let supports_has_timer = response.headers.iter().any(|h| matches!(h, rvoip_sip_core::types::TypedHeader::Require(r) if r.requires("timer")));
                        if !supports_has_timer {
                            response
                                .headers
                                .push(rvoip_sip_core::types::TypedHeader::Require(
                                    rvoip_sip_core::types::Require::with_tag("timer"),
                                ));
                        }
                    }
                }
            }
        }

        let mut reliable_spawn: Option<(DialogId, u32, Response)> = None;
        if should_send_reliably(&response) {
            if let Some(dialog_id_ref) = self.transaction_to_dialog.get(transaction_id) {
                let dialog_id = dialog_id_ref.clone();
                drop(dialog_id_ref);

                let our_policy = self.config_100rel_policy();
                let rseq_opt = match self.get_dialog_mut(&dialog_id) {
                    Ok(mut dialog) => {
                        if dialog.peer_supports_100rel
                            && !matches!(our_policy, RelUsage::NotSupported)
                        {
                            Some(dialog.next_local_rseq())
                        } else {
                            None
                        }
                    }
                    Err(_) => None,
                };

                if let Some(rseq) = rseq_opt {
                    inject_reliable_provisional_headers(&mut response, rseq);
                    reliable_spawn = Some((dialog_id, rseq, response.clone()));
                    debug!(
                        "Wrapping 18x {} as reliable (policy={:?}, rseq={})",
                        response.status_code(),
                        our_policy,
                        rseq
                    );
                }
            }
        }

        // Use transaction-core to send the response
        self.transaction_manager
            .send_response(transaction_id, response)
            .await
            .map_err(|e| crate::errors::DialogError::TransactionError {
                message: format!("Failed to send response: {}", e),
            })?;

        if let Some((dialog_id, rseq, stored_response)) = reliable_spawn {
            crate::transaction::server::reliable_invite::spawn_reliable_provisional_retransmit(
                dialog_id,
                rseq,
                transaction_id.clone(),
                stored_response,
                self.transaction_manager.clone(),
                self.reliable_provisional_tasks.clone(),
            );
        }

        debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id), "Successfully sent response");
        Ok(())
    }
}

/// RFC 3261 §22.2 — resend an INVITE with an `Authorization` or
/// `Proxy-Authorization` header after the UAS/proxy challenged with 401/407.
///
/// The local UAC request template is reused so the retry keeps the same
/// `Call-ID` and `From` tag, has no remote tag, and bumps CSeq on a new client
/// transaction. On the UAS side, the 401/407 final response terminates the
/// early dialog, so this retry must be routed as a fresh initial INVITE rather
/// than a re-INVITE. The caller supplies the fully-formatted auth header value.
impl DialogManager {
    /// Find the newest outbound INVITE transaction for a dialog.
    ///
    /// A challenged initial INVITE (401/407) and its authenticated retry share
    /// the same dialog record but are different transactions. RFC 3261 CANCEL
    /// must target the currently pending INVITE transaction, so prefer the
    /// mapped INVITE with the highest CSeq instead of returning an arbitrary
    /// DashMap entry.
    pub async fn find_latest_invite_transaction_for_dialog(
        &self,
        dialog_id: &DialogId,
    ) -> Option<TransactionKey> {
        use rvoip_sip_core::types::cseq::CSeq;

        let candidates: Vec<TransactionKey> = self
            .dialog_invite_transactions
            .get(dialog_id)
            .map(|entry| {
                entry
                    .iter()
                    .filter(|tx_key| {
                        !tx_key.is_server()
                            && self
                                .transaction_to_dialog
                                .get(tx_key)
                                .is_some_and(|mapped| mapped.value() == dialog_id)
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();

        let mut best: Option<(u32, TransactionKey)> = None;
        for tx_key in candidates {
            let cseq = self
                .transaction_manager
                .original_request(&tx_key)
                .await
                .ok()
                .flatten()
                .and_then(|request| request.typed_header::<CSeq>().map(|cseq| cseq.seq))
                .unwrap_or_default();

            match &best {
                Some((best_cseq, _)) if cseq < *best_cseq => {}
                _ => best = Some((cseq, tx_key)),
            }
        }

        if let Some((cseq, tx_key)) = best {
            debug!(
                "Selected latest INVITE transaction {} for dialog {} using CSeq {}",
                tx_key, dialog_id, cseq
            );
            Some(tx_key)
        } else {
            debug!(
                "No outbound INVITE transaction found for dialog {}",
                dialog_id
            );
            None
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn send_invite_with_auth(
        &self,
        dialog_id: &DialogId,
        body: Option<bytes::Bytes>,
        auth_header_name: &str,
        auth_header_value: String,
        extras: Vec<rvoip_sip_core::types::TypedHeader>,
        from_display: Option<String>,
        contact_override: Option<String>,
    ) -> DialogResult<TransactionKey> {
        use crate::transaction::client::builders::InviteBuilder;
        use rvoip_sip_core::types::TypedHeader;

        debug!("Resending INVITE with auth for dialog {}", dialog_id);

        let (fallback_destination, candidates, request) = {
            let mut dialog = self.get_dialog_mut(dialog_id)?;

            let fallback_destination =
                dialog.get_remote_target_address().await.ok_or_else(|| {
                    crate::errors::DialogError::routing_error(
                        "No remote target address available for auth retry",
                    )
                })?;

            let body_string = body.map(|b| String::from_utf8_lossy(&b).to_string());

            let template = dialog.create_request_template(Method::Invite);

            // Preserve the new INVITE's CSeq for later use by RAck (RFC 3262 §7.2).
            dialog.invite_cseq = Some(template.cseq_number);

            let local_tag = match template.local_tag.clone() {
                Some(tag) if !tag.is_empty() => tag,
                _ => {
                    let new_tag = dialog.generate_local_tag();
                    dialog.local_tag = Some(new_tag.clone());
                    new_tag
                }
            };

            // The challenge was a final response on the original INVITE, so no
            // remote tag was established. Rebuild as an initial INVITE with
            // the same Call-ID (dialog.create_request_template carries it).
            let local_address =
                self.local_address_for_target_and_routes(&template.target_uri, &template.route_set);
            let mut invite_builder = InviteBuilder::new()
                .from_detailed(
                    from_display.as_deref().or(Some("User")),
                    template.local_uri.to_string(),
                    Some(&local_tag),
                )
                .to_detailed(Some("User"), template.remote_uri.to_string(), None)
                .call_id(&template.call_id)
                .cseq(template.cseq_number)
                .request_uri(template.target_uri.to_string())
                .local_address(local_address);

            for route in &template.route_set {
                invite_builder = invite_builder.add_route(route.clone());
            }

            // Contact precedence mirrors the initial-INVITE path so the retry
            // wire form matches: structured override wins, else a Contact
            // smuggled through `extras`, else the local UA binding. Partition
            // any Contact out of `extras` so it is never appended twice
            // (InviteBuilder emits exactly one).
            let (caller_contact, extras): (Vec<TypedHeader>, Vec<TypedHeader>) = extras
                .into_iter()
                .partition(|h| matches!(h, TypedHeader::Contact(_)));
            let override_contact_uri = contact_override.or_else(|| {
                caller_contact.into_iter().find_map(|h| match h {
                    TypedHeader::Contact(c) => c.address().map(|addr| addr.uri.to_string()),
                    _ => None,
                })
            });
            if let Some(uri) = override_contact_uri {
                invite_builder = invite_builder.contact(uri);
            } else if let Some(contact) = self.local_contact_uri() {
                invite_builder = invite_builder.contact(contact);
            }

            if let Some(sdp_content) = body_string {
                invite_builder = invite_builder.with_sdp(sdp_content);
            }

            let mut request =
                invite_builder
                    .build()
                    .map_err(|e| crate::errors::DialogError::InternalError {
                        message: format!("Failed to build auth-retry INVITE: {}", e),
                        context: None,
                    })?;

            // Re-inject the negotiated policy headers (100rel, session-timer)
            // just like the initial send does.
            inject_100rel_policy(&mut request, self.config_100rel_policy());
            if let Some((secs, min_se)) = self.config_session_timer_settings() {
                inject_session_timer_headers(&mut request, secs, min_se);
            }

            // Attach the digest authorization header. Use TypedHeader::Other
            // with Raw bytes so we don't have to round-trip through a typed
            // Authorization parser — the server only needs to read the string.
            let header_name = rvoip_sip_core::validation::authorization_header_name(
                auth_header_name,
            )
            .map_err(|_| {
                crate::errors::DialogError::protocol_error(
                    "unsupported INVITE authorization header name",
                )
            })?;
            let authorization = rvoip_sip_core::validation::validated_authorization_header(
                header_name,
                auth_header_value,
            )
            .map_err(|_| {
                crate::errors::DialogError::protocol_error(
                    "INVITE authorization failed wire-safety validation",
                )
            })?;
            request.headers.push(authorization);

            // SIP_API_DESIGN_2 §7.3 — preserve application-staged extras
            // across the 401/407 → retry hop. The original INVITE's
            // extras live in `pending_invite_options` on session-core's
            // SessionState; the caller forwards them here so the retry
            // wire form matches the initial send.
            for extra in extras {
                request.headers.push(extra);
            }

            let candidates = self
                .resolve_uri_to_candidates(
                    &crate::transaction::transport::multiplexed::next_hop_uri_for_request(&request),
                )
                .await;

            (fallback_destination, candidates, request)
        };

        // RFC 3263 §4.3 multi-candidate failover. The auth-retry path
        // re-signs the PASSporT per attempt (fresh Via/branch) — the
        // helper fires `pre_send_request` inside the retry loop.
        let (transaction_id, _addr) = self
            .send_request_with_candidate_failover(
                request,
                candidates,
                fallback_destination,
                Some(dialog_id),
            )
            .await?;

        debug!(dialog=%dialog_id, transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), "Auth-retry INVITE sent via candidate failover path");
        Ok(transaction_id)
    }

    /// RFC 4028 §6 — resend an INVITE with a per-call `Session-Expires` /
    /// `Min-SE` override after the peer replied 422 Session Interval Too
    /// Small. The peer's `Min-SE` header dictates the required floor; callers
    /// pass it here together with the desired `Session-Expires` (typically
    /// set to `min_se` so the retry passes the first check).
    ///
    /// Mirrors `send_invite_with_auth` — reuses the original dialog's
    /// `Call-ID` + `From` tag, rebuilds as an initial INVITE (422 was a final
    /// response that did *not* establish a dialog), bumps CSeq via
    /// `Dialog::create_request_template`. The timer headers use the supplied
    /// overrides instead of the global `DialogManagerConfig` values.
    pub async fn send_invite_with_session_timer_override(
        &self,
        dialog_id: &DialogId,
        body: Option<bytes::Bytes>,
        session_secs: u32,
        min_se: u32,
    ) -> DialogResult<TransactionKey> {
        use crate::transaction::client::builders::InviteBuilder;

        debug!(
            "Resending INVITE with session-timer override (SE={}, Min-SE={}) for dialog {}",
            session_secs, min_se, dialog_id
        );

        let (fallback_destination, candidates, request) = {
            let mut dialog = self.get_dialog_mut(dialog_id)?;

            let fallback_destination =
                dialog.get_remote_target_address().await.ok_or_else(|| {
                    crate::errors::DialogError::routing_error(
                        "No remote target address available for 422 retry",
                    )
                })?;

            let body_string = body.map(|b| String::from_utf8_lossy(&b).to_string());

            let template = dialog.create_request_template(Method::Invite);
            dialog.invite_cseq = Some(template.cseq_number);

            let local_tag = match template.local_tag.clone() {
                Some(tag) if !tag.is_empty() => tag,
                _ => {
                    let new_tag = dialog.generate_local_tag();
                    dialog.local_tag = Some(new_tag.clone());
                    new_tag
                }
            };

            let local_address =
                self.local_address_for_target_and_routes(&template.target_uri, &template.route_set);
            let mut invite_builder = InviteBuilder::new()
                .from_detailed(
                    Some("User"),
                    template.local_uri.to_string(),
                    Some(&local_tag),
                )
                .to_detailed(Some("User"), template.remote_uri.to_string(), None)
                .call_id(&template.call_id)
                .cseq(template.cseq_number)
                .request_uri(template.target_uri.to_string())
                .local_address(local_address);

            for route in &template.route_set {
                invite_builder = invite_builder.add_route(route.clone());
            }

            if let Some(contact) = self.local_contact_uri() {
                invite_builder = invite_builder.contact(contact);
            }

            if let Some(sdp_content) = body_string {
                invite_builder = invite_builder.with_sdp(sdp_content);
            }

            let mut request =
                invite_builder
                    .build()
                    .map_err(|e| crate::errors::DialogError::InternalError {
                        message: format!("Failed to build 422-retry INVITE: {}", e),
                        context: None,
                    })?;

            // Re-inject policy headers. 100rel follows the global config (the
            // peer's 100rel preference didn't change); session-timer headers
            // use the per-call overrides so the retry carries the peer's
            // required Min-SE floor.
            inject_100rel_policy(&mut request, self.config_100rel_policy());
            inject_session_timer_headers(&mut request, session_secs, min_se);

            let candidates = self
                .resolve_uri_to_candidates(
                    &crate::transaction::transport::multiplexed::next_hop_uri_for_request(&request),
                )
                .await;

            (fallback_destination, candidates, request)
        };

        // RFC 3263 §4.3 multi-candidate failover. STIR/SHAKEN re-signs
        // per attempt inside the helper since the 422-retry carries a
        // new CSeq + adjusted Session-Expires (the original PASSporT
        // no longer covers the canonical form).
        let (transaction_id, _addr) = self
            .send_request_with_candidate_failover(
                request,
                candidates,
                fallback_destination,
                Some(dialog_id),
            )
            .await?;

        debug!(dialog=%dialog_id, transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), session_secs, min_se, "422-retry INVITE sent via candidate failover path");
        Ok(transaction_id)
    }

    /// Send an *initial* INVITE on a freshly-created outgoing dialog, with
    /// caller-supplied extra headers appended to the wire request.
    ///
    /// Mirrors `send_invite_with_auth` / `send_invite_with_session_timer_override`
    /// in construction shape (rebuild the INVITE via `InviteBuilder`, inject
    /// global policy headers, send via `create_invite_client_transaction`)
    /// but is intended for the *first* transmission rather than a retry.
    /// Callers go through `crate::manager::unified::UnifiedManager::make_call_with_extra_headers`
    /// rather than calling this directly; this method is the layer that
    /// actually puts the bytes on the wire.
    ///
    /// `extra_headers` is appended verbatim — typical contents:
    /// - `TypedHeader::PAssertedIdentity(...)` (RFC 3325) for trunk identity
    /// - `TypedHeader::PPreferredIdentity(...)` (RFC 3325) for asserted-identity preference
    /// - any other carrier-specific headers (`P-Charging-Vector`, etc.) the
    ///   application has already constructed.
    pub async fn send_initial_invite_with_extra_headers(
        &self,
        dialog_id: &DialogId,
        body: Option<bytes::Bytes>,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
        from_display: Option<String>,
        contact_override: Option<String>,
    ) -> DialogResult<TransactionKey> {
        use crate::transaction::client::builders::InviteBuilder;

        debug!(
            "Sending initial INVITE with {} extra header(s) for dialog {}",
            extra_headers.len(),
            dialog_id
        );

        let (fallback_destination, candidates, request) = {
            let mut dialog = self.get_dialog_mut(dialog_id)?;

            let fallback_destination =
                dialog.get_remote_target_address().await.ok_or_else(|| {
                    crate::errors::DialogError::routing_error(
                        "No remote target address available for initial INVITE",
                    )
                })?;

            let body_string = body.map(|b| String::from_utf8_lossy(&b).to_string());

            let template = dialog.create_request_template(Method::Invite);
            dialog.invite_cseq = Some(template.cseq_number);

            let local_tag = match template.local_tag.clone() {
                Some(tag) if !tag.is_empty() => tag,
                _ => {
                    let new_tag = dialog.generate_local_tag();
                    dialog.local_tag = Some(new_tag.clone());
                    new_tag
                }
            };

            let local_address =
                self.local_address_for_target_and_routes(&template.target_uri, &template.route_set);
            let mut invite_builder = InviteBuilder::new()
                .from_detailed(
                    from_display.as_deref().or(Some("User")),
                    template.local_uri.to_string(),
                    Some(&local_tag),
                )
                .to_detailed(Some("User"), template.remote_uri.to_string(), None)
                .call_id(&template.call_id)
                .cseq(template.cseq_number)
                .request_uri(template.target_uri.to_string())
                .local_address(local_address);

            for route in &template.route_set {
                invite_builder = invite_builder.add_route(route.clone());
            }

            // SIP_API_DESIGN_2 §7.2 — Contact override precedence: a structured
            // `contact_override` (Phase B `InviteRequestOptions.contact_uri`)
            // wins; otherwise the legacy partition of a Contact smuggled
            // through `extra_headers` (e.g. a B2BUA rewriting the upstream
            // Contact); otherwise the local UA binding. InviteBuilder always
            // emits exactly one Contact, so any caller-supplied Contact must be
            // fed through the typed `.contact(...)` setter, never appended.
            let (caller_contact, extra_headers): (
                Vec<rvoip_sip_core::types::TypedHeader>,
                Vec<rvoip_sip_core::types::TypedHeader>,
            ) = extra_headers
                .into_iter()
                .partition(|h| matches!(h, rvoip_sip_core::types::TypedHeader::Contact(_)));
            let override_contact_uri = contact_override.or_else(|| {
                caller_contact.into_iter().find_map(|h| match h {
                    rvoip_sip_core::types::TypedHeader::Contact(c) => {
                        c.address().map(|addr| addr.uri.to_string())
                    }
                    _ => None,
                })
            });
            if let Some(uri) = override_contact_uri {
                invite_builder = invite_builder.contact(uri);
            } else if let Some(contact) = self.local_contact_uri() {
                invite_builder = invite_builder.contact(contact);
            }

            if let Some(sdp_content) = body_string {
                invite_builder = invite_builder.with_sdp(sdp_content);
            }

            for hdr in extra_headers {
                invite_builder = invite_builder.header(hdr);
            }

            let mut request =
                invite_builder
                    .build()
                    .map_err(|e| crate::errors::DialogError::InternalError {
                        message: format!("Failed to build initial-INVITE-with-extras: {}", e),
                        context: None,
                    })?;

            // Re-inject the negotiated policy headers (100rel, session-timer),
            // mirroring `send_request_in_dialog`'s initial-INVITE arm.
            inject_100rel_policy(&mut request, self.config_100rel_policy());
            if let Some((secs, min_se)) = self.config_session_timer_settings() {
                inject_session_timer_headers(&mut request, secs, min_se);
            }

            let candidates = self
                .resolve_uri_to_candidates(
                    &crate::transaction::transport::multiplexed::next_hop_uri_for_request(&request),
                )
                .await;

            (fallback_destination, candidates, request)
        };

        // RFC 3263 §4.3 multi-candidate failover. Walks the resolved
        // candidates in order on transport-level failure. STIR/SHAKEN
        // signing (`pre_send_request`) fires once per attempt inside
        // the helper since Via/branch change between attempts. The
        // helper also registers tx→dialog BEFORE send so 401-driven
        // auth retry and other fast-response paths can locate the
        // dialog without racing.
        let (transaction_id, _addr) = self
            .send_request_with_candidate_failover(
                request,
                candidates,
                fallback_destination,
                Some(dialog_id),
            )
            .await?;

        debug!(dialog=%dialog_id, transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), "Initial INVITE-with-extras sent");
        Ok(transaction_id)
    }

    /// Send a freshly-built request via a new client transaction,
    /// retrying with the next `ResolvedTarget` on transport-level
    /// failure (RFC 3263 §4.3).
    ///
    /// On a recoverable transport error from `send_request` (the
    /// transaction terminated immediately, transport error event, or
    /// general transport failure), the helper destroys the failed
    /// transaction by leaving it for the normal cleanup path and
    /// creates a fresh client transaction targeted at the next
    /// candidate. Non-transport errors (parse failures, state-machine
    /// errors, signer errors) fail fast — retrying on a different
    /// candidate would not help.
    ///
    /// For INVITE specifically, fires
    /// `RequestLifecycle::pre_send_request` once per attempt so the
    /// installed signer sees the per-attempt request (Via / branch
    /// differ across attempts).
    ///
    /// Returns the transaction key of the first attempt that
    /// successfully reached `send_request`-Ok, along with the
    /// [`SocketAddr`] that succeeded. Caller is responsible for
    /// registering it into `transaction_to_dialog`.
    ///
    /// When `candidates` is empty, makes a single attempt against
    /// `fallback`. RFC 3261 §17.1.1.3 normal-termination after 2xx
    /// on a fast loopback is treated as success (matches the
    /// suppression already in [`Self::send_initial_invite_with_extra_headers`]).
    ///
    /// `tx_to_dialog`, when supplied, is the dialog id to register
    /// against the freshly-created transaction *before* `send_request`
    /// fires. Critical for paths whose response handling (e.g.,
    /// 401-driven auth retry, dialog state transitions) looks the
    /// dialog up via `transaction_to_dialog`: registering AFTER send
    /// would race with a fast response and the dialog would be
    /// unreachable. Pass `None` for stateless sends (e.g. the
    /// proxy's per-leg failover).
    pub async fn send_request_with_candidate_failover(
        &self,
        request: rvoip_sip_core::Request,
        candidates: Vec<rvoip_sip_transport::resolver::ResolvedTarget>,
        fallback: std::net::SocketAddr,
        tx_to_dialog: Option<&DialogId>,
    ) -> DialogResult<(TransactionKey, std::net::SocketAddr)> {
        use crate::manager::RequestLifecycle;
        use rvoip_sip_transport::resolver::ResolvedTarget;

        let method = request.method();
        let candidates: Vec<ResolvedTarget> = if candidates.is_empty() {
            let next_hop =
                crate::transaction::transport::multiplexed::next_hop_uri_for_request(&request);
            let transport = rvoip_sip_transport::resolver::select_transport_for_uri(&next_hop);
            vec![ResolvedTarget::immediate(fallback, transport)]
        } else {
            candidates
        };

        let total = candidates.len();
        let mut last_err: Option<crate::errors::DialogError> = None;

        for (idx, target) in candidates.iter().enumerate() {
            let attempt = idx + 1;
            let mut req = request.clone();

            if method == Method::Invite {
                if let Err(e) = self.pre_send_request(&mut req, target.addr).await {
                    return Err(e);
                }
            }

            let sent_request = req.clone();
            let request_key = crate::manager::core::outbound_request_key(&sent_request);

            let tx_result = if method == Method::Invite {
                self.transaction_manager
                    .create_invite_client_transaction(req, target.addr)
                    .await
            } else {
                self.transaction_manager
                    .create_non_invite_client_transaction(req, target.addr)
                    .await
            };
            let tx_id = match tx_result {
                Ok(id) => id,
                Err(e) => {
                    last_err = Some(crate::errors::DialogError::TransactionError {
                        message: format!(
                            "RFC 3263 §4.3 candidate {}/{} ({}): create_*_client_transaction failed: {}",
                            attempt, total, target.addr, e
                        ),
                    });
                    continue;
                }
            };

            // Register tx→dialog mapping BEFORE send so a fast
            // response (e.g. 401 hitting loopback before send_request
            // returns) can locate the dialog. Removed on failed
            // attempts so the next candidate's tx replaces it.
            if let Some(dialog_id) = tx_to_dialog {
                self.link_transaction_to_dialog_indexed(&tx_id, dialog_id);
            }

            match self.transaction_manager.send_request(&tx_id).await {
                Ok(()) => {
                    self.record_outbound_transport_context(
                        &tx_id,
                        request_key,
                        target.transport,
                        target.addr,
                    );
                    self.post_send_request(&sent_request, target.addr).await?;
                    if attempt > 1 {
                        debug!(
                            "RFC 3263 §4.3: candidate {}/{} ({}) succeeded after {} prior failure(s)",
                            attempt,
                            total,
                            target.addr,
                            attempt - 1
                        );
                    }
                    return Ok((tx_id, target.addr));
                }
                Err(e) => {
                    let msg = e.to_string();
                    let benign_terminate = method == Method::Invite
                        && (msg.contains("Transaction terminated after timeout")
                            || msg.contains("Transaction terminated"));
                    if benign_terminate {
                        // RFC 3261 §17.1.1.3 — INVITE client transitions
                        // Calling → Terminated on automatic ACK for 2xx;
                        // treat as success and don't fail over.
                        self.record_outbound_transport_context(
                            &tx_id,
                            request_key,
                            target.transport,
                            target.addr,
                        );
                        self.post_send_request(&sent_request, target.addr).await?;
                        if attempt > 1 {
                            debug!(
                                "RFC 3263 §4.3: candidate {}/{} ({}) succeeded after {} prior failure(s) (benign terminate)",
                                attempt,
                                total,
                                target.addr,
                                attempt - 1
                            );
                        }
                        return Ok((tx_id, target.addr));
                    }

                    let is_transport_failure =
                        matches!(&e, crate::transaction::error::Error::TransportError { .. });
                    if is_transport_failure && idx + 1 < total {
                        debug!(
                            attempt,
                            total,
                            destination=%target.addr,
                            error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e),
                            "Candidate failed with transport error; trying next"
                        );
                        // Drop the failed-leg mapping so the next
                        // attempt's tx is the canonical one for this
                        // dialog.
                        if tx_to_dialog.is_some() {
                            self.unlink_transaction_from_dialog_indexed(&tx_id);
                        }
                        last_err = Some(crate::errors::DialogError::TransactionError {
                            message: format!(
                                "RFC 3263 §4.3 candidate {} ({}) transport failure: {}",
                                attempt, target.addr, e
                            ),
                        });
                        continue;
                    }

                    return Err(crate::errors::DialogError::TransactionError {
                        message: format!("Failed to send {}: {}", method, e),
                    });
                }
            }
        }

        Err(
            last_err.unwrap_or_else(|| crate::errors::DialogError::TransactionError {
                message: format!(
                    "RFC 3263 §4.3 failover exhausted: all {} candidate(s) failed",
                    total
                ),
            }),
        )
    }
}

/// A response qualifies for RFC 3262 reliable-provisional wrapping when it
/// is a non-100 provisional (101–199) and carries a body (typically SDP
/// early media). 100 Trying is hop-by-hop and never reliable; bodiless
/// 180/183 are still sent unreliably since there's nothing to protect.
pub fn should_send_reliably(response: &Response) -> bool {
    let code = response.status_code();
    (101..200).contains(&code) && !response.body().is_empty()
}

/// Append RFC 4028 session-timer headers to an outgoing INVITE: a
/// `Session-Expires: <secs>;refresher=uac` (caller-side refresh by default —
/// keeps NAT pinholes alive on the UAC), a `Min-SE: <min_se>`, and the
/// `timer` option tag in `Supported`. No-op if `secs` is 0.
pub fn inject_session_timer_headers(request: &mut Request, secs: u32, min_se: u32) {
    use rvoip_sip_core::types::min_se::MinSE;
    use rvoip_sip_core::types::session_expires::{Refresher, SessionExpires};
    use rvoip_sip_core::types::{Supported, TypedHeader};

    if secs == 0 {
        return;
    }

    request
        .headers
        .push(TypedHeader::SessionExpires(SessionExpires::new(
            secs,
            Some(Refresher::Uac),
        )));
    request.headers.push(TypedHeader::MinSE(MinSE::new(min_se)));

    let mut found = false;
    for header in request.headers.iter_mut() {
        if let TypedHeader::Supported(ref mut sup) = header {
            if !sup.option_tags.iter().any(|t| t == "timer") {
                sup.option_tags.push("timer".to_string());
            }
            found = true;
            break;
        }
    }
    if !found {
        request
            .headers
            .push(TypedHeader::Supported(Supported::new(vec![
                "timer".to_string()
            ])));
    }
}

/// Append `Require: 100rel` and `RSeq: <rseq>` to an outgoing 18x. Creates
/// the `Require` header if absent, appends the tag otherwise.
pub fn inject_reliable_provisional_headers(response: &mut Response, rseq: u32) {
    use rvoip_sip_core::types::rseq::RSeq;
    use rvoip_sip_core::types::{Require, TypedHeader};

    let mut updated = false;
    for header in response.headers.iter_mut() {
        if let TypedHeader::Require(ref mut req) = header {
            if !req.requires("100rel") {
                req.add_tag("100rel");
            }
            updated = true;
            break;
        }
    }
    if !updated {
        response
            .headers
            .push(TypedHeader::Require(Require::with_tag("100rel")));
    }
    response.headers.push(TypedHeader::RSeq(RSeq::new(rseq)));
}

impl TransactionHelpers for DialogManager {
    /// Associate a transaction with a dialog
    ///
    /// Creates the mapping between transactions and dialogs for proper message routing.
    fn link_transaction_to_dialog(&self, transaction_id: &TransactionKey, dialog_id: &DialogId) {
        self.link_transaction_to_dialog_indexed(transaction_id, dialog_id);
        debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id), dialog=%dialog_id, "Linked transaction to dialog");
    }

    /// Create ACK for 2xx response using transaction-core helpers
    ///
    /// Uses transaction-core's ACK creation helpers while maintaining dialog-core concerns.
    async fn create_ack_for_success_response(
        &self,
        original_invite_tx_id: &TransactionKey,
        response: &Response,
    ) -> DialogResult<Request> {
        debug!("Creating ACK for 2xx response using transaction-core helpers");

        // Use transaction-core's helper method to create ACK for 2xx response
        // This ensures proper ACK construction according to RFC 3261
        let ack_request = self
            .transaction_manager
            .create_ack_for_2xx(original_invite_tx_id, response)
            .await
            .map_err(|e| crate::errors::DialogError::TransactionError {
                message: format!("Failed to create ACK for 2xx using transaction-core: {}", e),
            })?;

        debug!("Successfully created ACK for 2xx response");
        Ok(ack_request)
    }
}

// Transaction Event Processing Implementation
impl DialogManager {
    /// Process a transaction event and update dialog state accordingly
    ///
    /// This is the core event-driven state management for dialogs based on
    /// transaction layer events. It implements proper RFC 3261 dialog state transitions.
    pub async fn process_transaction_event(
        &self,
        transaction_id: &TransactionKey,
        dialog_id: &DialogId,
        event: TransactionEvent,
    ) -> DialogResult<()> {
        debug!(
            transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id),
            %dialog_id,
            event=?crate::transaction::safe_diagnostics::SafeTransactionEvent::new(&event),
            "Processing transaction event"
        );

        match event {
            TransactionEvent::StateChanged {
                previous_state,
                new_state,
                ..
            } => {
                self.handle_transaction_state_change(
                    dialog_id,
                    transaction_id,
                    previous_state,
                    new_state,
                )
                .await
            }

            TransactionEvent::SuccessResponse { response, .. } => {
                self.handle_transaction_success_response(dialog_id, transaction_id, response)
                    .await
            }

            TransactionEvent::FailureResponse { response, .. } => {
                self.handle_transaction_failure_response(dialog_id, transaction_id, response)
                    .await
            }

            TransactionEvent::ProvisionalResponse { response, .. } => {
                self.handle_transaction_provisional_response(dialog_id, transaction_id, response)
                    .await
            }

            TransactionEvent::TransactionTerminated { .. } => {
                self.handle_transaction_terminated(dialog_id, transaction_id)
                    .await
            }

            TransactionEvent::TimerTriggered { timer, .. } => {
                debug!(
                    timer_len=timer.len(),
                    transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id),
                    %dialog_id,
                    "Transaction timer triggered"
                );
                Ok(()) // Most timer events don't require dialog-level action
            }

            TransactionEvent::AckReceived { request, .. } => {
                self.handle_ack_received_event(dialog_id, transaction_id, request)
                    .await
            }

            TransactionEvent::CancelReceived { .. } => {
                // RFC 3261 §9.2. The transaction layer has already handled
                // the wire responses for this matched UAS-side CANCEL
                // (200 to CANCEL, 487 to INVITE). Dialog-core still owns the
                // dialog/session lifecycle notification.
                self.terminate_dialog_for_tx_and_emit_cancelled(transaction_id, "CANCEL received")
                    .await;
                Ok(())
            }

            _ => {
                debug!(
                    %dialog_id,
                    event=?crate::transaction::safe_diagnostics::SafeTransactionEvent::new(&event),
                    "Unhandled transaction event type for dialog"
                );
                Ok(())
            }
        }
    }

    /// Handle transaction state changes and update dialog state accordingly
    async fn handle_transaction_state_change(
        &self,
        dialog_id: &DialogId,
        transaction_id: &TransactionKey,
        previous_state: TransactionState,
        new_state: TransactionState,
    ) -> DialogResult<()> {
        debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id), ?previous_state, ?new_state, dialog=%dialog_id, "Transaction state changed");

        // Update dialog state based on transaction state changes
        match new_state {
            TransactionState::Completed => {
                debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id), dialog=%dialog_id, "Transaction completed");
                // Transaction completed successfully - dialog remains active
            }

            TransactionState::Terminated => {
                debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id), dialog=%dialog_id, "Transaction terminated");
                // Client INVITE transactions can emit StateChanged(Terminated)
                // before the final failure response has been routed up to the
                // dialog manager. Keep that mapping until the explicit
                // TransactionTerminated cleanup so 487-after-CANCEL reaches
                // session-core as CallCancelled instead of falling through to
                // the setup/teardown watchdog.
                if transaction_id.method() != &rvoip_sip_core::Method::Invite
                    || transaction_id.is_server()
                {
                    self.unlink_transaction_from_dialog_indexed(transaction_id);
                }
            }

            _ => {
                // Other state changes are informational
                debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id), ?new_state, dialog=%dialog_id, "Transaction state observed");
            }
        }

        Ok(())
    }

    /// Handle successful responses from transactions
    async fn handle_transaction_success_response(
        &self,
        dialog_id: &DialogId,
        transaction_id: &TransactionKey,
        response: Response,
    ) -> DialogResult<()> {
        info!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id), status=response.status_code(), reason_len=response.reason_phrase().len(), dialog=%dialog_id, "Transaction received success response");

        // Update dialog state based on successful response
        let dialog_state_changed = {
            let mut dialog = self.get_dialog_mut(dialog_id)?;
            let old_state = dialog.state.clone();

            // Update dialog with response information (remote tag, etc.)
            if let Some(to_header) = response.to() {
                if let Some(to_tag) = to_header.tag() {
                    info!(dialog=%dialog_id, remote_tag_len=to_tag.len(), "Updating remote tag for dialog");
                    dialog.set_remote_tag(to_tag.to_string());
                } else {
                    warn!("200 OK response has no To tag for dialog {}", dialog_id);
                }
            } else {
                warn!("200 OK response has no To header for dialog {}", dialog_id);
            }

            // Update dialog state based on response status and current state
            let state_changed = match response.status_code() {
                200 => {
                    if dialog.state == crate::dialog::DialogState::Early {
                        dialog.state = crate::dialog::DialogState::Confirmed;

                        // CRITICAL FIX: Update dialog lookup now that we have both tags
                        if let Some(tuple) = dialog.dialog_id_tuple() {
                            let key = crate::manager::utils::DialogUtils::create_lookup_key(
                                &tuple.0, &tuple.1, &tuple.2,
                            );
                            self.dialog_lookup.insert(key, dialog_id.clone());
                            info!("Updated dialog lookup for confirmed dialog {}", dialog_id);
                        }

                        // RFC 4028 UAC: capture negotiated Session-Expires
                        // from the 2xx. The refresher is whoever the peer
                        // named; if the peer omitted `refresher=`, RFC 4028
                        // §7.1 default for a UAC that originally requested
                        // `refresher=uac` is that the UAC refreshes.
                        if transaction_id.method() == &rvoip_sip_core::Method::Invite {
                            use rvoip_sip_core::types::session_expires::Refresher;
                            use rvoip_sip_core::types::TypedHeader;
                            if let Some(se) = response.headers.iter().find_map(|h| {
                                if let TypedHeader::SessionExpires(se) = h {
                                    Some(se)
                                } else {
                                    None
                                }
                            }) {
                                dialog.session_expires_secs = Some(se.delta_seconds);
                                dialog.is_session_refresher =
                                    matches!(se.refresher, None | Some(Refresher::Uac),);
                                info!(
                                    "UAC session timer negotiated: expires={}s, we_refresh={}",
                                    se.delta_seconds, dialog.is_session_refresher
                                );
                            }
                        }

                        true
                    } else {
                        false
                    }
                }
                _ => false,
            };

            if state_changed {
                Some((old_state, dialog.state.clone()))
            } else {
                None
            }
        };

        // Emit dialog events for session-core
        if let Some((old_state, new_state)) = dialog_state_changed {
            self.emit_dialog_event(DialogEvent::StateChanged {
                dialog_id: dialog_id.clone(),
                old_state,
                new_state,
            })
            .await;
        }

        // Emit session coordination events for session-core
        self.emit_session_coordination_event(SessionCoordinationEvent::ResponseReceived {
            dialog_id: dialog_id.clone(),
            response: response.clone(),
            transaction_id: transaction_id.clone(),
        })
        .await;

        // Handle specific successful response types
        let session_id_for_diag = self.get_session_id(dialog_id);
        match response.status_code() {
            200 => {
                // Check if this is a 200 OK to INVITE - need to send ACK
                if transaction_id.method() == &rvoip_sip_core::Method::Invite {
                    crate::diagnostics::record_uac_invite_2xx_response();
                    if let Some(session_id) = session_id_for_diag.as_deref() {
                        crate::diagnostics::record_call_timing_uac_invite_2xx_response(session_id);
                    }
                    info!(
                        "✅ Received 200 OK to INVITE, sending automatic ACK for dialog {}",
                        dialog_id
                    );

                    // Send ACK using transaction-core's send_ack_for_2xx method
                    crate::diagnostics::record_uac_invite_2xx_ack_attempt();
                    if let Some(session_id) = session_id_for_diag.as_deref() {
                        crate::diagnostics::record_call_timing_uac_ack_attempt(session_id);
                    }
                    match self
                        .transaction_manager
                        .send_ack_for_2xx(transaction_id, &response)
                        .await
                    {
                        Ok(_) => {
                            crate::diagnostics::record_uac_invite_2xx_ack_success();
                            if let Some(session_id) = session_id_for_diag.as_deref() {
                                crate::diagnostics::record_call_timing_uac_ack_success(session_id);
                            }
                            info!("Successfully sent automatic ACK for 200 OK to INVITE");

                            // Notify session-core that ACK was sent (for state machine transition)
                            let negotiated_sdp = if !response.body().is_empty() {
                                Some(String::from_utf8_lossy(response.body()).to_string())
                            } else {
                                None
                            };

                            self.emit_session_coordination_event(
                                SessionCoordinationEvent::AckSent {
                                    dialog_id: dialog_id.clone(),
                                    transaction_id: transaction_id.clone(),
                                    negotiated_sdp,
                                },
                            )
                            .await;
                        }
                        Err(e) => {
                            crate::diagnostics::record_uac_invite_2xx_ack_failure();
                            if let Some(session_id) = session_id_for_diag.as_deref() {
                                crate::diagnostics::record_call_timing_uac_ack_failure(session_id);
                            }
                            warn!(error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "Failed to send automatic ACK for 200 OK to INVITE");
                        }
                    }
                }

                // Check if this is a 200 OK to BYE - dialog is terminating
                if transaction_id.method() == &rvoip_sip_core::Method::Bye {
                    info!(
                        "✅ Received 200 OK to BYE, dialog {} is terminating",
                        dialog_id
                    );

                    // Emit CallTerminating event to notify session-core
                    self.emit_session_coordination_event(
                        SessionCoordinationEvent::CallTerminating {
                            dialog_id: dialog_id.clone(),
                            reason: "BYE completed successfully".to_string(),
                        },
                    )
                    .await;
                }

                // Successful completion - could be call answered, request completed, etc.
                if !response.body().is_empty() {
                    if transaction_id.method() == &rvoip_sip_core::Method::Invite {
                        crate::diagnostics::record_uac_invite_2xx_call_answered_emit();
                        if let Some(session_id) = session_id_for_diag.as_deref() {
                            crate::diagnostics::record_call_timing_uac_call_answered_emit(
                                session_id,
                            );
                        }
                    }
                    let sdp = String::from_utf8_lossy(response.body()).to_string();
                    self.emit_session_coordination_event(SessionCoordinationEvent::CallAnswered {
                        dialog_id: dialog_id.clone(),
                        session_answer: sdp,
                    })
                    .await;
                }

                // RFC 4028 UAC: spawn the refresh task now that the dialog
                // is confirmed and negotiated interval is on the dialog.
                if transaction_id.method() == &rvoip_sip_core::Method::Invite {
                    if let Ok(dlg) = self.get_dialog(dialog_id) {
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
            }
            _ => {
                debug!(
                    "Other successful response {} for dialog {}",
                    response.status_code(),
                    dialog_id
                );
            }
        }

        Ok(())
    }

    /// Handle failure responses from transactions
    async fn handle_transaction_failure_response(
        &self,
        dialog_id: &DialogId,
        transaction_id: &TransactionKey,
        response: Response,
    ) -> DialogResult<()> {
        if response.status_code() == 487 {
            info!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id), status=response.status_code(), reason_len=response.reason_phrase().len(), dialog=%dialog_id, "Transaction received CANCEL terminal response");
        } else {
            warn!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id), status=response.status_code(), reason_len=response.reason_phrase().len(), dialog=%dialog_id, "Transaction received failure response");
        }

        // Handle specific failure cases and emit appropriate events
        match response.status_code() {
            487 => {
                // RFC 3261 §15.1.2 — 487 Request Terminated is a
                // CANCEL-specific termination, distinct from a generic
                // dialog teardown. Emit only `CallCancelled`; emitting
                // `DialogEvent::Terminated` here too causes the event
                // hub to publish both `DialogToSessionEvent::CallTerminated`
                // and `DialogToSessionEvent::CallCancelled` for the same
                // 487, which races in the session-core dispatcher and
                // intermittently surfaces `Event::CallEnded` to the app
                // instead of `Event::CallCancelled`.
                info!("Call cancelled for dialog {}", dialog_id);

                self.emit_session_coordination_event(SessionCoordinationEvent::CallCancelled {
                    dialog_id: dialog_id.clone(),
                    reason: "Request terminated".to_string(),
                })
                .await;
            }

            status
                if transaction_id.method() == &rvoip_sip_core::Method::Bye
                    && matches!(status, 408 | 481) =>
            {
                // RFC 3261 BYE terminates this endpoint's participation in
                // the dialog. A 481 means the peer no longer has the dialog;
                // 408 means the request timed out. Both are terminal for our
                // local session state.
                self.emit_session_coordination_event(SessionCoordinationEvent::CallTerminating {
                    dialog_id: dialog_id.clone(),
                    reason: format!(
                        "BYE completed locally after {} {}",
                        status,
                        response.reason_phrase()
                    ),
                })
                .await;
            }

            status if status >= 400 && status < 500 && !response_has_auth_challenge(&response) => {
                // Client error - may require dialog termination
                warn!(
                    "Client error {} for dialog {} - considering termination",
                    status, dialog_id
                );

                // Emit session coordination event for failed requests
                self.emit_session_coordination_event(SessionCoordinationEvent::RequestFailed {
                    dialog_id: Some(dialog_id.clone()),
                    transaction_id: transaction_id.clone(),
                    status_code: status,
                    reason_phrase: response.reason_phrase().to_string(),
                    method: "Unknown".to_string(), // TODO: Extract from transaction context
                })
                .await;
            }

            status if matches!(status, 401 | 407) => {
                debug!(
                    "Auth challenge {} for dialog {} - deferring terminal failure handling",
                    status, dialog_id
                );
            }

            status if status >= 500 => {
                // Server error - may require retry or termination
                warn!(
                    "Server error {} for dialog {} - considering retry",
                    status, dialog_id
                );

                // Emit session coordination event for server errors
                self.emit_session_coordination_event(SessionCoordinationEvent::RequestFailed {
                    dialog_id: Some(dialog_id.clone()),
                    transaction_id: transaction_id.clone(),
                    status_code: status,
                    reason_phrase: response.reason_phrase().to_string(),
                    method: "Unknown".to_string(), // TODO: Extract from transaction context
                })
                .await;
            }

            _ => {
                debug!(
                    "Other failure response {} for dialog {}",
                    response.status_code(),
                    dialog_id
                );
            }
        }

        // Always emit the response received event for session-core to handle
        self.emit_session_coordination_event(SessionCoordinationEvent::ResponseReceived {
            dialog_id: dialog_id.clone(),
            response: response.clone(),
            transaction_id: transaction_id.clone(),
        })
        .await;

        Ok(())
    }

    /// Handle provisional responses from transactions
    async fn handle_transaction_provisional_response(
        &self,
        dialog_id: &DialogId,
        transaction_id: &TransactionKey,
        response: Response,
    ) -> DialogResult<()> {
        debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id), status=response.status_code(), reason_len=response.reason_phrase().len(), dialog=%dialog_id, "Transaction received provisional response");

        // Update dialog state for early dialogs
        let dialog_created = {
            let mut dialog = self.get_dialog_mut(dialog_id)?;
            let old_state = dialog.state.clone();

            // For provisional responses with to-tag, create early dialog
            if let Some(to_header) = response.to() {
                if let Some(to_tag) = to_header.tag() {
                    if dialog.remote_tag.is_none() {
                        dialog.set_remote_tag(to_tag.to_string());
                        if dialog.state == crate::dialog::DialogState::Initial {
                            dialog.state = crate::dialog::DialogState::Early;
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
            self.emit_dialog_event(DialogEvent::StateChanged {
                dialog_id: dialog_id.clone(),
                old_state,
                new_state,
            })
            .await;
        }

        // RFC 3262: auto-PRACK reliable provisionals.
        // Only applies to 18x (101..200), and only when the response carries
        // both Require: 100rel and an RSeq header.
        let status = response.status_code();
        if (101..200).contains(&status) {
            if let Some(rseq_value) = detect_reliable_provisional(&response) {
                let should_send = {
                    let mut dialog = self.get_dialog_mut(dialog_id)?;
                    match dialog.last_rseq_acked {
                        Some(prev) if rseq_value <= prev => {
                            debug!(
                                "Ignoring duplicate/out-of-order reliable {}: dialog {} already acked RSeq {} (got {})",
                                status, dialog_id, prev, rseq_value
                            );
                            false
                        }
                        _ => {
                            dialog.last_rseq_acked = Some(rseq_value);
                            true
                        }
                    }
                };

                if should_send {
                    if let Err(e) = self.send_prack(dialog_id, rseq_value).await {
                        warn!(dialog=%dialog_id, rseq=rseq_value, error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "Auto-PRACK failed");
                        // Roll back the ack record so a retransmit can re-trigger.
                        if let Ok(mut dialog) = self.get_dialog_mut(dialog_id) {
                            // Only roll back if we're still the most recent acker.
                            if dialog.last_rseq_acked == Some(rseq_value) {
                                dialog.last_rseq_acked = None;
                            }
                        }
                    }
                }
            }
        }

        // Handle specific provisional responses and emit session coordination events
        match response.status_code() {
            180 => {
                info!("Call ringing for dialog {}", dialog_id);

                self.emit_session_coordination_event(SessionCoordinationEvent::CallRinging {
                    dialog_id: dialog_id.clone(),
                })
                .await;
            }

            183 => {
                info!("Session progress for dialog {}", dialog_id);

                // Check for early media (SDP in 183)
                if !response.body().is_empty() {
                    let sdp = String::from_utf8_lossy(response.body()).to_string();
                    self.emit_session_coordination_event(SessionCoordinationEvent::EarlyMedia {
                        dialog_id: dialog_id.clone(),
                        sdp,
                    })
                    .await;
                } else {
                    self.emit_session_coordination_event(SessionCoordinationEvent::CallProgress {
                        dialog_id: dialog_id.clone(),
                        status_code: response.status_code(),
                        reason_phrase: response.reason_phrase().to_string(),
                    })
                    .await;
                }
            }

            _ => {
                debug!(
                    "Other provisional response {} for dialog {}",
                    response.status_code(),
                    dialog_id
                );

                // Emit general call progress event
                self.emit_session_coordination_event(SessionCoordinationEvent::CallProgress {
                    dialog_id: dialog_id.clone(),
                    status_code: response.status_code(),
                    reason_phrase: response.reason_phrase().to_string(),
                })
                .await;
            }
        }

        Ok(())
    }

    /// Handle transaction termination
    async fn handle_transaction_terminated(
        &self,
        dialog_id: &DialogId,
        transaction_id: &TransactionKey,
    ) -> DialogResult<()> {
        debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id), dialog=%dialog_id, "Transaction terminated for dialog");

        // Clean up transaction-dialog association
        self.unlink_transaction_from_dialog_indexed(transaction_id);

        if transaction_id.method() == &rvoip_sip_core::Method::Bye {
            self.emit_session_coordination_event(SessionCoordinationEvent::CallTerminating {
                dialog_id: dialog_id.clone(),
                reason: "BYE transaction terminated".to_string(),
            })
            .await;
        }

        // Note: Other methods do not automatically terminate dialogs when
        // transactions terminate because dialogs can have multiple
        // transactions. Dialog termination is handled by higher-level logic
        // (session-core) or explicit BYE requests.

        Ok(())
    }

    /// Handle ACK received event (RFC 3261 compliant media start point for UAS)
    async fn handle_ack_received_event(
        &self,
        dialog_id: &DialogId,
        transaction_id: &TransactionKey,
        request: rvoip_sip_core::Request,
    ) -> DialogResult<()> {
        if self
            .get_dialog_state(dialog_id)
            .map(|state| state.is_terminated())
            .unwrap_or(false)
        {
            debug!(dialog=%dialog_id, transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id), "Ignoring ACK for terminated dialog");
            return Ok(());
        }

        info!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id), dialog=%dialog_id, "ACK received; media may start on UAS side");

        // Extract any SDP from the ACK (though typically ACK doesn't have SDP for 2xx responses)
        let negotiated_sdp = if !request.body().is_empty() {
            let sdp = String::from_utf8_lossy(request.body()).to_string();
            info!(body_len = request.body().len(), "ACK contains SDP body");
            Some(sdp)
        } else {
            info!("ACK has no SDP body (normal for 2xx ACK)");
            None
        };

        info!(
            "🔔 About to emit AckReceived event for dialog {}",
            dialog_id
        );

        // RFC 3261 COMPLIANT: Emit ACK received event for UAS side media creation
        self.emit_session_coordination_event(SessionCoordinationEvent::AckReceived {
            dialog_id: dialog_id.clone(),
            transaction_id: transaction_id.clone(),
            negotiated_sdp,
        })
        .await;

        info!("🚀 RFC 3261: Emitted AckReceived event for UAS side media creation");
        Ok(())
    }
}

// Additional transaction integration methods for DialogManager
impl DialogManager {
    /// Create server transaction for incoming request
    ///
    /// Helper to create server transactions with proper error handling.
    pub async fn create_server_transaction_for_request(
        &self,
        request: Request,
        source: SocketAddr,
    ) -> DialogResult<TransactionKey> {
        debug!(
            method=%crate::transaction::safe_diagnostics::SafeMethod::new(&request.method()),
            %source,
            "Creating server transaction for request"
        );

        let server_transaction = self
            .transaction_manager
            .create_server_transaction(request, source)
            .await
            .map_err(|e| crate::errors::DialogError::TransactionError {
                message: format!("Failed to create server transaction: {}", e),
            })?;

        let transaction_id = server_transaction.id().clone();

        debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), "Created server transaction for request");
        Ok(transaction_id)
    }

    /// Create client transaction for outgoing request
    ///
    /// Helper to create client transactions with method-specific handling.
    pub async fn create_client_transaction_for_request(
        &self,
        request: Request,
        destination: SocketAddr,
        method: &Method,
    ) -> DialogResult<TransactionKey> {
        debug!(method=%crate::transaction::safe_diagnostics::SafeMethod::new(method), %destination, "Creating client transaction for request");

        // STIR/SHAKEN (RFC 8224) — fire the request lifecycle for
        // INVITE so the installed PASSporTSigner attaches an
        // `Identity:` header. Generic helper paths land here when
        // dialog-core's bespoke per-method send paths can't be used
        // (e.g. raw out-of-dialog INVITE injection from upper layers).
        let mut request = request;
        if *method == Method::Invite {
            use crate::manager::RequestLifecycle;
            self.pre_send_request(&mut request, destination).await?;
        }

        let transaction_id = if *method == Method::Invite {
            self.transaction_manager
                .create_invite_client_transaction(request, destination)
                .await
        } else {
            self.transaction_manager
                .create_non_invite_client_transaction(request, destination)
                .await
        }
        .map_err(|e| crate::errors::DialogError::TransactionError {
            message: format!("Failed to create {} client transaction: {}", method, e),
        })?;

        debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), method=%crate::transaction::safe_diagnostics::SafeMethod::new(method), "Created client transaction for request");
        Ok(transaction_id)
    }

    /// Terminate the dialog associated with an INVITE transaction and
    /// optionally emit a `CallCancelled` session-coordination event.
    ///
    /// UAC and UAS CANCEL differ:
    /// - UAC-side user cancel sends CANCEL and waits for the INVITE's final
    ///   outcome before session-core publishes `CallCancelled`.
    /// - UAS-side inbound CANCEL is already terminal for the pending INVITE
    ///   once 200(CANCEL)/487(INVITE) has been sent, so dialog-core must
    ///   publish `CallCancelled` to session-core.
    async fn dialog_id_for_invite_tx(&self, invite_tx_id: &TransactionKey) -> Option<DialogId> {
        if let Some(dialog_id) = self
            .transaction_to_dialog
            .get(invite_tx_id)
            .map(|d| d.clone())
        {
            return Some(dialog_id);
        } else {
            match self
                .transaction_manager
                .get_server_transaction_request(invite_tx_id)
                .await
            {
                Ok(request) => match self.find_dialog_for_request(&request).await {
                    Some(dialog_id) => return Some(dialog_id),
                    None => {
                        warn!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(invite_tx_id), "Cannot emit CallCancelled: no dialog mapping or request match");
                        return None;
                    }
                },
                Err(e) => {
                    warn!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(invite_tx_id), error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "Cannot emit CallCancelled: failed to fetch INVITE request");
                    return None;
                }
            }
        }
    }

    pub async fn terminate_dialog_for_tx(&self, invite_tx_id: &TransactionKey, _reason: &str) {
        let Some(dialog_id) = self.dialog_id_for_invite_tx(invite_tx_id).await else {
            return;
        };

        if let Ok(mut dialog) = self.get_dialog_mut(&dialog_id) {
            dialog.terminate();
            debug!("Terminated dialog {} due to INVITE cancellation", dialog_id);
        }
    }

    pub async fn terminate_dialog_for_tx_and_emit_cancelled(
        &self,
        invite_tx_id: &TransactionKey,
        reason: &str,
    ) {
        let Some(dialog_id) = self.dialog_id_for_invite_tx(invite_tx_id).await else {
            return;
        };

        if let Ok(mut dialog) = self.get_dialog_mut(&dialog_id) {
            dialog.terminate();
            debug!("Terminated dialog {} due to INVITE cancellation", dialog_id);
        }

        self.emit_session_coordination_event(SessionCoordinationEvent::CallCancelled {
            dialog_id,
            reason: reason.to_string(),
        })
        .await;
    }

    /// Cancel an INVITE transaction using transaction-core
    ///
    /// Properly cancels INVITE transactions while updating associated dialogs.
    pub async fn cancel_invite_transaction_with_dialog(
        &self,
        invite_tx_id: &TransactionKey,
    ) -> DialogResult<TransactionKey> {
        self.cancel_invite_transaction_with_dialog_and_extras(invite_tx_id, Vec::new())
            .await
    }

    /// CANCEL with application extras. The transaction-manager helper
    /// builds the wire CANCEL from the targeted INVITE (RFC 3261 §9.1
    /// — same Call-ID/From/To/CSeq-num/Via-branch/Route). When extras
    /// are supplied, they are appended to that wire form after the
    /// stack-managed slice; the resulting CANCEL is sent on its own
    /// new non-INVITE client transaction.
    pub async fn cancel_invite_transaction_with_dialog_and_extras(
        &self,
        invite_tx_id: &TransactionKey,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> DialogResult<TransactionKey> {
        debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(invite_tx_id), extra_header_count=extra_headers.len(), "Cancelling INVITE transaction with dialog cleanup");
        let pre_cancel_dialog_id = self
            .transaction_to_dialog
            .get(invite_tx_id)
            .map(|entry| entry.value().clone());

        // Cancel the transaction using transaction-core
        let cancel_tx_id = self
            .transaction_manager
            .cancel_invite_transaction_with_extras(invite_tx_id, extra_headers)
            .await
            .map_err(|e| crate::errors::DialogError::TransactionError {
                message: format!("Failed to cancel INVITE transaction: {}", e),
            })?;

        if let Some(dialog_id) = pre_cancel_dialog_id {
            if let Ok(mut dialog) = self.get_dialog_mut(&dialog_id) {
                dialog.terminate();
                debug!("Terminated dialog {} after sending CANCEL", dialog_id);
            }
        } else {
            self.terminate_dialog_for_tx(invite_tx_id, "INVITE transaction cancelled")
                .await;
        }

        debug!(invite_transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(invite_tx_id), cancel_transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&cancel_tx_id), "Successfully cancelled INVITE transaction and created CANCEL transaction");
        Ok(cancel_tx_id)
    }

    /// Get transaction statistics
    ///
    /// Provides insight into transaction-dialog associations.
    pub fn get_transaction_statistics(&self) -> (usize, usize) {
        let dialog_count = self.dialogs.len();
        let transaction_mapping_count = self.transaction_to_dialog.len();

        debug!(
            "Transaction statistics: {} dialogs, {} transaction mappings",
            dialog_count, transaction_mapping_count
        );
        (dialog_count, transaction_mapping_count)
    }

    /// Resolve the configured 100rel policy for outgoing INVITEs.
    ///
    /// Reads `DialogConfig.use_100rel` from the unified config when present,
    /// otherwise defaults to `RelUsage::Supported` (advertise capability).
    pub fn config_100rel_policy(&self) -> RelUsage {
        self.config
            .read()
            .ok()
            .and_then(|g| g.as_ref().map(|c| c.dialog_config().use_100rel))
            .unwrap_or_default()
    }

    /// Resolve session-timer settings for outgoing INVITEs.
    ///
    /// Returns `Some((session_expires_secs, min_se_secs))` when session
    /// timers are enabled in the config, otherwise `None`.
    pub fn config_session_timer_settings(&self) -> Option<(u32, u32)> {
        self.config.read().ok().and_then(|g| {
            g.as_ref().and_then(|c| {
                let dc = c.dialog_config();
                dc.session_timer_secs
                    .map(|secs| (secs, dc.session_timer_min_se))
            })
        })
    }

    /// Send a PRACK request acknowledging a reliable provisional (RFC 3262 §7.2).
    ///
    /// Builds a PRACK within the given dialog whose `RAck` header references the
    /// supplied `rseq` and the original INVITE's CSeq. A new non-INVITE client
    /// transaction is created and sent. This is the low-level send — callers that
    /// want auto-PRACK on receipt of a reliable 18x should go through
    /// `handle_transaction_provisional_response`.
    pub async fn send_prack(
        &self,
        dialog_id: &DialogId,
        rseq: u32,
    ) -> DialogResult<TransactionKey> {
        debug!(
            "Building PRACK for dialog {} acknowledging RSeq={}",
            dialog_id, rseq
        );

        let (destination, request) = {
            let mut dialog = self.get_dialog_mut(dialog_id)?;

            let fallback_destination =
                dialog.get_remote_target_address().await.ok_or_else(|| {
                    crate::errors::DialogError::routing_error(
                        "No remote target address available for PRACK",
                    )
                })?;

            let invite_cseq = dialog.invite_cseq.ok_or_else(|| {
                crate::errors::DialogError::protocol_error(
                    "Cannot send PRACK: dialog has no INVITE CSeq recorded",
                )
            })?;

            // Need both tags: PRACK is in-dialog and reliable 18x establishes an early dialog.
            let local_tag = dialog.local_tag.clone().ok_or_else(|| {
                crate::errors::DialogError::protocol_error("PRACK requires local tag")
            })?;
            let remote_tag = dialog.remote_tag.clone().ok_or_else(|| {
                crate::errors::DialogError::protocol_error(
                    "PRACK requires remote tag from the reliable 18x response",
                )
            })?;

            // Increment local CSeq for the PRACK (it's a new transaction).
            dialog.local_cseq += 1;
            let prack_cseq = dialog.local_cseq;
            let route_set = dialog.route_set.clone();
            let call_id = dialog.call_id.clone();
            let local_uri = dialog.local_uri.to_string();
            let target_uri = dialog.remote_uri.clone();
            let remote_uri = dialog.remote_uri.to_string();
            let local_address = self.local_address_for_target_and_routes(&target_uri, &route_set);

            let request = crate::transaction::dialog::prack_for_dialog(
                call_id,
                local_uri,
                local_tag,
                remote_uri,
                remote_tag,
                rseq,
                invite_cseq,
                prack_cseq,
                local_address,
                if route_set.is_empty() {
                    None
                } else {
                    Some(route_set)
                },
            )
            .map_err(|e| crate::errors::DialogError::InternalError {
                message: format!("Failed to build PRACK: {}", e),
                context: None,
            })?;

            let destination = self
                .resolve_uri_to_socketaddr(
                    &crate::transaction::transport::multiplexed::next_hop_uri_for_request(&request),
                )
                .await
                .unwrap_or(fallback_destination);

            (destination, request)
        };

        let request_key = crate::manager::core::outbound_request_key(&request);
        let next_hop =
            crate::transaction::transport::multiplexed::next_hop_uri_for_request(&request);
        let selected_transport = self
            .transaction_manager
            .get_best_transport_for_uri(&next_hop);
        let transaction_id = self
            .transaction_manager
            .create_non_invite_client_transaction(request, destination)
            .await
            .map_err(|e| crate::errors::DialogError::TransactionError {
                message: format!("Failed to create PRACK transaction: {}", e),
            })?;

        self.link_transaction_to_dialog_indexed(&transaction_id, dialog_id);

        self.transaction_manager
            .send_request(&transaction_id)
            .await
            .map_err(|e| crate::errors::DialogError::TransactionError {
                message: format!("Failed to send PRACK: {}", e),
            })?;
        self.record_outbound_transport_context(
            &transaction_id,
            request_key,
            selected_transport,
            destination,
        );

        info!(dialog=%dialog_id, transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&transaction_id), rseq, "Sent PRACK");
        Ok(transaction_id)
    }

    /// Cleanup orphaned transaction mappings
    ///
    /// Removes transaction-dialog mappings for terminated dialogs.
    pub async fn cleanup_orphaned_transaction_mappings(&self) -> usize {
        let mut orphaned_count = 0;
        let active_dialog_ids: std::collections::HashSet<crate::dialog::DialogId> = self
            .dialogs
            .iter()
            .map(|entry| entry.key().clone())
            .collect();

        // Collect orphaned transaction IDs
        let orphaned_transactions: Vec<TransactionKey> = self
            .transaction_to_dialog
            .iter()
            .filter_map(|entry| {
                let dialog_id = entry.value();
                if !active_dialog_ids.contains(dialog_id) {
                    Some(entry.key().clone())
                } else {
                    None
                }
            })
            .collect();

        // Remove orphaned mappings
        for tx_id in orphaned_transactions {
            self.unlink_transaction_from_dialog_indexed(&tx_id);
            orphaned_count += 1;
        }

        if orphaned_count > 0 {
            debug!(
                "Cleaned up {} orphaned transaction mappings",
                orphaned_count
            );
        }

        orphaned_count
    }
}
