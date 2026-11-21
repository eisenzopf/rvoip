// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-FileCopyrightText: 2023-2024 Luke Curley and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

mod capacity;
mod error;
mod fetch;
mod publish_namespace;
mod publish_received;
mod published;
mod published_namespace;
mod publisher;
mod reader;
mod request_id;
mod request_updates;
mod subscribe;
mod subscribed;
mod subscribed_namespace;
mod subscriber;
mod target;
mod track_status_requested;
mod writer;

pub use capacity::*;
pub use error::*;
pub use fetch::*;
pub use publish_namespace::*;
pub use publish_received::*;
pub use published::*;
pub use published_namespace::*;
pub use publisher::*;
pub use request_id::RequestId;
pub use subscribe::*;
pub use subscribed::*;
pub use subscribed_namespace::*;
pub use subscriber::*;
pub use target::*;
pub use track_status_requested::*;

use reader::*;
use request_updates::{RequestKind, RequestUpdateCredits};
use writer::*;

use bytes::Bytes;
use futures::{stream::FuturesUnordered, StreamExt};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use crate::coding::{AuthorizationToken, Encode, KeyValuePairs, Value};
use crate::message::Message;
use crate::mlog;
use crate::watch::Queue;
use crate::{message, setup};
use std::path::PathBuf;

/// Registry mapping bidi-request IDs to a channel for forwarding responses.
/// When `run_send` pops a response message from the outgoing queue, it checks
/// this map: if the response's target request ID has a registered sender, the
/// response is forwarded to the bidi handler task that owns the Writer.
pub(super) enum BidiCommand {
    Send(Message),
    Cancel(u32),
    RequestUpdate {
        update: message::RequestUpdate,
        forward: bool,
        completion: tokio::sync::oneshot::Sender<Result<(), SessionError>>,
    },
}

struct PendingReverseUpdate {
    id: u64,
    forward: bool,
    completion: tokio::sync::oneshot::Sender<Result<(), SessionError>>,
}

/// Ensures every retained representation of a peer-opened logical request is
/// removed on normal completion, rejection, reset, decode error, task abort,
/// or panic unwind.
struct InboundRequestGuard {
    kind: RequestKind,
    id: u64,
    publisher: Option<Publisher>,
    subscriber: Option<Subscriber>,
    responses: BidiResponseMap,
    response_ids: Arc<Mutex<std::collections::HashSet<u64>>>,
    request_lease: Arc<RequestLease>,
}

impl Drop for InboundRequestGuard {
    fn drop(&mut self) {
        let response_ids = self
            .response_ids
            .lock()
            .map(|ids| ids.iter().copied().collect::<Vec<_>>())
            .unwrap_or_else(|_| vec![self.id]);
        if let Ok(mut responses) = self.responses.lock() {
            for response_id in response_ids {
                responses.remove(&response_id);
            }
        }
        match self.kind {
            RequestKind::Subscribe => {
                if let Some(publisher) = self.publisher.as_mut() {
                    publisher.cleanup_inbound_subscribe(self.id);
                }
            }
            RequestKind::PublishNamespace => {
                if let Some(subscriber) = self.subscriber.as_mut() {
                    subscriber.cleanup_inbound_publish_namespace(self.id);
                }
            }
            RequestKind::Publish => {
                if let Some(subscriber) = self.subscriber.as_mut() {
                    subscriber.cleanup_inbound_publish(self.id);
                }
            }
            RequestKind::TrackStatus => {
                if let Some(publisher) = self.publisher.as_mut() {
                    publisher.cleanup_inbound_track_status(self.id);
                }
            }
            RequestKind::Fetch => {
                if let Some(publisher) = self.publisher.as_mut() {
                    publisher.cleanup_inbound_fetch(self.id);
                }
            }
            RequestKind::SubscribeNamespace => {
                if let Some(publisher) = self.publisher.as_mut() {
                    publisher.cleanup_inbound_subscribe_namespace(self.id);
                }
            }
            RequestKind::SubscribeTracks => {}
        }
        self.request_lease.release();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutgoingMessageDestination {
    Control,
    Request(u64),
}

#[derive(Clone, Copy)]
struct RequestUpdateLimits {
    incoming: u64,
    outgoing: u64,
    outgoing_hard: usize,
}

#[derive(Clone)]
struct BidiRequestRuntime {
    request_id: RequestId,
    responses: BidiResponseMap,
    update_limits: RequestUpdateLimits,
    request_capacity: SessionRequestCapacity,
}

struct SessionConfig {
    negotiated: NegotiatedTransport,
    target: SessionTarget,
    setup_authorizations: Vec<SetupAuthorization>,
    peer_max_request_updates: u64,
}

type BidiResponseMap = Arc<Mutex<HashMap<u64, tokio::sync::mpsc::Sender<BidiCommand>>>>;

/// Channel for spawned bidi response reader tasks. Publisher/Subscriber send
/// handles here; `Session::run` collects and polls them.
///
/// A wrapper is used instead of exposing Tokio's sender directly so a task
/// raced against session shutdown is aborted even when the caller ignores the
/// send error. Dropping a bare `JoinHandle` would detach the task.
#[derive(Clone)]
pub(super) struct BidiTaskSender(tokio::sync::mpsc::Sender<tokio::task::JoinHandle<()>>);

struct BidiTaskSendError(Option<tokio::task::JoinHandle<()>>);

impl Drop for BidiTaskSendError {
    fn drop(&mut self) {
        if let Some(task) = &self.0 {
            task.abort();
        }
    }
}

impl BidiTaskSendError {
    async fn abort_and_wait(mut self) {
        let Some(task) = self.0.take() else {
            return;
        };
        task.abort();
        match task.await {
            Err(error) if !error.is_cancelled() => {
                tracing::warn!(%error, "request-stream task failed while joining raced shutdown");
            }
            _ => {}
        }
    }
}

impl BidiTaskSender {
    fn channel(
        capacity: usize,
    ) -> (
        Self,
        tokio::sync::mpsc::Receiver<tokio::task::JoinHandle<()>>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::channel(capacity);
        (Self(tx), rx)
    }

    fn send(&self, task: tokio::task::JoinHandle<()>) -> Result<(), BidiTaskSendError> {
        self.0
            .try_send(task)
            .map_err(|error| BidiTaskSendError(Some(error.into_inner())))
    }
}

/// Process-wide and per-session admission limits for peer-opened data streams.
///
/// QUIC's transport stream limit controls how many streams the peer may have
/// open on the wire, while this separate bound controls how many application
/// futures we retain and poll.
struct DataStreamTaskLimits {
    global: Arc<tokio::sync::Semaphore>,
    per_session: usize,
}

impl DataStreamTaskLimits {
    fn production() -> Self {
        Self {
            global: GLOBAL_DATA_STREAM_TASKS.clone(),
            per_session: Session::MAX_CONCURRENT_DATA_STREAMS_PER_SESSION,
        }
    }

    fn try_admit(&self, active_for_session: usize) -> Option<tokio::sync::OwnedSemaphorePermit> {
        if active_for_session >= self.per_session {
            return None;
        }

        self.global.clone().try_acquire_owned().ok()
    }
}

static GLOBAL_DATA_STREAM_TASKS: LazyLock<Arc<tokio::sync::Semaphore>> = LazyLock::new(|| {
    Arc::new(tokio::sync::Semaphore::new(
        Session::MAX_CONCURRENT_DATA_STREAMS_GLOBAL,
    ))
});

struct BidiRequestTaskLimits {
    global: Arc<tokio::sync::Semaphore>,
    per_session: usize,
}

impl BidiRequestTaskLimits {
    fn production() -> Self {
        Self {
            global: GLOBAL_BIDI_REQUEST_TASKS.clone(),
            per_session: Session::MAX_CONCURRENT_BIDI_STREAMS,
        }
    }

    fn try_admit(&self, active_for_session: usize) -> Option<tokio::sync::OwnedSemaphorePermit> {
        if active_for_session >= self.per_session {
            return None;
        }
        self.global.clone().try_acquire_owned().ok()
    }
}

static GLOBAL_BIDI_REQUEST_TASKS: LazyLock<Arc<tokio::sync::Semaphore>> = LazyLock::new(|| {
    Arc::new(tokio::sync::Semaphore::new(
        Session::MAX_CONCURRENT_BIDI_STREAMS_GLOBAL,
    ))
});

/// The transport protocol negotiated for this MoQT connection.
///
/// MoQT can run over either WebTransport (HTTP/3 + QUIC) or raw QUIC.
/// The transport type affects protocol behavior — for example, the PATH
/// parameter is only sent in SETUP for raw QUIC connections,
/// since WebTransport carries the path in the HTTP/3 CONNECT URL.
///
/// This enum is intentionally extensible for future transport options
/// (e.g., QMUX, WebSocket fallback).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// WebTransport over HTTP/3 (RFC 9220).
    /// ALPN: "h3". Path carried in HTTP/3 CONNECT :path pseudo-header.
    WebTransport,
    /// Raw QUIC with MoQT framing directly on QUIC streams.
    /// ALPN: the negotiated MOQT draft identifier. Path and authority are
    /// carried in SETUP options.
    RawQuic,
}

/// Transport substrate plus the protocol identifier actually negotiated by
/// TLS ALPN or WebTransport's WT-Protocol response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NegotiatedTransport {
    pub substrate: Transport,
    pub protocol: &'static str,
}

impl NegotiatedTransport {
    pub const fn new(substrate: Transport, protocol: &'static str) -> Self {
        Self {
            substrate,
            protocol,
        }
    }
}

/// Bounded authorization material extracted from a peer's structured SETUP
/// token. The serialized alias structure is never passed to admission.
///
/// The contents are available to admission policies but intentionally omitted
/// from `Debug` output so bearer credentials cannot leak into logs.
#[derive(Clone, Eq, PartialEq)]
pub struct SetupAuthorization {
    token_type: u64,
    value: Bytes,
}

impl SetupAuthorization {
    pub const MAX_BYTES: usize = 4 * 1024;

    pub fn new(value: impl AsRef<[u8]>) -> Result<Self, SessionError> {
        Self::new_typed(0, value)
    }

    pub fn new_typed(token_type: u64, value: impl AsRef<[u8]>) -> Result<Self, SessionError> {
        let value = value.as_ref();
        if value.is_empty() {
            return Err(SessionError::ProtocolViolation(
                "SETUP authorization material must not be empty".into(),
            ));
        }
        if value.len() > Self::MAX_BYTES {
            return Err(SessionError::ProtocolViolation(
                "SETUP authorization material exceeds 4096 bytes".into(),
            ));
        }
        Ok(Self {
            token_type,
            value: Bytes::copy_from_slice(value),
        })
    }

    fn from_parsed(token_type: u64, value: &[u8]) -> Result<Self, SessionError> {
        if value.len() > Self::MAX_BYTES {
            return Err(SessionError::KeyValueFormatting(
                "authorization token value exceeds 4096 bytes".into(),
            ));
        }
        Ok(Self {
            token_type,
            value: Bytes::copy_from_slice(value),
        })
    }

    fn encode_wire_value(&self) -> Result<Vec<u8>, SessionError> {
        Ok(AuthorizationToken::UseValue {
            token_type: self.token_type,
            value: self.value.clone(),
        }
        .encode_bytes()?)
    }

    pub const fn token_type(&self) -> u64 {
        self.token_type
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.value
    }

    pub fn len(&self) -> usize {
        self.value.len()
    }

    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }
}

impl std::fmt::Debug for SetupAuthorization {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SetupAuthorization")
            .field("token_type", &self.token_type)
            .field("bytes", &format_args!("<redacted:{}>", self.len()))
            .finish()
    }
}

/// Session object for managing all communications in a single QUIC connection.
#[must_use = "run() must be called"]
pub struct Session {
    webtransport: web_transport::Session,

    /// Control Stream Reader and Writer (QUIC bi-directional stream)
    sender: Writer, // Control Stream Sender
    recver: Reader, // Control Stream Receiver

    publisher: Option<Publisher>, // Contains Publisher side logic, uses outgoing message queue to send control messages
    subscriber: Option<Subscriber>, // Contains Subscriber side logic, uses outgoing message queue to send control messages

    /// Queue used by Publisher and Subscriber for sending Control Messages
    outgoing: Queue<Message>,

    /// Session-level request ID manager.
    /// Publisher and Subscriber share one outbound request ID sequence.
    request_id: RequestId,

    /// Optional mlog writer for MoQ Transport events
    /// Wrapped in Arc<Mutex<>> to share across send/recv tasks when enabled
    mlog: Option<Arc<Mutex<mlog::MlogWriter>>>,

    /// Transport substrate and actual protocol selected for this connection.
    negotiated: NegotiatedTransport,

    /// Canonical `moqt://` target reconstructed identically on both substrates.
    target: SessionTarget,

    /// Bounded authorization material received from this session's peer.
    setup_authorizations: Vec<SetupAuthorization>,

    /// Receiver for spawned bidi reader task handles.
    /// Polled by Session::run; dropping FuturesUnordered aborts all tasks.
    bidi_task_rx: tokio::sync::mpsc::Receiver<tokio::task::JoinHandle<()>>,

    /// Maps bidi-request IDs to their response stream writers (draft-19).
    bidi_response_map: BidiResponseMap,

    /// Per-request-stream update concurrency advertised to the peer.
    max_request_updates: u64,

    /// Maximum concurrent reverse-direction REQUEST_UPDATEs advertised by the peer.
    peer_max_request_updates: u64,

    /// Logical request ownership shared by both transport roles.
    request_capacity: SessionRequestCapacity,
}

static GLOBAL_REQUEST_CAPACITY: LazyLock<RequestCapacity> = LazyLock::new(RequestCapacity::default);

impl Session {
    const DEFAULT_MAX_REQUEST_UPDATES: u64 = 16;
    pub(super) const REQUEST_STREAM_CANCELLED: u32 = 0x1;

    /// Application-level bounds for peer-opened unidirectional data stream
    /// handlers. These are deliberately independent from the negotiated QUIC
    /// stream count so peer behavior cannot create an unbounded task set.
    const MAX_CONCURRENT_DATA_STREAMS_PER_SESSION: usize = 256;
    const MAX_CONCURRENT_DATA_STREAMS_GLOBAL: usize = 4096;

    /// Draft-19 stream reset code used when data-stream admission is exhausted.
    const DATA_STREAM_EXCESSIVE_LOAD: u32 = 0x9;

    /// Validate a draft-19 path-abempty plus optional query while retaining
    /// its exact encoded identity. This compatibility helper now follows RFC
    /// 3986 and therefore does not reject or decode percent-encoded octets.
    pub fn normalize_connection_path(raw: &str) -> Result<Option<String>, SessionError> {
        SessionTarget::from_setup_parts("path-validation.invalid", raw)
            .map(|target| target.routing_path().map(str::to_string))
            .map_err(Self::map_target_error)
    }

    fn map_target_error(error: SessionTargetError) -> SessionError {
        match error {
            SessionTargetError::MissingAuthority | SessionTargetError::MalformedAuthority => {
                SessionError::MalformedAuthority(error.to_string())
            }
            SessionTargetError::MalformedPath(_) | SessionTargetError::TooLong => {
                SessionError::MalformedPath(error.to_string())
            }
            SessionTargetError::UnsupportedScheme(_) | SessionTargetError::Malformed(_) => {
                SessionError::ProtocolViolation(error.to_string())
            }
        }
    }

    fn setup_bytes(
        params: &KeyValuePairs,
        option: setup::ParameterType,
    ) -> Result<Option<&[u8]>, bool> {
        params
            .get(option.into())
            .map(|pair| match &pair.value {
                Value::BytesValue(bytes) => Ok(bytes.as_slice()),
                Value::IntValue(_) => Err(false),
            })
            .transpose()
    }

    fn setup_authorizations(
        params: &KeyValuePairs,
    ) -> Result<Vec<SetupAuthorization>, SessionError> {
        let key = setup::ParameterType::AuthorizationToken.into();
        let authorizations: Vec<_> = params
            .get_all(key)
            .map(|pair| {
                let Value::BytesValue(value) = &pair.value else {
                    return Err(SessionError::KeyValueFormatting(
                        "SETUP authorization option must be bytes-encoded".into(),
                    ));
                };
                match AuthorizationToken::decode_bytes(value)? {
                    AuthorizationToken::UseValue { token_type, value }
                    | AuthorizationToken::Register {
                        token_type, value, ..
                    } => {
                        // This implementation advertises the default zero-sized
                        // authorization cache. Draft-19 requires REGISTER to be
                        // treated as USE_VALUE when the alias cannot be cached.
                        SetupAuthorization::from_parsed(token_type, &value)
                    }
                    AuthorizationToken::Delete { .. } | AuthorizationToken::UseAlias { .. } => {
                        Err(SessionError::ProtocolViolation(
                            "SETUP cannot use DELETE or USE_ALIAS authorization tokens".into(),
                        ))
                    }
                }
            })
            .collect::<Result<_, _>>()?;
        let mut resolved = std::collections::HashSet::new();
        for authorization in &authorizations {
            if !resolved.insert((authorization.token_type, authorization.value.clone())) {
                return Err(SessionError::ProtocolViolation(
                    "SETUP authorization type/value combinations must be unique".into(),
                ));
            }
        }
        Ok(authorizations)
    }

    fn target_from_client_setup(
        session_url: &url::Url,
        negotiated: NegotiatedTransport,
        params: &KeyValuePairs,
    ) -> Result<SessionTarget, SessionError> {
        let path = Self::setup_bytes(params, setup::ParameterType::Path)
            .map_err(|_| SessionError::MalformedPath("PATH option must be bytes-encoded".into()))?;
        let authority =
            Self::setup_bytes(params, setup::ParameterType::Authority).map_err(|_| {
                SessionError::MalformedAuthority("AUTHORITY option must be bytes-encoded".into())
            })?;

        match negotiated.substrate {
            Transport::WebTransport => {
                if path.is_some() {
                    return Err(SessionError::InvalidPath(
                        "PATH is prohibited on WebTransport".into(),
                    ));
                }
                if authority.is_some() {
                    return Err(SessionError::InvalidAuthority(
                        "AUTHORITY is prohibited on WebTransport".into(),
                    ));
                }
                SessionTarget::from_webtransport_url(session_url).map_err(Self::map_target_error)
            }
            Transport::RawQuic => {
                let authority = authority.ok_or_else(|| {
                    SessionError::MalformedAuthority(
                        "native QUIC clients must send AUTHORITY".into(),
                    )
                })?;
                let authority = std::str::from_utf8(authority).map_err(|_| {
                    SessionError::MalformedAuthority("AUTHORITY must be UTF-8".into())
                })?;
                let path = path.ok_or_else(|| {
                    SessionError::MalformedPath("native QUIC clients must send PATH".into())
                })?;
                let path = std::str::from_utf8(path)
                    .map_err(|_| SessionError::MalformedPath("PATH must be UTF-8".into()))?;
                let target = SessionTarget::from_setup_parts(authority, path)
                    .map_err(Self::map_target_error)?;
                if !target.has_same_host(session_url) {
                    return Err(SessionError::InvalidAuthority(
                        "AUTHORITY host does not match the accepted TLS server name".into(),
                    ));
                }
                Ok(target)
            }
        }
    }

    fn validate_server_setup_options(params: &KeyValuePairs) -> Result<(), SessionError> {
        if params.get(setup::ParameterType::Path.into()).is_some() {
            return Err(SessionError::InvalidPath(
                "servers must not send PATH".into(),
            ));
        }
        if params.get(setup::ParameterType::Authority.into()).is_some() {
            return Err(SessionError::InvalidAuthority(
                "servers must not send AUTHORITY".into(),
            ));
        }
        Ok(())
    }

    fn validate_negotiated_transport(
        session: &web_transport::Session,
        negotiated: NegotiatedTransport,
    ) -> Result<(), SessionError> {
        if !setup::SUPPORTED_ALPNS.contains(&negotiated.protocol) {
            return Err(SessionError::ProtocolViolation(format!(
                "unsupported negotiated MOQT protocol {}",
                negotiated.protocol
            )));
        }
        if negotiated.substrate == Transport::WebTransport
            && session.protocol() != Some(negotiated.protocol)
        {
            return Err(SessionError::ProtocolViolation(format!(
                "WebTransport selected protocol {:?}, expected {}",
                session.protocol(),
                negotiated.protocol
            )));
        }
        Ok(())
    }

    /// Returns the negotiated transport protocol for this connection.
    pub fn transport(&self) -> Transport {
        self.negotiated.substrate
    }

    /// Returns the actual substrate and MOQT protocol selected by the peer.
    pub fn negotiated_transport(&self) -> NegotiatedTransport {
        self.negotiated
    }

    /// Returns the canonical `moqt://` session target.
    pub fn target(&self) -> &SessionTarget {
        &self.target
    }

    /// Returns redaction-safe, bounded SETUP authorization material from the peer.
    pub fn peer_setup_authorization(&self) -> Option<&SetupAuthorization> {
        match self.setup_authorizations.as_slice() {
            [authorization] => Some(authorization),
            _ => None,
        }
    }

    /// Returns every parsed SETUP authorization in peer wire order.
    pub fn peer_setup_authorizations(&self) -> &[SetupAuthorization] {
        &self.setup_authorizations
    }

    pub fn peer_setup_authorization_count(&self) -> usize {
        self.setup_authorizations.len()
    }

    /// Remove raw bearer material once admission has produced bounded claims.
    pub fn clear_peer_setup_authorization(&mut self) {
        self.setup_authorizations.clear();
    }

    /// Returns the canonical path and query used for routing this session.
    /// WebTransport derives it from the CONNECT URL; raw QUIC reconstructs the
    /// same value from the required PATH option. Root-only targets return
    /// `None` for compatibility with existing scope routing.
    pub fn connection_path(&self) -> Option<&str> {
        self.target.routing_path()
    }

    /// Log a control- or request-stream message with structured fields.
    /// Uses target "moq_transport::control" so it can be filtered independently.
    fn log_message(msg: &Message, direction: &str) {
        match msg {
            Message::Subscribe(m) => {
                tracing::debug!(
                    target: "moq_transport::control",
                    direction,
                    msg_type = "SUBSCRIBE",
                    subscribe_id = m.id,
                    namespace = %m.track_namespace,
                    track_name = %m.track_name,
                    "MoQT framed message"
                );
            }
            Message::SubscribeOk(m) => {
                tracing::debug!(
                    target: "moq_transport::control",
                    direction,
                    msg_type = "SUBSCRIBE_OK",
                    subscribe_id = m.id,
                    track_alias = m.track_alias,
                    "MoQT framed message"
                );
            }
            Message::PublishNamespace(m) => {
                tracing::debug!(
                    target: "moq_transport::control",
                    direction,
                    msg_type = "PUBLISH_NAMESPACE",
                    request_id = m.id,
                    namespace = %m.track_namespace,
                    "MoQT framed message"
                );
            }
            Message::Namespace(m) => {
                tracing::debug!(
                    target: "moq_transport::control",
                    direction,
                    msg_type = "NAMESPACE",
                    namespace_suffix = %m.track_namespace_suffix,
                    "MoQT framed message"
                );
            }
            Message::NamespaceDone(m) => {
                tracing::debug!(
                    target: "moq_transport::control",
                    direction,
                    msg_type = "NAMESPACE_DONE",
                    namespace_suffix = %m.track_namespace_suffix,
                    "MoQT framed message"
                );
            }
            Message::TrackStatus(m) => {
                tracing::debug!(
                    target: "moq_transport::control",
                    direction,
                    msg_type = "TRACK_STATUS",
                    request_id = m.id,
                    namespace = %m.track_namespace,
                    track_name = %m.track_name,
                    "MoQT framed message"
                );
            }
            Message::SubscribeNamespace(m) => {
                tracing::debug!(
                    target: "moq_transport::control",
                    direction,
                    msg_type = "SUBSCRIBE_NAMESPACE",
                    request_id = m.id,
                    namespace_prefix = %m.track_namespace_prefix,
                    "MoQT framed message"
                );
            }
            Message::SubscribeTracks(m) => {
                tracing::debug!(
                    target: "moq_transport::control",
                    direction,
                    msg_type = "SUBSCRIBE_TRACKS",
                    request_id = m.id,
                    namespace_prefix = %m.track_namespace_prefix,
                    "MoQT framed message"
                );
            }
            Message::Fetch(m) => {
                tracing::debug!(
                    target: "moq_transport::control",
                    direction,
                    msg_type = "FETCH",
                    request_id = m.id,
                    fetch_type = ?m.fetch_type,
                    "MoQT framed message"
                );
            }
            Message::FetchOk(m) => {
                tracing::debug!(
                    target: "moq_transport::control",
                    direction,
                    msg_type = "FETCH_OK",
                    request_id = m.id,
                    end_of_track = m.end_of_track,
                    "MoQT framed message"
                );
            }
            Message::Publish(m) => {
                tracing::debug!(
                    target: "moq_transport::control",
                    direction,
                    msg_type = "PUBLISH",
                    request_id = m.id,
                    namespace = %m.track_namespace,
                    track_name = %m.track_name,
                    track_alias = m.track_alias,
                    "MoQT framed message"
                );
            }
            Message::PublishSkipped(m) => {
                tracing::debug!(
                    target: "moq_transport::control",
                    direction,
                    msg_type = "PUBLISH_SKIPPED",
                    namespace_suffix = %m.track_namespace_suffix,
                    track_name = %m.track_name,
                    "MoQT framed message"
                );
            }
            Message::PublishDone(m) => {
                tracing::debug!(
                    target: "moq_transport::control",
                    direction,
                    msg_type = "PUBLISH_DONE",
                    request_id = m.id,
                    status_code = m.status_code,
                    stream_count = m.stream_count,
                    "MoQT framed message"
                );
            }
            Message::GoAway(m) => {
                tracing::debug!(
                    target: "moq_transport::control",
                    direction,
                    msg_type = "GOAWAY",
                    uri = %m.uri.0,
                    timeout_ms = m.timeout,
                    "MoQT framed message"
                );
            }
            Message::RequestOk(m) => {
                tracing::debug!(
                    target: "moq_transport::control",
                    direction,
                    msg_type = "REQUEST_OK",
                    request_id = m.id,
                    "MoQT framed message"
                );
            }
            Message::RequestError(m) => {
                tracing::debug!(
                    target: "moq_transport::control",
                    direction,
                    msg_type = "REQUEST_ERROR",
                    request_id = m.id,
                    error_code = m.error_code,
                    retry_interval = m.retry_interval,
                    "MoQT framed message"
                );
            }
            Message::RequestUpdate(m) => {
                tracing::debug!(
                    target: "moq_transport::control",
                    direction,
                    msg_type = "REQUEST_UPDATE",
                    request_id = m.id,
                    "MoQT framed message"
                );
            }
        }
    }

    fn new(
        webtransport: web_transport::Session,
        sender: Writer,
        recver: Reader,
        mlog: Option<mlog::MlogWriter>,
        request_id: RequestId,
        config: SessionConfig,
        request_capacity: SessionRequestCapacity,
    ) -> (Self, Option<Publisher>, Option<Subscriber>) {
        let limits = request_capacity.limits();
        let outgoing = Queue::bounded(limits.max_outbound_messages).split();

        // Wrap mlog in Arc<Mutex<>> for sharing across tasks
        let mlog_shared = mlog.map(|m| Arc::new(Mutex::new(m)));

        let (bidi_task_tx, bidi_task_rx) = BidiTaskSender::channel(limits.max_outbound_tasks);
        let bidi_response_map = Arc::new(Mutex::new(HashMap::new()));

        let publisher = Some(Publisher::new(
            outgoing.0.clone(),
            webtransport.clone(),
            mlog_shared.clone(),
            request_id.clone(),
            bidi_task_tx.clone(),
            bidi_response_map.clone(),
            request_capacity.clone(),
        ));
        let subscriber = Some(Subscriber::new(
            outgoing.0,
            webtransport.clone(),
            mlog_shared.clone(),
            request_id.clone(),
            bidi_task_tx,
            bidi_response_map.clone(),
            request_capacity.clone(),
        ));

        let session = Self {
            webtransport,
            sender,
            recver,
            publisher: publisher.clone(),
            subscriber: subscriber.clone(),
            outgoing: outgoing.1,
            request_id,
            mlog: mlog_shared,
            negotiated: config.negotiated,
            target: config.target,
            setup_authorizations: config.setup_authorizations,
            bidi_task_rx,
            bidi_response_map,
            max_request_updates: Self::DEFAULT_MAX_REQUEST_UPDATES,
            peer_max_request_updates: config.peer_max_request_updates,
            request_capacity,
        };

        (session, publisher, subscriber)
    }

    /// Create an outbound/client QUIC connection.
    ///
    /// Opens a unidirectional control stream, sends SETUP with
    /// parameters only (version is agreed via ALPN), and waits for SETUP.
    ///
    /// For native `moqt://` connections the PATH and AUTHORITY parameters are
    /// sent automatically.  For WebTransport the path is carried in the HTTP/3
    /// CONNECT URL so PATH is not sent.
    pub async fn connect(
        session: web_transport::Session,
        mlog_path: Option<PathBuf>,
        negotiated: NegotiatedTransport,
    ) -> Result<(Session, Publisher, Subscriber), SessionError> {
        Self::connect_with_authorization(session, mlog_path, negotiated, None).await
    }

    /// Create an outbound session using caller-owned process and session
    /// limits. Reuse one [`RequestCapacity`] for every process connection so
    /// request and retained-byte limits are enforced and observable globally.
    pub async fn connect_with_capacity(
        session: web_transport::Session,
        mlog_path: Option<PathBuf>,
        negotiated: NegotiatedTransport,
        request_capacity: &RequestCapacity,
    ) -> Result<(Session, Publisher, Subscriber), SessionError> {
        Self::connect_with_authorization_and_capacity(
            session,
            mlog_path,
            negotiated,
            None,
            request_capacity,
        )
        .await
    }

    /// Create an outbound session and include bounded authorization material
    /// in the SETUP option block.
    pub async fn connect_with_authorization(
        session: web_transport::Session,
        mlog_path: Option<PathBuf>,
        negotiated: NegotiatedTransport,
        authorization: Option<SetupAuthorization>,
    ) -> Result<(Session, Publisher, Subscriber), SessionError> {
        Self::connect_with_authorization_and_capacity(
            session,
            mlog_path,
            negotiated,
            authorization,
            &GLOBAL_REQUEST_CAPACITY,
        )
        .await
    }

    /// Create an authorized outbound session using caller-owned limits.
    pub async fn connect_with_authorization_and_capacity(
        session: web_transport::Session,
        mlog_path: Option<PathBuf>,
        negotiated: NegotiatedTransport,
        authorization: Option<SetupAuthorization>,
        request_capacity: &RequestCapacity,
    ) -> Result<(Session, Publisher, Subscriber), SessionError> {
        Self::validate_negotiated_transport(&session, negotiated)?;
        let url = session.url().clone();
        let target = match negotiated.substrate {
            Transport::RawQuic => {
                SessionTarget::try_from_url(url).map_err(Self::map_target_error)?
            }
            Transport::WebTransport => {
                SessionTarget::from_webtransport_url(&url).map_err(Self::map_target_error)?
            }
        };

        let mlog = mlog_path.and_then(|p| {
            mlog::MlogWriter::new(p)
                .map_err(|e| tracing::warn!("Failed to create mlog: {}", e))
                .ok()
        });

        // Open our unidirectional control send stream.
        let send_stream = session.open_uni().await?;
        let mut sender = Writer::new(send_stream);

        let mut params = KeyValuePairs::default();
        params.set_intvalue(
            setup::ParameterType::MaxRequestUpdates.into(),
            Self::DEFAULT_MAX_REQUEST_UPDATES,
        );
        if let Some(authorization) = authorization {
            params.set_bytesvalue(
                setup::ParameterType::AuthorizationToken.into(),
                authorization.encode_wire_value()?,
            );
        }

        if negotiated.substrate == Transport::RawQuic {
            // Draft-19 requires both options on every native QUIC session,
            // including an empty PATH value for a URI with path-abempty="".
            params.set_bytesvalue(
                setup::ParameterType::Authority.into(),
                target.authority().as_bytes().to_vec(),
            );
            params.set_bytesvalue(
                setup::ParameterType::Path.into(),
                target.path_and_query().as_bytes().to_vec(),
            );
        }

        let client = setup::Setup { params };

        tracing::debug!(
            target: "moq_transport::control",
            direction = "sent",
            msg_type = "SETUP",
            transport = ?negotiated.substrate,
            protocol = negotiated.protocol,
            target = %target.redacted_for_logging(),
            "MoQT framed message"
        );
        sender.encode(&client).await?;

        // Accept the peer's unidirectional control stream.
        let recv_stream = session.accept_uni().await?;
        let mut recver = Reader::new(recv_stream);
        let server: setup::Setup = recver.decode().await?;
        Self::validate_server_setup_options(&server.params)?;
        let setup_authorizations = Self::setup_authorizations(&server.params)?;
        let peer_max_request_updates = server.max_request_updates()?;
        tracing::debug!(
            target: "moq_transport::control",
            direction = "recv",
            msg_type = "SETUP (recv)",
            "MoQT framed message"
        );

        // Client sends even IDs (0); peer server sends odd IDs (1).
        let request_id = RequestId::new(0, 1);
        let session = Session::new(
            session,
            sender,
            recver,
            mlog,
            request_id,
            SessionConfig {
                negotiated,
                target,
                setup_authorizations,
                peer_max_request_updates,
            },
            request_capacity.session(),
        );
        Ok((session.0, session.1.unwrap(), session.2.unwrap()))
    }

    /// Accept an inbound server connection.
    ///
    /// Opens a unidirectional control stream and accepts the peer.s, decodes SETUP,
    /// sends SETUP with parameters only.  Version is already agreed
    /// via ALPN before this is called.
    pub async fn accept(
        session: web_transport::Session,
        mlog_path: Option<PathBuf>,
        negotiated: NegotiatedTransport,
    ) -> Result<(Session, Option<Publisher>, Option<Subscriber>), SessionError> {
        Self::accept_with_capacity(session, mlog_path, negotiated, &GLOBAL_REQUEST_CAPACITY).await
    }

    /// Accept an inbound session using caller-owned process and session
    /// limits. Reuse one [`RequestCapacity`] for every accepted connection.
    pub async fn accept_with_capacity(
        session: web_transport::Session,
        mlog_path: Option<PathBuf>,
        negotiated: NegotiatedTransport,
        request_capacity: &RequestCapacity,
    ) -> Result<(Session, Option<Publisher>, Option<Subscriber>), SessionError> {
        Self::validate_negotiated_transport(&session, negotiated)?;
        let mut mlog = mlog_path.and_then(|p| {
            mlog::MlogWriter::new(p)
                .map_err(|e| tracing::warn!("Failed to create mlog: {}", e))
                .ok()
        });

        // Open our unidirectional control send stream.
        let send_stream = session.open_uni().await?;
        let mut sender = Writer::new(send_stream);

        // Accept the peer's unidirectional control stream.
        let recv_stream = session.accept_uni().await?;
        let mut recver = Reader::new(recv_stream);

        let client: setup::Setup = recver.decode().await?;
        let peer_max_request_updates = client.max_request_updates()?;
        tracing::debug!(
            target: "moq_transport::control",
            direction = "recv",
            msg_type = "SETUP",
            "MoQT framed message"
        );

        let target = Self::target_from_client_setup(session.url(), negotiated, &client.params)?;
        let setup_authorizations = Self::setup_authorizations(&client.params)?;

        if target.routing_path().is_some() {
            tracing::debug!(
                connection_target = %target.redacted_for_logging(),
                query_present = target.query().is_some(),
                "Connection path resolved"
            );
        }

        if let Some(ref mut mlog) = mlog {
            let event = mlog::events::client_setup_parsed(mlog.elapsed_ms(), 0, &client);
            let _ = mlog.add_event(event);
        }

        let mut params = KeyValuePairs::default();
        params.set_intvalue(
            setup::ParameterType::MaxRequestUpdates.into(),
            Self::DEFAULT_MAX_REQUEST_UPDATES,
        );

        let server = setup::Setup { params };

        tracing::debug!(
            target: "moq_transport::control",
            direction = "sent",
            msg_type = "SETUP (recv)",
            "MoQT framed message"
        );

        if let Some(ref mut mlog) = mlog {
            let event = mlog::events::server_setup_created(mlog.elapsed_ms(), 0, &server);
            let _ = mlog.add_event(event);
        }

        sender.encode(&server).await?;

        // Server sends odd IDs (1); peer client sends even IDs (0).
        let request_id = RequestId::new(1, 0);
        Ok(Session::new(
            session,
            sender,
            recver,
            mlog,
            request_id,
            SessionConfig {
                negotiated,
                target,
                setup_authorizations,
                peer_max_request_updates,
            },
            request_capacity.session(),
        ))
    }

    /// Return session and process retained-byte gauges for diagnostics.
    pub fn retention_stats(&self) -> crate::serve::RetentionBudgetStats {
        self.request_capacity.retention_stats()
    }

    /// Return the effective limits used by this session.
    pub fn request_limits(&self) -> &RequestLimits {
        self.request_capacity.limits()
    }

    /// Run Tasks for the session, including sending of control messages, receiving and processing
    /// inbound control messages, receiving and processing new inbound uni-directional QUIC streams,
    /// and receiving and processing QUIC datagrams received
    pub async fn run(self) -> Result<(), SessionError> {
        let mut bidi_task_rx = self.bidi_task_rx;
        let mut reader_tasks = FuturesUnordered::new();

        let result = tokio::select! {
            res = Self::run_recv(self.recver, self.mlog.clone()) => res,
            res = Self::run_send(self.sender, self.outgoing, self.mlog.clone(), self.bidi_response_map.clone()) => res,
            res = Self::run_bidi_requests(
                self.webtransport.clone(),
                self.publisher.clone(),
                self.subscriber.clone(),
                BidiRequestRuntime {
                    request_id: self.request_id.clone(),
                    responses: self.bidi_response_map.clone(),
                    update_limits: RequestUpdateLimits {
                        incoming: self.max_request_updates,
                        outgoing: self.peer_max_request_updates,
                        outgoing_hard: self.request_capacity.limits().max_reverse_updates,
                    },
                    request_capacity: self.request_capacity.clone(),
                },
            ) => res,
            res = Self::run_streams(self.webtransport.clone(), self.subscriber.clone()) => res,
            res = Self::run_datagrams(self.webtransport, self.subscriber) => res,
            // Collect bidi reader task handles and poll them to completion.
            () = async {
                loop {
                    tokio::select! {
                        handle = bidi_task_rx.recv() => {
                            match handle {
                                Some(h) => reader_tasks.push(h),
                                None => break, // all senders dropped
                            }
                        }
                        Some(_) = reader_tasks.next() => {}
                    }
                }
            } => Ok(()),
        };

        Self::shutdown_bidi_tasks(&mut bidi_task_rx, &mut reader_tasks).await;

        result
    }

    /// Stop and join every request-stream task owned by the session.
    ///
    /// Closing the receiver first creates a linearization point with racing
    /// senders: a send either completed before the close and is drained below,
    /// or it fails and `BidiTaskSendError` aborts the returned handle. Every
    /// handle accepted by this collector is explicitly awaited after abort so
    /// no request-stream task can outlive `Session::run`.
    async fn shutdown_bidi_tasks(
        bidi_task_rx: &mut tokio::sync::mpsc::Receiver<tokio::task::JoinHandle<()>>,
        reader_tasks: &mut FuturesUnordered<tokio::task::JoinHandle<()>>,
    ) {
        bidi_task_rx.close();
        while let Some(task) = bidi_task_rx.recv().await {
            reader_tasks.push(task);
        }

        for task in reader_tasks.iter() {
            task.abort();
        }

        while let Some(result) = reader_tasks.next().await {
            if let Err(error) = result {
                if !error.is_cancelled() {
                    tracing::warn!(%error, "request-stream task failed during session shutdown");
                }
            }
        }
    }

    fn outgoing_message_destination(
        msg: &Message,
    ) -> Result<OutgoingMessageDestination, SessionError> {
        if let Some(request_id) = msg.response_target_id() {
            return Ok(OutgoingMessageDestination::Request(request_id));
        }

        if msg.placement().allows_control() {
            return Ok(OutgoingMessageDestination::Control);
        }

        tracing::error!(
            msg_type = msg.name(),
            "request-only message was enqueued without request-stream context"
        );
        Err(SessionError::Internal)
    }

    fn validate_control_message(msg: &Message) -> Result<(), SessionError> {
        if msg.placement().allows_control() {
            return Ok(());
        }

        Err(SessionError::ProtocolViolation(format!(
            "{} is not permitted on the control stream",
            msg.name()
        )))
    }

    /// Processes the shared outgoing queue. ID-bearing responses are routed to
    /// their owning bidirectional request stream. `GOAWAY` is the only message
    /// that may reach the post-SETUP control stream in draft-19.
    async fn run_send(
        mut sender: Writer,
        mut outgoing: Queue<message::Message>,
        mlog: Option<Arc<Mutex<mlog::MlogWriter>>>,
        bidi_response_map: BidiResponseMap,
    ) -> Result<(), SessionError> {
        while let Some(msg) = outgoing.pop().await {
            match Self::outgoing_message_destination(&msg)? {
                OutgoingMessageDestination::Request(target_id) => {
                    Self::log_message(&msg, "sent on request stream");
                    if let Some(ref mlog) = mlog {
                        if let Ok(mut mlog_guard) = mlog.lock() {
                            let time = mlog_guard.elapsed_ms();
                            let event = match &msg {
                                Message::SubscribeOk(m) => {
                                    Some(mlog::events::subscribe_ok_created(time, m.id, m))
                                }
                                _ => None,
                            };
                            if let Some(event) = event {
                                let _ = mlog_guard.add_event(event);
                            }
                        }
                    }

                    let tx_opt = bidi_response_map
                        .lock()
                        .map_err(|_| SessionError::Internal)?
                        .get(&target_id)
                        .cloned();
                    if let Some(tx) = tx_opt {
                        if tx.try_send(BidiCommand::Send(msg)).is_err() {
                            tracing::warn!(
                                target_id,
                                "bidi response channel closed, dropping message"
                            );
                        }
                    } else {
                        tracing::warn!(
                            target_id,
                            "bidi response map entry gone, dropping late response"
                        );
                    }
                }
                OutgoingMessageDestination::Control => {
                    Self::log_message(&msg, "sent on control stream");
                    if let (Some(mlog), Message::GoAway(goaway)) = (&mlog, &msg) {
                        if let Ok(mut mlog) = mlog.lock() {
                            let event = mlog::events::go_away_created(mlog.elapsed_ms(), 0, goaway);
                            let _ = mlog.add_event(event);
                        }
                    }
                    sender.encode(&msg).await?;
                }
            }
        }

        Ok(())
    }

    /// Accept incoming bidirectional request streams (draft-19 §10).
    /// Each peer-initiated bidi stream carries one request message followed
    /// by responses/follow-ups on the same stream.
    /// Maximum number of bidi request handler tasks running concurrently.
    /// Provides back-pressure when a peer opens many streams at once.
    const MAX_CONCURRENT_BIDI_STREAMS: usize = 128;
    const MAX_CONCURRENT_BIDI_STREAMS_GLOBAL: usize = 4096;

    async fn run_bidi_requests(
        webtransport: web_transport::Session,
        publisher: Option<Publisher>,
        subscriber: Option<Subscriber>,
        runtime: BidiRequestRuntime,
    ) -> Result<(), SessionError> {
        let mut tasks = FuturesUnordered::new();
        let limits = BidiRequestTaskLimits::production();

        loop {
            tokio::select! {
                res = webtransport.accept_bi(), if tasks.len() < Self::MAX_CONCURRENT_BIDI_STREAMS => {
                    let (mut send_stream, mut recv_stream) = res?;
                    let Some(permit) = limits.try_admit(tasks.len()) else {
                        let code = message::RequestErrorCode::ExcessiveLoad as u32;
                        send_stream.reset(code);
                        recv_stream.stop(code);
                        continue;
                    };
                    let mut pub_clone = publisher.clone();
                    let mut sub_clone = subscriber.clone();
                    let runtime = runtime.clone();

                    tasks.push(async move {
                        let _permit = permit;
                        Self::handle_bidi_request(
                            send_stream, recv_stream,
                            &mut pub_clone, &mut sub_clone, &runtime,
                        ).await
                    });
                }
                Some(result) = tasks.next() => match result {
                    Err(error) if error.is_request_stream_cancelled() => {
                        tracing::debug!(%error, "peer cancelled request stream");
                    }
                    other => other?,
                },
            }
        }
    }

    /// Handle a single bidi request stream: decode the request, dispatch
    /// to handlers, then wait for responses and write them back on the
    /// same stream (without Request ID, per draft-19).
    async fn handle_bidi_request(
        send_stream: web_transport::SendStream,
        recv_stream: web_transport::RecvStream,
        publisher: &mut Option<Publisher>,
        subscriber: &mut Option<Subscriber>,
        runtime: &BidiRequestRuntime,
    ) -> Result<(), SessionError> {
        let request_id = &runtime.request_id;
        let bidi_response_map = &runtime.responses;
        let update_limits = runtime.update_limits;
        let request_capacity = &runtime.request_capacity;
        let mut reader = Reader::new(recv_stream);
        let mut writer = Writer::new(send_stream);

        // Read the first (request) message from the bidi stream.
        let msg: Message = reader.decode().await?;
        let request_kind = RequestKind::from_first_message(&msg)?;
        let initial_id = msg.sequenced_request_id().ok_or_else(|| {
            SessionError::ProtocolViolation(
                "first request-stream message did not consume a Request ID".to_string(),
            )
        })?;

        request_id.validate_incoming(initial_id)?;

        let Some(request_class) = request_kind.request_class() else {
            Self::encode_bidi_response(
                &mut writer,
                &Message::RequestError(message::RequestError {
                    id: initial_id,
                    error_code: message::RequestErrorCode::NotSupported as u64,
                    retry_interval: 0,
                    reason: crate::coding::ReasonPhrase("not supported".to_string()),
                    redirect: None,
                }),
            )
            .await?;
            writer.finish();
            reader.stop(Self::REQUEST_STREAM_CANCELLED);
            tokio::task::yield_now().await;
            return Ok(());
        };
        let request_lease = match request_capacity
            .try_acquire(RequestDirection::Inbound, request_class)
        {
            Ok(lease) => Arc::new(lease),
            Err(error) => {
                tracing::warn!(
                    request_id = initial_id,
                    request_kind = ?request_kind,
                    %error,
                    "rejecting request because logical capacity is exhausted"
                );
                Self::encode_bidi_response(&mut writer, &Self::excessive_load_response(initial_id))
                    .await?;
                writer.finish();
                reader.stop(Self::DATA_STREAM_EXCESSIVE_LOAD);
                tokio::task::yield_now().await;
                return Ok(());
            }
        };

        let (response_tx, mut response_rx) = tokio::sync::mpsc::channel::<BidiCommand>(
            request_capacity.limits().max_response_commands,
        );
        bidi_response_map
            .lock()
            .map_err(|_| SessionError::Internal)?
            .insert(initial_id, response_tx.clone());

        let response_ids = Arc::new(Mutex::new(std::collections::HashSet::from([initial_id])));
        let _request_guard = InboundRequestGuard {
            kind: request_kind,
            id: initial_id,
            publisher: publisher.clone(),
            subscriber: subscriber.clone(),
            responses: bidi_response_map.clone(),
            response_ids: response_ids.clone(),
            request_lease: request_lease.clone(),
        };

        // Dispatch to the appropriate role handler (same as run_recv).
        // Capture the result so cleanup runs unconditionally on error.
        let dispatch_result = (|| -> Result<(), SessionError> {
            let msg = match TryInto::<message::Publisher>::try_into(msg) {
                Ok(msg) => {
                    subscriber
                        .as_mut()
                        .ok_or(SessionError::RoleViolation)?
                        .recv_request_message(msg, request_lease.clone())?;
                    return Ok(());
                }
                Err(msg) => msg,
            };
            match TryInto::<message::Subscriber>::try_into(msg) {
                Ok(msg) => {
                    publisher
                        .as_mut()
                        .ok_or(SessionError::RoleViolation)?
                        .recv_request_message(msg, request_lease.clone())?;
                }
                Err(msg) => {
                    tracing::warn!(
                        msg_type = msg.name(),
                        "unexpected message on bidi request stream"
                    );
                }
            }
            Ok(())
        })();
        dispatch_result?;

        let mut requester_open = true;
        let mut update_ids = std::collections::HashSet::new();
        let mut update_credits = RequestUpdateCredits::new(update_limits.incoming);
        let mut reverse_updates: std::collections::VecDeque<PendingReverseUpdate> =
            std::collections::VecDeque::new();

        let result = async {
            loop {
                tokio::select! {
                incoming = Self::decode_requester_followup(
                    &mut reader,
                    initial_id,
                    request_kind,
                    reverse_updates.front().map(|update| update.id),
                ), if requester_open => {
                    match incoming? {
                        Some(Message::RequestUpdate(update)) => {
                            if !request_kind.accepts_request_updates() {
                                break Err(SessionError::ProtocolViolation(format!(
                                    "unexpected REQUEST_UPDATE on {:?} request stream",
                                    request_kind
                                )));
                            }

                            request_id.validate_incoming(update.id)?;
                            update_credits.receive()?;

                            let update_id = update.id;
                            let previous = bidi_response_map
                                .lock()
                                .map_err(|_| SessionError::Internal)?
                                .insert(update_id, response_tx.clone());
                            response_ids
                                .lock()
                                .map_err(|_| SessionError::Internal)?
                                .insert(update_id);
                            if previous.is_some() || !update_ids.insert(update_id) {
                                break Err(SessionError::InvalidRequestId);
                            }

                            let dispatch = if request_kind.is_publisher_message() {
                                subscriber
                                    .as_mut()
                                    .ok_or(SessionError::RoleViolation)?
                                    .recv_request_update(initial_id, update)
                            } else {
                                publisher
                                    .as_mut()
                                    .ok_or(SessionError::RoleViolation)?
                                    .recv_request_update(initial_id, update)
                            };
                            if let Err(error) = dispatch {
                                break Err(error);
                            }
                        }
                        Some(Message::PublishDone(done)) if request_kind == RequestKind::Publish => {
                            let subscriber = subscriber
                                .as_mut()
                                .ok_or(SessionError::RoleViolation)?;
                            subscriber.recv_message(message::Publisher::PublishDone(done))?;
                            subscriber.await_publish_done_cleanup(initial_id).await;
                            break Ok(());
                        }
                        Some(Message::RequestOk(ok)) if request_kind == RequestKind::Publish => {
                            let Some(update) = reverse_updates.pop_front() else {
                                break Err(SessionError::ProtocolViolation(
                                    "PUBLISH requester sent REQUEST_OK without a pending reverse update"
                                        .to_string(),
                                ));
                            };
                            debug_assert_eq!(ok.id, update.id);
                            Self::validate_response_for_request(request_kind, true, &Message::RequestOk(ok))?;
                            let result = subscriber
                                .as_mut()
                                .ok_or(SessionError::RoleViolation)?
                                .set_publish_forward(initial_id, update.forward);
                            let completion_result = result.clone();
                            let _ = update.completion.send(completion_result);
                            result?;
                        }
                        Some(Message::RequestError(error)) if request_kind == RequestKind::Publish => {
                            let Some(update) = reverse_updates.pop_front() else {
                                break Err(SessionError::ProtocolViolation(
                                    "PUBLISH requester sent REQUEST_ERROR without a pending reverse update"
                                        .to_string(),
                                ));
                            };
                            debug_assert_eq!(error.id, update.id);
                            let _ = update.completion.send(Err(SessionError::Serve(
                                crate::serve::ServeError::Closed(error.error_code),
                            )));
                        }
                        Some(other) => {
                            break Err(SessionError::ProtocolViolation(format!(
                                "unexpected {} after first request-stream message",
                                other.name()
                            )));
                        }
                        None => {
                            if request_kind == RequestKind::Publish {
                                break Err(SessionError::ProtocolViolation(
                                    "PUBLISH requester sent FIN while its request remained established"
                                        .to_string(),
                                ));
                            }
                            if request_kind == RequestKind::PublishNamespace {
                                // Draft-19 represents graceful namespace
                                // completion with FIN on the request stream.
                                // Completing this handler closes the response
                                // direction and drops the inbound namespace
                                // registration, which provides an observable
                                // peer-acceptance barrier to the origin.
                                subscriber
                                    .as_mut()
                                    .ok_or(SessionError::RoleViolation)?
                                    .finish_inbound_publish_namespace(initial_id)
                                    .await?;
                                break Ok(());
                            }
                            requester_open = false;
                        }
                    }
                }
                command = response_rx.recv() => {
                    let Some(command) = command else {
                        break Err(SessionError::Internal);
                    };
                    let response = match command {
                        BidiCommand::Cancel(code) => {
                            reader.stop(code);
                            writer.reset(code);
                            break Ok(());
                        }
                        BidiCommand::RequestUpdate { update, forward, completion } => {
                            if request_kind != RequestKind::Publish {
                                let _ = completion.send(Err(SessionError::ProtocolViolation(
                                    "reverse REQUEST_UPDATE is only valid for PUBLISH".to_string(),
                                )));
                                continue;
                            }
                            let reverse_limit =
                                Self::effective_reverse_update_limit(update_limits);
                            if reverse_updates.len() >= reverse_limit {
                                let _ = completion.send(Err(SessionError::TooManyRequestUpdates));
                                continue;
                            }
                            writer.encode(&Message::RequestUpdate(update.clone())).await?;
                            reverse_updates.push_back(PendingReverseUpdate {
                                id: update.id,
                                forward,
                                completion,
                            });
                            continue;
                        }
                        BidiCommand::Send(response) => response,
                    };
                    let response_id = match response.response_target_id() {
                        Some(id) => id,
                        None
                            if request_kind == RequestKind::SubscribeNamespace
                                && matches!(
                                    response,
                                    Message::Namespace(_) | Message::NamespaceDone(_)
                                ) =>
                        {
                            initial_id
                        }
                        None => {
                            break Err(SessionError::ProtocolViolation(format!(
                                "{} is not valid on a {:?} response stream",
                                response.name(), request_kind
                            )));
                        }
                    };
                    let is_update_response = update_ids.remove(&response_id);

                    Self::validate_response_for_request(
                        request_kind,
                        is_update_response,
                        &response,
                    )?;
                    Self::encode_bidi_response(&mut writer, &response).await?;

                    if is_update_response {
                        if let Ok(mut map) = bidi_response_map.lock() {
                            map.remove(&response_id);
                        }
                        if let Ok(mut ids) = response_ids.lock() {
                            ids.remove(&response_id);
                        }
                        update_credits.respond();
                    }

                    if Self::response_is_terminal(request_kind, is_update_response, &response) {
                        break Ok(());
                    }
                }
                }
            }
        }
        .await;

        if let Ok(mut map) = bidi_response_map.lock() {
            map.remove(&initial_id);
            for update_id in update_ids {
                map.remove(&update_id);
            }
        }
        for update in reverse_updates {
            let _ = update
                .completion
                .send(Err(SessionError::Serve(crate::serve::ServeError::Cancel)));
        }

        if request_kind == RequestKind::Publish {
            if let Err(error) = &result {
                if let Some(subscriber) = subscriber.as_mut() {
                    let serve_error = if error.is_request_stream_cancelled() {
                        crate::serve::ServeError::Cancel
                    } else {
                        crate::serve::ServeError::internal_ctx(error.to_string())
                    };
                    subscriber.fail_publish_received(initial_id, serve_error);
                }
            }
        }

        // Explicitly finish the stream and yield for Quinn to flush.
        writer.finish();
        tokio::task::yield_now().await;

        result
    }

    fn excessive_load_response(request_id: u64) -> Message {
        Message::RequestError(message::RequestError {
            id: request_id,
            error_code: message::RequestErrorCode::ExcessiveLoad as u64,
            retry_interval: 1001,
            reason: crate::coding::ReasonPhrase("request capacity exhausted".to_string()),
            redirect: None,
        })
    }

    fn response_is_terminal(
        request_kind: RequestKind,
        is_update_response: bool,
        response: &Message,
    ) -> bool {
        let request_error_is_terminal = matches!(response, Message::RequestError(_))
            && !(is_update_response
                && matches!(
                    request_kind,
                    RequestKind::Subscribe | RequestKind::Publish | RequestKind::Fetch
                ));
        request_error_is_terminal
            || matches!(response, Message::PublishDone(_))
            || (request_kind == RequestKind::Fetch && matches!(response, Message::FetchOk(_)))
            || (request_kind == RequestKind::TrackStatus
                && matches!(response, Message::RequestOk(_)))
    }

    fn effective_reverse_update_limit(limits: RequestUpdateLimits) -> usize {
        if limits.outgoing == 0 {
            return limits.outgoing_hard;
        }
        usize::try_from(limits.outgoing)
            .unwrap_or(usize::MAX)
            .min(limits.outgoing_hard)
    }

    /// Decode a message sent after the first request-stream message.
    ///
    /// `PUBLISH_DONE` omits its Request ID because the request stream already
    /// identifies the publication. Other requester-side follow-ups currently
    /// use their regular encoding (`REQUEST_UPDATE` carries its own new ID).
    async fn decode_requester_followup(
        reader: &mut Reader,
        initial_id: u64,
        request_kind: RequestKind,
        pending_reverse_update: Option<u64>,
    ) -> Result<Option<Message>, SessionError> {
        if request_kind == RequestKind::Publish {
            if reader.done().await? {
                return Ok(None);
            }
            let (msg_type, payload) = Self::read_bidi_frame(reader).await?;
            let response_id =
                Self::publish_followup_request_id(msg_type, initial_id, pending_reverse_update)?;
            return Self::decode_bidi_response_payload(
                msg_type,
                payload,
                response_id,
                request_kind,
            )
            .map(Some);
        }
        reader.decode_optional::<Message>().await
    }

    fn publish_followup_request_id(
        msg_type: u64,
        initial_id: u64,
        pending_reverse_update: Option<u64>,
    ) -> Result<u64, SessionError> {
        match msg_type {
            message::wire_id::PublishDone => Ok(initial_id),
            message::wire_id::RequestOk | message::wire_id::RequestError => pending_reverse_update
                .ok_or_else(|| {
                    SessionError::ProtocolViolation(
                        "PUBLISH response arrived without a pending reverse update".to_string(),
                    )
                }),
            _ => Ok(initial_id),
        }
    }

    fn validate_response_for_request(
        request_kind: RequestKind,
        is_update_response: bool,
        response: &Message,
    ) -> Result<(), SessionError> {
        match response {
            Message::RequestOk(ok) => {
                let properties_allowed =
                    !is_update_response && request_kind == RequestKind::TrackStatus;
                if !properties_allowed && !ok.track_properties.is_empty() {
                    return Err(SessionError::ProtocolViolation(
                        "Track Properties are only valid in TRACK_STATUS_OK".to_string(),
                    ));
                }
            }
            Message::RequestError(error) => {
                if let Some(redirect) = &error.redirect {
                    if request_kind.is_namespace_scoped()
                        && !redirect.track_name.as_bytes().is_empty()
                    {
                        return Err(SessionError::ProtocolViolation(
                            "namespace-scoped redirect contained a Track Name".to_string(),
                        ));
                    }
                }
            }
            Message::Namespace(_) | Message::NamespaceDone(_)
                if request_kind != RequestKind::SubscribeNamespace =>
            {
                return Err(SessionError::ProtocolViolation(format!(
                    "{} is only valid on a SUBSCRIBE_NAMESPACE response stream",
                    response.name()
                )));
            }
            _ => {}
        }
        Ok(())
    }

    /// Encode a response message to a bidi stream, omitting the Request ID
    /// field per draft-19 (the stream identity provides the association).
    /// Build the wire frame for a bidi response message (type + length +
    /// payload with Request ID omitted). Separated from the async writer
    /// so tests can verify the encoding without a QUIC stream.
    fn encode_bidi_response_frame(msg: &Message) -> Result<bytes::BytesMut, SessionError> {
        use bytes::BufMut;

        // Encode the payload (all fields EXCEPT Request ID, which is
        // implicit from the bidi stream identity in draft-19).
        let mut payload = bytes::BytesMut::new();
        match msg {
            Message::RequestOk(m) => {
                m.params.encode(&mut payload)?;
                m.track_properties.encode(&mut payload)?;
            }
            Message::RequestError(m) => {
                m.error_code.encode(&mut payload)?;
                m.retry_interval.encode(&mut payload)?;
                m.reason.encode(&mut payload)?;
                match (
                    m.error_code == message::RequestErrorCode::Redirect as u64,
                    &m.redirect,
                ) {
                    (true, Some(redirect)) => redirect.encode(&mut payload)?,
                    (true, None) => {
                        return Err(SessionError::ProtocolViolation(
                            "REDIRECT REQUEST_ERROR omitted Redirect".to_string(),
                        ));
                    }
                    (false, Some(_)) => {
                        return Err(SessionError::ProtocolViolation(
                            "non-REDIRECT REQUEST_ERROR contained Redirect".to_string(),
                        ));
                    }
                    (false, None) => {}
                }
            }
            Message::SubscribeOk(m) => {
                m.track_alias.encode(&mut payload)?;
                m.params.encode(&mut payload)?;
                m.track_extensions.encode(&mut payload)?;
            }
            Message::PublishDone(m) => {
                m.status_code.encode(&mut payload)?;
                m.stream_count.encode(&mut payload)?;
                m.reason.encode(&mut payload)?;
            }
            Message::FetchOk(m) => {
                m.end_of_track.encode(&mut payload)?;
                m.end_location.encode(&mut payload)?;
                m.params.encode(&mut payload)?;
                m.track_extensions.encode(&mut payload)?;
            }
            Message::Namespace(m) => {
                m.track_namespace_suffix.encode(&mut payload)?;
            }
            Message::NamespaceDone(m) => {
                m.track_namespace_suffix.encode(&mut payload)?;
            }
            other => {
                tracing::warn!(
                    msg_type = other.name(),
                    "unexpected message type in encode_bidi_response — not a bidi response message"
                );
                return Err(SessionError::Internal);
            }
        };

        let msg_type = msg.id();

        if payload.len() > u16::MAX as usize {
            return Err(crate::coding::EncodeError::MsgBoundsExceeded.into());
        }
        let mut frame = bytes::BytesMut::new();
        msg_type.encode(&mut frame)?;
        (payload.len() as u16).encode(&mut frame)?;
        frame.put(payload);
        Ok(frame)
    }

    async fn encode_bidi_response(writer: &mut Writer, msg: &Message) -> Result<(), SessionError> {
        let frame = Self::encode_bidi_response_frame(msg)?;
        writer.write(&frame).await?;
        Ok(())
    }

    /// Decode a response message from a bidi request stream (draft-19).
    ///
    /// Response messages omit the Request ID field — the stream identity
    /// provides the association. The caller supplies the known `request_id`
    /// which is injected into the decoded `Message`.
    pub(super) async fn decode_bidi_response(
        reader: &mut Reader,
        request_id: u64,
        request_kind: RequestKind,
    ) -> Result<Message, SessionError> {
        let (msg_type, payload) = Self::read_bidi_frame(reader).await?;
        Self::decode_bidi_response_payload(msg_type, payload, request_id, request_kind)
    }

    async fn read_bidi_frame(reader: &mut Reader) -> Result<(u64, bytes::BytesMut), SessionError> {
        use crate::coding::DecodeError;
        let msg_type: u64 = reader.decode().await?;
        let msg_len: u16 = reader.decode().await?;
        let len = usize::from(msg_len);
        let mut payload = bytes::BytesMut::new();
        while payload.len() < len {
            let remaining = len - payload.len();
            match reader.read_chunk(remaining).await? {
                Some(chunk) => payload.extend_from_slice(&chunk),
                None => return Err(DecodeError::More(remaining).into()),
            }
        }
        Ok((msg_type, payload))
    }

    fn decode_bidi_response_payload(
        msg_type: u64,
        payload: bytes::BytesMut,
        request_id: u64,
        request_kind: RequestKind,
    ) -> Result<Message, SessionError> {
        use crate::coding::{Decode, ReasonPhrase};
        use bytes::Buf as _;
        let mut buf = &payload[..];

        use message::wire_id;

        let message = match msg_type {
            wire_id::RequestError => {
                let error_code = u64::decode(&mut buf)?;
                let retry_interval = u64::decode(&mut buf)?;
                let reason = ReasonPhrase::decode(&mut buf)?;
                let redirect = if error_code == message::RequestErrorCode::Redirect as u64 {
                    Some(message::Redirect::decode(&mut buf)?)
                } else {
                    None
                };
                Ok(Message::RequestError(message::RequestError {
                    id: request_id,
                    error_code,
                    retry_interval,
                    reason,
                    redirect,
                }))
            }
            wire_id::RequestOk => {
                let params = crate::coding::KeyValuePairs::decode(&mut buf)?;
                let track_properties = message::TrackProperties::decode(&mut buf)?;
                Ok(Message::RequestOk(message::RequestOk {
                    id: request_id,
                    params,
                    track_properties,
                }))
            }
            wire_id::SubscribeOk => {
                let track_alias = u64::decode(&mut buf)?;
                let params = crate::coding::KeyValuePairs::decode(&mut buf)?;
                let track_extensions = message::TrackExtensions::decode(&mut buf)?;
                Ok(Message::SubscribeOk(message::SubscribeOk {
                    id: request_id,
                    track_alias,
                    params,
                    track_extensions,
                }))
            }
            wire_id::PublishDone => {
                let status_code = u64::decode(&mut buf)?;
                let stream_count = u64::decode(&mut buf)?;
                let reason = ReasonPhrase::decode(&mut buf)?;
                Ok(Message::PublishDone(message::PublishDone {
                    id: request_id,
                    status_code,
                    stream_count,
                    reason,
                }))
            }
            wire_id::FetchOk => {
                let end_of_track = bool::decode(&mut buf)?;
                let end_location = crate::coding::Location::decode(&mut buf)?;
                let params = crate::coding::KeyValuePairs::decode(&mut buf)?;
                let track_extensions = message::TrackExtensions::decode(&mut buf)?;
                Ok(Message::FetchOk(message::FetchOk {
                    id: request_id,
                    end_of_track,
                    end_location,
                    params,
                    track_extensions,
                }))
            }
            other => {
                tracing::warn!(msg_type = other, "unexpected bidi response message type");
                Err(SessionError::unimplemented(&format!(
                    "bidi response type 0x{:x}",
                    other
                )))
            }
        }?;

        if buf.has_remaining() {
            return Err(SessionError::ProtocolViolation(format!(
                "response type 0x{:x} left {} unparsed body bytes",
                msg_type,
                buf.remaining()
            )));
        }
        Self::validate_response_for_request(request_kind, false, &message)?;
        Ok(message)
    }

    pub(super) async fn decode_publish_response(
        reader: &mut Reader,
        request_id: u64,
    ) -> Result<Message, SessionError> {
        let (msg_type, payload) = Self::read_bidi_frame(reader).await?;
        Self::decode_publish_response_payload(msg_type, payload, request_id)
    }

    fn decode_publish_response_payload(
        msg_type: u64,
        payload: bytes::BytesMut,
        request_id: u64,
    ) -> Result<Message, SessionError> {
        use crate::coding::Decode;
        use bytes::Buf as _;

        if msg_type != message::wire_id::RequestUpdate {
            return Self::decode_bidi_response_payload(
                msg_type,
                payload,
                request_id,
                RequestKind::Publish,
            );
        }

        let mut body = &payload[..];
        let update = message::RequestUpdate::decode(&mut body)?;
        if body.has_remaining() {
            return Err(SessionError::ProtocolViolation(format!(
                "REQUEST_UPDATE left {} unparsed body bytes",
                body.remaining()
            )));
        }
        Ok(Message::RequestUpdate(update))
    }

    /// Receives post-SETUP messages from the peer's control stream.
    /// Draft-19 permits only `GOAWAY` here; request messages and responses are
    /// rejected before request IDs or application state can be touched.
    async fn run_recv(
        mut recver: Reader,
        mlog: Option<Arc<Mutex<mlog::MlogWriter>>>,
    ) -> Result<(), SessionError> {
        let mut goaway_received = false;

        loop {
            let msg: message::Message = recver.decode().await?;
            Self::validate_control_message(&msg)?;

            // Emit structured tracing log for received control messages
            Self::log_message(&msg, "received on control stream");

            // Emit mlog event for received control messages
            if let Some(ref mlog) = mlog {
                if let Ok(mut mlog_guard) = mlog.lock() {
                    let time = mlog_guard.elapsed_ms();
                    let stream_id = 0; // Control stream is always stream 0

                    let event = match &msg {
                        Message::GoAway(m) => {
                            Some(mlog::events::go_away_parsed(time, stream_id, m))
                        }
                        _ => None,
                    };

                    if let Some(event) = event {
                        let _ = mlog_guard.add_event(event);
                    }
                }
            }

            match msg {
                Message::GoAway(ref m) => {
                    // Draft-19 §10.4: receiving a second GOAWAY is PROTOCOL_VIOLATION.
                    if goaway_received {
                        return Err(SessionError::ProtocolViolation(
                            "received multiple GOAWAY messages".to_string(),
                        ));
                    }
                    goaway_received = true;
                    tracing::info!(
                        target: "moq_transport::control",
                        new_uri = %m.uri.0,
                        "received GOAWAY"
                    );
                    // TODO(itzmanish): trigger session migration.
                }
                other => {
                    return Err(SessionError::ProtocolViolation(format!(
                        "{} is not permitted on the control stream",
                        other.name()
                    )));
                }
            }
        }
    }

    /// Accepts uni-directional quic streams and starts handling for them.
    /// Will read stream header to know what type of stream it is and create
    /// the appropriate stream handlers.
    async fn run_streams(
        webtransport: web_transport::Session,
        subscriber: Option<Subscriber>,
    ) -> Result<(), SessionError> {
        let mut tasks = FuturesUnordered::new();
        let limits = DataStreamTaskLimits::production();

        loop {
            tokio::select! {
                // Reap completed handlers before accepting more streams. This
                // keeps normal streams progressing even under an excess-open
                // flood from the peer.
                biased;
                _ = tasks.next(), if !tasks.is_empty() => {},
                res = webtransport.accept_uni() => {
                    let mut stream = res?;
                    let subscriber = subscriber.clone().ok_or(SessionError::RoleViolation)?;
                    let Some(global_permit) = limits.try_admit(tasks.len()) else {
                        stream.stop(Self::DATA_STREAM_EXCESSIVE_LOAD);
                        tracing::warn!(
                            active_for_session = tasks.len(),
                            per_session_limit = limits.per_session,
                            global_available = limits.global.available_permits(),
                            error_code = Self::DATA_STREAM_EXCESSIVE_LOAD,
                            "rejecting peer data stream: handler capacity exhausted"
                        );
                        continue;
                    };

                    tasks.push(async move {
                        let _global_permit = global_permit;
                        if let Err(err) = Subscriber::recv_stream(subscriber, stream).await {
                            tracing::warn!("failed to serve stream: {}", err);
                        };
                    });
                },
            };
        }
    }

    /// Receives QUIC datagrams and processes them using the Subscriber logic
    async fn run_datagrams(
        webtransport: web_transport::Session,
        mut subscriber: Option<Subscriber>,
    ) -> Result<(), SessionError> {
        loop {
            let datagram = webtransport.recv_datagram().await?;
            subscriber
                .as_mut()
                .ok_or(SessionError::RoleViolation)?
                .recv_datagram(datagram)
                .await?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TaskDropCounter(Arc<AtomicUsize>);

    impl Drop for TaskDropCounter {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn pending_task(dropped: Arc<AtomicUsize>) -> tokio::task::JoinHandle<()> {
        let drop_counter = TaskDropCounter(dropped);
        tokio::spawn(async move {
            let _drop_counter = drop_counter;
            futures::future::pending::<()>().await;
        })
    }

    fn goaway() -> Message {
        Message::GoAway(message::GoAway {
            uri: crate::coding::SessionUri(String::new()),
            timeout: 0,
        })
    }

    #[test]
    fn control_ingress_accepts_only_goaway() {
        for message in crate::message::tests::request_only_messages() {
            let error = Session::validate_control_message(&message).unwrap_err();
            assert!(matches!(&error, SessionError::ProtocolViolation(_)));
            assert_eq!(
                error.code(),
                0x3,
                "{} used the wrong close code",
                message.name()
            );
        }

        assert!(Session::validate_control_message(&goaway()).is_ok());
    }

    #[test]
    fn rejected_control_request_does_not_consume_its_request_id() {
        let request_ids = RequestId::new(1, 0);
        let request = Message::TrackStatus(message::TrackStatus {
            id: 0,
            track_namespace: crate::coding::TrackNamespace::from_utf8_path("live"),
            track_name: "audio".into(),
            params: Default::default(),
        });

        assert!(Session::validate_control_message(&request).is_err());
        request_ids.validate_incoming(0).unwrap();
    }

    #[test]
    fn outbound_queue_never_falls_through_to_control_for_request_messages() {
        for message in crate::message::tests::request_only_messages() {
            match message.response_target_id() {
                Some(request_id) => assert_eq!(
                    Session::outgoing_message_destination(&message).unwrap(),
                    OutgoingMessageDestination::Request(request_id),
                    "{} was not routed to its request stream",
                    message.name()
                ),
                None => assert!(
                    matches!(
                        Session::outgoing_message_destination(&message),
                        Err(SessionError::Internal)
                    ),
                    "{} was allowed to fall through to control",
                    message.name()
                ),
            }
        }

        assert_eq!(
            Session::outgoing_message_destination(&goaway()).unwrap(),
            OutgoingMessageDestination::Control
        );
    }

    #[test]
    fn excessive_load_is_a_retryable_request_response_not_session_failure() {
        let Message::RequestError(error) = Session::excessive_load_response(42) else {
            panic!("expected REQUEST_ERROR");
        };
        assert_eq!(error.id, 42);
        assert_eq!(
            error.error_code,
            message::RequestErrorCode::ExcessiveLoad as u64
        );
        assert_eq!(error.retry_interval, 1001);
        assert!(error.redirect.is_none());
        assert!(!error.reason.0.is_empty());
    }

    #[test]
    fn fetch_update_not_supported_is_scoped_to_the_update_request() {
        let update_error = Message::RequestError(message::RequestError::new(
            43,
            message::RequestErrorCode::NotSupported,
            0,
            "FETCH updates are not supported",
        ));
        assert!(!Session::response_is_terminal(
            RequestKind::Fetch,
            true,
            &update_error
        ));
        assert!(Session::response_is_terminal(
            RequestKind::Fetch,
            false,
            &update_error
        ));
        assert!(Session::response_is_terminal(
            RequestKind::Fetch,
            false,
            &Message::FetchOk(message::FetchOk {
                id: 42,
                end_of_track: false,
                end_location: crate::coding::Location::new(1, 2),
                params: Default::default(),
                track_extensions: Default::default(),
            })
        ));
    }

    #[test]
    fn reverse_updates_are_bounded_even_when_peer_advertises_unlimited() {
        assert_eq!(
            Session::effective_reverse_update_limit(RequestUpdateLimits {
                incoming: 16,
                outgoing: 0,
                outgoing_hard: 64,
            }),
            64
        );
        assert_eq!(
            Session::effective_reverse_update_limit(RequestUpdateLimits {
                incoming: 16,
                outgoing: 8,
                outgoing_hard: 64,
            }),
            8
        );
        assert_eq!(
            Session::effective_reverse_update_limit(RequestUpdateLimits {
                incoming: 16,
                outgoing: 4_096,
                outgoing_hard: 64,
            }),
            64
        );
    }

    #[test]
    fn inbound_guard_cleans_response_registry_and_capacity_during_unwind() {
        let mut limits = RequestLimits::default();
        limits.session_inbound.total = 1;
        limits.session_inbound.fetch = 1;
        limits.process_inbound.total = 1;
        limits.process_inbound.fetch = 1;
        let capacity = RequestCapacity::new(limits).unwrap();
        let session = capacity.session();
        let lease = Arc::new(
            session
                .try_acquire(RequestDirection::Inbound, RequestClass::Fetch)
                .unwrap(),
        );
        let responses: BidiResponseMap = Arc::new(Mutex::new(HashMap::new()));
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let (update_tx, _update_rx) = tokio::sync::mpsc::channel(1);
        responses
            .lock()
            .unwrap()
            .extend([(77, tx), (79, update_tx)]);

        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe({
            let responses = responses.clone();
            move || {
                let _guard = InboundRequestGuard {
                    kind: RequestKind::Fetch,
                    id: 77,
                    publisher: None,
                    subscriber: None,
                    responses,
                    response_ids: Arc::new(Mutex::new(std::collections::HashSet::from([77, 79]))),
                    request_lease: lease,
                };
                panic!("exercise lifecycle guard");
            }
        }));
        assert!(unwind.is_err());
        assert!(responses.lock().unwrap().is_empty());
        assert!(session
            .try_acquire(RequestDirection::Inbound, RequestClass::Fetch)
            .is_ok());
    }

    // ========================================================================
    // normalize_connection_path
    // ========================================================================

    #[test]
    fn normalize_empty_and_root() {
        assert_eq!(Session::normalize_connection_path("").unwrap(), None);
        assert_eq!(Session::normalize_connection_path("/").unwrap(), None);
        assert_eq!(
            Session::normalize_connection_path("///").unwrap(),
            Some("///".to_string())
        );
    }

    #[test]
    fn normalize_valid_paths() {
        assert_eq!(
            Session::normalize_connection_path("/app").unwrap(),
            Some("/app".to_string())
        );
        assert_eq!(
            Session::normalize_connection_path("/tenant/stream-1").unwrap(),
            Some("/tenant/stream-1".to_string())
        );
        // RFC 3986 path identity is retained exactly.
        assert_eq!(
            Session::normalize_connection_path("/app/").unwrap(),
            Some("/app/".to_string())
        );
    }

    #[test]
    fn normalize_rejects_missing_leading_slash() {
        assert!(Session::normalize_connection_path("app").is_err());
    }

    #[test]
    fn normalize_accepts_rfc3986_empty_segments() {
        assert_eq!(
            Session::normalize_connection_path("/app//stream").unwrap(),
            Some("/app//stream".to_string())
        );
    }

    #[test]
    fn normalize_retains_rfc3986_path_identity() {
        assert!(Session::normalize_connection_path("/app/./stream").is_ok());
        assert!(Session::normalize_connection_path("/app/../secret").is_ok());
        assert!(Session::normalize_connection_path("/..").is_ok());
    }

    #[test]
    fn normalize_preserves_percent_encoded_characters() {
        assert_eq!(
            Session::normalize_connection_path("/foo%2Fbar?x=%2F").unwrap(),
            Some("/foo%2Fbar?x=%2F".to_string())
        );
        assert!(Session::normalize_connection_path("/%2E%2E/secret").is_ok());
        assert!(Session::normalize_connection_path("/app/%00").is_ok());
    }

    #[test]
    fn normalize_rejects_too_long_path() {
        let long_path = format!("/{}", "a".repeat(SessionTarget::MAX_URI_BYTES));
        assert!(Session::normalize_connection_path(&long_path).is_err());
    }

    #[test]
    fn normalize_accepts_max_length_path() {
        let path = format!("/{}", "a".repeat(8_000));
        assert!(Session::normalize_connection_path(&path).is_ok());
    }

    #[test]
    fn webtransport_rejects_path_and_authority_setup_options() {
        let url = url::Url::parse("https://relay.example/live?q=1").unwrap();
        let negotiated =
            NegotiatedTransport::new(Transport::WebTransport, setup::SUPPORTED_ALPNS[0]);

        let mut params = KeyValuePairs::default();
        params.set_bytesvalue(setup::ParameterType::Path.into(), b"/other".to_vec());
        let error = Session::target_from_client_setup(&url, negotiated, &params).unwrap_err();
        assert!(matches!(error, SessionError::InvalidPath(_)));
        assert_eq!(error.code(), 0x8);

        let mut params = KeyValuePairs::default();
        params.set_bytesvalue(
            setup::ParameterType::Authority.into(),
            b"relay.example".to_vec(),
        );
        let error = Session::target_from_client_setup(&url, negotiated, &params).unwrap_err();
        assert!(matches!(error, SessionError::InvalidAuthority(_)));
        assert_eq!(error.code(), 0x19);
    }

    #[test]
    fn raw_quic_requires_well_formed_path_and_authority_setup_options() {
        let url = url::Url::parse("moqt://relay.example").unwrap();
        let negotiated = NegotiatedTransport::new(Transport::RawQuic, setup::SUPPORTED_ALPNS[0]);

        let missing =
            Session::target_from_client_setup(&url, negotiated, &KeyValuePairs::default())
                .unwrap_err();
        assert!(matches!(missing, SessionError::MalformedAuthority(_)));

        let mut malformed_path = KeyValuePairs::default();
        malformed_path.set_intvalue(setup::ParameterType::Path.into(), 1);
        malformed_path.set_bytesvalue(
            setup::ParameterType::Authority.into(),
            b"relay.example".to_vec(),
        );
        assert!(matches!(
            Session::target_from_client_setup(&url, negotiated, &malformed_path),
            Err(SessionError::MalformedPath(_))
        ));

        let mut malformed_authority = KeyValuePairs::default();
        malformed_authority.set_bytesvalue(setup::ParameterType::Path.into(), b"/live".to_vec());
        malformed_authority.set_intvalue(setup::ParameterType::Authority.into(), 1);
        assert!(matches!(
            Session::target_from_client_setup(&url, negotiated, &malformed_authority),
            Err(SessionError::MalformedAuthority(_))
        ));

        let mut userinfo_authority = KeyValuePairs::default();
        userinfo_authority.set_bytesvalue(setup::ParameterType::Path.into(), b"/live".to_vec());
        userinfo_authority.set_bytesvalue(
            setup::ParameterType::Authority.into(),
            b"user:password@relay.example".to_vec(),
        );
        let error =
            Session::target_from_client_setup(&url, negotiated, &userinfo_authority).unwrap_err();
        assert!(matches!(error, SessionError::MalformedAuthority(_)));
        let diagnostic = format!("{error:?} {error}");
        assert!(!diagnostic.contains("user"));
        assert!(!diagnostic.contains("password"));
    }

    #[test]
    fn accepted_session_logging_never_emits_raw_authority() {
        let implementation = include_str!("mod.rs").split("#[cfg(test)]").next().unwrap();
        assert!(!implementation.contains("authority = target.authority()"));
        assert!(implementation.contains("target.redacted_for_logging()"));
    }

    #[test]
    fn raw_quic_reconstructs_exact_target_and_rejects_wrong_host() {
        let url = url::Url::parse("moqt://relay.example").unwrap();
        let negotiated = NegotiatedTransport::new(Transport::RawQuic, setup::SUPPORTED_ALPNS[0]);
        let mut params = KeyValuePairs::default();
        params.set_bytesvalue(
            setup::ParameterType::Authority.into(),
            b"relay.example:4443".to_vec(),
        );
        params.set_bytesvalue(
            setup::ParameterType::Path.into(),
            b"/a%2Fb?token=x%2Fy".to_vec(),
        );

        let target = Session::target_from_client_setup(&url, negotiated, &params).unwrap();
        assert_eq!(
            target.canonical_url().as_str(),
            "moqt://relay.example:4443/a%2Fb?token=x%2Fy"
        );

        params.set_bytesvalue(
            setup::ParameterType::Authority.into(),
            b"other.example".to_vec(),
        );
        let error = Session::target_from_client_setup(&url, negotiated, &params).unwrap_err();
        assert!(matches!(error, SessionError::InvalidAuthority(_)));
        assert_eq!(error.code(), 0x19);
    }

    #[test]
    fn server_setup_must_not_contain_path_or_authority() {
        let mut params = KeyValuePairs::default();
        params.set_bytesvalue(setup::ParameterType::Path.into(), b"/live".to_vec());
        assert!(matches!(
            Session::validate_server_setup_options(&params),
            Err(SessionError::InvalidPath(_))
        ));

        let mut params = KeyValuePairs::default();
        params.set_bytesvalue(
            setup::ParameterType::Authority.into(),
            b"relay.example".to_vec(),
        );
        assert!(matches!(
            Session::validate_server_setup_options(&params),
            Err(SessionError::InvalidAuthority(_))
        ));
    }

    #[test]
    fn setup_authorization_is_bounded_redacted_and_type_checked() {
        let authorization = SetupAuthorization::new(b"super-secret-token").unwrap();
        assert_eq!(authorization.as_bytes(), b"super-secret-token");
        let debug = format!("{authorization:?}");
        assert!(debug.contains("redacted:18"));
        assert!(!debug.contains("super-secret-token"));
        assert!(SetupAuthorization::new([]).is_err());
        assert!(SetupAuthorization::new(vec![0; SetupAuthorization::MAX_BYTES + 1]).is_err());

        let mut params = KeyValuePairs::default();
        params.set_intvalue(setup::ParameterType::AuthorizationToken.into(), 1);
        assert!(Session::setup_authorizations(&params).is_err());
    }

    #[test]
    fn repeated_setup_authorizations_are_extracted_in_order() {
        let key = setup::ParameterType::AuthorizationToken.into();
        let params = KeyValuePairs(vec![
            crate::coding::KeyValuePair::new_bytes(
                key,
                AuthorizationToken::use_value(0, b"first")
                    .unwrap()
                    .encode_bytes()
                    .unwrap(),
            ),
            crate::coding::KeyValuePair::new_bytes(
                key,
                AuthorizationToken::use_value(7, b"second")
                    .unwrap()
                    .encode_bytes()
                    .unwrap(),
            ),
        ]);
        let authorizations = Session::setup_authorizations(&params).unwrap();
        assert_eq!(authorizations.len(), 2);
        assert_eq!(authorizations[0].token_type(), 0);
        assert_eq!(authorizations[0].as_bytes(), b"first");
        assert_eq!(authorizations[1].token_type(), 7);
        assert_eq!(authorizations[1].as_bytes(), b"second");
    }

    #[test]
    fn duplicate_resolved_setup_authorization_is_rejected() {
        let key = setup::ParameterType::AuthorizationToken.into();
        let token = AuthorizationToken::use_value(0, b"same")
            .unwrap()
            .encode_bytes()
            .unwrap();
        let params = KeyValuePairs(vec![
            crate::coding::KeyValuePair::new_bytes(key, token.clone()),
            crate::coding::KeyValuePair::new_bytes(key, token),
        ]);
        let error = Session::setup_authorizations(&params).unwrap_err();
        assert!(matches!(error, SessionError::ProtocolViolation(_)));
    }

    #[test]
    fn setup_register_is_use_value_when_cache_is_zero() {
        let key = setup::ParameterType::AuthorizationToken.into();
        let token = AuthorizationToken::register(37, 9, b"secret")
            .unwrap()
            .encode_bytes()
            .unwrap();
        let params = KeyValuePairs(vec![crate::coding::KeyValuePair::new_bytes(key, token)]);
        let authorizations = Session::setup_authorizations(&params).unwrap();
        assert_eq!(authorizations[0].token_type(), 9);
        assert_eq!(authorizations[0].as_bytes(), b"secret");
    }

    #[test]
    fn setup_delete_and_use_alias_are_protocol_violations() {
        for token in [
            AuthorizationToken::delete(37),
            AuthorizationToken::use_alias(37),
        ] {
            let key = setup::ParameterType::AuthorizationToken.into();
            let params = KeyValuePairs(vec![crate::coding::KeyValuePair::new_bytes(
                key,
                token.encode_bytes().unwrap(),
            )]);
            let error = Session::setup_authorizations(&params).unwrap_err();
            assert!(matches!(error, SessionError::ProtocolViolation(_)));
            assert_eq!(error.code(), 0x3);
        }
    }

    #[test]
    fn malformed_setup_authorization_maps_to_key_value_formatting_error() {
        let key = setup::ParameterType::AuthorizationToken.into();
        let params = KeyValuePairs(vec![crate::coding::KeyValuePair::new_bytes(
            key,
            vec![0x03],
        )]);
        let error = Session::setup_authorizations(&params).unwrap_err();
        assert!(matches!(error, SessionError::KeyValueFormatting(_)));
        assert_eq!(error.code(), 0x6);
    }

    #[test]
    fn outbound_setup_authorization_uses_use_value_structure() {
        let authorization = SetupAuthorization::new_typed(7, b"secret").unwrap();
        let wire = authorization.encode_wire_value().unwrap();
        assert_eq!(
            AuthorizationToken::decode_bytes(wire).unwrap(),
            AuthorizationToken::use_value(7, b"secret").unwrap()
        );
    }

    // ========================================================================
    // task admission and shutdown
    // ========================================================================

    #[test]
    fn data_stream_admission_enforces_per_session_and_global_limits() {
        let per_session = DataStreamTaskLimits {
            global: Arc::new(tokio::sync::Semaphore::new(8)),
            per_session: 1,
        };
        let permit = per_session.try_admit(0).expect("first stream admitted");
        assert!(per_session.try_admit(1).is_none());
        drop(permit);

        let global = Arc::new(tokio::sync::Semaphore::new(2));
        let first_session = DataStreamTaskLimits {
            global: global.clone(),
            per_session: 8,
        };
        let second_session = DataStreamTaskLimits {
            global,
            per_session: 8,
        };
        let first = first_session.try_admit(0).expect("first global permit");
        let second = second_session.try_admit(0).expect("second global permit");
        assert!(first_session.try_admit(1).is_none());
        drop(first);
        assert!(first_session.try_admit(1).is_some());
        drop(second);

        assert_eq!(Session::DATA_STREAM_EXCESSIVE_LOAD, 0x9);
    }

    #[test]
    fn bidi_request_admission_enforces_process_cap_and_releases() {
        let global = Arc::new(tokio::sync::Semaphore::new(2));
        let first_session = BidiRequestTaskLimits {
            global: global.clone(),
            per_session: 2,
        };
        let second_session = BidiRequestTaskLimits {
            global,
            per_session: 2,
        };
        let first = first_session.try_admit(0).expect("first request admitted");
        let second = second_session
            .try_admit(0)
            .expect("second request admitted");
        assert!(first_session.try_admit(1).is_none());
        assert!(first_session.try_admit(2).is_none());
        drop(first);
        assert!(first_session.try_admit(1).is_some());
        drop(second);
    }

    #[tokio::test]
    async fn bidi_task_shutdown_drains_aborts_and_awaits_all_accepted_handles() {
        let dropped = Arc::new(AtomicUsize::new(0));
        let (sender, mut receiver) = BidiTaskSender::channel(2);
        let mut collected = FuturesUnordered::new();

        collected.push(pending_task(dropped.clone()));
        assert!(sender.send(pending_task(dropped.clone())).is_ok());
        assert!(sender.send(pending_task(dropped.clone())).is_ok());

        Session::shutdown_bidi_tasks(&mut receiver, &mut collected).await;

        assert!(collected.is_empty());
        assert_eq!(dropped.load(Ordering::SeqCst), 3);
        assert!(receiver.is_closed());
    }

    #[tokio::test]
    async fn bidi_task_queue_rejects_n_plus_one_and_aborts_rejected_task() {
        let dropped = Arc::new(AtomicUsize::new(0));
        let (sender, mut receiver) = BidiTaskSender::channel(1);
        let mut collected = FuturesUnordered::new();

        assert!(sender.send(pending_task(dropped.clone())).is_ok());
        let rejected = sender.send(pending_task(dropped.clone()));
        assert!(rejected.is_err());
        drop(rejected);

        Session::shutdown_bidi_tasks(&mut receiver, &mut collected).await;
        for _ in 0..100 {
            if dropped.load(Ordering::SeqCst) == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(dropped.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn bidi_task_sender_aborts_handle_raced_after_collector_close() {
        let dropped = Arc::new(AtomicUsize::new(0));
        let (sender, mut receiver) = BidiTaskSender::channel(1);
        receiver.close();

        let result = sender.send(pending_task(dropped.clone()));
        assert!(result.is_err());
        drop(result);

        for _ in 0..100 {
            if dropped.load(Ordering::SeqCst) == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn bidi_task_send_error_can_abort_and_join_raced_handle() {
        let dropped = Arc::new(AtomicUsize::new(0));
        let (sender, mut receiver) = BidiTaskSender::channel(1);
        receiver.close();

        let Err(error) = sender.send(pending_task(dropped.clone())) else {
            panic!("closed task collector accepted a handle");
        };
        error.abort_and_wait().await;

        assert_eq!(dropped.load(Ordering::SeqCst), 1);
    }

    // ========================================================================
    // encode_bidi_response — verify wire format (no Request ID)
    // ========================================================================

    /// Helper: calls the production `encode_bidi_response_frame` and
    /// returns the raw bytes. No duplicate encoding logic.
    fn encode_bidi_response_bytes(msg: &Message) -> Vec<u8> {
        Session::encode_bidi_response_frame(msg).unwrap().to_vec()
    }

    #[test]
    fn encode_bidi_request_ok_omits_request_id() {
        use message::wire_id;
        let msg = Message::RequestOk(message::RequestOk {
            id: 42, // should NOT appear on the wire
            params: crate::coding::KeyValuePairs::default(),
            track_properties: Default::default(),
        });
        let bytes = encode_bidi_response_bytes(&msg);
        // type (1 byte) + length 0x0001 (2 bytes) + params_count=0 (1 byte) = 4 bytes
        assert_eq!(bytes[0], wire_id::RequestOk as u8);
        assert_eq!(bytes.len(), 4);
    }

    #[test]
    fn encode_bidi_request_error_omits_request_id() {
        use message::wire_id;
        let msg = Message::RequestError(message::RequestError {
            id: 99, // should NOT appear on the wire
            error_code: 0x10,
            retry_interval: 0,
            reason: crate::coding::ReasonPhrase("nf".to_string()),
            redirect: None,
        });
        let bytes = encode_bidi_response_bytes(&msg);
        assert_eq!(bytes[0], wire_id::RequestError as u8);
        // No 99 (0x63) anywhere in the output
        assert!(
            !bytes.contains(&99),
            "Request ID must not appear in bidi encoding"
        );
    }

    #[test]
    fn encode_namespace_response_uses_stream_association() {
        use message::wire_id;
        let msg = Message::Namespace(message::Namespace {
            track_namespace_suffix: crate::coding::TrackNamespacePrefix::from_utf8_path(
                "live/clock",
            ),
        });
        let bytes = encode_bidi_response_bytes(&msg);
        assert_eq!(bytes[0], wire_id::Namespace as u8);
        assert!(Session::validate_response_for_request(
            RequestKind::SubscribeNamespace,
            false,
            &msg,
        )
        .is_ok());
        assert!(matches!(
            Session::validate_response_for_request(RequestKind::Subscribe, false, &msg),
            Err(SessionError::ProtocolViolation(_))
        ));
    }

    #[test]
    fn response_target_id_covers_responses_only() {
        // Response messages should return Some(id)
        assert!(Message::RequestOk(message::RequestOk {
            id: 1,
            params: Default::default(),
            track_properties: Default::default(),
        })
        .response_target_id()
        .is_some());
        assert!(Message::RequestError(message::RequestError {
            id: 1,
            error_code: 0,
            retry_interval: 0,
            reason: Default::default(),
            redirect: None,
        })
        .response_target_id()
        .is_some());

        // Request messages should return None
        assert!(Message::Subscribe(message::Subscribe {
            id: 1,
            track_namespace: crate::coding::TrackNamespace::from_utf8_path("t"),
            track_name: "n".into(),
            params: Default::default(),
        })
        .response_target_id()
        .is_none());
        assert!(Message::GoAway(message::GoAway {
            uri: crate::coding::SessionUri(String::new()),
            timeout: 0,
        })
        .response_target_id()
        .is_none());
    }

    #[test]
    fn request_ok_properties_are_rejected_outside_track_status() {
        let mut properties = message::TrackProperties::default();
        properties.set_int_extension(0x78, 1);
        let response = Message::RequestOk(message::RequestOk {
            id: 2,
            params: Default::default(),
            track_properties: properties,
        });

        assert!(matches!(
            Session::validate_response_for_request(RequestKind::Subscribe, false, &response),
            Err(SessionError::ProtocolViolation(_))
        ));
        assert!(
            Session::validate_response_for_request(RequestKind::TrackStatus, false, &response)
                .is_ok()
        );
        assert!(matches!(
            Session::validate_response_for_request(RequestKind::TrackStatus, true, &response),
            Err(SessionError::ProtocolViolation(_))
        ));
    }

    #[test]
    fn namespace_redirect_rejects_track_name() {
        let response = Message::RequestError(message::RequestError {
            id: 2,
            error_code: message::RequestErrorCode::Redirect as u64,
            retry_interval: 0,
            reason: Default::default(),
            redirect: Some(message::Redirect {
                connect_uri: Default::default(),
                track_namespace: Default::default(),
                track_name: "audio".into(),
            }),
        });

        assert!(matches!(
            Session::validate_response_for_request(RequestKind::PublishNamespace, false, &response),
            Err(SessionError::ProtocolViolation(_))
        ));
    }

    #[test]
    fn encode_bidi_publish_done_omits_request_id() {
        use message::wire_id;
        let msg = Message::PublishDone(message::PublishDone {
            id: 77,
            status_code: 0,
            stream_count: 3,
            reason: crate::coding::ReasonPhrase("done".to_string()),
        });
        let bytes = encode_bidi_response_bytes(&msg);
        assert_eq!(bytes[0], wire_id::PublishDone as u8);
        assert!(
            !bytes.contains(&77),
            "Request ID must not appear in bidi encoding"
        );
    }

    #[test]
    fn encode_bidi_fetch_ok_omits_request_id() {
        use message::wire_id;
        let msg = Message::FetchOk(message::FetchOk {
            id: 88,
            end_of_track: true,
            end_location: crate::coding::Location::new(5, 10),
            params: crate::coding::KeyValuePairs::default(),
            track_extensions: Default::default(),
        });
        let bytes = encode_bidi_response_bytes(&msg);
        assert_eq!(bytes[0], wire_id::FetchOk as u8);
        assert!(
            !bytes.contains(&88),
            "Request ID must not appear in bidi encoding"
        );
    }

    #[test]
    fn publish_followups_keep_terminal_and_update_response_associations_distinct() {
        assert_eq!(
            Session::publish_followup_request_id(message::wire_id::PublishDone, 10, Some(12))
                .unwrap(),
            10
        );
        assert_eq!(
            Session::publish_followup_request_id(message::wire_id::RequestOk, 10, Some(12))
                .unwrap(),
            12
        );
        assert!(matches!(
            Session::publish_followup_request_id(message::wire_id::RequestError, 10, None),
            Err(SessionError::ProtocolViolation(_))
        ));
    }

    #[test]
    fn publish_response_direction_decodes_full_request_update_id_and_forward() {
        let mut params = KeyValuePairs::default();
        params.set_forward(false);
        let update = message::RequestUpdate { id: 14, params };
        let mut payload = bytes::BytesMut::new();
        update.encode(&mut payload).unwrap();

        let decoded =
            Session::decode_publish_response_payload(message::wire_id::RequestUpdate, payload, 10)
                .unwrap();
        let Message::RequestUpdate(decoded) = decoded else {
            panic!("expected REQUEST_UPDATE");
        };
        assert_eq!(decoded.id, 14);
        assert_eq!(decoded.params.forward().unwrap(), Some(false));
    }
}
