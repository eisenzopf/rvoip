//! Exact transaction response operations for [`DialogServer`].
//!
//! These retained compatibility methods do not build SIP messages themselves.
//! They validate the caller's transaction/dialog authority and delegate to the
//! dialog manager's canonical response lifecycle and transaction wire fence.

use tracing::debug;

use super::super::{ApiError, ApiResult};
use super::core::DialogServer;
use crate::dialog::DialogId;
use crate::transaction::TransactionKey;
use rvoip_sip_core::{Method, StatusCode};

/// Response sending implementations for [`DialogServer`].
impl DialogServer {
    async fn send_exact_response(
        &self,
        transaction_id: &TransactionKey,
        expected_dialog_id: Option<&DialogId>,
        status_code: StatusCode,
        reason: Option<&str>,
        body: Option<&str>,
        contact_uri: Option<&str>,
    ) -> ApiResult<()> {
        if !transaction_id.is_server() {
            return Err(ApiError::Protocol {
                message: "a server response requires a server transaction".to_string(),
            });
        }

        let owned_dialog_id = self
            .dialog_manager
            .find_dialog_for_transaction(transaction_id)
            .map_err(ApiError::from)?;
        if expected_dialog_id.is_some_and(|expected| expected != &owned_dialog_id) {
            return Err(ApiError::Protocol {
                message: "response transaction is owned by a different dialog".to_string(),
            });
        }

        self.dialog_manager
            .send_known_transaction_response(
                &owned_dialog_id,
                transaction_id,
                status_code.as_u16(),
                reason,
                body,
                &[],
                contact_uri,
            )
            .await
            .map_err(ApiError::from)
    }

    fn require_invite_transaction(transaction_id: &TransactionKey) -> ApiResult<()> {
        if transaction_id.method() != &Method::Invite {
            return Err(ApiError::Protocol {
                message: "this response operation requires an INVITE transaction".to_string(),
            });
        }
        Ok(())
    }

    /// Send a SIP response on an exact dialog-owned server transaction.
    ///
    /// The operation fails when the transaction is absent, is a client
    /// transaction, or is not currently mapped to a dialog. It never reports
    /// success without crossing the transaction wire boundary.
    pub async fn send_simple_response(
        &self,
        transaction_id: &TransactionKey,
        status_code: StatusCode,
        reason: Option<String>,
    ) -> ApiResult<()> {
        debug!(
            status = status_code.as_u16(),
            "Sending exact server transaction response"
        );
        self.send_exact_response(
            transaction_id,
            None,
            status_code,
            reason.as_deref(),
            None,
            None,
        )
        .await
    }

    /// Send a status response with an optional reason phrase.
    pub async fn send_status_response(
        &self,
        transaction_id: &TransactionKey,
        status_code: StatusCode,
        reason: Option<String>,
    ) -> ApiResult<()> {
        self.send_simple_response(transaction_id, status_code, reason)
            .await
    }

    /// Send a `100 Trying` response on an exact server transaction.
    pub async fn send_trying_response(&self, transaction_id: &TransactionKey) -> ApiResult<()> {
        self.send_simple_response(transaction_id, StatusCode::Trying, None)
            .await
    }

    /// Send a `180 Ringing` response to an INVITE.
    ///
    /// A supplied dialog must own the transaction. Early-media SDP and Contact
    /// are preserved by the canonical dialog response materializer.
    pub async fn send_ringing_response(
        &self,
        transaction_id: &TransactionKey,
        dialog_id: Option<&DialogId>,
        early_media_sdp: Option<String>,
        contact_uri: Option<String>,
    ) -> ApiResult<()> {
        Self::require_invite_transaction(transaction_id)?;
        self.send_exact_response(
            transaction_id,
            dialog_id,
            StatusCode::Ringing,
            None,
            early_media_sdp.as_deref(),
            contact_uri.as_deref(),
        )
        .await
    }

    /// Send a `200 OK` response to an INVITE.
    ///
    /// A supplied dialog must own the transaction. The SDP answer and Contact
    /// are sent through the same canonical response path as every other
    /// dialog-owned transaction response.
    pub async fn send_ok_invite_response(
        &self,
        transaction_id: &TransactionKey,
        dialog_id: Option<&DialogId>,
        sdp_answer: String,
        contact_uri: String,
    ) -> ApiResult<()> {
        Self::require_invite_transaction(transaction_id)?;
        self.send_exact_response(
            transaction_id,
            dialog_id,
            StatusCode::Ok,
            None,
            Some(&sdp_answer),
            Some(&contact_uri),
        )
        .await
    }

    /// Send a final failure or redirection response to an INVITE.
    pub async fn send_invite_error_response(
        &self,
        transaction_id: &TransactionKey,
        status_code: StatusCode,
        reason: Option<String>,
    ) -> ApiResult<()> {
        Self::require_invite_transaction(transaction_id)?;
        if !(300..=699).contains(&status_code.as_u16()) {
            return Err(ApiError::Protocol {
                message: "an INVITE error response must have a 3xx-6xx status".to_string(),
            });
        }
        self.send_exact_response(
            transaction_id,
            None,
            status_code,
            reason.as_deref(),
            None,
            None,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ServerConfig;
    use crate::manager::dialog_operations::DialogLookup;
    use crate::transaction::TransactionManager;
    use async_trait::async_trait;
    use rvoip_sip_core::builder::SimpleRequestBuilder;
    use rvoip_sip_core::{HeaderName, Message, Request};
    use rvoip_sip_transport::error::Result as TransportResult;
    use rvoip_sip_transport::{Transport, TransportEvent};
    use std::net::SocketAddr;
    use std::str::FromStr;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use tokio::sync::{mpsc, Mutex};

    #[derive(Debug)]
    struct RecordingTransport {
        addr: SocketAddr,
        closed: AtomicBool,
        messages: Mutex<Vec<Message>>,
    }

    impl RecordingTransport {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                addr: SocketAddr::from_str("127.0.0.1:5060").unwrap(),
                closed: AtomicBool::new(false),
                messages: Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait]
    impl Transport for RecordingTransport {
        fn local_addr(&self) -> TransportResult<SocketAddr> {
            Ok(self.addr)
        }

        async fn send_message(
            &self,
            message: Message,
            _destination: SocketAddr,
        ) -> TransportResult<()> {
            self.messages.lock().await.push(message);
            Ok(())
        }

        async fn close(&self) -> TransportResult<()> {
            self.closed.store(true, Ordering::SeqCst);
            Ok(())
        }

        fn is_closed(&self) -> bool {
            self.closed.load(Ordering::SeqCst)
        }
    }

    async fn make_server() -> (DialogServer, Arc<RecordingTransport>) {
        let transport = RecordingTransport::new();
        let (_transport_tx, transport_rx) = mpsc::channel::<TransportEvent>(16);
        let (transaction_manager, _events_rx) =
            TransactionManager::new(transport.clone(), transport_rx, Some(16))
                .await
                .expect("build TransactionManager");
        let server = DialogServer::with_dependencies(
            Arc::new(transaction_manager),
            ServerConfig::new(transport.addr),
        )
        .await
        .expect("build DialogServer");
        (server, transport)
    }

    fn initial_invite() -> Request {
        SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
            .unwrap()
            .from("Alice", "sip:alice@example.com", Some("alice-tag"))
            .to("Bob", "sip:bob@example.com", None)
            .contact("sip:alice@127.0.0.1:5061", None)
            .call_id("server-response-facade-test")
            .cseq(1)
            .via(
                "127.0.0.1:5061",
                "UDP",
                Some("z9hG4bK-server-response-facade"),
            )
            .max_forwards(70)
            .build()
    }

    fn source_between<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
        let start = source.find(start).expect("start marker");
        let remainder = &source[start..];
        let end = remainder.find(end).expect("end marker");
        &remainder[..end]
    }

    #[test]
    fn response_facade_has_one_exact_wire_delegate_and_no_local_materializer() {
        let source = include_str!("response_builder.rs");
        let exact = source_between(
            source,
            "async fn send_exact_response(",
            "fn require_invite_transaction(",
        );
        assert!(exact.contains(".find_dialog_for_transaction(transaction_id)"));
        assert!(exact.contains(".send_known_transaction_response("));
        assert!(!exact.contains("Ok(())"));

        let production = source_between(source, "impl DialogServer {", "#[cfg(test)]");
        assert!(!production.contains("SimpleResponseBuilder"));
        assert!(!production.contains("response_builders::"));
        assert!(!production.contains("Response::new"));
        assert!(!production.contains("For now"));
        assert_eq!(
            production
                .matches(".send_known_transaction_response(")
                .count(),
            1,
            "all server response methods must share one canonical materializer"
        );

        let ringing = source_between(
            source,
            "pub async fn send_ringing_response(",
            "/// Send a `200 OK` response",
        );
        assert!(ringing.contains("early_media_sdp.as_deref()"));
        assert!(ringing.contains("contact_uri.as_deref()"));

        let ok = source_between(
            source,
            "pub async fn send_ok_invite_response(",
            "/// Send a final failure",
        );
        assert!(ok.contains("Some(&sdp_answer)"));
        assert!(ok.contains("Some(&contact_uri)"));

        let invite_error = source_between(
            source,
            "pub async fn send_invite_error_response(",
            "}\n}\n\n#[cfg(test)]",
        );
        assert!(invite_error.contains("(300..=699).contains"));
        assert!(invite_error.contains("self.send_exact_response("));
    }

    #[tokio::test]
    async fn unowned_or_invalid_response_operations_fail_closed() {
        let (server, transport) = make_server().await;
        let missing_invite =
            TransactionKey::new("z9hG4bK-missing".to_string(), Method::Invite, true);

        assert!(server
            .send_simple_response(&missing_invite, StatusCode::Trying, None)
            .await
            .is_err());
        assert!(server
            .send_ringing_response(&missing_invite, None, None, None)
            .await
            .is_err());
        assert!(server
            .send_ok_invite_response(
                &missing_invite,
                None,
                "v=0\r\n".to_string(),
                "sip:bob@127.0.0.1:5060".to_string(),
            )
            .await
            .is_err());

        let options = TransactionKey::new("z9hG4bK-options".to_string(), Method::Options, true);
        assert!(server
            .send_ringing_response(&options, None, None, None)
            .await
            .is_err());
        assert!(server
            .send_invite_error_response(&missing_invite, StatusCode::Ok, None)
            .await
            .is_err());

        let client_invite =
            TransactionKey::new("z9hG4bK-client".to_string(), Method::Invite, false);
        assert!(server
            .send_simple_response(&client_invite, StatusCode::Trying, None)
            .await
            .is_err());
        assert!(
            transport.messages.lock().await.is_empty(),
            "failed validation and missing ownership must remain zero-wire"
        );
    }

    #[tokio::test]
    async fn ringing_facade_preserves_sdp_and_contact_on_the_wire() {
        let (server, transport) = make_server().await;
        let invite = initial_invite();
        let transaction = server
            .dialog_manager
            .transaction_manager()
            .create_server_transaction(
                invite.clone(),
                SocketAddr::from_str("127.0.0.1:5061").unwrap(),
            )
            .await
            .expect("create server transaction");
        let transaction_id = transaction.id().clone();
        let dialog_id = server
            .dialog_manager
            .create_early_dialog_from_invite(&invite)
            .await
            .expect("create early dialog");
        server
            .dialog_manager
            .associate_transaction_with_dialog(&transaction_id, &dialog_id);

        server
            .send_ringing_response(
                &transaction_id,
                Some(&dialog_id),
                Some("v=0\r\n".to_string()),
                Some("sip:bob@127.0.0.1:5060".to_string()),
            )
            .await
            .expect("send exact ringing response");

        let messages = transport.messages.lock().await;
        assert_eq!(messages.len(), 1, "one facade call must produce one write");
        let Message::Response(response) = &messages[0] else {
            panic!("server response facade wrote a request");
        };
        assert_eq!(response.status_code(), 180);
        assert_eq!(response.body(), b"v=0\r\n");
        assert_eq!(
            response
                .headers
                .iter()
                .filter(|header| header.name() == HeaderName::Contact)
                .count(),
            1,
            "Contact override must be materialized exactly once"
        );
        assert!(response
            .headers
            .iter()
            .any(|header| header.name() == HeaderName::ContentType));
    }
}
