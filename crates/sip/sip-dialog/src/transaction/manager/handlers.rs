/// # SIP Transaction Message Handlers
///
/// This module implements handlers for processing incoming and outgoing SIP messages through
/// the transaction layer as defined in RFC 3261 Section 17. These handlers are responsible for:
///
/// 1. **Matching** - Matching incoming messages to existing transactions
/// 2. **Routing** - Routing messages to appropriate transactions
/// 3. **State Transitions** - Triggering state transitions in transaction state machines
/// 4. **Special Method Handling** - Processing special cases like ACK, CANCEL, and stray messages
/// 5. **Response Generation** - Automatically generating specific responses (e.g., 200 OK for CANCEL)
///
/// The handlers implement the core logic required for the transaction layer to fulfill its role
/// as the reliability layer between the Transport layer and the Transaction User (TU).
///
/// ## RFC 3261 Specification Coverage
///
/// These handlers implement the behavior required by:
/// - Section 17.1.3: Matching responses to client transactions
/// - Section 17.2.3: Matching requests to server transactions
/// - Section 8.2.6: Generating automatic responses
/// - Section 9.2: CANCEL handling
/// - Section 17.1.1.3: ACK handling
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;

use rvoip_infra_common::events::cross_crate::SipTraceDirection;
use rvoip_sip_core::prelude::*;
use rvoip_sip_transport::transport::TransportType;
use rvoip_sip_transport::{Transport, TransportEvent, TransportFlowId, TransportRoute};
use tokio::sync::mpsc;
use tracing::{debug, error, warn};

use crate::diagnostics;
use crate::transaction::error::{Error, Result};
use crate::transaction::runner::HasLifecycle;
use crate::transaction::server::ServerTransaction;
use crate::transaction::state::TransactionLifecycle;
use crate::transaction::utils::{create_ack_from_invite, transaction_key_from_message};
use crate::transaction::{SipRequestAuthorization, SipRequestIngressContext, SipRequestRejection};
use crate::transaction::{TransactionEvent, TransactionKey, TransactionKind, TransactionState};

use super::types::*;
use super::TransactionManager;

fn bind_client_response_route(
    expected: &TransportRoute,
    source: SocketAddr,
    transport_type: TransportType,
    ingress_flow_id: Option<TransportFlowId>,
) -> Option<TransportRoute> {
    if expected.destination != source || expected.transport_type != Some(transport_type) {
        return None;
    }

    let mut bound = expected.clone();
    match transport_type {
        TransportType::Udp => {
            if ingress_flow_id.is_some() {
                return None;
            }
            bound.flow_id = None;
        }
        TransportType::Tcp | TransportType::Tls | TransportType::Ws | TransportType::Wss => {
            // A stream response is authenticated only by the opaque flow that
            // carried the original request. Resolving by address here could
            // bind a retired transaction to a later co-addressed connection.
            let expected_flow_id = expected.flow_id?;
            if ingress_flow_id != Some(expected_flow_id) {
                return None;
            }
            bound.flow_id = Some(expected_flow_id);
        }
    }
    Some(bound)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientResponseRouteAuthentication {
    Authenticated,
    UnknownTransaction,
    Rejected,
}

impl TransactionManager {
    /// Authenticate a response against either the active transaction route or
    /// the bounded INVITE tombstone that replaced it. A transaction key alone
    /// is attacker-controlled wire data and is never sufficient to revive a
    /// retired transaction.
    async fn authenticate_client_response_route(
        &self,
        transaction_id: &TransactionKey,
        source: SocketAddr,
        transport_type: TransportType,
        ingress_flow_id: Option<TransportFlowId>,
    ) -> ClientResponseRouteAuthentication {
        // The transaction's initial write may have selected a concrete stream
        // flow after the initial route entry was populated. Snapshot that
        // route before taking the DashMap shard and then make exactly one
        // authorization decision against the authoritative Active/Retired
        // state below.
        let sent_route = if let Some(client) = self
            .client_transactions
            .get(transaction_id)
            .map(|transaction| transaction.value().clone())
        {
            Some(client.data().request_route.lock().await.clone())
        } else {
            None
        };

        let Some(mut state) = self.transaction_destinations.get_mut(transaction_id) else {
            return ClientResponseRouteAuthentication::UnknownTransaction;
        };

        let expected = match state.value() {
            super::ClientResponseRouteState::Active(indexed_route) => sent_route
                .filter(|sent_route| {
                    sent_route.destination == indexed_route.destination
                        && sent_route.transport_type == indexed_route.transport_type
                        && sent_route.authority == indexed_route.authority
                        && sent_route.flow_id.is_some()
                })
                .unwrap_or_else(|| indexed_route.clone()),
            super::ClientResponseRouteState::Retired(retired) => {
                if retired.expires_at <= Instant::now() {
                    drop(state);
                    self.transaction_destinations
                        .remove_if(transaction_id, |_, current| {
                            current
                                .retired()
                                .is_some_and(|retired| retired.expires_at <= Instant::now())
                        });
                    return ClientResponseRouteAuthentication::UnknownTransaction;
                }
                retired.route.clone()
            }
        };

        let Some(bound) =
            bind_client_response_route(&expected, source, transport_type, ingress_flow_id)
        else {
            return ClientResponseRouteAuthentication::Rejected;
        };

        match state.value_mut() {
            super::ClientResponseRouteState::Active(route) => *route = bound,
            super::ClientResponseRouteState::Retired(retired) => retired.route = bound,
        }
        ClientResponseRouteAuthentication::Authenticated
    }
}

/// Handle transport message events and route them to appropriate transactions.
///
/// This is the main entry point for all incoming SIP messages from the transport
/// layer. It implements the message matching rules specified in RFC 3261 sections
/// 17.1.3 (client transactions) and 17.2.3 (server transactions).
///
/// The function:
/// 1. Identifies the transaction that should handle the message
/// 2. Routes requests/responses to appropriate transactions
/// 3. Handles special cases (ACK, CANCEL)
/// 4. Reports "stray" messages that don't match any transaction
///
/// # Arguments
/// * `event` - The transport event containing the message and addressing information
/// * `transport` - The transport layer for sending responses
/// * `client_transactions` - Map of active client transactions
/// * `server_transactions` - Map of active server transactions
/// * `events_tx` - Channel for broadcasting transaction events
/// * `event_subscribers` - Additional event subscribers
/// * `manager` - Reference to the TransactionManager
///
/// # Returns
/// * `Result<()>` - Success or error depending on message processing outcome
///
/// Retained for the upcoming transport-event dispatcher refactor; today
/// the manager dispatches transport events inline.
#[allow(dead_code)]
pub(crate) async fn handle_transport_message(
    event: TransportEvent,
    transport: &Arc<dyn Transport>,
    client_transactions: &Arc<
        dashmap::DashMap<TransactionKey, crate::transaction::manager::ArcClientTransaction>,
    >,
    server_transactions: &Arc<dashmap::DashMap<TransactionKey, Arc<dyn ServerTransaction>>>,
    events_tx: &mpsc::Sender<TransactionEvent>,
    event_subscribers: &Arc<arc_swap::ArcSwap<Vec<super::EventSubscriber>>>,
    manager: &TransactionManager,
) -> Result<()> {
    match event {
        TransportEvent::MessageReceived {
            message,
            source,
            destination,
            transport_type,
            flow_id,
            connection_metadata,
            ..
        } => {
            let ingress_context =
                SipRequestIngressContext::new(source, destination, transport_type);
            let ingress_context = match flow_id {
                Some(flow_id) => ingress_context.with_flow_id(flow_id),
                None => ingress_context,
            };
            let ingress_context = match connection_metadata {
                Some(metadata) => ingress_context.with_connection_metadata(metadata),
                None => ingress_context,
            };
            match message {
                Message::Request(request) => {
                    // First, determine the transaction ID/key
                    let tx_id =
                        match transaction_key_from_message(&Message::Request(request.clone())) {
                            Some(key) => key,
                            None => {
                                return Err(Error::Other(
                                    "Could not determine transaction ID from request".into(),
                                ));
                            }
                        };

                    // Handle ACK specially
                    if request.method() == Method::Ack {
                        let ack_request = request.clone();

                        // DashMap path — either direct key hit or a
                        // dialog-identifier lookup via the manager's ACK
                        // index. Neither holds across `.await`.
                        let tx_opt = server_transactions
                            .get(&tx_id)
                            .map(|entry| entry.value().clone());

                        // If we found a transaction, process the ACK
                        if let Some(tx) = tx_opt {
                            let tx_id = tx.id().clone();
                            let tx_kind = tx.kind();

                            if tx_kind == TransactionKind::InviteServer {
                                debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), "Processing ACK for server INVITE transaction");

                                let tx_clone = tx.clone();

                                // Use a timeout to avoid blocking indefinitely if the transaction is shutting down
                                match tokio::time::timeout(
                                    std::time::Duration::from_millis(500),
                                    tx_clone.process_request(ack_request.clone()),
                                )
                                .await
                                {
                                    Ok(result) => {
                                        // Process the result
                                        match result {
                                            Ok(_) => {
                                                // Successfully processed ACK
                                                manager
                                                    .mark_invite_2xx_response_cache_acked(&tx_id);

                                                // Broadcast the event
                                                TransactionManager::broadcast_event(
                                                    TransactionEvent::AckReceived {
                                                        transaction_id: tx_id.clone(),
                                                        request: ack_request,
                                                    },
                                                    events_tx,
                                                    event_subscribers,
                                                    Some(&manager.subscriber_to_transactions),
                                                    Some(&manager.transaction_to_subscribers),
                                                    None,
                                                )
                                                .await;

                                                return Ok(());
                                            }
                                            Err(e) => {
                                                // Transaction error - likely channel closed
                                                warn!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "Failed to process ACK request, treating as stray ACK");
                                                // Fall through to stray ACK handling
                                            }
                                        }
                                    }
                                    Err(_) => {
                                        // Timeout waiting for transaction to process ACK
                                        warn!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), "Timeout processing ACK request, treating as stray ACK");
                                        // Fall through to stray ACK handling
                                    }
                                }
                            }
                            // else: not an INVITE server transaction → fall through
                        }

                        if let Some(invite_tx_id) = manager.find_server_invite_for_ack(&ack_request)
                        {
                            TransactionManager::broadcast_event(
                                TransactionEvent::AckReceived {
                                    transaction_id: invite_tx_id,
                                    request: ack_request,
                                },
                                events_tx,
                                event_subscribers,
                                Some(&manager.subscriber_to_transactions),
                                Some(&manager.transaction_to_subscribers),
                                None,
                            )
                            .await;
                            return Ok(());
                        }

                        // Handle as stray ACK if we reached this point
                        debug!("Received ACK that doesn't match any server transaction");
                        TransactionManager::broadcast_event(
                            TransactionEvent::StrayAck {
                                request: ack_request,
                                source,
                            },
                            events_tx,
                            event_subscribers,
                            Some(&manager.subscriber_to_transactions),
                            Some(&manager.transaction_to_subscribers),
                            None,
                        )
                        .await;

                        return Ok(());
                    }

                    // Handle CANCEL specially
                    if request.method() == Method::Cancel {
                        // Extract the branch parameter from the CANCEL request
                        let cancel_branch = match request.first_via() {
                            Some(via) => match via.branch() {
                                Some(branch) => branch.to_string(),
                                None => {
                                    debug!(
                                        "CANCEL request has no branch parameter, can't find matching INVITE"
                                    );
                                    // Fall through to stray CANCEL handling
                                    handle_stray_cancel(
                                        request.clone(),
                                        ingress_context.response_route(),
                                        transport,
                                    )
                                    .await?;

                                    // Broadcast stray CANCEL event
                                    TransactionManager::broadcast_event(
                                        TransactionEvent::StrayCancel { request, source },
                                        events_tx,
                                        event_subscribers,
                                        Some(&manager.subscriber_to_transactions),
                                        Some(&manager.transaction_to_subscribers),
                                        None,
                                    )
                                    .await;
                                    return Ok(());
                                }
                            },
                            None => {
                                debug!(
                                    "CANCEL request has no Via header, can't find matching INVITE"
                                );
                                // Fall through to stray CANCEL handling
                                handle_stray_cancel(
                                    request.clone(),
                                    ingress_context.response_route(),
                                    transport,
                                )
                                .await?;

                                // Broadcast stray CANCEL event
                                TransactionManager::broadcast_event(
                                    TransactionEvent::StrayCancel { request, source },
                                    events_tx,
                                    event_subscribers,
                                    Some(&manager.subscriber_to_transactions),
                                    Some(&manager.transaction_to_subscribers),
                                    None,
                                )
                                .await;
                                return Ok(());
                            }
                        };

                        // Create a modified key for the INVITE transaction with the same branch
                        let invite_tx_id = TransactionKey::new(cancel_branch, Method::Invite, true);

                        debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&invite_tx_id), "Looking for INVITE transaction");

                        // Check if we have a matching INVITE transaction with the same branch.
                        // Clone the Arc out of the DashMap shard so the shard
                        // guard drops before any subsequent `.await`.
                        let tx_clone_opt = server_transactions.get(&invite_tx_id).and_then(|r| {
                            let tx = r.value();
                            if tx.kind() == TransactionKind::InviteServer {
                                Some((tx.clone(), invite_tx_id.clone()))
                            } else {
                                None
                            }
                        });

                        if let Some((tx, tx_id_clone)) = tx_clone_opt {
                            // Now proceed with the transaction outside the lock
                            let request_clone = request.clone();

                            debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id_clone), "Processing CANCEL for server INVITE transaction");

                            // Broadcast event
                            TransactionManager::broadcast_event(
                                TransactionEvent::CancelReceived {
                                    transaction_id: tx_id_clone.clone(),
                                    cancel_request: request_clone.clone(),
                                },
                                events_tx,
                                event_subscribers,
                                Some(&manager.subscriber_to_transactions),
                                Some(&manager.transaction_to_subscribers),
                                None,
                            )
                            .await;

                            // Send OK response to CANCEL
                            let mut builder = ResponseBuilder::new(StatusCode::Ok, None);

                            // Add necessary headers
                            if let Some(to) = request_clone.to() {
                                builder = builder.header(TypedHeader::To(to.clone()));
                            }

                            if let Some(from) = request_clone.from() {
                                builder = builder.header(TypedHeader::From(from.clone()));
                            }

                            if let Some(call_id) = request_clone.call_id() {
                                builder = builder.header(TypedHeader::CallId(call_id.clone()));
                            }

                            if let Some(cseq) = request_clone.cseq() {
                                builder = builder.header(TypedHeader::CSeq(cseq.clone()));
                            }

                            if let Some(via) = request_clone.header(&HeaderName::Via) {
                                builder = builder.header(via.clone());
                            }

                            // Build and send response to CANCEL
                            let cancel_response = builder.build();
                            if let Err(e) = transport
                                .send_message_via(
                                    Message::Response(cancel_response),
                                    ingress_context.response_route(),
                                )
                                .await
                            {
                                return Err(Error::transport_error(
                                    e,
                                    "Failed to send 200 OK response to CANCEL",
                                ));
                            }

                            // Now send 487 Request Terminated for the original INVITE
                            debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id_clone), "Sending 487 Request Terminated for the original INVITE");

                            let mut builder =
                                ResponseBuilder::new(StatusCode::RequestTerminated, None);

                            if let Some(invite_request) = tx.original_request().await {
                                // Add necessary headers from the INVITE request
                                if let Some(to) = invite_request.to() {
                                    builder = builder.header(TypedHeader::To(to.clone()));
                                }

                                if let Some(from) = invite_request.from() {
                                    builder = builder.header(TypedHeader::From(from.clone()));
                                }

                                if let Some(call_id) = invite_request.call_id() {
                                    builder = builder.header(TypedHeader::CallId(call_id.clone()));
                                }

                                if let Some(cseq) = invite_request.cseq() {
                                    builder = builder.header(TypedHeader::CSeq(cseq.clone()));
                                }

                                if let Some(via) = invite_request.header(&HeaderName::Via) {
                                    builder = builder.header(via.clone());
                                }

                                // Build the 487 response
                                let invite_response = builder.build();

                                // Instead of sending directly through the transport,
                                // send through the transaction's send_response method
                                // This ensures proper state transition and processing
                                if let Err(e) = tx.send_response(invite_response).await {
                                    warn!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id_clone), error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "Failed to send 487 Request Terminated through transaction");
                                    return Err(Error::Other(format!(
                                        "Failed to send 487 Request Terminated: {}",
                                        e
                                    )));
                                }
                            }

                            return Ok(());
                        }

                        // If no matching transaction was found, handle as stray CANCEL
                        debug!("Received CANCEL that doesn't match any INVITE server transaction");
                        handle_stray_cancel(
                            request.clone(),
                            ingress_context.response_route(),
                            transport,
                        )
                        .await?;

                        // Broadcast stray CANCEL event
                        TransactionManager::broadcast_event(
                            TransactionEvent::StrayCancel { request, source },
                            events_tx,
                            event_subscribers,
                            Some(&manager.subscriber_to_transactions),
                            Some(&manager.transaction_to_subscribers),
                            None,
                        )
                        .await;
                        return Ok(());
                    }

                    // Handle regular request retransmission and new requests.
                    // Clone the Arc out of the DashMap shard before any `.await`.
                    let existing_tx = server_transactions.get(&tx_id).map(|r| r.value().clone());

                    if let Some(tx) = existing_tx {
                        debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), "Processing retransmission of existing request");

                        let lifecycle = tx.data().get_lifecycle();
                        if !matches!(lifecycle, TransactionLifecycle::Active) {
                            if request.method() == Method::Invite
                                && manager
                                    .retransmit_cached_invite_2xx_response_on_route(
                                        &tx_id,
                                        ingress_context.response_route(),
                                    )
                                    .await?
                            {
                                return Ok(());
                            }
                            debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), ?lifecycle, "Skipping request processing for non-active transaction");
                            return Ok(());
                        }

                        if request.method() == Method::Invite
                            && tx.state() == TransactionState::Terminated
                            && manager
                                .retransmit_cached_invite_2xx_response_on_route(
                                    &tx_id,
                                    ingress_context.response_route(),
                                )
                                .await?
                        {
                            return Ok(());
                        }

                        tx.process_request(request.clone()).await?;
                        return Ok(());
                    }

                    if request.method() == Method::Invite
                        && manager
                            .retransmit_cached_invite_2xx_response_on_route(
                                &tx_id,
                                ingress_context.response_route(),
                            )
                            .await?
                    {
                        return Ok(());
                    }

                    // If we get here, this is a new request
                    debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), method=%crate::transaction::safe_diagnostics::SafeMethod::new(&request.method()), "Received new request, delegate to proper handler");

                    // Delegate to the actual request handler which will create appropriate transactions
                    // and generate the correct InviteRequest or NonInviteRequest events
                    if let Err(e) = manager
                        .handle_request(request, source, &ingress_context)
                        .await
                    {
                        warn!(error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "Failed to handle new request");
                    }

                    return Ok(());
                }
                Message::Response(response) => {
                    // Try to match the response to a client transaction by deriving its ID
                    let tx_id =
                        match transaction_key_from_message(&Message::Response(response.clone())) {
                            Some(key) => key,
                            None => {
                                return Err(Error::Other(
                                    "Could not determine transaction ID from response".into(),
                                ));
                            }
                        };

                    match manager
                        .authenticate_client_response_route(&tx_id, source, transport_type, flow_id)
                        .await
                    {
                        ClientResponseRouteAuthentication::Authenticated => {}
                        ClientResponseRouteAuthentication::Rejected => {
                            warn!(
                                transport = %transport_type,
                                "Dropping client response received outside its authenticated transaction route"
                            );
                            return Ok(());
                        }
                        ClientResponseRouteAuthentication::UnknownTransaction => {
                            TransactionManager::broadcast_event(
                                TransactionEvent::StrayResponse { response, source },
                                events_tx,
                                event_subscribers,
                                Some(&manager.subscriber_to_transactions),
                                Some(&manager.transaction_to_subscribers),
                                None,
                            )
                            .await;
                            return Ok(());
                        }
                    }

                    // Look up the client transaction — clone Arc out of shard.
                    let client_tx_arc = client_transactions.get(&tx_id).map(|r| r.value().clone());

                    if let Some(tx) = client_tx_arc {
                        let tx_kind = tx.kind();
                        let remote_addr = tx.remote_addr();

                        debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), status = ?response.status(), "Routing response to client transaction");

                        let lifecycle = tx.data().get_lifecycle();
                        if !matches!(lifecycle, TransactionLifecycle::Active) {
                            debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), ?lifecycle, "Skipping response processing for non-active transaction");
                            if tx_id.method() == &Method::Invite && response.status().is_success() {
                                TransactionManager::broadcast_event(
                                    TransactionEvent::SuccessResponse {
                                        transaction_id: tx_id,
                                        response,
                                        need_ack: true,
                                        source,
                                    },
                                    events_tx,
                                    event_subscribers,
                                    Some(&manager.subscriber_to_transactions),
                                    Some(&manager.transaction_to_subscribers),
                                    None,
                                )
                                .await;
                            }
                            return Ok(());
                        }

                        tx.process_response(response.clone()).await?;

                        // Automatic ACK for non-2xx responses to INVITE
                        if !response.status().is_success()
                            && tx_kind == TransactionKind::InviteClient
                        {
                            debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), status=%response.status(), "Sending ACK automatically for non-2xx response");

                            // Create a dummy request for ACK creation
                            let dummy_uri = if let Some(to) = response.to() {
                                to.address().uri.clone()
                            } else {
                                Uri::sip("invalid")
                            };

                            let dummy_request = Request::new(Method::Invite, dummy_uri);

                            match create_ack_from_invite(&dummy_request, &response) {
                                Ok(ack_request) => {
                                    // Send the ACK
                                    if let Err(e) = transport
                                        .send_message(Message::Request(ack_request), remote_addr)
                                        .await
                                    {
                                        return Err(Error::transport_error(
                                            e,
                                            "Failed to send ACK for non-2xx response",
                                        ));
                                    }
                                }
                                Err(e) => {
                                    warn!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "Failed to create ACK request");
                                }
                            }
                        }

                        return Ok(());
                    }

                    if tx_id.method() == &Method::Invite && response.status().is_success() {
                        // Route authentication can succeed from the bounded
                        // retired-transaction tombstone after the live Arc is
                        // removed. Forked/retransmitted INVITE 2xx still belong
                        // to the TU and require ACK handling.
                        TransactionManager::broadcast_event(
                            TransactionEvent::SuccessResponse {
                                transaction_id: tx_id,
                                response,
                                need_ack: true,
                                source,
                            },
                            events_tx,
                            event_subscribers,
                            Some(&manager.subscriber_to_transactions),
                            Some(&manager.transaction_to_subscribers),
                            None,
                        )
                        .await;
                        return Ok(());
                    }

                    // If we get here, this is a stray response
                    debug!(status=%response.status(), "Received stray response that doesn't match any client transaction");

                    // Broadcast stray response event
                    TransactionManager::broadcast_event(
                        TransactionEvent::StrayResponse { response, source },
                        events_tx,
                        event_subscribers,
                        Some(&manager.subscriber_to_transactions),
                        Some(&manager.transaction_to_subscribers),
                        None,
                    )
                    .await;

                    return Ok(());
                }
            }
        }
        TransportEvent::Error { error } => {
            warn!(error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&error), "Transport error");
            // TODO: Determine if any transactions were affected by this error
            // and propagate the error to them
        }
        _ => {
            // Ignore other transport events for now
        }
    }

    Ok(())
}

/// Determine ACK destination for 2xx responses according to RFC 3261 Section 13.2.2.4.
///
/// For 2xx responses to INVITE, ACK requests are sent directly from the TU to the peer.
/// This function implements the algorithm to determine where to send the ACK based on:
/// 1. The Contact header if present
/// 2. Fallback to Via header received/rport parameters or sent-by value
///
/// # Arguments
/// * `response` - The 2xx response to ACK
///
/// # Returns
/// * `Option<SocketAddr>` - The destination socket address if it can be determined
///
/// `utils::determine_ack_destination` is the canonical implementation
/// at the module surface; this variant + its `resolve_*` helpers
/// below are kept for the upcoming DNS-aware fallback path.
#[allow(dead_code)]
pub(crate) async fn determine_ack_destination(response: &Response) -> Option<SocketAddr> {
    if let Some(contact_header) = response.header(&HeaderName::Contact) {
        if let TypedHeader::Contact(contact) = contact_header {
            if let Some(addr) = contact.addresses().next() {
                if let Some(dest) = resolve_uri_to_socketaddr(&addr.uri).await {
                    return Some(dest);
                }
            }
        }
    }

    // Try via received/rport
    if let Some(via) = response.first_via() {
        if let (Some(received_ip_str), Some(port)) = (
            via.received().map(|ip| ip.to_string()),
            via.rport().flatten(),
        ) {
            if let Ok(ip) = IpAddr::from_str(&received_ip_str) {
                let dest = SocketAddr::new(ip, port);
                return Some(dest);
            } else {
                warn!(ip=%received_ip_str, "Failed to parse received IP in Via");
            }
        }

        // Fallback to Via host/port
        // For the sent_by, use ViaHeader struct fields
        if let Some(via_header) = via.headers().first() {
            let host = &via_header.sent_by_host;
            let port = via_header.sent_by_port.unwrap_or(5060);

            if let Some(dest) = resolve_host_to_socketaddr(host, port).await {
                return Some(dest);
            }
        }
    }
    None
}

/// Helper to resolve URI host to SocketAddr for ACK destinations.
///
/// This implements the address resolution for SIP URIs according to
/// RFC 3263 procedures.
///
/// # Arguments
/// * `uri` - SIP URI to resolve
///
/// # Returns
/// * `Option<SocketAddr>` - Resolved socket address if successful
#[allow(dead_code)]
async fn resolve_uri_to_socketaddr(uri: &Uri) -> Option<SocketAddr> {
    // Delegate to the shared RFC 3263 resolver. Preserves the ACK
    // destination semantics (`sips:`→5061 default, `transport=` / scheme
    // honoured) while picking up NAPTR / SRV / weighted selection.
    crate::dialog::dialog_utils::resolve_uri_to_socketaddr(uri).await
}

/// Helper to resolve Host enum to SocketAddr for network addressing.
///
/// SIP specification allows both IP addresses and domain names as hosts.
/// This function resolves them to socket addresses for actual transmission.
///
/// # Arguments
/// * `host` - SIP host to resolve (IP or domain)
/// * `port` - Port number to use
///
/// # Returns
/// * `Option<SocketAddr>` - Resolved socket address if successful
#[allow(dead_code)]
async fn resolve_host_to_socketaddr(host: &rvoip_sip_core::Host, port: u16) -> Option<SocketAddr> {
    match host {
        rvoip_sip_core::Host::Address(ip) => Some(SocketAddr::new(*ip, port)),
        rvoip_sip_core::Host::Domain(domain) => {
            if let Ok(ip) = IpAddr::from_str(domain) {
                return Some(SocketAddr::new(ip, port));
            }
            match tokio::net::lookup_host(format!("{}:{}", domain, port)).await {
                Ok(mut addrs) => addrs.next(),
                Err(e) => {
                    error!(
                        error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e),
                        domain_len=domain.len(),
                        "DNS lookup failed for ACK destination"
                    );
                    None
                }
            }
        }
    }
}

/// Create a UDP Via header with a branch parameter for a local address.
///
/// Always requests `rport` (RFC 3581) with no value — carriers and NAT
/// gateways use this to echo back the received port in responses so we
/// can route ACKs and BYEs back through the same pinhole. The incoming
/// path honors `received=` / `rport=` echoed on responses (see the
/// response handler earlier in this module).
pub fn create_via_header(local_addr: &SocketAddr, branch: &str) -> Result<TypedHeader> {
    create_via_header_for_transport(local_addr, branch, "UDP")
}

/// Create a Via header with a branch parameter for a local address and transport.
///
/// This is used by the transaction layer only when a request reaches it without
/// an existing Via. Request builders normally choose the correct transport
/// first; transaction normalization must preserve that choice.
pub fn create_via_header_for_transport(
    local_addr: &SocketAddr,
    branch: &str,
    transport: &str,
) -> Result<TypedHeader> {
    use rvoip_sip_core::types::via::Via;
    use rvoip_sip_core::types::Param;

    let via_params = vec![Param::branch(branch.to_string()), Param::Rport(None)];

    let local_host = local_addr.ip().to_string();
    let local_port = local_addr.port();

    let via = Via::new(
        "SIP",
        "2.0",
        transport.to_ascii_uppercase(),
        &local_host,
        Some(local_port),
        via_params,
    )
    .map_err(Error::SipCoreError)?;

    Ok(TypedHeader::Via(via))
}

#[cfg(test)]
mod via_header_tests {
    use super::*;

    #[test]
    fn via_header_includes_rport_param() {
        let local: SocketAddr = "127.0.0.1:5060".parse().unwrap();
        let header = create_via_header(&local, "z9hG4bK-test").expect("create_via_header");
        let serialized = format!("{}", header);
        assert!(
            serialized.contains("SIP/2.0/UDP"),
            "Via header should default to UDP, got: {}",
            serialized
        );
        assert!(
            serialized.contains(";rport"),
            "Via header should include rport param, got: {}",
            serialized
        );
        assert!(
            serialized.contains(";branch=z9hG4bK-test"),
            "Via header should include branch param, got: {}",
            serialized
        );
    }

    #[test]
    fn via_header_uses_requested_transport() {
        let local: SocketAddr = "127.0.0.1:5061".parse().unwrap();
        let header = create_via_header_for_transport(&local, "z9hG4bK-test", "tls")
            .expect("create_via_header_for_transport");
        let serialized = format!("{}", header);
        assert!(
            serialized.contains("SIP/2.0/TLS"),
            "Via header should use requested transport, got: {}",
            serialized
        );
        assert!(
            serialized.contains(";rport"),
            "Via header should include rport param, got: {}",
            serialized
        );
        assert!(
            serialized.contains(";branch=z9hG4bK-test"),
            "Via header should include branch param, got: {}",
            serialized
        );
    }
}

impl TransactionManager {
    pub(crate) async fn publish_inbound_sip_trace(
        &self,
        message: &Message,
        source: SocketAddr,
        destination: SocketAddr,
        transport_type: TransportType,
    ) {
        if let Some(trace) = &self.sip_trace {
            trace.publish(
                SipTraceDirection::Inbound,
                transport_type,
                destination,
                source,
                message,
            );
        }
    }

    /// Handle an incoming SIP message from the transport layer.
    ///
    /// This is the entry point for incoming messages from the transport layer to
    /// the TransactionManager. It delegates to more specific handlers based on
    /// message type.
    ///
    /// # Arguments
    /// * `event` - Transport event containing the message and addressing information
    ///
    /// # Returns
    /// * `Result<()>` - Success or error depending on message processing outcome
    pub(crate) async fn handle_transport_event(&self, event: TransportEvent) -> Result<()> {
        match event {
            TransportEvent::MessageReceived {
                message,
                source,
                destination,
                transport_type,
                flow_id,
                raw_bytes,
                timing,
                connection_metadata,
            } => {
                debug!("Received message from {}", source);
                self.publish_inbound_sip_trace(&message, source, destination, transport_type)
                    .await;
                let transaction_key =
                    crate::transaction::utils::transaction_key_from_message(&message);
                if let (Message::Response(response), Some(key)) =
                    (&message, transaction_key.as_ref())
                {
                    match self
                        .authenticate_client_response_route(key, source, transport_type, flow_id)
                        .await
                    {
                        ClientResponseRouteAuthentication::Authenticated => {}
                        ClientResponseRouteAuthentication::Rejected => {
                            warn!(
                                transport = %transport_type,
                                ingress_flow = flow_id.is_some(),
                                "Dropping client response received outside its authenticated transaction route"
                            );
                            return Ok(());
                        }
                        ClientResponseRouteAuthentication::UnknownTransaction => {
                            Self::broadcast_event(
                                TransactionEvent::StrayResponse {
                                    response: response.clone(),
                                    source,
                                },
                                &self.events_tx,
                                &self.event_subscribers,
                                Some(&self.subscriber_to_transactions),
                                Some(&self.transaction_to_subscribers),
                                None,
                            )
                            .await;
                            return Ok(());
                        }
                    }
                }
                if let Some(bytes) = raw_bytes.as_ref() {
                    let cache_raw_bytes = match &message {
                        Message::Request(request) => {
                            !matches!(request.method(), Method::Ack | Method::Bye)
                        }
                        Message::Response(_) => transaction_key
                            .as_ref()
                            .is_some_and(|key| self.client_transactions.contains_key(key)),
                    };
                    if cache_raw_bytes {
                        if let Some(key) = transaction_key.as_ref() {
                            // `Bytes::clone` is a refcount bump — no heap alloc.
                            self.pending_inbound_bytes
                                .insert(key.clone(), bytes.clone());
                            self.pending_inbound_inserted_at
                                .insert(key.clone(), Instant::now());
                        }
                    }
                }
                if let Some(key) = transaction_key.as_ref() {
                    let cache_transport = match &message {
                        Message::Request(_) => true,
                        Message::Response(_) => self.client_transactions.contains_key(key),
                    };
                    if cache_transport {
                        self.pending_inbound_transport.insert(
                            key.clone(),
                            rvoip_infra_common::events::cross_crate::SipTransportContext::new(
                                transport_type.to_string(),
                                destination.to_string(),
                                source.to_string(),
                                matches!(
                                    transport_type,
                                    rvoip_sip_transport::transport::TransportType::Tls
                                        | rvoip_sip_transport::transport::TransportType::Wss
                                ),
                            ),
                        );
                    }
                }
                if let (Some(key), Some(timing)) = (transaction_key.as_ref(), timing) {
                    let cache_timing = matches!(
                        &message,
                        Message::Request(request)
                            if matches!(request.method(), Method::Invite | Method::Bye)
                    );
                    if cache_timing {
                        self.pending_inbound_timing.insert(key.clone(), timing);
                    }
                }
                let ingress_context =
                    SipRequestIngressContext::new(source, destination, transport_type);
                let ingress_context = match flow_id {
                    Some(flow_id) => ingress_context.with_flow_id(flow_id),
                    None => ingress_context,
                };
                let ingress_context = match connection_metadata {
                    Some(metadata) => ingress_context.with_connection_metadata(metadata),
                    None => ingress_context,
                };
                self.handle_message(message, source, destination, &ingress_context)
                    .await
            }
            TransportEvent::KeepAlivePongReceived {
                source, flow_id, ..
            } => {
                // RFC 5626 §3.5.1 pong arrived on a connection-oriented
                // transport. Forward to dialog-core's outbound-flow
                // monitor if it's subscribed; no-op otherwise.
                if let Some(sender) = self.flow_event_sender.read().await.as_ref() {
                    let _ = sender
                        .send(
                            crate::manager::outbound_flow::FlowTransportEvent::PongReceived {
                                source,
                                flow_id,
                            },
                        )
                        .await;
                }
                Ok(())
            }
            TransportEvent::ConnectionClosed {
                remote_addr,
                flow_id,
                ..
            } => {
                // Connection-oriented transport lost its flow. Forward
                // so outbound-flow monitor can emit OutboundFlowFailed
                // and trigger re-REGISTER.
                if let Some(sender) = self.flow_event_sender.read().await.as_ref() {
                    let _ = sender
                        .send(
                            crate::manager::outbound_flow::FlowTransportEvent::ConnectionClosed {
                                remote_addr,
                                flow_id,
                            },
                        )
                        .await;
                }
                Ok(())
            }
            _ => {
                // Other transport events (Error, shutdown variants) are
                // handled elsewhere or deliberately ignored.
                Ok(())
            }
        }
    }

    /// Handle a SIP message, routing it to appropriate transaction or creating a new one.
    ///
    /// This method dispatches incoming messages to either request or response
    /// handlers, which implement the core transaction layer logic.
    ///
    /// # Arguments
    /// * `message` - The SIP message (request or response)
    /// * `source` - The source address of the message
    /// * `destination` - The local address that received the message
    ///
    /// # Returns
    /// * `Result<()>` - Success or error depending on message processing outcome
    async fn handle_message(
        &self,
        message: Message,
        source: SocketAddr,
        _destination: SocketAddr,
        ingress_context: &SipRequestIngressContext,
    ) -> Result<()> {
        match message {
            Message::Request(request) => {
                // Special handling for ACK to 2xx responses
                if request.method() == Method::Ack {
                    // ACK requests matching a 2xx response are end-to-end and don't have a transaction
                    return self
                        .handle_ack_request(request, source, ingress_context)
                        .await;
                }

                self.handle_request(request, source, ingress_context).await
            }
            Message::Response(response) => self.handle_response(response, source).await,
        }
    }

    async fn enforce_ingress_authorization(
        &self,
        transaction: &Arc<dyn ServerTransaction>,
        request: &Request,
        ingress_context: &SipRequestIngressContext,
        inherited_principal: Option<rvoip_core_traits::identity::AuthenticatedPrincipal>,
    ) -> Result<bool> {
        let Some(authorizer) = self.request_ingress_authorizer() else {
            return Ok(true);
        };

        let decision = match inherited_principal {
            Some(principal) => SipRequestAuthorization::Authorized { principal },
            None => authorizer.authorize(request, ingress_context).await,
        };

        match decision {
            SipRequestAuthorization::Authorized { principal } => {
                self.retain_inbound_principal(transaction.id().clone(), principal, ingress_context);
                Ok(true)
            }
            SipRequestAuthorization::Rejected(SipRequestRejection {
                status,
                headers,
                reason,
            }) => {
                let mut response =
                    crate::transaction::utils::response_builders::create_response(request, status);
                response.headers.extend(headers);
                if let Some(reason) = reason {
                    warn!(
                        method=%crate::transaction::safe_diagnostics::SafeMethod::new(&request.method()),
                        source = %ingress_context.source,
                        reason_present=true,
                        reason_len=reason.len(),
                        "SIP listener authorization rejected request"
                    );
                }
                self.send_response(transaction.id(), response).await?;
                Ok(false)
            }
        }
    }

    /// Handle an incoming SIP request according to RFC 3261 transaction rules.
    ///
    /// This method:
    /// 1. Attempts to match the request to an existing server transaction
    /// 2. Creates a new server transaction if no match is found
    /// 3. Notifies the TU about the request based on its method
    ///
    /// # Arguments
    /// * `request` - The incoming SIP request
    /// * `source` - The source address of the request
    ///
    /// # Returns
    /// * `Result<()>` - Success or error depending on request processing outcome
    async fn handle_request(
        &self,
        request: Request,
        source: SocketAddr,
        ingress_context: &SipRequestIngressContext,
    ) -> Result<()> {
        // Try to find a matching transaction
        if let Some(key) = crate::transaction::utils::transaction_key_from_message(
            &Message::Request(request.clone()),
        ) {
            // Check for existing server transaction. Clone the Arc
            // out of the DashMap shard so we don't hold the shard
            // guard across `process_request().await`.
            let existing = self
                .server_transactions
                .get(&key)
                .map(|r| r.value().clone());
            if let Some(transaction) = existing {
                if request.method() == Method::Invite {
                    diagnostics::record_duplicate_invite_existing_transaction();
                } else if request.method() == Method::Bye {
                    diagnostics::record_duplicate_bye_existing_transaction();
                }
                // A rejected request deliberately has no retained principal.
                // Retransmit its last transaction response without invoking
                // the authorizer again or publishing a TU event. This covers
                // both UDP retransmissions and duplicates racing a slow auth
                // provider.
                if self.request_ingress_authorizer().is_some() {
                    if self.peek_inbound_principal(&key).is_some()
                        && self
                            .inbound_principal_for_context(&key, ingress_context)
                            .is_none()
                    {
                        warn!(
                            transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&key),
                            source = %ingress_context.source,
                            "Dropping transaction replay from a different authenticated peer binding"
                        );
                        return Ok(());
                    }
                    if self.peek_inbound_principal(&key).is_none() {
                        let last_response = transaction.data().last_response.lock().await.clone();
                        if let Some(response) = last_response {
                            let wire_bytes =
                                bytes::Bytes::from(Message::Response(response.clone()).to_bytes());
                            self.send_cached_response(
                                response,
                                wire_bytes,
                                ingress_context.response_route(),
                                "Failed to retransmit listener authorization response",
                            )
                            .await
                            .map_err(|error| {
                                Error::transport_error(
                                    error,
                                    "Failed to retransmit listener authorization response",
                                )
                            })?;
                        }
                        return Ok(());
                    }
                }
                let lifecycle = transaction.data().get_lifecycle();
                if !matches!(lifecycle, TransactionLifecycle::Active) {
                    if request.method() == Method::Invite {
                        if self
                            .retransmit_cached_invite_2xx_response_on_route(
                                &key,
                                ingress_context.response_route(),
                            )
                            .await?
                        {
                            return Ok(());
                        }
                        diagnostics::record_duplicate_invite_cache_miss();
                    }
                    debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&key), ?lifecycle, "Skipping request processing for non-active transaction");
                    return Ok(());
                }
                if request.method() == Method::Invite
                    && transaction.state() == TransactionState::Terminated
                {
                    if self
                        .retransmit_cached_invite_2xx_response_on_route(
                            &key,
                            ingress_context.response_route(),
                        )
                        .await?
                    {
                        return Ok(());
                    }
                    diagnostics::record_duplicate_invite_cache_miss();
                }
                let dispatch_started = diagnostics::transaction_timing_enabled().then(Instant::now);
                let result = transaction.process_request(request).await;
                if let Some(started) = dispatch_started {
                    diagnostics::record_existing_transaction_dispatch(started.elapsed());
                }
                return result;
            }

            if request.method() == Method::Invite
                && self
                    .retransmit_cached_invite_2xx_response_on_route(
                        &key,
                        ingress_context.response_route(),
                    )
                    .await?
            {
                return Ok(());
            }
        }

        // ACK and CANCEL are not independently challenged. A matching CANCEL
        // inherits the principal of the INVITE transaction it terminates.
        // When listener authorization is enabled, an unmatched CANCEL is
        // rejected with 481 and never reaches the TU.
        let inherited_cancel_principal = if request.method() == Method::Cancel
            && self.request_ingress_authorizer().is_some()
        {
            crate::transaction::utils::transaction_key_from_message(&Message::Request(
                request.clone(),
            ))
            .map(|key| key.with_method(Method::Invite))
            .and_then(|invite_key| self.inbound_principal_for_context(&invite_key, ingress_context))
        } else {
            None
        };

        // Reject an unmatched or differently bound CANCEL before allocating a
        // server transaction. Otherwise an attacker that guesses the INVITE
        // branch can create the CANCEL transaction first and poison the key,
        // preventing the legitimately bound peer from cancelling the call.
        if request.method() == Method::Cancel
            && self.request_ingress_authorizer().is_some()
            && inherited_cancel_principal.is_none()
        {
            handle_stray_cancel(request, ingress_context.response_route(), &self.transport).await?;
            return Ok(());
        }

        // No existing transaction found, create a new one
        let create_started = diagnostics::transaction_timing_enabled().then(Instant::now);
        let transaction = self
            .create_server_transaction_deferred_events_on_route(
                request.clone(),
                ingress_context.response_route(),
            )
            .await?;
        if let Some(started) = create_started {
            diagnostics::record_server_transaction_create(started.elapsed());
        }

        if !self
            .enforce_ingress_authorization(
                &transaction,
                &request,
                ingress_context,
                inherited_cancel_principal,
            )
            .await?
        {
            return Ok(());
        }

        // Notify the transaction user about the new transaction
        match transaction.kind() {
            TransactionKind::InviteServer => {
                send_transaction_event(
                    &self.events_tx,
                    crate::transaction::TransactionEvent::InviteRequest {
                        transaction_id: transaction.id().clone(),
                        request,
                        source,
                    },
                )
                .await
                .ok();
            }
            TransactionKind::NonInviteServer => {
                // For non-INVITE requests, notify based on the method
                match request.method() {
                    Method::Cancel => {
                        let invite_tx_id = transaction.id().with_method(Method::Invite);
                        if self.server_transactions.contains_key(&invite_tx_id)
                            || self.client_transactions.contains_key(&invite_tx_id)
                        {
                            send_transaction_event(
                                &self.events_tx,
                                crate::transaction::TransactionEvent::CancelRequest {
                                    transaction_id: transaction.id().clone(),
                                    target_transaction_id: invite_tx_id,
                                    request,
                                    source,
                                },
                            )
                            .await
                            .ok();
                        } else {
                            send_transaction_event(
                                &self.events_tx,
                                crate::transaction::TransactionEvent::NonInviteRequest {
                                    transaction_id: transaction.id().clone(),
                                    request,
                                    source,
                                },
                            )
                            .await
                            .ok();
                        }
                    }
                    _ => {
                        send_transaction_event(
                            &self.events_tx,
                            crate::transaction::TransactionEvent::NonInviteRequest {
                                transaction_id: transaction.id().clone(),
                                request,
                                source,
                            },
                        )
                        .await
                        .ok();
                    }
                }
            }
            // Client transaction kinds shouldn't occur here, but handle them for completeness
            TransactionKind::InviteClient | TransactionKind::NonInviteClient => {
                warn!("Unexpected client transaction kind in handle_request");
            }
        }

        Ok(())
    }

    /// Handle an incoming SIP response according to RFC 3261 transaction rules.
    ///
    /// This method:
    /// 1. Attempts to match the response to an existing client transaction
    /// 2. Delivers the response to the matched transaction
    /// 3. Generates a "stray response" event if no match is found
    ///
    /// # Arguments
    /// * `response` - The incoming SIP response
    /// * `source` - The source address of the response
    ///
    /// # Returns
    /// * `Result<()>` - Success or error depending on response processing
    async fn handle_response(&self, response: Response, source: SocketAddr) -> Result<()> {
        // Debug logging for response processing
        debug!(
            "🔍 RESPONSE HANDLER: Processing response {} from {}",
            response.status(),
            source
        );

        // Try to find a matching client transaction
        if let Some(key) = crate::transaction::utils::transaction_key_from_message(
            &Message::Response(response.clone()),
        ) {
            debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&key), "🔍 RESPONSE HANDLER: Generated transaction key from response");

            // Debug the current client transactions (gated on debug level)
            if tracing::enabled!(tracing::Level::DEBUG) {
                let client_keys: Vec<String> = self
                    .client_transactions
                    .iter()
                    .map(|r| {
                        crate::transaction::safe_diagnostics::SafeTransactionKey::new(r.key())
                            .to_string()
                    })
                    .collect();
                debug!(
                    "🔍 RESPONSE HANDLER: Current client transactions: {:?}",
                    client_keys
                );
            }

            debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&key), "Found matching transaction for response");

            if key.is_server() {
                return Err(Error::Other(format!(
                    "Received response but matching transaction key {} is for a server transaction",
                    key
                )));
            }

            // Clone the Arc<dyn ClientTransaction> out of the DashMap
            // shard. No outer guard is held across `process_response`.
            // The transaction stays in the map; we just hold an Arc.
            let tx_arc = self
                .client_transactions
                .get(&key)
                .map(|r| r.value().clone());
            let mut processed = false;

            if let Some(transaction) = tx_arc {
                debug!(
                    "🔍 RESPONSE HANDLER: Found matching client transaction, processing response"
                );

                let lifecycle = transaction.data().get_lifecycle();
                if !matches!(lifecycle, TransactionLifecycle::Active) {
                    debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&key), ?lifecycle, "Skipping response processing for non-active transaction");
                    // Preserve the historical suppression of every retired
                    // response except INVITE 2xx. A late/forked INVITE 2xx
                    // must reach the TU for ACK/cleanup, while replaying a
                    // retired provisional or failure response would repeat
                    // application state transitions.
                    processed =
                        !(key.method() == &Method::Invite && response.status().is_success());
                } else {
                    let dispatch_started =
                        diagnostics::transaction_timing_enabled().then(Instant::now);
                    let process_result = transaction.process_response(response.clone()).await;
                    if let Some(started) = dispatch_started {
                        diagnostics::record_existing_transaction_dispatch(started.elapsed());
                    }
                    if let Err(e) = process_result {
                        warn!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&key), error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "Error processing response");
                    } else {
                        debug!(
                            "🔍 RESPONSE HANDLER: Successfully processed response in transaction"
                        );
                        processed = true;
                    }
                }
            } else {
                debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&key), "No matching client transaction found for response key");
            }

            // If not processed via transaction, still send the event
            if !processed {
                debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&key), "Response matches key but no active transaction found");

                // Deliver to the transaction user anyway
                let status = response.status();
                if key.method() == &Method::Invite && status.is_success() {
                    // Special handling for 2xx responses to INVITE
                    send_transaction_event(
                        &self.events_tx,
                        crate::transaction::TransactionEvent::SuccessResponse {
                            transaction_id: key,
                            response,
                            need_ack: true,
                            source,
                        },
                    )
                    .await
                    .ok();
                } else {
                    // All other responses - classify by status code
                    let status_code = response.status_code();
                    if status_code >= 100 && status_code < 200 {
                        // 1xx provisional response
                        send_transaction_event(
                            &self.events_tx,
                            crate::transaction::TransactionEvent::ProvisionalResponse {
                                transaction_id: key,
                                response,
                            },
                        )
                        .await
                        .ok();
                    } else if status.is_success() && key.method() != &Method::Invite {
                        // 2xx success response for non-INVITE
                        send_transaction_event(
                            &self.events_tx,
                            crate::transaction::TransactionEvent::SuccessResponse {
                                transaction_id: key,
                                response,
                                need_ack: false,
                                source,
                            },
                        )
                        .await
                        .ok();
                    } else {
                        // 3xx, 4xx, 5xx, 6xx failure response
                        send_transaction_event(
                            &self.events_tx,
                            crate::transaction::TransactionEvent::FailureResponse {
                                transaction_id: key,
                                response,
                            },
                        )
                        .await
                        .ok();
                    }
                }
            }

            return Ok(());
        } else {
            debug!("🔍 RESPONSE HANDLER: Could not generate transaction key from response");
        }

        // No transaction match
        debug!("No matching transaction found for response");

        // This could be a response for a transaction that has already terminated
        // or a response forwarded by another SIP entity (for proxy scenarios)
        // In any case, deliver it to the transaction user
        send_transaction_event(
            &self.events_tx,
            crate::transaction::TransactionEvent::StrayResponse { response, source },
        )
        .await
        .ok();

        Ok(())
    }

    /// Handle an ACK request with RFC 3261 compliant dialog-based matching.
    ///
    /// ACK is a special method in SIP:
    /// - ACK for non-2xx responses is part of the INVITE transaction (same branch)
    /// - ACK for 2xx responses is a separate end-to-end transaction (different branch)
    ///
    /// This method uses dialog-based matching (Call-ID, From tag, To tag) as required
    /// by RFC 3261 Section 17.1.1.3 for proper 2xx ACK handling.
    ///
    /// # Arguments
    /// * `request` - The ACK request
    /// * `source` - The source address of the request
    ///
    /// # Returns
    /// * `Result<()>` - Success or error depending on ACK processing
    async fn handle_ack_request(
        &self,
        request: Request,
        source: SocketAddr,
        ingress_context: &SipRequestIngressContext,
    ) -> Result<()> {
        debug!("Processing ACK request with dialog-based matching");

        // First try direct branch-based matching for non-2xx ACKs
        if let Some(key) = crate::transaction::utils::transaction_key_from_message(
            &Message::Request(request.clone()),
        ) {
            let invite_key = key.with_method(Method::Invite);

            let invite_tx = self
                .server_transactions
                .get(&invite_key)
                .map(|r| r.value().clone());
            if let Some(transaction) = invite_tx {
                if transaction.state() != TransactionState::Confirmed {
                    if self.request_ingress_authorizer().is_some()
                        && self
                            .inbound_principal_for_context(&invite_key, ingress_context)
                            .is_none()
                    {
                        warn!(
                            transaction_id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&invite_key),
                            source = %source,
                            "Dropping non-2xx ACK from an unauthorized transport peer"
                        );
                        return Ok(());
                    }
                    let lifecycle = transaction.data().get_lifecycle();
                    if !matches!(lifecycle, TransactionLifecycle::Active) {
                        debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&invite_key), ?lifecycle, "Skipping ACK processing for non-active transaction");
                        return Ok(());
                    }
                    debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&invite_key), "Processing ACK for non-2xx response");
                    self.mark_invite_2xx_response_cache_acked(&invite_key);
                    let dispatch_started =
                        diagnostics::transaction_timing_enabled().then(Instant::now);
                    let result = transaction.process_request(request).await;
                    if let Some(started) = dispatch_started {
                        diagnostics::record_existing_transaction_dispatch(started.elapsed());
                    }
                    return result;
                }
            }
        }

        // RFC 3261 Section 17.1.1.3: For 2xx responses, ACK has different branch.
        // Use dialog-based matching (Call-ID, From tag, To tag) through the
        // server INVITE dialog index.
        if let Some(tx_id) = self.find_server_invite_for_ack(&request) {
            if self.request_ingress_authorizer().is_some()
                && self
                    .inbound_principal_for_context(&tx_id, ingress_context)
                    .is_none()
            {
                warn!(
                    transaction_id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id),
                    source = %source,
                    "Dropping 2xx ACK from an unauthorized transport peer"
                );
                return Ok(());
            }
            debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), "Found ACK for 2xx response using dialog-based matching");
            self.mark_invite_2xx_response_cache_acked(&tx_id);

            // RFC 3261: ACK for 2xx responses should NOT be processed in the transaction
            // Instead, emit AckReceived event for dialog-core to handle
            send_transaction_event(
                &self.events_tx,
                crate::transaction::TransactionEvent::AckReceived {
                    transaction_id: tx_id,
                    request,
                },
            )
            .await
            .map_err(|e| Error::Other(format!("Failed to emit AckReceived event: {}", e)))?;

            debug!("Emitted AckReceived event for dialog-core to handle 2xx ACK");
            return Ok(());
        }

        // No matching INVITE transaction found, this is a stray ACK
        debug!("No matching INVITE transaction found for ACK request");

        if self.request_ingress_authorizer().is_some() {
            warn!(source = %source, "Dropping stray ACK while listener authorization is enabled");
            return Ok(());
        }

        // Notify the transaction user about the stray ACK
        send_transaction_event(
            &self.events_tx,
            crate::transaction::TransactionEvent::StrayAckRequest { request, source },
        )
        .await
        .ok();

        Ok(())
    }

    pub(crate) fn find_server_invite_for_ack(&self, request: &Request) -> Option<TransactionKey> {
        let (exact_key, fallback_key) = ServerInviteDialogKey::ack_lookup_keys(request)?;

        if let Some(entry) = self.lookup_server_invite_by_dialog_key(&exact_key) {
            debug!(
                call_id_len = exact_key.call_id.len(),
                "Found matching INVITE server transaction for ACK by dialog index"
            );
            let transaction_id = entry.transaction_id;
            self.mark_invite_2xx_response_cache_acked(&transaction_id);
            return Some(transaction_id);
        }

        if let Some(fallback_key) = fallback_key.as_ref() {
            if let Some(entry) = self.lookup_server_invite_by_dialog_key(fallback_key) {
                debug!(
                    call_id_len = fallback_key.call_id.len(),
                    "Found matching INVITE server transaction for ACK by dialog index fallback"
                );
                let transaction_id = entry.transaction_id.clone();
                self.insert_server_invite_dialog_index_entry(exact_key, entry);
                self.mark_invite_2xx_response_cache_acked(&transaction_id);
                return Some(transaction_id);
            }
        }

        debug!(
            call_id_len = exact_key.call_id.len(),
            "No matching INVITE server transaction for ACK in dialog index"
        );
        None
    }

    fn lookup_server_invite_by_dialog_key(
        &self,
        dialog_key: &ServerInviteDialogKey,
    ) -> Option<ServerInviteAckIndexEntry> {
        let entry = self
            .server_invite_dialog_index
            .get(dialog_key)
            .map(|entry| entry.value().clone())?;

        if entry.is_expired(std::time::Instant::now()) {
            self.server_invite_dialog_index.remove(dialog_key);
            None
        } else {
            Some(entry)
        }
    }
}

async fn send_transaction_event(
    events_tx: &mpsc::Sender<TransactionEvent>,
    event: TransactionEvent,
) -> std::result::Result<(), mpsc::error::SendError<TransactionEvent>> {
    let started = diagnostics::transaction_timing_enabled().then(Instant::now);
    let result = events_tx.send(event).await;
    if let Some(started) = started {
        diagnostics::record_transaction_event_broadcast(started.elapsed());
    }
    result
}

/// Helper function to handle stray CANCEL requests. Retained for the
/// upcoming stray-CANCEL dispatcher; today the manager handles them
/// inline via the matched-server-transaction path.
#[allow(dead_code)]
async fn handle_stray_cancel(
    request: Request,
    response_route: rvoip_sip_transport::TransportRoute,
    transport: &Arc<dyn Transport>,
) -> Result<()> {
    // Send 481 Transaction Does Not Exist
    let mut builder = ResponseBuilder::new(StatusCode::CallOrTransactionDoesNotExist, None);

    // Add necessary headers
    if let Some(to) = request.to() {
        builder = builder.header(TypedHeader::To(to.clone()));
    }

    if let Some(from) = request.from() {
        builder = builder.header(TypedHeader::From(from.clone()));
    }

    if let Some(call_id) = request.call_id() {
        builder = builder.header(TypedHeader::CallId(call_id.clone()));
    }

    if let Some(cseq) = request.cseq() {
        builder = builder.header(TypedHeader::CSeq(cseq.clone()));
    }

    if let Some(via) = request.header(&HeaderName::Via) {
        builder = builder.header(via.clone());
    }

    // Build the response
    let cancel_response = builder.build();

    // Send the response
    if let Err(e) = transport
        .send_message_via(Message::Response(cancel_response), response_route)
        .await
    {
        return Err(Error::transport_error(
            e,
            "Failed to send 481 response to stray CANCEL",
        ));
    }

    Ok(())
}
