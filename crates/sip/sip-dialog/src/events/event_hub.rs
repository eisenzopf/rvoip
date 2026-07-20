//! Dialog Event Hub for Global Event Coordination
//!
//! This module provides the central event hub that integrates dialog-core with the global
//! event coordinator from infra-common, replacing channel-based communication.

use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tracing::{debug, error, info, trace, warn};

use crate::transaction::TransactionKey;
use rvoip_infra_common::events::coordinator::{CrossCrateEventHandler, GlobalEventCoordinator};
use rvoip_infra_common::events::cross_crate::{
    CallState, CrossCrateEvent, DialogToSessionEvent, OutboundRequestOutcome, RvoipCrossCrateEvent,
    SessionToDialogEvent, TerminationReason,
};
use rvoip_sip_core::Method;

use crate::diagnostics::safe_log::method_class;
use crate::dialog::{DialogId, DialogState};
use crate::events::session_coordination::tracks_generic_outbound_request_completion;
use crate::events::{DialogEvent, SessionCoordinationEvent};
use crate::manager::DialogManager;

/// Dialog Event Hub that handles all cross-crate event communication
#[derive(Clone)]
pub struct DialogEventHub {
    /// Global event coordinator for cross-crate communication
    global_coordinator: Arc<GlobalEventCoordinator>,

    /// Reference to dialog manager for handling incoming events
    dialog_manager: Arc<DialogManager>,
}

impl std::fmt::Debug for DialogEventHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DialogEventHub")
            .field("global_coordinator", &"Arc<GlobalEventCoordinator>")
            .field("dialog_manager", &"Arc<DialogManager>")
            .finish()
    }
}

fn session_coordination_event_kind(event: &SessionCoordinationEvent) -> &'static str {
    match event {
        SessionCoordinationEvent::IncomingCall { .. } => "incoming_call",
        SessionCoordinationEvent::AckReceived { .. } => "ack_received",
        SessionCoordinationEvent::ByeReceived { .. } => "bye_received",
        _ => "other",
    }
}

fn parse_event_transaction_key(value: &str) -> Result<TransactionKey> {
    value
        .parse::<TransactionKey>()
        .map_err(|_error| anyhow::anyhow!("Invalid transaction identifier"))
}

impl DialogEventHub {
    /// Create a new dialog event hub
    pub async fn new(
        global_coordinator: Arc<GlobalEventCoordinator>,
        dialog_manager: Arc<DialogManager>,
    ) -> Result<Arc<Self>> {
        let hub = Arc::new(Self {
            global_coordinator: global_coordinator.clone(),
            dialog_manager,
        });

        // Clone hub for registration (CrossCrateEventHandler must be implemented for DialogEventHub not Arc<DialogEventHub>)
        let handler = DialogEventHub {
            global_coordinator: global_coordinator.clone(),
            dialog_manager: hub.dialog_manager.clone(),
        };

        // Register as handler for session-to-dialog events
        global_coordinator
            .register_handler("session_to_dialog", handler.clone())
            .await?;

        // Register as handler for transport-to-dialog events
        global_coordinator
            .register_handler("transport_to_dialog", handler)
            .await?;

        info!("Dialog Event Hub initialized and registered with GlobalEventCoordinator");

        Ok(hub)
    }

    /// Publish a dialog event to the global bus
    pub async fn publish_dialog_event(&self, event: DialogEvent) -> Result<()> {
        debug!("Publishing dialog event");

        // Convert to cross-crate event if applicable
        if let Some(cross_crate_event) = self.convert_dialog_to_cross_crate(event) {
            self.global_coordinator
                .publish(Arc::new(cross_crate_event))
                .await?;
        }

        Ok(())
    }

    /// Publish a session coordination event to the global bus
    pub async fn publish_session_coordination_event(
        &self,
        event: SessionCoordinationEvent,
    ) -> Result<()> {
        let _ = self.try_publish_session_coordination_event(event).await?;
        Ok(())
    }

    /// Publish a session coordination event and report whether it mapped to a
    /// cross-crate event.
    pub(crate) async fn try_publish_session_coordination_event(
        &self,
        event: SessionCoordinationEvent,
    ) -> Result<bool> {
        debug!(
            "Publishing session coordination event class={}",
            session_coordination_event_kind(&event)
        );

        // Convert to cross-crate event
        if let Some(mut cross_crate_event) = self.convert_coordination_to_cross_crate(event.clone())
        {
            // STIR/SHAKEN (RFC 8224): run the installed verifier on
            // IncomingCall events before publishing. Uses the shared
            // `DialogManager::run_identity_verification` helper so the
            // adapter's publish path and this one apply the same
            // policy + reject contract — no drift between bridges.
            if let RvoipCrossCrateEvent::DialogToSession(DialogToSessionEvent::IncomingCall {
                ref raw_request,
                identity_verification: ref mut iv,
                ..
            }) = cross_crate_event
            {
                match self
                    .dialog_manager
                    .run_identity_verification(&event, raw_request)
                    .await
                {
                    crate::manager::IdentityVerificationDecision::Drop => {
                        debug!("STIR/SHAKEN rejected event; not publishing");
                        return Ok(false);
                    }
                    crate::manager::IdentityVerificationDecision::Publish(status) => {
                        *iv = status;
                    }
                }
            }

            let publish_kind = session_coordination_event_kind(&event);
            let publish_started =
                crate::diagnostics::dialog_timing_enabled().then(std::time::Instant::now);
            let handler_count = if crate::diagnostics::dialog_timing_enabled() {
                self.global_coordinator.stats().await.registered_handlers
            } else {
                0
            };
            self.global_coordinator
                .publish(Arc::new(cross_crate_event))
                .await?;
            if let Some(started) = publish_started {
                crate::diagnostics::record_global_publish(
                    publish_kind,
                    handler_count,
                    started.elapsed(),
                );
            }
            trace!("Published cross-crate event successfully");
            Ok(true)
        } else {
            trace!("convert_coordination_to_cross_crate returned None");
            Ok(false)
        }
    }

    /// Publish a BYE cleanup event through the acknowledged internal
    /// dialog-to-session route. Unlike the general observational API, this
    /// propagates handler failures and reports a missing handler.
    pub(crate) async fn publish_bye_received_authoritative(
        &self,
        dialog_id: DialogId,
    ) -> Result<bool> {
        let event = SessionCoordinationEvent::ByeReceived { dialog_id };
        self.publish_session_coordination_authoritative(event).await
    }

    /// Publish a response-bearing coordination event through the single
    /// acknowledged dialog-to-session handler.
    pub(crate) async fn publish_session_coordination_authoritative(
        &self,
        event: SessionCoordinationEvent,
    ) -> Result<bool> {
        let Some(cross_crate_event) = self.convert_coordination_to_cross_crate(event) else {
            return Ok(false);
        };
        self.global_coordinator
            .publish_authoritative(Arc::new(cross_crate_event))
            .await
    }

    /// Publish a cross-crate event directly
    pub async fn publish_cross_crate_event(&self, event: RvoipCrossCrateEvent) -> Result<()> {
        debug!("Publishing cross-crate event directly");
        self.global_coordinator.publish(Arc::new(event)).await?;
        Ok(())
    }

    /// Convert DialogEvent to cross-crate event
    fn convert_dialog_to_cross_crate(&self, event: DialogEvent) -> Option<RvoipCrossCrateEvent> {
        match event {
            DialogEvent::StateChanged {
                dialog_id,
                old_state: _,
                new_state,
            } => {
                // Map dialog states to cross-crate call states
                let call_state = match new_state {
                    DialogState::Initial => CallState::Initiating,
                    DialogState::Early => CallState::Ringing,
                    DialogState::Confirmed => CallState::Active,
                    DialogState::Recovering => CallState::Active, // Still active but recovering
                    DialogState::Terminated => CallState::Terminated,
                };

                // Get session ID from dialog ID mapping
                if let Some(session_id) = self.dialog_manager.get_session_id(&dialog_id) {
                    Some(RvoipCrossCrateEvent::DialogToSession(
                        DialogToSessionEvent::CallStateChanged {
                            session_id,
                            new_state: call_state,
                            reason: None,
                        },
                    ))
                } else {
                    warn!("No session ID found for dialog {:?}", dialog_id);
                    None
                }
            }

            DialogEvent::Terminated {
                dialog_id,
                reason: _,
            } => {
                if let Some(session_id) = self.dialog_manager.get_session_id(&dialog_id) {
                    Some(RvoipCrossCrateEvent::DialogToSession(
                        DialogToSessionEvent::CallTerminated {
                            session_id,
                            reason: TerminationReason::RemoteHangup,
                        },
                    ))
                } else {
                    None
                }
            }

            _ => None, // Other events are internal only
        }
    }

    /// Convert SessionCoordinationEvent to cross-crate event
    fn convert_coordination_to_cross_crate(
        &self,
        event: SessionCoordinationEvent,
    ) -> Option<RvoipCrossCrateEvent> {
        match event {
            SessionCoordinationEvent::IncomingCall {
                dialog_id,
                transaction_id,
                request,
                source,
            } => {
                let call_id = request
                    .call_id()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| format!("unknown-{}", uuid::Uuid::new_v4()));

                let from = request
                    .from()
                    .map(|f| f.to_string())
                    .unwrap_or_else(|| "anonymous".to_string());

                let to = request
                    .to()
                    .map(|t| t.to_string())
                    .unwrap_or_else(|| "unknown".to_string());

                let sdp_offer = if !request.body().is_empty() {
                    String::from_utf8(request.body().to_vec()).ok()
                } else {
                    None
                };

                // Generate session ID
                let session_id = format!("session-{}", uuid::Uuid::new_v4());

                // Store mapping
                self.dialog_manager.store_dialog_mapping(
                    &session_id,
                    dialog_id.clone(),
                    transaction_id.clone(),
                    request.clone(),
                    source,
                );

                // Include dialog_id in headers since IncomingCall doesn't have a dialog_id field
                let mut headers = std::collections::HashMap::new();
                headers.insert("X-Dialog-Id".to_string(), dialog_id.to_string());

                // Surface RFC 3325 P-Asserted-Identity / P-Preferred-Identity
                // verbatim so session-core can expose them on `IncomingCallInfo`
                // for trunk-side caller-ID assertion.
                for hdr in &request.headers {
                    match hdr {
                        rvoip_sip_core::types::TypedHeader::PAssertedIdentity(pai) => {
                            headers.insert("P-Asserted-Identity".to_string(), pai.to_string());
                        }
                        rvoip_sip_core::types::TypedHeader::PPreferredIdentity(ppi) => {
                            headers.insert("P-Preferred-Identity".to_string(), ppi.to_string());
                        }
                        _ => {}
                    }
                }

                // SIP_API_DESIGN_2 §7.5: surface the original wire
                // bytes that the transport parsed. The transaction
                // manager cached them via
                // `TransactionEvent::MessageReceived.raw_bytes`;
                // consuming here keeps the upstream form byte-exact for
                // STIR/SHAKEN Identity validation (RFC 8224) and
                // signature-preserving SBC pass-through. Fall back to
                // re-serialising if the cache entry is missing (e.g.,
                // mock transports that publish `raw_bytes: None`).
                if let Some(timing) = self
                    .dialog_manager
                    .transaction_manager()
                    .peek_inbound_timing(&transaction_id)
                {
                    if let Some(received_at) = timing.received_at {
                        crate::diagnostics::record_udp_receive_to_incoming_call_emit(
                            received_at.elapsed(),
                        );
                    }
                }
                let raw_request = self
                    .dialog_manager
                    .transaction_manager()
                    .take_inbound_bytes(&transaction_id)
                    .or_else(|| {
                        Some(bytes::Bytes::from(
                            rvoip_sip_core::Message::Request(request.clone()).to_bytes(),
                        ))
                    });
                let transport = self
                    .dialog_manager
                    .transaction_manager()
                    .take_inbound_transport(&transaction_id);

                Some(RvoipCrossCrateEvent::DialogToSession(
                    DialogToSessionEvent::IncomingCall {
                        session_id,
                        call_id,
                        from,
                        to,
                        sdp_offer,
                        headers,
                        transaction_id: transaction_id.to_string(),
                        source_addr: source.to_string(),
                        raw_request,
                        transport,
                        // STIR/SHAKEN Phase 1: verification is performed in
                        // `events/adapter.rs` (the legacy `convert_*` path
                        // that owns the raw bytes lifecycle). This bridge
                        // does not currently run the verifier; populated as
                        // `None` until the call lands on the verifier-aware
                        // publish path. Safe default — `Annotate` policy
                        // treats `None` identically to "no verifier
                        // installed."
                        identity_verification: None,
                    },
                ))
            }

            SessionCoordinationEvent::CallAnswered {
                dialog_id,
                session_answer,
            } => {
                debug!("Processing CallAnswered for dialog: {}", dialog_id);
                match self.dialog_manager.get_session_id(&dialog_id) {
                    Some(session_id) => {
                        crate::diagnostics::record_hub_call_answered_session(true);
                        crate::diagnostics::record_call_timing_hub_call_answered(
                            session_id.as_str(),
                        );
                        debug!("Found session ID {} for dialog {}", session_id, dialog_id);
                        Some(RvoipCrossCrateEvent::DialogToSession(
                            DialogToSessionEvent::CallEstablished {
                                session_id,
                                sdp_answer: Some(session_answer),
                                raw_response: None,
                            },
                        ))
                    }
                    None => {
                        crate::diagnostics::record_hub_call_answered_session(false);
                        warn!("❌ [event_hub] No session ID found for dialog {} in CallAnswered event", dialog_id);
                        None
                    }
                }
            }

            SessionCoordinationEvent::CallTerminating {
                dialog_id,
                reason: _,
            } => {
                if let Some(session_id) = self.dialog_manager.get_session_id(&dialog_id) {
                    Some(RvoipCrossCrateEvent::DialogToSession(
                        DialogToSessionEvent::CallTerminated {
                            session_id,
                            reason: TerminationReason::RemoteHangup,
                        },
                    ))
                } else {
                    None
                }
            }

            SessionCoordinationEvent::CallCancelled { dialog_id, .. } => self
                .dialog_manager
                .get_session_id(&dialog_id)
                .map(|session_id| {
                    RvoipCrossCrateEvent::DialogToSession(DialogToSessionEvent::CallCancelled {
                        session_id,
                    })
                }),

            SessionCoordinationEvent::SessionRefreshed {
                dialog_id,
                expires_secs,
            } => self
                .dialog_manager
                .get_session_id(&dialog_id)
                .map(|session_id| {
                    RvoipCrossCrateEvent::DialogToSession(DialogToSessionEvent::SessionRefreshed {
                        session_id,
                        expires_secs,
                    })
                }),

            SessionCoordinationEvent::SessionRefreshFailed { dialog_id, reason } => self
                .dialog_manager
                .get_session_id(&dialog_id)
                .map(|session_id| {
                    RvoipCrossCrateEvent::DialogToSession(
                        DialogToSessionEvent::SessionRefreshFailed { session_id, reason },
                    )
                }),

            SessionCoordinationEvent::OutboundFlowFailed {
                aor,
                reg_id,
                instance,
                reason,
            } => {
                // RFC 5626 flow-level event: no session_id association
                // (registrations can be coordinator-global, not tied to a
                // single dialog). Session-core locates the registration
                // session by matching the AoR in the handler.
                Some(RvoipCrossCrateEvent::DialogToSession(
                    DialogToSessionEvent::OutboundFlowFailed {
                        aor,
                        reg_id,
                        instance,
                        reason: format!("{:?}", reason),
                    },
                ))
            }

            SessionCoordinationEvent::ResponseReceived {
                dialog_id,
                response,
                transaction_id,
                request_uri,
            } => {
                // Try to get session ID from stored mapping first
                if let Some(session_id) = self.dialog_manager.get_session_id(&dialog_id) {
                    // SIP_API_DESIGN_2 §7.5: pull the original wire
                    // bytes the transport cached so STIR/SHAKEN and
                    // signature-preserving consumers see the upstream
                    // form. Fall back to re-serialising for synthetic
                    // responses (auto-100, fabricated timeouts) that
                    // never crossed the wire.
                    let raw_response = self
                        .dialog_manager
                        .transaction_manager()
                        .take_inbound_bytes(&transaction_id)
                        .or_else(|| {
                            Some(bytes::Bytes::from(
                                rvoip_sip_core::Message::Response(response.clone()).to_bytes(),
                            ))
                        });
                    // The transaction key is the locally-owned correlation
                    // authority. A response CSeq is peer input; transaction
                    // matching has already validated it, but it must not
                    // override the method used for lifecycle correlation.
                    let response_method = Some(transaction_id.method().clone());
                    let is_invite_response =
                        matches!(response_method, Some(rvoip_sip_core::Method::Invite));
                    if response.status_code() == 200 && is_invite_response {
                        crate::diagnostics::record_hub_response_invite_2xx_session(true);
                        crate::diagnostics::record_call_timing_hub_response_invite_2xx(
                            session_id.as_str(),
                        );
                    }
                    // Handle specific response codes
                    match response.status_code() {
                        200 if is_invite_response => {
                            // 200 OK - call established
                            let sdp_answer = if !response.body().is_empty() {
                                String::from_utf8(response.body().to_vec()).ok()
                            } else {
                                None
                            };

                            Some(RvoipCrossCrateEvent::DialogToSession(
                                DialogToSessionEvent::CallEstablished {
                                    session_id,
                                    sdp_answer,
                                    raw_response: raw_response.clone(),
                                },
                            ))
                        }
                        100..=199 if is_invite_response => {
                            // Preserve the actual provisional response. Session-core
                            // decides how to map it onto call state while apps can
                            // observe the response code, reason, and early-media SDP.
                            let sdp = if !response.body().is_empty() {
                                String::from_utf8(response.body().to_vec()).ok()
                            } else {
                                None
                            };
                            Some(RvoipCrossCrateEvent::DialogToSession(
                                DialogToSessionEvent::CallProgress {
                                    session_id,
                                    status_code: response.status_code(),
                                    reason_phrase: response.reason_phrase().to_string(),
                                    sdp,
                                    raw_response: raw_response.clone(),
                                },
                            ))
                        }
                        487 if is_invite_response => {
                            // RFC 3261 §15.1.2 — 487 Request Terminated follows a
                            // CANCEL. Publish from the response path as well
                            // as the explicit CallCancelled coordination path
                            // so client-side cancellation release is not lost
                            // if the final transaction event has already
                            // dropped its dialog lookup. Session-core treats a
                            // second cancellation as idempotent once the
                            // session has been released.
                            Some(RvoipCrossCrateEvent::DialogToSession(
                                DialogToSessionEvent::CallCancelled { session_id },
                            ))
                        }
                        491 if is_invite_response => {
                            // RFC 3261 §14.1 — 491 Request Pending on a
                            // re-INVITE. Session layer should wait a random
                            // backoff and retry. UPDATE is reported through the
                            // exact generic completion path below.
                            Some(RvoipCrossCrateEvent::DialogToSession(
                                DialogToSessionEvent::ReinviteGlare { session_id },
                            ))
                        }
                        422 if is_invite_response => {
                            // RFC 4028 §6 — Session Interval Too Small. The UAS's
                            // Min-SE: header carries the required floor. Pass
                            // that value up so session-core can bump
                            // Session-Expires and retry. If Min-SE is missing or
                            // unparseable, fall through to generic CallFailed.
                            //
                            // Try the typed `TypedHeader::MinSE(MinSE)` first
                            // (produced by sip-core's parser for "Min-SE: <n>"
                            // lines), then fall back to untyped lookups for
                            // `HeaderName::MinSE` and `HeaderName::Other("Min-SE")`
                            // so we handle peers whose headers were stored as
                            // opaque types too.
                            use rvoip_sip_core::types::headers::HeaderAccess;
                            let min_se = response
                                .headers
                                .iter()
                                .find_map(|h| match h {
                                    rvoip_sip_core::TypedHeader::MinSE(m) => Some(m.delta_seconds),
                                    _ => None,
                                })
                                .or_else(|| {
                                    response
                                        .raw_header_value(
                                            &rvoip_sip_core::types::header::HeaderName::MinSE,
                                        )
                                        .and_then(|s| {
                                            s.trim()
                                                .split(|c: char| !c.is_ascii_digit())
                                                .next()
                                                .and_then(|n| n.parse::<u32>().ok())
                                        })
                                })
                                .or_else(|| {
                                    response
                                        .raw_header_value(
                                            &rvoip_sip_core::types::header::HeaderName::Other(
                                                "Min-SE".to_string(),
                                            ),
                                        )
                                        .and_then(|s| {
                                            s.trim()
                                                .split(|c: char| !c.is_ascii_digit())
                                                .next()
                                                .and_then(|n| n.parse::<u32>().ok())
                                        })
                                });
                            if let Some(min_se_secs) = min_se {
                                Some(RvoipCrossCrateEvent::DialogToSession(
                                    DialogToSessionEvent::SessionIntervalTooSmall {
                                        session_id,
                                        min_se_secs,
                                    },
                                ))
                            } else {
                                Some(RvoipCrossCrateEvent::DialogToSession(
                                    DialogToSessionEvent::CallFailed {
                                        session_id,
                                        status_code: 422,
                                        reason_phrase: response.reason_phrase().to_string(),
                                        raw_response: raw_response.clone(),
                                    },
                                ))
                            }
                        }
                        code if is_invite_response && (300..400).contains(&code) => {
                            // RFC 3261 §8.1.3.4 / §21.3 — redirect. Extract Contact
                            // URIs with q-values so the UAC can retry. Any 3xx
                            // response carries one or more Contact: headers per
                            // §19.1.5.
                            let mut targets: Vec<String> = Vec::new();
                            let mut q_values: Vec<f32> = Vec::new();
                            for header in &response.headers {
                                if let rvoip_sip_core::TypedHeader::Contact(contact) = header {
                                    for address in contact.addresses() {
                                        targets.push(address.uri.to_string());
                                        let q = address
                                            .params
                                            .iter()
                                            .find_map(|p| {
                                                if let rvoip_sip_core::types::param::Param::Q(v) = p
                                                {
                                                    Some(*v.as_ref() as f32)
                                                } else {
                                                    None
                                                }
                                            })
                                            .unwrap_or(1.0);
                                        q_values.push(q);
                                    }
                                }
                            }
                            Some(RvoipCrossCrateEvent::DialogToSession(
                                DialogToSessionEvent::CallRedirected {
                                    session_id,
                                    status_code: code,
                                    targets,
                                    q_values,
                                },
                            ))
                        }
                        401 | 407 => {
                            // RFC 3261 §22.2 — SIP auth challenge. If the
                            // response carries a WWW-Authenticate (401) or
                            // Proxy-Authenticate (407) header, surface it as
                            // `AuthRequired` so session-core can negotiate a
                            // configured auth response and retry. Method-agnostic:
                            // this path fires for INVITE, REGISTER, and any
                            // future auth-challenged request. A 401/407
                            // without a challenge header falls through
                            // to CallFailed below.
                            //
                            // SIP_API_DESIGN_2 R2 — also extract the SIP
                            // method from `CSeq:` so session-core can route
                            // the retry to the matching per-method auth
                            // handler. CSeq's method field is mandatory per
                            // RFC 3261 §20.16; if it's somehow absent we
                            // fall back to "" and the consumer treats it as
                            // method-agnostic.
                            use rvoip_sip_core::types::headers::HeaderAccess;
                            let header_name = if response.status_code() == 407 {
                                rvoip_sip_core::types::header::HeaderName::ProxyAuthenticate
                            } else {
                                rvoip_sip_core::types::header::HeaderName::WwwAuthenticate
                            };
                            let method = transaction_id.method().to_string();
                            let challenges = response
                                .raw_headers(&header_name)
                                .into_iter()
                                .filter_map(|bytes| String::from_utf8(bytes).ok())
                                .collect::<Vec<_>>();
                            if !challenges.is_empty() && request_uri.is_some() {
                                let challenge = challenges.join(", ");
                                let realm = extract_digest_realm(&challenge);
                                Some(RvoipCrossCrateEvent::DialogToSession(
                                    DialogToSessionEvent::AuthRequired {
                                        session_id,
                                        transaction_id: transaction_id.to_string(),
                                        request_uri: request_uri
                                            .expect("request URI checked above")
                                            .to_string(),
                                        status_code: response.status_code(),
                                        challenge,
                                        realm,
                                        method,
                                        outbound_transport: self
                                            .dialog_manager
                                            .outbound_transport_context_for_transaction(
                                                &transaction_id,
                                            ),
                                    },
                                ))
                            } else if tracks_generic_outbound_request_completion(
                                transaction_id.method(),
                            ) {
                                // Missing challenge material or the exact
                                // challenged Request-URI cannot be retried
                                // safely. Treat the attempt as an ordinary
                                // terminal response; never reconstruct the URI
                                // from session/dialog metadata.
                                Some(RvoipCrossCrateEvent::DialogToSession(
                                    DialogToSessionEvent::OutboundRequestCompleted {
                                        session_id,
                                        transaction_id: transaction_id.to_string(),
                                        method: transaction_id.method().to_string(),
                                        outcome: OutboundRequestOutcome::FinalResponse {
                                            status_code: response.status_code(),
                                        },
                                    },
                                ))
                            } else if !is_invite_response {
                                None
                            } else {
                                Some(RvoipCrossCrateEvent::DialogToSession(
                                    DialogToSessionEvent::CallFailed {
                                        session_id,
                                        status_code: response.status_code(),
                                        reason_phrase: response.reason_phrase().to_string(),
                                        raw_response: raw_response.clone(),
                                    },
                                ))
                            }
                        }
                        code if is_invite_response && (400..700).contains(&code) => {
                            // RFC 3261 §8.1.3 — any other final 3xx/4xx/5xx/6xx
                            // response ends the UAC's INVITE transaction. Propagate
                            // to the session layer so it can emit CallFailed and
                            // run the Dialog{4,5,6}xxFailure state transitions.
                            Some(RvoipCrossCrateEvent::DialogToSession(
                                DialogToSessionEvent::CallFailed {
                                    session_id,
                                    status_code: code,
                                    reason_phrase: response.reason_phrase().to_string(),
                                    raw_response: raw_response.clone(),
                                },
                            ))
                        }
                        code if code >= 200
                            && tracks_generic_outbound_request_completion(
                                transaction_id.method(),
                            ) =>
                        {
                            Some(RvoipCrossCrateEvent::DialogToSession(
                                DialogToSessionEvent::OutboundRequestCompleted {
                                    session_id,
                                    transaction_id: transaction_id.to_string(),
                                    method: transaction_id.method().to_string(),
                                    outcome: OutboundRequestOutcome::FinalResponse {
                                        status_code: code,
                                    },
                                },
                            ))
                        }
                        _ => None,
                    }
                } else {
                    let is_invite_2xx = response.status_code() == 200
                        && matches!(
                            response.cseq().map(|cseq| cseq.method.clone()),
                            Some(rvoip_sip_core::Method::Invite)
                        );
                    if is_invite_2xx {
                        crate::diagnostics::record_hub_response_invite_2xx_session(false);
                    }
                    warn!("No session ID found for dialog {:?}", dialog_id);
                    None
                }
            }

            SessionCoordinationEvent::OutboundRequestCompleted {
                dialog_id,
                transaction_id,
                method,
                outcome,
            } => {
                if !tracks_generic_outbound_request_completion(&method) {
                    return None;
                }
                self.dialog_manager
                    .get_session_id(&dialog_id)
                    .map(|session_id| {
                        RvoipCrossCrateEvent::DialogToSession(
                            DialogToSessionEvent::OutboundRequestCompleted {
                                session_id,
                                transaction_id: transaction_id.to_string(),
                                method: method.to_string(),
                                outcome,
                            },
                        )
                    })
            }

            SessionCoordinationEvent::TransferRequest {
                dialog_id,
                transaction_id,
                refer_to,
                referred_by,
                replaces,
                raw_request,
            } => {
                if let Some(session_id) = self.dialog_manager.get_session_id(&dialog_id) {
                    // Convert ReferTo to string
                    let refer_to_uri = refer_to.uri().to_string();

                    // Determine transfer type based on Replaces header
                    let transfer_type = if replaces.is_some() {
                        rvoip_infra_common::events::cross_crate::TransferType::Attended
                    } else {
                        rvoip_infra_common::events::cross_crate::TransferType::Blind
                    };

                    // SIP_API_DESIGN_2 §7.5 — thread the inbound REFER
                    // bytes through to the cross-crate variant so
                    // session-core can build a typed `IncomingRequest`
                    // view. A `None` here at this point means the
                    // publish site upstream (`protocol_handlers.rs`)
                    // did not preserve the bytes; warn loudly so the
                    // regression is observable.
                    if raw_request.is_none() {
                        tracing::warn!(
                            "TransferRequest cross-crate bridge: raw_request was None — \
                             upstream publish site did not preserve REFER bytes"
                        );
                    }

                    Some(RvoipCrossCrateEvent::DialogToSession(
                        DialogToSessionEvent::TransferRequested {
                            session_id,
                            refer_to: refer_to_uri,
                            transfer_type,
                            transaction_id: transaction_id.to_string(),
                            referred_by,
                            replaces,
                            raw_request,
                            transport: self
                                .dialog_manager
                                .transaction_manager()
                                .take_inbound_transport(&transaction_id),
                        },
                    ))
                } else {
                    warn!(
                        "No session ID found for dialog {:?} in TransferRequest",
                        dialog_id
                    );
                    None
                }
            }

            // ACK events for state machine transitions
            SessionCoordinationEvent::AckSent { dialog_id, .. } => {
                // AckSent is primarily for UAC - session layer may need to know ACK was sent
                // but typically this isn't needed for state transitions
                // We'll pass it through in case session-core-v2 wants to track it
                debug!(
                    "AckSent event for dialog {}, converting to cross-crate format",
                    dialog_id
                );
                if let Some(session_id) = self.dialog_manager.get_session_id(&dialog_id) {
                    crate::diagnostics::record_hub_ack_sent_session(true);
                    crate::diagnostics::record_call_timing_hub_ack_sent(session_id.as_str());
                } else {
                    crate::diagnostics::record_hub_ack_sent_session(false);
                }
                None // For now, UAC doesn't need this event
            }

            SessionCoordinationEvent::AckReceived {
                dialog_id,
                negotiated_sdp,
                ..
            } => {
                // AckReceived is critical for UAS - dialog-core received ACK, now session must transition
                if let Some(session_id) = self.dialog_manager.get_session_id(&dialog_id) {
                    crate::diagnostics::record_ack_matched_session();
                    crate::diagnostics::record_call_timing_uas_ack_received(session_id.as_str());
                    debug!(
                        "Converting AckReceived to cross-crate event for session {}",
                        session_id
                    );
                    Some(RvoipCrossCrateEvent::DialogToSession(
                        DialogToSessionEvent::AckReceived {
                            session_id,
                            sdp: negotiated_sdp,
                        },
                    ))
                } else {
                    crate::diagnostics::record_ack_unmatched_session();
                    warn!(
                        "No session ID found for dialog {:?} in AckReceived",
                        dialog_id
                    );
                    None
                }
            }

            SessionCoordinationEvent::CallTerminating {
                dialog_id,
                reason: _,
            } => {
                // When BYE completes, notify session-core that dialog is terminating
                if let Some(session_id) = self.dialog_manager.get_session_id(&dialog_id) {
                    debug!(
                        "Converting CallTerminating to CallTerminated for session {}",
                        session_id
                    );
                    Some(RvoipCrossCrateEvent::DialogToSession(
                        DialogToSessionEvent::CallTerminated {
                            session_id,
                            reason: rvoip_infra_common::events::cross_crate::TerminationReason::RemoteHangup,
                        }
                    ))
                } else {
                    warn!(
                        "No session ID found for dialog {:?} in CallTerminating",
                        dialog_id
                    );
                    None
                }
            }

            SessionCoordinationEvent::ByeReceived { dialog_id } => {
                if let Some(session_id) = self.dialog_manager.get_session_id(&dialog_id) {
                    debug!(
                        "Converting inbound BYE to cross-crate event for session {}",
                        session_id
                    );
                    Some(RvoipCrossCrateEvent::DialogToSession(
                        DialogToSessionEvent::ByeReceived { session_id },
                    ))
                } else {
                    warn!(
                        "No session ID found for dialog {:?} in ByeReceived",
                        dialog_id
                    );
                    None
                }
            }

            // 180 Ringing reached the UAC. Surface the exact provisional
            // status; session-core will also drive the Ringing state change.
            SessionCoordinationEvent::CallRinging { dialog_id } => self
                .dialog_manager
                .get_session_id(&dialog_id)
                .map(|session_id| {
                    RvoipCrossCrateEvent::DialogToSession(DialogToSessionEvent::CallProgress {
                        session_id,
                        status_code: 180,
                        reason_phrase: "Ringing".to_string(),
                        sdp: None,
                        raw_response: None,
                    })
                }),

            SessionCoordinationEvent::EarlyMedia { dialog_id, sdp } => self
                .dialog_manager
                .get_session_id(&dialog_id)
                .map(|session_id| {
                    RvoipCrossCrateEvent::DialogToSession(DialogToSessionEvent::CallProgress {
                        session_id,
                        status_code: 183,
                        reason_phrase: "Session Progress".to_string(),
                        sdp: Some(sdp),
                        raw_response: None,
                    })
                }),

            SessionCoordinationEvent::CallProgress {
                dialog_id,
                status_code,
                reason_phrase,
            } => self
                .dialog_manager
                .get_session_id(&dialog_id)
                .map(|session_id| {
                    RvoipCrossCrateEvent::DialogToSession(DialogToSessionEvent::CallProgress {
                        session_id,
                        status_code,
                        reason_phrase,
                        sdp: None,
                        raw_response: None,
                    })
                }),

            // DTMF events would be handled separately if implemented
            // SessionCoordinationEvent doesn't have DtmfReceived yet

            // Mid-dialog INVITE (re-INVITE) or UPDATE. Session-core drives
            // the UAS-side response (200 OK for normal, 491 Request Pending
            // on glare) through its state machine. INFO and NOTIFY are also
            // emitted via this variant today — we deliberately skip them
            // here so they do not get misrouted to the re-INVITE handler.
            //
            // SIP_API_DESIGN_2 Phase E: today's protocol handlers route
            // inbound in-dialog INFO and MESSAGE through this same
            // `ReInvite` variant (it's the only mid-dialog request
            // variant). We dispatch by method so each gets its own
            // cross-crate variant.
            SessionCoordinationEvent::ReInvite {
                dialog_id,
                request,
                transaction_id,
            } => {
                let method = request.method();
                let raw_request = self
                    .dialog_manager
                    .transaction_manager()
                    .take_inbound_bytes(&transaction_id)
                    .or_else(|| Some(bytes::Bytes::from(request.to_bytes())));
                let transport = self
                    .dialog_manager
                    .transaction_manager()
                    .take_inbound_transport(&transaction_id);
                let session_id = match self.dialog_manager.get_session_id(&dialog_id) {
                    Some(s) => s,
                    None => {
                        warn!(
                            "No session ID found for dialog {:?} in mid-dialog {} request",
                            dialog_id,
                            method_class(&method)
                        );
                        return None;
                    }
                };
                match method {
                    Method::Invite | Method::Update => {
                        let sdp = if !request.body().is_empty() {
                            String::from_utf8(request.body().to_vec()).ok()
                        } else {
                            None
                        };
                        debug!(
                            "Converting ReInvite ({}) to cross-crate event for session {}",
                            method_class(&method),
                            session_id
                        );
                        Some(RvoipCrossCrateEvent::DialogToSession(
                            DialogToSessionEvent::ReinviteReceived {
                                session_id,
                                sdp,
                                method: method.to_string(),
                                raw_request,
                                transport,
                            },
                        ))
                    }
                    Method::Info => Some(RvoipCrossCrateEvent::DialogToSession(
                        DialogToSessionEvent::InfoReceived {
                            session_id,
                            transaction_id: transaction_id.to_string(),
                            raw_request,
                            transport,
                        },
                    )),
                    Method::Message => Some(RvoipCrossCrateEvent::DialogToSession(
                        DialogToSessionEvent::MessageReceived {
                            session_id,
                            raw_request,
                            transport,
                        },
                    )),
                    _ => {
                        debug!(
                            "Skipping ReInvite cross-crate conversion for method {:?} (dialog {})",
                            method_class(&method),
                            dialog_id
                        );
                        None
                    }
                }
            }

            // SIP_API_DESIGN_2 Phase E — bridge inbound OPTIONS to
            // session-core. In-dialog OPTIONS rides the existing
            // dialog mapping; out-of-dialog OPTIONS has an empty
            // session_id (the cross-crate `session_id()` accessor
            // normalizes the empty string to `None`).
            SessionCoordinationEvent::CapabilityQuery {
                request,
                transaction_id,
                ..
            } => {
                let raw_request = self
                    .dialog_manager
                    .transaction_manager()
                    .take_inbound_bytes(&transaction_id)
                    .or_else(|| Some(bytes::Bytes::from(request.to_bytes())));
                let transport = self
                    .dialog_manager
                    .transaction_manager()
                    .take_inbound_transport(&transaction_id);
                // CapabilityQuery in today's dialog-core does not carry
                // a dialog id; OPTIONS therefore surfaces as
                // out-of-dialog (empty session_id, which the cross-
                // crate session_id() accessor returns as `None`).
                Some(RvoipCrossCrateEvent::DialogToSession(
                    DialogToSessionEvent::OptionsReceived {
                        session_id: String::new(),
                        raw_request,
                        transport,
                    },
                ))
            }

            _ => None, // Other events not yet mapped
        }
    }
}

#[async_trait]
impl CrossCrateEventHandler for DialogEventHub {
    async fn handle(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()> {
        debug!("Handling cross-crate event: {}", event.event_type());

        // Use trait-based downcasting via as_any()
        if let Some(concrete) = event.as_any().downcast_ref::<RvoipCrossCrateEvent>() {
            match concrete {
                RvoipCrossCrateEvent::SessionToDialog(session_event) => {
                    match session_event {
                        SessionToDialogEvent::SendRegisterResponse {
                            transaction_id,
                            status_code,
                            reason,
                            www_authenticate,
                            contact,
                            expires,
                            min_expires,
                            service_route,
                            path_echo,
                            associated_uri,
                            extra_headers,
                        } => {
                            info!(
                                "📩 Handling SendRegisterResponse via trait: {} reason_present={}",
                                status_code,
                                !reason.is_empty()
                            );
                            self.handle_register_response_with_extras(
                                transaction_id,
                                *status_code,
                                reason,
                                www_authenticate.as_deref(),
                                contact.as_deref(),
                                *expires,
                                *min_expires,
                                service_route,
                                *path_echo,
                                associated_uri,
                                extra_headers,
                            )
                            .await?;
                            return Ok(()); // Early return after handling
                        }
                        SessionToDialogEvent::StoreDialogMapping {
                            session_id,
                            dialog_id,
                        } => {
                            self.store_dialog_mapping(session_id, dialog_id);
                            return Ok(());
                        }
                        SessionToDialogEvent::ReferResponse {
                            transaction_id,
                            accept,
                            status_code,
                            reason,
                        } => {
                            self.handle_refer_response_parts(
                                transaction_id,
                                *accept,
                                *status_code,
                                reason,
                            )
                            .await?;
                            return Ok(());
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        match event.event_type() {
            "transport_to_dialog" => {
                info!("Processing transport-to-dialog event");
                // Handle transport events if needed
            }

            _ => {
                debug!("Unhandled event type: {}", event.event_type());
            }
        }

        Ok(())
    }
}

impl DialogEventHub {
    fn store_dialog_mapping(&self, session_id: &str, dialog_id: &str) {
        debug!(
            session_bytes = session_id.len(),
            dialog_bytes = dialog_id.len(),
            "Storing typed dialog mapping"
        );
        if let Ok(uuid) = dialog_id.parse::<uuid::Uuid>() {
            let parsed_dialog_id = DialogId(uuid);
            self.dialog_manager
                .session_to_dialog
                .insert(session_id.to_string(), parsed_dialog_id.clone());
            self.dialog_manager
                .dialog_to_session
                .insert(parsed_dialog_id, session_id.to_string());
            info!("Stored typed dialog mapping");
        } else {
            warn!("Failed to parse dialog mapping identifier");
        }
    }

    /// Handle SendRegisterResponse event from session-core
    async fn handle_register_response(
        &self,
        transaction_id: &str,
        status_code: u16,
        reason: &str,
        www_authenticate: Option<&str>,
        contact: Option<&str>,
        expires: Option<u32>,
    ) -> Result<()> {
        debug!(
            "Handling SendRegisterResponse: status={} reason_present={}",
            status_code,
            !reason.is_empty()
        );

        // Parse transaction_id to TransactionKey
        let tx_key = parse_event_transaction_key(transaction_id)?;

        // Check if this transaction exists in our dialog manager
        // This prevents multiple DialogEventHubs from trying to handle the same event
        if self
            .dialog_manager
            .transaction_manager()
            .original_request(&tx_key)
            .await
            .is_err()
        {
            debug!("Transaction not found in this DialogManager - skipping");
            return Ok(()); // Not our transaction, skip silently
        }

        // Call the dialog manager's send_register_response method
        self.dialog_manager
            .send_register_response(
                &tx_key,
                status_code,
                reason,
                www_authenticate,
                contact,
                expires,
            )
            .await
            .map_err(|_error| anyhow::anyhow!("REGISTER response send failed"))?;

        info!(
            "✅ Sent REGISTER response: {} reason_present={}",
            status_code,
            !reason.is_empty()
        );
        Ok(())
    }

    /// SIP_API_DESIGN_2 Phase D — registrar response with the full
    /// set of additive fields (Min-Expires, Service-Route, Path echo,
    /// P-Associated-URI, generic extras). Falls back to the legacy
    /// path when no new fields are populated so existing callers see
    /// no behaviour change.
    #[allow(clippy::too_many_arguments)]
    async fn handle_register_response_with_extras(
        &self,
        transaction_id: &str,
        status_code: u16,
        reason: &str,
        www_authenticate: Option<&str>,
        contact: Option<&str>,
        expires: Option<u32>,
        min_expires: Option<u32>,
        service_route: &[String],
        path_echo: bool,
        associated_uri: &[String],
        extra_headers: &[(String, String)],
    ) -> Result<()> {
        let has_extras = min_expires.is_some()
            || !service_route.is_empty()
            || path_echo
            || !associated_uri.is_empty()
            || !extra_headers.is_empty();

        if !has_extras {
            return self
                .handle_register_response(
                    transaction_id,
                    status_code,
                    reason,
                    www_authenticate,
                    contact,
                    expires,
                )
                .await;
        }

        let tx_key = parse_event_transaction_key(transaction_id)?;

        if self
            .dialog_manager
            .transaction_manager()
            .original_request(&tx_key)
            .await
            .is_err()
        {
            debug!("Transaction not found in this DialogManager - skipping");
            return Ok(());
        }

        self.dialog_manager
            .send_register_response_with_extras(
                &tx_key,
                status_code,
                reason,
                www_authenticate,
                contact,
                expires,
                min_expires,
                service_route,
                path_echo,
                associated_uri,
                extra_headers,
            )
            .await
            .map_err(|_error| anyhow::anyhow!("REGISTER response send failed"))?;

        info!(
            "✅ Sent REGISTER response (with {} extras): {} reason_present={}",
            extra_headers.len(),
            status_code,
            !reason.is_empty()
        );
        Ok(())
    }

    /// Handle a typed ReferResponse event from session-core.
    async fn handle_refer_response_parts(
        &self,
        transaction_id: &str,
        accept: bool,
        status_code: u16,
        reason: &str,
    ) -> Result<()> {
        info!(
            "Handling ReferResponse: accept={}, status={} reason_present={}",
            accept,
            status_code,
            !reason.is_empty()
        );

        // Parse transaction_id and send response
        if let Ok(tx_key) = transaction_id.parse::<crate::transaction::TransactionKey>() {
            use rvoip_sip_core::StatusCode;
            let status = StatusCode::from_u16(status_code).unwrap_or(StatusCode::Accepted);

            // Get original REFER request to build proper response
            match self
                .dialog_manager
                .transaction_manager()
                .original_request(&tx_key)
                .await
            {
                Ok(Some(original_request)) => {
                    // Build proper response using the original request
                    let response = crate::transaction::utils::response_builders::create_response(
                        &original_request,
                        status,
                    );

                    if let Err(_error) = self.dialog_manager.send_response(&tx_key, response).await
                    {
                        error!("Failed to send REFER response");
                    } else {
                        info!(
                            "Successfully sent REFER response: {} reason_present={}",
                            status_code,
                            !reason.is_empty()
                        );
                    }
                }
                Ok(None) => {
                    // Demoted to debug — common during test teardown when
                    // the REFER transaction completes before the
                    // ReferResponse event is processed.
                    debug!("No original request found for transaction");
                }
                Err(_error) => {
                    // Same teardown race as above; surface as debug
                    // rather than error so test logs stay readable.
                    debug!("Failed to get original request for transaction");
                }
            }
        } else {
            error!("Failed to parse transaction identifier");
        }

        Ok(())
    }
}

/// Extract the `realm="..."` parameter from a `Digest` challenge header value,
/// for logging / app-level routing. The authoritative parse happens in
/// session-core via `auth-core::DigestAuthenticator::parse_challenge`, so
/// this helper deliberately does the minimum needed to populate the optional
/// `realm` field on `AuthRequired`. Returns `None` for non-digest schemes.
fn extract_digest_realm(challenge: &str) -> Option<String> {
    let marker = "realm=\"";
    let start = challenge.find(marker)? + marker.len();
    let rest = &challenge[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

#[cfg(test)]
mod safe_diagnostic_tests {
    use super::*;
    use std::net::SocketAddr;
    use tokio::sync::{mpsc, Mutex};

    use rvoip_infra_common::events::{EventCoordinatorConfig, GlobalEventCoordinator};
    use rvoip_sip_core::types::auth::{ProxyAuthenticate, WwwAuthenticate};
    use rvoip_sip_core::{Message, Request, Response, StatusCode, TypedHeader, Uri};
    use rvoip_sip_transport::{Error as TransportError, Transport};

    #[derive(Debug)]
    struct TestTransport {
        local_addr: SocketAddr,
        sent: Mutex<Vec<(Message, SocketAddr)>>,
    }

    #[async_trait::async_trait]
    impl Transport for TestTransport {
        async fn send_message(
            &self,
            message: Message,
            destination: SocketAddr,
        ) -> Result<(), TransportError> {
            self.sent.lock().await.push((message, destination));
            Ok(())
        }

        fn local_addr(&self) -> Result<SocketAddr, TransportError> {
            Ok(self.local_addr)
        }

        async fn close(&self) -> Result<(), TransportError> {
            Ok(())
        }

        fn is_closed(&self) -> bool {
            false
        }
    }

    async fn test_hub() -> (DialogEventHub, DialogId) {
        let local_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let transport = Arc::new(TestTransport {
            local_addr,
            sent: Mutex::new(Vec::new()),
        });
        let (_transport_tx, transport_rx) = mpsc::channel(8);
        let (transaction_manager, _transaction_events) =
            crate::transaction::TransactionManager::new(transport, transport_rx, Some(16))
                .await
                .unwrap();
        let dialog_manager = Arc::new(
            DialogManager::new(Arc::new(transaction_manager), local_addr)
                .await
                .unwrap(),
        );
        let dialog_id = DialogId::new();
        dialog_manager
            .dialog_to_session
            .insert(dialog_id.clone(), "session-terminal-test".to_string());
        let coordinator = Arc::new(
            GlobalEventCoordinator::new(EventCoordinatorConfig::default())
                .await
                .unwrap(),
        );
        (
            DialogEventHub {
                global_coordinator: coordinator,
                dialog_manager,
            },
            dialog_id,
        )
    }

    fn transaction(method: Method) -> TransactionKey {
        TransactionKey::new("z9hG4bK-exact-terminal-test".to_string(), method, false)
    }

    fn response_event(
        dialog_id: &DialogId,
        transaction_id: &TransactionKey,
        status_code: u16,
        request_uri: Option<Uri>,
    ) -> SessionCoordinationEvent {
        SessionCoordinationEvent::ResponseReceived {
            dialog_id: dialog_id.clone(),
            response: Response::new(StatusCode::from_u16(status_code).unwrap()),
            transaction_id: transaction_id.clone(),
            request_uri,
        }
    }

    #[test]
    fn malformed_transaction_error_does_not_reflect_input() {
        const SECRET: &str = "event-transaction-secret-canary\r\nX-Leak: yes";
        let error = parse_event_transaction_key(SECRET).expect_err("malformed key");
        let rendered = error.to_string();

        assert_eq!(rendered, "Invalid transaction identifier");
        assert!(!rendered.contains(SECRET));
        assert!(!rendered.contains("X-Leak"));
    }

    #[test]
    fn session_to_dialog_routing_never_parses_debug_text() {
        let source = include_str!("event_hub.rs");
        assert!(source.contains("SessionToDialogEvent::StoreDialogMapping"));
        assert!(source.contains("SessionToDialogEvent::ReferResponse"));
        assert!(source.contains("handle_refer_response_parts"));
        for forbidden in [
            ["let event_str = ", "format!(\"{:?}\", event)"].concat(),
            ["async fn handle_refer_response(&self, ", "event_str"].concat(),
            ["fn extract_field(&self, ", "event_str"].concat(),
        ] {
            assert!(!source.contains(&forbidden), "legacy fallback returned");
        }
    }

    #[tokio::test]
    async fn raw_message_fallback_preserves_binary_request_body() {
        let (hub, dialog_id) = test_hub().await;
        let mut request = Request::new(
            Method::Message,
            "sip:binary@example.invalid".parse().unwrap(),
        );
        request.body = bytes::Bytes::from_static(&[0x09, 0x00, 0xfe, 0x0d, 0x0a, 0x07]);
        let expected = request.to_bytes();

        let mapped = hub
            .convert_coordination_to_cross_crate(SessionCoordinationEvent::ReInvite {
                dialog_id,
                transaction_id: transaction(Method::Message),
                request,
            })
            .expect("in-dialog MESSAGE must map");

        assert!(matches!(
            mapped,
            RvoipCrossCrateEvent::DialogToSession(DialogToSessionEvent::MessageReceived {
                raw_request: Some(raw_request),
                ..
            }) if raw_request.as_ref() == expected.as_slice()
        ));
    }

    #[tokio::test]
    async fn raw_response_fallback_preserves_binary_body() {
        let (hub, dialog_id) = test_hub().await;
        let mut response = Response::new(StatusCode::Ringing);
        response.body = bytes::Bytes::from_static(&[0x09, 0x00, 0xfe, 0x0d, 0x0a, 0x07]);
        let expected = Message::Response(response.clone()).to_bytes();

        let mapped = hub
            .convert_coordination_to_cross_crate(SessionCoordinationEvent::ResponseReceived {
                dialog_id,
                response,
                transaction_id: transaction(Method::Invite),
                request_uri: None,
            })
            .expect("provisional INVITE response must map");

        assert!(matches!(
            mapped,
            RvoipCrossCrateEvent::DialogToSession(DialogToSessionEvent::CallProgress {
                raw_response: Some(raw_response),
                ..
            }) if raw_response.as_ref() == expected.as_slice()
        ));
    }

    #[tokio::test]
    async fn non_invite_final_responses_map_to_exact_completion() {
        let (hub, dialog_id) = test_hub().await;
        let transaction_id = transaction(Method::Info);

        for method in [Method::Info, Method::Refer, Method::Notify, Method::Update] {
            let tracked_transaction = transaction(method.clone());
            let mapped = hub
                .convert_coordination_to_cross_crate(response_event(
                    &dialog_id,
                    &tracked_transaction,
                    200,
                    None,
                ))
                .expect("tracked non-INVITE method must complete");
            assert!(matches!(
                mapped,
                RvoipCrossCrateEvent::DialogToSession(
                    DialogToSessionEvent::OutboundRequestCompleted { ref method, .. }
                ) if method == &tracked_transaction.method().to_string()
            ));
        }

        for status_code in [200, 302, 401, 404, 407, 487, 503, 603] {
            let mapped = hub
                .convert_coordination_to_cross_crate(response_event(
                    &dialog_id,
                    &transaction_id,
                    status_code,
                    None,
                ))
                .expect("non-INVITE final response must map");
            assert!(matches!(
                mapped,
                RvoipCrossCrateEvent::DialogToSession(
                    DialogToSessionEvent::OutboundRequestCompleted {
                        ref session_id,
                        transaction_id: ref mapped_transaction,
                        ref method,
                        outcome: OutboundRequestOutcome::FinalResponse {
                            status_code: mapped_status,
                        },
                    }
                ) if session_id == "session-terminal-test"
                    && mapped_transaction == &transaction_id.to_string()
                    && method == "INFO"
                    && mapped_status == status_code
            ));
        }

        assert!(hub
            .convert_coordination_to_cross_crate(response_event(
                &dialog_id,
                &transaction_id,
                180,
                None,
            ))
            .is_none());

        for method in [
            Method::Bye,
            Method::Cancel,
            Method::Message,
            Method::Options,
            Method::Subscribe,
        ] {
            let untracked_transaction = transaction(method);
            assert!(hub
                .convert_coordination_to_cross_crate(response_event(
                    &dialog_id,
                    &untracked_transaction,
                    200,
                    None,
                ))
                .is_none());
        }
    }

    #[tokio::test]
    async fn invite_and_update_491_use_distinct_lifecycle_events() {
        let (hub, dialog_id) = test_hub().await;

        let invite_transaction = transaction(Method::Invite);
        let invite_event = hub
            .convert_coordination_to_cross_crate(response_event(
                &dialog_id,
                &invite_transaction,
                491,
                None,
            ))
            .expect("re-INVITE 491 must map to glare handling");
        assert!(matches!(
            invite_event,
            RvoipCrossCrateEvent::DialogToSession(DialogToSessionEvent::ReinviteGlare {
                ref session_id,
            }) if session_id == "session-terminal-test"
        ));

        let update_transaction = transaction(Method::Update);
        let update_event = hub
            .convert_coordination_to_cross_crate(response_event(
                &dialog_id,
                &update_transaction,
                491,
                None,
            ))
            .expect("tracked UPDATE 491 must complete its exact attempt");
        assert!(matches!(
            update_event,
            RvoipCrossCrateEvent::DialogToSession(
                DialogToSessionEvent::OutboundRequestCompleted {
                    ref session_id,
                    transaction_id: ref mapped_transaction,
                    ref method,
                    outcome: OutboundRequestOutcome::FinalResponse { status_code: 491 },
                }
            ) if session_id == "session-terminal-test"
                && mapped_transaction == &update_transaction.to_string()
                && method == "UPDATE"
        ));
    }

    #[tokio::test]
    async fn auth_challenge_requires_exact_request_uri_and_does_not_complete() {
        let (hub, dialog_id) = test_hub().await;
        let transaction_id = transaction(Method::Info);
        let request_uri: Uri = "sip:agent@auth.example.invalid".parse().unwrap();
        let mut response = Response::new(StatusCode::Unauthorized);
        response
            .headers
            .push(TypedHeader::WwwAuthenticate(WwwAuthenticate::new(
                "example-realm",
                "example-nonce",
            )));

        let mapped = hub
            .convert_coordination_to_cross_crate(SessionCoordinationEvent::ResponseReceived {
                dialog_id: dialog_id.clone(),
                response: response.clone(),
                transaction_id: transaction_id.clone(),
                request_uri: Some(request_uri.clone()),
            })
            .expect("valid challenge must map");
        assert!(matches!(
            mapped,
            RvoipCrossCrateEvent::DialogToSession(DialogToSessionEvent::AuthRequired {
                transaction_id: ref mapped_transaction,
                request_uri: ref mapped_uri,
                status_code: 401,
                ref method,
                ..
            }) if mapped_transaction == &transaction_id.to_string()
                && mapped_uri == &request_uri.to_string()
                && method == "INFO"
        ));

        let missing_uri = hub
            .convert_coordination_to_cross_crate(SessionCoordinationEvent::ResponseReceived {
                dialog_id,
                response,
                transaction_id: transaction_id.clone(),
                request_uri: None,
            })
            .expect("unsafe challenge must fail closed as terminal");
        assert!(matches!(
            missing_uri,
            RvoipCrossCrateEvent::DialogToSession(DialogToSessionEvent::OutboundRequestCompleted {
                outcome: OutboundRequestOutcome::FinalResponse { status_code: 401 },
                ..
            })
        ));

        let (proxy_hub, proxy_dialog_id) = test_hub().await;
        let mut proxy_response = Response::new(StatusCode::ProxyAuthenticationRequired);
        proxy_response
            .headers
            .push(TypedHeader::ProxyAuthenticate(ProxyAuthenticate::new(
                "proxy-realm",
                "proxy-nonce",
            )));
        let proxy_mapped = proxy_hub
            .convert_coordination_to_cross_crate(SessionCoordinationEvent::ResponseReceived {
                dialog_id: proxy_dialog_id,
                response: proxy_response,
                transaction_id: transaction_id.clone(),
                request_uri: Some(request_uri.clone()),
            })
            .expect("valid proxy challenge must map");
        assert!(matches!(
            proxy_mapped,
            RvoipCrossCrateEvent::DialogToSession(DialogToSessionEvent::AuthRequired {
                transaction_id: ref mapped_transaction,
                request_uri: ref mapped_uri,
                status_code: 407,
                ..
            }) if mapped_transaction == &transaction_id.to_string()
                && mapped_uri == &request_uri.to_string()
        ));

        let (bye_hub, bye_dialog_id) = test_hub().await;
        let bye_transaction = transaction(Method::Bye);
        let mut bye_response = Response::new(StatusCode::Unauthorized);
        bye_response
            .headers
            .push(TypedHeader::WwwAuthenticate(WwwAuthenticate::new(
                "bye-realm",
                "bye-nonce",
            )));
        assert!(matches!(
            bye_hub.convert_coordination_to_cross_crate(
                SessionCoordinationEvent::ResponseReceived {
                    dialog_id: bye_dialog_id,
                    response: bye_response,
                    transaction_id: bye_transaction.clone(),
                    request_uri: Some(request_uri),
                }
            ),
            Some(RvoipCrossCrateEvent::DialogToSession(
                DialogToSessionEvent::AuthRequired {
                    transaction_id: ref mapped_transaction,
                    ..
                }
            )) if mapped_transaction == &bye_transaction.to_string()
        ));
    }

    #[tokio::test]
    async fn timeout_and_transport_failure_map_once_without_termination_fallback() {
        let (hub, dialog_id) = test_hub().await;
        let transaction_id = transaction(Method::Refer);

        for outcome in [
            OutboundRequestOutcome::Timeout,
            OutboundRequestOutcome::TransportFailure,
        ] {
            let mapped = hub
                .convert_coordination_to_cross_crate(
                    SessionCoordinationEvent::OutboundRequestCompleted {
                        dialog_id: dialog_id.clone(),
                        transaction_id: transaction_id.clone(),
                        method: Method::Refer,
                        outcome,
                    },
                )
                .expect("terminal failure must map");
            assert!(matches!(
                mapped,
                RvoipCrossCrateEvent::DialogToSession(
                    DialogToSessionEvent::OutboundRequestCompleted {
                        transaction_id: ref mapped_transaction,
                        ref method,
                        outcome: mapped_outcome,
                        ..
                    }
                ) if mapped_transaction == &transaction_id.to_string()
                    && method == "REFER"
                    && mapped_outcome == outcome
            ));
        }

        let (coordination_tx, mut coordination_rx) = mpsc::channel(8);
        *hub.dialog_manager.session_coordinator.write().await = Some(coordination_tx);

        hub.dialog_manager
            .process_transaction_event(
                &transaction_id,
                &dialog_id,
                crate::transaction::TransactionEvent::TransactionTimeout {
                    transaction_id: transaction_id.clone(),
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            coordination_rx.recv().await,
            Some(SessionCoordinationEvent::OutboundRequestCompleted {
                transaction_id: ref emitted_transaction,
                outcome: OutboundRequestOutcome::Timeout,
                ..
            }) if emitted_transaction == &transaction_id
        ));

        hub.dialog_manager
            .process_transaction_event(
                &transaction_id,
                &dialog_id,
                crate::transaction::TransactionEvent::TransactionTerminated {
                    transaction_id: transaction_id.clone(),
                },
            )
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), coordination_rx.recv(),)
                .await
                .is_err()
        );

        let untracked_transaction = transaction(Method::Bye);
        hub.dialog_manager
            .process_transaction_event(
                &untracked_transaction,
                &dialog_id,
                crate::transaction::TransactionEvent::TransactionTimeout {
                    transaction_id: untracked_transaction.clone(),
                },
            )
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), coordination_rx.recv(),)
                .await
                .is_err()
        );

        let transport_transaction = TransactionKey::new(
            "z9hG4bK-exact-transport-test".to_string(),
            Method::Refer,
            false,
        );
        hub.dialog_manager
            .process_transaction_event(
                &transport_transaction,
                &dialog_id,
                crate::transaction::TransactionEvent::TransportError {
                    transaction_id: transport_transaction.clone(),
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            coordination_rx.recv().await,
            Some(SessionCoordinationEvent::OutboundRequestCompleted {
                transaction_id: ref emitted_transaction,
                outcome: OutboundRequestOutcome::TransportFailure,
                ..
            }) if emitted_transaction == &transport_transaction
        ));

        let generic_error_transaction = TransactionKey::new(
            "z9hG4bK-exact-generic-error-test".to_string(),
            Method::Update,
            false,
        );
        hub.dialog_manager
            .process_transaction_event(
                &generic_error_transaction,
                &dialog_id,
                crate::transaction::TransactionEvent::Error {
                    transaction_id: Some(generic_error_transaction.clone()),
                    error: "redacted test failure".to_string(),
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            coordination_rx.recv().await,
            Some(SessionCoordinationEvent::OutboundRequestCompleted {
                transaction_id: ref emitted_transaction,
                outcome: OutboundRequestOutcome::TransportFailure,
                ..
            }) if emitted_transaction == &generic_error_transaction
        ));
    }
}
