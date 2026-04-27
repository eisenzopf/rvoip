//! SIP Protocol Handlers Coordination for Dialog Management
//!
//! This module provides unified coordination of all SIP method handlers for the DialogManager.
//! It delegates to specialized protocol handler modules while maintaining a clean interface
//! for the core dialog management system.
//!
//! ## Architecture
//!
//! This module serves as a coordination layer that:
//! - Implements the main protocol handler traits by delegating to specialized modules
//! - Provides a unified interface for DialogManager to handle all SIP methods
//! - Maintains backwards compatibility with existing DialogManager API
//! - Reduces code duplication across protocol handlers

use std::net::SocketAddr;
use tracing::debug;

use super::core::DialogManager;
use super::session_coordination::SessionCoordinator;
use super::utils::SourceExtractor;
use crate::errors::{DialogError, DialogResult};
use crate::transaction::TransactionKey;
use rvoip_sip_core::types::header::{HeaderName, TypedHeader, TypedHeaderTrait};
use rvoip_sip_core::types::refer_to::ReferTo;
use rvoip_sip_core::{Method, Request, Response, StatusCode};

// Import all the specialized protocol handlers
use crate::protocol::{
    ByeHandler, InviteHandler, PrackHandler, RegisterHandler, ResponseHandler, UpdateHandler,
};

/// Trait for SIP method handling (main protocol coordination)
pub trait ProtocolHandlers {
    /// Handle INVITE requests (dialog-creating and re-INVITE)
    fn handle_invite_method(
        &self,
        request: Request,
        source: SocketAddr,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;

    /// Handle BYE requests (dialog-terminating)
    fn handle_bye_method(
        &self,
        request: Request,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;

    /// Handle CANCEL requests (transaction-cancelling)
    fn handle_cancel_method(
        &self,
        request: Request,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;

    /// Handle ACK requests (transaction-completing)
    fn handle_ack_method(
        &self,
        request: Request,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;

    /// Handle OPTIONS requests (capability discovery)
    fn handle_options_method(
        &self,
        request: Request,
        source: SocketAddr,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;

    /// Handle UPDATE requests (session modification)
    fn handle_update_method(
        &self,
        request: Request,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;

    /// Handle PRACK requests (RFC 3262 reliable-provisional acknowledgment)
    fn handle_prack_method(
        &self,
        request: Request,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;

    /// Handle responses to client transactions
    fn handle_response_message(
        &self,
        response: Response,
        transaction_id: TransactionKey,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
}

/// Trait for additional method handling
pub trait MethodHandler {
    /// Handle REGISTER requests (non-dialog)
    fn handle_register_method(
        &self,
        request: Request,
        source: SocketAddr,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;

    /// Handle INFO requests (mid-dialog)
    fn handle_info_method(
        &self,
        request: Request,
        source: SocketAddr,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;

    /// Handle REFER requests (call transfer)
    fn handle_refer_method(
        &self,
        request: Request,
        source: SocketAddr,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;

    /// Handle SUBSCRIBE requests (event subscription)
    fn handle_subscribe_method(
        &self,
        request: Request,
        source: SocketAddr,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;

    /// Handle NOTIFY requests (event notification)
    fn handle_notify_method(
        &self,
        request: Request,
        source: SocketAddr,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
}

/// Implementation of ProtocolHandlers for DialogManager using specialized modules
impl ProtocolHandlers for DialogManager {
    /// Delegate INVITE handling to the specialized invite_handler module
    async fn handle_invite_method(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        InviteHandler::handle_invite_method(self, request, source).await
    }

    /// Delegate BYE handling to the specialized bye_handler module
    async fn handle_bye_method(&self, request: Request) -> DialogResult<()> {
        ByeHandler::handle_bye_method(self, request).await
    }

    /// UAS-side CANCEL handler (RFC 3261 §9.2).
    ///
    /// An inbound CANCEL targets a pending *server* INVITE transaction. We
    /// must:
    /// 1. Respond 200 OK on the CANCEL transaction itself.
    /// 2. Respond 487 Request Terminated on the pending INVITE server
    ///    transaction.
    /// 3. Terminate the associated dialog and emit a `CallCancelled`
    ///    session-coordination event.
    ///
    /// If there is no matching server INVITE (or the INVITE is ours, i.e.
    /// a client-side transaction — a peer can't cancel one of our outgoing
    /// calls), we respond 481 Call/Transaction Does Not Exist.
    async fn handle_cancel_method(&self, request: Request) -> DialogResult<()> {
        debug!("Processing CANCEL request");

        // Always create a server transaction for the CANCEL first so we
        // have a handle to respond on (either 200 OK or 481).
        let source = SourceExtractor::extract_from_request(&request);
        let cancel_tx = self
            .transaction_manager
            .create_server_transaction(request.clone(), source)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to create server transaction for CANCEL: {}", e),
            })?;
        let cancel_tx_id = cancel_tx.id().clone();

        // Look up the matching INVITE server transaction.
        let invite_tx_id = self
            .transaction_manager
            .find_invite_server_transaction_for_cancel(&request)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to find INVITE server transaction for CANCEL: {}", e),
            })?;

        let Some(invite_tx_id) = invite_tx_id else {
            // No matching INVITE — 481.
            let response = crate::transaction::utils::response_builders::create_response(
                &request,
                StatusCode::CallOrTransactionDoesNotExist,
            );
            self.transaction_manager
                .send_response(&cancel_tx_id, response)
                .await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to send 481 response to CANCEL: {}", e),
                })?;
            debug!("CANCEL processed with 481 response (no matching INVITE)");
            return Ok(());
        };

        // 200 OK to the CANCEL transaction.
        let ok =
            crate::transaction::utils::response_builders::create_response(&request, StatusCode::Ok);
        self.transaction_manager
            .send_response(&cancel_tx_id, ok)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to send 200 OK to CANCEL: {}", e),
            })?;

        // 487 Request Terminated to the pending INVITE server transaction.
        // Fetch the original INVITE so `create_response` can copy its From,
        // To, Call-ID, CSeq, and Via headers.
        let original_invite = self
            .transaction_manager
            .get_server_transaction_request(&invite_tx_id)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to fetch pending INVITE for 487: {}", e),
            })?;
        let terminated = crate::transaction::utils::response_builders::create_response(
            &original_invite,
            StatusCode::RequestTerminated,
        );
        self.transaction_manager
            .send_response(&invite_tx_id, terminated)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to send 487 Request Terminated: {}", e),
            })?;

        // Terminate the dialog and notify the session layer.
        self.terminate_dialog_for_tx(&invite_tx_id, "CANCEL received")
            .await;

        debug!(
            "CANCEL processed for INVITE server transaction {} (200 CANCEL, 487 INVITE sent)",
            invite_tx_id
        );
        Ok(())
    }

    /// Handle ACK requests (related to INVITE processing)
    async fn handle_ack_method(&self, request: Request) -> DialogResult<()> {
        debug!("Processing ACK request");

        // ACK can be for 2xx response (goes to dialog) or non-2xx response (goes to transaction)
        if let Some(dialog_id) = self.find_dialog_for_request(&request).await {
            // Dialog-level ACK (for 2xx responses) - delegate to invite handler
            self.process_ack_in_dialog(request, dialog_id).await
        } else {
            // Transaction-level ACK (for non-2xx responses)
            // These are handled automatically by transaction-core
            debug!("ACK for non-2xx response - handled by transaction layer");
            Ok(())
        }
    }

    /// Handle OPTIONS requests with unified configuration support
    async fn handle_options_method(
        &self,
        request: Request,
        source: SocketAddr,
    ) -> DialogResult<()> {
        debug!("Processing OPTIONS request from {}", source);

        // Create server transaction
        let server_transaction = self
            .transaction_manager
            .create_server_transaction(request.clone(), source)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to create server transaction for OPTIONS: {}", e),
            })?;

        let transaction_id = server_transaction.id().clone();

        // **NEW**: Check unified configuration for auto-response behavior
        // If the manager is configured for auto-OPTIONS response, send immediate response
        // Otherwise, forward to session layer for application handling
        if self.should_auto_respond_to_options() {
            debug!("Auto-responding to OPTIONS request (configured for auto-response)");
            self.send_basic_options_response(&transaction_id, &request)
                .await?;
        } else {
            debug!("Forwarding OPTIONS request to session layer (auto-response disabled)");

            // Send session coordination event for capability query
            let event = crate::events::SessionCoordinationEvent::CapabilityQuery {
                transaction_id: transaction_id.clone(),
                request: request.clone(),
                source,
            };

            if !self.try_emit_session_coordination_event(event).await? {
                debug!("No session layer OPTIONS consumer; sending fallback response");
                self.send_basic_options_response(&transaction_id, &request)
                    .await?;
            }
        }

        debug!("OPTIONS request processed");
        Ok(())
    }

    /// Delegate UPDATE handling to the specialized update_handler module
    async fn handle_update_method(&self, request: Request) -> DialogResult<()> {
        UpdateHandler::handle_update_method(self, request).await
    }

    /// Delegate PRACK handling to the specialized prack_handler module
    async fn handle_prack_method(&self, request: Request) -> DialogResult<()> {
        PrackHandler::handle_prack_method(self, request).await
    }

    /// Delegate response handling to the specialized response_handler module
    async fn handle_response_message(
        &self,
        response: Response,
        transaction_id: TransactionKey,
    ) -> DialogResult<()> {
        ResponseHandler::handle_response_message(self, response, transaction_id).await
    }
}

/// Implementation of MethodHandler for DialogManager
impl MethodHandler for DialogManager {
    /// Delegate REGISTER handling to the specialized register_handler module
    async fn handle_register_method(
        &self,
        request: Request,
        source: SocketAddr,
    ) -> DialogResult<()> {
        RegisterHandler::handle_register_method(self, request, source).await
    }

    /// Handle INFO requests (simple forwarding to session layer)
    async fn handle_info_method(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        debug!("Processing INFO request from {}", source);

        if let Some(dialog_id) = self.find_dialog_for_request(&request).await {
            // Forward to session layer for application-specific handling
            let server_transaction = self
                .transaction_manager
                .create_server_transaction(request.clone(), source)
                .await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to create server transaction for INFO: {}", e),
                })?;

            let transaction_id = server_transaction.id().clone();

            let event = crate::events::SessionCoordinationEvent::ReInvite {
                dialog_id: dialog_id.clone(),
                transaction_id,
                request: request.clone(),
            };

            self.notify_session_layer(event).await?;
            debug!(
                "INFO request forwarded to session layer for dialog {}",
                dialog_id
            );
            Ok(())
        } else {
            // Send 481 Call/Transaction Does Not Exist
            let server_transaction = self
                .transaction_manager
                .create_server_transaction(request.clone(), source)
                .await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to create server transaction for INFO: {}", e),
                })?;

            let transaction_id = server_transaction.id().clone();
            let response = crate::transaction::utils::response_builders::create_response(
                &request,
                StatusCode::CallOrTransactionDoesNotExist,
            );

            self.transaction_manager
                .send_response(&transaction_id, response)
                .await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to send 481 response to INFO: {}", e),
                })?;

            debug!("INFO processed with 481 response (no dialog found)");
            Ok(())
        }
    }

    /// Handle REFER requests (call transfer)
    async fn handle_refer_method(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        debug!("Processing REFER request from {}", source);

        if let Some(dialog_id) = self.find_dialog_for_request(&request).await {
            // Create server transaction
            let server_transaction = self
                .transaction_manager
                .create_server_transaction(request.clone(), source)
                .await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to create server transaction for REFER: {}", e),
                })?;

            let transaction_id = server_transaction.id().clone();

            // Parse Refer-To header using sip-core's ReferTo type
            let refer_to = request
                .typed_header::<ReferTo>()
                .ok_or_else(|| DialogError::ProtocolError {
                    message: "Missing or invalid Refer-To header".to_string(),
                })?
                .clone();

            // Extract optional Referred-By header
            let referred_by = request
                .get_header_value(&rvoip_sip_core::HeaderName::ReferredBy)
                .map(|s| s.to_string());

            // Extract optional Replaces header (for attended transfer)
            // Note: Replaces is not a standard HeaderName in sip-core yet,
            // so we'll look for it as a raw header
            let replaces = request
                .all_headers()
                .iter()
                .find(|h| h.name().to_string().eq_ignore_ascii_case("replaces"))
                .map(|h| {
                    let header_str = h.to_string();
                    header_str.split(':').nth(1).map(|s| s.trim().to_string())
                })
                .flatten();

            // Forward to session layer FIRST - let session-core decide Accept/Reject
            // Session-core will send the appropriate response (202 Accepted or 4xx/5xx rejection)
            // via the transaction_id that we include in the event
            let event = crate::events::SessionCoordinationEvent::TransferRequest {
                dialog_id: dialog_id.clone(),
                transaction_id: transaction_id.clone(),
                refer_to,
                referred_by,
                replaces,
            };

            self.notify_session_layer(event).await?;
            debug!(
                "REFER request forwarded to session layer as TransferRequest for dialog {}",
                dialog_id
            );
            Ok(())
        } else {
            // REFER outside dialog - send 481
            let server_transaction = self
                .transaction_manager
                .create_server_transaction(request.clone(), source)
                .await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to create server transaction for REFER: {}", e),
                })?;

            let transaction_id = server_transaction.id().clone();
            let response = crate::transaction::utils::response_builders::create_response(
                &request,
                StatusCode::CallOrTransactionDoesNotExist,
            );

            self.transaction_manager
                .send_response(&transaction_id, response)
                .await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to send 481 response to REFER: {}", e),
                })?;

            debug!("REFER processed with 481 response (no dialog found)");
            Ok(())
        }
    }

    /// Handle SUBSCRIBE requests using SubscriptionManager
    async fn handle_subscribe_method(
        &self,
        request: Request,
        source: SocketAddr,
    ) -> DialogResult<()> {
        debug!("Processing SUBSCRIBE request from {}", source);

        // Use SubscriptionManager if available
        if let Some(ref subscription_manager) = self.subscription_manager {
            // Get local address - use configured or dialog manager's local address
            let local_addr = self.local_address;

            // Handle subscription with SubscriptionManager
            let (response, dialog_id) = subscription_manager
                .handle_subscribe(request.clone(), source, local_addr)
                .await?;

            // Create server transaction for the response
            let server_transaction = self
                .transaction_manager
                .create_server_transaction(request.clone(), source)
                .await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to create server transaction for SUBSCRIBE: {}", e),
                })?;

            let transaction_id = server_transaction.id().clone();

            // Send the response
            self.transaction_manager
                .send_response(&transaction_id, response)
                .await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to send SUBSCRIBE response: {}", e),
                })?;

            // If a dialog was created, store it
            if let Some(dialog_id) = dialog_id {
                debug!("SUBSCRIBE created subscription dialog {}", dialog_id);
                // Note: The actual dialog creation happens in SubscriptionManager
                // We might want to sync this with DialogManager's dialog store later
            }

            debug!("SUBSCRIBE request handled by SubscriptionManager");
            Ok(())
        } else {
            // Fallback to forwarding to session layer
            let server_transaction = self
                .transaction_manager
                .create_server_transaction(request.clone(), source)
                .await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to create server transaction for SUBSCRIBE: {}", e),
                })?;

            let transaction_id = server_transaction.id().clone();

            let event = crate::events::SessionCoordinationEvent::CapabilityQuery {
                transaction_id,
                request: request.clone(),
                source,
            };

            self.notify_session_layer(event).await?;
            debug!("SUBSCRIBE request forwarded to session layer");
            Ok(())
        }
    }

    /// Handle NOTIFY requests using SubscriptionManager
    async fn handle_notify_method(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        debug!("Processing NOTIFY request from {}", source);

        // Resolve the matching dialog up front — both the SubscriptionManager
        // path and the fallback need the id to route the downstream
        // cross-crate NotifyReceived event back to session-core.
        let dialog_id = self.find_dialog_for_request(&request).await;

        // Extract the fields session-core needs before we move `request` into
        // the server transaction. Values are raw strings so session-core
        // owns parsing (sipfrag / subscription-state / etc.).
        let notify_fields = extract_notify_fields(&request);

        // Use SubscriptionManager if available
        if let Some(ref subscription_manager) = self.subscription_manager {
            // Handle NOTIFY with SubscriptionManager
            let response = subscription_manager
                .handle_notify(request.clone(), source)
                .await?;

            // Create server transaction for the response
            let server_transaction = self
                .transaction_manager
                .create_server_transaction(request.clone(), source)
                .await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to create server transaction for NOTIFY: {}", e),
                })?;

            let transaction_id = server_transaction.id().clone();

            // Send the response (always 200 OK per RFC 6665)
            self.transaction_manager
                .send_response(&transaction_id, response)
                .await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to send NOTIFY response: {}", e),
                })?;

            debug!("NOTIFY request handled by SubscriptionManager");

            if let Some(dialog_id) = dialog_id {
                self.publish_notify_received(&dialog_id, notify_fields)
                    .await;
            }

            Ok(())
        } else if let Some(dialog_id) = dialog_id {
            // Fallback: NOTIFY in an existing dialog without SubscriptionManager.
            let server_transaction = self
                .transaction_manager
                .create_server_transaction(request.clone(), source)
                .await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to create server transaction for NOTIFY: {}", e),
                })?;

            let transaction_id = server_transaction.id().clone();

            // Reply 200 OK per RFC 6665 §4.1.3 — the session layer doesn't
            // need to approve acceptance of a NOTIFY in an established
            // subscription; it only wants the payload for its own tracking.
            let response = crate::transaction::utils::response_builders::create_response(
                &request,
                StatusCode::Ok,
            );
            self.transaction_manager
                .send_response(&transaction_id, response)
                .await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to send 200 OK to NOTIFY: {}", e),
                })?;

            debug!("NOTIFY accepted (fallback path) for dialog {}", dialog_id);
            self.publish_notify_received(&dialog_id, notify_fields)
                .await;
            Ok(())
        } else {
            // NOTIFY outside dialog - could be unsolicited, send 481
            let server_transaction = self
                .transaction_manager
                .create_server_transaction(request.clone(), source)
                .await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to create server transaction for NOTIFY: {}", e),
                })?;

            let transaction_id = server_transaction.id().clone();
            let response = crate::transaction::utils::response_builders::create_response(
                &request,
                StatusCode::CallOrTransactionDoesNotExist,
            );

            self.transaction_manager
                .send_response(&transaction_id, response)
                .await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to send 481 response to NOTIFY: {}", e),
                })?;

            debug!("NOTIFY processed with 481 response (no dialog found)");
            Ok(())
        }
    }
}

/// Raw NOTIFY fields extracted from the wire, ready to ship on
/// `DialogToSessionEvent::NotifyReceived`. Kept as raw strings so the
/// session layer owns any further parsing (`message/sipfrag` for REFER
/// progress reporting, typed Subscription-State, etc.).
struct NotifyFields {
    event_package: String,
    subscription_state: Option<String>,
    content_type: Option<String>,
    body: Option<String>,
}

fn extract_notify_fields(request: &Request) -> NotifyFields {
    let event_package = request
        .header(&HeaderName::Event)
        .and_then(|h| match h {
            TypedHeader::Event(e) => Some(e.event_type.to_string()),
            _ => None,
        })
        .unwrap_or_default();

    let subscription_state = request
        .header(&HeaderName::SubscriptionState)
        .and_then(|h| match h {
            TypedHeader::SubscriptionState(s) => Some(s.to_string()),
            _ => None,
        });

    let content_type = request
        .header(&HeaderName::ContentType)
        .and_then(|h| match h {
            TypedHeader::ContentType(ct) => Some(ct.to_string()),
            _ => None,
        });

    let body_bytes = request.body();
    let body = if body_bytes.is_empty() {
        None
    } else {
        Some(String::from_utf8_lossy(body_bytes).into_owned())
    };

    NotifyFields {
        event_package,
        subscription_state,
        content_type,
        body,
    }
}

impl DialogManager {
    async fn publish_notify_received(
        &self,
        dialog_id: &crate::dialog::dialog_id::DialogId,
        fields: NotifyFields,
    ) {
        let Some(hub) = self.event_hub.read().await.as_ref().cloned() else {
            debug!(
                "No event hub wired; dropping NOTIFY surface for dialog {}",
                dialog_id
            );
            return;
        };
        let Some(session_id) = self.get_session_id(dialog_id) else {
            debug!(
                "No session mapping for dialog {}; dropping NOTIFY surface",
                dialog_id
            );
            return;
        };

        let event = rvoip_infra_common::events::cross_crate::RvoipCrossCrateEvent::DialogToSession(
            rvoip_infra_common::events::cross_crate::DialogToSessionEvent::NotifyReceived {
                session_id,
                event_package: fields.event_package,
                subscription_state: fields.subscription_state,
                content_type: fields.content_type,
                body: fields.body,
            },
        );

        if let Err(e) = hub.publish_cross_crate_event(event).await {
            tracing::warn!("Failed to publish NotifyReceived event: {}", e);
        }
    }
}

/// Helper methods for protocol coordination
impl DialogManager {
    /// Send basic OPTIONS response with supported methods
    async fn send_basic_options_response(
        &self,
        transaction_id: &TransactionKey,
        request: &Request,
    ) -> DialogResult<()> {
        // Use transaction-core helper for OPTIONS response with Allow header
        let allowed_methods = vec![
            Method::Invite,
            Method::Bye,
            Method::Cancel,
            Method::Ack,
            Method::Options,
            Method::Update,
            Method::Info,
            Method::Refer,
        ];

        let response = crate::transaction::utils::response_builders::create_ok_response_for_options(
            request,
            &allowed_methods,
        );

        self.transaction_manager
            .send_response(transaction_id, response)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to send OPTIONS response: {}", e),
            })?;

        debug!("Sent basic OPTIONS response");
        Ok(())
    }
}
