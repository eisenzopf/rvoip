//! INVITE Request Handler for Dialog-Core
//!
//! This module provides specialized handling for INVITE requests according to RFC 3261.
//! INVITE is the most complex SIP method as it can create dialogs, modify sessions,
//! and requires careful state management.
//!
//! ## INVITE Types Handled
//!
//! - **Initial INVITE**: Creates new dialogs and establishes sessions
//! - **Re-INVITE**: Modifies existing sessions within established dialogs
//! - **Refresh INVITE**: Refreshes session timers and state
//!
//! ## Dialog Creation Process
//!
//! 1. Parse INVITE request and validate headers
//! 2. Create early dialog upon receiving INVITE
//! 3. Send provisional responses (100 Trying, 180 Ringing)
//! 4. Confirm dialog with 2xx response
//! 5. Complete with ACK reception

use std::net::SocketAddr;
use tracing::{debug, info, warn};

use crate::api::config::RelUsage;
use crate::diagnostics::safe_log::SafeTransactionKey;
use crate::dialog::DialogId;
use crate::errors::{DialogError, DialogResult};
use crate::events::SessionCoordinationEvent;
use crate::manager::transaction_integration::detect_peer_100rel_support;
use crate::manager::{DialogLookup, DialogManager};
use crate::transaction::utils::response_builders;
use crate::transaction::TransactionKey;
use rvoip_sip_core::types::min_se::MinSE;
use rvoip_sip_core::types::unsupported::Unsupported;
use rvoip_sip_core::types::TypedHeader;
use rvoip_sip_core::{HeaderName, Request, StatusCode};

/// INVITE-specific handling operations
pub trait InviteHandler {
    /// Handle INVITE requests (dialog-creating and re-INVITE)
    fn handle_invite_method(
        &self,
        request: Request,
        source: SocketAddr,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InviteCausalOwner {
    AcknowledgedSession,
    DirectApi,
}

/// Implementation of INVITE handling for DialogManager
impl InviteHandler for DialogManager {
    /// Handle INVITE requests according to RFC 3261 Section 14
    ///
    /// Supports both initial INVITE (dialog-creating) and re-INVITE (session modification).
    async fn handle_invite_method(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        self.handle_invite_for_owner(request, source, InviteCausalOwner::AcknowledgedSession)
            .await
            .map(|_| ())
    }
}

/// INVITE-specific helper methods for DialogManager
impl DialogManager {
    async fn handle_invite_for_owner(
        &self,
        request: Request,
        source: SocketAddr,
        owner: InviteCausalOwner,
    ) -> DialogResult<Option<DialogId>> {
        debug!("Processing INVITE request from {}", source);

        // Create server transaction using transaction-core
        let server_transaction = self
            .transaction_manager
            .create_server_transaction(request.clone(), source)
            .await
            .map_err(|_error| DialogError::TransactionError {
                message: "Failed to create server transaction for INVITE".to_string(),
            })?;

        let transaction_id = server_transaction.id().clone();
        let dialog_setup_lock = self
            .transaction_manager
            .server_invite_dialog_setup_lock(&transaction_id)
            .ok_or_else(|| DialogError::TransactionError {
                message: "INVITE server transaction had no dialog setup owner".to_string(),
            })?;
        let _dialog_setup_guard = dialog_setup_lock.lock().await;

        // An exact server transaction may be returned again for a duplicate
        // manual delivery or transport retransmission. Its existing dialog
        // binding is authoritative; do not re-run dialog matching, sequence
        // mutation, causal delivery, or response selection for that wire
        // generation.
        if let Ok(dialog_id) = self.find_dialog_for_transaction(&transaction_id) {
            return Ok(Some(dialog_id));
        }

        // Check if this is an initial INVITE or re-INVITE
        if let Some(dialog_id) = self.find_dialog_for_request(&request).await {
            // This is a re-INVITE within existing dialog
            self.handle_reinvite_for_owner(transaction_id, request, dialog_id, owner)
                .await
        } else {
            if request.to().and_then(|to| to.tag()).is_some() {
                let response = response_builders::create_response(
                    &request,
                    StatusCode::CallOrTransactionDoesNotExist,
                );
                self.send_unowned_final_response_classified(&transaction_id, response)
                    .await?;
                return Ok(None);
            }
            // This is an initial INVITE — `handle_initial_invite` does the
            // 100rel policy check and may short-circuit with 420 before
            // creating a dialog.
            self.handle_initial_invite_for_owner(transaction_id, request, source, owner)
                .await
        }
    }

    /// Process a manually supplied INVITE with the API caller as its direct
    /// causal owner. This reuses the protocol ingress mechanics and returns
    /// the exact dialog created or matched by the exact server transaction;
    /// it never manufactures a session event or an EventHub fallback.
    pub(crate) async fn handle_direct_invite(
        &self,
        request: Request,
        source: SocketAddr,
    ) -> DialogResult<DialogId> {
        self.handle_invite_for_owner(request, source, InviteCausalOwner::DirectApi)
            .await?
            .ok_or_else(|| {
                DialogError::routing_error(
                    "manually supplied INVITE did not produce an owned dialog",
                )
            })
    }

    /// Handle initial INVITE (dialog-creating)
    pub async fn handle_initial_invite(
        &self,
        transaction_id: TransactionKey,
        request: Request,
        source: SocketAddr,
    ) -> DialogResult<()> {
        let dialog_setup_lock = self
            .transaction_manager
            .server_invite_dialog_setup_lock(&transaction_id)
            .ok_or_else(|| DialogError::TransactionError {
                message: "INVITE server transaction had no dialog setup owner".to_string(),
            })?;
        let _dialog_setup_guard = dialog_setup_lock.lock().await;
        if self.find_dialog_for_transaction(&transaction_id).is_ok() {
            return Ok(());
        }
        self.handle_initial_invite_for_owner(
            transaction_id,
            request,
            source,
            InviteCausalOwner::AcknowledgedSession,
        )
        .await
        .map(|_| ())
    }

    async fn handle_initial_invite_for_owner(
        &self,
        transaction_id: TransactionKey,
        request: Request,
        source: SocketAddr,
        owner: InviteCausalOwner,
    ) -> DialogResult<Option<DialogId>> {
        let setup_started =
            crate::diagnostics::dialog_timing_enabled().then(std::time::Instant::now);
        tracing::debug!(
            "🔍 INVITE HANDLER: Processing initial INVITE request from {}",
            source
        );
        debug!("Processing initial INVITE request");

        // RFC 3262 §4: enforce our 100rel policy before creating a dialog.
        // A mismatch short-circuits with 420 Bad Extension + `Unsupported:
        // 100rel`. Performed here (not in `handle_invite_method`) because
        // the transaction-event path at `manager/core.rs` bypasses that
        // entry point and calls `handle_initial_invite` directly.
        let (peer_supports_100rel, peer_requires_100rel) = detect_peer_100rel_support(&request);
        let our_policy = self.config_100rel_policy();
        let policy_mismatch = matches!(
            (our_policy, peer_supports_100rel, peer_requires_100rel),
            (RelUsage::NotSupported, _, true) | (RelUsage::Required, false, _)
        );
        if policy_mismatch {
            warn!(
                "100rel policy mismatch on initial INVITE (our={:?}, peer_supports={}, peer_requires={}) — rejecting with 420",
                our_policy, peer_supports_100rel, peer_requires_100rel
            );
            let mut response =
                response_builders::create_response(&request, StatusCode::BadExtension);
            response
                .headers
                .push(TypedHeader::Unsupported(Unsupported::with_tags(vec![
                    "100rel".to_string(),
                ])));
            self.send_unowned_final_response_classified(&transaction_id, response)
                .await?;
            return Ok(None);
        }

        // RFC 4028 §6: If the peer's `Min-SE:` exceeds our configured
        // `session_timer_secs`, respond 422 Session Interval Too Small with
        // our own Min-SE so the peer can retry with a valid value. Only
        // enforced when session timers are enabled on our side.
        if let Some((our_session_secs, our_min_se)) = self.config_session_timer_settings() {
            let peer_min_se = request.headers.iter().find_map(|h| {
                if let TypedHeader::MinSE(min) = h {
                    Some(min.delta_seconds)
                } else {
                    None
                }
            });
            if let Some(peer_min) = peer_min_se {
                if peer_min > our_session_secs {
                    warn!(
                        "Peer's Min-SE {} exceeds our Session-Expires {} — rejecting with 422",
                        peer_min, our_session_secs
                    );
                    let mut response = response_builders::create_response(
                        &request,
                        StatusCode::SessionIntervalTooSmall,
                    );
                    response
                        .headers
                        .push(TypedHeader::MinSE(MinSE::new(our_min_se)));
                    self.send_unowned_final_response_classified(&transaction_id, response)
                        .await?;
                    return Ok(None);
                }
            }
        }

        // Create early dialog
        let dialog_id = self.create_early_dialog_from_invite(&request).await?;
        tracing::debug!("🔍 INVITE HANDLER: Created early dialog {}", dialog_id);

        // Capture INVITE CSeq + peer 100rel support on the dialog so the UAS
        // can emit reliable 18x and the PRACK handler can validate `RAck`.
        let invite_cseq = request.header(&HeaderName::CSeq).and_then(|h| {
            if let TypedHeader::CSeq(cseq) = h {
                Some(cseq.sequence())
            } else {
                None
            }
        });
        // RFC 4028: parse peer's Session-Expires and compute negotiated
        // interval. UAS is refresher when peer's Session-Expires had
        // `refresher=uas` or when peer didn't name a refresher and our
        // config has session timers enabled.
        let peer_session_expires = request.headers.iter().find_map(|h| {
            if let TypedHeader::SessionExpires(se) = h {
                Some(se.clone())
            } else {
                None
            }
        });
        let (negotiated_session_secs, uas_is_refresher) =
            match (peer_session_expires, self.config_session_timer_settings()) {
                (Some(peer_se), Some((our_secs, _))) => {
                    let secs = peer_se.delta_seconds.min(our_secs);
                    let uas = matches!(
                        peer_se.refresher,
                        Some(rvoip_sip_core::types::session_expires::Refresher::Uas)
                    );
                    (Some(secs), uas)
                }
                (None, Some((our_secs, _))) => (Some(our_secs), true),
                _ => (None, false),
            };

        if let Ok(mut dialog) = self.get_dialog_mut(&dialog_id) {
            dialog.peer_supports_100rel = peer_supports_100rel;
            dialog.invite_cseq = invite_cseq;
            dialog.session_expires_secs = negotiated_session_secs;
            dialog.is_session_refresher = uas_is_refresher;
        }

        // Associate transaction with dialog
        self.associate_transaction_with_dialog(&transaction_id, &dialog_id);
        tracing::debug!(
            "🔍 INVITE HANDLER: Associated transaction {:?} with dialog {}",
            SafeTransactionKey(&transaction_id),
            dialog_id
        );

        if owner == InviteCausalOwner::AcknowledgedSession {
            let event = SessionCoordinationEvent::IncomingCall {
                dialog_id: dialog_id.clone(),
                transaction_id: transaction_id.clone(),
                request: request.clone(),
                source,
            };

            tracing::debug!(
                "🔍 INVITE HANDLER: About to send SessionCoordinationEvent::IncomingCall for dialog {}",
                dialog_id
            );
            match self.try_emit_session_coordination_event(event).await {
                Ok(true) => {}
                Ok(false) => {
                    let mut response = response_builders::create_response(
                        &request,
                        StatusCode::ServiceUnavailable,
                    );
                    self.stamp_initial_invite_response_tag(&dialog_id, &mut response)?;
                    self.send_unowned_final_response_classified(&transaction_id, response)
                        .await?;
                    self.retire_unowned_response_indexes(&dialog_id, &transaction_id);
                    self.remove_dialog_storage(&dialog_id);
                    return Ok(None);
                }
                Err(error) => return Err(error),
            }
            tracing::debug!("🔍 INVITE HANDLER: Successfully sent SessionCoordinationEvent::IncomingCall for dialog {}", dialog_id);
        }
        if let Some(started) = setup_started {
            crate::diagnostics::record_dialog_initial_invite_setup(started.elapsed());
        }
        info!("Initial INVITE processed, created dialog {}", dialog_id);
        Ok(Some(dialog_id))
    }

    /// Handle re-INVITE (session modification)
    pub async fn handle_reinvite(
        &self,
        transaction_id: TransactionKey,
        request: Request,
        dialog_id: DialogId,
    ) -> DialogResult<()> {
        let dialog_setup_lock = self
            .transaction_manager
            .server_invite_dialog_setup_lock(&transaction_id)
            .ok_or_else(|| DialogError::TransactionError {
                message: "INVITE server transaction had no dialog setup owner".to_string(),
            })?;
        let _dialog_setup_guard = dialog_setup_lock.lock().await;
        if self.find_dialog_for_transaction(&transaction_id).is_ok() {
            return Ok(());
        }
        self.handle_reinvite_for_owner(
            transaction_id,
            request,
            dialog_id,
            InviteCausalOwner::AcknowledgedSession,
        )
        .await
        .map(|_| ())
    }

    async fn handle_reinvite_for_owner(
        &self,
        transaction_id: TransactionKey,
        request: Request,
        dialog_id: DialogId,
        owner: InviteCausalOwner,
    ) -> DialogResult<Option<DialogId>> {
        debug!("Processing re-INVITE for dialog {}", dialog_id);

        // Associate transaction with dialog
        self.associate_transaction_with_dialog(&transaction_id, &dialog_id);

        // Update dialog sequence number
        {
            let mut dialog = self.get_dialog_mut(&dialog_id)?;
            dialog.update_remote_sequence(&request)?;
        }

        if owner == InviteCausalOwner::AcknowledgedSession {
            let event = SessionCoordinationEvent::ReInvite {
                dialog_id: dialog_id.clone(),
                transaction_id: transaction_id.clone(),
                request: request.clone(),
            };

            match self.try_emit_session_coordination_event(event).await {
                Ok(true) => {}
                Ok(false) => {
                    let response = response_builders::create_response(
                        &request,
                        StatusCode::ServiceUnavailable,
                    );
                    self.send_unowned_final_response_classified(&transaction_id, response)
                        .await?;
                    self.retire_unowned_response_indexes(&dialog_id, &transaction_id);
                    return Ok(None);
                }
                Err(error) => return Err(error),
            }
        }
        info!("Re-INVITE processed for dialog {}", dialog_id);
        Ok(Some(dialog_id))
    }

    /// Process ACK within a dialog (related to INVITE processing)
    pub async fn process_ack_in_dialog(
        &self,
        request: Request,
        dialog_id: DialogId,
    ) -> DialogResult<()> {
        let transaction_id = self
            .transaction_manager
            .find_server_invite_for_ack(&request)
            .ok_or_else(|| {
                DialogError::routing_error("ACK has no exact matching server INVITE transaction")
            })?;

        // Keep this public compatibility facade thin. The transaction-event
        // path and direct protocol path share the exact same ACK owner, which
        // validates the transaction-to-dialog binding before causal delivery.
        self.handle_ack_received_event(&dialog_id, &transaction_id, request)
            .await
    }
}

#[cfg(test)]
mod exact_invite_response_tests {
    use super::*;
    use async_trait::async_trait;
    use rvoip_sip_core::builder::SimpleRequestBuilder;
    use rvoip_sip_core::{Message, Method};
    use rvoip_sip_transport::error::Result as TransportResult;
    use rvoip_sip_transport::{Transport, TransportEvent};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::sync::{mpsc, Mutex};

    use crate::transaction::TransactionManager;

    #[derive(Debug)]
    struct RecordingTransport {
        local_addr: SocketAddr,
        closed: AtomicBool,
        sent: Mutex<Vec<Message>>,
    }

    #[derive(Clone)]
    struct FailingSessionHandler {
        attempts: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl rvoip_infra_common::events::coordinator::CrossCrateEventHandler for FailingSessionHandler {
        async fn handle(
            &self,
            _event: Arc<dyn rvoip_infra_common::events::cross_crate::CrossCrateEvent>,
        ) -> anyhow::Result<()> {
            self.attempts.fetch_add(1, Ordering::AcqRel);
            Err(anyhow::anyhow!("injected authoritative handler failure"))
        }
    }

    impl RecordingTransport {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                local_addr: "127.0.0.1:5060".parse().unwrap(),
                closed: AtomicBool::new(false),
                sent: Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait]
    impl Transport for RecordingTransport {
        fn local_addr(&self) -> TransportResult<SocketAddr> {
            Ok(self.local_addr)
        }

        async fn send_message(
            &self,
            message: Message,
            _destination: SocketAddr,
        ) -> TransportResult<()> {
            self.sent.lock().await.push(message);
            Ok(())
        }

        async fn close(&self) -> TransportResult<()> {
            self.closed.store(true, Ordering::Release);
            Ok(())
        }

        fn is_closed(&self) -> bool {
            self.closed.load(Ordering::Acquire)
        }
    }

    fn initial_invite(call_id: &str, branch: &str) -> Request {
        SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
            .unwrap()
            .from("Alice", "sip:alice@example.com", Some("alice-tag"))
            .to("Bob", "sip:bob@example.com", None)
            .contact("sip:alice@127.0.0.1:5061", None)
            .call_id(call_id)
            .cseq(1)
            .via("127.0.0.1:5061", "UDP", Some(branch))
            .max_forwards(70)
            .build()
    }

    async fn recording_manager() -> (DialogManager, Arc<RecordingTransport>) {
        let transport = RecordingTransport::new();
        let transaction_transport: Arc<dyn Transport> = transport.clone();
        let (_transport_tx, transport_rx) = mpsc::channel::<TransportEvent>(8);
        let (transaction_manager, _events) =
            TransactionManager::new(transaction_transport, transport_rx, Some(8))
                .await
                .expect("transaction manager");
        let manager = DialogManager::new(
            Arc::new(transaction_manager),
            "127.0.0.1:5060".parse().unwrap(),
        )
        .await
        .expect("dialog manager");
        (manager, transport)
    }

    #[test]
    fn initial_and_reinvite_failure_responses_are_transaction_classified() {
        let source = include_str!("invite_handler.rs");
        let entry = source
            .split("async fn handle_invite_for_owner")
            .nth(1)
            .and_then(|tail| tail.split("/// Process a manually supplied INVITE").next())
            .expect("INVITE entry source");
        assert!(entry.contains("StatusCode::CallOrTransactionDoesNotExist"));
        assert!(entry.contains("send_unowned_final_response_classified"));

        let initial = source
            .split("pub async fn handle_initial_invite")
            .nth(1)
            .and_then(|tail| tail.split("/// Handle re-INVITE").next())
            .expect("initial INVITE source");
        assert!(initial.contains("StatusCode::ServiceUnavailable"));
        assert!(initial.contains("retire_unowned_response_indexes"));
        assert!(initial.contains("remove_dialog_storage"));

        let reinvite = source
            .split("pub async fn handle_reinvite")
            .nth(1)
            .and_then(|tail| tail.split("/// Process ACK").next())
            .expect("re-INVITE source");
        assert!(reinvite.contains("StatusCode::ServiceUnavailable"));
        assert!(reinvite.contains("send_unowned_final_response_classified"));
        assert!(reinvite.contains("retire_unowned_response_indexes"));
    }

    #[tokio::test]
    async fn direct_api_owns_one_exact_dialog_without_automatic_response() {
        let (manager, transport) = recording_manager().await;
        let request = initial_invite("direct-api-owned-invite", "z9hG4bK-direct-api-owned");

        let source = "127.0.0.1:5061".parse().unwrap();
        let first = manager.handle_direct_invite(request.clone(), source);
        let second = manager.handle_direct_invite(request.clone(), source);
        let (dialog_id, duplicate_dialog_id) = tokio::join!(first, second);
        let dialog_id = dialog_id.expect("direct API INVITE");
        let duplicate_dialog_id = duplicate_dialog_id.expect("duplicate direct API INVITE");

        assert_eq!(duplicate_dialog_id, dialog_id);
        assert_eq!(manager.dialog_count(), 1);
        assert_eq!(
            manager.find_dialog_for_request(&request).await,
            Some(dialog_id.clone())
        );
        let transactions = manager.server_transactions_for_dialog(&dialog_id);
        assert_eq!(
            transactions.len(),
            1,
            "one exact server transaction owns the dialog"
        );
        assert_eq!(
            manager
                .find_dialog_for_transaction(&transactions[0])
                .expect("transaction dialog owner"),
            dialog_id
        );
        assert!(
            transport.sent.lock().await.is_empty(),
            "the direct API caller owns the response decision"
        );
    }

    #[tokio::test]
    async fn protocol_ingress_without_session_owner_sends_one_503_and_cleans_dialog() {
        let (manager, transport) = recording_manager().await;
        let request = initial_invite("unowned-ingress-invite", "z9hG4bK-unowned-ingress");
        let source = "127.0.0.1:5061".parse().unwrap();
        let transaction = manager
            .transaction_manager()
            .create_server_transaction(request.clone(), source)
            .await
            .expect("ingress INVITE transaction");

        manager
            .handle_initial_invite(transaction.id().clone(), request.clone(), source)
            .await
            .expect("classified no-owner response");

        let sent = transport.sent.lock().await;
        assert_eq!(sent.len(), 1, "no-owner ingress emits one final response");
        assert!(matches!(
            sent.first(),
            Some(Message::Response(response))
                if response.status_code() == StatusCode::ServiceUnavailable.as_u16()
        ));
        drop(sent);

        assert_eq!(manager.find_dialog_for_request(&request).await, None);
        let retained = manager.retention_counts();
        assert_eq!(retained.dialogs, 0);
        assert_eq!(retained.dialog_lookup, 0);
        assert_eq!(retained.early_dialog_lookup, 0);
        assert_eq!(retained.transaction_to_dialog, 0);
        assert_eq!(retained.dialog_invite_transactions, 0);
        assert_eq!(retained.dialog_server_transactions, 0);
        assert_eq!(retained.pending_response_transaction_by_dialog, 0);
    }

    #[tokio::test]
    async fn authoritative_handler_error_never_authors_a_competing_503() {
        use rvoip_infra_common::events::{EventCoordinatorConfig, GlobalEventCoordinator};

        let (manager, transport) = recording_manager().await;
        let attempts = Arc::new(AtomicUsize::new(0));
        let coordinator = Arc::new(
            GlobalEventCoordinator::new(EventCoordinatorConfig::monolithic())
                .await
                .expect("event coordinator"),
        );
        coordinator
            .register_handler(
                "dialog_to_session",
                FailingSessionHandler {
                    attempts: attempts.clone(),
                },
            )
            .await
            .expect("authoritative handler");
        let event_hub = crate::events::DialogEventHub::new(coordinator, Arc::new(manager.clone()))
            .await
            .expect("dialog event hub");
        manager.set_event_hub(event_hub).await;
        let request = initial_invite("failed-owner-invite", "z9hG4bK-failed-owner");
        let source = "127.0.0.1:5061".parse().unwrap();
        let transaction = manager
            .transaction_manager()
            .create_server_transaction(request.clone(), source)
            .await
            .expect("ingress INVITE transaction");

        assert!(manager
            .handle_initial_invite(transaction.id().clone(), request.clone(), source)
            .await
            .is_err());

        assert_eq!(attempts.load(Ordering::Acquire), 1);
        assert!(
            transport.sent.lock().await.is_empty(),
            "handler failure may follow partial causal work and must not trigger a fallback"
        );
        assert_eq!(manager.dialog_count(), 1);
        assert!(manager.find_dialog_for_request(&request).await.is_some());
    }
}
