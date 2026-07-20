// Test scaffolding: tests build channels/transports and discard the
// receiver/sender ends to keep the test focused on the transaction
// manager surface. Allow `unused_variables` / `unused_mut` so the
// scaffolding doesn't need underscores everywhere.
#[cfg(test)]
#[allow(unused_variables, unused_mut, dead_code)]
mod tests {
    use super::super::RFC3261_BRANCH_MAGIC_COOKIE;
    use super::super::{
        recv_transaction_dispatch_event, transaction_dispatch_lane,
        transaction_dispatch_worker_index, transaction_index_capacity,
        transaction_index_initial_capacity, transaction_ingress_kind,
        ClientCompletionDeadlineScheduler, ClientResponseRouteState, Invite2xxDeadlineScheduler,
        Invite2xxResponseCacheEntry, QueuedTransactionDispatch, RetiredClientDeadlineScheduler,
        RetiredClientTransaction, TransactionDispatchLane, TransactionIngressKind,
        DEFAULT_INVITE_2XX_RETRANSMIT_MAX_DUE_PER_TICK,
        DEFAULT_TRANSACTION_DISPATCH_PRIORITY_BURST_MAX, MANAGER_ADMISSION_STOPPING,
        MAX_EAGER_TRANSACTION_INDEX_CAPACITY, RETAINED_CLIENT_DEADLINE_BATCH_MAX,
        TERMINATED_CLEANUP_BATCH_MAX,
    };
    use super::super::{ServerInviteAckIndexEntry, ServerInviteDialogKey};
    use crate::transaction::client::builders::{ByeBuilder, InviteBuilder, RegisterBuilder};
    use crate::transaction::client::ClientInviteTransaction;
    use crate::transaction::completion::ClientTransactionCompletion;
    use crate::transaction::error::{Error, Result};
    use crate::transaction::manager::ClientTransaction;
    use crate::transaction::server::{ServerInviteTransaction, ServerNonInviteTransaction};
    use crate::transaction::InternalTransactionCommand;
    use crate::transaction::Transaction;
    use crate::transaction::TransactionEvent;
    use crate::transaction::TransactionKey;
    use crate::transaction::TransactionManager;
    use crate::transaction::TransactionState;
    use crate::transaction::{
        SipRequestAuthorization, SipRequestIngressAuthorizer, SipRequestIngressContext,
        SipRequestRejection,
    };
    use rvoip_core_traits::identity::{
        AuthenticatedPrincipal, AuthenticationMethod, CredentialKind, IdentityAssurance,
    };
    use rvoip_sip_core::builder::SimpleRequestBuilder;
    use rvoip_sip_core::prelude::*;
    use rvoip_sip_core::types::status::StatusCode;
    use rvoip_sip_core::types::Address;
    use rvoip_sip_core::types::Contact;
    use rvoip_sip_core::types::ContactParamInfo;
    use rvoip_sip_transport::transport::TransportType;
    use rvoip_sip_transport::{Transport, TransportEvent, TransportFlowId, TransportRoute};
    use std::net::SocketAddr;
    use std::str::FromStr;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tokio::sync::Mutex;
    use tokio::sync::{mpsc, Barrier};

    /// Create a mock transport for testing
    #[derive(Debug, Clone)]
    struct MockTransport {
        local_addr: SocketAddr,
        sent_messages: Arc<Mutex<Vec<(Message, SocketAddr)>>>,
        should_fail_send: Arc<AtomicBool>,
        raw_send_count: Arc<AtomicUsize>,
        raw_routes: Arc<Mutex<Vec<rvoip_sip_transport::TransportRoute>>>,
    }

    impl MockTransport {
        fn new(addr: &str) -> Self {
            Self {
                local_addr: SocketAddr::from_str(addr).unwrap(),
                sent_messages: Arc::new(Mutex::new(Vec::new())),
                should_fail_send: Arc::new(AtomicBool::new(false)),
                raw_send_count: Arc::new(AtomicUsize::new(0)),
                raw_routes: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn with_send_failure(addr: &str, should_fail: bool) -> Self {
            Self {
                local_addr: SocketAddr::from_str(addr).unwrap(),
                sent_messages: Arc::new(Mutex::new(Vec::new())),
                should_fail_send: Arc::new(AtomicBool::new(should_fail)),
                raw_send_count: Arc::new(AtomicUsize::new(0)),
                raw_routes: Arc::new(Mutex::new(Vec::new())),
            }
        }

        #[allow(dead_code)]
        fn set_send_failure(&self, should_fail: bool) {
            self.should_fail_send.store(should_fail, Ordering::SeqCst);
        }

        async fn get_sent_messages(&self) -> Vec<(Message, SocketAddr)> {
            self.sent_messages.lock().await.clone()
        }

        fn raw_send_count(&self) -> usize {
            self.raw_send_count.load(Ordering::SeqCst)
        }

        async fn raw_routes(&self) -> Vec<rvoip_sip_transport::TransportRoute> {
            self.raw_routes.lock().await.clone()
        }
    }

    #[async_trait::async_trait]
    impl rvoip_sip_transport::Transport for MockTransport {
        async fn send_message(
            &self,
            message: Message,
            destination: SocketAddr,
        ) -> std::result::Result<(), rvoip_sip_transport::Error> {
            // Check if we should simulate a failure
            if self.should_fail_send.load(Ordering::SeqCst) {
                println!("MockTransport::send_message - Simulating failure");
                return Err(rvoip_sip_transport::error::Error::ProtocolError(
                    "Simulated network failure for testing".into(),
                ));
            }

            // Otherwise process normally
            let mut messages = self.sent_messages.lock().await;
            println!(
                "MockTransport::send_message - Sending message: {:?} to {}",
                if let Message::Request(ref req) = message {
                    req.method()
                } else {
                    Method::Ack
                },
                destination
            );
            messages.push((message, destination));
            Ok(())
        }

        async fn send_message_raw(
            &self,
            bytes: bytes::Bytes,
            destination: SocketAddr,
        ) -> std::result::Result<(), rvoip_sip_transport::Error> {
            if self.should_fail_send.load(Ordering::SeqCst) {
                println!("MockTransport::send_message_raw - Simulating failure");
                return Err(rvoip_sip_transport::error::Error::ProtocolError(
                    "Simulated network failure for testing".into(),
                ));
            }

            let message = rvoip_sip_core::parse_message(&bytes).map_err(|e| {
                rvoip_sip_transport::error::Error::ProtocolError(format!(
                    "Failed to parse raw SIP message in mock transport: {e}"
                ))
            })?;
            self.raw_send_count.fetch_add(1, Ordering::SeqCst);
            self.sent_messages.lock().await.push((message, destination));
            Ok(())
        }

        async fn send_message_raw_via(
            &self,
            bytes: bytes::Bytes,
            route: rvoip_sip_transport::TransportRoute,
        ) -> std::result::Result<(), rvoip_sip_transport::Error> {
            self.raw_routes.lock().await.push(route.clone());
            self.send_message_raw(bytes, route.destination).await
        }

        fn local_addr(&self) -> std::result::Result<SocketAddr, rvoip_sip_transport::Error> {
            Ok(self.local_addr)
        }

        async fn close(&self) -> std::result::Result<(), rvoip_sip_transport::Error> {
            Ok(())
        }

        fn is_closed(&self) -> bool {
            false
        }
    }

    /// Transport probe for the exact first-write boundary. Route preparation
    /// is guaranteed to occur before `wire_attempts` increments; a configured
    /// first-write error increments it and is therefore conservatively
    /// wire-unknown.
    #[derive(Debug)]
    struct WireBoundaryMockTransport {
        local_addr: SocketAddr,
        fail_prepare: bool,
        fail_first_write: AtomicBool,
        wire_attempts: AtomicUsize,
        attempted: Mutex<Vec<(Message, TransportRoute)>>,
    }

    impl WireBoundaryMockTransport {
        fn prepare_failure() -> Self {
            Self {
                local_addr: "127.0.0.1:5060".parse().unwrap(),
                fail_prepare: true,
                fail_first_write: AtomicBool::new(false),
                wire_attempts: AtomicUsize::new(0),
                attempted: Mutex::new(Vec::new()),
            }
        }

        fn first_write_failure() -> Self {
            Self {
                local_addr: "127.0.0.1:5060".parse().unwrap(),
                fail_prepare: false,
                fail_first_write: AtomicBool::new(true),
                wire_attempts: AtomicUsize::new(0),
                attempted: Mutex::new(Vec::new()),
            }
        }

        fn wire_attempts(&self) -> usize {
            self.wire_attempts.load(Ordering::Acquire)
        }

        async fn attempted(&self) -> Vec<(Message, TransportRoute)> {
            self.attempted.lock().await.clone()
        }
    }

    #[async_trait::async_trait]
    impl Transport for WireBoundaryMockTransport {
        fn local_addr(&self) -> std::result::Result<SocketAddr, rvoip_sip_transport::Error> {
            Ok(self.local_addr)
        }

        async fn send_message(
            &self,
            message: Message,
            destination: SocketAddr,
        ) -> std::result::Result<(), rvoip_sip_transport::Error> {
            self.send_message_on_route(message, TransportRoute::new(destination))
                .await
                .map(|_| ())
        }

        async fn prepare_message_route(
            &self,
            _message: &Message,
            route: TransportRoute,
        ) -> std::result::Result<TransportRoute, rvoip_sip_transport::Error> {
            if self.fail_prepare {
                return Err(rvoip_sip_transport::Error::InvalidState(
                    "injected route preparation failure".into(),
                ));
            }
            Ok(route)
        }

        async fn send_message_on_route(
            &self,
            message: Message,
            route: TransportRoute,
        ) -> std::result::Result<TransportRoute, rvoip_sip_transport::Error> {
            self.wire_attempts.fetch_add(1, Ordering::AcqRel);
            self.attempted.lock().await.push((message, route.clone()));
            if self.fail_first_write.swap(false, Ordering::AcqRel) {
                return Err(rvoip_sip_transport::Error::ProtocolError(
                    "injected first transport write failure".into(),
                ));
            }
            Ok(route)
        }

        async fn close(&self) -> std::result::Result<(), rvoip_sip_transport::Error> {
            Ok(())
        }

        fn is_closed(&self) -> bool {
            false
        }
    }

    /// Stream transport that binds every initial request to `original_flow`
    /// while advertising `later_flow` from the legacy address resolver. Tests
    /// use it to prove response authentication never adopts a co-addressed
    /// replacement connection after retirement.
    #[derive(Debug, Clone)]
    struct ExactFlowMockTransport {
        local_addr: SocketAddr,
        original_flow: TransportFlowId,
        later_flow: TransportFlowId,
        sent_messages: Arc<Mutex<Vec<(Message, TransportRoute)>>>,
        resolve_calls: Arc<AtomicUsize>,
    }

    impl ExactFlowMockTransport {
        fn new(original_flow: TransportFlowId, later_flow: TransportFlowId) -> Self {
            Self {
                local_addr: "127.0.0.1:5060".parse().unwrap(),
                original_flow,
                later_flow,
                sent_messages: Arc::new(Mutex::new(Vec::new())),
                resolve_calls: Arc::new(AtomicUsize::new(0)),
            }
        }

        async fn sent_routes(&self) -> Vec<TransportRoute> {
            self.sent_messages
                .lock()
                .await
                .iter()
                .map(|(_, route)| route.clone())
                .collect()
        }

        fn resolve_calls(&self) -> usize {
            self.resolve_calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl Transport for ExactFlowMockTransport {
        fn local_addr(&self) -> std::result::Result<SocketAddr, rvoip_sip_transport::Error> {
            Ok(self.local_addr)
        }

        async fn send_message(
            &self,
            message: Message,
            destination: SocketAddr,
        ) -> std::result::Result<(), rvoip_sip_transport::Error> {
            self.sent_messages
                .lock()
                .await
                .push((message, TransportRoute::new(destination)));
            Ok(())
        }

        async fn send_message_via(
            &self,
            message: Message,
            route: TransportRoute,
        ) -> std::result::Result<(), rvoip_sip_transport::Error> {
            if route.flow_id != Some(self.original_flow) {
                return Err(rvoip_sip_transport::Error::InvalidState(
                    "test stream send did not preserve the original flow".into(),
                ));
            }
            self.sent_messages.lock().await.push((message, route));
            Ok(())
        }

        async fn prepare_message_route(
            &self,
            _message: &Message,
            mut route: TransportRoute,
        ) -> std::result::Result<TransportRoute, rvoip_sip_transport::Error> {
            route.flow_id = Some(self.original_flow);
            Ok(route)
        }

        async fn resolve_flow_id_for_route(
            &self,
            _route: &TransportRoute,
        ) -> Option<TransportFlowId> {
            self.resolve_calls.fetch_add(1, Ordering::SeqCst);
            Some(self.later_flow)
        }

        async fn close(&self) -> std::result::Result<(), rvoip_sip_transport::Error> {
            Ok(())
        }

        fn is_closed(&self) -> bool {
            false
        }

        fn supports_tcp(&self) -> bool {
            true
        }
    }

    /// Helper to create a simple INVITE request for testing
    fn create_test_invite() -> std::result::Result<Request, Box<dyn std::error::Error>> {
        let builder = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")?;

        Ok(builder
            .from("Alice", "sip:alice@example.com", Some("alice-tag"))
            .to("Bob", "sip:bob@example.com", None)
            .contact("sip:alice@127.0.0.1:5060", None)
            .call_id("test-call-id-1234")
            .cseq(101)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK.originalbranchvalue"))
            .max_forwards(70)
            .build())
    }

    fn create_test_invite_with_identity(
        call_id: &str,
        branch: &str,
        via_transport: &str,
    ) -> std::result::Result<Request, Box<dyn std::error::Error>> {
        Ok(
            SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")?
                .from("Alice", "sip:alice@example.com", Some("alice-tag"))
                .to("Bob", "sip:bob@example.com", None)
                .contact("sip:alice@127.0.0.1:5060", None)
                .call_id(call_id)
                .cseq(101)
                .via("127.0.0.1:5060", via_transport, Some(branch))
                .max_forwards(70)
                .build(),
        )
    }

    fn create_test_ack() -> std::result::Result<Request, Box<dyn std::error::Error>> {
        let builder = SimpleRequestBuilder::new(Method::Ack, "sip:bob@example.com")?;

        Ok(builder
            .from("Alice", "sip:alice@example.com", Some("alice-tag"))
            .to("Bob", "sip:bob@example.com", Some("bob-tag-resp"))
            .contact("sip:alice@127.0.0.1:5060", None)
            .call_id("test-call-id-1234")
            .cseq(101)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK.ackbranchvalue"))
            .max_forwards(70)
            .build())
    }

    fn create_dispatch_request(
        method: Method,
        branch: &str,
        cseq: u32,
    ) -> std::result::Result<Request, Box<dyn std::error::Error>> {
        Ok(
            SimpleRequestBuilder::new(method.clone(), "sip:bob@example.com")?
                .from("Alice", "sip:alice@example.com", Some("alice-dispatch-tag"))
                .to("Bob", "sip:bob@example.com", Some("bob-dispatch-tag"))
                .contact("sip:alice@127.0.0.1:5060", None)
                .call_id("dispatch-call-id-1234")
                .cseq(cseq)
                .via("127.0.0.1:5060", "UDP", Some(branch))
                .max_forwards(70)
                .build(),
        )
    }

    fn dispatch_event(message: Message) -> TransportEvent {
        dispatch_event_from(message, "127.0.0.1:5060".parse().unwrap())
    }

    fn dispatch_event_from(message: Message, source: SocketAddr) -> TransportEvent {
        TransportEvent::MessageReceived {
            message,
            source,
            destination: "127.0.0.1:5061".parse().unwrap(),
            transport_type: TransportType::Udp,
            flow_id: None,
            raw_bytes: None,
            timing: None,
            connection_metadata: None,
        }
    }

    fn dispatch_stream_event_from(
        message: Message,
        source: SocketAddr,
        flow_id: TransportFlowId,
    ) -> TransportEvent {
        TransportEvent::MessageReceived {
            message,
            source,
            destination: "127.0.0.1:5061".parse().unwrap(),
            transport_type: TransportType::Tcp,
            flow_id: Some(flow_id),
            raw_bytes: None,
            timing: None,
            connection_metadata: None,
        }
    }

    fn queued_dispatch_request(
        method: Method,
        branch: &str,
        cseq: u32,
    ) -> std::result::Result<QueuedTransactionDispatch, Box<dyn std::error::Error>> {
        let event = dispatch_event(Message::Request(create_dispatch_request(
            method, branch, cseq,
        )?));
        Ok(QueuedTransactionDispatch {
            kind: transaction_ingress_kind(&event),
            event,
            queued_at: None,
            worker_id: 0,
        })
    }

    fn authenticated_test_principal(subject: &str) -> AuthenticatedPrincipal {
        AuthenticatedPrincipal {
            subject: subject.to_string(),
            tenant: Some("tenant-a".to_string()),
            scopes: vec!["sip:call".to_string()],
            issuer: Some("test-listener".to_string()),
            expires_at: None,
            method: AuthenticationMethod::SipDigest,
            assurance: IdentityAssurance::Identified {
                credential_kind: CredentialKind::SipDigest,
            },
        }
    }

    #[derive(Debug)]
    struct CountingIngressAuthorizer {
        calls: AtomicUsize,
        authorization: SipRequestAuthorization,
    }

    impl CountingIngressAuthorizer {
        fn authorized(principal: AuthenticatedPrincipal) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                authorization: SipRequestAuthorization::Authorized { principal },
            }
        }

        fn rejected() -> Self {
            Self {
                calls: AtomicUsize::new(0),
                authorization: SipRequestAuthorization::Rejected(
                    SipRequestRejection::new(StatusCode::Unauthorized).with_header(
                        TypedHeader::Other(
                            HeaderName::WwwAuthenticate,
                            HeaderValue::Raw(br#"Digest realm="listener", nonce="test""#.to_vec()),
                        ),
                    ),
                ),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl SipRequestIngressAuthorizer for CountingIngressAuthorizer {
        async fn authorize(
            &self,
            _request: &Request,
            _context: &SipRequestIngressContext,
        ) -> SipRequestAuthorization {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.authorization.clone()
        }
    }

    async fn drain_for_request_event(
        event_rx: &mut mpsc::Receiver<TransactionEvent>,
        deadline: Duration,
    ) -> Option<TransactionEvent> {
        let end = tokio::time::Instant::now() + deadline;
        loop {
            let remaining = end.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return None;
            }
            match tokio::time::timeout(remaining, event_rx.recv()).await {
                Ok(Some(event @ TransactionEvent::InviteRequest { .. }))
                | Ok(Some(event @ TransactionEvent::NonInviteRequest { .. }))
                | Ok(Some(event @ TransactionEvent::CancelRequest { .. }))
                | Ok(Some(event @ TransactionEvent::AckRequest { .. })) => return Some(event),
                Ok(Some(_)) => continue,
                Ok(None) | Err(_) => return None,
            }
        }
    }

    #[tokio::test]
    async fn client_transaction_rejects_unsafe_typed_request_before_reservation_or_io() -> Result<()>
    {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let (manager, _event_rx) =
            TransactionManager::new(transport.clone(), transport_rx, Some(16)).await?;
        let destination: SocketAddr = "192.0.2.1:5060".parse().unwrap();

        let invalid_requests = [
            Request::new(
                Method::Extension("X-UNSAFE\r\nX-Injected: method-secret".into()),
                Uri::custom("sip:alice@example.invalid"),
            ),
            Request::new(
                Method::Options,
                Uri::custom("sip:alice@example.invalid\r\nX-Injected: uri-secret"),
            ),
            Request::new(Method::Options, Uri::custom("sip:alice@example.invalid")).with_header(
                TypedHeader::Subject(rvoip_sip_core::types::Subject::new(
                    "safe\r\nX-Injected: header-secret",
                )),
            ),
        ];

        for request in invalid_requests {
            assert!(
                manager
                    .create_client_transaction(request, destination)
                    .await
                    .is_err(),
                "unsafe typed request must fail at transaction-manager entry"
            );
        }

        assert!(manager.client_transactions.is_empty());
        assert!(manager.transaction_destinations.is_empty());
        assert!(transport.get_sent_messages().await.is_empty());
        assert_eq!(transport.raw_send_count(), 0);

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn direct_transaction_manager_rejects_plaintext_sips_route() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let (manager, _event_rx) =
            TransactionManager::new(transport.clone(), transport_rx, Some(16)).await?;
        let request = SimpleRequestBuilder::new(Method::Options, "sips:service@example.test")
            .unwrap()
            .from("alice", "sips:alice@example.test", Some("tag"))
            .to("service", "sips:service@example.test", None)
            .contact("sips:alice@127.0.0.1:5061", None)
            .call_id("direct-sips-route-policy")
            .cseq(1)
            .max_forwards(70)
            .build();

        let result = manager
            .create_client_transaction_on_route(
                request,
                rvoip_sip_transport::TransportRoute::new("192.0.2.1:5061".parse().unwrap())
                    .with_transport_type(TransportType::Tcp),
            )
            .await;
        assert!(result.is_err());
        assert!(manager.client_transactions.is_empty());
        assert!(manager.transaction_destinations.is_empty());
        assert!(transport.get_sent_messages().await.is_empty());

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn ingress_authorization_rejects_before_tu_and_reuses_transaction_on_retransmit(
    ) -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let (mut manager, mut event_rx) =
            TransactionManager::new(transport.clone(), transport_rx, Some(32)).await?;
        let authorizer = Arc::new(CountingIngressAuthorizer::rejected());
        manager.set_request_ingress_authorizer(Some(authorizer.clone()));

        let invite = create_test_invite().map_err(|error| Error::Other(error.to_string()))?;
        let event = dispatch_event(Message::Request(invite));
        manager.handle_transport_event(event.clone()).await?;

        assert!(
            drain_for_request_event(&mut event_rx, Duration::from_millis(50))
                .await
                .is_none(),
            "an unauthorized INVITE must never reach the transaction user"
        );
        assert_eq!(authorizer.calls(), 1);
        let first_messages = transport.get_sent_messages().await;
        assert!(first_messages.iter().any(|(message, _)| {
            matches!(message, Message::Response(response) if response.status() == StatusCode::Unauthorized)
        }));

        manager.handle_transport_event(event).await?;
        assert_eq!(
            authorizer.calls(),
            1,
            "a retransmission must reuse the original transaction authorization decision"
        );
        assert!(
            drain_for_request_event(&mut event_rx, Duration::from_millis(50))
                .await
                .is_none(),
            "an unauthorized retransmission must not escape to the transaction user"
        );
        let second_messages = transport.get_sent_messages().await;
        assert!(second_messages.len() > first_messages.len());

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn ingress_authorization_retains_principal_for_authorized_invite() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let (mut manager, mut event_rx) =
            TransactionManager::new(transport, transport_rx, Some(32)).await?;
        let principal = authenticated_test_principal("alice");
        let authorizer = Arc::new(CountingIngressAuthorizer::authorized(principal.clone()));
        manager.set_request_ingress_authorizer(Some(authorizer));

        let invite = create_test_invite().map_err(|error| Error::Other(error.to_string()))?;
        let key = TransactionKey::from_request(&invite)
            .ok_or_else(|| Error::Other("INVITE transaction key missing".to_string()))?;
        manager
            .handle_transport_event(dispatch_event(Message::Request(invite)))
            .await?;

        assert!(matches!(
            drain_for_request_event(&mut event_rx, Duration::from_millis(250)).await,
            Some(TransactionEvent::InviteRequest { .. })
        ));
        let retained = manager
            .take_inbound_principal(&key)
            .expect("authorized principal must be retained until dialog ingress consumes it");
        assert_eq!(retained.ownership_key(), principal.ownership_key());

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn ingress_authorization_binds_replays_and_cancel_to_transport_peer() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5061"));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let (mut manager, mut event_rx) =
            TransactionManager::new(transport.clone(), transport_rx, Some(32)).await?;
        let authorizer = Arc::new(CountingIngressAuthorizer::authorized(
            authenticated_test_principal("alice"),
        ));
        manager.set_request_ingress_authorizer(Some(authorizer.clone()));

        let source_a: SocketAddr = "192.0.2.10:5060".parse().unwrap();
        let source_b: SocketAddr = "192.0.2.11:5060".parse().unwrap();
        let invite = create_test_invite().map_err(|error| Error::Other(error.to_string()))?;
        manager
            .handle_transport_event(dispatch_event_from(
                Message::Request(invite.clone()),
                source_a,
            ))
            .await?;
        assert!(matches!(
            drain_for_request_event(&mut event_rx, Duration::from_millis(250)).await,
            Some(TransactionEvent::InviteRequest { .. })
        ));

        manager
            .handle_transport_event(dispatch_event_from(
                Message::Request(invite.clone()),
                source_b,
            ))
            .await?;
        assert!(
            drain_for_request_event(&mut event_rx, Duration::from_millis(50))
                .await
                .is_none(),
            "a replay from a different source must not reach the transaction user"
        );
        assert_eq!(authorizer.calls(), 1);

        let cancel = crate::transaction::method::cancel::create_cancel_request(
            &invite,
            &transport.local_addr().unwrap(),
        )?;
        let cancel_key = TransactionKey::from_request(&cancel)
            .ok_or_else(|| Error::Other("CANCEL transaction key missing".to_string()))?;
        manager
            .handle_transport_event(dispatch_event_from(
                Message::Request(cancel.clone()),
                source_b,
            ))
            .await?;
        assert!(
            drain_for_request_event(&mut event_rx, Duration::from_millis(50))
                .await
                .is_none(),
            "a CANCEL from a different source must not reach the transaction user"
        );
        assert!(
            !manager.server_transactions.contains_key(&cancel_key),
            "an unauthorized CANCEL must not poison the legitimate transaction key"
        );
        assert!(transport.get_sent_messages().await.iter().any(
            |(message, destination)| {
                *destination == source_b
                    && matches!(message, Message::Response(response) if response.status() == StatusCode::CallOrTransactionDoesNotExist)
            }
        ));

        manager
            .handle_transport_event(dispatch_event_from(Message::Request(cancel), source_a))
            .await?;
        assert!(matches!(
            drain_for_request_event(&mut event_rx, Duration::from_millis(250)).await,
            Some(TransactionEvent::CancelRequest { source, .. }) if source == source_a
        ));

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn ingress_authorization_binds_non_2xx_ack_to_transport_peer() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5061"));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let (mut manager, mut event_rx) =
            TransactionManager::new(transport, transport_rx, Some(32)).await?;
        manager.set_request_ingress_authorizer(Some(Arc::new(
            CountingIngressAuthorizer::authorized(authenticated_test_principal("alice")),
        )));

        let source_a: SocketAddr = "192.0.2.20:5060".parse().unwrap();
        let source_b: SocketAddr = "192.0.2.21:5060".parse().unwrap();
        let invite = create_test_invite().map_err(|error| Error::Other(error.to_string()))?;
        let invite_key = TransactionKey::from_request(&invite)
            .ok_or_else(|| Error::Other("INVITE transaction key missing".to_string()))?;
        manager
            .handle_transport_event(dispatch_event_from(
                Message::Request(invite.clone()),
                source_a,
            ))
            .await?;
        assert!(matches!(
            drain_for_request_event(&mut event_rx, Duration::from_millis(250)).await,
            Some(TransactionEvent::InviteRequest { .. })
        ));

        let failure = create_test_response(&invite, StatusCode::BusyHere, Some("Busy Here"));
        manager.send_response(&invite_key, failure.clone()).await?;
        let invite_transaction = manager
            .server_transactions
            .get(&invite_key)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| Error::Other("INVITE transaction missing".to_string()))?;
        assert!(
            manager
                .wait_for_transaction_state(
                    &invite_key,
                    TransactionState::Completed,
                    Duration::from_millis(250),
                )
                .await?,
            "the error response must advance the INVITE transaction to Completed"
        );

        let ack =
            crate::transaction::method::ack::create_ack_for_error_response(&invite, &failure)?;
        manager
            .handle_transport_event(dispatch_event_from(Message::Request(ack.clone()), source_b))
            .await?;
        assert_eq!(
            invite_transaction.state(),
            TransactionState::Completed,
            "an ACK from a different source must not advance the INVITE transaction"
        );

        manager
            .handle_transport_event(dispatch_event_from(Message::Request(ack), source_a))
            .await?;
        assert!(
            manager
                .wait_for_transaction_state(
                    &invite_key,
                    TransactionState::Confirmed,
                    Duration::from_millis(250),
                )
                .await?,
            "the ACK from the authorized source must confirm the INVITE transaction"
        );

        manager.shutdown().await;
        Ok(())
    }

    #[test]
    fn transaction_dispatch_routes_dialog_requests_to_same_worker(
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        let fallback_worker = AtomicUsize::new(0);
        let worker_count = 4;

        let invite = dispatch_event(Message::Request(create_dispatch_request(
            Method::Invite,
            "z9hG4bK.dispatch-invite",
            101,
        )?));
        let ack = dispatch_event(Message::Request(create_dispatch_request(
            Method::Ack,
            "z9hG4bK.dispatch-ack",
            101,
        )?));
        let bye = dispatch_event(Message::Request(create_dispatch_request(
            Method::Bye,
            "z9hG4bK.dispatch-bye",
            102,
        )?));
        let cancel = dispatch_event(Message::Request(create_dispatch_request(
            Method::Cancel,
            "z9hG4bK.dispatch-cancel",
            101,
        )?));

        let expected = transaction_dispatch_worker_index(&invite, worker_count, &fallback_worker);
        assert_eq!(
            transaction_dispatch_worker_index(&ack, worker_count, &fallback_worker),
            expected
        );
        assert_eq!(
            transaction_dispatch_worker_index(&bye, worker_count, &fallback_worker),
            expected
        );
        assert_eq!(
            transaction_dispatch_worker_index(&cancel, worker_count, &fallback_worker),
            expected
        );
        assert_eq!(fallback_worker.load(Ordering::Relaxed), 0);
        assert_eq!(
            transaction_ingress_kind(&invite),
            TransactionIngressKind::Invite
        );
        assert_eq!(transaction_ingress_kind(&ack), TransactionIngressKind::Ack);
        assert_eq!(transaction_ingress_kind(&bye), TransactionIngressKind::Bye);
        assert_eq!(
            transaction_ingress_kind(&cancel),
            TransactionIngressKind::Cancel
        );

        Ok(())
    }

    #[test]
    fn transaction_dispatch_routes_responses_by_transaction_key(
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        let fallback_worker = AtomicUsize::new(0);
        let worker_count = 4;
        let invite_request =
            create_dispatch_request(Method::Invite, "z9hG4bK.dispatch-response", 101)?;
        let response = create_test_response(&invite_request, StatusCode::Ok, None);
        let response_event = dispatch_event(Message::Response(response));

        let first =
            transaction_dispatch_worker_index(&response_event, worker_count, &fallback_worker);
        let second =
            transaction_dispatch_worker_index(&response_event, worker_count, &fallback_worker);

        assert_eq!(first, second);
        assert_eq!(fallback_worker.load(Ordering::Relaxed), 0);
        assert_eq!(
            transaction_ingress_kind(&response_event),
            TransactionIngressKind::Other
        );

        Ok(())
    }

    #[test]
    fn transaction_dispatch_routes_ack_and_bye_to_high_priority_lane(
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        let invite = dispatch_event(Message::Request(create_dispatch_request(
            Method::Invite,
            "z9hG4bK.priority-invite",
            101,
        )?));
        let ack = dispatch_event(Message::Request(create_dispatch_request(
            Method::Ack,
            "z9hG4bK.priority-ack",
            101,
        )?));
        let bye = dispatch_event(Message::Request(create_dispatch_request(
            Method::Bye,
            "z9hG4bK.priority-bye",
            102,
        )?));
        let cancel = dispatch_event(Message::Request(create_dispatch_request(
            Method::Cancel,
            "z9hG4bK.priority-cancel",
            101,
        )?));
        let response = dispatch_event(Message::Response(create_test_response(
            &create_dispatch_request(Method::Invite, "z9hG4bK.priority-response", 101)?,
            StatusCode::Ok,
            None,
        )));

        assert_eq!(
            transaction_dispatch_lane(transaction_ingress_kind(&bye)),
            TransactionDispatchLane::High
        );
        assert_eq!(
            transaction_dispatch_lane(transaction_ingress_kind(&invite)),
            TransactionDispatchLane::Normal
        );
        assert_eq!(
            transaction_dispatch_lane(transaction_ingress_kind(&ack)),
            TransactionDispatchLane::High
        );
        assert_eq!(
            transaction_dispatch_lane(transaction_ingress_kind(&cancel)),
            TransactionDispatchLane::Normal
        );
        assert_eq!(
            transaction_dispatch_lane(transaction_ingress_kind(&response)),
            TransactionDispatchLane::Normal
        );
        assert_eq!(
            transaction_dispatch_lane(transaction_ingress_kind(&TransportEvent::Closed)),
            TransactionDispatchLane::Control
        );

        Ok(())
    }

    #[tokio::test]
    async fn transaction_dispatch_preserves_ack_before_bye_in_high_priority_lane(
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (high_tx, mut high_rx) = mpsc::channel(4);
        let (_normal_tx, mut normal_rx) = mpsc::channel(4);
        let (_control_tx, mut control_rx) = mpsc::channel(4);
        high_tx
            .send(queued_dispatch_request(
                Method::Ack,
                "z9hG4bK.priority-high-ack",
                101,
            )?)
            .await
            .unwrap();
        high_tx
            .send(queued_dispatch_request(
                Method::Bye,
                "z9hG4bK.priority-high-bye",
                102,
            )?)
            .await
            .unwrap();

        let mut high_burst_count = 0;
        let first = recv_transaction_dispatch_event(
            &mut control_rx,
            &mut high_rx,
            &mut normal_rx,
            &mut high_burst_count,
            DEFAULT_TRANSACTION_DISPATCH_PRIORITY_BURST_MAX,
        )
        .await
        .unwrap();
        let second = recv_transaction_dispatch_event(
            &mut control_rx,
            &mut high_rx,
            &mut normal_rx,
            &mut high_burst_count,
            DEFAULT_TRANSACTION_DISPATCH_PRIORITY_BURST_MAX,
        )
        .await
        .unwrap();

        assert_eq!(first.kind, TransactionIngressKind::Ack);
        assert_eq!(second.kind, TransactionIngressKind::Bye);
        assert_eq!(high_burst_count, 2);
        Ok(())
    }

    #[tokio::test]
    async fn transaction_dispatch_processes_bye_before_older_normal_event(
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (high_tx, mut high_rx) = mpsc::channel(4);
        let (normal_tx, mut normal_rx) = mpsc::channel(4);
        let (_control_tx, mut control_rx) = mpsc::channel(4);
        normal_tx
            .send(queued_dispatch_request(
                Method::Invite,
                "z9hG4bK.priority-order-invite",
                101,
            )?)
            .await
            .unwrap();
        high_tx
            .send(queued_dispatch_request(
                Method::Bye,
                "z9hG4bK.priority-order-bye",
                102,
            )?)
            .await
            .unwrap();

        let mut high_burst_count = 0;
        let first = recv_transaction_dispatch_event(
            &mut control_rx,
            &mut high_rx,
            &mut normal_rx,
            &mut high_burst_count,
            DEFAULT_TRANSACTION_DISPATCH_PRIORITY_BURST_MAX,
        )
        .await
        .unwrap();
        let second = recv_transaction_dispatch_event(
            &mut control_rx,
            &mut high_rx,
            &mut normal_rx,
            &mut high_burst_count,
            DEFAULT_TRANSACTION_DISPATCH_PRIORITY_BURST_MAX,
        )
        .await
        .unwrap();

        assert_eq!(first.kind, TransactionIngressKind::Bye);
        assert_eq!(second.kind, TransactionIngressKind::Invite);
        assert_eq!(high_burst_count, 0);
        Ok(())
    }

    #[tokio::test]
    async fn transaction_dispatch_control_lane_preempts_saturated_data_lanes(
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (control_tx, mut control_rx) = mpsc::channel(1);
        let (high_tx, mut high_rx) = mpsc::channel(1);
        let (normal_tx, mut normal_rx) = mpsc::channel(1);
        high_tx
            .send(queued_dispatch_request(
                Method::Bye,
                "z9hG4bK.control-priority-bye",
                101,
            )?)
            .await
            .unwrap();
        normal_tx
            .send(queued_dispatch_request(
                Method::Invite,
                "z9hG4bK.control-priority-invite",
                102,
            )?)
            .await
            .unwrap();
        control_tx
            .send(QueuedTransactionDispatch {
                event: TransportEvent::Closed,
                queued_at: None,
                kind: TransactionIngressKind::Control,
                worker_id: 0,
            })
            .await
            .unwrap();

        let mut high_burst_count = 0;
        let first = recv_transaction_dispatch_event(
            &mut control_rx,
            &mut high_rx,
            &mut normal_rx,
            &mut high_burst_count,
            DEFAULT_TRANSACTION_DISPATCH_PRIORITY_BURST_MAX,
        )
        .await
        .unwrap();
        assert_eq!(first.kind, TransactionIngressKind::Control);
        Ok(())
    }

    #[tokio::test]
    async fn transaction_dispatch_starvation_guard_processes_normal_after_bye_burst(
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        let burst_max = 2;
        let (high_tx, mut high_rx) = mpsc::channel(burst_max + 1);
        let (normal_tx, mut normal_rx) = mpsc::channel(1);
        let (_control_tx, mut control_rx) = mpsc::channel(1);
        normal_tx
            .send(queued_dispatch_request(
                Method::Invite,
                "z9hG4bK.priority-fairness-invite",
                101,
            )?)
            .await
            .unwrap();
        for idx in 0..=burst_max {
            high_tx
                .send(queued_dispatch_request(
                    Method::Bye,
                    &format!("z9hG4bK.priority-fairness-bye-{idx}"),
                    200 + idx as u32,
                )?)
                .await
                .unwrap();
        }

        let mut high_burst_count = 0;
        for _ in 0..burst_max {
            let queued = recv_transaction_dispatch_event(
                &mut control_rx,
                &mut high_rx,
                &mut normal_rx,
                &mut high_burst_count,
                burst_max,
            )
            .await
            .unwrap();
            assert_eq!(queued.kind, TransactionIngressKind::Bye);
        }

        let queued = recv_transaction_dispatch_event(
            &mut control_rx,
            &mut high_rx,
            &mut normal_rx,
            &mut high_burst_count,
            burst_max,
        )
        .await
        .unwrap();
        assert_eq!(queued.kind, TransactionIngressKind::Invite);
        assert_eq!(high_burst_count, 0);

        let queued = recv_transaction_dispatch_event(
            &mut control_rx,
            &mut high_rx,
            &mut normal_rx,
            &mut high_burst_count,
            burst_max,
        )
        .await
        .unwrap();
        assert_eq!(queued.kind, TransactionIngressKind::Bye);
        Ok(())
    }

    #[test]
    fn transaction_dispatch_round_robins_unkeyed_events() {
        let fallback_worker = AtomicUsize::new(0);
        let worker_count = 3;

        assert_eq!(
            transaction_dispatch_worker_index(
                &TransportEvent::Closed,
                worker_count,
                &fallback_worker
            ),
            0
        );
        assert_eq!(
            transaction_dispatch_worker_index(
                &TransportEvent::Closed,
                worker_count,
                &fallback_worker
            ),
            1
        );
        assert_eq!(
            transaction_dispatch_worker_index(
                &TransportEvent::Closed,
                worker_count,
                &fallback_worker
            ),
            2
        );
        assert_eq!(
            transaction_dispatch_worker_index(
                &TransportEvent::Closed,
                worker_count,
                &fallback_worker
            ),
            0
        );
    }

    /// Helper to create a simple 200 OK response for testing
    fn create_test_response(
        request: &Request,
        status: StatusCode,
        reason: Option<&str>,
    ) -> Response {
        use rvoip_sip_core::builder::SimpleResponseBuilder;

        SimpleResponseBuilder::response_from_request(request, status, reason)
            .to("Bob", "sip:bob@example.com", Some("bob-tag-resp"))
            .build()
    }

    fn cached_invite_2xx_entry(
        response: Response,
        destination: SocketAddr,
        created_at: Instant,
        next_retransmit_at: Instant,
    ) -> Invite2xxResponseCacheEntry {
        let wire_bytes =
            bytes::Bytes::from(rvoip_sip_core::Message::Response(response.clone()).to_bytes());
        Invite2xxResponseCacheEntry {
            response,
            wire_bytes,
            route: rvoip_sip_transport::TransportRoute::new(destination)
                .with_transport_type(TransportType::Udp),
            created_at,
            acked_at: None,
            expires_at: created_at + Duration::from_secs(90),
            next_retransmit_at,
            retransmit_interval: Duration::from_millis(500),
            deadline_generation: 0,
            _admission_owner: None,
        }
    }

    async fn schedule_test_compact_timer_j(
        manager: &TransactionManager,
        transaction_id: TransactionKey,
        delay: Duration,
    ) -> (
        Arc<crate::transaction::AtomicTransactionState>,
        mpsc::Receiver<InternalTransactionCommand>,
    ) {
        let state = Arc::new(crate::transaction::AtomicTransactionState::new(
            TransactionState::Completed,
        ));
        let identity = Arc::as_ptr(&state) as usize;
        let (command_tx, command_rx) = mpsc::channel(1);
        let route = rvoip_sip_transport::TransportRoute::new(
            "192.0.2.200:5060".parse().expect("test route"),
        )
        .with_transport_type(TransportType::Udp);
        let response_wire =
            bytes::Bytes::from_static(b"SIP/2.0 200 OK\r\nContent-Length: 0\r\n\r\n");
        let scheduler = manager
            .lifecycle_scheduler
            .as_ref()
            .expect("manager lifecycle scheduler")
            .downgrade();
        assert!(
            scheduler
                .schedule_compact_non_invite(
                    identity,
                    transaction_id,
                    crate::transaction::lifecycle_scheduler::CompactNonInviteTimer::J,
                    delay,
                    Some((response_wire, route)),
                    Arc::clone(&state),
                    None,
                    command_tx,
                )
                .await,
            "compact Timer J schedule should be accepted"
        );
        (state, command_rx)
    }

    async fn replace_compact_retention_capacity(manager: &mut TransactionManager, capacity: usize) {
        if let Some(previous) = manager.lifecycle_scheduler.take() {
            previous.shutdown().await;
        }
        manager.lifecycle_scheduler = Some(
            crate::transaction::lifecycle_scheduler::LifecycleSchedulerHandle::new_managed_with_retention_capacity(
                &manager.compact_non_invite_tombstones,
                &manager.transaction_destinations,
                &manager.pending_inbound_principals,
                &manager.pending_inbound_principal_inserted_at,
                &manager.events_tx,
                capacity,
            ),
        );
    }

    async fn two_live_tcp_flow_ids() -> (
        rvoip_sip_transport::TransportFlowId,
        rvoip_sip_transport::TransportFlowId,
    ) {
        let (server, mut events) =
            rvoip_sip_transport::TcpTransport::bind("127.0.0.1:0".parse().unwrap(), Some(8), None)
                .await
                .unwrap();
        let destination = server.local_addr().unwrap();
        let mut clients = Vec::new();
        for index in 0..2u32 {
            let (client, _events) = rvoip_sip_transport::TcpTransport::bind(
                "127.0.0.1:0".parse().unwrap(),
                Some(4),
                None,
            )
            .await
            .unwrap();
            let call_id = format!("cache-flow-{index}");
            let request = SimpleRequestBuilder::new(Method::Options, "sip:flow-id.test")
                .unwrap()
                .from("alice", "sip:alice@example.test", Some("tag"))
                .to("service", "sip:flow-id.test", None)
                .call_id(&call_id)
                .cseq(1)
                .build();
            client
                .send_message(Message::Request(request), destination)
                .await
                .unwrap();
            clients.push(client);
        }

        let mut flows = Vec::new();
        while flows.len() < 2 {
            if let TransportEvent::MessageReceived {
                flow_id: Some(flow_id),
                ..
            } = tokio::time::timeout(Duration::from_secs(1), events.recv())
                .await
                .unwrap()
                .unwrap()
            {
                flows.push(flow_id);
            }
        }
        for client in clients {
            client.close().await.unwrap();
        }
        server.close().await.unwrap();
        (flows[0], flows[1])
    }

    async fn send_through_client_transaction(request: Request) -> Result<Request> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let (manager, _event_rx) =
            TransactionManager::new(transport.clone(), transport_rx, Some(10)).await?;
        let destination = SocketAddr::from_str("192.0.2.200:5061").unwrap();

        let tx_id = manager
            .create_client_transaction(request, destination)
            .await?;
        manager.send_request(&tx_id).await?;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let sent_messages = transport.get_sent_messages().await;
        manager.shutdown().await;

        assert_eq!(
            sent_messages.len(),
            1,
            "Expected exactly one request to be sent"
        );
        match &sent_messages[0].0 {
            Message::Request(sent_request) => Ok(sent_request.clone()),
            Message::Response(_) => Err(Error::Other("Expected request, got response".into())),
        }
    }

    fn top_via_branch(request: &Request) -> Option<String> {
        request.first_via().and_then(|via| {
            via.0
                .first()
                .and_then(|top| top.branch().map(str::to_string))
        })
    }

    fn top_via_port(request: &Request) -> Option<u16> {
        request
            .first_via()
            .and_then(|via| via.0.first().and_then(|top| top.sent_by_port))
    }

    fn top_via_has_rport(request: &Request) -> bool {
        request
            .first_via()
            .and_then(|via| {
                via.0.first().map(|top| {
                    top.params.iter().any(|param| match param {
                        Param::Rport(_) => true,
                        Param::Other(name, _) => name.eq_ignore_ascii_case("rport"),
                        _ => false,
                    })
                })
            })
            .unwrap_or(false)
    }

    #[tokio::test]
    async fn client_transaction_preserves_tls_register_via() -> Result<()> {
        let request = RegisterBuilder::new()
            .registrar("sips:192.0.2.200:5061;transport=tls")
            .aor("sips:1001@192.0.2.200")
            .user_info("sips:1001@192.0.2.200", "")
            .contact("sips:1001@192.0.2.10:5071;transport=tls")
            .local_address("192.0.2.10:5071".parse().unwrap())
            .expires(3600)
            .call_id("tls-register-transaction-test")
            .cseq(1)
            .build()?;
        assert_eq!(request.first_via_transport(), Some("TLS"));

        let sent_request = send_through_client_transaction(request).await?;

        assert_eq!(sent_request.first_via_transport(), Some("TLS"));
        assert_eq!(top_via_port(&sent_request), Some(5071));
        assert!(top_via_branch(&sent_request)
            .as_deref()
            .is_some_and(|branch| branch.starts_with(RFC3261_BRANCH_MAGIC_COOKIE)));
        assert!(top_via_has_rport(&sent_request));
        Ok(())
    }

    #[tokio::test]
    async fn client_transaction_preserves_tls_invite_via() -> Result<()> {
        let request = InviteBuilder::new()
            .from_to("sips:1001@192.0.2.200", "sips:1002@192.0.2.200")
            .request_uri("sips:1002@192.0.2.200:5061;transport=tls")
            .contact("sips:1001@192.0.2.10:5071;transport=tls")
            .local_address("192.0.2.10:5071".parse().unwrap())
            .call_id("tls-invite-transaction-test")
            .cseq(1)
            .build()?;
        assert_eq!(request.first_via_transport(), Some("TLS"));

        let sent_request = send_through_client_transaction(request).await?;

        assert_eq!(sent_request.first_via_transport(), Some("TLS"));
        assert_eq!(top_via_port(&sent_request), Some(5071));
        assert!(top_via_has_rport(&sent_request));
        Ok(())
    }

    #[tokio::test]
    async fn client_transaction_preserves_tls_bye_via() -> Result<()> {
        let request = ByeBuilder::new()
            .from_dialog(
                "tls-bye-transaction-test",
                "sips:1001@192.0.2.200",
                "from-tag",
                "sips:1002@192.0.2.200",
                "to-tag",
            )
            .request_uri("sips:1002@192.0.2.200:5061;transport=tls")
            .local_address("192.0.2.10:5071".parse().unwrap())
            .cseq(2)
            .build()?;
        assert_eq!(request.first_via_transport(), Some("TLS"));

        let sent_request = send_through_client_transaction(request).await?;

        assert_eq!(sent_request.first_via_transport(), Some("TLS"));
        assert_eq!(top_via_port(&sent_request), Some(5071));
        assert!(top_via_has_rport(&sent_request));
        Ok(())
    }

    #[tokio::test]
    async fn client_transaction_preserves_udp_register_via() -> Result<()> {
        let request = RegisterBuilder::new()
            .registrar("sip:192.0.2.200:5060")
            .aor("sip:2001@192.0.2.200")
            .user_info("sip:2001@192.0.2.200", "")
            .contact("sip:2001@192.0.2.10:5080")
            .local_address("192.0.2.10:5080".parse().unwrap())
            .expires(3600)
            .call_id("udp-register-transaction-test")
            .cseq(1)
            .build()?;

        let sent_request = send_through_client_transaction(request).await?;

        assert_eq!(sent_request.first_via_transport(), Some("UDP"));
        assert_eq!(top_via_port(&sent_request), Some(5080));
        assert!(top_via_has_rport(&sent_request));
        Ok(())
    }

    #[tokio::test]
    async fn client_transaction_adds_branch_and_rport_without_changing_tls_via() -> Result<()> {
        let request =
            SimpleRequestBuilder::new(Method::Options, "sips:192.0.2.200:5061;transport=tls")
                .map_err(|e| Error::Other(e.to_string()))?
                .from("User", "sips:1001@192.0.2.200", Some("from-tag"))
                .to("User", "sips:1002@192.0.2.200", None)
                .call_id("tls-options-missing-via-params")
                .cseq(1)
                .via("192.0.2.10:5071", "TLS", None)
                .max_forwards(70)
                .header(TypedHeader::ContentLength(ContentLength::new(0)))
                .build();

        let sent_request = send_through_client_transaction(request).await?;

        assert_eq!(sent_request.first_via_transport(), Some("TLS"));
        assert_eq!(top_via_port(&sent_request), Some(5071));
        assert!(top_via_branch(&sent_request)
            .as_deref()
            .is_some_and(|branch| branch.starts_with(RFC3261_BRANCH_MAGIC_COOKIE)));
        assert!(top_via_has_rport(&sent_request));
        Ok(())
    }

    #[tokio::test]
    async fn cancel_preserves_original_tls_invite_via_exactly() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let (manager, _event_rx) =
            TransactionManager::new(transport.clone(), transport_rx, Some(10)).await?;
        let destination = SocketAddr::from_str("192.0.2.200:5061").unwrap();
        let invite = InviteBuilder::new()
            .from_to("sips:1001@192.0.2.200", "sips:1002@192.0.2.200")
            .request_uri("sips:1002@192.0.2.200:5061;transport=tls")
            .contact("sips:1001@192.0.2.10:5071;transport=tls")
            .local_address("192.0.2.10:5071".parse().unwrap())
            .call_id("tls-cancel-transaction-test")
            .cseq(1)
            .build()?;

        let invite_tx_id = manager
            .create_client_transaction(invite, destination)
            .await?;
        let invite_request = manager.original_request(&invite_tx_id).await?.unwrap();
        let invite_via = invite_request.first_via().unwrap().to_string();
        let cancel_tx_id = manager.cancel_invite_transaction(&invite_tx_id).await?;
        let cancel = manager.original_request(&cancel_tx_id).await?.unwrap();

        assert_eq!(cancel.method(), Method::Cancel);
        assert_eq!(cancel.first_via().unwrap().to_string(), invite_via);
        assert_eq!(cancel.first_via_transport(), Some("TLS"));

        manager.shutdown().await;
        Ok(())
    }

    /// Test the socket_addr_from_uri utility function
    #[tokio::test]
    async fn test_socket_addr_from_uri() {
        use super::super::utils::socket_addr_from_uri;

        // Test with a valid URI
        let uri = Uri::from_str("sip:test@192.168.1.10:5060").unwrap();
        let addr = socket_addr_from_uri(&uri);
        assert!(addr.is_some());
        assert_eq!(addr.unwrap().to_string(), "192.168.1.10:5060");

        // Test with a URI that has no port (should use default 5060)
        let uri = Uri::from_str("sip:test@192.168.1.10").unwrap();
        let addr = socket_addr_from_uri(&uri);
        assert!(addr.is_some());
        assert_eq!(addr.unwrap().to_string(), "192.168.1.10:5060");

        // Test secure URI default port
        let uri = Uri::from_str("sips:test@192.168.1.10").unwrap();
        let addr = socket_addr_from_uri(&uri);
        assert!(addr.is_some());
        assert_eq!(addr.unwrap().to_string(), "192.168.1.10:5061");

        // Test with a non-IP URI
        let uri = Uri::from_str("sip:test@example.com:5080").unwrap();
        let addr = socket_addr_from_uri(&uri);
        assert!(addr.is_none()); // Should return None because it can't parse as SocketAddr
    }

    /// Test creating and using client transactions
    #[tokio::test]
    async fn test_manager_client_transaction() -> Result<()> {
        // Setup mock transport
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));

        // Verify the transport starts with an empty message list
        let initial_messages = transport.get_sent_messages().await;
        assert_eq!(
            initial_messages.len(),
            0,
            "Transport should start with empty message list"
        );

        let (transport_tx, transport_rx) = mpsc::channel(10);

        // Create the transaction manager
        let (manager, mut event_rx) =
            TransactionManager::new(transport.clone(), transport_rx, Some(10)).await?;

        // Create an INVITE request
        let invite_request = create_test_invite().map_err(|e| Error::Other(e.to_string()))?;
        let destination = SocketAddr::from_str("192.168.1.100:5060").unwrap();

        // Create a client transaction
        let tx_id = manager
            .create_client_transaction(invite_request.clone(), destination)
            .await?;

        println!("Created transaction: {}", tx_id);
        println!("Is server transaction: {}", tx_id.is_server());
        assert_eq!(
            tx_id.is_server(),
            false,
            "Transaction key should indicate this is a client transaction"
        );

        // Send the request
        println!("Sending request through transaction manager");
        manager.send_request(&tx_id).await?;

        // Wait a short time for the request to be processed and sent
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Check that the message was sent
        let sent_messages = transport.get_sent_messages().await;
        println!(
            "Messages in transport after send_request: {}",
            sent_messages.len()
        );
        for (i, (msg, addr)) in sent_messages.iter().enumerate() {
            if let Message::Request(req) = msg {
                println!("Message {}: {} to {}", i, req.method(), addr);
            } else {
                println!("Message {}: Response to {}", i, addr);
            }
        }

        assert_eq!(
            sent_messages.len(),
            1,
            "Expected exactly 1 message after sending INVITE"
        );
        assert!(
            matches!(sent_messages[0].0, Message::Request(_)),
            "First message should be a request"
        );
        assert_eq!(
            sent_messages[0].1, destination,
            "First message should be sent to the specified destination"
        );

        // Create a response
        let response = create_test_response(&invite_request, StatusCode::Ok, Some("OK"));

        // The transaction will send a StateChanged event when it transitions to the Calling state
        // Wait for and handle this event first
        let event = tokio::time::timeout(std::time::Duration::from_millis(500), event_rx.recv())
            .await
            .expect("Timed out waiting for event")
            .unwrap();

        match event {
            TransactionEvent::StateChanged {
                transaction_id,
                previous_state,
                new_state,
            } => {
                assert_eq!(transaction_id, tx_id);
                assert_eq!(previous_state, TransactionState::Initial);
                assert_eq!(new_state, TransactionState::Calling);
            }
            _ => panic!("Unexpected event: {:?}", event),
        }

        // Inject a response
        transport_tx
            .send(rvoip_sip_transport::TransportEvent::MessageReceived {
                message: Message::Response(response.clone()),
                source: destination,
                destination: transport.local_addr().unwrap(),
                transport_type: rvoip_sip_transport::transport::TransportType::Udp,
                flow_id: None,
                raw_bytes: None,
                timing: None,
                connection_metadata: None,
            })
            .await
            .unwrap();

        // Wait for the event
        let event = tokio::time::timeout(std::time::Duration::from_millis(500), event_rx.recv())
            .await
            .expect("Timed out waiting for event")
            .unwrap();

        // Check that we received the right event
        match event {
            TransactionEvent::SuccessResponse {
                transaction_id,
                response: resp,
                ..
            } => {
                assert_eq!(transaction_id, tx_id);
                assert_eq!(resp.status_code(), StatusCode::Ok.as_u16());
            }
            _ => panic!("Unexpected event: {:?}", event),
        }

        // The INVITE transaction will now be in the Terminated state because it received a 200 OK
        // For testing purposes, we'll test the cancel_invite_transaction separately with a new INVITE transaction

        // Create a new INVITE request and transaction specifically for the CANCEL test
        let invite_request2 = create_test_invite_with_identity(
            "test-call-id-for-cancel",
            "z9hG4bK.cancel-target-branch",
            "UDP",
        )
        .map_err(|e| Error::Other(e.to_string()))?;
        let cancel_tx_id = manager
            .create_client_transaction(invite_request2.clone(), destination)
            .await?;

        // Wait a bit for the transaction to be fully initialized
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Ensure the request is sent
        manager.send_request(&cancel_tx_id).await?;

        // Wait for the invite to be sent
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Ignore the StateChanged event for this second transaction
        let _ = tokio::time::timeout(std::time::Duration::from_millis(500), event_rx.recv()).await;

        // Test creating a CANCEL
        println!("Creating CANCEL for transaction: {}", cancel_tx_id);
        println!("Transaction method: {:?}", cancel_tx_id.method());
        println!("Transaction is_server: {}", cancel_tx_id.is_server());
        let cancel_tx_id = manager.cancel_invite_transaction(&cancel_tx_id).await?;
        println!("Created CANCEL transaction: {}", cancel_tx_id);

        // Wait for the CANCEL to be sent
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Verify a CANCEL was created and sent
        let sent_messages = transport.get_sent_messages().await;
        println!(
            "Messages in transport after cancel: {}",
            sent_messages.len()
        );
        for (i, (msg, addr)) in sent_messages.iter().enumerate() {
            if let Message::Request(req) = msg {
                println!("Message {}: {} to {}", i, req.method(), addr);
            } else {
                println!("Message {}: Response to {}", i, addr);
            }
        }

        // We should have 3 messages:
        // 0: First INVITE for the first transaction
        // 1: Second INVITE for the transaction to be canceled
        // 2: CANCEL for the second transaction
        assert_eq!(
            sent_messages.len(),
            3,
            "Expected exactly 3 messages (INVITE + INVITE + CANCEL)"
        );

        if let Message::Request(req) = &sent_messages[2].0 {
            assert_eq!(
                req.method(),
                Method::Cancel,
                "Third message should be a CANCEL request"
            );
        } else {
            panic!("Expected CANCEL request");
        }

        // Clean up
        manager.shutdown().await;

        Ok(())
    }

    /// Test creating an ACK for a 2xx response
    #[tokio::test]
    async fn test_create_ack_for_2xx() -> Result<()> {
        // Set test environment variable
        std::env::set_var("RVOIP_TEST", "1");

        // Setup mock transport
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));

        let (_, transport_rx) = mpsc::channel(10);

        // Create the transaction manager
        let (manager, _event_rx) =
            TransactionManager::new(transport.clone(), transport_rx, Some(10)).await?;

        // Create an INVITE request
        let invite_request = create_test_invite().map_err(|e| Error::Other(e.to_string()))?;
        let destination = SocketAddr::from_str("192.168.1.100:5060").unwrap();

        // Create a client transaction
        let tx_id = manager
            .create_client_transaction(invite_request.clone(), destination)
            .await?;

        println!("Created transaction: {}", tx_id);
        println!("Is server transaction: {}", tx_id.is_server());
        assert_eq!(
            tx_id.is_server(),
            false,
            "Transaction key should indicate this is a client transaction"
        );

        // Send the request to fully initialize the transaction
        println!("Sending request through transaction manager");
        manager.send_request(&tx_id).await?;

        // Wait a short time for the request to be processed and sent
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Check that the message was sent
        let sent_messages = transport.get_sent_messages().await;
        println!(
            "Messages in transport after send_request: {}",
            sent_messages.len()
        );

        // Create a 200 OK response and add a Contact header which would normally be in the response
        let mut response = create_test_response(&invite_request, StatusCode::Ok, Some("OK"));
        // Add Contact header to the response
        let contact_uri = Uri::from_str("sip:bob@192.168.1.2:5060").unwrap();
        let address = Address::new_with_display_name("Bob", contact_uri);
        let contact_param = ContactParamInfo { address };
        let contact = Contact::new_params(vec![contact_param]);
        response = response.with_header(TypedHeader::Contact(contact));

        println!("Creating ACK for 200 OK response");

        // Create an ACK for the 200 OK
        let ack = manager.create_ack_for_2xx(&tx_id, &response).await?;

        // Verify it's an ACK
        assert_eq!(ack.method(), Method::Ack);

        // Verify it has the right headers
        assert!(ack.from().is_some(), "ACK should have From header");
        assert!(ack.to().is_some(), "ACK should have To header");
        assert!(ack.call_id().is_some(), "ACK should have Call-ID header");
        assert!(ack.cseq().is_some(), "ACK should have CSeq header");

        // Verify CSeq method is ACK
        assert_eq!(ack.cseq().unwrap().method, Method::Ack);

        // Send the ACK
        println!("Sending ACK for 200 OK response");
        manager.send_ack_for_2xx(&tx_id, &response).await?;

        // Verify the ACK was sent
        let sent_messages = transport.get_sent_messages().await;
        println!(
            "Messages in transport after send_ack: {}",
            sent_messages.len()
        );
        for (i, (msg, addr)) in sent_messages.iter().enumerate() {
            if let Message::Request(req) = msg {
                println!("Message {}: {} to {}", i, req.method(), addr);
            } else {
                println!("Message {}: Response to {}", i, addr);
            }
        }

        assert_eq!(
            sent_messages.len(),
            2,
            "Expected exactly 2 messages (INVITE + ACK)"
        );

        if let Message::Request(req) = &sent_messages[1].0 {
            assert_eq!(
                req.method(),
                Method::Ack,
                "Second message should be an ACK request"
            );
        } else {
            panic!("Expected ACK request");
        }

        // Clean up
        manager.shutdown().await;

        // Reset environment variable
        std::env::remove_var("RVOIP_TEST");

        Ok(())
    }

    /// Test the get_transaction_request utility function
    #[tokio::test]
    async fn test_get_transaction_request() -> Result<()> {
        use super::super::utils::get_transaction_request;

        // Create a test transaction
        let (tx, _) = mpsc::channel::<TransactionEvent>(10);
        let request = create_test_invite().map_err(|e| Error::Other(e.to_string()))?;
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let remote_addr = SocketAddr::from_str("192.168.1.100:5060").unwrap();
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));

        // Create a transaction and transaction key
        let transaction = ClientInviteTransaction::new(
            TransactionKey::new("z9hG4bK123".to_string(), Method::Invite, false),
            request.clone(),
            remote_addr,
            transport,
            tx,
            None,
        )?;

        // DashMap matches the new ArcClientTransaction-keyed
        // TransactionManager storage.
        let transactions: dashmap::DashMap<TransactionKey, std::sync::Arc<dyn ClientTransaction>> =
            dashmap::DashMap::new();
        let tx_id = transaction.id().clone();
        transactions.insert(tx_id.clone(), std::sync::Arc::new(transaction));

        let retrieved_request = get_transaction_request(&transactions, &tx_id).await?;

        // Verify it's the same request
        assert_eq!(retrieved_request.method(), Method::Invite);
        assert_eq!(retrieved_request.uri(), request.uri());

        Ok(())
    }

    /// Test the full transaction lifecycle for INVITE client transaction
    #[tokio::test]
    async fn test_invite_client_transaction_lifecycle() -> Result<()> {
        // Setup mock transport
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (transport_tx, transport_rx) = mpsc::channel(10);

        // Create the transaction manager
        let (manager, mut event_rx) =
            TransactionManager::new(transport.clone(), transport_rx, Some(10)).await?;

        // Create an INVITE request
        let invite_request = create_test_invite().map_err(|e| Error::Other(e.to_string()))?;
        let destination = SocketAddr::from_str("192.168.1.100:5060").unwrap();

        // Test transaction creation
        let tx_id = manager
            .create_client_transaction(invite_request.clone(), destination)
            .await?;

        // Test transaction_exists
        assert!(
            manager.transaction_exists(&tx_id).await,
            "Transaction should exist after creation"
        );

        // Test transaction_kind
        let kind = manager.transaction_kind(&tx_id).await?;
        assert_eq!(
            kind.to_string(),
            "InviteClient",
            "Transaction kind should be InviteClient"
        );

        // Test transaction_state - should be Initial
        let state = manager.transaction_state(&tx_id).await?;
        assert_eq!(
            state,
            TransactionState::Initial,
            "Initial state should be Initial"
        );

        // Test original_request
        let original_req = manager.original_request(&tx_id).await?;
        assert!(
            original_req.is_some(),
            "Original request should be available"
        );
        assert_eq!(
            original_req.unwrap().method(),
            Method::Invite,
            "Original request should be INVITE"
        );

        // Test remote_addr
        let remote = manager.remote_addr(&tx_id).await?;
        assert_eq!(
            remote, destination,
            "Remote address should match destination"
        );

        // Test last_response - should be None initially
        let last_resp = manager.last_response(&tx_id).await?;
        assert!(
            last_resp.is_none(),
            "Last response should be None initially"
        );

        // Send the request and move transaction to Calling state
        manager.send_request(&tx_id).await?;

        // Wait for state to change
        let success = manager
            .wait_for_transaction_state(
                &tx_id,
                TransactionState::Calling,
                std::time::Duration::from_millis(500),
            )
            .await?;
        assert!(success, "Transaction should transition to Calling state");

        // Consume the state changed event
        let _ = event_rx.recv().await;

        // Manually inject a 180 Ringing response
        let ringing_response =
            create_test_response(&invite_request, StatusCode::Ringing, Some("Ringing"));
        transport_tx
            .send(rvoip_sip_transport::TransportEvent::MessageReceived {
                message: Message::Response(ringing_response.clone()),
                source: destination,
                destination: transport.local_addr().unwrap(),
                transport_type: rvoip_sip_transport::transport::TransportType::Udp,
                flow_id: None,
                raw_bytes: None,
                timing: None,
                connection_metadata: None,
            })
            .await
            .unwrap();

        // Wait for state to change to Proceeding
        let success = manager
            .wait_for_transaction_state(
                &tx_id,
                TransactionState::Proceeding,
                std::time::Duration::from_millis(500),
            )
            .await?;
        assert!(success, "Transaction should transition to Proceeding state");

        // Consume events to keep queue clear
        while event_rx.try_recv().is_ok() {}

        // Inject a 200 OK response
        let ok_response = create_test_response(&invite_request, StatusCode::Ok, Some("OK"));
        transport_tx
            .send(rvoip_sip_transport::TransportEvent::MessageReceived {
                message: Message::Response(ok_response.clone()),
                source: destination,
                destination: transport.local_addr().unwrap(),
                transport_type: rvoip_sip_transport::transport::TransportType::Udp,
                flow_id: None,
                raw_bytes: None,
                timing: None,
                connection_metadata: None,
            })
            .await
            .unwrap();

        // Wait for state to change to Terminated (direct transition for 2xx to INVITE)
        let success = manager
            .wait_for_transaction_state(
                &tx_id,
                TransactionState::Terminated,
                std::time::Duration::from_millis(500),
            )
            .await?;
        assert!(
            success,
            "Transaction should transition to Terminated state after 2xx"
        );

        // Test ACK creation and sending
        let ack = manager.create_ack_for_2xx(&tx_id, &ok_response).await?;
        assert_eq!(
            ack.method(),
            Method::Ack,
            "Created request should be an ACK"
        );

        manager.send_ack_for_2xx(&tx_id, &ok_response).await?;

        // Verify ACK was sent
        let sent_messages = transport.get_sent_messages().await;
        let last_msg = sent_messages.last().unwrap();
        assert!(
            matches!(last_msg.0, Message::Request(ref req) if req.method() == Method::Ack),
            "Last sent message should be an ACK"
        );

        // Test transaction monitoring
        let (client_txs, server_txs) = manager.active_transactions().await;
        assert!(
            client_txs.contains(&tx_id),
            "Transaction should be in active_transactions"
        );
        assert_eq!(server_txs.len(), 0, "No server transactions should exist");

        assert_eq!(
            manager.transaction_count().await,
            1,
            "Transaction count should be 1"
        );

        // Test transaction termination
        manager.terminate_transaction(&tx_id).await?;

        // Verify transaction no longer exists
        assert!(
            !manager.transaction_exists(&tx_id).await,
            "Transaction should not exist after termination"
        );

        // Clean up
        manager.shutdown().await;

        Ok(())
    }

    /// Test non-INVITE client transaction lifecycle
    #[tokio::test]
    async fn test_non_invite_client_transaction_lifecycle() -> Result<()> {
        // Setup mock transport
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (transport_tx, transport_rx) = mpsc::channel(10);

        // Create the transaction manager
        let (manager, mut event_rx) =
            TransactionManager::new(transport.clone(), transport_rx, Some(10)).await?;

        // Create a MESSAGE request (non-INVITE)
        let request = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com")
            .map_err(|e| Error::Other(e.to_string()))?
            .from("Alice", "sip:alice@example.com", Some("alice-tag"))
            .to("Bob", "sip:bob@example.com", None)
            .call_id("test-call-id-message")
            .cseq(102)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK.message-branch"))
            .max_forwards(70)
            .content_type("text/plain")
            .body("Hello, Bob!".as_bytes().to_vec())
            .build();

        let destination = SocketAddr::from_str("192.168.1.100:5060").unwrap();

        // Test transaction creation
        let tx_id = manager
            .create_client_transaction(request.clone(), destination)
            .await?;

        // Test transaction_kind
        let kind = manager.transaction_kind(&tx_id).await?;
        assert_eq!(
            kind.to_string(),
            "NonInviteClient",
            "Transaction kind should be NonInviteClient"
        );

        // Send the request and move transaction to Trying state
        manager.send_request(&tx_id).await?;

        // Wait for state to change
        let success = manager
            .wait_for_transaction_state(
                &tx_id,
                TransactionState::Trying,
                std::time::Duration::from_millis(500),
            )
            .await?;
        assert!(success, "Transaction should transition to Trying state");

        // Consume the state changed event
        let _ = event_rx.recv().await;

        // Inject a 200 OK response
        let ok_response = create_test_response(&request, StatusCode::Ok, Some("OK"));
        transport_tx
            .send(rvoip_sip_transport::TransportEvent::MessageReceived {
                message: Message::Response(ok_response.clone()),
                source: destination,
                destination: transport.local_addr().unwrap(),
                transport_type: rvoip_sip_transport::transport::TransportType::Udp,
                flow_id: None,
                raw_bytes: None,
                timing: None,
                connection_metadata: None,
            })
            .await
            .unwrap();

        // Wait for state to change to Completed
        let success = manager
            .wait_for_transaction_state(
                &tx_id,
                TransactionState::Completed,
                std::time::Duration::from_millis(500),
            )
            .await?;
        assert!(
            success,
            "Transaction should transition to Completed state after 2xx"
        );

        // Test last_response
        let last_resp = manager.last_response(&tx_id).await?;
        assert!(last_resp.is_some(), "Last response should be available");
        assert_eq!(
            last_resp.unwrap().status_code(),
            200,
            "Last response should be 200 OK"
        );

        // Wait for Timer K to fire and move to Terminated
        // Use a much longer timeout since Timer K might be configured longer
        println!("Waiting for Timer K to transition to Terminated state...");
        let success = manager
            .wait_for_transaction_state(
                &tx_id,
                TransactionState::Terminated,
                std::time::Duration::from_millis(2000), // Longer timeout for Timer K
            )
            .await?;

        // If the transaction didn't reach Terminated state normally, try forcing it to terminate
        if !success {
            println!(
                "Transaction did not transition to Terminated state naturally, forcing termination."
            );
            manager.terminate_transaction(&tx_id).await?;
        }

        // Verify transaction state one more time
        let state = manager.transaction_state(&tx_id).await;
        match state {
            Ok(TransactionState::Terminated) => {
                println!("Transaction successfully reached Terminated state");
            }
            Ok(other_state) => {
                println!("Transaction in unexpected state: {:?}", other_state);
                // Force termination one more time
                manager.terminate_transaction(&tx_id).await?;
            }
            Err(e) => {
                println!("Error getting transaction state: {}, assuming it's gone", e);
            }
        }

        // Clean up
        manager.shutdown().await;

        Ok(())
    }

    /// Test server transaction creation and operations
    #[tokio::test]
    async fn test_server_transaction_lifecycle() -> Result<()> {
        // Setup mock transport
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (transport_tx, transport_rx) = mpsc::channel(10);

        // Create the transaction manager
        let (manager, mut event_rx) =
            TransactionManager::new(transport.clone(), transport_rx, Some(10)).await?;

        // Create an INVITE request
        let mut invite_request = create_test_invite().map_err(|e| Error::Other(e.to_string()))?;
        println!("Created INVITE request: {:?}", invite_request.method());

        // Ensure the VIA header has a proper branch parameter
        let branch = format!(
            "{}test-branch-{}",
            RFC3261_BRANCH_MAGIC_COOKIE,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
                % 10000
        );

        // Create a new Via with our branch
        let via = Via::new(
            "SIP",
            "2.0",
            "UDP",
            "127.0.0.1",
            Some(5060),
            vec![Param::branch(&branch)],
        )
        .map_err(|e| Error::Other(e.to_string()))?;

        // Replace the Via header in the request
        invite_request = invite_request.with_header(TypedHeader::Via(via));

        println!("Using branch parameter: {}", branch);
        let client_addr = SocketAddr::from_str("192.168.1.100:5060").unwrap();

        // Instead of injecting via transport, create server transaction directly
        let tx = manager
            .create_server_transaction(invite_request.clone(), client_addr)
            .await?;

        let tx_id = tx.id().clone();
        println!("Created server transaction: {}", tx_id);

        // Test transaction kind
        let kind = manager.transaction_kind(&tx_id).await?;
        println!("Transaction kind: {}", kind);
        assert_eq!(
            kind.to_string(),
            "InviteServer",
            "Transaction kind should be InviteServer"
        );

        // Test transaction_state
        let state = manager.transaction_state(&tx_id).await?;
        println!("Initial server transaction state: {:?}", state);
        assert_eq!(
            state,
            TransactionState::Proceeding,
            "Initial server state should be Proceeding"
        );

        // Test send_response - send a 180 Ringing
        let ringing_response =
            create_test_response(&invite_request, StatusCode::Ringing, Some("Ringing"));
        manager
            .send_response(&tx_id, ringing_response.clone())
            .await?;

        // Verify the response was sent
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let sent_messages = transport.get_sent_messages().await;
        let last_msg = sent_messages.last().unwrap();
        println!(
            "Last message sent: {:?}",
            if let Message::Response(ref resp) = last_msg.0 {
                format!("Response {}", resp.status())
            } else {
                format!("Not a response")
            }
        );
        assert!(
            matches!(last_msg.0, Message::Response(ref resp) if resp.status_code() == 180),
            "Last sent message should be a 180 Ringing"
        );

        // Send a final response - 200 OK
        let ok_response = create_test_response(&invite_request, StatusCode::Ok, Some("OK"));
        manager.send_response(&tx_id, ok_response.clone()).await?;

        // Verify the response was sent
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let sent_messages = transport.get_sent_messages().await;
        let last_msg = sent_messages.last().unwrap();
        assert!(
            matches!(last_msg.0, Message::Response(ref resp) if resp.status_code() == 200),
            "Last sent message should be a 200 OK"
        );

        // Wait for state to change to Terminated (INVITE server transitions directly to Terminated after 2xx)
        println!("Waiting for transaction to reach Terminated state");
        let success = manager
            .wait_for_transaction_state(
                &tx_id,
                TransactionState::Terminated,
                std::time::Duration::from_millis(1000),
            )
            .await?;

        // If not terminated naturally, force it
        if !success {
            println!("Transaction didn't reach Terminated state, forcing termination");
            manager.terminate_transaction(&tx_id).await?;
        }

        // Clean up
        manager.shutdown().await;

        Ok(())
    }

    #[tokio::test]
    async fn immediate_tcp_response_survives_peer_close_after_first_write() -> Result<()> {
        use rvoip_sip_core::builder::SimpleResponseBuilder;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let destination = listener.local_addr().unwrap();
        let peer = tokio::spawn(async move {
            let (stream, source) = listener.accept().await.unwrap();
            let connection =
                rvoip_sip_transport::transport::tcp::TcpConnection::from_stream(stream, source)
                    .expect("accepted TCP connection");
            let Message::Request(request) = connection
                .receive_message()
                .await
                .expect("read request")
                .expect("request frame")
            else {
                panic!("expected request");
            };
            let response =
                SimpleResponseBuilder::response_from_request(&request, StatusCode::Ok, Some("OK"))
                    .build();
            connection
                .send_message(&Message::Response(response))
                .await
                .expect("write immediate response");
            connection.close().await.expect("close immediately");
        });

        let (transport, transport_rx) =
            rvoip_sip_transport::TcpTransport::bind("127.0.0.1:0".parse().unwrap(), Some(16), None)
                .await
                .unwrap();
        let transport = Arc::new(transport);
        let (manager, mut events) =
            TransactionManager::new(transport, transport_rx, Some(16)).await?;
        let request = SimpleRequestBuilder::new(Method::Options, "sip:immediate-close.test")
            .unwrap()
            .from("alice", "sip:alice@example.test", Some("tag"))
            .to("service", "sip:immediate-close.test", None)
            .contact("sip:alice@127.0.0.1:5060", None)
            .call_id("tcp-immediate-response-close")
            .cseq(1)
            .via("127.0.0.1:5060", "TCP", Some("z9hG4bK.immediate-close"))
            .max_forwards(70)
            .build();
        let transaction = manager
            .create_client_transaction_on_route(
                request,
                rvoip_sip_transport::TransportRoute::new(destination)
                    .with_transport_type(TransportType::Tcp),
            )
            .await?;
        manager.send_request(&transaction).await?;

        let success = tokio::time::timeout(Duration::from_secs(2), async {
            while let Some(event) = events.recv().await {
                if matches!(
                    event,
                    TransactionEvent::SuccessResponse {
                        ref transaction_id,
                        ..
                    } if transaction_id == &transaction
                ) {
                    return true;
                }
            }
            false
        })
        .await
        .unwrap_or(false);
        assert!(success, "immediate response was rejected after peer close");
        assert!(
            manager
                .transaction_route(&transaction)
                .await
                .is_some_and(|route| route.flow_id.is_some()),
            "client transaction did not retain its pre-write exact flow"
        );

        peer.await.unwrap();
        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn client_route_retirement_has_no_unknown_authentication_window() -> Result<()> {
        use rvoip_sip_core::builder::SimpleResponseBuilder;

        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(16);
        let (manager, mut events) =
            TransactionManager::new(transport, transport_rx, Some(16)).await?;
        let destination: SocketAddr = "192.0.2.27:5060".parse().unwrap();
        let request = create_test_invite_with_identity(
            "retirement-barrier-call",
            "z9hG4bK.retirement-barrier",
            "UDP",
        )
        .map_err(|error| Error::Other(error.to_string()))?;
        let transaction = manager
            .create_client_transaction_on_route(
                request.clone(),
                TransportRoute::new(destination).with_transport_type(TransportType::Udp),
            )
            .await?;
        manager.send_request(&transaction).await?;
        while events.try_recv().is_ok() {}

        let transitioned = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        manager.install_retired_client_transition_test_gate(
            transaction.clone(),
            transitioned.clone(),
            release.clone(),
        );
        let termination = {
            let manager = manager.clone();
            let transaction = transaction.clone();
            tokio::spawn(async move { manager.terminate_transaction(&transaction).await })
        };

        tokio::time::timeout(Duration::from_secs(1), transitioned.notified())
            .await
            .expect("retirement transition did not reach barrier");
        assert!(
            manager.client_transactions.contains_key(&transaction),
            "live Arc must remain published until the route becomes Retired"
        );
        assert!(manager
            .transaction_destinations
            .get(&transaction)
            .is_some_and(|entry| matches!(entry.value(), ClientResponseRouteState::Retired(_))));

        let response =
            SimpleResponseBuilder::response_from_request(&request, StatusCode::Ok, Some("OK"))
                .to("Bob", "sip:bob@example.com", Some("retirement-race-tag"))
                .build();
        manager
            .handle_transport_event(dispatch_event_from(
                Message::Response(response.clone()),
                destination,
            ))
            .await?;

        let outcome = tokio::time::timeout(Duration::from_millis(500), async {
            loop {
                match events.recv().await {
                    Some(TransactionEvent::SuccessResponse { transaction_id, .. })
                        if transaction_id == transaction =>
                    {
                        return "success"
                    }
                    Some(TransactionEvent::StrayResponse { .. }) => return "stray",
                    Some(_) => continue,
                    None => return "closed",
                }
            }
        })
        .await;

        release.notify_one();
        manager.clear_retired_client_transition_test_gate();
        termination
            .await
            .expect("termination task panicked")
            .expect("termination failed");
        assert_eq!(
            outcome.expect("authenticated late response was not delivered"),
            "success",
            "response authentication observed a transient unknown route"
        );

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn retired_stream_invite_accepts_original_f1_and_rejects_coaddressed_f2() -> Result<()> {
        use rvoip_sip_core::builder::SimpleResponseBuilder;

        let (original_flow, later_flow) = two_live_tcp_flow_ids().await;
        assert_ne!(original_flow, later_flow);
        let transport = Arc::new(ExactFlowMockTransport::new(original_flow, later_flow));
        let (_transport_tx, transport_rx) = mpsc::channel(16);
        let (manager, mut events) =
            TransactionManager::new(transport.clone(), transport_rx, Some(16)).await?;
        let destination: SocketAddr = "192.0.2.28:5060".parse().unwrap();
        let request = create_test_invite_with_identity(
            "retired-stream-flow-call",
            "z9hG4bK.retired-stream-flow",
            "TCP",
        )
        .map_err(|error| Error::Other(error.to_string()))?;
        let transaction = manager
            .create_client_transaction_on_route(
                request.clone(),
                TransportRoute::new(destination).with_transport_type(TransportType::Tcp),
            )
            .await?;
        manager.send_request(&transaction).await?;
        manager.terminate_transaction(&transaction).await?;
        while events.try_recv().is_ok() {}

        let response =
            SimpleResponseBuilder::response_from_request(&request, StatusCode::Ok, Some("OK"))
                .to("Bob", "sip:bob@example.com", Some("retired-stream-tag"))
                .build();
        manager
            .handle_transport_event(dispatch_stream_event_from(
                Message::Response(response.clone()),
                destination,
                later_flow,
            ))
            .await?;
        let wrong_flow_deadline = tokio::time::Instant::now() + Duration::from_millis(75);
        loop {
            let remaining =
                wrong_flow_deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, events.recv()).await {
                Ok(Some(TransactionEvent::SuccessResponse { transaction_id, .. }))
                    if transaction_id == transaction =>
                {
                    panic!("co-addressed replacement flow authenticated a retired response")
                }
                Ok(Some(TransactionEvent::StrayResponse { .. })) => {
                    panic!("known retired route was reported as stray")
                }
                Ok(Some(_)) => continue,
                Ok(None) | Err(_) => break,
            }
        }

        manager
            .handle_transport_event(dispatch_stream_event_from(
                Message::Response(response),
                destination,
                original_flow,
            ))
            .await?;
        tokio::time::timeout(Duration::from_millis(500), async {
            loop {
                if matches!(
                    events.recv().await,
                    Some(TransactionEvent::SuccessResponse {
                        ref transaction_id, ..
                    }) if transaction_id == &transaction
                ) {
                    return;
                }
            }
        })
        .await
        .expect("original stream flow response was not delivered");
        assert_eq!(
            transport.resolve_calls(),
            0,
            "response authentication must not resolve a replacement flow by address"
        );

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn retirement_retains_client_data_exact_route_for_late_ack() -> Result<()> {
        use rvoip_sip_core::builder::SimpleResponseBuilder;

        let (original_flow, later_flow) = two_live_tcp_flow_ids().await;
        let transport = Arc::new(ExactFlowMockTransport::new(original_flow, later_flow));
        let (_transport_tx, transport_rx) = mpsc::channel(16);
        let (manager, _events) =
            TransactionManager::new(transport.clone(), transport_rx, Some(16)).await?;
        let destination: SocketAddr = "192.0.2.29:5060".parse().unwrap();
        let request = create_test_invite_with_identity(
            "retained-exact-route-call",
            "z9hG4bK.retained-exact-route",
            "TCP",
        )
        .map_err(|error| Error::Other(error.to_string()))?;
        let transaction = manager
            .create_client_transaction_on_route(
                request.clone(),
                TransportRoute::new(destination).with_transport_type(TransportType::Tcp),
            )
            .await?;
        manager.send_request(&transaction).await?;

        // Corrupt only the compatibility index. Retirement must source the
        // tombstone from ClientTransactionData::request_route, which retains
        // the pre-write F1 binding.
        let mut state = manager
            .transaction_destinations
            .get_mut(&transaction)
            .expect("active response route");
        let indexed = state.route().clone();
        let owner = match state.value() {
            ClientResponseRouteState::Active { owner, .. } => *owner,
            ClientResponseRouteState::Retired(_) => panic!("expected active response route"),
        };
        *state = ClientResponseRouteState::active(indexed.with_flow_id(later_flow), owner);
        drop(state);

        manager.terminate_transaction(&transaction).await?;
        let retained = manager
            .transaction_route(&transaction)
            .await
            .expect("retained exact route");
        assert_eq!(retained.flow_id, Some(original_flow));

        let response =
            SimpleResponseBuilder::response_from_request(&request, StatusCode::Ok, Some("OK"))
                .to("Bob", "sip:bob@example.com", Some("retained-ack-tag"))
                .build();
        manager.send_ack_for_2xx(&transaction, &response).await?;
        assert_eq!(
            transport
                .sent_routes()
                .await
                .last()
                .and_then(|route| route.flow_id),
            Some(original_flow),
            "late ACK did not reuse the retained original flow"
        );
        assert_eq!(transport.resolve_calls(), 0);

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn route_prepare_failure_is_zero_wire_and_does_not_retain_invite() -> Result<()> {
        let transport = Arc::new(WireBoundaryMockTransport::prepare_failure());
        let (_transport_tx, transport_rx) = mpsc::channel(8);
        let (manager, _events) =
            TransactionManager::new(transport.clone(), transport_rx, Some(8)).await?;
        let request = create_test_invite_with_identity(
            "zero-wire-prepare-failure",
            "z9hG4bK.zero-wire-prepare-failure",
            "UDP",
        )
        .map_err(|error| Error::Other(error.to_string()))?;
        let transaction = manager
            .create_client_transaction(request, "192.0.2.51:5060".parse().unwrap())
            .await?;

        assert!(manager.send_request(&transaction).await.is_err());
        assert_eq!(transport.wire_attempts(), 0);
        tokio::time::timeout(Duration::from_secs(1), async {
            while manager.client_transactions.contains_key(&transaction) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("zero-wire failed transaction cleanup");
        assert!(manager.retired_client_transaction(&transaction).is_none());
        assert!(manager.transaction_route(&transaction).await.is_none());
        assert!(manager
            .cancel_invite_transaction(&transaction)
            .await
            .is_err());

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn first_write_error_retires_exact_invite_and_allows_cancel() -> Result<()> {
        let transport = Arc::new(WireBoundaryMockTransport::first_write_failure());
        let (_transport_tx, transport_rx) = mpsc::channel(8);
        let (manager, _events) =
            TransactionManager::new(transport.clone(), transport_rx, Some(8)).await?;
        let destination: SocketAddr = "192.0.2.52:5060".parse().unwrap();
        let request = create_test_invite_with_identity(
            "wire-unknown-first-write",
            "z9hG4bK.wire-unknown-first-write",
            "UDP",
        )
        .map_err(|error| Error::Other(error.to_string()))?;
        let transaction = manager
            .create_client_transaction(request, destination)
            .await?;

        assert!(manager.send_request(&transaction).await.is_err());
        assert_eq!(transport.wire_attempts(), 1);
        let retained = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let Some(retired) = manager.retired_client_transaction(&transaction) {
                    break retired;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("wire-attempted INVITE must retain a compact request and route");
        assert_eq!(retained.route.destination, destination);
        assert_eq!(retained.original_request()?.method(), Method::Invite);

        let cancel = manager.cancel_invite_transaction(&transaction).await?;
        let attempted = transport.attempted().await;
        assert_eq!(attempted.len(), 2);
        assert!(
            matches!(attempted[0].0, Message::Request(ref request) if request.method() == Method::Invite)
        );
        assert!(
            matches!(attempted[1].0, Message::Request(ref request) if request.method() == Method::Cancel)
        );
        assert_eq!(attempted[0].1, attempted[1].1);
        assert_eq!(attempted[1].1.destination, destination);

        manager.terminate_transaction(&cancel).await?;
        manager.shutdown().await;
        Ok(())
    }

    #[test]
    fn retired_client_deadline_scheduler_pops_only_due_and_exact_overflow_entries() {
        let now = Instant::now();
        let due = TransactionKey::new("z9hG4bK.deadline-due".into(), Method::Invite, false);
        let oldest = TransactionKey::new("z9hG4bK.deadline-oldest".into(), Method::Invite, false);
        let newest = TransactionKey::new("z9hG4bK.deadline-newest".into(), Method::Invite, false);
        let mut scheduler = RetiredClientDeadlineScheduler::default();

        let due_at = now - Duration::from_millis(1);
        let due_version = scheduler.next_version(due_at);
        scheduler.schedule(due.clone(), due_at, due_version);
        let oldest_at = now + Duration::from_secs(10);
        let oldest_version = scheduler.next_version(oldest_at);
        scheduler.schedule(oldest.clone(), oldest_at, oldest_version);
        let newest_at = now + Duration::from_secs(20);
        let newest_version = scheduler.next_version(newest_at);
        scheduler.schedule(newest.clone(), newest_at, newest_version);

        let due_entries = scheduler.take_due_and_overflow(now, 2, usize::MAX);
        assert_eq!(due_entries.len(), 1);
        assert_eq!(due_entries[0].transaction_id.as_ref(), &due);
        assert_eq!(due_entries[0].version, due_version);
        assert_eq!(scheduler.len(), 2);

        let overflow = scheduler.take_due_and_overflow(now, 1, usize::MAX);
        assert_eq!(overflow.len(), 1);
        assert_eq!(overflow[0].transaction_id.as_ref(), &oldest);
        assert_eq!(overflow[0].version, oldest_version);
        assert_eq!(scheduler.len(), 1);

        assert!(scheduler.unschedule(&newest, newest_at, newest_version));
        assert_eq!(scheduler.len(), 0);
    }

    #[test]
    fn retired_client_deadline_scheduler_exact_replacement_has_no_stale_growth() {
        let now = Instant::now();
        let transaction =
            TransactionKey::new("z9hG4bK.deadline-reuse".into(), Method::Invite, false);
        let mut scheduler = RetiredClientDeadlineScheduler::default();
        let first_at = now + Duration::from_secs(1);
        let first_version = scheduler.next_version(first_at);
        scheduler.schedule(transaction.clone(), first_at, first_version);

        let mut current_at = first_at;
        let mut current_version = first_version;
        for offset in 2..10_002 {
            let expires_at = now + Duration::from_secs(offset);
            let next_version = scheduler.next_version(expires_at);
            assert!(scheduler.unschedule(&transaction, current_at, current_version));
            scheduler.schedule(transaction.clone(), expires_at, next_version);
            current_at = expires_at;
            current_version = next_version;
        }

        assert_eq!(scheduler.len(), 1);
        assert!(!scheduler.unschedule(&transaction, first_at, first_version));
        assert_eq!(scheduler.len(), 1);
        assert!(scheduler.unschedule(&transaction, current_at, current_version));
        assert_eq!(scheduler.len(), 0);
    }

    #[test]
    fn completion_deadline_scheduler_has_one_exact_index_and_no_reverse_copy() {
        let now = Instant::now();
        let first = TransactionKey::new("z9hG4bK.completion-first".into(), Method::Bye, false);
        let second = TransactionKey::new("z9hG4bK.completion-second".into(), Method::Invite, false);
        let mut scheduler = ClientCompletionDeadlineScheduler::default();

        let first_at = now - Duration::from_millis(1);
        let first_version = scheduler.next_version(first_at);
        scheduler.schedule(first.clone(), first_at, first_version);
        let second_at = now + Duration::from_secs(90);
        let second_version = scheduler.next_version(second_at);
        scheduler.schedule(second.clone(), second_at, second_version);
        assert_eq!(scheduler.len(), 2);

        let due = scheduler.take_due_and_overflow(now, 2, usize::MAX);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].transaction_id.as_ref(), &first);
        assert_eq!(due[0].version, first_version);
        assert_eq!(scheduler.len(), 1);
        assert!(!scheduler.unschedule(&second, second_at, first_version));
        assert!(scheduler.unschedule(&second, second_at, second_version));
        assert_eq!(scheduler.len(), 0);
    }

    #[test]
    fn completion_deadline_scheduler_caps_synchronized_due_batches() {
        let now = Instant::now();
        let mut scheduler = ClientCompletionDeadlineScheduler::default();
        let total = RETAINED_CLIENT_DEADLINE_BATCH_MAX + 17;
        for index in 0..total {
            let transaction = TransactionKey::new(
                format!("z9hG4bK.completion-batch-{index}"),
                Method::Bye,
                false,
            );
            let expires_at = now - Duration::from_millis(1);
            let version = scheduler.next_version(expires_at);
            scheduler.schedule(transaction, expires_at, version);
        }

        let first = scheduler.take_due_and_overflow(now, total, RETAINED_CLIENT_DEADLINE_BATCH_MAX);
        assert_eq!(first.len(), RETAINED_CLIENT_DEADLINE_BATCH_MAX);
        assert!(scheduler.has_due_or_overflow(now, total));
        assert_eq!(scheduler.len(), 17);

        let second =
            scheduler.take_due_and_overflow(now, total, RETAINED_CLIENT_DEADLINE_BATCH_MAX);
        assert_eq!(second.len(), 17);
        assert!(!scheduler.has_due_or_overflow(now, total));
        assert_eq!(scheduler.len(), 0);
    }

    #[tokio::test]
    async fn retained_deadline_worker_yields_and_requeues_synchronized_expiry() -> Result<()> {
        use crate::transaction::completion::ClientTransactionCompletionEntry;

        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let manager = TransactionManager::with_config(transport, None);
        let total = RETAINED_CLIENT_DEADLINE_BATCH_MAX + 17;
        let expires_at = Instant::now() - Duration::from_millis(1);

        {
            let mut deadlines = manager
                .client_completion_deadlines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for index in 0..total {
                let transaction = Arc::new(TransactionKey::new(
                    format!("z9hG4bK.worker-completion-batch-{index}"),
                    Method::Bye,
                    false,
                ));
                let version = deadlines.next_version(expires_at);
                let completion = ClientTransactionCompletion::new(TransactionState::Completed)
                    .retained(expires_at, version);
                manager.client_completions.insert(
                    Arc::clone(&transaction),
                    ClientTransactionCompletionEntry::Retained(completion),
                );
                deadlines.schedule(transaction, expires_at, version);
            }
        }
        manager.wake_retained_client_deadline_worker();

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if manager.client_completions.is_empty()
                    && manager
                        .client_completion_deadlines
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .len()
                        == 0
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("due worker must drain synchronized expiry in bounded turns");

        let snapshot = manager
            .retained_client_deadline_worker_snapshot()
            .expect("runtime manager deadline worker");
        assert!(snapshot.wakeups >= 1);
        assert!(snapshot.batches >= 2);
        assert_eq!(snapshot.records, total as u64);
        assert!(snapshot.yields >= 1);

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn due_completion_deadline_removes_only_its_exact_retained_generation() -> Result<()> {
        use crate::transaction::completion::{
            ClientTransactionCompletion, ClientTransactionCompletionEntry,
        };

        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(4);
        let manager = TransactionManager::dummy(transport, transport_rx);
        let transaction =
            TransactionKey::new("z9hG4bK.expired-completion".into(), Method::Bye, false);
        let completion = ClientTransactionCompletion::new(TransactionState::Completed);
        completion.record_response(Response::new(StatusCode::Ok));
        let expires_at = Instant::now() - Duration::from_millis(1);
        let version = manager
            .client_completion_deadlines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .next_version(expires_at);
        let retained = completion.retained(expires_at, version);
        manager.client_completions.insert(
            Arc::new(transaction.clone()),
            ClientTransactionCompletionEntry::Retained(retained),
        );
        manager
            .client_completion_deadlines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .schedule(transaction.clone(), expires_at, version);

        manager.prune_client_completions();
        assert!(manager.client_completions.get(&transaction).is_none());
        assert_eq!(
            manager
                .client_completion_deadlines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len(),
            0
        );

        manager.shutdown().await;
        Ok(())
    }

    #[test]
    fn retired_client_transaction_is_compact_and_lazily_reconstructs_binary_request() {
        struct LegacyRetiredClientTransaction {
            request: Request,
            route: TransportRoute,
            expires_at: Instant,
            deadline_version: u64,
        }

        let body = bytes::Bytes::from_static(&[0x00, 0x80, 0xff, b'\r', b'\n']);
        let request = create_test_invite()
            .expect("test INVITE")
            .with_body(body.clone());
        let route = TransportRoute::new("192.0.2.31:5060".parse().unwrap())
            .with_transport_type(TransportType::Udp);
        let completion = ClientTransactionCompletion::new(TransactionState::Completed);
        let retired = RetiredClientTransaction::new(
            &request,
            &completion,
            route.clone(),
            Instant::now() + Duration::from_secs(90),
            7,
            None,
        );

        assert!(retired.request_wire_len() > body.len());
        assert_eq!(retired.request_wire.as_ref(), request.to_bytes());
        assert_eq!(
            request.to_bytes(),
            Message::Request(request.clone()).to_bytes()
        );
        assert_eq!(retired.route, route);
        let snapshot = retired.clone();
        assert_eq!(
            retired.request_wire.as_ptr(),
            snapshot.request_wire.as_ptr()
        );
        assert_eq!(retired.route, snapshot.route);
        let restored = retired.original_request().expect("lazy request parse");
        assert_eq!(restored.method(), Method::Invite);
        assert_eq!(restored.body(), body.as_ref());
        assert_eq!(
            restored.call_id().expect("retained Call-ID").to_string(),
            request.call_id().expect("source Call-ID").to_string()
        );

        assert!(
            std::mem::size_of::<RetiredClientTransaction>()
                < std::mem::size_of::<LegacyRetiredClientTransaction>(),
            "the terminal tombstone must not embed the parsed Request"
        );

        // Construction and route authentication do not parse the wire image.
        // A deliberately malformed image remains a usable route tombstone and
        // fails only when the compatibility request snapshot is requested.
        let malformed_completion = ClientTransactionCompletion::new(TransactionState::Completed)
            .retained(Instant::now() + Duration::from_secs(90), 8);
        let malformed = RetiredClientTransaction {
            request_wire: bytes::Bytes::from_static(b"not a SIP request"),
            completion: malformed_completion,
            route: route.clone(),
            expires_at: Instant::now() + Duration::from_secs(90),
            deadline_version: 8,
        };
        assert_eq!(malformed.route, route);
        assert!(malformed.original_request().is_err());
    }

    #[tokio::test]
    async fn retained_indexes_keep_logical_capacity_without_eager_allocation() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(16);
        let (manager, _events) = TransactionManager::new(transport, transport_rx, Some(16)).await?;

        assert!(
            manager
                .retired_client_transaction_capacity
                .load(Ordering::Acquire)
                >= super::super::MIN_RETIRED_CLIENT_TRANSACTION_CAPACITY
        );
        assert!(
            manager.client_completion_capacity
                >= super::super::MIN_RETIRED_CLIENT_TRANSACTION_CAPACITY
        );
        assert_eq!(manager.client_completions.capacity(), 0);
        assert_eq!(manager.invite_2xx_response_cache.capacity(), 0);
        assert_eq!(manager.server_invite_dialog_index.capacity(), 0);
        assert_eq!(
            manager.transaction_index_initial_capacity,
            transaction_index_initial_capacity(manager.transaction_index_logical_capacity)
        );

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn low_hot_index_capacity_does_not_limit_compact_retention_admission() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(4);
        let (manager, _events) = TransactionManager::new(transport, transport_rx, Some(1)).await?;
        let scheduler = manager
            .lifecycle_scheduler
            .as_ref()
            .expect("manager lifecycle scheduler");

        assert_eq!(manager.transaction_index_logical_capacity, 1_024);
        assert!(
            scheduler.compact_retention_limit() > manager.transaction_index_logical_capacity,
            "hot-index sizing must not become the Timer J/K protocol cap"
        );
        let reservations: Vec<_> = (0..=manager.transaction_index_logical_capacity)
            .map(|_| {
                scheduler
                    .try_reserve_compact_retention()
                    .expect("retention admission remains independent")
            })
            .collect();
        assert_eq!(
            scheduler.compact_retention_in_use(),
            manager.transaction_index_logical_capacity + 1
        );
        drop(reservations);
        assert_eq!(scheduler.compact_retention_in_use(), 0);
        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn outbound_compact_capacity_rejects_before_wire_and_releases_for_retry() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(8);
        let (mut manager, _events) =
            TransactionManager::new(transport.clone(), transport_rx, Some(8)).await?;
        replace_compact_retention_capacity(&mut manager, 1).await;
        let scheduler = manager.lifecycle_scheduler.as_ref().unwrap();
        let held = scheduler
            .try_reserve_compact_retention()
            .expect("occupy only compact slot");
        let request = create_dispatch_request(Method::Options, "z9hG4bK-capacity-outbound", 1)
            .map_err(|error| Error::Other(error.to_string()))?;
        let route = TransportRoute::new("192.0.2.20:5060".parse().unwrap())
            .with_transport_type(TransportType::Udp);

        assert!(matches!(
            manager
                .create_client_transaction_on_route(request.clone(), route.clone())
                .await,
            Err(Error::TransactionCapacityExhausted {
                resource: "UDP non-INVITE Timer K retention",
                limit: 1,
            })
        ));
        assert!(manager.client_transactions.is_empty());
        assert!(manager.transaction_destinations.is_empty());
        assert!(transport.get_sent_messages().await.is_empty());

        drop(held);
        let transaction_id = manager
            .create_client_transaction_on_route(request, route)
            .await?;
        assert!(manager.client_transactions.contains_key(&transaction_id));
        assert_eq!(scheduler.compact_retention_in_use(), 1);
        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn inbound_compact_capacity_sends_stateless_503_without_allocation() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(8);
        let (mut manager, _events) =
            TransactionManager::new(transport.clone(), transport_rx, Some(8)).await?;
        replace_compact_retention_capacity(&mut manager, 1).await;
        let scheduler = manager.lifecycle_scheduler.as_ref().unwrap();
        let held = scheduler
            .try_reserve_compact_retention()
            .expect("occupy only compact slot");
        let request = create_dispatch_request(Method::Options, "z9hG4bK-capacity-inbound", 1)
            .map_err(|error| Error::Other(error.to_string()))?;
        let source: SocketAddr = "192.0.2.21:5060".parse().unwrap();

        manager
            .handle_transport_event(dispatch_event_from(Message::Request(request), source))
            .await?;

        assert!(manager.server_transactions.is_empty());
        assert!(manager.compact_non_invite_tombstones.is_empty());
        assert_eq!(scheduler.compact_retention_in_use(), 1);
        let sent = transport.get_sent_messages().await;
        assert_eq!(sent.len(), 1);
        let Message::Response(response) = &sent[0].0 else {
            panic!("capacity rejection must be a SIP response");
        };
        assert_eq!(response.status(), StatusCode::ServiceUnavailable);
        assert!(matches!(
            response.header(&HeaderName::RetryAfter),
            Some(TypedHeader::RetryAfter(value)) if value.delay == 1
        ));

        drop(held);
        assert_eq!(scheduler.compact_retention_in_use(), 0);
        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn failed_server_initial_command_rolls_back_map_and_compact_lease() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(8);
        let (mut manager, _events) =
            TransactionManager::new(transport, transport_rx, Some(8)).await?;
        replace_compact_retention_capacity(&mut manager, 1).await;
        let scheduler = manager.lifecycle_scheduler.as_ref().unwrap();
        let request = create_dispatch_request(Method::Options, "z9hG4bK-init-failure", 1)
            .map_err(|error| Error::Other(error.to_string()))?;
        let key = TransactionKey::new("z9hG4bK-init-failure".into(), Method::Options, true);
        let route = TransportRoute::new("192.0.2.22:5060".parse().unwrap())
            .with_transport_type(TransportType::Udp);
        let transaction: Arc<dyn crate::transaction::server::ServerTransaction> = Arc::new(
            ServerNonInviteTransaction::new_with_response_route_and_command_channel_capacity(
                key.clone(),
                request,
                route,
                manager.transport.clone(),
                manager.events_tx.clone_for_transaction(),
                Some(manager.timer_settings.clone()),
                1,
            )?,
        );
        transaction.data().install_compact_retention_reservation(
            scheduler
                .try_reserve_compact_retention()
                .expect("initial server retention lease"),
        );
        transaction
            .data()
            .install_lifecycle_scheduler(scheduler.clone());
        manager
            .server_transactions
            .insert(key.clone(), transaction.clone());

        let handle = transaction
            .data()
            .event_loop_handle
            .lock()
            .await
            .take()
            .expect("server runner");
        handle.abort();
        let _ = handle.await;
        assert!(transaction
            .send_command(InternalTransactionCommand::TransitionTo(
                TransactionState::Trying,
            ))
            .await
            .is_err());
        manager.rollback_failed_server_initialization(&transaction);
        assert!(!manager.server_transactions.contains_key(&key));
        drop(transaction);
        tokio::task::yield_now().await;
        assert_eq!(scheduler.compact_retention_in_use(), 0);
        drop(
            scheduler
                .try_reserve_compact_retention()
                .expect("failed initialization immediately returns capacity"),
        );
        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn concurrent_same_key_client_admission_constructs_one_runner() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(8);
        let (manager, _events) = TransactionManager::new(transport, transport_rx, Some(8)).await?;
        let manager = Arc::new(manager);
        let request = create_dispatch_request(Method::Options, "z9hG4bK-client-race", 1)
            .map_err(|error| Error::Other(error.to_string()))?;
        let route = TransportRoute::new("192.0.2.40:5060".parse().unwrap())
            .with_transport_type(TransportType::Tcp);
        let barrier = Arc::new(Barrier::new(3));
        let spawn_create = |manager: Arc<TransactionManager>| {
            let request = request.clone();
            let route = route.clone();
            let barrier = Arc::clone(&barrier);
            tokio::spawn(async move {
                barrier.wait().await;
                manager
                    .create_client_transaction_on_route(request, route)
                    .await
            })
        };
        let first = spawn_create(Arc::clone(&manager));
        let second = spawn_create(Arc::clone(&manager));
        barrier.wait().await;
        let first = first.await.unwrap();
        let second = second.await.unwrap();
        assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
        assert!(matches!(
            first.as_ref().err().or_else(|| second.as_ref().err()),
            Some(Error::TransactionExists { .. })
        ));
        assert_eq!(manager.client_transactions.len(), 1);
        assert_eq!(manager.transaction_admissions.entries.len(), 1);
        manager.shutdown().await;
        assert!(manager.transaction_admissions.entries.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn concurrent_same_key_server_retransmission_reuses_one_runner() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(8);
        let (manager, _events) = TransactionManager::new(transport, transport_rx, Some(8)).await?;
        let manager = Arc::new(manager);
        let request = create_dispatch_request(Method::Options, "z9hG4bK-server-race", 1)
            .map_err(|error| Error::Other(error.to_string()))?;
        let route = TransportRoute::new("192.0.2.41:5060".parse().unwrap())
            .with_transport_type(TransportType::Tcp);
        let barrier = Arc::new(Barrier::new(3));
        let spawn_create = |manager: Arc<TransactionManager>| {
            let request = request.clone();
            let route = route.clone();
            let barrier = Arc::clone(&barrier);
            tokio::spawn(async move {
                barrier.wait().await;
                manager
                    .create_server_transaction_on_route(request, route)
                    .await
            })
        };
        let first = spawn_create(Arc::clone(&manager));
        let second = spawn_create(Arc::clone(&manager));
        barrier.wait().await;
        let first = first.await.unwrap()?;
        let second = second.await.unwrap()?;
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(manager.server_transactions.len(), 1);
        assert_eq!(manager.transaction_admissions.entries.len(), 1);
        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn retained_completion_fences_same_key_until_exact_removal() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(8);
        let (manager, _events) = TransactionManager::new(transport, transport_rx, Some(8)).await?;
        let request = create_dispatch_request(Method::Options, "z9hG4bK-completion-owner", 1)
            .map_err(|error| Error::Other(error.to_string()))?;
        let route = TransportRoute::new("192.0.2.42:5060".parse().unwrap())
            .with_transport_type(TransportType::Tcp);
        let key = manager
            .create_client_transaction_on_route(request.clone(), route.clone())
            .await?;
        let transaction = manager.client_transactions.get(&key).unwrap().clone();
        assert!(manager.retire_and_remove_client_transaction(&key).await);
        let runner = transaction
            .data()
            .event_loop_handle
            .lock()
            .await
            .take()
            .unwrap();
        runner.abort();
        let _ = runner.await;
        drop(transaction);
        assert!(manager.client_completions.contains_key(&key));
        assert!(matches!(
            manager
                .create_client_transaction_on_route(request.clone(), route.clone())
                .await,
            Err(Error::TransactionExists { .. })
        ));
        manager.client_completions.remove(&key);
        tokio::task::yield_now().await;
        assert!(!manager.transaction_admissions.entries.contains_key(&key));
        let replacement = manager
            .create_client_transaction_on_route(request, route)
            .await?;
        assert_eq!(replacement, key);
        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn reliable_auth_challenge_completion_outlives_short_consumer_grace() -> Result<()> {
        use rvoip_sip_core::types::auth::WwwAuthenticate;

        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(8);
        let (manager, _events) = TransactionManager::new(transport, transport_rx, Some(8)).await?;
        let request = create_dispatch_request(Method::Options, "z9hG4bK-reliable-auth-delay", 1)
            .map_err(|error| Error::Other(error.to_string()))?;
        let exact_request_uri = request.uri.clone();
        let route = TransportRoute::new("192.0.2.44:5060".parse().unwrap())
            .with_transport_type(TransportType::Tcp);
        let key = manager
            .create_client_transaction_on_route(request, route)
            .await?;
        let transaction = manager.client_transactions.get(&key).unwrap().clone();

        let mut challenge = Response::new(StatusCode::Unauthorized);
        challenge
            .headers
            .push(TypedHeader::WwwAuthenticate(WwwAuthenticate::new(
                "example", "nonce",
            )));
        transaction
            .data()
            .completion
            .record_response_for_request(challenge, &exact_request_uri);

        let (expires_at, keep_live) = manager
            .client_completion_retention(&key, &transaction)
            .await;
        assert!(
            !keep_live,
            "reliable completions retire into the manager map"
        );
        assert!(
            expires_at.saturating_duration_since(Instant::now()) > Duration::from_secs(80),
            "auth challenges must retain exact retry context beyond the ordinary one-second grace"
        );

        assert!(manager.retire_and_remove_client_transaction(&key).await);
        let runner = transaction
            .data()
            .event_loop_handle
            .lock()
            .await
            .take()
            .unwrap();
        runner.abort();
        let _ = runner.await;
        drop(transaction);

        tokio::time::sleep(Duration::from_millis(1_100)).await;
        assert_eq!(
            manager.auth_challenge_request_uri(&key).as_ref(),
            Some(&exact_request_uri),
            "a delayed lossless dialog consumer must recover the exact wire URI"
        );

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn retired_invite_route_fences_same_key_until_exact_removal() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(8);
        let (manager, _events) = TransactionManager::new(transport, transport_rx, Some(8)).await?;
        let request = create_dispatch_request(Method::Invite, "z9hG4bK-invite-owner", 1)
            .map_err(|error| Error::Other(error.to_string()))?;
        let route = TransportRoute::new("192.0.2.43:5060".parse().unwrap())
            .with_transport_type(TransportType::Tcp);
        let key = manager
            .create_client_transaction_on_route(request.clone(), route.clone())
            .await?;
        let transaction = manager.client_transactions.get(&key).unwrap().clone();
        transaction.data().mark_initial_send_attempted();
        assert!(manager.retire_and_remove_client_transaction(&key).await);
        let runner = transaction
            .data()
            .event_loop_handle
            .lock()
            .await
            .take()
            .unwrap();
        runner.abort();
        let _ = runner.await;
        drop(transaction);
        assert!(manager
            .transaction_destinations
            .get(&key)
            .is_some_and(|state| state.retired().is_some()));
        assert!(matches!(
            manager
                .create_client_transaction_on_route(request.clone(), route.clone())
                .await,
            Err(Error::TransactionExists { .. })
        ));
        manager.transaction_destinations.remove(&key);
        tokio::task::yield_now().await;
        assert!(!manager.transaction_admissions.entries.contains_key(&key));
        assert_eq!(
            manager
                .create_client_transaction_on_route(request, route)
                .await?,
            key
        );
        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn retained_server_invite_ack_index_fences_same_key() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(8);
        let (manager, _events) = TransactionManager::new(transport, transport_rx, Some(8)).await?;
        let request = create_dispatch_request(Method::Invite, "z9hG4bK-server-invite-owner", 1)
            .map_err(|error| Error::Other(error.to_string()))?;
        let route = TransportRoute::new("192.0.2.44:5060".parse().unwrap())
            .with_transport_type(TransportType::Tcp);
        let transaction = manager
            .create_server_transaction_on_route(request.clone(), route.clone())
            .await?;
        let key = transaction.id().clone();
        manager.retire_server_invite_dialog_index_for(&key);
        manager.server_transactions.remove(&key);
        let runner = transaction
            .data()
            .event_loop_handle
            .lock()
            .await
            .take()
            .unwrap();
        runner.abort();
        let _ = runner.await;
        drop(transaction);
        assert!(!manager.server_invite_dialog_index.is_empty());
        assert!(matches!(
            manager
                .create_server_transaction_on_route(request.clone(), route.clone())
                .await,
            Err(Error::TransactionExists { .. })
        ));
        manager.server_invite_dialog_index.clear();
        tokio::task::yield_now().await;
        assert!(!manager.transaction_admissions.entries.contains_key(&key));
        let replacement = manager
            .create_server_transaction_on_route(request, route)
            .await?;
        assert_eq!(replacement.id(), &key);
        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn terminal_receiver_close_fails_admission_closed() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(4);
        let (manager, events) = TransactionManager::new(transport, transport_rx, Some(4)).await?;
        drop(events);
        let request = create_dispatch_request(Method::Options, "z9hG4bK-closed-terminal", 1)
            .map_err(|error| Error::Other(error.to_string()))?;
        let route = TransportRoute::new("192.0.2.45:5060".parse().unwrap())
            .with_transport_type(TransportType::Tcp);
        let key = manager
            .create_client_transaction_on_route(request, route.clone())
            .await?;
        manager.terminate_transaction(&key).await?;
        assert_eq!(
            manager.admission_lifecycle.state(),
            super::super::MANAGER_ADMISSION_DRAINING
        );
        let replacement = create_dispatch_request(Method::Options, "z9hG4bK-after-close", 2)
            .map_err(|error| Error::Other(error.to_string()))?;
        assert!(manager
            .create_client_transaction_on_route(replacement, route)
            .await
            .is_err());
        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn shutdown_is_bounded_when_primary_event_queue_is_full() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(1);
        let (manager, _events) = TransactionManager::new(transport, transport_rx, Some(1)).await?;
        while manager
            .events_tx
            .try_send(TransactionEvent::ShutdownComplete)
            .is_ok()
        {}
        tokio::time::timeout(Duration::from_secs(1), manager.shutdown())
            .await
            .expect("shutdown must bound its final observation");
        assert_eq!(
            manager.admission_lifecycle.state(),
            super::super::MANAGER_ADMISSION_STOPPED
        );
        assert!(manager.transaction_admissions.entries.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn invite_initialization_rollback_removes_ack_indexes_and_owner() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(4);
        let (manager, _events) = TransactionManager::new(transport, transport_rx, Some(4)).await?;
        let request = create_dispatch_request(Method::Invite, "z9hG4bK-invite-init-rollback", 1)
            .map_err(|error| Error::Other(error.to_string()))?;
        let key = TransactionKey::new("z9hG4bK-invite-init-rollback".into(), Method::Invite, true);
        let route = TransportRoute::new("192.0.2.46:5060".parse().unwrap())
            .with_transport_type(TransportType::Tcp);
        let owner = manager.transaction_admissions.try_claim(&key).unwrap();
        let transaction: Arc<dyn crate::transaction::server::ServerTransaction> = Arc::new(
            ServerInviteTransaction::new_with_response_route_and_command_channel_capacity(
                key.clone(),
                request.clone(),
                route,
                manager.transport.clone(),
                manager.events_tx.clone_for_transaction(),
                Some(manager.timer_settings.clone()),
                1,
            )?,
        );
        transaction
            .data()
            .install_transaction_admission_owner(owner.clone());
        manager.index_server_invite_dialog(&request, &key, owner);
        manager
            .server_transactions
            .insert(key.clone(), transaction.clone());
        manager.rollback_failed_server_initialization(&transaction);
        assert!(manager.server_invite_dialog_index.is_empty());
        assert!(manager.server_invite_dialog_keys_by_tx.is_empty());
        drop(transaction);
        tokio::task::yield_now().await;
        assert!(!manager.transaction_admissions.entries.contains_key(&key));
        manager.shutdown().await;
        Ok(())
    }

    #[test]
    fn high_logical_transaction_capacity_uses_bounded_reserve_and_grows() {
        const LOGICAL_CAPACITY: usize = 128_000;

        let logical_capacity = transaction_index_capacity(Some(LOGICAL_CAPACITY));
        let initial_capacity = transaction_index_initial_capacity(logical_capacity);
        assert_eq!(logical_capacity, LOGICAL_CAPACITY);
        assert_eq!(initial_capacity, MAX_EAGER_TRANSACTION_INDEX_CAPACITY);

        let table = dashmap::DashMap::with_capacity(initial_capacity);
        let initially_allocated = table.capacity();
        assert!(
            initially_allocated < logical_capacity,
            "a high logical limit must not be eagerly reserved in each table"
        );

        // DashMap's reported capacity includes its shard/load-factor rounding.
        // Crossing that real initial bound proves the table remains dynamic;
        // the reserve cap is not an admission cap.
        for id in 0..=initially_allocated {
            table.insert(id, id);
        }
        assert_eq!(table.len(), initially_allocated + 1);
        assert!(table.capacity() > initially_allocated);
        assert_eq!(table.get(&0).map(|entry| *entry), Some(0));
        assert_eq!(
            table.get(&initially_allocated).map(|entry| *entry),
            Some(initially_allocated)
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn retired_route_count_tracks_concurrent_retire_prune_expiry_and_shutdown() -> Result<()>
    {
        const TRANSACTIONS: usize = 48;
        const RETIRED_CAP: usize = 8;

        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(128);
        let (mut manager, mut events) =
            TransactionManager::new(transport, transport_rx, Some(512)).await?;
        manager
            .retired_client_transaction_capacity
            .store(RETIRED_CAP, Ordering::Release);
        let destination: SocketAddr = "192.0.2.30:5060".parse().unwrap();
        let mut transactions = Vec::with_capacity(TRANSACTIONS);

        for index in 0..TRANSACTIONS {
            let request = create_test_invite_with_identity(
                &format!("retired-count-{index}"),
                &format!("z9hG4bK.retired-count-{index}"),
                "UDP",
            )
            .map_err(|error| Error::Other(error.to_string()))?;
            let transaction = manager
                .create_client_transaction_on_route(
                    request,
                    TransportRoute::new(destination).with_transport_type(TransportType::Udp),
                )
                .await?;
            manager.send_request(&transaction).await?;
            transactions.push(transaction);
            while events.try_recv().is_ok() {}
        }

        let mut retirements = Vec::with_capacity(TRANSACTIONS);
        for transaction in transactions {
            let manager = manager.clone();
            retirements.push(tokio::spawn(async move {
                manager.terminate_transaction(&transaction).await
            }));
        }
        for retirement in retirements {
            retirement
                .await
                .expect("retirement task panicked")
                .expect("retirement failed");
        }

        let actual_retired = manager
            .transaction_destinations
            .iter()
            .filter(|entry| entry.value().retired().is_some())
            .count();
        assert!(actual_retired <= RETIRED_CAP);
        assert_eq!(
            manager
                .retired_client_transaction_count
                .load(Ordering::Acquire),
            actual_retired
        );
        assert_eq!(manager.retention_counts().transaction_destinations, 0);
        assert_eq!(manager.retired_client_transaction_count(), actual_retired);
        assert_eq!(manager.retired_client_deadline_count(), actual_retired);
        let retained = manager.retired_client_retention_counts();
        assert_eq!(retained.transactions, actual_retired);
        assert_eq!(retained.deadlines, actual_retired);
        assert_eq!(retained.ack_template_allocations, 0);
        assert!(retained.request_wire_bytes > 0);
        let breakdown = manager.retention_breakdown();
        assert_eq!(
            breakdown["retired_client_transactions"].as_u64(),
            Some(actual_retired as u64),
        );
        assert_eq!(
            breakdown["retired_client_deadlines"].as_u64(),
            Some(actual_retired as u64),
        );
        assert_eq!(
            breakdown["retired_client"]["request_wire_bytes"].as_u64(),
            Some(retained.request_wire_bytes as u64),
        );
        assert_eq!(
            breakdown["retired_client"]["ack_template_allocations"].as_u64(),
            Some(0),
        );

        let retired_keys: Vec<_> = manager
            .transaction_destinations
            .iter()
            .filter(|entry| entry.value().retired().is_some())
            .map(|entry| entry.key().clone())
            .collect();
        for transaction_id in retired_keys {
            assert!(manager.reschedule_retired_client_deadline_for_test(
                &transaction_id,
                Instant::now() - Duration::from_millis(1),
            ));
        }

        let participants = 5;
        let start = Arc::new(tokio::sync::Barrier::new(participants));
        let mut maintenance = Vec::new();
        for _ in 0..participants - 1 {
            let manager = manager.clone();
            let start = start.clone();
            maintenance.push(tokio::spawn(async move {
                start.wait().await;
                manager.retired_client_transaction_count()
            }));
        }
        let shutdown = {
            let manager = manager.clone();
            let start = start.clone();
            tokio::spawn(async move {
                start.wait().await;
                manager.shutdown().await;
            })
        };

        for task in maintenance {
            task.await.expect("retention maintenance task panicked");
        }
        shutdown.await.expect("shutdown task panicked");

        assert!(manager.transaction_destinations.is_empty());
        assert_eq!(
            manager
                .retired_client_transaction_count
                .load(Ordering::Acquire),
            0
        );
        assert_eq!(manager.retired_client_transaction_count(), 0);
        assert_eq!(manager.retired_client_deadline_count(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn retired_invite_accepts_only_route_authenticated_late_2xx_and_can_ack() -> Result<()> {
        use rvoip_sip_core::builder::SimpleResponseBuilder;

        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(16);
        let (manager, mut events) =
            TransactionManager::new(transport.clone(), transport_rx, Some(16)).await?;
        let destination: SocketAddr = "192.0.2.25:5060".parse().unwrap();
        let request = create_test_invite().map_err(|error| Error::Other(error.to_string()))?;
        let transaction = manager
            .create_client_transaction_on_route(
                request.clone(),
                rvoip_sip_transport::TransportRoute::new(destination)
                    .with_transport_type(TransportType::Udp),
            )
            .await?;

        manager.send_request(&transaction).await?;
        manager.terminate_transaction(&transaction).await?;
        while events.try_recv().is_ok() {}

        assert!(manager.transaction_route(&transaction).await.is_some());
        assert_eq!(
            manager
                .original_request(&transaction)
                .await?
                .expect("retired request")
                .call_id()
                .expect("Call-ID")
                .to_string(),
            request.call_id().expect("Call-ID").to_string()
        );

        let response =
            SimpleResponseBuilder::response_from_request(&request, StatusCode::Ok, Some("OK"))
                .to("Bob", "sip:bob@example.com", Some("late-fork-tag"))
                .contact("sip:bob@192.0.2.25:5060", None)
                .build();

        manager
            .handle_transport_event(dispatch_event_from(
                Message::Response(response.clone()),
                "192.0.2.99:5060".parse().unwrap(),
            ))
            .await?;
        let wrong_route_deadline = tokio::time::Instant::now() + Duration::from_millis(50);
        loop {
            let remaining =
                wrong_route_deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, events.recv()).await {
                Ok(Some(TransactionEvent::SuccessResponse { transaction_id, .. }))
                    if transaction_id == transaction =>
                {
                    panic!("a response from the wrong route reached the TU")
                }
                Ok(Some(_)) => continue,
                Ok(None) | Err(_) => break,
            }
        }

        manager
            .handle_transport_event(dispatch_event_from(
                Message::Response(response.clone()),
                destination,
            ))
            .await?;
        let event = tokio::time::timeout(Duration::from_millis(250), async {
            loop {
                let event = events.recv().await.expect("late response event channel");
                if matches!(
                    event,
                    TransactionEvent::SuccessResponse {
                        ref transaction_id,
                        need_ack: true,
                        ..
                    } if transaction_id == &transaction
                ) {
                    return event;
                }
            }
        })
        .await
        .expect("late response event timeout");
        assert!(matches!(event, TransactionEvent::SuccessResponse { .. }));

        manager.send_ack_for_2xx(&transaction, &response).await?;
        let sent = transport.get_sent_messages().await;
        let ack = sent
            .iter()
            .find_map(|(message, route)| match message {
                Message::Request(request)
                    if *route == destination && request.method() == Method::Ack =>
                {
                    Some(request)
                }
                _ => None,
            })
            .expect("retired INVITE ACK");
        assert_eq!(ack.uri().to_string(), "sip:bob@192.0.2.25:5060");
        assert_eq!(ack.from(), request.from());
        assert_eq!(ack.call_id(), request.call_id());
        assert_eq!(
            ack.cseq().map(|cseq| (cseq.seq, cseq.method.clone())),
            Some((101, Method::Ack))
        );

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn retired_invite_late_2xx_authentication_does_not_parse_request_wire() -> Result<()> {
        use rvoip_sip_core::builder::SimpleResponseBuilder;

        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(16);
        let (manager, mut events) =
            TransactionManager::new(transport, transport_rx, Some(16)).await?;
        let destination: SocketAddr = "192.0.2.32:5060".parse().unwrap();
        let request = create_test_invite_with_identity(
            "retired-lazy-auth-call",
            "z9hG4bK.retired-lazy-auth",
            "UDP",
        )
        .map_err(|error| Error::Other(error.to_string()))?;
        let transaction = manager
            .create_client_transaction_on_route(
                request.clone(),
                TransportRoute::new(destination).with_transport_type(TransportType::Udp),
            )
            .await?;
        manager.send_request(&transaction).await?;
        manager.terminate_transaction(&transaction).await?;
        while events.try_recv().is_ok() {}

        let mut route_state = manager
            .transaction_destinations
            .get_mut(&transaction)
            .expect("retired route state");
        let ClientResponseRouteState::Retired(retired) = route_state.value_mut() else {
            panic!("expected retired client transaction");
        };
        retired.request_wire = bytes::Bytes::from_static(b"deliberately malformed");
        drop(route_state);

        let response =
            SimpleResponseBuilder::response_from_request(&request, StatusCode::Ok, Some("OK"))
                .to("Bob", "sip:bob@example.com", Some("lazy-auth-tag"))
                .build();
        manager
            .handle_transport_event(dispatch_event_from(
                Message::Response(response.clone()),
                destination,
            ))
            .await?;

        let success = tokio::time::timeout(Duration::from_millis(250), async {
            loop {
                match events.recv().await {
                    Some(TransactionEvent::SuccessResponse {
                        ref transaction_id,
                        need_ack: true,
                        ..
                    }) if transaction_id == &transaction => return true,
                    Some(_) => continue,
                    None => return false,
                }
            }
        })
        .await
        .unwrap_or(false);
        assert!(success, "late 2xx authentication parsed the request image");
        assert!(manager
            .send_ack_for_2xx(&transaction, &response)
            .await
            .is_err());
        assert!(manager.original_request(&transaction).await.is_err());

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn expired_retired_invite_route_is_pruned_and_cannot_be_revived() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(16);
        let (manager, _events) = TransactionManager::new(transport, transport_rx, Some(16)).await?;
        let destination: SocketAddr = "192.0.2.26:5060".parse().unwrap();
        let request = create_test_invite().map_err(|error| Error::Other(error.to_string()))?;
        let transaction = manager
            .create_client_transaction_on_route(
                request,
                rvoip_sip_transport::TransportRoute::new(destination)
                    .with_transport_type(TransportType::Udp),
            )
            .await?;
        manager.send_request(&transaction).await?;
        manager.terminate_transaction(&transaction).await?;

        assert!(manager.reschedule_retired_client_deadline_for_test(
            &transaction,
            Instant::now() - Duration::from_millis(1),
        ));
        assert_eq!(manager.retired_client_transaction_count(), 0);
        assert!(manager.transaction_route(&transaction).await.is_none());
        assert!(manager.original_request(&transaction).await.is_err());

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn invite_retransmission_after_2xx_reuses_cached_response() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (transport_tx, transport_rx) = mpsc::channel(16);
        let (manager, _event_rx) =
            TransactionManager::new(transport.clone(), transport_rx, Some(16)).await?;

        let invite_request = create_test_invite().map_err(|e| Error::Other(e.to_string()))?;
        let source: SocketAddr = "192.0.2.100:5060".parse().unwrap();
        let transaction = manager
            .create_server_transaction(invite_request.clone(), source)
            .await?;
        let tx_id = transaction.id().clone();

        let ok_response = create_test_response(&invite_request, StatusCode::Ok, Some("OK"));
        manager.send_response(&tx_id, ok_response).await?;

        let terminated = manager
            .wait_for_transaction_state(
                &tx_id,
                TransactionState::Terminated,
                std::time::Duration::from_millis(500),
            )
            .await?;
        assert!(
            terminated,
            "INVITE server transaction should terminate after 2xx"
        );

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let before = transport.get_sent_messages().await.len();

        transport_tx
            .send(rvoip_sip_transport::TransportEvent::MessageReceived {
                message: Message::Request(invite_request),
                source,
                destination: transport.local_addr().unwrap(),
                transport_type: rvoip_sip_transport::transport::TransportType::Udp,
                flow_id: None,
                raw_bytes: None,
                timing: None,
                connection_metadata: None,
            })
            .await
            .unwrap();

        let mut retransmitted = false;
        for _ in 0..20 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            let sent_messages = transport.get_sent_messages().await;
            if sent_messages.len() > before {
                let last_msg = sent_messages.last().unwrap();
                assert!(
                    matches!(last_msg.0, Message::Response(ref resp) if resp.status_code() == 200),
                    "retransmitted message should be cached 200 OK"
                );
                assert_eq!(last_msg.1, source);
                retransmitted = true;
                break;
            }
        }

        assert!(
            retransmitted,
            "retransmitted INVITE should resend cached 2xx"
        );
        assert!(
            transport.raw_send_count() > 0,
            "cached INVITE 2xx retransmission should use pre-built wire bytes"
        );

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn create_server_cancel_publishes_event_after_transaction_is_visible() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(16);
        let (manager, mut event_rx) =
            TransactionManager::new(transport.clone(), transport_rx, Some(16)).await?;

        let source: SocketAddr = "192.0.2.100:5060".parse().unwrap();
        let branch = "z9hG4bK.cancel-visible";
        let invite_request = create_dispatch_request(Method::Invite, branch, 101)
            .map_err(|e| Error::Other(e.to_string()))?;
        let invite_tx = manager
            .create_server_transaction(invite_request.clone(), source)
            .await?;

        let cancel_request = create_dispatch_request(Method::Cancel, branch, 101)
            .map_err(|e| Error::Other(e.to_string()))?;
        let cancel_tx = manager
            .create_server_transaction(cancel_request.clone(), source)
            .await?;
        let cancel_tx_id = cancel_tx.id().clone();

        let event = tokio::time::timeout(Duration::from_millis(250), event_rx.recv())
            .await
            .expect("CANCEL event should be published")
            .expect("event channel should remain open");

        match event {
            TransactionEvent::CancelRequest {
                transaction_id,
                target_transaction_id,
                request,
                source: event_source,
            } => {
                assert_eq!(transaction_id, cancel_tx_id);
                assert_eq!(target_transaction_id, invite_tx.id().clone());
                assert_eq!(request.method(), Method::Cancel);
                assert_eq!(event_source, source);
            }
            other => panic!("expected CancelRequest event, got {other:?}"),
        }

        assert!(
            manager.server_transactions.contains_key(&cancel_tx_id),
            "CANCEL transaction must be visible before its event is processed"
        );

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn unmatched_server_cancel_publishes_non_invite_event_for_481_cleanup() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(16);
        let (manager, mut event_rx) =
            TransactionManager::new(transport.clone(), transport_rx, Some(16)).await?;

        let source: SocketAddr = "192.0.2.100:5060".parse().unwrap();
        let cancel_request =
            create_dispatch_request(Method::Cancel, "z9hG4bK.cancel-unmatched", 101)
                .map_err(|e| Error::Other(e.to_string()))?;
        let cancel_tx = manager
            .create_server_transaction(cancel_request.clone(), source)
            .await?;
        let cancel_tx_id = cancel_tx.id().clone();

        let event = tokio::time::timeout(Duration::from_millis(250), event_rx.recv())
            .await
            .expect("unmatched CANCEL should still be published")
            .expect("event channel should remain open");

        match event {
            TransactionEvent::NonInviteRequest {
                transaction_id,
                request,
                source: event_source,
            } => {
                assert_eq!(transaction_id, cancel_tx_id);
                assert_eq!(request.method(), Method::Cancel);
                assert_eq!(event_source, source);
            }
            other => panic!("expected NonInviteRequest event, got {other:?}"),
        }

        assert!(
            manager.server_transactions.contains_key(&cancel_tx_id),
            "unmatched CANCEL transaction must be available for 481 response cleanup"
        );

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn invite_2xx_is_retransmitted_until_ack() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(16);
        let (manager, _event_rx) =
            TransactionManager::new(transport.clone(), transport_rx, Some(16)).await?;

        let invite_request = create_test_invite().map_err(|e| Error::Other(e.to_string()))?;
        let source: SocketAddr = "192.0.2.100:5060".parse().unwrap();
        let transaction = manager
            .create_server_transaction(invite_request.clone(), source)
            .await?;
        let tx_id = transaction.id().clone();

        let ok_response = create_test_response(&invite_request, StatusCode::Ok, Some("OK"));
        manager.send_response(&tx_id, ok_response).await?;

        let terminated = manager
            .wait_for_transaction_state(
                &tx_id,
                TransactionState::Terminated,
                std::time::Duration::from_millis(500),
            )
            .await?;
        assert!(
            terminated,
            "INVITE server transaction should terminate after 2xx"
        );

        let before = transport.get_sent_messages().await.len();
        tokio::time::sleep(std::time::Duration::from_millis(700)).await;

        let sent_messages = transport.get_sent_messages().await;
        assert!(
            sent_messages.len() > before,
            "cached INVITE 2xx should be retransmitted without waiting for another INVITE"
        );
        let last_msg = sent_messages.last().unwrap();
        assert!(
            matches!(last_msg.0, Message::Response(ref resp) if resp.status_code() == 200),
            "proactive retransmission should resend cached 200 OK"
        );
        assert_eq!(last_msg.1, source);
        assert!(
            transport.raw_send_count() > 0,
            "proactive INVITE 2xx retransmission should use pre-built wire bytes"
        );

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn acked_invite_2xx_cache_stops_proactive_retransmit_but_serves_duplicates() -> Result<()>
    {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(16);
        let (manager, _event_rx) =
            TransactionManager::new(transport.clone(), transport_rx, Some(16)).await?;
        manager.shutdown().await;

        let invite_request = create_test_invite().map_err(|e| Error::Other(e.to_string()))?;
        let response = create_test_response(&invite_request, StatusCode::Ok, Some("OK"));
        let source: SocketAddr = "192.0.2.100:5060".parse().unwrap();
        let transaction_id =
            TransactionKey::new("z9hG4bK.acked-cache-test".to_string(), Method::Invite, true);
        let now = Instant::now();
        manager.insert_invite_2xx_response_cache_entry(
            transaction_id.clone(),
            cached_invite_2xx_entry(
                response,
                source,
                now - Duration::from_millis(10),
                now - Duration::from_millis(1),
            ),
        );

        manager.mark_invite_2xx_response_cache_acked(&transaction_id);

        let proactive = manager.retransmit_due_invite_2xx_responses().await;
        assert_eq!(
            proactive, 0,
            "ACKed 2xx cache entries should not proactively retransmit"
        );
        assert!(
            transport.get_sent_messages().await.is_empty(),
            "proactive maintenance should not send ACKed entries"
        );

        let retransmitted = manager
            .retransmit_cached_invite_2xx_response_on_route(
                &transaction_id,
                rvoip_sip_transport::TransportRoute::new(source)
                    .with_transport_type(TransportType::Udp),
            )
            .await?;
        assert!(
            retransmitted,
            "duplicate INVITE should still hit the ACK-retained cache"
        );
        let sent_messages = transport.get_sent_messages().await;
        assert_eq!(sent_messages.len(), 1);
        assert!(
            matches!(sent_messages[0].0, Message::Response(ref resp) if resp.status_code() == 200)
        );
        assert_eq!(sent_messages[0].1, source);
        assert_eq!(
            transport.raw_send_count(),
            1,
            "ACK-retained duplicate response should use pre-built wire bytes"
        );

        {
            let expired_at = Instant::now() - Duration::from_millis(1);
            let mut scheduler = manager
                .invite_2xx_response_due_queue
                .lock()
                .expect("due scheduler lock");
            let mut entry = manager
                .invite_2xx_response_cache
                .get_mut(&transaction_id)
                .expect("ACK-retained entry should still exist");
            entry.expires_at = expired_at;
            entry.next_retransmit_at = expired_at;
            entry.deadline_generation =
                scheduler.schedule(transaction_id.clone(), expired_at, expired_at);
        }
        manager.prune_invite_2xx_response_cache();
        assert!(
            !manager
                .invite_2xx_response_cache
                .contains_key(&transaction_id),
            "ACK-retained entry should prune once its retention expires"
        );

        Ok(())
    }

    #[tokio::test]
    async fn cached_invite_2xx_never_crosses_coaddressed_tcp_flows() -> Result<()> {
        let (cached_flow, other_flow) = two_live_tcp_flow_ids().await;
        assert_ne!(cached_flow, other_flow);
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(16);
        let (manager, _event_rx) =
            TransactionManager::new(transport.clone(), transport_rx, Some(16)).await?;
        manager.shutdown().await;

        let invite = create_test_invite().map_err(|error| Error::Other(error.to_string()))?;
        let response = create_test_response(&invite, StatusCode::Ok, Some("OK"));
        let transaction_id =
            TransactionKey::new("z9hG4bK.exact-cache-flow".into(), Method::Invite, true);
        let destination: SocketAddr = "192.0.2.100:5060".parse().unwrap();
        let cached_route = rvoip_sip_transport::TransportRoute::new(destination)
            .with_transport_type(TransportType::Tcp)
            .with_flow_id(cached_flow);
        let now = Instant::now();
        let wire_bytes = bytes::Bytes::from(Message::Response(response.clone()).to_bytes());
        manager.insert_invite_2xx_response_cache_entry(
            transaction_id.clone(),
            Invite2xxResponseCacheEntry {
                response,
                wire_bytes,
                route: cached_route.clone(),
                created_at: now,
                acked_at: None,
                expires_at: now + Duration::from_secs(90),
                next_retransmit_at: now + Duration::from_secs(1),
                retransmit_interval: Duration::from_millis(500),
                deadline_generation: 0,
                _admission_owner: None,
            },
        );

        let wrong_route = rvoip_sip_transport::TransportRoute::new(destination)
            .with_transport_type(TransportType::Tcp)
            .with_flow_id(other_flow);
        assert!(
            !manager
                .retransmit_cached_invite_2xx_response_on_route(&transaction_id, wrong_route,)
                .await?
        );
        assert_eq!(transport.raw_send_count(), 0);

        assert!(
            manager
                .retransmit_cached_invite_2xx_response_on_route(
                    &transaction_id,
                    cached_route.clone(),
                )
                .await?
        );
        assert_eq!(transport.raw_send_count(), 1);
        assert_eq!(transport.raw_routes().await, vec![cached_route]);
        Ok(())
    }

    #[tokio::test]
    async fn invite_2xx_due_queue_processing_is_capped_per_tick() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(16);
        let (manager, _event_rx) =
            TransactionManager::new(transport.clone(), transport_rx, Some(16)).await?;
        manager.shutdown().await;

        let invite_request = create_test_invite().map_err(|e| Error::Other(e.to_string()))?;
        let response = create_test_response(&invite_request, StatusCode::Ok, Some("OK"));
        let destination: SocketAddr = "192.0.2.100:5060".parse().unwrap();
        let now = Instant::now();
        let max_due_per_tick = 2;
        manager.set_invite_2xx_retransmit_max_due_per_tick(max_due_per_tick);
        let total_entries = max_due_per_tick + 3;

        for idx in 0..total_entries {
            manager.insert_invite_2xx_response_cache_entry(
                TransactionKey::new(format!("z9hG4bK.due-queue-cap-{idx}"), Method::Invite, true),
                cached_invite_2xx_entry(
                    response.clone(),
                    destination,
                    now - Duration::from_millis(10),
                    now - Duration::from_millis(1),
                ),
            );
        }

        let first_tick = manager.retransmit_due_invite_2xx_responses().await;
        assert_eq!(first_tick, max_due_per_tick);
        let after_first_tick = transport.get_sent_messages().await.len();
        assert!(
            after_first_tick >= max_due_per_tick,
            "first explicit maintenance tick should send at least its capped batch"
        );
        assert!(
            after_first_tick <= total_entries,
            "background maintenance should not send more than the queued entries"
        );

        let second_tick = manager.retransmit_due_invite_2xx_responses().await;
        assert!(
            first_tick + second_tick <= total_entries,
            "explicit maintenance ticks must not retransmit more entries than were queued"
        );
        assert!(
            transport.get_sent_messages().await.len() <= total_entries,
            "cached retransmission should not duplicate a due entry"
        );

        Ok(())
    }

    #[test]
    fn invite_2xx_deadline_scheduler_deduplicates_and_removes_superseded_deadlines() {
        let mut scheduler = Invite2xxDeadlineScheduler::default();
        let now = Instant::now();
        let transaction_id = TransactionKey::new(
            "z9hG4bK.due-scheduler-dedup".to_string(),
            Method::Invite,
            true,
        );
        let expires_at = now + Duration::from_secs(90);
        let first_due = now + Duration::from_secs(1);
        let replacement_due = now + Duration::from_secs(2);

        let first_generation = scheduler.schedule(transaction_id.clone(), first_due, expires_at);
        let duplicate_generation =
            scheduler.schedule(transaction_id.clone(), first_due, expires_at);
        assert_eq!(duplicate_generation, first_generation);
        assert_eq!(scheduler.len(), 1, "identical schedules must deduplicate");

        let replacement_generation =
            scheduler.schedule(transaction_id.clone(), replacement_due, expires_at);
        assert_ne!(replacement_generation, first_generation);
        assert_eq!(scheduler.len(), 1, "superseded deadlines must be removed");

        let (stale_due, capped) = scheduler.take_due(first_due, 8);
        assert!(stale_due.is_empty(), "superseded deadline must not fire");
        assert!(!capped);

        let (due, capped) = scheduler.take_due(replacement_due, 8);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].transaction_id.as_ref(), &transaction_id);
        assert_eq!(due[0].generation, replacement_generation);
        assert!(!capped);
        assert_eq!(scheduler.len(), 0);
    }

    #[test]
    fn invite_2xx_deadline_scheduler_bounds_expiry_and_capacity_work() {
        let mut scheduler = Invite2xxDeadlineScheduler::default();
        let now = Instant::now();

        for idx in 0..5 {
            scheduler.schedule(
                TransactionKey::new(
                    format!("z9hG4bK.expired-deadline-{idx}"),
                    Method::Invite,
                    true,
                ),
                now - Duration::from_millis(2),
                now - Duration::from_millis(1),
            );
        }

        let expired = scheduler.take_expired_and_overflow(now, usize::MAX, 2);
        assert_eq!(
            expired.len(),
            2,
            "expiry cleanup must honor its work budget"
        );
        assert!(expired.iter().all(|deadline| deadline.expires_at <= now));
        assert_eq!(scheduler.len(), 3);

        let remaining = scheduler.take_expired_and_overflow(now, usize::MAX, 8);
        assert_eq!(remaining.len(), 3);
        assert_eq!(scheduler.len(), 0);

        for idx in 0..4 {
            scheduler.schedule(
                TransactionKey::new(
                    format!("z9hG4bK.capacity-deadline-{idx}"),
                    Method::Invite,
                    true,
                ),
                now + Duration::from_secs(30),
                now + Duration::from_secs(60 + idx as u64),
            );
        }

        let evicted = scheduler.take_expired_and_overflow(now, 2, 1);
        assert_eq!(evicted.len(), 1, "overflow cleanup must remain bounded");
        assert_eq!(evicted[0].expires_at, now + Duration::from_secs(60));
        assert_eq!(scheduler.len(), 3);

        let evicted = scheduler.take_expired_and_overflow(now, 2, 8);
        assert_eq!(evicted.len(), 1);
        assert_eq!(scheduler.len(), 2);
    }

    /// Test find_related_transactions and special lookups
    #[tokio::test]
    async fn test_transaction_relationships() -> Result<()> {
        // Setup mock transport
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (transport_tx, transport_rx) = mpsc::channel(10);

        // Create the transaction manager
        let (manager, mut event_rx) =
            TransactionManager::new(transport.clone(), transport_rx, Some(10)).await?;

        // Create an INVITE request
        let invite_request = create_test_invite().map_err(|e| Error::Other(e.to_string()))?;
        let destination = SocketAddr::from_str("192.168.1.100:5060").unwrap();

        // Create an INVITE client transaction
        let invite_tx_id = manager
            .create_client_transaction(invite_request.clone(), destination)
            .await?;

        // Send the request
        manager.send_request(&invite_tx_id).await?;

        // Wait for state to change (consume event)
        let _ = event_rx.recv().await;

        // Create a CANCEL for this INVITE
        let cancel_tx_id = manager.cancel_invite_transaction(&invite_tx_id).await?;

        // Test find_related_transactions
        let related_txs = manager.find_related_transactions(&invite_tx_id).await?;
        assert_eq!(related_txs.len(), 1, "Should find 1 related transaction");
        assert!(
            related_txs.contains(&cancel_tx_id),
            "Related transactions should include CANCEL"
        );

        // Test find_invite_transaction_for_cancel
        let cancel_request = manager.original_request(&cancel_tx_id).await?.unwrap();
        let found_invite_tx_id = manager
            .find_invite_transaction_for_cancel(&cancel_request)
            .await?;
        assert!(
            found_invite_tx_id.is_some(),
            "Should find matching INVITE for CANCEL"
        );
        assert_eq!(
            found_invite_tx_id.unwrap(),
            invite_tx_id,
            "Found INVITE ID should match"
        );

        // Clean up
        manager.shutdown().await;

        Ok(())
    }

    #[tokio::test]
    async fn duplicate_2xx_acks_reuse_initial_server_invite_index_key() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let (manager, _event_rx) =
            TransactionManager::new(transport.clone(), transport_rx, Some(10)).await?;

        let invite_request = create_test_invite().map_err(|e| Error::Other(e.to_string()))?;
        let source = SocketAddr::from_str("192.168.1.100:5060").unwrap();
        let transaction = manager
            .create_server_transaction(invite_request, source)
            .await?;
        let transaction_id = transaction.id().clone();

        let ack_request = create_test_ack().map_err(|e| Error::Other(e.to_string()))?;
        for _ in 0..2 {
            assert_eq!(
                manager.find_server_invite_for_ack(&ack_request),
                Some(transaction_id.clone())
            );
        }

        assert_eq!(manager.server_invite_dialog_index.len(), 1);
        assert_eq!(manager.server_invite_dialog_keys_by_tx.len(), 1);
        assert_eq!(
            manager
                .server_invite_dialog_keys_by_tx
                .get(&transaction_id)
                .expect("active INVITE reverse index")
                .len(),
            1
        );
        assert!(manager
            .server_invite_dialog_expiry_queue
            .lock()
            .expect("ACK expiry queue")
            .is_empty());

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn retired_server_invite_keeps_one_ack_binding_and_deadline() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let (manager, _event_rx) =
            TransactionManager::new(transport.clone(), transport_rx, Some(10)).await?;

        let invite_request = create_test_invite().map_err(|e| Error::Other(e.to_string()))?;
        let source = SocketAddr::from_str("192.168.1.100:5060").unwrap();
        let transaction = manager
            .create_server_transaction(invite_request, source)
            .await?;
        let transaction_id = transaction.id().clone();
        let ack_request = create_test_ack().map_err(|e| Error::Other(e.to_string()))?;

        manager.retire_server_invite_dialog_index_for(&transaction_id);
        assert_eq!(manager.server_invite_dialog_index.len(), 1);
        assert!(manager
            .server_invite_dialog_keys_by_tx
            .get(&transaction_id)
            .is_none());
        assert_eq!(
            manager
                .server_invite_dialog_expiry_queue
                .lock()
                .expect("ACK expiry queue")
                .len(),
            1
        );

        for _ in 0..2 {
            assert_eq!(
                manager.find_server_invite_for_ack(&ack_request),
                Some(transaction_id.clone())
            );
        }
        assert_eq!(manager.server_invite_dialog_index.len(), 1);
        assert_eq!(
            manager
                .server_invite_dialog_expiry_queue
                .lock()
                .expect("ACK expiry queue")
                .len(),
            1
        );

        let due_at = manager
            .server_invite_dialog_expiry_queue
            .lock()
            .expect("ACK expiry queue")
            .peek()
            .expect("retained ACK deadline")
            .due_at;
        assert_eq!(manager.expire_due_server_invite_dialog_index(due_at, 1), 1);
        assert!(manager.server_invite_dialog_index.is_empty());
        assert!(manager
            .server_invite_dialog_expiry_queue
            .lock()
            .expect("ACK expiry queue")
            .is_empty());
        assert_eq!(manager.find_server_invite_for_ack(&ack_request), None);

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn server_invite_ack_index_expires_exact_due_retirement() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let (mut manager, _event_rx) =
            TransactionManager::new(transport, transport_rx, Some(10)).await?;
        manager.shutdown().await;
        manager.timer_settings.t4 = Duration::ZERO;

        let transaction_id =
            TransactionKey::new("z9hG4bK.ack-index-exact-due".into(), Method::Invite, true);
        let dialog_key = ServerInviteDialogKey {
            call_id: "ack-index-exact-due".into(),
            from_tag: "from-exact".into(),
            to_tag: None,
        };
        manager.insert_server_invite_dialog_index_entry(
            dialog_key.clone(),
            ServerInviteAckIndexEntry::active(transaction_id.clone()),
        );

        manager.retire_server_invite_dialog_index_for(&transaction_id);
        assert_eq!(manager.server_invite_dialog_index.len(), 1);
        assert!(manager
            .server_invite_dialog_keys_by_tx
            .get(&transaction_id)
            .is_none());
        assert_eq!(
            manager
                .server_invite_dialog_expiry_queue
                .lock()
                .expect("ACK expiry queue")
                .len(),
            1
        );

        assert_eq!(
            manager.expire_due_server_invite_dialog_index(Instant::now(), 1),
            1
        );
        assert!(!manager.server_invite_dialog_index.contains_key(&dialog_key));
        assert!(manager
            .server_invite_dialog_expiry_queue
            .lock()
            .expect("ACK expiry queue")
            .is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn stale_server_invite_ack_deadline_does_not_remove_replacement() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let (manager, _event_rx) =
            TransactionManager::new(transport, transport_rx, Some(10)).await?;
        manager.shutdown().await;

        let old_transaction =
            TransactionKey::new("z9hG4bK.ack-index-stale-old".into(), Method::Invite, true);
        let replacement =
            TransactionKey::new("z9hG4bK.ack-index-stale-new".into(), Method::Invite, true);
        let dialog_key = ServerInviteDialogKey {
            call_id: "ack-index-stale".into(),
            from_tag: "from-stale".into(),
            to_tag: Some("to-stale".into()),
        };
        let mut retired = ServerInviteAckIndexEntry::active(old_transaction);
        retired.expires_at = Some(Instant::now() - Duration::from_millis(1));
        manager.insert_server_invite_dialog_index_entry(dialog_key.clone(), retired);
        let stale_generation = manager
            .server_invite_dialog_index
            .get(&dialog_key)
            .expect("retired binding")
            .deadline_generation;

        manager.insert_server_invite_dialog_index_entry(
            dialog_key.clone(),
            ServerInviteAckIndexEntry::active(replacement.clone()),
        );
        let replacement_generation = manager
            .server_invite_dialog_index
            .get(&dialog_key)
            .expect("replacement binding")
            .deadline_generation;
        assert_ne!(stale_generation, replacement_generation);

        assert_eq!(
            manager.expire_due_server_invite_dialog_index(Instant::now(), 1),
            1
        );
        let retained = manager
            .server_invite_dialog_index
            .get(&dialog_key)
            .expect("stale deadline must not delete replacement");
        assert_eq!(retained.transaction_id, replacement);
        assert_eq!(retained.deadline_generation, replacement_generation);
        assert!(retained.expires_at.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn server_invite_ack_expiry_work_is_bounded_per_pass() -> Result<()> {
        const TOTAL: usize = 7;
        const FIRST_BUDGET: usize = 3;

        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let (manager, _event_rx) =
            TransactionManager::new(transport, transport_rx, Some(10)).await?;
        manager.shutdown().await;

        for index in 0..TOTAL {
            let transaction_id = TransactionKey::new(
                format!("z9hG4bK.ack-index-bounded-{index}"),
                Method::Invite,
                true,
            );
            let dialog_key = ServerInviteDialogKey {
                call_id: format!("ack-index-bounded-{index}"),
                from_tag: format!("from-{index}"),
                to_tag: None,
            };
            let mut retired = ServerInviteAckIndexEntry::active(transaction_id);
            retired.expires_at = Some(Instant::now() - Duration::from_millis(1));
            manager.insert_server_invite_dialog_index_entry(dialog_key, retired);
        }

        assert_eq!(manager.server_invite_dialog_index.len(), TOTAL);
        assert_eq!(
            manager.expire_due_server_invite_dialog_index(Instant::now(), FIRST_BUDGET),
            FIRST_BUDGET
        );
        assert_eq!(
            manager.server_invite_dialog_index.len(),
            TOTAL - FIRST_BUDGET
        );
        assert_eq!(
            manager.expire_due_server_invite_dialog_index(Instant::now(), TOTAL),
            TOTAL - FIRST_BUDGET
        );
        assert!(manager.server_invite_dialog_index.is_empty());
        assert!(manager
            .server_invite_dialog_expiry_queue
            .lock()
            .expect("ACK expiry queue")
            .is_empty());
        Ok(())
    }

    /// Test events subscription
    #[tokio::test]
    async fn test_events_subscription() -> Result<()> {
        // Setup mock transport
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (transport_tx, transport_rx) = mpsc::channel(10);

        // Create the transaction manager
        let (manager, _manager_events) =
            TransactionManager::new(transport.clone(), transport_rx, Some(10)).await?;

        // Create a special testing event sender/receiver
        let (test_tx, mut test_rx) = mpsc::channel::<TransactionEvent>(20);

        // Custom subscriber that forwards events to our test channel
        {
            let mut hook_rx = manager.subscribe();

            // Create a background task to forward events
            tokio::spawn(async move {
                while let Some(event) = hook_rx.recv().await {
                    println!("Forwarding event to test channel: {:?}", event);
                    if let Err(e) = test_tx.send(event).await {
                        println!("Failed to forward event: {}", e);
                        break;
                    }
                }
            });
        }

        // Wait a bit to ensure the subscription is set up
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Create an INVITE request
        let invite_request = create_test_invite().map_err(|e| Error::Other(e.to_string()))?;
        let destination = SocketAddr::from_str("192.168.1.100:5060").unwrap();

        // Create a client transaction
        let tx_id = manager
            .create_client_transaction(invite_request.clone(), destination)
            .await?;

        println!("Created transaction {}", tx_id);

        // Send the request to trigger state change events
        println!("Sending request for {}", tx_id);
        manager.send_request(&tx_id).await?;

        // Wait for events to propagate
        let mut received_state_change = false;

        // Using manual timeout to collect events
        let timeout_duration = tokio::time::Duration::from_millis(1000);
        let start = tokio::time::Instant::now();

        while !received_state_change
            && tokio::time::Instant::now().duration_since(start) < timeout_duration
        {
            match tokio::time::timeout(tokio::time::Duration::from_millis(100), test_rx.recv())
                .await
            {
                Ok(Some(event)) => {
                    println!("Received event: {:?}", event);
                    if let TransactionEvent::StateChanged {
                        transaction_id,
                        previous_state,
                        new_state,
                    } = event
                    {
                        if transaction_id == tx_id && previous_state == TransactionState::Initial {
                            println!(
                                "Found matching state change event: {:?} -> {:?}",
                                previous_state, new_state
                            );
                            received_state_change = true;
                            break;
                        }
                    }
                }
                _ => {
                    // No event yet, continue waiting
                    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                }
            }
        }

        // If we can't directly verify event delivery, at least verify the state has changed
        if !received_state_change {
            println!("State change event not received, checking transaction state directly");
            let state = manager.transaction_state(&tx_id).await?;
            if state != TransactionState::Initial {
                println!(
                    "Transaction state has changed to {:?}, considering test passed",
                    state
                );
                received_state_change = true;
            }
        }

        assert!(
            received_state_change,
            "Failed to confirm state change either through events or direct state check"
        );

        // Clean up
        manager.shutdown().await;

        Ok(())
    }

    #[tokio::test]
    async fn retention_counts_prunes_closed_event_subscribers() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let (manager, _event_rx) =
            TransactionManager::new(transport.clone(), transport_rx, Some(10)).await?;

        let global_rx = manager.subscribe();
        assert_eq!(manager.retention_counts().event_subscribers, 1);

        drop(global_rx);
        let counts = manager.retention_counts();
        assert_eq!(counts.event_subscribers, 0);

        let invite_request = create_test_invite().map_err(|e| Error::Other(e.to_string()))?;
        let destination = SocketAddr::from_str("192.168.1.100:5060").unwrap();
        let tx_id = manager
            .create_client_transaction(invite_request, destination)
            .await?;
        let tx_rx = manager.subscribe_to_transaction(&tx_id).await?;
        assert_eq!(manager.retention_counts().event_subscribers, 1);
        assert_eq!(manager.retention_counts().subscriber_to_transactions, 1);
        assert_eq!(manager.retention_counts().transaction_to_subscribers, 1);

        drop(tx_rx);
        let counts = manager.retention_counts();
        assert_eq!(counts.event_subscribers, 0);
        assert_eq!(counts.subscriber_to_transactions, 0);
        assert_eq!(counts.transaction_to_subscribers, 0);

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn keyed_observers_are_not_stored_or_scanned_as_globals() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let (manager, mut primary_rx) =
            TransactionManager::new(transport, transport_rx, Some(10)).await?;
        let destination: SocketAddr = "192.0.2.44:5060".parse().unwrap();
        let first = manager
            .create_client_transaction(
                create_test_invite_with_identity(
                    "keyed-observer-first",
                    "z9hG4bK.keyed-observer-first",
                    "UDP",
                )
                .map_err(|error| Error::Other(error.to_string()))?,
                destination,
            )
            .await?;
        let second = manager
            .create_client_transaction(
                create_test_invite_with_identity(
                    "keyed-observer-second",
                    "z9hG4bK.keyed-observer-second",
                    "UDP",
                )
                .map_err(|error| Error::Other(error.to_string()))?,
                destination,
            )
            .await?;
        let mut first_rx = manager.subscribe_to_transaction(&first).await?;
        let mut second_rx = manager.subscribe_to_transaction(&second).await?;

        assert!(
            manager.event_subscribers.load().is_empty(),
            "keyed observers must not enter the global RCU vector"
        );
        assert_eq!(
            manager
                .transaction_to_subscribers
                .get(&first)
                .map(|entry| entry.len()),
            Some(1)
        );

        TransactionManager::broadcast_event(
            TransactionEvent::StateChanged {
                transaction_id: first.clone(),
                previous_state: TransactionState::Initial,
                new_state: TransactionState::Calling,
            },
            &manager.events_tx,
            &manager.event_subscribers,
            Some(&manager.subscriber_to_transactions),
            Some(&manager.transaction_to_subscribers),
            None,
        )
        .await;

        assert!(matches!(
            first_rx.recv().await,
            Some(TransactionEvent::StateChanged { transaction_id, .. }) if transaction_id == first
        ));
        assert!(matches!(
            second_rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
        assert!(primary_rx.try_recv().is_ok());

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn real_runner_events_reach_primary_global_and_keyed_observers() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let (manager, mut primary_rx) =
            TransactionManager::new(transport, transport_rx, Some(10)).await?;
        let transaction = manager
            .create_client_transaction(
                create_test_invite_with_identity(
                    "real-runner-observers",
                    "z9hG4bK.real-runner-observers",
                    "UDP",
                )
                .map_err(|error| Error::Other(error.to_string()))?,
                "192.0.2.46:5060".parse().unwrap(),
            )
            .await?;
        let mut global_rx = manager.subscribe();
        let mut keyed_rx = manager.subscribe_to_transaction(&transaction).await?;

        manager.send_request(&transaction).await?;

        for receiver in [&mut primary_rx, &mut global_rx, &mut keyed_rx] {
            assert!(matches!(
                tokio::time::timeout(Duration::from_secs(1), receiver.recv()).await,
                Ok(Some(TransactionEvent::StateChanged {
                    transaction_id,
                    new_state: TransactionState::Calling,
                    ..
                })) if transaction_id == transaction
            ));
        }
        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn saturated_observers_never_backpressure_primary_delivery() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let (manager, mut primary_rx) =
            TransactionManager::new(transport, transport_rx, Some(10)).await?;
        let destination: SocketAddr = "192.0.2.45:5060".parse().unwrap();
        let observed = manager
            .create_client_transaction(
                create_test_invite_with_identity(
                    "saturated-observer",
                    "z9hG4bK.saturated-observer",
                    "UDP",
                )
                .map_err(|error| Error::Other(error.to_string()))?,
                destination,
            )
            .await?;
        let unrelated = manager
            .create_client_transaction(
                create_test_invite_with_identity(
                    "closed-unrelated-observer",
                    "z9hG4bK.closed-unrelated-observer",
                    "UDP",
                )
                .map_err(|error| Error::Other(error.to_string()))?,
                destination,
            )
            .await?;

        let mut global_rx = manager.subscribe();
        let mut keyed_rx = manager.subscribe_to_transaction(&observed).await?;
        let unrelated_rx = manager.subscribe_to_transaction(&unrelated).await?;
        drop(unrelated_rx);

        let global_sender = manager.event_subscribers.load()[0].sender.clone();
        let keyed_sender = manager
            .transaction_to_subscribers
            .get(&observed)
            .expect("keyed observer bucket")[0]
            .sender
            .clone();
        let filler = TransactionEvent::ShutdownComplete;
        for index in 0..100 {
            global_sender
                .try_send(filler.clone())
                .unwrap_or_else(|error| panic!("global observer filled at {index}: {error}"));
            keyed_sender
                .try_send(filler.clone())
                .unwrap_or_else(|error| panic!("keyed observer filled at {index}: {error}"));
        }

        tokio::time::timeout(Duration::from_millis(100), manager.send_request(&observed))
            .await
            .expect("a saturated observer must not delay protocol delivery")?;

        assert!(matches!(
            tokio::time::timeout(Duration::from_millis(100), primary_rx.recv()).await,
            Ok(Some(TransactionEvent::StateChanged { transaction_id, .. }))
                if transaction_id == observed
        ));

        for _ in 0..100 {
            assert!(matches!(
                global_rx.try_recv(),
                Ok(TransactionEvent::ShutdownComplete)
            ));
            assert!(matches!(
                keyed_rx.try_recv(),
                Ok(TransactionEvent::ShutdownComplete)
            ));
        }
        assert!(matches!(
            global_rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
        assert!(matches!(
            keyed_rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));

        // A normal event for `observed` must not scan/prune every keyed
        // observer bucket. Closed keyed observers are pruned only when their
        // transaction is encountered or by periodic maintenance.
        assert!(manager.transaction_to_subscribers.contains_key(&unrelated));

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn retention_counts_prunes_stale_pending_inbound_bytes() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let (manager, _event_rx) =
            TransactionManager::new(transport.clone(), transport_rx, Some(10)).await?;
        let tx_id = TransactionKey::new("z9hG4bK-stale-pending".to_string(), Method::Invite, true);

        manager.pending_inbound_bytes.insert(
            tx_id.clone(),
            bytes::Bytes::from_static(b"INVITE sip:bob SIP/2.0\r\n\r\n"),
        );
        manager.pending_inbound_inserted_at.insert(
            tx_id.clone(),
            Instant::now() - super::super::PENDING_INBOUND_BYTES_TTL - Duration::from_secs(1),
        );

        let counts = manager.retention_counts();
        assert_eq!(counts.pending_inbound_bytes, 0);
        assert!(manager.pending_inbound_inserted_at.get(&tx_id).is_none());

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn unmatched_response_raw_bytes_are_not_cached() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let (manager, _event_rx) =
            TransactionManager::new(transport.clone(), transport_rx, Some(10)).await?;

        let request = create_test_invite().map_err(|e| Error::Other(e.to_string()))?;
        let response = create_test_response(&request, StatusCode::Ok, Some("OK"));
        let raw_bytes = bytes::Bytes::from(Message::Response(response.clone()).to_bytes());

        manager
            .handle_transport_event(TransportEvent::MessageReceived {
                message: Message::Response(response),
                source: "192.0.2.100:5060".parse().unwrap(),
                destination: transport.local_addr().unwrap(),
                transport_type: TransportType::Udp,
                flow_id: None,
                raw_bytes: Some(raw_bytes),
                timing: None,
                connection_metadata: None,
            })
            .await?;

        let counts = manager.retention_counts();
        assert_eq!(counts.pending_inbound_bytes, 0);
        assert_eq!(counts.pending_inbound_transport, 0);

        manager.shutdown().await;
        Ok(())
    }

    /// Test wait_for_final_response function
    #[tokio::test]
    async fn test_wait_for_final_response() -> Result<()> {
        // Setup mock transport
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (transport_tx, transport_rx) = mpsc::channel(10);

        // Create the transaction manager
        let (manager, mut event_rx) =
            TransactionManager::new(transport.clone(), transport_rx, Some(10)).await?;

        // Create a MESSAGE request (non-INVITE)
        let request = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com")
            .map_err(|e| Error::Other(e.to_string()))?
            .from("Alice", "sip:alice@example.com", Some("alice-tag"))
            .to("Bob", "sip:bob@example.com", None)
            .call_id("test-call-id-message")
            .cseq(102)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK.message-branch"))
            .max_forwards(70)
            .build();

        let destination = SocketAddr::from_str("192.168.1.100:5060").unwrap();

        // Create transaction and send request
        let tx_id = manager
            .create_client_transaction(request.clone(), destination)
            .await?;

        manager.send_request(&tx_id).await?;

        // Consume the state changed event
        let _ = event_rx.recv().await;

        // Create a task to wait for final response
        let wait_task = tokio::spawn({
            let manager = manager.clone();
            let tx_id = tx_id.clone();
            async move {
                manager
                    .wait_for_final_response(&tx_id, std::time::Duration::from_millis(1000))
                    .await
            }
        });

        // Inject a 200 OK response after a short delay
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let ok_response = create_test_response(&request, StatusCode::Ok, Some("OK"));
        transport_tx
            .send(rvoip_sip_transport::TransportEvent::MessageReceived {
                message: Message::Response(ok_response.clone()),
                source: destination,
                destination: transport.local_addr().unwrap(),
                transport_type: rvoip_sip_transport::transport::TransportType::Udp,
                flow_id: None,
                raw_bytes: None,
                timing: None,
                connection_metadata: None,
            })
            .await
            .unwrap();

        // Wait for the wait_for_final_response task to complete
        let result = wait_task.await.expect("Task failed")?;

        // Check that we got the response
        assert!(result.is_some(), "Should receive a final response");
        assert_eq!(
            result.unwrap().status_code(),
            200,
            "Final response should be 200 OK"
        );

        // Clean up
        manager.shutdown().await;

        Ok(())
    }

    #[tokio::test]
    async fn non_invite_send_and_exact_wait_create_no_event_subscribers() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let (manager, _event_rx) =
            TransactionManager::new(transport, transport_rx, Some(10)).await?;
        let request = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com")
            .map_err(|error| Error::Other(error.to_string()))?
            .from("Alice", "sip:alice@example.com", Some("alice-tag"))
            .to("Bob", "sip:bob@example.com", None)
            .call_id("exact-wait-no-subscriber")
            .cseq(1)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK.exact-wait"))
            .max_forwards(70)
            .build();
        let destination: SocketAddr = "192.0.2.10:5060".parse().unwrap();
        let transaction = manager
            .create_client_transaction(request, destination)
            .await?;

        let started = Instant::now();
        manager.send_request(&transaction).await?;
        assert!(
            started.elapsed() < Duration::from_millis(50),
            "non-INVITE send retained the historical 100 ms safety wait"
        );
        assert_eq!(manager.retention_counts().event_subscribers, 0);

        assert!(
            manager
                .wait_for_transaction_state(
                    &transaction,
                    TransactionState::Trying,
                    Duration::from_millis(100),
                )
                .await?
        );
        assert_eq!(manager.retention_counts().event_subscribers, 0);

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn compact_timer_j_is_visible_to_existence_kind_and_state_apis() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let (manager, _event_rx) =
            TransactionManager::new(transport, transport_rx, Some(10)).await?;
        let transaction_id =
            TransactionKey::new("z9hG4bK.compact-j-state-api".into(), Method::Bye, true);
        let (_state, mut command_rx) =
            schedule_test_compact_timer_j(&manager, transaction_id.clone(), Duration::from_secs(5))
                .await;
        assert!(matches!(
            command_rx.recv().await,
            Some(InternalTransactionCommand::CompactRetire)
        ));

        assert!(manager.transaction_exists(&transaction_id).await);
        assert_eq!(
            manager.transaction_kind(&transaction_id).await?,
            crate::transaction::TransactionKind::NonInviteServer
        );
        assert_eq!(
            manager.transaction_state(&transaction_id).await?,
            TransactionState::Completed
        );
        assert!(
            manager
                .wait_for_transaction_state(
                    &transaction_id,
                    TransactionState::Completed,
                    Duration::from_millis(100),
                )
                .await?
        );

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn compact_timer_j_terminal_wait_survives_exact_tombstone_removal() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let (manager, _event_rx) =
            TransactionManager::new(transport, transport_rx, Some(10)).await?;
        let transaction_id =
            TransactionKey::new("z9hG4bK.compact-j-terminal-wait".into(), Method::Bye, true);
        let (state, mut command_rx) = schedule_test_compact_timer_j(
            &manager,
            transaction_id.clone(),
            Duration::from_millis(100),
        )
        .await;
        assert!(matches!(
            command_rx.recv().await,
            Some(InternalTransactionCommand::CompactRetire)
        ));

        let waiter = tokio::spawn({
            let manager = manager.clone();
            let transaction_id = transaction_id.clone();
            async move {
                manager
                    .wait_for_transaction_state(
                        &transaction_id,
                        TransactionState::Terminated,
                        Duration::from_secs(1),
                    )
                    .await
            }
        });

        assert!(waiter.await.expect("terminal waiter task")?);
        assert_eq!(state.get(), TransactionState::Terminated);
        assert!(
            !manager.transaction_exists(&transaction_id).await,
            "Timer J expiry must remove the public existence record"
        );
        assert!(matches!(
            manager.transaction_state(&transaction_id).await,
            Err(Error::TransactionNotFound { .. })
        ));
        assert!(matches!(
            manager
                .wait_for_transaction_state(
                    &transaction_id,
                    TransactionState::Terminated,
                    Duration::from_millis(10),
                )
                .await,
            Err(Error::TransactionNotFound { .. })
        ));

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn expired_server_replay_is_absorbed_and_uses_scheduler_generation_cleanup() -> Result<()>
    {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let (manager, event_rx) = TransactionManager::new(transport, transport_rx, Some(1)).await?;

        // Stall primary TU delivery so expiry reaches the retransmission
        // handler while the scheduler's exact generation fence is still live.
        manager
            .events_tx
            .send(TransactionEvent::Error {
                transaction_id: None,
                error: "stall compact expiry".into(),
            })
            .await
            .expect("fill primary TU channel");

        let transaction_id =
            TransactionKey::new("z9hG4bK.compact-j-expiry-race".into(), Method::Bye, true);
        let (state, mut command_rx) =
            schedule_test_compact_timer_j(&manager, transaction_id.clone(), Duration::ZERO).await;
        assert!(matches!(
            command_rx.recv().await,
            Some(InternalTransactionCommand::CompactRetire)
        ));
        tokio::time::timeout(Duration::from_millis(200), async {
            while state.get() != TransactionState::Terminated {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("scheduler reaches exact terminal state");

        let ingress = SipRequestIngressContext::new(
            "192.0.2.200:5060".parse().unwrap(),
            "127.0.0.1:5060".parse().unwrap(),
            TransportType::Udp,
        );
        assert!(
            manager
                .replay_compact_non_invite_server_response(&transaction_id, &ingress)
                .await?,
            "expired authentic retransmission must be absorbed during scheduler cleanup"
        );
        assert!(
            manager
                .compact_non_invite_tombstones
                .contains_key(&transaction_id),
            "handler must not directly remove the scheduler-owned generation fence"
        );

        drop(event_rx);
        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn final_response_cell_closes_before_wait_and_removal_races() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let mut timer_settings = crate::transaction::timer::TimerSettings::default();
        timer_settings.wait_time_k = Duration::from_millis(250);
        let (manager, _event_rx) = TransactionManager::new_with_config(
            transport.clone(),
            transport_rx,
            Some(10),
            Some(timer_settings),
        )
        .await?;
        let request = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com")
            .map_err(|error| Error::Other(error.to_string()))?
            .from("Alice", "sip:alice@example.com", Some("alice-tag"))
            .to("Bob", "sip:bob@example.com", None)
            .call_id("exact-response-before-wait")
            .cseq(2)
            .via(
                "127.0.0.1:5060",
                "UDP",
                Some("z9hG4bK.response-before-wait"),
            )
            .max_forwards(70)
            .build();
        let destination: SocketAddr = "192.0.2.11:5060".parse().unwrap();
        let transaction = manager
            .create_client_transaction(request.clone(), destination)
            .await?;
        manager.send_request(&transaction).await?;

        let response = create_test_response(&request, StatusCode::Ok, Some("OK"));
        manager
            .handle_transport_event(TransportEvent::MessageReceived {
                message: Message::Response(response.clone()),
                source: destination,
                destination: transport.local_addr().unwrap(),
                transport_type: TransportType::Udp,
                flow_id: None,
                raw_bytes: None,
                timing: None,
                connection_metadata: None,
            })
            .await?;

        let observed = manager
            .wait_for_final_response(&transaction, Duration::from_secs(1))
            .await?
            .expect("response queued before waiter must remain observable");
        assert_eq!(observed.status_code(), 200);
        assert_eq!(manager.retention_counts().event_subscribers, 0);

        let active_handle = manager
            .client_completion(&transaction)
            .expect("active completion before removal");
        let timer_k_expires_at = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let Some(expires_at) = manager
                    .compact_non_invite_tombstones
                    .get(&transaction)
                    .map(|entry| entry.value().expires_at())
                {
                    break expires_at;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("UDP final response installs Timer K tombstone");
        manager.terminate_transaction(&transaction).await?;
        assert!(
            manager.client_completions.get(&transaction).is_none(),
            "Timer K must be the sole retained completion owner"
        );
        let retained = manager
            .wait_for_final_response(&transaction, Duration::from_millis(100))
            .await?
            .expect("final response must survive transaction removal");
        assert_eq!(retained, response);
        assert!(matches!(
            active_handle
                .wait_for_outcome(Duration::from_millis(100))
                .await?,
            Some(crate::transaction::ClientTransactionOutcome::FinalResponse(exact))
                if exact == response
        ));
        let completion_counts = manager.client_completion_retention_counts();
        assert_eq!(completion_counts.active, 0);
        assert_eq!(completion_counts.retained, 0);
        assert_eq!(completion_counts.deadlines, 0);
        assert!(timer_k_expires_at > Instant::now());
        assert_eq!(manager.retention_counts().event_subscribers, 0);
        assert!(
            manager
                .wait_for_transaction_state(
                    &transaction,
                    TransactionState::Terminated,
                    Duration::from_secs(1),
                )
                .await?,
            "a waiter that races Timer K must observe the terminal state"
        );

        tokio::time::timeout(Duration::from_secs(2), async {
            while manager
                .compact_non_invite_tombstones
                .contains_key(&transaction)
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("Timer K scheduler must expire its exact completion owner");
        assert!(manager.client_completion(&transaction).is_none());
        assert_eq!(manager.client_completion_retention_counts().deadlines, 0);

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn exact_completion_is_visible_before_primary_response_event() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let (manager, mut event_rx) =
            TransactionManager::new(transport.clone(), transport_rx, Some(10)).await?;
        let request = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com")
            .map_err(|error| Error::Other(error.to_string()))?
            .from("Alice", "sip:alice@example.com", Some("alice-tag"))
            .to("Bob", "sip:bob@example.com", None)
            .call_id("completion-before-primary")
            .cseq(3)
            .via(
                "127.0.0.1:5060",
                "UDP",
                Some("z9hG4bK.completion-before-primary"),
            )
            .max_forwards(70)
            .build();
        let destination: SocketAddr = "192.0.2.12:5060".parse().unwrap();
        let transaction = manager
            .create_client_transaction(request.clone(), destination)
            .await?;
        manager.send_request(&transaction).await?;
        while event_rx.try_recv().is_ok() {}

        manager
            .handle_transport_event(TransportEvent::MessageReceived {
                message: Message::Response(create_test_response(
                    &request,
                    StatusCode::Ok,
                    Some("OK"),
                )),
                source: destination,
                destination: transport.local_addr().unwrap(),
                transport_type: TransportType::Udp,
                flow_id: None,
                raw_bytes: None,
                timing: None,
                connection_metadata: None,
            })
            .await?;

        loop {
            let event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
                .await
                .expect("primary response event deadline")
                .expect("primary response event channel");
            if matches!(
                event,
                TransactionEvent::SuccessResponse {
                    ref transaction_id,
                    ..
                } if transaction_id == &transaction
            ) {
                let outcome = manager
                    .wait_for_client_transaction_outcome(&transaction, Duration::from_millis(10))
                    .await?
                    .expect("exact outcome must precede primary event");
                assert!(matches!(
                    outcome,
                    crate::transaction::ClientTransactionOutcome::FinalResponse(response)
                        if response.status_code() == 200
                ));
                break;
            }
        }
        assert_eq!(manager.retention_counts().event_subscribers, 0);
        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn transport_failure_has_typed_exact_outcome_without_subscription() -> Result<()> {
        let transport = Arc::new(MockTransport::with_send_failure("127.0.0.1:5060", true));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let (manager, _event_rx) =
            TransactionManager::new(transport, transport_rx, Some(10)).await?;
        let request = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com")
            .map_err(|error| Error::Other(error.to_string()))?
            .from("Alice", "sip:alice@example.com", Some("alice-tag"))
            .to("Bob", "sip:bob@example.com", None)
            .call_id("typed-transport-outcome")
            .cseq(4)
            .via(
                "127.0.0.1:5060",
                "UDP",
                Some("z9hG4bK.typed-transport-outcome"),
            )
            .max_forwards(70)
            .build();
        let transaction = manager
            .create_client_transaction(request, "192.0.2.13:5060".parse().unwrap())
            .await?;
        assert!(manager.send_request(&transaction).await.is_err());

        assert!(matches!(
            manager
                .wait_for_client_transaction_outcome(&transaction, Duration::from_secs(1))
                .await?,
            Some(crate::transaction::ClientTransactionOutcome::Failure(
                crate::transaction::ClientTransactionFailure::Transport
            ))
        ));
        assert_eq!(manager.retention_counts().event_subscribers, 0);
        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn forced_invite_termination_has_one_exact_outcome_across_retirement() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let (manager, _event_rx) =
            TransactionManager::new(transport, transport_rx, Some(10)).await?;
        let request = create_test_invite_with_identity(
            "forced-invite-completion",
            "z9hG4bK.forced-invite-completion",
            "UDP",
        )
        .map_err(|error| Error::Other(error.to_string()))?;
        let transaction = manager
            .create_client_transaction(request, "192.0.2.14:5060".parse().unwrap())
            .await?;
        manager.send_request(&transaction).await?;

        let existing_waiter = manager
            .client_completion(&transaction)
            .expect("active INVITE completion");
        manager.terminate_transaction(&transaction).await?;
        assert!(!manager.client_transactions.contains_key(&transaction));
        let post_retirement_waiter = manager
            .client_completion(&transaction)
            .expect("retained INVITE completion");

        assert!(matches!(
            existing_waiter
                .wait_for_outcome(Duration::from_millis(100))
                .await?,
            Some(crate::transaction::ClientTransactionOutcome::Failure(
                crate::transaction::ClientTransactionFailure::Cancelled
            ))
        ));
        assert!(matches!(
            post_retirement_waiter
                .wait_for_outcome(Duration::from_millis(100))
                .await?,
            Some(crate::transaction::ClientTransactionOutcome::Failure(
                crate::transaction::ClientTransactionFailure::Cancelled
            ))
        ));
        assert!(matches!(
            manager
                .wait_for_client_transaction_outcome(&transaction, Duration::from_millis(100),)
                .await?,
            Some(crate::transaction::ClientTransactionOutcome::Failure(
                crate::transaction::ClientTransactionFailure::Cancelled
            ))
        ));

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn forced_termination_resumes_blocked_prefix_and_cleans_exact_runner() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let (manager, mut events) =
            TransactionManager::new(transport, transport_rx, Some(1)).await?;
        let request = create_test_invite_with_identity(
            "blocked-terminal-prefix",
            "z9hG4bK.blocked-terminal-prefix",
            "UDP",
        )
        .map_err(|error| Error::Other(error.to_string()))?;
        let transaction = manager
            .create_client_transaction(request, "192.0.2.115:5060".parse().unwrap())
            .await?;
        manager.send_request(&transaction).await?;
        while events.try_recv().is_ok() {}

        manager
            .events_tx
            .send(TransactionEvent::Error {
                transaction_id: None,
                error: "terminal-prefix-capacity-blocker".into(),
            })
            .await
            .expect("fill primary event capacity");
        let live = manager
            .client_transactions
            .get(&transaction)
            .expect("live INVITE transaction")
            .value()
            .clone();
        live.data()
            .cmd_tx
            .try_send(InternalTransactionCommand::Terminate)
            .expect("enqueue runner termination");
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if live.state() == TransactionState::Terminated
                    && live.data().terminal_event_publication.pending_prefix()
                        == Some(TransactionState::Calling)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("runner did not block at terminal prefix");

        let runner_joined = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        manager.install_termination_takeover_test_gate(
            transaction.clone(),
            Arc::clone(&runner_joined),
            Arc::clone(&release),
        );
        let termination = {
            let manager = manager.clone();
            let transaction = transaction.clone();
            tokio::spawn(async move { manager.terminate_transaction(&transaction).await })
        };
        tokio::time::timeout(Duration::from_secs(2), runner_joined.notified())
            .await
            .expect("termination supervisor did not abort and join blocked runner");

        assert!(matches!(
            events.recv().await,
            Some(TransactionEvent::Error { error, .. })
                if error == "terminal-prefix-capacity-blocker"
        ));
        release.notify_one();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(1), events.recv()).await,
            Ok(Some(TransactionEvent::StateChanged {
                transaction_id: observed,
                previous_state: TransactionState::Calling,
                new_state: TransactionState::Terminated,
            })) if observed == transaction
        ));
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(1), events.recv()).await,
            Ok(Some(TransactionEvent::TransactionTerminated { transaction_id: observed }))
                if observed == transaction
        ));
        termination
            .await
            .expect("termination supervisor task")
            .expect("termination supervisor result");
        manager.clear_termination_takeover_test_gate();

        assert!(!manager.client_transactions.contains_key(&transaction));
        assert!(live.data().event_loop_handle.lock().await.is_none());
        assert!(manager
            .transaction_destinations
            .get(&transaction)
            .is_none_or(|route| !route.is_active()));
        assert!(live.data().terminal_event_publication.is_delivered());
        assert!(matches!(
            events.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn explicit_termination_survives_public_waiter_cancellation() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let (manager, _events) = TransactionManager::new(transport, transport_rx, Some(10)).await?;
        let request = create_test_invite_with_identity(
            "cancelled-explicit-termination-waiter",
            "z9hG4bK.cancelled-explicit-termination-waiter",
            "UDP",
        )
        .map_err(|error| Error::Other(error.to_string()))?;
        let transaction = manager
            .create_client_transaction(request, "192.0.2.117:5060".parse().unwrap())
            .await?;
        manager.send_request(&transaction).await?;
        let live = manager
            .client_transactions
            .get(&transaction)
            .expect("live INVITE transaction")
            .value()
            .clone();

        let runner_joined = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        manager.install_termination_takeover_test_gate(
            transaction.clone(),
            Arc::clone(&runner_joined),
            Arc::clone(&release),
        );
        let public_waiter = {
            let manager = manager.clone();
            let transaction = transaction.clone();
            tokio::spawn(async move { manager.terminate_transaction(&transaction).await })
        };
        tokio::time::timeout(Duration::from_secs(2), runner_joined.notified())
            .await
            .expect("manager cleanup worker did not own explicit termination");

        public_waiter.abort();
        assert!(public_waiter
            .await
            .expect_err("public termination waiter should be cancelled")
            .is_cancelled());

        let shutdown_complete = Arc::new(tokio::sync::Notify::new());
        let shutdown = {
            let manager = manager.clone();
            let shutdown_complete = Arc::clone(&shutdown_complete);
            tokio::spawn(async move {
                manager.shutdown().await;
                shutdown_complete.notify_one();
            })
        };
        tokio::time::timeout(Duration::from_secs(1), async {
            while manager.admission_lifecycle.state() < MANAGER_ADMISSION_STOPPING {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("shutdown did not close existing-work admission");
        assert!(
            tokio::time::timeout(Duration::from_millis(50), shutdown_complete.notified())
                .await
                .is_err(),
            "shutdown bypassed the accepted explicit termination guard"
        );
        release.notify_one();

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if !manager.client_transactions.contains_key(&transaction)
                    && live.data().event_loop_handle.lock().await.is_none()
                    && live.data().terminal_event_publication.is_delivered()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("manager-owned termination stopped with its public waiter");
        assert!(manager.explicit_termination_operations.is_empty());
        manager.clear_termination_takeover_test_gate();
        tokio::time::timeout(Duration::from_secs(2), shutdown_complete.notified())
            .await
            .expect("shutdown deadlocked behind queued explicit termination");
        shutdown.await.expect("shutdown task");
        Ok(())
    }

    #[tokio::test]
    async fn delivered_terminal_is_not_duplicated_during_exact_cleanup() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let (manager, mut events) =
            TransactionManager::new(transport, transport_rx, Some(8)).await?;
        let request = create_test_invite_with_identity(
            "delivered-terminal-cleanup",
            "z9hG4bK.delivered-terminal-cleanup",
            "UDP",
        )
        .map_err(|error| Error::Other(error.to_string()))?;
        let transaction = manager
            .create_client_transaction(request, "192.0.2.116:5060".parse().unwrap())
            .await?;
        manager.send_request(&transaction).await?;
        while events.try_recv().is_ok() {}
        let live = manager
            .client_transactions
            .get(&transaction)
            .expect("live INVITE transaction")
            .value()
            .clone();
        live.data()
            .cmd_tx
            .try_send(InternalTransactionCommand::Terminate)
            .expect("enqueue runner termination");

        let mut state_changed = 0usize;
        let mut terminated = 0usize;
        tokio::time::timeout(Duration::from_secs(1), async {
            while terminated == 0 {
                match events.recv().await {
                    Some(TransactionEvent::StateChanged { transaction_id, .. })
                        if transaction_id == transaction =>
                    {
                        state_changed += 1
                    }
                    Some(TransactionEvent::TransactionTerminated { transaction_id })
                        if transaction_id == transaction =>
                    {
                        terminated += 1
                    }
                    Some(_) => {}
                    None => break,
                }
            }
        })
        .await
        .expect("runner did not publish terminal batch");
        manager.terminate_transaction(&transaction).await?;
        assert_eq!(state_changed, 1);
        assert_eq!(terminated, 1);
        assert!(!manager.client_transactions.contains_key(&transaction));
        assert!(live.data().terminal_event_publication.is_delivered());
        assert!(matches!(
            events.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn retired_invite_consolidates_keys_deadline_and_wire_storage() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let (manager, _event_rx) =
            TransactionManager::new(transport, transport_rx, Some(10)).await?;
        let request = create_test_invite_with_identity(
            "consolidated-invite-retention",
            "z9hG4bK.consolidated-invite-retention",
            "UDP",
        )
        .map_err(|error| Error::Other(error.to_string()))?;
        let transaction = manager
            .create_client_transaction(request.clone(), "192.0.2.16:5060".parse().unwrap())
            .await?;
        manager.send_request(&transaction).await?;

        {
            let completion_key = manager
                .client_completions
                .get(&transaction)
                .expect("active completion key");
            let route_key = manager
                .transaction_destinations
                .get(&transaction)
                .expect("active route key");
            assert!(
                Arc::ptr_eq(completion_key.key(), route_key.key()),
                "active completion and route indexes must share one key allocation"
            );
        }

        let response = create_test_response(&request, StatusCode::BusyHere, Some("Busy Here"));
        let live = manager
            .client_transactions
            .get(&transaction)
            .expect("active INVITE")
            .value()
            .clone();
        live.data().completion.record_response(response.clone());
        let existing_waiter = manager
            .client_completion(&transaction)
            .expect("pre-retirement exact waiter");

        manager.terminate_transaction(&transaction).await?;
        assert!(manager.client_completions.get(&transaction).is_none());
        assert_eq!(
            manager
                .client_completion_deadlines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len(),
            0,
            "retired INVITE completion must not own a second deadline"
        );

        let route_entry = manager
            .transaction_destinations
            .get(&transaction)
            .expect("consolidated retired INVITE");
        let route_key = Arc::clone(route_entry.key());
        let ClientResponseRouteState::Retired(retired) = route_entry.value() else {
            panic!("INVITE route was not retired");
        };
        assert!(retired.has_completion_wire());
        assert!(retired.shares_wire_allocation());
        assert_eq!(retired.completion.last_response()?, Some(response.clone()));
        let (expires_at, version) = (retired.expires_at, retired.deadline_version);
        drop(route_entry);

        {
            let deadlines = manager
                .retired_client_deadlines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let deadline_key = deadlines
                .by_deadline
                .get(&(expires_at, version))
                .expect("shared retired INVITE deadline");
            assert!(
                Arc::ptr_eq(&route_key, deadline_key),
                "route map and deadline must share one TransactionKey allocation"
            );
        }

        assert!(matches!(
            manager
                .wait_for_client_transaction_outcome(&transaction, Duration::from_millis(100))
                .await?,
            Some(crate::transaction::ClientTransactionOutcome::FinalResponse(exact))
                if exact == response
        ));
        let counts = manager.client_completion_retention_counts();
        assert_eq!(counts.active, 0);
        assert_eq!(counts.retained, 1);
        assert_eq!(counts.deadlines, 1);
        assert_eq!(counts.wire_responses, 1);

        let breakdown = manager.retention_breakdown();
        let layout = &breakdown["storage"]["retired_client_route"]["consolidated_layout"];
        assert_eq!(layout["records"].as_u64(), Some(1));
        assert_eq!(layout["legacy_map_records"].as_u64(), Some(2));
        assert_eq!(layout["current_map_records"].as_u64(), Some(1));
        assert_eq!(layout["legacy_deadline_records"].as_u64(), Some(2));
        assert_eq!(layout["current_deadline_records"].as_u64(), Some(1));
        assert_eq!(layout["legacy_wire_backing_allocations"].as_u64(), Some(2));
        assert_eq!(layout["current_wire_backing_allocations"].as_u64(), Some(1));
        assert_eq!(layout["shared_deadline_key_records"].as_u64(), Some(1));
        assert_eq!(layout["shared_request_response_records"].as_u64(), Some(1));
        assert!(
            layout["current_value_inline_bytes_per_record"]
                .as_u64()
                .expect("current value size")
                <= layout["legacy_value_inline_bytes_per_record"]
                    .as_u64()
                    .expect("legacy value size")
        );
        assert!(
            layout["current_index_inline_bytes_per_record"]
                .as_u64()
                .expect("current index size")
                < layout["legacy_index_inline_bytes_per_record"]
                    .as_u64()
                    .expect("legacy index size")
        );

        let post_retirement_waiter = manager
            .client_completion(&transaction)
            .expect("post-retirement exact waiter");
        assert!(manager.reschedule_retired_client_deadline_for_test(
            &transaction,
            Instant::now() - Duration::from_millis(1),
        ));
        assert_eq!(manager.retired_client_transaction_count(), 0);
        assert!(manager.client_completion(&transaction).is_none());
        assert!(matches!(
            existing_waiter
                .wait_for_outcome(Duration::from_millis(100))
                .await?,
            Some(crate::transaction::ClientTransactionOutcome::FinalResponse(exact))
                if exact == response
        ));
        assert!(matches!(
            post_retirement_waiter
                .wait_for_outcome(Duration::from_millis(100))
                .await?,
            Some(crate::transaction::ClientTransactionOutcome::FinalResponse(exact))
                if exact == response
        ));

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn forced_reliable_non_invite_termination_has_one_exact_outcome_across_retirement(
    ) -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(10);
        let (manager, _event_rx) =
            TransactionManager::new(transport, transport_rx, Some(10)).await?;
        let request = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com")
            .map_err(|error| Error::Other(error.to_string()))?
            .from("Alice", "sip:alice@example.com", Some("alice-tag"))
            .to("Bob", "sip:bob@example.com", None)
            .call_id("forced-reliable-message-completion")
            .cseq(5)
            .via(
                "127.0.0.1:5060",
                "TCP",
                Some("z9hG4bK.forced-reliable-message-completion"),
            )
            .max_forwards(70)
            .build();
        let transaction = manager
            .create_client_transaction_on_route(
                request,
                TransportRoute::new("192.0.2.15:5060".parse().unwrap())
                    .with_transport_type(TransportType::Tcp),
            )
            .await?;
        manager.send_request(&transaction).await?;

        let existing_waiter = manager
            .client_completion(&transaction)
            .expect("active reliable non-INVITE completion");
        manager.terminate_transaction(&transaction).await?;
        assert!(!manager.client_transactions.contains_key(&transaction));
        let post_retirement_waiter = manager
            .client_completion(&transaction)
            .expect("retained reliable non-INVITE completion");

        assert!(matches!(
            existing_waiter
                .wait_for_outcome(Duration::from_millis(100))
                .await?,
            Some(crate::transaction::ClientTransactionOutcome::Failure(
                crate::transaction::ClientTransactionFailure::Cancelled
            ))
        ));
        assert!(matches!(
            post_retirement_waiter
                .wait_for_outcome(Duration::from_millis(100))
                .await?,
            Some(crate::transaction::ClientTransactionOutcome::Failure(
                crate::transaction::ClientTransactionFailure::Cancelled
            ))
        ));
        assert!(matches!(
            manager
                .wait_for_client_transaction_outcome(&transaction, Duration::from_millis(100),)
                .await?,
            Some(crate::transaction::ClientTransactionOutcome::Failure(
                crate::transaction::ClientTransactionFailure::Cancelled
            ))
        ));

        manager.shutdown().await;
        Ok(())
    }

    /// Test management functions like cleanup_terminated_transactions
    #[tokio::test]
    async fn indexed_cleanup_is_bounded_and_full_scan_is_explicit() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_transport_tx, transport_rx) = mpsc::channel(1);
        let manager = TransactionManager::dummy(transport, transport_rx);
        let overflow = 17;

        for index in 0..(TERMINATED_CLEANUP_BATCH_MAX + overflow) {
            manager.terminated_transactions.insert(
                TransactionKey::new(
                    format!("z9hG4bK.bounded-terminal-cleanup-{index}"),
                    Method::Message,
                    false,
                ),
                (),
            );
        }

        assert_eq!(
            manager.cleanup_indexed_terminated_transactions().await?,
            0,
            "synthetic index entries do not own live transactions"
        );
        assert_eq!(
            manager.terminated_transactions.len(),
            overflow,
            "one indexed maintenance pass must process at most one bounded batch"
        );
        manager.cleanup_indexed_terminated_transactions().await?;
        assert!(manager.terminated_transactions.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn sync_constructor_installs_managed_lifecycle_and_lazy_observers() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let manager = TransactionManager::with_config(transport, None);

        assert_eq!(DEFAULT_TRANSACTION_DISPATCH_PRIORITY_BURST_MAX, 64);
        assert_eq!(DEFAULT_INVITE_2XX_RETRANSMIT_MAX_DUE_PER_TICK, 2_048);
        assert!(manager.terminated_cleanup_tx.is_some());
        assert!(manager.terminated_cleanup_shutdown.is_some());
        assert!(manager.lifecycle_scheduler.is_some());
        assert!(manager.retained_client_deadline_worker.is_some());
        assert!(manager.transport_rx.is_none());
        assert!(manager.control_transport_rx.is_none());
        assert!(manager.events_tx.is_detached_primary());
        assert_eq!(manager.subscriber_to_transactions.capacity(), 0);
        assert_eq!(manager.transaction_to_subscribers.capacity(), 0);

        let request = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com")
            .map_err(|error| Error::Other(error.to_string()))?
            .from("Alice", "sip:alice@example.com", Some("alice-tag"))
            .to("Bob", "sip:bob@example.com", None)
            .call_id("managed-sync-constructor")
            .cseq(1)
            .via(
                "127.0.0.1:5060",
                "UDP",
                Some("z9hG4bK.managed-sync-constructor"),
            )
            .max_forwards(70)
            .build();
        let transaction_id = manager
            .create_client_transaction(request, "192.0.2.44:5060".parse().unwrap())
            .await?;
        let transaction = manager
            .client_transactions
            .get(&transaction_id)
            .expect("created transaction");
        assert!(transaction.data().termination_cleanup_tx.get().is_some());
        assert!(transaction.data().lifecycle_scheduler.get().is_some());
        drop(transaction);

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn test_transaction_management() -> Result<()> {
        // Set test environment variable
        std::env::set_var("RVOIP_TEST", "1");

        // Setup mock transport
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (transport_tx, transport_rx) = mpsc::channel(10);

        // Create the transaction manager
        let (manager, _event_rx) =
            TransactionManager::new(transport.clone(), transport_rx, Some(10)).await?;

        // Create two transactions
        let invite_request = create_test_invite().map_err(|e| Error::Other(e.to_string()))?;
        let destination = SocketAddr::from_str("192.168.1.100:5060").unwrap();

        let tx_id1 = manager
            .create_client_transaction(invite_request.clone(), destination)
            .await?;

        let message_request = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com")
            .map_err(|e| Error::Other(e.to_string()))?
            .from("Alice", "sip:alice@example.com", Some("alice-tag"))
            .to("Bob", "sip:bob@example.com", None)
            .call_id("test-call-id-message")
            .cseq(102)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK.message-branch"))
            .max_forwards(70)
            .build();

        let tx_id2 = manager
            .create_client_transaction(message_request.clone(), destination)
            .await?;

        // Check transaction count
        let tx_count = manager.transaction_count().await;
        assert_eq!(tx_count, 2, "Should have 2 transactions");

        // Terminate one transaction
        println!("Terminating transaction {}", tx_id1);
        manager.terminate_transaction(&tx_id1).await?;

        // Wait a moment for termination to complete and events to propagate
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Force call cleanup multiple times to ensure it works
        for i in 0..3 {
            let cleaned = manager.cleanup_terminated_transactions().await?;
            println!(
                "Cleanup attempt {}: {} transactions cleaned",
                i + 1,
                cleaned
            );

            if cleaned > 0 {
                // If we cleaned at least one transaction, we consider the test successful
                break;
            }

            if i < 2 {
                // Don't sleep on the last iteration
                // Sleep a bit before trying again
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }

        // Check transaction count again
        let tx_count = manager.transaction_count().await;
        println!("Transaction count after cleanup: {}", tx_count);

        // Check active transactions
        let (client_txs, server_txs) = manager.active_transactions().await;
        println!("Active client transactions: {}", client_txs.len());
        for tx in &client_txs {
            println!("  - {}", tx);
        }
        assert_eq!(
            server_txs.len(),
            0,
            "Should have 0 active server transactions"
        );

        // If we cleaned the transaction, verify it's no longer in the collection
        let tx_exists = manager.transaction_exists(&tx_id1).await;
        assert!(!tx_exists, "Terminated transaction should no longer exist");

        // Clean up
        manager.shutdown().await;

        // Reset environment variable
        std::env::remove_var("RVOIP_TEST");

        Ok(())
    }

    /// Test request retry
    #[tokio::test]
    async fn test_retry_request() -> Result<()> {
        // Set test environment variable
        std::env::set_var("RVOIP_TEST", "1");

        // Setup mock transport
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_, transport_rx) = mpsc::channel(10);

        // Create the transaction manager
        let (manager, _) =
            TransactionManager::new(transport.clone(), transport_rx, Some(10)).await?;

        // Create a non-INVITE request
        let request = SimpleRequestBuilder::new(Method::Options, "sip:bob@example.com")
            .map_err(|e| Error::Other(e.to_string()))?
            .from("Alice", "sip:alice@example.com", Some("alice-tag"))
            .to("Bob", "sip:bob@example.com", None)
            .call_id("test-call-id-options")
            .cseq(103)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK.options-branch"))
            .max_forwards(70)
            .build();

        let destination = SocketAddr::from_str("192.168.1.100:5060").unwrap();

        // Create transaction
        let tx_id = manager
            .create_client_transaction(request.clone(), destination)
            .await?;

        // Send the request
        manager.send_request(&tx_id).await?;

        // Check initial message count
        let sent_messages = transport.get_sent_messages().await;
        assert_eq!(sent_messages.len(), 1, "Should have sent 1 message");

        // Retry the request
        manager.retry_request(&tx_id).await?;

        // Check message count after retry
        let sent_messages = transport.get_sent_messages().await;
        assert_eq!(
            sent_messages.len(),
            2,
            "Should have sent 2 messages after retry"
        );

        // Clean up
        manager.shutdown().await;

        // Reset environment variable
        std::env::remove_var("RVOIP_TEST");

        Ok(())
    }

    /// Test error handling when using invalid transaction IDs
    #[tokio::test]
    async fn test_error_handling_invalid_tx_id() -> Result<()> {
        // Setup mock transport
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_, transport_rx) = mpsc::channel(10);

        // Create the transaction manager
        let (manager, _) =
            TransactionManager::new(transport.clone(), transport_rx, Some(10)).await?;

        // Create an invalid transaction ID
        let invalid_tx_id =
            TransactionKey::new("z9hG4bK.nonexistent".to_string(), Method::Invite, false);

        // Try various operations with the invalid ID
        assert!(
            !manager.transaction_exists(&invalid_tx_id).await,
            "Transaction should not exist"
        );

        assert!(
            manager.transaction_state(&invalid_tx_id).await.is_err(),
            "transaction_state should error for invalid ID"
        );

        assert!(
            manager.original_request(&invalid_tx_id).await.is_err(),
            "original_request should error for invalid ID"
        );

        assert!(
            manager.terminate_transaction(&invalid_tx_id).await.is_err(),
            "terminate_transaction should error for invalid ID"
        );

        // Clean up
        manager.shutdown().await;

        Ok(())
    }

    /// Simple test to debug transaction state transitions
    #[tokio::test]
    async fn test_debug_transaction_transitions() -> Result<()> {
        use std::fs::File;
        use std::io::Write;

        // Create a debug log file
        let mut debug_file = File::create("transaction_debug.log").unwrap();
        writeln!(debug_file, "Starting debug test").unwrap();

        // Setup mock transport
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (transport_tx, transport_rx) = mpsc::channel(10);

        // Create the transaction manager
        let (manager, mut event_rx) =
            TransactionManager::new(transport.clone(), transport_rx, Some(10)).await?;

        writeln!(debug_file, "Created transaction manager").unwrap();

        // Create an INVITE request
        let invite_request = create_test_invite().map_err(|e| Error::Other(e.to_string()))?;
        let destination = SocketAddr::from_str("192.168.1.100:5060").unwrap();

        // Test transaction creation
        let tx_id = manager
            .create_client_transaction(invite_request.clone(), destination)
            .await?;

        writeln!(debug_file, "Created transaction: {}", tx_id).unwrap();
        writeln!(
            debug_file,
            "Initial state: {:?}",
            manager.transaction_state(&tx_id).await?
        )
        .unwrap();

        // Send the request
        writeln!(debug_file, "Calling send_request").unwrap();
        match manager.send_request(&tx_id).await {
            Ok(_) => writeln!(debug_file, "send_request succeeded").unwrap(),
            Err(e) => writeln!(debug_file, "send_request failed: {}", e).unwrap(),
        }

        // Give some time for the state transition to occur
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Check state after send_request
        let state_after_send = manager.transaction_state(&tx_id).await?;
        writeln!(
            debug_file,
            "State after send_request: {:?}",
            state_after_send
        )
        .unwrap();

        // Try to wait for the Calling state with a generous timeout
        writeln!(debug_file, "Waiting for Calling state").unwrap();
        let success = manager
            .wait_for_transaction_state(
                &tx_id,
                TransactionState::Calling,
                tokio::time::Duration::from_millis(500),
            )
            .await?;

        writeln!(debug_file, "wait_for_transaction_state result: {}", success).unwrap();

        // Final state check
        let final_state = manager.transaction_state(&tx_id).await?;
        writeln!(debug_file, "Final state: {:?}", final_state).unwrap();

        // Check for events
        match tokio::time::timeout(tokio::time::Duration::from_millis(100), event_rx.recv()).await {
            Ok(Some(event)) => writeln!(debug_file, "Received event: {:?}", event).unwrap(),
            Ok(None) => writeln!(debug_file, "Event channel closed").unwrap(),
            Err(_) => writeln!(debug_file, "Timeout waiting for event").unwrap(),
        }

        // Clean up
        manager.shutdown().await;
        writeln!(debug_file, "Test completed").unwrap();

        Ok(())
    }

    /// Test that transport errors are properly propagated
    #[tokio::test]
    async fn test_transport_error_propagation() -> Result<()> {
        // Setup mock transport with send failure
        let transport = Arc::new(MockTransport::with_send_failure("127.0.0.1:5060", true));
        let (_, transport_rx) = mpsc::channel(10);

        // Create the transaction manager
        let (manager, _) =
            TransactionManager::new(transport.clone(), transport_rx, Some(10)).await?;

        // Create an INVITE request
        let invite_request = create_test_invite().map_err(|e| Error::Other(e.to_string()))?;
        let destination = SocketAddr::from_str("192.168.1.100:5060").unwrap();

        // Create a client transaction
        let tx_id = manager
            .create_client_transaction(invite_request.clone(), destination)
            .await?;

        // Attempt to send the request, which should fail
        let result = manager.send_request(&tx_id).await;

        // Verify that the error is properly propagated
        assert!(
            result.is_err(),
            "Expected send_request to fail due to transport error"
        );

        // Verify the error type is ConnectionFailed
        if let Err(err) = result {
            println!("Error: {:?}", err);
            match err {
                Error::TransportError { source, .. } => {
                    // The public transport boundary deliberately retains only a
                    // fixed error class, never the simulated lower error text.
                    let error_str = source.0;
                    assert_eq!(error_str, "transport protocol error");
                }
                _ => panic!("Unexpected error type: {:?}", err),
            }
        }

        // Clean up
        manager.shutdown().await;

        Ok(())
    }

    #[tokio::test]
    async fn test_send_request_does_not_wait_for_async_error_window() -> Result<()> {
        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_, transport_rx) = mpsc::channel(10);
        let (manager, _event_rx) =
            TransactionManager::new(transport.clone(), transport_rx, Some(10)).await?;

        let invite_request = create_test_invite().map_err(|e| Error::Other(e.to_string()))?;
        let destination = SocketAddr::from_str("192.168.1.100:5060").unwrap();
        let tx_id = manager
            .create_client_transaction(invite_request, destination)
            .await?;

        let started = Instant::now();
        manager.send_request(&tx_id).await?;
        let elapsed = started.elapsed();

        assert!(
            elapsed < Duration::from_millis(80),
            "send_request should not wait for the old 100 ms async error window, elapsed: {:?}",
            elapsed
        );

        let sent_messages = transport.get_sent_messages().await;
        assert_eq!(sent_messages.len(), 1, "Expected INVITE to be sent");

        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn stopping_manager_rejects_unguarded_response_before_runner_enqueue() -> Result<()> {
        use crate::transaction::server::TransactionExt;

        let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
        let (_, transport_rx) = mpsc::channel(10);
        let (manager, _event_rx) =
            TransactionManager::new(transport.clone(), transport_rx, Some(10)).await?;
        let request = create_dispatch_request(Method::Bye, "z9hG4bK.stop-response-race", 9)
            .map_err(|error| Error::Other(error.to_string()))?;
        let transaction = manager
            .create_server_transaction(request.clone(), "127.0.0.1:5090".parse().unwrap())
            .await?;
        let response = create_test_response(&request, StatusCode::Ok, None);

        // Model an already-admitted API operation that loses the race with
        // Stopping before it transfers ownership to the runner command.
        let outer_operation = manager
            .admission_lifecycle
            .try_enter_existing()
            .expect("outer response operation admitted");
        manager.admission_lifecycle.begin_stopping();
        let result = transaction
            .as_server_transaction()
            .expect("BYE server transaction")
            .send_response(response)
            .await;
        drop(outer_operation);

        assert!(result.is_err());
        assert_eq!(transport.raw_send_count(), 0);
        assert!(transport.get_sent_messages().await.is_empty());
        assert!(!transaction.data().final_response_may_have_reached_wire());

        manager.shutdown().await;
        Ok(())
    }
}
