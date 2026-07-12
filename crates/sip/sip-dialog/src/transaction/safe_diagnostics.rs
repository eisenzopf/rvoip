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

/// A bounded-cardinality classifier for caller-supplied timer names.
pub struct SafeTimerName<'a>(&'a str);

impl<'a> SafeTimerName<'a> {
    pub const fn new(timer_name: &'a str) -> Self {
        Self(timer_name)
    }
}

impl fmt::Display for SafeTimerName<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            "A" | "B" | "C" | "D" | "E" | "F" | "G" | "H" | "I" | "J" | "K" => f.write_str(self.0),
            "100" | "Timer_100" => f.write_str("100"),
            _ => f.write_str("other"),
        }
    }
}

impl fmt::Debug for SafeTimerName<'_> {
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

    const SECRETS: [&str; 9] = [
        "branch-secret",
        "EXTENSION-SECRET",
        "uri-secret",
        "authorization-secret",
        "body-secret",
        "reason-secret",
        "error-secret",
        "header-secret",
        "timer-name-secret",
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
            format!("{}", SafeTimerName::new("timer-name-secret")),
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

        assert_eq!(SafeTimerName::new("Timer_100").to_string(), "100");
        assert_eq!(SafeTimerName::new("timer-name-secret").to_string(), "other");
    }

    /// Extract tracing macro invocations with balanced parentheses. Parentheses
    /// inside normal string literals are ignored, as are escaped quotes. This is
    /// intentionally independent of line layout: nested calls may finish before
    /// a sensitive argument later in the same tracing invocation.
    fn extract_log_blocks(source: &str) -> Vec<String> {
        const MARKERS: [&str; 5] = ["trace!(", "debug!(", "info!(", "warn!(", "error!("];

        let mut blocks = Vec::new();
        let mut cursor = 0;
        while cursor < source.len() {
            let next = MARKERS
                .iter()
                .filter_map(|marker| source[cursor..].find(marker).map(|offset| (offset, marker)))
                .min_by_key(|(offset, _)| *offset);
            let Some((offset, marker)) = next else {
                break;
            };
            let start = cursor + offset;
            let bytes = source.as_bytes();
            let mut index = start + marker.len() - 1;
            let mut depth = 0usize;
            let mut in_string = false;
            let mut escaped = false;
            let mut end = None;

            while index < bytes.len() {
                let byte = bytes[index];
                if in_string {
                    if escaped {
                        escaped = false;
                    } else if byte == b'\\' {
                        escaped = true;
                    } else if byte == b'"' {
                        in_string = false;
                    }
                } else if byte == b'"' {
                    in_string = true;
                } else if byte == b'(' {
                    depth += 1;
                } else if byte == b')' {
                    depth = depth.checked_sub(1).expect("balanced log macro scanner");
                    if depth == 0 {
                        end = Some(index + 1);
                        break;
                    }
                }
                index += 1;
            }

            let end = end.unwrap_or_else(|| panic!("unterminated tracing macro at byte {start}"));
            blocks.push(source[start..end].to_string());
            cursor = end;
        }
        blocks
    }

    fn compact(source: &str) -> String {
        source
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect()
    }

    fn contains_identifier(source: &str, identifier: &str) -> bool {
        source.match_indices(identifier).any(|(start, _)| {
            let before = source[..start].chars().next_back();
            let after = source[start + identifier.len()..].chars().next();
            let is_identifier =
                |character: char| character.is_ascii_alphanumeric() || character == '_';
            before.is_none_or(|character| !is_identifier(character))
                && after.is_none_or(|character| !is_identifier(character))
        })
    }

    /// Returns true only when an expression is rendered as a tracing field or
    /// format argument. Merely using an expression to compute safe metadata does
    /// not count as rendering it.
    fn renders_expression(compact_block: &str, expression: &str) -> bool {
        let patterns = [
            format!("=%{expression},"),
            format!("=%{expression})"),
            format!("=?{expression},"),
            format!("=?{expression})"),
            format!("(%{expression},"),
            format!("(%{expression})"),
            format!("(?{expression},"),
            format!("(?{expression})"),
            format!(",%{expression},"),
            format!(",%{expression})"),
            format!(",?{expression},"),
            format!(",?{expression})"),
            format!("({expression},"),
            format!(",{expression},"),
            format!(",{expression})"),
        ];
        patterns
            .iter()
            .any(|pattern| compact_block.contains(pattern))
    }

    fn scan_log_block(path: &str, block: &str) -> Vec<&'static str> {
        let compact_block = compact(block);
        let mut findings = Vec::new();
        let renders_any = |expressions: &[&str]| {
            expressions
                .iter()
                .any(|expression| renders_expression(&compact_block, expression))
        };

        if renders_any(&[
            "tx_id",
            "tx_id_clone",
            "tx_id_for_timer",
            "tx_id_for_task",
            "tx_key",
            "transaction_id",
            "invite_key",
            "invite_tx_id",
            "cancel_tx_id",
            "id_for_logging",
            "data.id",
            "self.id",
            "data.as_ref_key()",
            "transaction.id()",
            "tx.id()",
        ]) {
            findings.push("transaction_key");
        }

        if renders_any(&["e", "err", "error"]) {
            findings.push("error");
        }

        let references_timer_name = ["timer_name", "timer_name_clone"]
            .iter()
            .any(|name| contains_identifier(block, name));
        if renders_any(&["timer_name", "timer_name_clone"])
            || (references_timer_name
                && (!block.contains("SafeTimerName") || !block.contains("timer_len")))
        {
            findings.push("timer_name");
        }

        let typed_dialog_id_allowlist = path.ends_with("manager/transaction_integration.rs")
            || path.ends_with("transaction/server/reliable_invite.rs");
        if renders_any(&["dialog_id", "id_for_dialog"]) && !typed_dialog_id_allowlist {
            findings.push("dialog_id");
        }

        if renders_any(&[
            "method",
            "request.method()",
            "request.method",
            "modified_request.method()",
            "cseq.method",
            "cseq_header.method",
            "original_method",
            "key.method()",
        ]) {
            findings.push("method");
        }

        if renders_any(&[
            "branch",
            "received_branch",
            "expected_branch",
            "tx_id.branch",
        ]) {
            findings.push("branch");
        }

        if renders_any(&[
            "uri",
            "domain",
            "target_uri",
            "contact_addr.uri",
            "request.uri()",
            "next_hop_uri_for_request",
        ]) {
            findings.push("uri");
        }

        if renders_any(&["via", "top_via"])
            || block.contains("Via[")
            || compact_block.contains("top_via=%")
        {
            findings.push("via");
        }

        if renders_any(&["route", "top_route", "next_hop"]) {
            findings.push("route");
        }

        if renders_any(&["reason", "response.reason_phrase()"]) {
            findings.push("reason");
        }

        if renders_any(&["body", "sdp"]) || block.contains("String::from_utf8_lossy") {
            findings.push("body");
        }

        for (expression, finding) in [
            ("command", "command"),
            ("message", "message"),
            ("event", "event"),
        ] {
            if renders_expression(&compact_block, expression) {
                findings.push(finding);
            }
        }

        findings
    }

    fn scan_source(path: &str, source: &str) -> Vec<(&'static str, String)> {
        extract_log_blocks(source)
            .into_iter()
            .flat_map(|block| {
                scan_log_block(path, &block)
                    .into_iter()
                    .map(move |finding| (finding, block.clone()))
            })
            .collect()
    }

    fn scan_retention_source(source: &str) -> Vec<&'static str> {
        let safe = "SafeMethod::new(key.method()).to_string()";
        let mut findings = Vec::new();
        if source.contains("key.method().to_string()") {
            findings.push("retention_method");
        }
        if source.matches(safe).count() < 2 {
            findings.push("retention_safe_method_missing");
        }
        findings
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

        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();
        rust_files(&manifest.join("src/transaction"), &mut files);
        files.push(manifest.join("src/manager/transaction_integration.rs"));

        for path in files {
            let source = fs::read_to_string(&path).expect("read Rust source");
            let findings = scan_source(path.to_string_lossy().as_ref(), &source);
            assert!(findings.is_empty(), "{}: {findings:#?}", path.display());
        }

        let manager = fs::read_to_string(manifest.join("src/transaction/manager/mod.rs"))
            .expect("read transaction manager source");
        assert_eq!(scan_retention_source(&manager), Vec::<&str>::new());

        let parser = fs::read_to_string(manifest.join("../sip-core/src/parser/message.rs"))
            .expect("read SIP parser source");
        assert!(!parser.contains("String::from_utf8_lossy(e.input)"));
        assert!(parser.contains("remaining_len = e.input.len()"));
    }

    #[test]
    fn transaction_log_scanner_rejects_every_sensitive_field_class() {
        let mutations = [
            (
                "transaction/mutant.rs",
                r#"debug!("tx {}", tx_key);"#,
                "transaction_key",
            ),
            (
                "transaction/mutant.rs",
                r#"warn!("failed {}", err);"#,
                "error",
            ),
            (
                "transaction/mutant.rs",
                r#"warn!(timer = %timer_name, "timer");"#,
                "timer_name",
            ),
            (
                "transaction/server/builders.rs",
                r#"debug!("dialog {}", dialog_id);"#,
                "dialog_id",
            ),
            (
                "transaction/mutant.rs",
                r#"warn!(received_branch = ?branch, "bad");"#,
                "branch",
            ),
            (
                "transaction/mutant.rs",
                r#"debug!(method = %request.method(), "m");"#,
                "method",
            ),
            (
                "transaction/mutant.rs",
                r#"debug!(uri = %request.uri(), "u");"#,
                "uri",
            ),
            (
                "transaction/mutant.rs",
                r#"debug!("Via[{}]", via);"#,
                "via",
            ),
            (
                "transaction/mutant.rs",
                r#"debug!(route = %route, "r");"#,
                "route",
            ),
            (
                "transaction/mutant.rs",
                r#"warn!(reason = %reason, "r");"#,
                "reason",
            ),
            (
                "transaction/mutant.rs",
                r#"debug!(body = %body, "b");"#,
                "body",
            ),
            (
                "transaction/mutant.rs",
                r#"debug!(?command, "c");"#,
                "command",
            ),
            (
                "transaction/mutant.rs",
                r#"debug!(?message, "m");"#,
                "message",
            ),
            (
                "transaction/mutant.rs",
                r#"debug!(?event, "e");"#,
                "event",
            ),
            // A nested call ending on an earlier line must not hide the later
            // unsafe error operand from the balanced macro extractor.
            (
                "transaction/mutant.rs",
                "warn!(\n    state = ?classify(nested(value)),\n    error = %err,\n    \"failed\"\n);",
                "error",
            ),
        ];

        for (path, source, expected) in mutations {
            let findings = scan_source(path, source);
            assert!(
                findings.iter().any(|(finding, _)| *finding == expected),
                "scanner accepted unsafe {expected} mutation: {source}; findings={findings:#?}"
            );
        }

        let assert_spelling_rejected = |expression: &str, expected: &str| {
            let source = format!(r#"debug!(value = %{expression}, "unsafe");"#);
            let findings = scan_source("transaction/mutant.rs", &source);
            assert!(
                findings.iter().any(|(finding, _)| *finding == expected),
                "scanner accepted unsafe {expected} spelling {expression}; findings={findings:#?}"
            );
        };
        for expression in [
            "tx_id",
            "tx_id_clone",
            "tx_id_for_timer",
            "tx_id_for_task",
            "tx_key",
            "transaction_id",
            "invite_key",
            "invite_tx_id",
            "cancel_tx_id",
            "id_for_logging",
            "data.id",
            "self.id",
            "data.as_ref_key()",
            "transaction.id()",
            "tx.id()",
        ] {
            assert_spelling_rejected(expression, "transaction_key");
        }
        for expression in ["e", "err", "error"] {
            assert_spelling_rejected(expression, "error");
        }
        for expression in ["timer_name", "timer_name_clone"] {
            assert_spelling_rejected(expression, "timer_name");
        }
        for expression in [
            "method",
            "request.method()",
            "request.method",
            "modified_request.method()",
            "cseq.method",
            "cseq_header.method",
            "original_method",
            "key.method()",
        ] {
            assert_spelling_rejected(expression, "method");
        }
        for expression in [
            "uri",
            "domain",
            "target_uri",
            "contact_addr.uri",
            "request.uri()",
            "next_hop_uri_for_request",
        ] {
            assert_spelling_rejected(expression, "uri");
        }
        for (expressions, expected) in [
            (
                &[
                    "branch",
                    "received_branch",
                    "expected_branch",
                    "tx_id.branch",
                ][..],
                "branch",
            ),
            (&["via", "top_via"][..], "via"),
            (&["route", "top_route", "next_hop"][..], "route"),
            (&["reason", "response.reason_phrase()"][..], "reason"),
            (&["body", "sdp"][..], "body"),
            (&["dialog_id", "id_for_dialog"][..], "dialog_id"),
            (&["command"][..], "command"),
            (&["message"][..], "message"),
            (&["event"][..], "event"),
        ] {
            for expression in expressions {
                assert_spelling_rejected(expression, expected);
            }
        }

        let unsafe_retention = r#"
            increment(&mut client_by_method, key.method().to_string());
            increment(&mut server_by_method, SafeMethod::new(key.method()).to_string());
        "#;
        assert!(scan_retention_source(unsafe_retention).contains(&"retention_method"));
    }

    #[test]
    fn transaction_log_scanner_does_not_treat_bare_id_as_a_transaction_key() {
        let source = r#"debug!(id = %id, "bounded internal identifier");"#;
        assert!(scan_source("transaction/mutant.rs", source).is_empty());
    }
}
