//! Response Lifecycle Management
//!
//! This module provides a unified approach to dialog state management for both
//! UAC (receiving responses) and UAS (sending responses).
//!
//! ## Design Philosophy
//!
//! Dialog state transitions should happen at consistent points in the message lifecycle:
//! - **UAC**: After receiving a response (learns remote tag)
//! - **UAS**: After transaction-core accepts the response send
//!
//! The UAS allocates a stable local tag while materializing the response, but
//! does not publish a Confirmed/Terminated dialog state until the exact server
//! transaction has accepted the wire operation. This prevents a local send
//! failure from leaving an application-visible state transition behind.
//!
//! ## Architecture
//!
//! ```text
//! UAC Flow (Alice):
//!   INVITE sent → Early dialog
//!   ↓
//!   200 OK received → handle_response_received() → Confirmed + lookup registered
//!
//! UAS Flow (Bob):
//!   INVITE received → Early dialog
//!   ↓
//!   2xx built + preflighted → 2xx sent
//!   ↓
//!   Confirmed + lookup registered
//!
//! UAS final rejection flow (Bob):
//!   INVITE received → Early dialog
//!   ↓
//!   3xx-6xx built + preflighted → final response sent
//!   ↓
//!   Terminated + early lookup removed
//! ```

use rvoip_sip_core::{HeaderName, Method, Request, Response, StatusCode, TypedHeader, Uri};
use tracing::{debug, info};

use crate::diagnostics::safe_log::method_class;
use crate::dialog::{DialogId, DialogState};
use crate::errors::{DialogError, DialogResult};
use crate::manager::core::DialogManager;
use crate::manager::utils::DialogUtils;
use crate::transaction::server::FinalResponseCompletionDisposition;
use crate::transaction::TransactionKey;

pub(crate) struct ClassifiedDialogResponseError {
    pub(crate) source: DialogError,
    pub(crate) disposition: FinalResponseCompletionDisposition,
}

/// Response lifecycle hooks for dialog state management
///
/// This trait defines lifecycle hooks that are called at critical points when
/// sending or receiving responses, allowing for consistent dialog state management.
pub trait ResponseLifecycle {
    /// Called BEFORE sending a response (UAS perspective)
    ///
    /// This hook validates lifecycle preconditions without committing a dialog
    /// state transition. The canonical response materializer commits only
    /// after transaction-core accepts the response send.
    ///
    /// # Arguments
    /// * `dialog_id` - The dialog this response belongs to
    /// * `response` - The response about to be sent
    /// * `transaction_id` - The transaction this response is for
    /// * `original_request` - The original request being responded to
    ///
    /// # Returns
    /// Ok(()) if the pre-send processing succeeded
    fn pre_send_response(
        &self,
        dialog_id: &DialogId,
        response: &Response,
        transaction_id: &TransactionKey,
        original_request: &Request,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;

    /// Called AFTER sending a response (UAS perspective)
    ///
    /// This hook can be used for observational post-send actions like logging
    /// and metrics. The canonical lifecycle commit has already completed when
    /// it is called. It is currently a no-op but retained for compatibility.
    ///
    /// # Arguments
    /// * `dialog_id` - The dialog this response belongs to
    /// * `response` - The response that was sent
    fn post_send_response(
        &self,
        dialog_id: &DialogId,
        response: &Response,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
}

/// Implementation of response lifecycle for DialogManager
impl ResponseLifecycle for DialogManager {
    /// Pre-send hook for UAS responses
    ///
    /// Validates lifecycle state needed by the response that is about to be
    /// sent. Application-visible state remains unchanged until the exact send
    /// succeeds.
    async fn pre_send_response(
        &self,
        dialog_id: &DialogId,
        response: &Response,
        _transaction_id: &TransactionKey,
        original_request: &Request,
    ) -> DialogResult<()> {
        debug!(
            "pre_send_response: dialog={}, status={}, method={}",
            dialog_id,
            response.status_code(),
            method_class(&original_request.method())
        );

        self.validate_sent_response_lifecycle(dialog_id, response, original_request)
    }

    /// Post-send hook for UAS responses
    async fn post_send_response(
        &self,
        _dialog_id: &DialogId,
        _response: &Response,
    ) -> DialogResult<()> {
        // Currently a no-op, but provided for future extensibility
        Ok(())
    }
}

/// Helper methods for dialog confirmation
impl DialogManager {
    /// Author one exact final response and classify the transaction runner's
    /// first-write completion. This primitive requires no session or dialog
    /// lookup, so protocol handlers can safely use it before a session exists
    /// or after causal delivery fails.
    pub(crate) async fn send_exact_final_response_classified(
        &self,
        transaction_id: &TransactionKey,
        response: Response,
    ) -> Result<FinalResponseCompletionDisposition, ClassifiedDialogResponseError> {
        if !transaction_id.is_server() || !(200..=699).contains(&response.status_code()) {
            return Err(ClassifiedDialogResponseError {
                source: DialogError::protocol_error(
                    "classified exact response requires a final server transaction",
                ),
                disposition: FinalResponseCompletionDisposition::ZeroWireRetryable,
            });
        }

        match self.send_response(transaction_id, response).await {
            Ok(()) => Ok(FinalResponseCompletionDisposition::WrittenSuccessTerminal),
            Err(source) => {
                let disposition = self
                    .transaction_manager()
                    .classify_final_response_completion(transaction_id)
                    .await;
                if disposition == FinalResponseCompletionDisposition::WrittenSuccessTerminal {
                    Ok(disposition)
                } else {
                    Err(ClassifiedDialogResponseError {
                        source,
                        disposition,
                    })
                }
            }
        }
    }

    /// Protocol-handler policy for an unowned final response: written and
    /// wire-unknown both consume the exact response authority; only proven
    /// zero-wire remains an error that a causal caller may retry.
    pub(crate) async fn send_unowned_final_response_classified(
        &self,
        transaction_id: &TransactionKey,
        response: Response,
    ) -> DialogResult<FinalResponseCompletionDisposition> {
        match self
            .send_exact_final_response_classified(transaction_id, response)
            .await
        {
            Ok(disposition) => Ok(disposition),
            Err(error)
                if error.disposition
                    == FinalResponseCompletionDisposition::WireUnknownErrorTerminal =>
            {
                Ok(error.disposition)
            }
            Err(error) => Err(error.source),
        }
    }

    /// Stamp a stable local To tag on an initial-INVITE fallback response.
    /// The early dialog retains the tag across a proven zero-wire retry.
    pub(crate) fn stamp_initial_invite_response_tag(
        &self,
        dialog_id: &DialogId,
        response: &mut Response,
    ) -> DialogResult<()> {
        let local_tag = {
            let mut dialog = self.get_dialog_mut(dialog_id)?;
            match dialog.local_tag.clone() {
                Some(tag) if !tag.is_empty() => tag,
                _ => {
                    let tag = dialog.generate_local_tag();
                    dialog.local_tag = Some(tag.clone());
                    tag
                }
            }
        };
        set_response_to_tag(response, &local_tag)
    }

    /// Send the one final response owned by an initial server INVITE.
    ///
    /// This is the canonical response boundary for the retained split call
    /// handles. It resolves an exact, still-open server transaction; builds one
    /// dialog-aware response; applies the existing response lifecycle; and
    /// delegates the actual first-writer-wins wire operation to transaction-core.
    pub(crate) async fn send_initial_invite_final_response(
        &self,
        dialog_id: &DialogId,
        status_code: StatusCode,
        reason: Option<&str>,
        sdp_answer: Option<&str>,
    ) -> DialogResult<()> {
        if !(200..=699).contains(&status_code.as_u16()) {
            return Err(DialogError::protocol_error(
                "initial INVITE response must be final",
            ));
        }

        if self.get_dialog_state(dialog_id)? != DialogState::Early {
            return Err(DialogError::invalid_state(
                "early dialog awaiting an initial INVITE response",
                "dialog is not awaiting an initial INVITE response",
            ));
        }

        let transaction_id = self
            .open_initial_invite_server_transaction(dialog_id)
            .await?;
        self.send_known_transaction_response(
            dialog_id,
            &transaction_id,
            status_code.as_u16(),
            reason,
            sdp_answer,
            &[],
            None,
        )
        .await
    }

    /// Build and send a response for an exact server transaction owned by a
    /// dialog. Both the unified session API and the retained call facades use
    /// this implementation, so tags, Contact, body typing, lifecycle updates,
    /// and the transaction wire fence cannot drift between public surfaces.
    pub(crate) async fn send_known_transaction_response(
        &self,
        dialog_id: &DialogId,
        transaction_id: &TransactionKey,
        status_code: u16,
        reason: Option<&str>,
        body: Option<&str>,
        extra_headers: &[TypedHeader],
        contact_uri: Option<&str>,
    ) -> DialogResult<()> {
        let status = StatusCode::from_u16(status_code)
            .map_err(|_| DialogError::protocol_error("invalid SIP response status"))?;
        let transaction_dialog = self.find_dialog_for_transaction(transaction_id)?;
        if &transaction_dialog != dialog_id {
            return Err(DialogError::routing_error(
                "response transaction is not owned by the requested dialog",
            ));
        }

        let original_request = self
            .transaction_manager()
            .original_request(transaction_id)
            .await
            .map_err(|_| DialogError::TransactionError {
                message: "failed to read response transaction request".to_string(),
            })?
            .ok_or_else(|| DialogError::TransactionError {
                message: "response transaction request is unavailable".to_string(),
            })?;

        let is_initial_invite = original_request.method() == Method::Invite
            && original_request.to().and_then(|to| to.tag()).is_none();

        // RFC 3261 §8.2.6.2 requires a To tag on every initial-INVITE
        // response except 100. Allocate it once on the dialog so a retry after
        // a proven zero-wire failure cannot fork the dialog identity.
        let local_tag = if is_initial_invite && status_code > 100 {
            Some({
                let mut dialog = self.get_dialog_mut(dialog_id)?;
                match dialog.local_tag.clone() {
                    Some(tag) if !tag.is_empty() => tag,
                    _ => {
                        let tag = dialog.generate_local_tag();
                        dialog.local_tag = Some(tag.clone());
                        tag
                    }
                }
            })
        } else {
            None
        };

        let mut builder = rvoip_sip_core::builder::SimpleResponseBuilder::response_from_request(
            &original_request,
            status,
            reason,
        );
        if let Some(body) = body {
            builder = builder.body(body.as_bytes().to_vec());
            if original_request.method() == Method::Invite || body.trim_start().starts_with("v=") {
                builder = builder.content_type("application/sdp");
            }
        }
        let mut response = builder.build();
        if let Some(local_tag) = local_tag.as_deref() {
            set_response_to_tag(&mut response, local_tag)?;
        }

        if let Some(contact_uri) = contact_uri {
            if original_request.method() != Method::Invite {
                return Err(DialogError::protocol_error(
                    "a Contact override is only valid for an INVITE response",
                ));
            }
            add_contact_header(&mut response, contact_uri)?;
        } else if is_initial_invite && (200..300).contains(&status_code) {
            let contact_uri = self.local_contact_uri().unwrap_or_else(|| {
                let local = self.local_address_for_uri(original_request.uri());
                format!("sip:server@{local}")
            });
            add_contact_header(&mut response, &contact_uri)?;
        }

        for header in extra_headers {
            if !is_response_stack_managed(&header.name()) {
                response.headers.push(header.clone());
            }
        }

        // Reject malformed custom reasons, bodies, and headers before the
        // response reaches transaction-core. Transaction-core validates again
        // at its wire boundary; this earlier pass keeps zero-wire input
        // failures retryable on the same early dialog.
        rvoip_sip_core::validation::validate_wire_response(&response).map_err(|_| {
            DialogError::protocol_error("response failed SIP wire-safety validation")
        })?;

        self.pre_send_response(dialog_id, &response, transaction_id, &original_request)
            .await?;
        self.send_response(transaction_id, response.clone()).await?;
        // This commit is deliberately synchronous. Once the exact send
        // future returns success there must be no cancellation/yield point
        // before dialog state records that wire outcome.
        self.commit_sent_response_lifecycle(dialog_id, &response, &original_request)?;
        if status_code >= 200 {
            self.clear_pending_response_transaction(dialog_id, transaction_id);
        }
        self.post_send_response(dialog_id, &response).await?;
        Ok(())
    }

    /// Validate all fallible initial-INVITE lifecycle inputs before the
    /// response reaches transaction-core. This method deliberately performs
    /// no dialog or lookup mutation.
    fn validate_sent_response_lifecycle(
        &self,
        dialog_id: &DialogId,
        response: &Response,
        original_request: &Request,
    ) -> DialogResult<()> {
        if original_request.method() != Method::Invite
            || original_request.to().and_then(|to| to.tag()).is_some()
        {
            return Ok(());
        }

        let dialog = self.get_dialog(dialog_id)?;
        if !(200..300).contains(&response.status_code()) || dialog.state != DialogState::Early {
            return Ok(());
        }

        let response_local_tag = response
            .to()
            .and_then(|to| to.tag())
            .filter(|tag| !tag.is_empty());
        let has_local_tag = dialog
            .local_tag
            .as_deref()
            .is_some_and(|tag| !tag.is_empty())
            || response_local_tag.is_some();
        if !has_local_tag {
            return Err(DialogError::protocol_error(
                "successful initial INVITE response must have a To tag",
            ));
        }
        if !dialog
            .remote_tag
            .as_deref()
            .is_some_and(|tag| !tag.is_empty())
        {
            return Err(DialogError::protocol_error(
                "initial INVITE dialog is missing its remote tag",
            ));
        }

        Ok(())
    }

    /// Commit the UAS dialog lifecycle only after the exact transaction send
    /// succeeds. This is the sole writer for initial-INVITE response state.
    fn commit_sent_response_lifecycle(
        &self,
        dialog_id: &DialogId,
        response: &Response,
        original_request: &Request,
    ) -> DialogResult<()> {
        if original_request.method() != Method::Invite
            || original_request.to().and_then(|to| to.tag()).is_some()
        {
            return Ok(());
        }

        match response.status_code() {
            200..=299 => self.confirm_uas_dialog(dialog_id, response),
            300..=699 => self.terminate_uas_early_dialog_for_final_response(dialog_id),
            _ => Ok(()),
        }
    }

    async fn open_initial_invite_server_transaction(
        &self,
        dialog_id: &DialogId,
    ) -> DialogResult<TransactionKey> {
        let mut candidates = Vec::new();
        if let Some(pending) = self.pending_response_transaction_for_dialog(dialog_id) {
            candidates.push(pending);
        }
        for candidate in self.server_transactions_for_dialog(dialog_id) {
            if !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }

        let mut open = Vec::new();
        for candidate in candidates {
            if candidate.method() != &Method::Invite || !candidate.is_server() {
                continue;
            }
            if self.find_dialog_for_transaction(&candidate).ok().as_ref() != Some(dialog_id) {
                continue;
            }
            if matches!(
                self.transaction_manager()
                    .transaction_state(&candidate)
                    .await,
                Ok(crate::transaction::TransactionState::Initial)
                    | Ok(crate::transaction::TransactionState::Trying)
                    | Ok(crate::transaction::TransactionState::Proceeding)
            ) {
                let is_initial = matches!(
                    self.transaction_manager().original_request(&candidate).await,
                    Ok(Some(request))
                        if request.method() == Method::Invite
                            && request.to().and_then(|to| to.tag()).is_none()
                );
                if is_initial {
                    open.push(candidate);
                }
            }
        }

        match open.len() {
            1 => Ok(open.remove(0)),
            0 => Err(DialogError::routing_error(
                "no open initial INVITE server transaction owns this dialog",
            )),
            _ => Err(DialogError::routing_error(
                "multiple open INVITE server transactions make the call response ambiguous",
            )),
        }
    }

    /// Confirm a UAS dialog after sending a 2xx to an initial INVITE
    ///
    /// This method handles the dialog state transition from Early to Confirmed
    /// for UAS (server) dialogs. It:
    /// 1. Extracts the local tag from the successful response
    /// 2. Updates the dialog's local_tag field
    /// 3. Transitions the dialog state to Confirmed
    /// 4. Registers the dialog in the lookup table
    ///
    /// # Arguments
    /// * `dialog_id` - The dialog to confirm
    /// * `response` - The 2xx response that was sent
    ///
    /// # Returns
    /// Ok(()) if confirmation succeeded, Err if dialog not found or invalid state
    fn confirm_uas_dialog(&self, dialog_id: &DialogId, response: &Response) -> DialogResult<()> {
        debug!(
            "Confirming UAS dialog {} after successful 2xx to INVITE",
            dialog_id
        );

        // Validate every fallible field before changing dialog state. The
        // preflight already checked these values, but doing so again under the
        // write guard closes a concurrent lifecycle-change window.
        let response_local_tag = response
            .to()
            .and_then(|to_header| to_header.tag())
            .filter(|tag| !tag.is_empty())
            .map(str::to_owned);

        let mut dialog = self.get_dialog_mut(dialog_id)?;
        if dialog.state != DialogState::Early {
            debug!(
                "Dialog {} already in {:?} state, not transitioning",
                dialog_id, dialog.state
            );
            return Ok(());
        }

        let local_tag = dialog
            .local_tag
            .as_deref()
            .filter(|tag| !tag.is_empty())
            .map(str::to_owned)
            .or(response_local_tag)
            .ok_or_else(|| {
                DialogError::protocol_error("successful initial INVITE response must have a To tag")
            })?;
        let remote_tag = dialog
            .remote_tag
            .as_deref()
            .filter(|tag| !tag.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| {
                DialogError::protocol_error("initial INVITE dialog is missing its remote tag")
            })?;
        let call_id = dialog.call_id.clone();
        let early_key = DialogUtils::create_early_lookup_key(&call_id, &remote_tag);
        let confirmed_key = DialogUtils::create_lookup_key(&call_id, &local_tag, &remote_tag);

        dialog.local_tag = Some(local_tag);
        dialog.state = DialogState::Confirmed;
        drop(dialog);

        self.early_dialog_lookup.remove(&early_key);
        self.dialog_lookup.insert(confirmed_key, dialog_id.clone());
        info!(
            "Dialog {} transitioned Early -> Confirmed after successful UAS 2xx send",
            dialog_id
        );

        Ok(())
    }

    /// Terminate an early UAS dialog before sending a final non-2xx response
    /// to the initial INVITE.
    ///
    /// RFC 3261 §12.3 says early dialogs terminate when a non-2xx final
    /// response is sent for the initial INVITE. Removing the early lookup here
    /// closes the race where an authenticated retry after 401/407 could arrive
    /// before upper-layer session cleanup and be misclassified as a re-INVITE.
    fn terminate_uas_early_dialog_for_final_response(
        &self,
        dialog_id: &DialogId,
    ) -> DialogResult<()> {
        let mut dialog = self.get_dialog_mut(dialog_id)?;
        if dialog.state != DialogState::Early {
            debug!(
                "Dialog {} is {:?}, not terminating as rejected early dialog",
                dialog_id, dialog.state
            );
            return Ok(());
        }

        if let Some(remote_tag) = dialog.remote_tag.as_ref() {
            let early_key = DialogUtils::create_early_lookup_key(&dialog.call_id, remote_tag);
            self.early_dialog_lookup.remove(&early_key);
        }

        dialog.state = DialogState::Terminated;
        info!(
            "Dialog {} transitioned Early -> Terminated (UAS sending final non-2xx INVITE response)",
            dialog_id
        );
        Ok(())
    }
}

fn set_response_to_tag(response: &mut Response, tag: &str) -> DialogResult<()> {
    let to_index = response
        .headers
        .iter()
        .position(|header| header.name() == HeaderName::To)
        .ok_or_else(|| DialogError::protocol_error("INVITE response is missing To"))?;
    let TypedHeader::To(to) = response.headers[to_index].clone() else {
        return Err(DialogError::protocol_error(
            "INVITE response has a malformed To header",
        ));
    };
    response.headers[to_index] = TypedHeader::To(to.with_tag(tag));
    Ok(())
}

fn add_contact_header(response: &mut Response, contact_uri: &str) -> DialogResult<()> {
    use rvoip_sip_core::types::{
        address::Address,
        contact::{Contact, ContactParamInfo},
    };

    let uri = contact_uri
        .parse::<Uri>()
        .map_err(|_| DialogError::protocol_error("configured Contact URI is invalid"))?;
    response
        .headers
        .retain(|header| header.name() != HeaderName::Contact);
    response
        .headers
        .push(TypedHeader::Contact(Contact::new_params(vec![
            ContactParamInfo {
                address: Address::new(uri),
            },
        ])));
    Ok(())
}

fn is_response_stack_managed(name: &HeaderName) -> bool {
    matches!(
        name,
        HeaderName::CallId
            | HeaderName::CSeq
            | HeaderName::Via
            | HeaderName::ContentLength
            | HeaderName::RecordRoute
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::DialogLookup;
    use crate::transaction::TransactionManager;
    use async_trait::async_trait;
    use rvoip_sip_core::builder::SimpleRequestBuilder;
    use rvoip_sip_core::StatusCode;
    use rvoip_sip_transport::error::{Error as TransportError, Result as TransportResult};
    use rvoip_sip_transport::{Transport, TransportEvent};
    use std::net::SocketAddr;
    use std::str::FromStr;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use tokio::sync::mpsc;

    #[test]
    fn classified_final_response_uses_only_transaction_completion_state() {
        let source = include_str!("response_lifecycle.rs");
        let classified = source
            .split("pub(crate) async fn send_exact_final_response_classified")
            .nth(1)
            .and_then(|tail| tail.split("/// Protocol-handler policy").next())
            .expect("classified response primitive source");
        assert!(classified.contains("classify_final_response_completion(transaction_id)"));
        assert!(!classified.contains("session_to_dialog"));
        assert!(!classified.contains("pending_response_transaction_for_dialog"));
        assert!(!classified.contains("server_transactions_for_dialog"));
        assert!(!classified.contains("error.to_string()"));

        let unowned = source
            .split("pub(crate) async fn send_unowned_final_response_classified")
            .nth(1)
            .and_then(|tail| tail.split("/// Send the one final response").next())
            .expect("unowned response policy source");
        assert!(unowned.contains("WireUnknownErrorTerminal"));
        assert!(unowned.contains("Err(error) => Err(error.source)"));
    }

    #[derive(Debug)]
    struct NoopTransport {
        addr: SocketAddr,
        closed: AtomicBool,
    }

    #[derive(Debug)]
    struct FailingTransport {
        addr: SocketAddr,
    }

    impl NoopTransport {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                addr: SocketAddr::from_str("127.0.0.1:5060").unwrap(),
                closed: AtomicBool::new(false),
            })
        }
    }

    #[async_trait]
    impl Transport for FailingTransport {
        fn local_addr(&self) -> TransportResult<SocketAddr> {
            Ok(self.addr)
        }

        async fn send_message(
            &self,
            _message: rvoip_sip_core::Message,
            _destination: SocketAddr,
        ) -> TransportResult<()> {
            Err(TransportError::TransportClosed)
        }

        async fn close(&self) -> TransportResult<()> {
            Ok(())
        }

        fn is_closed(&self) -> bool {
            true
        }
    }

    #[async_trait]
    impl Transport for NoopTransport {
        fn local_addr(&self) -> TransportResult<SocketAddr> {
            Ok(self.addr)
        }

        async fn send_message(
            &self,
            _message: rvoip_sip_core::Message,
            _destination: SocketAddr,
        ) -> TransportResult<()> {
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

    async fn make_manager() -> DialogManager {
        make_manager_with_transport(NoopTransport::new()).await
    }

    async fn make_manager_with_transport(transport: Arc<dyn Transport>) -> DialogManager {
        let (_tx, transport_rx) = mpsc::channel::<TransportEvent>(16);
        let (transaction_manager, _events_rx) =
            TransactionManager::new(transport, transport_rx, Some(16))
                .await
                .expect("build TransactionManager");
        DialogManager::new(
            Arc::new(transaction_manager),
            SocketAddr::from_str("127.0.0.1:5060").unwrap(),
        )
        .await
        .expect("build DialogManager")
    }

    async fn make_failing_manager() -> DialogManager {
        make_manager_with_transport(Arc::new(FailingTransport {
            addr: SocketAddr::from_str("127.0.0.1:5060").unwrap(),
        }))
        .await
    }

    fn initial_invite() -> Request {
        SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
            .unwrap()
            .from("Alice", "sip:alice@example.com", Some("alice-tag"))
            .to("Bob", "sip:bob@example.com", None)
            .contact("sip:alice@127.0.0.1:5061", None)
            .call_id("auth-retry-dialog-test")
            .cseq(1)
            .via("127.0.0.1:5061", "UDP", Some("z9hG4bK-auth-retry"))
            .max_forwards(70)
            .build()
    }

    #[tokio::test]
    async fn final_non_2xx_commits_only_after_the_wire_boundary() {
        let manager = make_manager().await;
        let request = initial_invite();
        let dialog_id = manager
            .create_early_dialog_from_invite(&request)
            .await
            .expect("create early dialog");

        assert_eq!(
            manager.find_dialog_for_request(&request).await,
            Some(dialog_id.clone()),
            "initial early dialog should be discoverable before final response"
        );

        let response = Response::new(StatusCode::Unauthorized);
        let transaction_id =
            TransactionKey::new("z9hG4bK-auth-retry".to_string(), Method::Invite, true);

        manager
            .pre_send_response(&dialog_id, &response, &transaction_id, &request)
            .await
            .expect("pre-send lifecycle");

        assert_eq!(
            manager
                .get_dialog_state(&dialog_id)
                .expect("dialog should remain before the send"),
            DialogState::Early,
            "pre-send validation must not publish a terminal dialog state"
        );
        assert_eq!(
            manager.find_dialog_for_request(&request).await,
            Some(dialog_id.clone()),
            "pre-send validation must not remove the early lookup"
        );

        manager
            .commit_sent_response_lifecycle(&dialog_id, &response, &request)
            .expect("post-send lifecycle commit");

        assert_eq!(
            manager
                .get_dialog_state(&dialog_id)
                .expect("dialog should remain until upper-layer cleanup"),
            DialogState::Terminated
        );
        assert_eq!(
            manager.find_dialog_for_request(&request).await,
            None,
            "a no-To-tag authenticated retry must not resolve as a re-INVITE"
        );
    }

    #[tokio::test]
    async fn every_initial_invite_2xx_confirms_only_at_post_send_commit() {
        let manager = make_manager().await;
        let request = initial_invite();
        let dialog_id = manager
            .create_early_dialog_from_invite(&request)
            .await
            .expect("create early dialog");
        let mut response = rvoip_sip_core::builder::SimpleResponseBuilder::response_from_request(
            &request,
            StatusCode::Accepted,
            None,
        )
        .build();
        set_response_to_tag(&mut response, "server-tag").expect("stamp To tag");
        let transaction_id =
            TransactionKey::new("z9hG4bK-accepted".to_string(), Method::Invite, true);

        manager
            .pre_send_response(&dialog_id, &response, &transaction_id, &request)
            .await
            .expect("pre-send lifecycle");
        assert_eq!(
            manager.get_dialog_state(&dialog_id).unwrap(),
            DialogState::Early
        );

        manager
            .commit_sent_response_lifecycle(&dialog_id, &response, &request)
            .expect("post-send lifecycle commit");
        assert_eq!(
            manager.get_dialog_state(&dialog_id).unwrap(),
            DialogState::Confirmed
        );
    }

    #[tokio::test]
    async fn failed_final_response_send_leaves_early_dialog_uncommitted() {
        let manager = make_failing_manager().await;
        let request = initial_invite();
        let dialog_id = manager
            .create_early_dialog_from_invite(&request)
            .await
            .expect("create early dialog");
        let transaction = manager
            .transaction_manager()
            .create_server_transaction(
                request.clone(),
                SocketAddr::from_str("127.0.0.1:5061").unwrap(),
            )
            .await
            .expect("create server transaction");
        let transaction_id = transaction.id().clone();
        manager.associate_transaction_with_dialog(&transaction_id, &dialog_id);
        manager
            .pending_response_transaction_by_dialog
            .insert(dialog_id.clone(), transaction_id.clone());

        assert!(manager
            .send_known_transaction_response(
                &dialog_id,
                &transaction_id,
                StatusCode::Unauthorized.as_u16(),
                None,
                None,
                &[],
                None,
            )
            .await
            .is_err());
        assert_eq!(
            manager.get_dialog_state(&dialog_id).unwrap(),
            DialogState::Early,
            "a failed wire send must not publish the rejection lifecycle"
        );
        assert_eq!(
            manager.find_dialog_for_request(&request).await,
            Some(dialog_id),
            "a failed wire send must preserve the early-dialog lookup"
        );
    }
}
