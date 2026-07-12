//! Metadata-only diagnostic views for transaction-layer values.
//!
//! These wrappers are deliberately separate from functional `Display`/`Debug`
//! implementations. Transaction keys and SIP messages participate in legacy
//! event/API round trips, while logs must never reveal peer-controlled branch,
//! extension-method, URI, header, body, reason, or arbitrary error text.

use std::fmt;

use rvoip_sip_core::{Message, Method, Request, Response};

use super::{
    error::Error, InternalTransactionCommand, SipRequestRejection, TransactionEvent, TransactionKey,
};

/// A standard method name or the literal classifier `extension`.
pub struct SafeMethod<'a>(&'a Method);

impl<'a> SafeMethod<'a> {
    pub const fn new(method: &'a Method) -> Self {
        Self(method)
    }
}

impl fmt::Display for SafeMethod<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Method::Invite => f.write_str("INVITE"),
            Method::Ack => f.write_str("ACK"),
            Method::Bye => f.write_str("BYE"),
            Method::Cancel => f.write_str("CANCEL"),
            Method::Register => f.write_str("REGISTER"),
            Method::Options => f.write_str("OPTIONS"),
            Method::Subscribe => f.write_str("SUBSCRIBE"),
            Method::Notify => f.write_str("NOTIFY"),
            Method::Update => f.write_str("UPDATE"),
            Method::Refer => f.write_str("REFER"),
            Method::Info => f.write_str("INFO"),
            Method::Message => f.write_str("MESSAGE"),
            Method::Prack => f.write_str("PRACK"),
            Method::Publish => f.write_str("PUBLISH"),
            Method::Extension(_) => f.write_str("extension"),
        }
    }
}

impl fmt::Debug for SafeMethod<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// A log-only transaction identity that never renders the Via branch.
pub struct SafeTransactionKey<'a>(&'a TransactionKey);

impl<'a> SafeTransactionKey<'a> {
    pub const fn new(key: &'a TransactionKey) -> Self {
        Self(key)
    }
}

impl fmt::Display for SafeTransactionKey<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "transaction(method={}, side={}, branch_len={})",
            SafeMethod::new(&self.0.method),
            if self.0.is_server { "server" } else { "client" },
            self.0.branch.len()
        )
    }
}

impl fmt::Debug for SafeTransactionKey<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Metadata-only diagnostic view of a SIP request.
pub struct SafeSipRequest<'a>(&'a Request);

impl<'a> SafeSipRequest<'a> {
    pub const fn new(request: &'a Request) -> Self {
        Self(request)
    }
}

impl fmt::Debug for SafeSipRequest<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SipRequest")
            .field("method", &SafeMethod::new(&self.0.method()))
            .field("header_count", &self.0.all_headers().len())
            .field("body_len", &self.0.body().len())
            .finish()
    }
}

impl fmt::Display for SafeSipRequest<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

/// Metadata-only diagnostic view of a SIP response.
pub struct SafeSipResponse<'a>(&'a Response);

impl<'a> SafeSipResponse<'a> {
    pub const fn new(response: &'a Response) -> Self {
        Self(response)
    }
}

impl fmt::Debug for SafeSipResponse<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SipResponse")
            .field("status", &self.0.status())
            .field("header_count", &self.0.all_headers().len())
            .field("body_len", &self.0.body().len())
            .finish()
    }
}

impl fmt::Display for SafeSipResponse<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

/// Metadata-only diagnostic view of either SIP message kind.
pub struct SafeSipMessage<'a>(&'a Message);

impl<'a> SafeSipMessage<'a> {
    pub const fn new(message: &'a Message) -> Self {
        Self(message)
    }
}

impl fmt::Debug for SafeSipMessage<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Message::Request(request) => SafeSipRequest::new(request).fmt(f),
            Message::Response(response) => SafeSipResponse::new(response).fmt(f),
        }
    }
}

impl fmt::Display for SafeSipMessage<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

/// Metadata-only diagnostic view of an internal command.
pub struct SafeTransactionCommand<'a>(&'a InternalTransactionCommand);

impl<'a> SafeTransactionCommand<'a> {
    pub const fn new(command: &'a InternalTransactionCommand) -> Self {
        Self(command)
    }
}

impl fmt::Debug for SafeTransactionCommand<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            InternalTransactionCommand::TransitionTo(state) => {
                f.debug_tuple("TransitionTo").field(state).finish()
            }
            InternalTransactionCommand::ProcessMessage(message) => f
                .debug_tuple("ProcessMessage")
                .field(&SafeSipMessage::new(message))
                .finish(),
            InternalTransactionCommand::Timer(timer) => f
                .debug_struct("Timer")
                .field("name_len", &timer.len())
                .finish(),
            InternalTransactionCommand::TransportError => f.write_str("TransportError"),
            InternalTransactionCommand::Terminate => f.write_str("Terminate"),
            InternalTransactionCommand::CancelTimer100 => f.write_str("CancelTimer100"),
        }
    }
}

impl fmt::Display for SafeTransactionCommand<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

/// Metadata-only diagnostic view of a transaction event.
pub struct SafeTransactionEvent<'a>(&'a TransactionEvent);

impl<'a> SafeTransactionEvent<'a> {
    pub const fn new(event: &'a TransactionEvent) -> Self {
        Self(event)
    }
}

impl fmt::Debug for SafeTransactionEvent<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use TransactionEvent::*;
        match self.0 {
            AckReceived {
                transaction_id,
                request,
            } => event_with_request(f, "AckReceived", Some(transaction_id), request),
            CancelReceived {
                transaction_id,
                cancel_request,
            } => event_with_request(f, "CancelReceived", Some(transaction_id), cancel_request),
            ProvisionalResponse {
                transaction_id,
                response,
            } => event_with_response(f, "ProvisionalResponse", Some(transaction_id), response),
            SuccessResponse {
                transaction_id,
                response,
                need_ack,
                source,
            } => f
                .debug_struct("SuccessResponse")
                .field("transaction", &SafeTransactionKey::new(transaction_id))
                .field("response", &SafeSipResponse::new(response))
                .field("need_ack", need_ack)
                .field("source", source)
                .finish(),
            FailureResponse {
                transaction_id,
                response,
            } => event_with_response(f, "FailureResponse", Some(transaction_id), response),
            ProvisionalResponseSent {
                transaction_id,
                response,
            } => event_with_response(f, "ProvisionalResponseSent", Some(transaction_id), response),
            FinalResponseSent {
                transaction_id,
                response,
            } => event_with_response(f, "FinalResponseSent", Some(transaction_id), response),
            TransactionTimeout { transaction_id } => {
                event_key(f, "TransactionTimeout", transaction_id)
            }
            AckTimeout { transaction_id } => event_key(f, "AckTimeout", transaction_id),
            TransportError { transaction_id } => event_key(f, "TransportError", transaction_id),
            Error {
                transaction_id,
                error,
            } => f
                .debug_struct("Error")
                .field(
                    "transaction",
                    &transaction_id.as_ref().map(SafeTransactionKey::new),
                )
                .field("error_len", &error.len())
                .finish(),
            StrayRequest { request, source } => stray_request(f, "StrayRequest", request, source),
            StrayAck { request, source } => stray_request(f, "StrayAck", request, source),
            StrayCancel { request, source } => stray_request(f, "StrayCancel", request, source),
            StrayAckRequest { request, source } => {
                stray_request(f, "StrayAckRequest", request, source)
            }
            StrayResponse { response, source } => f
                .debug_struct("StrayResponse")
                .field("response", &SafeSipResponse::new(response))
                .field("source", source)
                .finish(),
            TransactionTerminated { transaction_id } => {
                event_key(f, "TransactionTerminated", transaction_id)
            }
            StateChanged {
                transaction_id,
                previous_state,
                new_state,
            } => f
                .debug_struct("StateChanged")
                .field("transaction", &SafeTransactionKey::new(transaction_id))
                .field("previous_state", previous_state)
                .field("new_state", new_state)
                .finish(),
            TimerTriggered {
                transaction_id,
                timer,
            } => f
                .debug_struct("TimerTriggered")
                .field("transaction", &SafeTransactionKey::new(transaction_id))
                .field("timer_len", &timer.len())
                .finish(),
            CancelRequest {
                transaction_id,
                target_transaction_id,
                request,
                source,
            } => f
                .debug_struct("CancelRequest")
                .field("transaction", &SafeTransactionKey::new(transaction_id))
                .field(
                    "target_transaction",
                    &SafeTransactionKey::new(target_transaction_id),
                )
                .field("request", &SafeSipRequest::new(request))
                .field("source", source)
                .finish(),
            AckRequest {
                transaction_id,
                request,
                source,
            } => request_event(f, "AckRequest", transaction_id, request, source),
            InviteRequest {
                transaction_id,
                request,
                source,
            } => request_event(f, "InviteRequest", transaction_id, request, source),
            NonInviteRequest {
                transaction_id,
                request,
                source,
            } => request_event(f, "NonInviteRequest", transaction_id, request, source),
            ShutdownRequested => f.write_str("ShutdownRequested"),
            ShutdownReady => f.write_str("ShutdownReady"),
            ShutdownNow => f.write_str("ShutdownNow"),
            ShutdownComplete => f.write_str("ShutdownComplete"),
        }
    }
}

impl fmt::Display for SafeTransactionEvent<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

fn event_key(f: &mut fmt::Formatter<'_>, name: &str, key: &TransactionKey) -> fmt::Result {
    f.debug_struct(name)
        .field("transaction", &SafeTransactionKey::new(key))
        .finish()
}

fn event_with_request(
    f: &mut fmt::Formatter<'_>,
    name: &str,
    key: Option<&TransactionKey>,
    request: &Request,
) -> fmt::Result {
    f.debug_struct(name)
        .field("transaction", &key.map(SafeTransactionKey::new))
        .field("request", &SafeSipRequest::new(request))
        .finish()
}

fn event_with_response(
    f: &mut fmt::Formatter<'_>,
    name: &str,
    key: Option<&TransactionKey>,
    response: &Response,
) -> fmt::Result {
    f.debug_struct(name)
        .field("transaction", &key.map(SafeTransactionKey::new))
        .field("response", &SafeSipResponse::new(response))
        .finish()
}

fn stray_request(
    f: &mut fmt::Formatter<'_>,
    name: &str,
    request: &Request,
    source: &std::net::SocketAddr,
) -> fmt::Result {
    f.debug_struct(name)
        .field("request", &SafeSipRequest::new(request))
        .field("source", source)
        .finish()
}

fn request_event(
    f: &mut fmt::Formatter<'_>,
    name: &str,
    key: &TransactionKey,
    request: &Request,
    source: &std::net::SocketAddr,
) -> fmt::Result {
    f.debug_struct(name)
        .field("transaction", &SafeTransactionKey::new(key))
        .field("request", &SafeSipRequest::new(request))
        .field("source", source)
        .finish()
}

/// Metadata-only diagnostic view of transaction errors.
pub struct SafeTransactionError<'a>(&'a Error);

impl<'a> SafeTransactionError<'a> {
    pub const fn new(error: &'a Error) -> Self {
        Self(error)
    }
}

impl fmt::Display for SafeTransactionError<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use Error::*;
        match self.0 {
            SipCoreError(_) => f.write_str("sip_core"),
            TransportError { .. } => f.write_str("transport"),
            TransactionNotFound { .. } => f.write_str("transaction_not_found"),
            TransactionExists { .. } => f.write_str("transaction_exists"),
            InvalidStateTransition { .. } => f.write_str("invalid_state_transition"),
            TransactionTimeout { .. } => f.write_str("transaction_timeout"),
            TimerError { .. } => f.write_str("timer"),
            Io(_) => f.write_str("io"),
            ChannelError { .. } => f.write_str("channel"),
            TransactionCreationError { .. } => f.write_str("transaction_creation"),
            MessageProcessingError { .. } => f.write_str("message_processing"),
            Transport(_) => f.write_str("transport_manager"),
            Other(_) => f.write_str("other"),
        }
    }
}

impl fmt::Debug for SafeTransactionError<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// A fail-closed view for an error type whose external text is not trusted.
///
/// Callers should prefer [`SafeTransactionError`] when the concrete transaction
/// error is available. This wrapper is for channel, transport, parser, and
/// builder errors crossing generic boundaries where only the failure class is
/// safe to retain.
pub struct SafeOpaqueError<'a, E: ?Sized>(&'a E);

impl<'a, E: ?Sized> SafeOpaqueError<'a, E> {
    pub const fn new(error: &'a E) -> Self {
        Self(error)
    }
}

impl<E: ?Sized> fmt::Display for SafeOpaqueError<'_, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = self.0;
        f.write_str("operation_error")
    }
}

impl<E: ?Sized> fmt::Debug for SafeOpaqueError<'_, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Metadata-only diagnostic view of an authorizer rejection.
pub struct SafeSipRequestRejection<'a>(&'a SipRequestRejection);

impl<'a> SafeSipRequestRejection<'a> {
    pub const fn new(rejection: &'a SipRequestRejection) -> Self {
        Self(rejection)
    }
}

impl fmt::Debug for SafeSipRequestRejection<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SipRequestRejection")
            .field("status", &self.0.status)
            .field("header_count", &self.0.headers.len())
            .field("has_reason", &self.0.reason.is_some())
            .finish()
    }
}

impl fmt::Display for SafeSipRequestRejection<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use rvoip_sip_core::{HeaderName, HeaderValue, StatusCode, TypedHeader, Uri};

    use super::*;

    const SECRETS: [&str; 8] = [
        "branch-secret",
        "EXTENSION-SECRET",
        "uri-secret",
        "authorization-secret",
        "body-secret",
        "reason-secret",
        "error-secret",
        "header-secret",
    ];

    fn assert_redacted(rendered: &str) {
        for secret in SECRETS {
            assert!(
                !rendered.contains(secret),
                "diagnostic leaked {secret}: {rendered}"
            );
        }
    }

    #[test]
    fn safe_views_redact_transaction_message_event_command_error_and_rejection_secrets() {
        let key = TransactionKey::new(
            "branch-secret".into(),
            Method::Extension("EXTENSION-SECRET".into()),
            true,
        );
        let request = Request::new(
            Method::Extension("EXTENSION-SECRET".into()),
            Uri::custom("sip:uri-secret@example.invalid"),
        )
        .with_header(TypedHeader::Other(
            HeaderName::Other("Authorization".into()),
            HeaderValue::Raw("authorization-secret header-secret".into()),
        ))
        .with_body(Bytes::from_static(b"body-secret"));
        let message = Message::Request(request.clone());
        let response_message = Message::Response(
            Response::new(StatusCode::Ok)
                .with_reason("reason-secret")
                .with_body(Bytes::from_static(b"body-secret")),
        );
        let command = InternalTransactionCommand::ProcessMessage(message.clone());
        let event = TransactionEvent::Error {
            transaction_id: Some(key.clone()),
            error: "error-secret".into(),
        };
        let error = Error::Other("error-secret".into());
        let rejection = SipRequestRejection::new(StatusCode::Unauthorized)
            .with_header(TypedHeader::Other(
                HeaderName::Other("WWW-Authenticate".into()),
                HeaderValue::Raw("authorization-secret".into()),
            ))
            .with_reason("reason-secret");

        for rendered in [
            format!("{}", SafeTransactionKey::new(&key)),
            format!("{:?}", SafeSipMessage::new(&message)),
            format!("{:?}", SafeSipMessage::new(&response_message)),
            format!("{:?}", SafeTransactionCommand::new(&command)),
            format!("{:?}", SafeTransactionEvent::new(&event)),
            format!("{}", SafeTransactionError::new(&error)),
            format!("{:?}", SafeSipRequestRejection::new(&rejection)),
            format!("{command:?}"),
            format!("{event:?}"),
            format!("{rejection:?}"),
        ] {
            assert_redacted(&rendered);
        }
    }

    #[test]
    fn transaction_log_source_uses_only_metadata_safe_views() {
        use std::fs;
        use std::path::{Path, PathBuf};

        fn rust_files(path: &Path, out: &mut Vec<PathBuf>) {
            for entry in fs::read_dir(path).expect("read source directory") {
                let path = entry.expect("source entry").path();
                if path.is_dir() {
                    rust_files(&path, out);
                } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs")
                    && path.file_name().and_then(|name| name.to_str())
                        != Some("safe_diagnostics.rs")
                {
                    out.push(path);
                }
            }
        }

        fn check_log_block(path: &Path, block: &str) {
            fn contains_identifier(source: &str, identifier: &str) -> bool {
                source.match_indices(identifier).any(|(start, _)| {
                    let before = source[..start].chars().next_back();
                    let after = source[start + identifier.len()..].chars().next();
                    let is_ident =
                        |character: char| character.is_ascii_alphanumeric() || character == '_';
                    before.map_or(true, |character| !is_ident(character))
                        && after.map_or(true, |character| !is_ident(character))
                })
            }

            let key_tokens = [
                "tx_id",
                "transaction_id",
                "invite_key",
                "invite_tx_id",
                "cancel_tx_id",
                "id_for_logging",
                "data.id",
                "self.id",
                "transaction.id()",
                "tx.id()",
            ];
            if key_tokens
                .iter()
                .any(|token| contains_identifier(block, token))
            {
                assert!(
                    block.contains("SafeTransactionKey"),
                    "{} contains an unwrapped transaction key log:\n{}",
                    path.display(),
                    block
                );
            }

            if block.contains(".method") || block.contains("method,") || block.contains("%method") {
                assert!(
                    block.contains("SafeMethod")
                        || block.contains("safe_method_label")
                        || block.contains("status"),
                    "{} contains an unclassified method log:\n{}",
                    path.display(),
                    block
                );
            }

            for (needle, safe_marker) in [
                ("?command", "SafeTransactionCommand"),
                ("?message", "SafeSipMessage"),
                ("?event", "SafeTransactionEvent"),
            ] {
                if block.contains(needle) {
                    assert!(
                        block.contains(safe_marker),
                        "{} contains an unsafe derived diagnostic:\n{}",
                        path.display(),
                        block
                    );
                }
            }

            if block.contains("error=%e")
                || block.contains("error = %e")
                || block.contains("error=?e")
                || block.contains("error = ?e")
                || block.contains(", e\n")
            {
                assert!(
                    block.contains("SafeOpaqueError") || block.contains("SafeTransactionError"),
                    "{} contains an unclassified error log:\n{}",
                    path.display(),
                    block
                );
            }

            for forbidden in [
                "received_branch=?",
                "expected_branch=%",
                "Via[",
                "top_via = %",
                "uri = %",
                "top_route = %",
                "next_hop = %",
                "domain = %",
                "reason = %",
                "String::from_utf8_lossy",
            ] {
                assert!(
                    !block.contains(forbidden),
                    "{} contains forbidden transaction log form {forbidden}:\n{}",
                    path.display(),
                    block
                );
            }

            if block.contains("reason_phrase()") {
                assert!(
                    block.contains("reason_len"),
                    "{} renders an untrusted response reason:\n{}",
                    path.display(),
                    block
                );
            }
        }

        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();
        rust_files(&manifest.join("src/transaction"), &mut files);
        files.push(manifest.join("src/manager/transaction_integration.rs"));

        for path in files {
            let source = fs::read_to_string(&path).expect("read Rust source");
            let mut block = String::new();
            let mut in_log = false;
            for line in source.lines() {
                if !in_log
                    && ["trace!(", "debug!(", "info!(", "warn!(", "error!("]
                        .iter()
                        .any(|marker| line.contains(marker))
                {
                    in_log = true;
                    block.clear();
                }
                if in_log {
                    block.push_str(line);
                    block.push('\n');
                    if line.contains(");") {
                        check_log_block(&path, &block);
                        in_log = false;
                    }
                }
            }
            assert!(
                !in_log,
                "unterminated tracing macro scan in {}",
                path.display()
            );
        }

        let parser = fs::read_to_string(manifest.join("../sip-core/src/parser/message.rs"))
            .expect("read SIP parser source");
        assert!(!parser.contains("String::from_utf8_lossy(e.input)"));
        assert!(parser.contains("remaining_len = e.input.len()"));
    }
}
