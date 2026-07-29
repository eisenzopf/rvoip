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
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use rvoip_infra_common::events::cross_crate::SipTraceDirection;
use rvoip_sip_core::prelude::*;
use rvoip_sip_transport::transport::TransportType;
use rvoip_sip_transport::{
    Transport, TransportEvent, TransportFlowId, TransportReceiveTiming, TransportRoute,
};
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::diagnostics;
use crate::transaction::error::{Error, Result};
use crate::transaction::runner::HasLifecycle;
use crate::transaction::server::ServerTransaction;
use crate::transaction::state::TransactionLifecycle;
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

fn server_request_route_matches(
    expected_source: SocketAddr,
    expected_response_route: &TransportRoute,
    ingress_context: &SipRequestIngressContext,
) -> bool {
    let actual = ingress_context.response_route();
    expected_source == actual.destination
        && expected_response_route.transport_type == actual.transport_type
        && expected_response_route.flow_id == actual.flow_id
}

fn top_via_sent_by_matches(invite: &Request, cancel: &Request) -> bool {
    super::NormalizedViaSentBy::from_request(invite)
        == super::NormalizedViaSentBy::from_request(cancel)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientResponseRouteAuthentication {
    Authenticated,
    UnknownTransaction,
    Rejected,
}

/// Exact rollback guard for the interval after a server transaction becomes
/// visible but before authorization/rejection and TU handoff establish its
/// durable owner. Its synchronous Drop path is cancellation-safe.
struct StagedServerPublicationGuard {
    manager: TransactionManager,
    transaction: Arc<dyn ServerTransaction>,
    match_reservation: Option<super::ServerTransactionMatchReservation>,
    committed: bool,
}

impl StagedServerPublicationGuard {
    fn new(
        manager: &TransactionManager,
        transaction: Arc<dyn ServerTransaction>,
        match_reservation: Option<super::ServerTransactionMatchReservation>,
    ) -> Self {
        Self {
            manager: manager.clone(),
            transaction,
            match_reservation,
            committed: false,
        }
    }

    fn commit(mut self) {
        if let Some(reservation) = self.match_reservation.take() {
            reservation.commit();
        }
        self.committed = true;
    }
}

impl Drop for StagedServerPublicationGuard {
    fn drop(&mut self) {
        if !self.committed {
            self.manager
                .rollback_failed_server_initialization(&self.transaction);
        }
    }
}

impl TransactionManager {
    async fn registered_server_transaction_id(
        &self,
        match_key: &super::ServerTransactionMatchKey,
    ) -> Option<TransactionKey> {
        let registration = self.server_transaction_matches.registration(match_key)?;
        registration
            .wait_published()
            .await
            .then(|| registration.transaction_id.clone())
    }

    async fn legacy_server_transaction_matches(
        &self,
        transaction_id: &TransactionKey,
        request: &Request,
        ingress_context: &SipRequestIngressContext,
    ) -> bool {
        let route_matches =
            self.server_transactions
                .get(transaction_id)
                .is_some_and(|transaction| {
                    let data = transaction.value().data();
                    server_request_route_matches(
                        data.remote_addr,
                        &data.response_route,
                        ingress_context,
                    )
                });
        if !route_matches {
            return false;
        }
        match self.original_request(transaction_id).await {
            Ok(Some(original)) => top_via_sent_by_matches(&original, request),
            Ok(None) | Err(_) => false,
        }
    }

    async fn matching_server_transaction_id(
        &self,
        match_key: &super::ServerTransactionMatchKey,
        wire_key: &TransactionKey,
        request: &Request,
        ingress_context: &SipRequestIngressContext,
    ) -> Option<TransactionKey> {
        if let Some(transaction_id) = self.registered_server_transaction_id(match_key).await {
            return Some(transaction_id);
        }
        self.legacy_server_transaction_matches(wire_key, request, ingress_context)
            .await
            .then(|| wire_key.clone())
    }

    async fn authorize_unmatched_cancel_without_challenge(
        &self,
        request: &Request,
        ingress_context: &SipRequestIngressContext,
    ) -> bool {
        let Some(authorizer) = self.request_ingress_authorizer() else {
            return true;
        };
        match authorizer.authorize(request, ingress_context).await {
            SipRequestAuthorization::Authorized { .. } => true,
            SipRequestAuthorization::Rejected(rejection) => {
                warn!(
                    method=%crate::transaction::safe_diagnostics::SafeMethod::new(&request.method()),
                    source=%ingress_context.source,
                    status=%rejection.status.as_u16(),
                    "Dropping unmatched proxy CANCEL denied by listener admission"
                );
                false
            }
        }
    }

    fn cache_new_server_ingress(
        &self,
        transaction_id: &TransactionKey,
        request: &Request,
        ingress_context: &SipRequestIngressContext,
        raw_bytes: Option<bytes::Bytes>,
        timing: Option<TransportReceiveTiming>,
    ) {
        if !matches!(request.method(), Method::Ack | Method::Bye) {
            if let Some(value) = raw_bytes {
                self.pending_inbound_inserted_at
                    .insert(transaction_id.clone(), Instant::now());
                self.pending_inbound_bytes
                    .insert(transaction_id.clone(), value);
            }
        }
        self.pending_inbound_transport.insert(
            transaction_id.clone(),
            rvoip_infra_common::events::cross_crate::SipTransportContext::new(
                ingress_context.transport_type.to_string(),
                ingress_context.destination.to_string(),
                ingress_context.source.to_string(),
                matches!(
                    ingress_context.transport_type,
                    TransportType::Tls | TransportType::Wss
                ),
            ),
        );
        if matches!(request.method(), Method::Invite | Method::Bye) {
            if let Some(value) = timing {
                self.pending_inbound_timing
                    .insert(transaction_id.clone(), value);
            }
        }
    }

    async fn dispatch_matching_server_request(
        &self,
        transaction_id: &TransactionKey,
        request: Request,
        ingress_context: &SipRequestIngressContext,
    ) -> Result<bool> {
        let existing = self
            .server_transactions
            .get(transaction_id)
            .map(|entry| entry.value().clone());
        if let Some(transaction) = existing {
            if request.method() == Method::Invite {
                diagnostics::record_duplicate_invite_existing_transaction();
            } else if request.method() == Method::Bye {
                diagnostics::record_duplicate_bye_existing_transaction();
            }
            if self.request_ingress_authorizer().is_some() {
                if self.peek_inbound_principal(transaction_id).is_some()
                    && self
                        .inbound_principal_for_context(transaction_id, ingress_context)
                        .is_none()
                {
                    warn!(
                        transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id),
                        source=%ingress_context.source,
                        "Dropping transaction replay from a different authenticated peer binding"
                    );
                    return Ok(true);
                }
                if self.peek_inbound_principal(transaction_id).is_none() {
                    let last_response = transaction.data().last_response.lock().await.clone();
                    if let Some(response) = last_response {
                        let wire_bytes =
                            bytes::Bytes::from(Message::Response(response.clone()).to_bytes());
                        self.send_cached_response(
                            response,
                            wire_bytes,
                            ingress_context.response_route_for_request(&request),
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
                    return Ok(true);
                }
            }

            let lifecycle = transaction.data().get_lifecycle();
            if !matches!(lifecycle, TransactionLifecycle::Active) {
                if request.method() == Method::Invite {
                    if self
                        .retransmit_cached_invite_2xx_response_on_route(
                            transaction_id,
                            ingress_context.response_route_for_request(&request),
                        )
                        .await?
                    {
                        return Ok(true);
                    }
                    diagnostics::record_duplicate_invite_cache_miss();
                }
                debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id), ?lifecycle, "Skipping request processing for non-active transaction");
                return Ok(true);
            }
            if request.method() == Method::Invite
                && transaction.state() == TransactionState::Terminated
            {
                if self
                    .retransmit_cached_invite_2xx_response_on_route(
                        transaction_id,
                        ingress_context.response_route_for_request(&request),
                    )
                    .await?
                {
                    return Ok(true);
                }
                diagnostics::record_duplicate_invite_cache_miss();
            }
            let dispatch_started = diagnostics::transaction_timing_enabled().then(Instant::now);
            transaction.process_request(request).await?;
            if let Some(started) = dispatch_started {
                diagnostics::record_existing_transaction_dispatch(started.elapsed());
            }
            return Ok(true);
        }

        if request.method() != Method::Invite
            && self
                .replay_compact_non_invite_server_response(
                    transaction_id,
                    &request,
                    ingress_context,
                )
                .await?
        {
            return Ok(true);
        }
        if request.method() == Method::Invite
            && self
                .retransmit_cached_invite_2xx_response_on_route(
                    transaction_id,
                    ingress_context.response_route_for_request(&request),
                )
                .await?
        {
            return Ok(true);
        }
        Ok(false)
    }

    fn absorb_compact_non_invite_client_response(&self, transaction_id: &TransactionKey) -> bool {
        self.compact_non_invite_tombstones
            .get(transaction_id)
            .is_some_and(|entry| entry.value().is_client())
    }

    pub(super) async fn replay_compact_non_invite_server_response(
        &self,
        transaction_id: &TransactionKey,
        request: &Request,
        ingress_context: &SipRequestIngressContext,
    ) -> Result<bool> {
        let replay = self
            .compact_non_invite_tombstones
            .get(transaction_id)
            .and_then(|entry| {
                let tombstone = entry.value();
                let (wire, route) = tombstone.server_replay()?;
                Some((
                    wire.clone(),
                    route.clone(),
                    tombstone.expires_at(),
                    tombstone.generation(),
                ))
            });
        let Some((wire, route, expires_at, generation)) = replay else {
            return Ok(false);
        };

        if expires_at <= Instant::now() {
            // The lifecycle scheduler owns the tombstone, route/auth indexes,
            // terminal state, public event ordering, and dialog ACK fence as
            // one exact-generation cleanup. A retransmission at the expiry
            // boundary may only expedite that owner; it must never delete the
            // map entry directly and leave the derived indexes/events behind.
            if let Some(scheduler) = self.lifecycle_scheduler.as_ref() {
                scheduler.request_compact_expiry(transaction_id.clone(), generation);
            }
            // The request matched an authentic retained generation. Absorb it
            // while the scheduler completes expiry instead of falling through
            // to new-transaction creation (which would only hit the fence).
            return Ok(true);
        }

        let expected_transport = route.transport_type.unwrap_or(TransportType::Udp);
        let peer_route = ingress_context.response_route();
        let response_route = ingress_context.response_route_for_request(request);
        let ingress_transport = peer_route.transport_type.unwrap_or(TransportType::Udp);
        if expected_transport != TransportType::Udp
            || ingress_transport != TransportType::Udp
            || route.destination != response_route.destination
            || peer_route.flow_id.is_some()
        {
            warn!(
                transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id),
                "Dropping compact non-INVITE retransmission from a different transport peer"
            );
            return Ok(true);
        }

        if self.request_ingress_authorizer().is_some()
            && self.peek_inbound_principal(transaction_id).is_some()
            && self
                .inbound_principal_for_context(transaction_id, ingress_context)
                .is_none()
        {
            warn!(
                transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(transaction_id),
                "Dropping compact non-INVITE retransmission from an unauthorized peer"
            );
            return Ok(true);
        }

        self.transport
            .send_message_raw_via(wire, route)
            .await
            .map_err(|error| {
                Error::transport_error(error, "Failed to replay compact non-INVITE server response")
            })?;
        Ok(true)
    }

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
        let Some(mut state) = self.transaction_destinations.get_mut(transaction_id) else {
            return ClientResponseRouteAuthentication::UnknownTransaction;
        };

        let expected = match state.value() {
            super::ClientResponseRouteState::Active { route, .. } => route.clone(),
            super::ClientResponseRouteState::Retired(retired) => {
                if retired.expires_at <= Instant::now() {
                    let expires_at = retired.expires_at;
                    let deadline_version = retired.deadline_version;
                    drop(state);
                    if self
                        .transaction_destinations
                        .remove_if(transaction_id, |_, current| {
                            current.retired().is_some_and(|retired| {
                                retired.deadline_version == deadline_version
                                    && retired.expires_at == expires_at
                                    && retired.expires_at <= Instant::now()
                            })
                        })
                        .is_some()
                    {
                        self.decrement_retired_client_transaction_count();
                        self.unschedule_retired_client_deadline(
                            transaction_id,
                            expires_at,
                            deadline_version,
                        );
                    }
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
            super::ClientResponseRouteState::Active { route, .. } => *route = bound,
            super::ClientResponseRouteState::Retired(retired) if retired.route != bound => {
                retired.route = bound
            }
            super::ClientResponseRouteState::Retired(_) => {}
        }
        ClientResponseRouteAuthentication::Authenticated
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
        let Some(_dispatch_operation) = self.admission_lifecycle.try_enter_existing() else {
            debug!("Dropping transport event because transaction manager is stopping");
            return Ok(());
        };
        tokio::select! {
            biased;
            _ = self.operation_cancellation.cancelled() => Ok(()),
            result = self.handle_transport_event_inner(event) => result,
        }
    }

    async fn handle_transport_event_inner(&self, event: TransportEvent) -> Result<()> {
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
                            let mut response_route =
                                TransportRoute::new(source).with_transport_type(transport_type);
                            response_route.flow_id = flow_id;
                            if self.stateful_proxy_tu_mode() {
                                self.send_stateful_proxy_ingress_event(
                                    crate::transaction::StatefulProxyIngressEvent::StrayResponse {
                                        response: response.clone(),
                                        source,
                                        response_route: Some(response_route),
                                    },
                                )
                                .await?;
                            }
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
                        Message::Request(_) => false,
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
                        // Requests with transactions are cached only after
                        // successful transaction admission in
                        // `cache_new_server_ingress`. ACK has no standalone
                        // server transaction: a non-2xx ACK belongs to the
                        // INVITE transaction and a 2xx ACK is end-to-end.
                        // Caching it under the ACK wire key would therefore
                        // create metadata with no lifecycle owner.
                        Message::Request(_) => false,
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
                self.handle_message(
                    message,
                    source,
                    destination,
                    &ingress_context,
                    raw_bytes,
                    timing,
                )
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
        raw_bytes: Option<bytes::Bytes>,
        timing: Option<TransportReceiveTiming>,
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

                self.handle_request(request, source, ingress_context, raw_bytes, timing)
                    .await
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
        preauthorized: Option<SipRequestAuthorization>,
    ) -> Result<bool> {
        let Some(authorizer) = self.request_ingress_authorizer() else {
            return Ok(true);
        };

        let decision = match (inherited_principal, preauthorized) {
            (Some(principal), _) => SipRequestAuthorization::Authorized { principal },
            (None, Some(decision)) => decision,
            (None, None) => authorizer.authorize(request, ingress_context).await,
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
                let admission_generation = transaction
                    .data()
                    .transaction_admission_owner()
                    .map(|owner| owner.generation());
                let mut response = match admission_generation {
                    Some(generation) => crate::transaction::utils::response_builders::
                        create_response_for_transaction_generation(
                            request,
                            status,
                            transaction.id(),
                            generation,
                        ),
                    None => {
                        debug_assert!(
                            false,
                            "an admitted server transaction must retain its exact generation"
                        );
                        crate::transaction::utils::response_builders::create_response(
                            request, status,
                        )
                    }
                };
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
        raw_bytes: Option<bytes::Bytes>,
        timing: Option<TransportReceiveTiming>,
    ) -> Result<()> {
        let wire_key = crate::transaction::utils::transaction_key_from_message(&Message::Request(
            request.clone(),
        ));
        let match_key = super::ServerTransactionMatchKey::from_request(&request, ingress_context);

        // An independently completed CANCEL transaction owns its retransmits.
        // Resolve it before proxy unmatched-CANCEL handling so a late duplicate
        // replays the cached 200 and cannot create a second downstream CANCEL.
        if let (Some(wire_key), Some(match_key)) = (&wire_key, &match_key) {
            if let Some(transaction_id) = self
                .matching_server_transaction_id(match_key, wire_key, &request, ingress_context)
                .await
            {
                if self
                    .dispatch_matching_server_request(
                        &transaction_id,
                        request.clone(),
                        ingress_context,
                    )
                    .await?
                {
                    return Ok(());
                }
            }
        }

        // CANCEL is a separate transaction, but it matches its target INVITE
        // with the INVITE method exception from RFC 3261 §§9.2 and 17.2.3.
        let matching_invite = if request.method() == Method::Cancel {
            match (&wire_key, &match_key) {
                (Some(wire_key), Some(match_key)) => {
                    let invite_wire_key = wire_key.with_method(Method::Invite);
                    let invite_match_key = match_key.with_method(Method::Invite);
                    self.matching_server_transaction_id(
                        &invite_match_key,
                        &invite_wire_key,
                        &request,
                        ingress_context,
                    )
                    .await
                    .filter(|transaction_id| {
                        self.request_ingress_authorizer().is_none()
                            || self
                                .inbound_principal_for_context(transaction_id, ingress_context)
                                .is_some()
                    })
                }
                _ => None,
            }
        } else {
            None
        };

        // RFC 3261 §16.10 gives an unmatched proxy CANCEL stateless handling,
        // but listener admission still applies. A denial is dropped silently:
        // the proxy must not challenge the CANCEL or manufacture a UAS 481.
        if request.method() == Method::Cancel
            && self.forward_unmatched_cancel_to_tu()
            && matching_invite.is_none()
        {
            if !self
                .authorize_unmatched_cancel_without_challenge(&request, ingress_context)
                .await
            {
                return Ok(());
            }
            let response_route = ingress_context.response_route_for_request(&request);
            self.send_stateful_proxy_ingress_event(
                crate::transaction::StatefulProxyIngressEvent::UnmatchedCancelRequest {
                    request,
                    source,
                    response_route,
                },
            )
            .await
            .map_err(|error| {
                Error::Other(format!(
                    "failed to publish unmatched CANCEL to exact proxy ingress: {error}"
                ))
            })?;
            return Ok(());
        }

        // ACK and CANCEL are not independently challenged. A matching CANCEL
        // inherits the exact transport-bound principal of its target INVITE.
        let inherited_cancel_principal = matching_invite
            .as_ref()
            .and_then(|invite_key| self.inbound_principal_for_context(invite_key, ingress_context));

        // Reject an unmatched or differently bound CANCEL before allocating a
        // server transaction. Otherwise an attacker that guesses the INVITE
        // branch can create the CANCEL transaction first and poison the key,
        // preventing the legitimately bound peer from cancelling the call.
        if request.method() == Method::Cancel
            && self.request_ingress_authorizer().is_some()
            && inherited_cancel_principal.is_none()
            && !self.forward_unmatched_cancel_to_tu()
        {
            let response_route = ingress_context.response_route_for_request(&request);
            handle_stray_cancel(request, response_route, &self.transport).await?;
            return Ok(());
        }

        // Keep shutdown's in-flight fence continuously held across creation,
        // authorization/rejection and primary TU handoff. The nested create
        // guard protects its own map publication; this outer guard prevents a
        // slow authorizer from resuming after shutdown has force-cleared maps.
        let _staged_operation = self.admission_lifecycle.try_enter().ok_or_else(|| {
            Error::Other("transaction manager is draining; inbound request rejected".into())
        })?;

        let wire_key = wire_key.ok_or_else(|| {
            Error::Other("Missing RFC 3261 transaction key in inbound request".into())
        })?;
        let match_key = match_key.ok_or_else(|| {
            Error::Other("Missing valid top Via sent-by in inbound request".into())
        })?;
        let mut preauthorization = None;
        let authenticated_owner = if let Some(principal) = inherited_cancel_principal.as_ref() {
            Some(super::ServerAuthenticatedOwner::from(principal))
        } else if let Some(authorizer) = self.request_ingress_authorizer() {
            let decision = authorizer.authorize(&request, ingress_context).await;
            let owner = match &decision {
                SipRequestAuthorization::Authorized { principal } => {
                    Some(super::ServerAuthenticatedOwner::from(principal))
                }
                SipRequestAuthorization::Rejected(_) => None,
            };
            preauthorization = Some(decision);
            owner
        } else {
            match_key
                .peer
                .tls_leaf_sha256
                .as_deref()
                .map(super::ServerAuthenticatedOwner::transport_tls)
        };
        let wire_key_is_available = !self.server_transactions.contains_key(&wire_key)
            && !self.compact_non_invite_tombstones.contains_key(&wire_key)
            && !self.transaction_admissions.entries.contains_key(&wire_key);
        let match_reservation = match self.server_transaction_matches.claim(
            match_key,
            &wire_key,
            wire_key_is_available,
            authenticated_owner,
        ) {
            super::ServerTransactionMatchClaim::Reserved(reservation) => reservation,
            super::ServerTransactionMatchClaim::Existing(registration) => {
                let published = registration.wait_published().await;
                drop(_staged_operation);
                if published
                    && self
                        .dispatch_matching_server_request(
                            &registration.transaction_id,
                            request.clone(),
                            ingress_context,
                        )
                        .await?
                {
                    return Ok(());
                }
                return Box::pin(self.handle_request(
                    request,
                    source,
                    ingress_context,
                    raw_bytes,
                    timing,
                ))
                .await;
            }
            super::ServerTransactionMatchClaim::PendingCollision(registrations) => {
                for registration in registrations {
                    let _ = registration.wait_published().await;
                }
                drop(_staged_operation);
                return Box::pin(self.handle_request(
                    request,
                    source,
                    ingress_context,
                    raw_bytes,
                    timing,
                ))
                .await;
            }
            super::ServerTransactionMatchClaim::ConflictingOwner => {
                warn!(
                    method=%crate::transaction::safe_diagnostics::SafeMethod::new(&request.method()),
                    source=%ingress_context.source,
                    "Dropping transaction identity collision without a distinct authenticated owner"
                );
                return Ok(());
            }
        };
        let transaction_id = match_reservation.transaction_id().clone();

        // No existing transaction found, create a new one
        let create_started = diagnostics::transaction_timing_enabled().then(Instant::now);
        let transaction = match self
            .create_server_transaction_deferred_events_on_route_with_id(
                request.clone(),
                ingress_context.source,
                ingress_context.response_route_for_request(&request),
                transaction_id.clone(),
            )
            .await
        {
            Ok(transaction) => transaction,
            Err(Error::TransactionCapacityExhausted { resource, limit }) => {
                warn!(
                    resource,
                    limit, "Rejecting inbound SIP request at transaction retention capacity"
                );
                send_stateless_transaction_overload(
                    &request,
                    ingress_context.response_route_for_request(&request),
                    &self.transport,
                )
                .await?;
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        if let Some(started) = create_started {
            diagnostics::record_server_transaction_create(started.elapsed());
        }
        self.cache_new_server_ingress(
            &transaction_id,
            &request,
            ingress_context,
            raw_bytes,
            timing,
        );
        let publication =
            StagedServerPublicationGuard::new(self, transaction.clone(), Some(match_reservation));

        if !self
            .enforce_ingress_authorization(
                &transaction,
                &request,
                ingress_context,
                inherited_cancel_principal,
                preauthorization,
            )
            .await?
        {
            // A successfully installed transaction response is the durable
            // owner for authorization rejection and future retransmissions.
            publication.commit();
            return Ok(());
        }

        // Notify the transaction user about the new transaction
        let publication_result = match transaction.kind() {
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
            }
            TransactionKind::NonInviteServer => {
                // For non-INVITE requests, notify based on the method
                match request.method() {
                    Method::Cancel => {
                        if let Some(invite_tx_id) = matching_invite {
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
                    }
                }
            }
            // Client transaction kinds shouldn't occur here, but handle them for completeness
            TransactionKind::InviteClient | TransactionKind::NonInviteClient => {
                warn!("Unexpected client transaction kind in handle_request");
                Err(tokio::sync::mpsc::error::SendError(
                    crate::transaction::TransactionEvent::TransactionTerminated {
                        transaction_id: transaction.id().clone(),
                    },
                ))
            }
        };
        publication_result.map_err(|error| {
            Error::Other(format!(
                "failed to publish authenticated inbound transaction to TU: {error}"
            ))
        })?;
        publication.commit();

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
            let mut processed = self.absorb_compact_non_invite_client_response(&key);
            if processed {
                debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&key), "Absorbed retransmitted final response in compact Timer K tombstone");
            }

            if !processed {
                if let Some(transaction) = tx_arc {
                    debug!(
                        "🔍 RESPONSE HANDLER: Found matching client transaction, processing response"
                    );

                    let lifecycle = transaction.data().get_lifecycle();
                    if !matches!(lifecycle, TransactionLifecycle::Active) {
                        debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&key), ?lifecycle, "Skipping response processing for non-active transaction");
                        // UA mode retains its bounded late-2xx safety path.
                        // RFC 6026 proxy mode drops every response after Timer
                        // M destroys the matching client transaction.
                        processed = self.stateful_proxy_tu_mode()
                            || !(key.method() == &Method::Invite && response.status().is_success());
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
            }

            // If not processed via transaction, still send the event
            if !processed {
                debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&key), "Response matches key but no active transaction found");

                // Deliver to the transaction user anyway
                let status = response.status();
                if key.method() == &Method::Invite && status.is_success() {
                    if self.stateful_proxy_tu_mode() {
                        debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&key), "Dropping RFC 6026 stray INVITE response in stateful-proxy mode");
                        return Ok(());
                    }
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
        if self.stateful_proxy_tu_mode() {
            self.send_stateful_proxy_ingress_event(
                crate::transaction::StatefulProxyIngressEvent::StrayResponse {
                    response: response.clone(),
                    source,
                    response_route: None,
                },
            )
            .await?;
        }
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

        // First try direct branch-based matching for non-2xx ACKs. Only a
        // server INVITE in Completed/Confirmed owns such an ACK. Treating an
        // arbitrary same-branch ACK as transaction-owned can consume a 2xx
        // ACK before the retained dialog index gets a chance to classify it.
        if let Some(key) = crate::transaction::utils::transaction_key_from_message(
            &Message::Request(request.clone()),
        ) {
            let invite_key = key.with_method(Method::Invite);

            let invite_tx = self
                .server_transactions
                .get(&invite_key)
                .map(|r| r.value().clone());
            if let Some(transaction) = invite_tx {
                if matches!(
                    transaction.state(),
                    TransactionState::Completed | TransactionState::Confirmed
                ) {
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

            // RFC 3261: ACK for 2xx responses is end-to-end and is not
            // processed by the INVITE transaction. Publish the distinct
            // AckRequest event so a stateful proxy can forward it statelessly
            // while DialogManager can retain its existing UAS media-start
            // behavior.
            send_transaction_event(
                &self.events_tx,
                crate::transaction::TransactionEvent::AckRequest {
                    transaction_id: tx_id,
                    request,
                    source,
                },
            )
            .await
            .map_err(|e| Error::Other(format!("Failed to emit AckRequest event: {}", e)))?;

            debug!("Emitted AckRequest event for dialog-core to handle 2xx ACK");
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
                // Keep the initial INVITE's no-To-tag binding canonical. A
                // successful response adds the local To tag to its ACK, so
                // caching that exact lookup would retain a second equivalent
                // key and retirement deadline for every normal call. Reusing
                // the fallback on duplicate ACKs preserves matching while
                // retaining exactly one binding for the transaction.
                let transaction_id = entry.transaction_id;
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
            // Do not let an expired snapshot race with a replacement binding
            // for the same dialog key. The deadline generation is the exact
            // identity of the observed index entry.
            self.server_invite_dialog_index
                .remove_if(dialog_key, |_, current| {
                    current.deadline_generation == entry.deadline_generation
                        && current.is_expired(std::time::Instant::now())
                });
            None
        } else {
            Some(entry)
        }
    }
}

async fn send_transaction_event(
    events_tx: &crate::transaction::event_sender::TransactionEventSender,
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

/// Reject before server-transaction allocation when bounded Timer J/K
/// retention is saturated. The response is stateless by design: allocating a
/// transaction here would consume the capacity whose absence triggered it.
async fn send_stateless_transaction_overload(
    request: &Request,
    response_route: TransportRoute,
    transport: &Arc<dyn Transport>,
) -> Result<()> {
    // ACK never receives a response. It is normally handled before this path,
    // but retain the RFC rule defensively.
    if request.method() == Method::Ack {
        return Ok(());
    }

    let mut builder = ResponseBuilder::new(StatusCode::ServiceUnavailable, None).header(
        TypedHeader::RetryAfter(rvoip_sip_core::types::retry_after::RetryAfter::new(1)),
    );
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

    transport
        .send_message_via(Message::Response(builder.build()), response_route)
        .await
        .map_err(|error| {
            Error::transport_error(
                error,
                "Failed to send stateless transaction overload response",
            )
        })
}
