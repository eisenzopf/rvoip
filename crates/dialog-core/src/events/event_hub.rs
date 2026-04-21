//! Dialog Event Hub for Global Event Coordination
//!
//! This module provides the central event hub that integrates dialog-core with the global
//! event coordinator from infra-common, replacing channel-based communication.

use std::sync::Arc;
use anyhow::Result;
use async_trait::async_trait;
use tracing::{debug, info, warn, error};

use rvoip_infra_common::events::coordinator::{GlobalEventCoordinator, CrossCrateEventHandler};
use rvoip_infra_common::events::cross_crate::{
    CrossCrateEvent, RvoipCrossCrateEvent, DialogToSessionEvent, SessionToDialogEvent,
    DialogToTransportEvent, TransportToDialogEvent, CallState, TerminationReason
};
use rvoip_sip_core::{StatusCode, Request, Method};
use crate::transaction::TransactionKey;

use crate::events::{DialogEvent, SessionCoordinationEvent};
use crate::dialog::{DialogId, DialogState};
use crate::errors::DialogError;
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
        debug!("Publishing dialog event: {:?}", event);
        
        // Convert to cross-crate event if applicable
        if let Some(cross_crate_event) = self.convert_dialog_to_cross_crate(event) {
            self.global_coordinator.publish(Arc::new(cross_crate_event)).await?;
        }
        
        Ok(())
    }
    
    /// Publish a session coordination event to the global bus
    pub async fn publish_session_coordination_event(&self, event: SessionCoordinationEvent) -> Result<()> {
        debug!("Publishing session coordination event: {:?}", event);

        // Convert to cross-crate event
        if let Some(cross_crate_event) = self.convert_coordination_to_cross_crate(event.clone()) {
            info!("🚀 [event_hub] About to publish cross-crate event for event: {:?}", event);
            self.global_coordinator.publish(Arc::new(cross_crate_event)).await?;
            info!("✅ [event_hub] Published cross-crate event successfully");
        } else {
            info!("⚠️ [event_hub] convert_coordination_to_cross_crate returned None for: {:?}", event);
        }

        Ok(())
    }
    
    /// Publish a cross-crate event directly
    pub async fn publish_cross_crate_event(&self, event: RvoipCrossCrateEvent) -> Result<()> {
        debug!("Publishing cross-crate event directly: {:?}", event);
        self.global_coordinator.publish(Arc::new(event)).await?;
        Ok(())
    }
    
    /// Convert DialogEvent to cross-crate event
    fn convert_dialog_to_cross_crate(&self, event: DialogEvent) -> Option<RvoipCrossCrateEvent> {
        match event {
            DialogEvent::StateChanged { dialog_id, old_state, new_state } => {
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
                        }
                    ))
                } else {
                    warn!("No session ID found for dialog {:?}", dialog_id);
                    None
                }
            }
            
            DialogEvent::Terminated { dialog_id, reason } => {
                if let Some(session_id) = self.dialog_manager.get_session_id(&dialog_id) {
                    Some(RvoipCrossCrateEvent::DialogToSession(
                        DialogToSessionEvent::CallTerminated {
                            session_id,
                            reason: TerminationReason::RemoteHangup,
                        }
                    ))
                } else {
                    None
                }
            }
            
            _ => None, // Other events are internal only
        }
    }
    
    /// Convert SessionCoordinationEvent to cross-crate event
    fn convert_coordination_to_cross_crate(&self, event: SessionCoordinationEvent) -> Option<RvoipCrossCrateEvent> {
        match event {
            SessionCoordinationEvent::IncomingCall { dialog_id, transaction_id, request, source } => {
                let call_id = request.call_id()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| format!("unknown-{}", uuid::Uuid::new_v4()));
                
                let from = request.from()
                    .map(|f| f.to_string())
                    .unwrap_or_else(|| "anonymous".to_string());
                    
                let to = request.to()
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
                self.dialog_manager.store_dialog_mapping(&session_id, dialog_id.clone(), transaction_id.clone(), request.clone(), source);
                
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
                    }
                ))
            }
            
            SessionCoordinationEvent::CallAnswered { dialog_id, session_answer } => {
                info!("🔍 [event_hub] Processing CallAnswered for dialog: {}", dialog_id);
                match self.dialog_manager.get_session_id(&dialog_id) {
                    Some(session_id) => {
                        info!("✅ [event_hub] Found session ID {} for dialog {}", session_id, dialog_id);
                        Some(RvoipCrossCrateEvent::DialogToSession(
                            DialogToSessionEvent::CallEstablished {
                                session_id,
                                sdp_answer: Some(session_answer),
                            }
                        ))
                    }
                    None => {
                        warn!("❌ [event_hub] No session ID found for dialog {} in CallAnswered event", dialog_id);
                        None
                    }
                }
            }
            
            SessionCoordinationEvent::CallTerminating { dialog_id, reason } => {
                if let Some(session_id) = self.dialog_manager.get_session_id(&dialog_id) {
                    Some(RvoipCrossCrateEvent::DialogToSession(
                        DialogToSessionEvent::CallTerminated {
                            session_id,
                            reason: TerminationReason::RemoteHangup,
                        }
                    ))
                } else {
                    None
                }
            }

            SessionCoordinationEvent::SessionRefreshed { dialog_id, expires_secs } => {
                self.dialog_manager.get_session_id(&dialog_id).map(|session_id| {
                    RvoipCrossCrateEvent::DialogToSession(
                        DialogToSessionEvent::SessionRefreshed { session_id, expires_secs },
                    )
                })
            }

            SessionCoordinationEvent::SessionRefreshFailed { dialog_id, reason } => {
                self.dialog_manager.get_session_id(&dialog_id).map(|session_id| {
                    RvoipCrossCrateEvent::DialogToSession(
                        DialogToSessionEvent::SessionRefreshFailed { session_id, reason },
                    )
                })
            }

            SessionCoordinationEvent::ResponseReceived { dialog_id, response, .. } => {
                // Try to get session ID from stored mapping first
                if let Some(session_id) = self.dialog_manager.get_session_id(&dialog_id) {
                    // Handle specific response codes
                    match response.status_code() {
                        200 => {
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
                                }
                            ))
                        }
                        180 | 183 => {
                            // 180 Ringing / 183 Session Progress (early media)
                            Some(RvoipCrossCrateEvent::DialogToSession(
                                DialogToSessionEvent::CallStateChanged {
                                    session_id,
                                    new_state: CallState::Ringing,
                                    reason: None,
                                }
                            ))
                        }
                        181 | 182 | 199 => {
                            // Less common provisional responses (RFC 3261 §21.1):
                            //   181 Call Is Being Forwarded
                            //   182 Queued
                            //   199 Early Dialog Terminated
                            // All three keep the dialog in a Ringing-like state
                            // from the session layer's POV; attach the reason
                            // phrase so the application can distinguish them.
                            let reason = match response.status_code() {
                                181 => Some("Forwarded".to_string()),
                                182 => Some("Queued".to_string()),
                                199 => Some("EarlyDialogTerminated".to_string()),
                                _ => None,
                            };
                            Some(RvoipCrossCrateEvent::DialogToSession(
                                DialogToSessionEvent::CallStateChanged {
                                    session_id,
                                    new_state: CallState::Ringing,
                                    reason,
                                }
                            ))
                        }
                        487 => {
                            // RFC 3261 §15.1.2 — 487 Request Terminated follows a
                            // CANCEL. Distinct from generic CallFailed so UIs can
                            // render "missed call" rather than "call rejected".
                            Some(RvoipCrossCrateEvent::DialogToSession(
                                DialogToSessionEvent::CallCancelled { session_id }
                            ))
                        }
                        491 => {
                            // RFC 3261 §14.1 — 491 Request Pending on a re-INVITE
                            // or UPDATE. Session layer should wait a random
                            // backoff and retry. 491 on the initial INVITE is
                            // nonsensical per the spec so we surface it as glare
                            // unconditionally; higher layers decide how to handle.
                            Some(RvoipCrossCrateEvent::DialogToSession(
                                DialogToSessionEvent::ReinviteGlare { session_id }
                            ))
                        }
                        422 => {
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
                                    },
                                ))
                            }
                        }
                        code if (300..400).contains(&code) => {
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
                                                if let rvoip_sip_core::types::param::Param::Q(v) = p {
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
                                }
                            ))
                        }
                        401 | 407 => {
                            // RFC 3261 §22.2 — digest auth challenge. If the
                            // response carries a WWW-Authenticate (401) or
                            // Proxy-Authenticate (407) header, surface it as
                            // `AuthRequired` so session-core can compute a
                            // digest response and retry. Method-agnostic:
                            // this path fires for INVITE, REGISTER, and any
                            // future auth-challenged request. Malformed 401/
                            // 407 without a parseable challenge falls through
                            // to CallFailed below.
                            use rvoip_sip_core::types::headers::HeaderAccess;
                            let header_name = if response.status_code() == 407 {
                                rvoip_sip_core::types::header::HeaderName::ProxyAuthenticate
                            } else {
                                rvoip_sip_core::types::header::HeaderName::WwwAuthenticate
                            };
                            if let Some(challenge) = response.raw_header_value(&header_name) {
                                let realm = extract_digest_realm(&challenge);
                                Some(RvoipCrossCrateEvent::DialogToSession(
                                    DialogToSessionEvent::AuthRequired {
                                        session_id,
                                        status_code: response.status_code(),
                                        challenge,
                                        realm,
                                    }
                                ))
                            } else {
                                Some(RvoipCrossCrateEvent::DialogToSession(
                                    DialogToSessionEvent::CallFailed {
                                        session_id,
                                        status_code: response.status_code(),
                                        reason_phrase: response.reason_phrase().to_string(),
                                    }
                                ))
                            }
                        }
                        code if (400..700).contains(&code) => {
                            // RFC 3261 §8.1.3 — any other final 3xx/4xx/5xx/6xx
                            // response ends the UAC's INVITE transaction. Propagate
                            // to the session layer so it can emit CallFailed and
                            // run the Dialog{4,5,6}xxFailure state transitions.
                            Some(RvoipCrossCrateEvent::DialogToSession(
                                DialogToSessionEvent::CallFailed {
                                    session_id,
                                    status_code: code,
                                    reason_phrase: response.reason_phrase().to_string(),
                                }
                            ))
                        }
                        _ => None, // Other 1xx provisional responses stay unmapped
                    }
                } else {
                    warn!("No session ID found for dialog {:?}", dialog_id);
                    None
                }
            }
            
            SessionCoordinationEvent::TransferRequest { dialog_id, transaction_id, refer_to, replaces, .. } => {
                if let Some(session_id) = self.dialog_manager.get_session_id(&dialog_id) {
                    // Convert ReferTo to string
                    let refer_to_uri = refer_to.uri().to_string();

                    // Determine transfer type based on Replaces header
                    let transfer_type = if replaces.is_some() {
                        rvoip_infra_common::events::cross_crate::TransferType::Attended
                    } else {
                        rvoip_infra_common::events::cross_crate::TransferType::Blind
                    };

                    Some(RvoipCrossCrateEvent::DialogToSession(
                        DialogToSessionEvent::TransferRequested {
                            session_id,
                            refer_to: refer_to_uri,
                            transfer_type,
                            transaction_id: transaction_id.to_string(),
                        }
                    ))
                } else {
                    warn!("No session ID found for dialog {:?} in TransferRequest", dialog_id);
                    None
                }
            }

            // ACK events for state machine transitions
            SessionCoordinationEvent::AckSent { dialog_id, .. } => {
                // AckSent is primarily for UAC - session layer may need to know ACK was sent
                // but typically this isn't needed for state transitions
                // We'll pass it through in case session-core-v2 wants to track it
                debug!("AckSent event for dialog {}, converting to cross-crate format", dialog_id);
                None // For now, UAC doesn't need this event
            }

            SessionCoordinationEvent::AckReceived { dialog_id, negotiated_sdp, .. } => {
                // AckReceived is critical for UAS - dialog-core received ACK, now session must transition
                if let Some(session_id) = self.dialog_manager.get_session_id(&dialog_id) {
                    info!("Converting AckReceived to cross-crate event for session {}", session_id);
                    Some(RvoipCrossCrateEvent::DialogToSession(
                        DialogToSessionEvent::AckReceived {
                            session_id,
                            sdp: negotiated_sdp,
                        }
                    ))
                } else {
                    warn!("No session ID found for dialog {:?} in AckReceived", dialog_id);
                    None
                }
            }

            SessionCoordinationEvent::CallTerminating { dialog_id, reason } => {
                // When BYE completes, notify session-core that dialog is terminating
                if let Some(session_id) = self.dialog_manager.get_session_id(&dialog_id) {
                    info!("Converting CallTerminating to CallTerminated for session {}", session_id);
                    Some(RvoipCrossCrateEvent::DialogToSession(
                        DialogToSessionEvent::CallTerminated {
                            session_id,
                            reason: rvoip_infra_common::events::cross_crate::TerminationReason::RemoteHangup,
                        }
                    ))
                } else {
                    warn!("No session ID found for dialog {:?} in CallTerminating", dialog_id);
                    None
                }
            }

            // 180 Ringing reached the UAC. Surface it to session-core as a
            // `CallStateChanged { Ringing }` so the state machine can
            // transition Initiating → Ringing (which is what the
            // `UAC/Ringing/HangupCall → CANCEL` path and the public
            // ringback signals rely on).
            SessionCoordinationEvent::CallRinging { dialog_id } => {
                self.dialog_manager.get_session_id(&dialog_id).map(|session_id| {
                    RvoipCrossCrateEvent::DialogToSession(
                        DialogToSessionEvent::CallStateChanged {
                            session_id,
                            new_state: CallState::Ringing,
                            reason: Some("180 Ringing".to_string()),
                        },
                    )
                })
            }

            // DTMF events would be handled separately if implemented
            // SessionCoordinationEvent doesn't have DtmfReceived yet

            // Mid-dialog INVITE (re-INVITE) or UPDATE. Session-core drives
            // the UAS-side response (200 OK for normal, 491 Request Pending
            // on glare) through its state machine. INFO and NOTIFY are also
            // emitted via this variant today — we deliberately skip them
            // here so they do not get misrouted to the re-INVITE handler.
            SessionCoordinationEvent::ReInvite { dialog_id, request, .. } => {
                let method = request.method();
                if !matches!(method, Method::Invite | Method::Update) {
                    debug!(
                        "Skipping ReInvite cross-crate conversion for method {:?} (dialog {})",
                        method, dialog_id
                    );
                    return None;
                }
                if let Some(session_id) = self.dialog_manager.get_session_id(&dialog_id) {
                    let sdp = if !request.body().is_empty() {
                        String::from_utf8(request.body().to_vec()).ok()
                    } else {
                        None
                    };
                    info!(
                        "Converting ReInvite ({}) to cross-crate event for session {}",
                        method, session_id
                    );
                    Some(RvoipCrossCrateEvent::DialogToSession(
                        DialogToSessionEvent::ReinviteReceived {
                            session_id,
                            sdp,
                            method: method.to_string(),
                        },
                    ))
                } else {
                    warn!("No session ID found for dialog {:?} in ReInvite", dialog_id);
                    None
                }
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
                        } => {
                            info!("📩 Handling SendRegisterResponse via trait: {} {}", status_code, reason);
                            self.handle_register_response(
                                transaction_id,
                                *status_code,
                                reason,
                                www_authenticate.as_deref(),
                                contact.as_deref(),
                                *expires,
                            ).await?;
                            return Ok(()); // Early return after handling
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        
        // Fallback to string parsing for other events (legacy support)
        let event_str = format!("{:?}", event);
        
        match event.event_type() {
            "session_to_dialog" => {
                debug!("Processing session-to-dialog event: {}", event_str);
                
                // Handle ReferResponse event
                if event_str.contains("ReferResponse") {
                    self.handle_refer_response(&event_str).await?;
                }
                // Handle StoreDialogMapping event
                else if event_str.contains("StoreDialogMapping") {
                    // Extract session_id and dialog_id
                    if let Some(session_id_start) = event_str.find("session_id: \"") {
                        let session_id_content_start = session_id_start + 13;
                        if let Some(session_id_end) = event_str[session_id_content_start..].find("\"") {
                            let session_id = event_str[session_id_content_start..session_id_content_start + session_id_end].to_string();
                            
                            if let Some(dialog_id_start) = event_str.find("dialog_id: \"") {
                                let dialog_id_content_start = dialog_id_start + 12;
                                if let Some(dialog_id_end) = event_str[dialog_id_content_start..].find("\"") {
                                    let dialog_id = event_str[dialog_id_content_start..dialog_id_content_start + dialog_id_end].to_string();
                                    
                                    info!("Storing dialog mapping: session {} -> dialog {}", session_id, dialog_id);
                                    
                                    // Parse dialog ID from UUID string
                                    if let Ok(uuid) = dialog_id.parse::<uuid::Uuid>() {
                                        let parsed_dialog_id = DialogId(uuid);
                                        // Store the bidirectional mapping
                                        self.dialog_manager.session_to_dialog.insert(session_id.clone(), parsed_dialog_id.clone());
                                        self.dialog_manager.dialog_to_session.insert(parsed_dialog_id, session_id.clone());
                                        
                                        info!("Successfully stored dialog mapping for session {}", session_id);
                                    } else {
                                        warn!("Failed to parse dialog ID: {}", dialog_id);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            
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
        debug!("Handling SendRegisterResponse: transaction={}, status={} {}", 
               transaction_id, status_code, reason);
        
        // Parse transaction_id to TransactionKey
        let tx_key = transaction_id.parse::<TransactionKey>()
            .map_err(|e| anyhow::anyhow!("Failed to parse transaction_id: {}", e))?;
        
        // Check if this transaction exists in our dialog manager
        // This prevents multiple DialogEventHubs from trying to handle the same event
        if self.dialog_manager.transaction_manager().original_request(&tx_key).await.is_err() {
            debug!("Transaction {} not found in this DialogManager - skipping", transaction_id);
            return Ok(()); // Not our transaction, skip silently
        }
        
        // Call the dialog manager's send_register_response method
        self.dialog_manager.send_register_response(
            &tx_key,
            status_code,
            reason,
            www_authenticate,
            contact,
            expires,
        ).await
        .map_err(|e| anyhow::anyhow!("Failed to send REGISTER response: {}", e))?;
        
        info!("✅ Sent REGISTER response: {} {}", status_code, reason);
        Ok(())
    }
    
    /// Handle ReferResponse event from session-core
    async fn handle_refer_response(&self, event_str: &str) -> Result<()> {
        // Extract transaction_id, accept flag, status_code, and reason
        let transaction_id = self.extract_field(event_str, "transaction_id: \"")
            .ok_or_else(|| anyhow::anyhow!("Missing transaction_id in ReferResponse"))?;
        
        let accept = event_str.contains("accept: true");
        
        let status_code = self.extract_field(event_str, "status_code: ")
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(if accept { 202 } else { 603 });
            
        let reason = self.extract_field(event_str, "reason: \"")
            .unwrap_or_else(|| if accept { "Accepted".to_string() } else { "Decline".to_string() });
        
        info!("Handling ReferResponse: transaction={}, accept={}, status={} {}", 
              transaction_id, accept, status_code, reason);
        
        // Parse transaction_id and send response
        if let Ok(tx_key) = transaction_id.parse::<crate::transaction::TransactionKey>() {
            use rvoip_sip_core::StatusCode;
            let status = StatusCode::from_u16(status_code).unwrap_or(StatusCode::Accepted);
            
            // Get original REFER request to build proper response
            match self.dialog_manager.transaction_manager().original_request(&tx_key).await {
                Ok(Some(original_request)) => {
                    // Build proper response using the original request
                    let response = crate::transaction::utils::response_builders::create_response(
                        &original_request, 
                        status
                    );
                    
                    if let Err(e) = self.dialog_manager.send_response(&tx_key, response).await {
                        error!("Failed to send REFER response: {}", e);
                    } else {
                        info!("Successfully sent REFER response: {} {}", status_code, reason);
                    }
                }
                Ok(None) => {
                    error!("No original request found for transaction: {}", transaction_id);
                }
                Err(e) => {
                    error!("Failed to get original request for transaction {}: {}", transaction_id, e);
                }
            }
        } else {
            error!("Failed to parse transaction_id: {}", transaction_id);
        }
        
        Ok(())
    }
    
    /// Extract field value from event debug string
    fn extract_field(&self, event_str: &str, field_prefix: &str) -> Option<String> {
        if let Some(start) = event_str.find(field_prefix) {
            let start = start + field_prefix.len();
            if field_prefix.ends_with("\"") {
                // String field - find closing quote
                if let Some(end) = event_str[start..].find('"') {
                    return Some(event_str[start..start+end].to_string());
                }
            } else {
                // Numeric field - find next space or comma
                let end = event_str[start..].find(|c: char| c.is_whitespace() || c == ',')
                    .unwrap_or(event_str[start..].len());
                return Some(event_str[start..start+end].to_string());
            }
        }
        None
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
