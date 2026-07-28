// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-FileCopyrightText: 2023-2024 Luke Curley and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use futures::{stream::FuturesUnordered, StreamExt};

use crate::{
    coding::{KeyValuePairs, TrackNamespace, TrackNamespacePrefix},
    message::{self, Message},
    mlog,
    serve::{ServeError, TrackReader, TracksReader},
};

use crate::watch::Queue;

use super::subscribed::{
    lookup_joining_subscription as lookup_joining_subscription_in, JoiningSubscriptionLookup,
    JoiningSubscriptionLookupError,
};
use super::{
    BidiCommand, BidiResponseMap, FetchRequested, FetchRequestedRecv, PublishNamespace,
    PublishNamespaceRecv, PublishNamespaceRejection, Published, PublishedInfo, RequestClass,
    RequestDirection, RequestId, RequestLease, RequestUpdateCredits, Session, SessionError,
    SessionRequestCapacity, Subscribed, SubscribedNamespace, SubscribedNamespaceRecv,
    SubscribedRecv, TrackStatusRequested, DEFAULT_PUBLISH_NAMESPACE_ACCEPTANCE_TIMEOUT,
};
use crate::message::RequestErrorCode;

enum PublishRequestStreamEvent {
    Response(Result<Option<Message>, SessionError>),
    Command(Option<BidiCommand>),
    SendStopped(Result<Option<u8>, SessionError>),
}

struct SubscribedNamespaceRegistry<T> {
    active: HashMap<u64, (TrackNamespacePrefix, T)>,
}

impl<T> Default for SubscribedNamespaceRegistry<T> {
    fn default() -> Self {
        Self {
            active: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubscribedNamespaceInsertError {
    DuplicateRequestId,
    PrefixOverlap,
}

impl<T> SubscribedNamespaceRegistry<T> {
    fn try_insert(
        &mut self,
        request_id: u64,
        prefix: TrackNamespacePrefix,
        value: T,
    ) -> Result<(), SubscribedNamespaceInsertError> {
        if self.active.contains_key(&request_id) {
            return Err(SubscribedNamespaceInsertError::DuplicateRequestId);
        }
        if self
            .active
            .values()
            .any(|(active, _)| namespace_prefixes_overlap(active, &prefix))
        {
            return Err(SubscribedNamespaceInsertError::PrefixOverlap);
        }
        self.active.insert(request_id, (prefix, value));
        Ok(())
    }

    fn remove(&mut self, request_id: u64) -> Option<T> {
        self.active.remove(&request_id).map(|(_, value)| value)
    }
}

fn namespace_prefixes_overlap(left: &TrackNamespacePrefix, right: &TrackNamespacePrefix) -> bool {
    left.fields
        .iter()
        .zip(&right.fields)
        .all(|(left, right)| left == right)
}

struct PublishNamespaceResponseGuard {
    publisher: Publisher,
    request_id: u64,
}

struct PublishResponseGuard {
    publisher: Publisher,
    request_id: u64,
    responses: BidiResponseMap,
}

impl Drop for PublishResponseGuard {
    fn drop(&mut self) {
        if let Ok(mut responses) = self.responses.lock() {
            responses.remove(&self.request_id);
        }
        self.publisher
            .reject_published_locally(self.request_id, ServeError::Cancel);
    }
}

impl Drop for PublishNamespaceResponseGuard {
    fn drop(&mut self) {
        self.publisher
            .recv_publish_namespace_response_stream_closed(self.request_id);
    }
}

// TODO remove Clone.
#[derive(Clone)]
pub struct Publisher {
    webtransport: web_transport::Session,

    /// Active outbound PUBLISH_NAMESPACE requests, keyed by namespace.
    publish_namespaces: Arc<Mutex<HashMap<TrackNamespace, PublishNamespaceRecv>>>,

    /// When a Subscribe is received and we have a matching publish_namespace entry, the
    /// subscription is routed to that PublishNamespaceRecv.  Otherwise it goes here.
    subscribeds: Arc<Mutex<HashMap<u64, SubscribedRecv>>>,

    /// Active outbound PUBLISH requests keyed by request ID.
    published: Arc<Mutex<HashMap<u64, SubscribedRecv>>>,

    /// Subscriptions for namespaces that have no matching PUBLISH_NAMESPACE.
    unknown_subscribed: Queue<Subscribed>,

    /// Active inbound SUBSCRIBE_NAMESPACE requests, keyed by request ID.
    subscribed_namespaces: Arc<Mutex<SubscribedNamespaceRegistry<SubscribedNamespaceRecv>>>,

    /// Inbound SUBSCRIBE_NAMESPACE requests surfaced to the application.
    unknown_subscribed_namespace: Queue<SubscribedNamespace>,

    /// Active inbound FETCH requests, keyed by request ID.
    fetches: Arc<Mutex<HashMap<u64, FetchRequestedRecv>>>,

    /// Joining FETCH requests waiting for application serving policy.
    unknown_fetch_requested: Queue<FetchRequested>,

    /// TRACK_STATUS requests for namespaces that have no matching PUBLISH_NAMESPACE.
    unknown_track_status_requested: Queue<TrackStatusRequested>,

    /// Shared queue for request-stream responses and session-level messages.
    outgoing: Queue<Message>,

    /// Shared with Subscriber so all requests within a session use unique IDs.
    /// When we need a new Request Id for sending a request, we can get it from here.
    /// The manager is shared with the Subscriber, so the session uses unique request ids
    /// for all requests generated.  If we initiated the QUIC connection then request
    /// IDs start at 0 and increment by 2 (even numbers).  If we accepted an inbound
    /// QUIC connection then request IDs start at 1 and increment by 2 (odd numbers).
    request_id: RequestId,

    /// Optional mlog writer for logging transport events
    mlog: Option<Arc<Mutex<mlog::MlogWriter>>>,

    /// Channel for sending spawned bidi reader task handles to Session::run.
    bidi_task_tx: super::BidiTaskSender,

    /// Request-stream writers used to deliver PUBLISH_DONE on the same bidi
    /// stream as the original outbound PUBLISH.
    bidi_response_map: BidiResponseMap,

    /// Shared fail-fast ownership for logical inbound and outbound requests.
    request_capacity: SessionRequestCapacity,
}

impl Publisher {
    pub(super) fn retention_budget(&self) -> crate::serve::RetentionBudget {
        self.request_capacity.retention_budget()
    }

    pub(super) fn retention_track_limits(&self) -> crate::serve::RetentionLimits {
        self.request_capacity.retention_track_limits()
    }

    pub(super) fn new(
        outgoing: Queue<Message>,
        webtransport: web_transport::Session,
        mlog: Option<Arc<Mutex<mlog::MlogWriter>>>,
        request_id: RequestId,
        bidi_task_tx: super::BidiTaskSender,
        bidi_response_map: BidiResponseMap,
        request_capacity: SessionRequestCapacity,
    ) -> Self {
        let limits = request_capacity.limits();
        Self {
            webtransport,
            publish_namespaces: Default::default(),
            subscribeds: Default::default(),
            published: Default::default(),
            unknown_subscribed: Queue::bounded(limits.session_inbound.subscribe),
            subscribed_namespaces: Default::default(),
            unknown_subscribed_namespace: Queue::bounded(limits.session_inbound.subscribe),
            fetches: Default::default(),
            unknown_fetch_requested: Queue::bounded(limits.session_inbound.fetch),
            unknown_track_status_requested: Queue::bounded(limits.session_inbound.track_status),
            outgoing,
            request_id,
            mlog,
            bidi_task_tx,
            bidi_response_map,
            request_capacity,
        }
    }

    pub async fn accept(
        session: web_transport::Session,
        negotiated: super::NegotiatedTransport,
    ) -> Result<(Session, Publisher), SessionError> {
        let (session, publisher, _) = Session::accept(session, None, negotiated).await?;
        Ok((session, publisher.unwrap()))
    }

    pub async fn accept_with_capacity(
        session: web_transport::Session,
        negotiated: super::NegotiatedTransport,
        request_capacity: &super::RequestCapacity,
    ) -> Result<(Session, Publisher), SessionError> {
        let (session, publisher, _) =
            Session::accept_with_capacity(session, None, negotiated, request_capacity).await?;
        Ok((session, publisher.unwrap()))
    }

    pub async fn connect(
        session: web_transport::Session,
        negotiated: super::NegotiatedTransport,
    ) -> Result<(Session, Publisher), SessionError> {
        let (session, publisher, _) = Session::connect(session, None, negotiated).await?;
        Ok((session, publisher))
    }

    pub async fn connect_with_capacity(
        session: web_transport::Session,
        negotiated: super::NegotiatedTransport,
        request_capacity: &super::RequestCapacity,
    ) -> Result<(Session, Publisher), SessionError> {
        let (session, publisher, _) =
            Session::connect_with_capacity(session, None, negotiated, request_capacity).await?;
        Ok((session, publisher))
    }

    /// Resolve a Joining FETCH reference only within this MOQT session.
    ///
    /// Pending subscriber-initiated subscriptions are returned so the FETCH
    /// layer can wait for establishment. Unknown, publisher-initiated, and
    /// terminated subscriptions are rejected by the state helper.
    #[allow(dead_code)] // Consumed when the request-stream FETCH handler lands.
    pub(super) fn lookup_joining_subscription(
        &self,
        request_id: u64,
    ) -> Result<JoiningSubscriptionLookup, JoiningSubscriptionLookupError> {
        let subscriptions = self
            .subscribeds
            .lock()
            .map_err(|_| JoiningSubscriptionLookupError::Internal)?;
        lookup_joining_subscription_in(&subscriptions, request_id)
    }

    pub(super) async fn resolve_joining_subscription(
        &self,
        request_id: u64,
    ) -> Result<
        (
            super::subscribed::JoiningSubscription,
            crate::serve::RetainedTrack,
        ),
        ServeError,
    > {
        let subscription = self
            .subscribeds
            .lock()
            .map_err(|_| ServeError::internal_ctx("joining subscription registry unavailable"))?
            .get(&request_id)
            .cloned()
            .ok_or(ServeError::Closed(
                RequestErrorCode::InvalidJoiningRequestId as u64,
            ))?;
        subscription
            .wait_for_joining_fetch()
            .await
            .map_err(|error| ServeError::Closed(error.request_error_code() as u64))
    }

    /// Send a PUBLISH_NAMESPACE for a namespace and serve tracks using the provided
    /// [`TracksReader`]. Blocks until the namespace is unannounced or an error occurs.
    ///
    /// Draft-19: sends PUBLISH_NAMESPACE on a new bidi request stream and reads
    /// responses from the same stream.
    pub async fn publish_namespace(&mut self, tracks: TracksReader) -> Result<(), SessionError> {
        let publish = self
            .publish_namespace_open(tracks.namespace.clone())
            .await?;
        publish
            .accepted_with_timeout(DEFAULT_PUBLISH_NAMESPACE_ACCEPTANCE_TIMEOUT)
            .await
            .map_err(ServeError::from)?;
        publish.serve(tracks).await
    }

    /// Open a long-lived PUBLISH_NAMESPACE request without assuming acceptance.
    ///
    /// The returned handle owns the request-stream send direction. Call
    /// [`PublishNamespace::accepted`] or
    /// [`PublishNamespace::accepted_with_timeout`] before serving tracks.
    pub async fn publish_namespace_open(
        &mut self,
        namespace: TrackNamespace,
    ) -> Result<PublishNamespace, SessionError> {
        let request_lease = Arc::new(
            self.request_capacity
                .try_acquire(RequestDirection::Outbound, RequestClass::PublishNamespace)?,
        );
        // Phase 1: allocate under lock, release before any await.
        let (mut publish_ns, wire_msg, request_id) = {
            let mut namespaces = self
                .publish_namespaces
                .lock()
                .map_err(|_| SessionError::Internal)?;

            if namespaces.contains_key(&namespace) {
                return Err(ServeError::Duplicate.into());
            }

            let request_id = self.request_id.allocate()?;
            let (send, recv) =
                PublishNamespace::new(self.clone(), request_id, namespace.clone(), request_lease);
            namespaces.insert(namespace.clone(), recv);
            let wire_msg: Message = send.wire_message().into();
            (send, wire_msg, request_id)
        };
        // Lock released here.

        // Phase 2: open bidi stream and send (async, no lock held).
        // If open_bi fails, remove the entry we inserted in Phase 1.
        let (send_stream, recv_stream) = match self.webtransport.open_bi().await {
            Ok(streams) => streams,
            Err(e) => {
                if let Ok(mut ns) = self.publish_namespaces.lock() {
                    ns.remove(&namespace);
                }
                return Err(e.into());
            }
        };
        let mut writer = super::Writer::new(send_stream);
        if let Err(e) = writer.encode(&wire_msg).await {
            if let Ok(mut ns) = self.publish_namespaces.lock() {
                ns.remove(&namespace);
            }
            return Err(e);
        }
        let (response_cancel, mut response_cancelled) = tokio::sync::oneshot::channel();
        publish_ns.attach_request_stream(writer, response_cancel);

        // Spawn a reader task for responses on this bidi stream.
        // Draft-19: responses omit Request ID (the stream identity provides it).
        // Handle is sent to Session::run via bidi_task_tx; dropped on session exit.
        let mut this = self.clone();
        let bidi_request_id = request_id;
        let response_guard = PublishNamespaceResponseGuard {
            publisher: this.clone(),
            request_id: bidi_request_id,
        };
        let handle = tokio::spawn(async move {
            let _response_guard = response_guard;
            let mut reader = super::Reader::new(recv_stream);
            loop {
                let response = tokio::select! {
                    _ = &mut response_cancelled => {
                        reader.stop(Session::REQUEST_STREAM_CANCELLED);
                        break;
                    }
                    response = Session::decode_bidi_response(
                        &mut reader,
                        bidi_request_id,
                        super::RequestKind::PublishNamespace,
                    ) => response,
                };
                match response {
                    Ok(msg) => {
                        let terminal = matches!(&msg, Message::RequestError(_));
                        let Ok(sub_msg) = TryInto::<message::Subscriber>::try_into(msg) else {
                            tracing::warn!(
                                bidi_request_id,
                                "unexpected response on PUBLISH_NAMESPACE request stream"
                            );
                            break;
                        };
                        if let Err(error) = this.recv_message(sub_msg) {
                            tracing::warn!(%error, "error handling bidi response");
                            break;
                        }
                        if terminal {
                            break;
                        }
                    }
                    Err(error) => {
                        tracing::debug!(%error, bidi_request_id, "bidi response reader ended");
                        break;
                    }
                }
            }
        });
        if let Err(error) = self.bidi_task_tx.send(handle) {
            error.abort_and_wait().await;
            if let Ok(mut namespaces) = self.publish_namespaces.lock() {
                namespaces.remove(&namespace);
            }
            return Err(SessionError::Internal);
        }

        Ok(publish_ns)
    }

    pub(super) async fn serve_publish_namespace(
        publish_ns: &PublishNamespace,
        tracks: TracksReader,
    ) -> Result<bool, SessionError> {
        let mut publisher_fetch = publish_ns.publisher();
        let mut subscribe_tasks = FuturesUnordered::new();
        let mut status_tasks = FuturesUnordered::new();
        let mut fetch_tasks = FuturesUnordered::new();
        let mut subscribe_done = false;
        let mut status_done = false;
        let mut tracks_done = false;

        loop {
            tokio::select! {
                () = tracks.closed(), if !tracks_done => {
                    tracks_done = true;
                    subscribe_done = true;
                    status_done = true;
                },
                closed = publish_ns.closed(), if !tracks_done => {
                    closed?;
                    return Ok(false);
                },
                res = publish_ns.subscribed(), if !subscribe_done => {
                    match res? {
                        Some(subscribed) => {
                            let tracks = tracks.clone();
                            subscribe_tasks.push(async move {
                                let info = subscribed.info.clone();
                                if let Err(err) = Self::serve_subscribe(subscribed, tracks).await {
                                    tracing::warn!(
                                        subscribe_info = ?info,
                                        error = %err,
                                        "failed serving subscribe"
                                    );
                                }
                            });
                        }
                        None => subscribe_done = true,
                    }
                },
                res = publish_ns.track_status_requested(), if !status_done => {
                    match res? {
                        Some(status) => {
                            let tracks = tracks.clone();
                            status_tasks.push(async move {
                                let request_msg = status.request_msg.clone();
                                if let Err(err) = Self::serve_track_status(status, tracks).await {
                                    tracing::warn!(
                                        request = ?request_msg,
                                        error = %err,
                                        "failed serving track status request"
                                    );
                                }
                            });
                        }
                        None => status_done = true,
                    }
                },
                Some(fetch) = publisher_fetch.fetch_requested(), if !tracks_done => {
                    fetch_tasks.push(async move {
                        let id = fetch.id();
                        if let Err(error) = fetch.serve().await {
                            tracing::warn!(request_id = id, %error, "failed serving Joining FETCH");
                        }
                    });
                },
                Some(res) = subscribe_tasks.next() => res,
                Some(res) = status_tasks.next() => res,
                Some(()) = fetch_tasks.next() => {},
                else => return Ok(tracks_done),
            }
        }
    }

    /// Publish one exact track and serve it until the track or peer closes.
    pub async fn publish(&mut self, track: TrackReader) -> Result<(), SessionError> {
        self.publish_open(track).await?.serve().await
    }

    /// Open a publisher-initiated subscription for one exact track.
    ///
    /// The returned handle owns the track and retains the request-stream send
    /// direction until it writes `PUBLISH_DONE` and FIN, as required by
    /// draft-19.
    pub async fn publish_open(&mut self, track: TrackReader) -> Result<Published, SessionError> {
        let request_lease = Arc::new(
            self.request_capacity
                .try_acquire(RequestDirection::Outbound, RequestClass::Publish)?,
        );
        let request_id = self.request_id.allocate()?;
        // Request IDs are unique across both roles in a session; using the ID
        // as the alias therefore cannot collide with aliases assigned to
        // inbound SUBSCRIBE requests (which use the peer's opposite parity).
        let track_alias = request_id;
        let largest_location = track.largest_location();
        let mut params = KeyValuePairs::default();
        params.set_forward(true);
        if let Some(largest) = largest_location {
            params.set_largest_object(largest)?;
        }
        let publish = message::Publish {
            id: request_id,
            track_namespace: track.namespace.clone(),
            track_name: track.name.clone(),
            track_alias,
            params,
            track_extensions: Default::default(),
        };
        let (subscription, recv) =
            Subscribed::new_published(self.clone(), &publish, self.mlog.clone(), request_lease)?;
        self.published
            .lock()
            .map_err(|_| SessionError::Internal)?
            .insert(request_id, recv);

        let (send_stream, recv_stream) = match self.webtransport.open_bi().await {
            Ok(streams) => streams,
            Err(err) => {
                self.reject_published_locally(
                    request_id,
                    ServeError::internal_ctx(err.to_string()),
                );
                return Err(err.into());
            }
        };
        let mut writer = super::Writer::new(send_stream);
        if let Err(err) = writer.encode(&Message::Publish(publish.clone())).await {
            self.reject_published_locally(
                request_id,
                ServeError::internal_ctx("failed to write PUBLISH request"),
            );
            return Err(err);
        }

        let (terminal_tx, terminal_rx) =
            tokio::sync::mpsc::channel(self.request_capacity.limits().max_response_commands);
        let previous = self
            .bidi_response_map
            .lock()
            .map_err(|_| SessionError::Internal)?
            .insert(request_id, terminal_tx);
        if previous.is_some() {
            self.reject_published_locally(request_id, ServeError::Duplicate);
            return Err(SessionError::InvalidRequestId);
        }

        let mut this = self.clone();
        let map = self.bidi_response_map.clone();
        let response_guard = PublishResponseGuard {
            publisher: this.clone(),
            request_id,
            responses: map.clone(),
        };
        let handle = tokio::spawn(async move {
            let _response_guard = response_guard;
            let result = Self::run_publish_request_stream(
                request_id,
                writer,
                super::Reader::new(recv_stream),
                terminal_rx,
                &mut this,
            )
            .await;
            if let Ok(mut responses) = map.lock() {
                responses.remove(&request_id);
            }
            if let Err(err) = result {
                tracing::warn!(request_id, error = %err, "outbound PUBLISH request stream failed");
                let serve_error = if err.is_request_stream_cancelled() {
                    ServeError::Cancel
                } else {
                    ServeError::internal_ctx(err.to_string())
                };
                this.reject_published_locally(request_id, serve_error);
            }
        });
        if let Err(error) = self.bidi_task_tx.send(handle) {
            error.abort_and_wait().await;
            self.bidi_response_map
                .lock()
                .map_err(|_| SessionError::Internal)?
                .remove(&request_id);
            self.reject_published_locally(
                request_id,
                ServeError::internal_ctx("session request task collector closed"),
            );
            return Err(SessionError::Internal);
        }

        let info = PublishedInfo {
            id: request_id,
            track_namespace: publish.track_namespace,
            track_name: publish.track_name,
            track_alias,
            largest_location,
        };
        Ok(Published::new(subscription, track, info))
    }

    async fn run_publish_request_stream(
        request_id: u64,
        mut writer: super::Writer,
        mut reader: super::Reader,
        mut terminal_rx: tokio::sync::mpsc::Receiver<BidiCommand>,
        publisher: &mut Publisher,
    ) -> Result<(), SessionError> {
        let mut response_open = true;
        let mut accepted = false;
        let mut update_credits = RequestUpdateCredits::new(Session::DEFAULT_MAX_REQUEST_UPDATES);

        loop {
            let event = tokio::select! {
                response = async {
                    if response_open {
                        if reader.done().await? {
                            return Ok(None);
                        }
                        Session::decode_publish_response(&mut reader, request_id)
                            .await
                            .map(Some)
                    } else {
                        std::future::pending::<Result<Option<Message>, SessionError>>().await
                    }
                } => PublishRequestStreamEvent::Response(response),
                command = terminal_rx.recv() => PublishRequestStreamEvent::Command(command),
                stopped = writer.stopped() => PublishRequestStreamEvent::SendStopped(stopped),
            };

            match event {
                PublishRequestStreamEvent::Response(response) => match response? {
                    Some(Message::RequestOk(ok)) if !accepted => {
                        publisher.recv_message(message::Subscriber::RequestOk(ok))?;
                        accepted = true;
                    }
                    Some(Message::RequestError(error)) if !accepted => {
                        publisher.recv_message(message::Subscriber::RequestError(error))?;
                        writer.finish();
                        tokio::task::yield_now().await;
                        return Ok(());
                    }
                    Some(Message::RequestUpdate(update)) if accepted => {
                        publisher.request_id.validate_incoming(update.id)?;
                        update_credits.receive()?;
                        let apply = publisher.apply_publish_update(request_id, &update);
                        let response = match &apply {
                            Ok(()) => Message::RequestOk(message::RequestOk {
                                id: update.id,
                                params: Default::default(),
                                track_properties: Default::default(),
                            }),
                            Err(error) => Message::RequestError(message::RequestError {
                                id: update.id,
                                error_code: RequestErrorCode::NotSupported as u64,
                                retry_interval: 0,
                                reason: crate::coding::ReasonPhrase(error.to_string()),
                                redirect: None,
                            }),
                        };
                        Session::encode_bidi_response(&mut writer, &response).await?;
                        update_credits.respond();
                        if let Err(error) = apply {
                            let mut published = publisher
                                    .drop_published(request_id)
                                    .ok_or_else(|| {
                                        SessionError::ProtocolViolation(format!(
                                            "failed update targeted inactive PUBLISH request {request_id}"
                                        ))
                                    })?;
                            published.recv_update_failed()?;
                            tracing::debug!(
                                request_id,
                                error = %error,
                                "waiting for PUBLISH media streams before UPDATE_FAILED terminal"
                            );
                        }
                    }
                    Some(other) => {
                        return Err(SessionError::ProtocolViolation(format!(
                            "unexpected {} on outbound PUBLISH response direction",
                            other.name()
                        )));
                    }
                    None if accepted => response_open = false,
                    None => {
                        return Err(SessionError::ProtocolViolation(
                            "PUBLISH response direction closed before REQUEST_OK or REQUEST_ERROR"
                                .to_string(),
                        ));
                    }
                },
                PublishRequestStreamEvent::Command(command) => {
                    let command = command.ok_or(SessionError::Internal)?;
                    let BidiCommand::Send(terminal) = command else {
                        if let BidiCommand::Cancel(code) = command {
                            writer.reset(code);
                            reader.stop(code);
                            return Ok(());
                        }
                        return Err(SessionError::ProtocolViolation(
                            "local reverse REQUEST_UPDATE was routed to an outbound PUBLISH"
                                .to_string(),
                        ));
                    };
                    if !matches!(terminal, Message::PublishDone(_)) {
                        return Err(SessionError::ProtocolViolation(format!(
                            "{} cannot terminate an outbound PUBLISH request",
                            terminal.name()
                        )));
                    }
                    Session::encode_bidi_response(&mut writer, &terminal).await?;
                    writer.finish();
                    tokio::task::yield_now().await;
                    return Ok(());
                }
                PublishRequestStreamEvent::SendStopped(stopped) => {
                    match stopped? {
                        Some(code) => {
                            tracing::debug!(
                                request_id,
                                stop_code = code,
                                "peer cancelled outbound PUBLISH with STOP_SENDING"
                            );
                        }
                        None => {
                            tracing::debug!(
                                request_id,
                                "outbound PUBLISH send direction closed before terminal"
                            );
                        }
                    }
                    publisher.reject_published_locally(request_id, ServeError::Cancel);
                    reader.stop(Session::REQUEST_STREAM_CANCELLED);
                    return Ok(());
                }
            }
        }
    }

    pub(super) fn cancel_request_stream(&mut self, id: u64, code: u32) {
        let command = self
            .bidi_response_map
            .lock()
            .ok()
            .and_then(|streams| streams.get(&id).cloned());
        if let Some(command) = command {
            let _ = command.try_send(BidiCommand::Cancel(code));
        }
        if let Some(mut published) = self.drop_published(id) {
            let _ = published.recv_error(ServeError::Cancel);
        }
    }

    fn apply_publish_update(
        &mut self,
        id: u64,
        update: &message::RequestUpdate,
    ) -> Result<(), SessionError> {
        if update.params.0.iter().any(|pair| pair.key != 0x10) {
            return Err(SessionError::ProtocolViolation(
                "PUBLISH REQUEST_UPDATE contained unsupported parameters".to_string(),
            ));
        }
        let forward = update.params.forward()?.ok_or_else(|| {
            SessionError::ProtocolViolation(
                "PUBLISH REQUEST_UPDATE omitted the FORWARD parameter".to_string(),
            )
        })?;
        let mut published = self.published.lock().map_err(|_| SessionError::Internal)?;
        let recv = published.get_mut(&id).ok_or_else(|| {
            SessionError::ProtocolViolation(format!(
                "REQUEST_UPDATE targeted inactive PUBLISH request {id}"
            ))
        })?;
        recv.recv_forward_update(forward)?;
        Ok(())
    }

    fn reject_published_locally(&mut self, id: u64, err: ServeError) {
        if let Some(mut published) = self.drop_published(id) {
            let _ = published.recv_error(err);
        }
    }

    pub async fn serve_subscribe(
        subscribed: Subscribed,
        mut tracks: TracksReader,
    ) -> Result<(), SessionError> {
        if let Some(track) = tracks.subscribe(
            subscribed.info.track_namespace.clone(),
            &subscribed.info.track_name,
        ) {
            subscribed.serve(track).await?;
        } else {
            let namespace = subscribed.info.track_namespace.clone();
            let name = subscribed.info.track_name.clone();
            subscribed.close(ServeError::not_found_ctx(format!(
                "track '{}/{}' not found in tracks",
                namespace, name
            )))?;
        }

        Ok(())
    }

    pub async fn serve_track_status(
        track_status_request: TrackStatusRequested,
        mut tracks: TracksReader,
    ) -> Result<(), SessionError> {
        let track = tracks
            .subscribe(
                track_status_request.request_msg.track_namespace.clone(),
                &track_status_request.request_msg.track_name,
            )
            .ok_or_else(|| {
                ServeError::not_found_ctx(format!(
                    "track '{}/{}' not found for track_status",
                    track_status_request.request_msg.track_namespace,
                    track_status_request.request_msg.track_name
                ))
            })?;

        track_status_request.respond_ok(&track)?;

        Ok(())
    }

    /// Returns the next subscription that did not match any active PUBLISH_NAMESPACE.
    pub async fn subscribed(&mut self) -> Option<Subscribed> {
        self.unknown_subscribed.pop().await
    }

    /// Return the next inbound namespace-discovery request.
    pub async fn subscribed_namespace(&mut self) -> Option<SubscribedNamespace> {
        self.unknown_subscribed_namespace.pop().await
    }

    /// Return the next supported inbound Relative Joining FETCH.
    pub async fn fetch_requested(&mut self) -> Option<FetchRequested> {
        self.unknown_fetch_requested.pop().await
    }

    /// Returns the next TRACK_STATUS request that did not match any active PUBLISH_NAMESPACE.
    pub async fn track_status_requested(&mut self) -> Option<TrackStatusRequested> {
        self.unknown_track_status_requested.pop().await
    }

    fn add_mlog_event<F>(&self, make_event: F)
    where
        F: FnOnce(f64) -> mlog::Event,
    {
        if let Some(ref mlog) = self.mlog {
            if let Ok(mut mlog) = mlog.lock() {
                let event = make_event(mlog.elapsed_ms());
                let _ = mlog.add_event(event);
            }
        }
    }

    fn log_request_ok_parsed(&self, request_kind: &str, msg: &message::RequestOk) {
        self.add_mlog_event(|time| mlog::events::request_ok_parsed(time, 0, request_kind, msg));
    }

    fn log_request_error_parsed(&self, request_kind: &str, msg: &message::RequestError) {
        self.add_mlog_event(|time| mlog::events::request_error_parsed(time, 0, request_kind, msg));
    }

    fn log_request_error_created(&self, request_kind: &str, msg: &message::RequestError) {
        self.add_mlog_event(|time| mlog::events::request_error_created(time, 0, request_kind, msg));
    }

    pub(super) fn send_request_ok(&mut self, request_kind: &str, msg: message::RequestOk) {
        self.add_mlog_event(|time| mlog::events::request_ok_created(time, 0, request_kind, &msg));
        self.send_message(msg);
    }

    pub(super) fn send_request_error(&mut self, request_kind: &str, msg: message::RequestError) {
        self.log_request_error_created(request_kind, &msg);
        self.send_message(msg);
    }

    pub(crate) fn recv_message(&mut self, msg: message::Subscriber) -> Result<(), SessionError> {
        match msg {
            message::Subscriber::Subscribe(msg) => {
                let lease = Arc::new(
                    self.request_capacity
                        .try_acquire(RequestDirection::Inbound, RequestClass::Subscribe)?,
                );
                self.recv_subscribe(msg, lease)?;
            }
            message::Subscriber::RequestUpdate(_) => {
                return Err(SessionError::ProtocolViolation(
                    "REQUEST_UPDATE was not associated with a request stream".to_string(),
                ));
            }
            // Draft-16: REQUEST_OK from subscriber is acceptance of PUBLISH_NAMESPACE.
            message::Subscriber::RequestOk(msg) => {
                if !self.recv_publish_ok(&msg)? {
                    self.recv_publish_namespace_ok(msg)?;
                }
            }
            // Draft-16: REQUEST_ERROR from subscriber is rejection of PUBLISH_NAMESPACE.
            message::Subscriber::RequestError(msg) => {
                if !self.recv_publish_error(&msg)? {
                    self.recv_publish_namespace_error(msg)?;
                }
            }
            message::Subscriber::Fetch(msg) => {
                let lease = Arc::new(
                    self.request_capacity
                        .try_acquire(RequestDirection::Inbound, RequestClass::Fetch)?,
                );
                self.recv_fetch(msg, lease)?;
            }
            message::Subscriber::TrackStatus(msg) => {
                let lease = Arc::new(
                    self.request_capacity
                        .try_acquire(RequestDirection::Inbound, RequestClass::TrackStatus)?,
                );
                self.recv_track_status(msg, lease)?;
            }
            message::Subscriber::SubscribeNamespace(msg) => {
                let lease = Arc::new(
                    self.request_capacity
                        .try_acquire(RequestDirection::Inbound, RequestClass::Subscribe)?,
                );
                self.recv_subscribe_namespace(msg, lease)?;
            }
            // SUBSCRIBE_TRACKS is wire-supported in draft-19, but automatic
            // PUBLISH fanout is a later session-layer tranche.
            message::Subscriber::SubscribeTracks(msg) => {
                self.send_not_supported(msg.id, "subscribe_tracks");
            }
        }

        Ok(())
    }

    /// Dispatch the first message on a peer-opened request stream while
    /// attaching its already-acquired logical request lease to retained state.
    pub(super) fn recv_request_message(
        &mut self,
        msg: message::Subscriber,
        request_lease: Arc<RequestLease>,
    ) -> Result<(), SessionError> {
        match msg {
            message::Subscriber::Subscribe(msg) => self.recv_subscribe(msg, request_lease),
            message::Subscriber::Fetch(msg) => self.recv_fetch(msg, request_lease),
            message::Subscriber::TrackStatus(msg) => self.recv_track_status(msg, request_lease),
            message::Subscriber::SubscribeNamespace(msg) => {
                self.recv_subscribe_namespace(msg, request_lease)
            }
            other => self.recv_message(other),
        }
    }

    pub(crate) fn recv_request_update(
        &mut self,
        initial_request_id: u64,
        update: message::RequestUpdate,
    ) -> Result<(), SessionError> {
        let fetch_found = self
            .fetches
            .lock()
            .map_err(|_| SessionError::Internal)?
            .contains_key(&initial_request_id);
        if fetch_found {
            self.send_not_supported(update.id, "fetch_request_update");
            return Ok(());
        }

        let found = {
            let mut subscribeds = self
                .subscribeds
                .lock()
                .map_err(|_| SessionError::Internal)?;
            if let Some(subscribed) = subscribeds.get_mut(&initial_request_id) {
                subscribed.recv_update_failed()?;
                true
            } else {
                false
            }
        };

        if !found {
            return Err(SessionError::ProtocolViolation(format!(
                "REQUEST_UPDATE targeted inactive request stream {}",
                initial_request_id
            )));
        }

        self.send_not_supported(update.id, "request_update");
        Ok(())
    }

    /// Send REQUEST_ERROR NOT_SUPPORTED for an incoming request we do not implement.
    ///
    /// Draft-16 §4: limited endpoints SHOULD respond with NOT_SUPPORTED rather
    /// than ignoring unsupported request types.
    fn send_not_supported(&mut self, request_id: u64, request_kind: &str) {
        tracing::debug!(
            target: "moq_transport::control",
            request_id,
            "sending REQUEST_ERROR NOT_SUPPORTED for unimplemented request"
        );
        self.send_request_error(
            request_kind,
            message::RequestError {
                id: request_id,
                error_code: RequestErrorCode::NotSupported as u64,
                retry_interval: 0,
                reason: crate::coding::ReasonPhrase("not supported".to_string()),
                redirect: None,
            },
        );
    }

    /// Handle REQUEST_OK from subscriber — acceptance of our PUBLISH_NAMESPACE.
    fn recv_publish_namespace_ok(&mut self, msg: message::RequestOk) -> Result<(), SessionError> {
        self.log_request_ok_parsed("publish_namespace", &msg);
        // The publish_namespaces map is keyed by namespace; we must search by request_id.
        // TODO(itzmanish): maintain a second index keyed by request_id to make this O(1).
        let mut namespaces = self
            .publish_namespaces
            .lock()
            .map_err(|_| SessionError::Internal)?;
        if let Some(entry) = namespaces.iter_mut().find(|(_k, v)| v.request_id == msg.id) {
            entry.1.recv_ok()?;
        }

        Ok(())
    }

    fn recv_publish_ok(&mut self, msg: &message::RequestOk) -> Result<bool, SessionError> {
        let mut published = self.published.lock().map_err(|_| SessionError::Internal)?;
        let Some(recv) = published.get_mut(&msg.id) else {
            return Ok(false);
        };
        self.log_request_ok_parsed("publish", msg);
        recv.recv_publish_ok(msg)?;
        Ok(true)
    }

    fn recv_publish_error(&mut self, msg: &message::RequestError) -> Result<bool, SessionError> {
        let Some(mut recv) = self.drop_published(msg.id) else {
            return Ok(false);
        };
        self.log_request_error_parsed("publish", msg);
        recv.recv_error(ServeError::Closed(msg.error_code))?;
        Ok(true)
    }

    /// Handle REQUEST_ERROR from subscriber — rejection of our PUBLISH_NAMESPACE.
    fn recv_publish_namespace_error(
        &mut self,
        msg: message::RequestError,
    ) -> Result<(), SessionError> {
        self.log_request_error_parsed("publish_namespace", &msg);
        if let Some(recv) = self.drop_publish_namespace(msg.id) {
            recv.release_request_lease();
            recv.recv_rejected(PublishNamespaceRejection::from(msg))?;
        }
        Ok(())
    }

    fn recv_publish_namespace_response_stream_closed(&mut self, id: u64) {
        if let Some(mut recv) = self.drop_publish_namespace(id) {
            recv.recv_response_stream_closed();
            recv.release_request_lease();
        }
    }

    fn recv_subscribe(
        &mut self,
        msg: message::Subscribe,
        request_lease: Arc<RequestLease>,
    ) -> Result<(), SessionError> {
        let namespace = msg.track_namespace.clone();

        let subscribed = {
            let mut subscribeds = self
                .subscribeds
                .lock()
                .map_err(|_| SessionError::Internal)?;

            if subscribeds.contains_key(&msg.id) {
                return Err(SessionError::InvalidRequestId);
            }

            let (send, recv) =
                Subscribed::new(self.clone(), msg, self.mlog.clone(), request_lease)?;
            subscribeds.insert(send.info.id, recv);

            send
        };

        // Route to an active PUBLISH_NAMESPACE if present.
        if let Some(ns) = self
            .publish_namespaces
            .lock()
            .map_err(|_| SessionError::Internal)?
            .get_mut(&namespace)
        {
            return ns.recv_subscribe(subscribed).map_err(Into::into);
        }

        // Otherwise, surface it to the application via the unknown queue.
        if let Err(err) = self.unknown_subscribed.push(subscribed) {
            err.close(ServeError::not_found_ctx(format!(
                "unknown_subscribed queue full for namespace {:?}",
                namespace
            )))?;
        }

        Ok(())
    }

    fn recv_subscribe_namespace(
        &mut self,
        msg: message::SubscribeNamespace,
        request_lease: Arc<RequestLease>,
    ) -> Result<(), SessionError> {
        let id = msg.id;
        let prefix = msg.track_namespace_prefix.clone();
        let (request, recv) = SubscribedNamespace::new(self.clone(), msg, request_lease);
        {
            let mut requests = self
                .subscribed_namespaces
                .lock()
                .map_err(|_| SessionError::Internal)?;
            match requests.try_insert(id, prefix, recv) {
                Ok(()) => {}
                Err(SubscribedNamespaceInsertError::DuplicateRequestId) => {
                    return Err(SessionError::InvalidRequestId);
                }
                Err(SubscribedNamespaceInsertError::PrefixOverlap) => {
                    drop(requests);
                    request.close(ServeError::Closed(RequestErrorCode::PrefixOverlap as u64));
                    return Ok(());
                }
            }
        }

        if let Err(request) = self.unknown_subscribed_namespace.push(request) {
            request.close(ServeError::Closed(RequestErrorCode::ExcessiveLoad as u64));
            self.cleanup_inbound_subscribe_namespace(id);
        }
        Ok(())
    }

    fn recv_track_status(
        &mut self,
        msg: message::TrackStatus,
        request_lease: Arc<RequestLease>,
    ) -> Result<(), SessionError> {
        let namespace = msg.track_namespace.clone();

        let track_status_requested = TrackStatusRequested::new(self.clone(), msg, request_lease);

        if let Some(ns) = self
            .publish_namespaces
            .lock()
            .map_err(|_| SessionError::Internal)?
            .get_mut(&namespace)
        {
            return ns
                .recv_track_status_requested(track_status_requested)
                .map_err(Into::into);
        }

        if let Err(mut err) = self
            .unknown_track_status_requested
            .push(track_status_requested)
        {
            err.respond_error(RequestErrorCode::InternalError as u64, "internal error")?;
        }

        Ok(())
    }

    fn recv_fetch(
        &mut self,
        msg: message::Fetch,
        request_lease: Arc<RequestLease>,
    ) -> Result<(), SessionError> {
        // Reject unsupported variants at the request boundary. They are
        // request-scoped capability errors, never connection failures.
        if let Err(error) = super::fetch::validate_joining_request(&msg) {
            let code = match error {
                SessionError::Serve(ServeError::NotImplemented(_))
                | SessionError::Serve(ServeError::NotImplementedWithId(_, _)) => {
                    RequestErrorCode::NotSupported
                }
                _ => RequestErrorCode::InvalidRange,
            };
            self.send_request_error(
                "fetch",
                message::RequestError::new(msg.id, code, 0, &error.to_string()),
            );
            request_lease.release();
            return Ok(());
        }

        let id = msg.id;
        let (requested, recv) = FetchRequested::new(self.clone(), msg, request_lease);
        {
            let mut fetches = self.fetches.lock().map_err(|_| SessionError::Internal)?;
            if fetches.insert(id, recv).is_some() {
                return Err(SessionError::InvalidRequestId);
            }
        }
        if let Err(mut requested) = self.unknown_fetch_requested.push(requested) {
            requested.reject_with_retry(
                RequestErrorCode::ExcessiveLoad,
                super::fetch::FETCH_OVERLOAD_RETRY_INTERVAL,
                "FETCH queue capacity exhausted",
            )?;
            self.cleanup_inbound_fetch(id);
        }
        Ok(())
    }

    /// Pre-send hook: clean up internal state when terminal publisher messages are enqueued.
    fn act_on_message_to_send<T: Into<message::Publisher>>(
        &mut self,
        msg: T,
    ) -> message::Publisher {
        let msg = msg.into();
        if let message::Publisher::PublishDone(m) = &msg {
            self.drop_subscribe(m.id);
            self.drop_published(m.id);
        }
        msg
    }

    /// Enqueue a control message for sending (fire-and-forget).
    pub(super) fn send_message<T: Into<message::Publisher> + Into<Message>>(&mut self, msg: T) {
        let msg = self.act_on_message_to_send(msg);
        self.outgoing.push(msg.into()).ok();
    }

    /// Send a stream-associated response that has no embedded request ID.
    ///
    /// NAMESPACE and NAMESPACE_DONE are associated by the request stream, so
    /// they bypass the shared outgoing queue's ID-based response routing.
    pub(super) fn send_associated_message(
        &mut self,
        request_id: u64,
        msg: Message,
    ) -> Result<(), SessionError> {
        let command = self
            .bidi_response_map
            .lock()
            .map_err(|_| SessionError::Internal)?
            .get(&request_id)
            .cloned()
            .ok_or_else(|| SessionError::Serve(ServeError::Cancel))?;
        command
            .try_send(BidiCommand::Send(msg))
            .map_err(|_| SessionError::Serve(ServeError::Cancel))
    }

    /// Enqueue a control message and wait until it has been dequeued for sending.
    pub(super) async fn send_message_and_wait<T: Into<message::Publisher> + Into<Message>>(
        &mut self,
        msg: T,
    ) {
        let msg = self.act_on_message_to_send(msg);
        self.outgoing
            .push_and_wait_until_popped(msg.into())
            .await
            .ok();
    }

    pub(super) fn drop_subscribe(&mut self, id: u64) {
        let _ = self.remove_subscribe(id);
    }

    fn remove_subscribe(&mut self, id: u64) -> Result<(), SessionError> {
        self.subscribeds
            .lock()
            .map_err(|_| SessionError::Internal)?
            .remove(&id);
        Ok(())
    }

    /// Remove every retained representation of a peer-opened SUBSCRIBE.
    pub(super) fn cleanup_inbound_subscribe(&mut self, id: u64) {
        let recv = self
            .subscribeds
            .lock()
            .ok()
            .and_then(|mut subscribes| subscribes.remove(&id));
        if let Some(mut recv) = recv {
            let _ = recv.recv_error(ServeError::Cancel);
        }
        self.unknown_subscribed
            .remove_where(|subscribe| subscribe.info.id == id);
        if let Ok(mut namespaces) = self.publish_namespaces.lock() {
            for namespace in namespaces.values_mut() {
                namespace.remove_subscribe(id);
            }
        }
    }

    /// Remove every retained representation of a peer-opened
    /// SUBSCRIBE_NAMESPACE and wake the application-facing handle.
    pub(super) fn cleanup_inbound_subscribe_namespace(&mut self, id: u64) {
        self.unknown_subscribed_namespace
            .remove_where(|request| request.info.request_id == id);
        if let Some(mut recv) = self
            .subscribed_namespaces
            .lock()
            .ok()
            .and_then(|mut requests| requests.remove(id))
        {
            recv.recv_closed();
        }
    }

    /// Remove every queued representation of a peer-opened TRACK_STATUS.
    pub(super) fn cleanup_inbound_track_status(&mut self, id: u64) {
        self.unknown_track_status_requested
            .remove_where(|request| request.request_msg.id == id);
        if let Ok(mut namespaces) = self.publish_namespaces.lock() {
            for namespace in namespaces.values_mut() {
                namespace.remove_track_status(id);
            }
        }
    }

    /// Remove every retained representation of a peer-opened FETCH.
    pub(super) fn cleanup_inbound_fetch(&mut self, id: u64) {
        if let Some(fetch) = self.fetches.lock().ok().and_then(|mut map| map.remove(&id)) {
            fetch.cancel();
        }
        self.unknown_fetch_requested
            .remove_where(|request| request.id() == id);
    }

    pub(super) fn drop_publish_namespace(&mut self, id: u64) -> Option<PublishNamespaceRecv> {
        if let Ok(mut ns) = self.publish_namespaces.lock() {
            let key = ns
                .iter()
                .find(|(_k, v)| v.request_id == id)
                .map(|(k, _)| k.clone());
            if let Some(key) = key {
                return ns.remove(&key);
            }
        }
        None
    }

    pub(super) fn drop_published(&mut self, id: u64) -> Option<SubscribedRecv> {
        let recv = self.published.lock().ok()?.remove(&id);
        if let Some(recv) = &recv {
            recv.release_request_lease();
        }
        recv
    }

    pub(super) async fn open_uni(&mut self) -> Result<web_transport::SendStream, SessionError> {
        Ok(self.webtransport.open_uni().await?)
    }

    pub(super) async fn send_datagram(&mut self, data: bytes::Bytes) -> Result<(), SessionError> {
        Ok(self.webtransport.send_datagram(data).await?)
    }
}

#[cfg(test)]
mod tests {
    use crate::coding::TrackNamespacePrefix;

    use super::{SubscribedNamespaceInsertError, SubscribedNamespaceRegistry};

    fn prefix(value: &str) -> TrackNamespacePrefix {
        TrackNamespacePrefix::from_utf8_path(value)
    }

    #[test]
    fn namespace_registry_rejects_overlapping_active_prefixes() {
        let mut requests = SubscribedNamespaceRegistry::default();
        requests.try_insert(1, prefix("tenant/live"), ()).unwrap();

        for (request_id, candidate) in [
            (3, "tenant"),
            (5, "tenant/live"),
            (7, "tenant/live/audio"),
            (9, ""),
        ] {
            assert_eq!(
                requests.try_insert(request_id, prefix(candidate), ()),
                Err(SubscribedNamespaceInsertError::PrefixOverlap)
            );
        }

        requests
            .try_insert(11, prefix("tenant/archive"), ())
            .unwrap();
        requests.try_insert(13, prefix("other/live"), ()).unwrap();
    }

    #[test]
    fn namespace_registry_releases_prefix_for_reuse_after_close() {
        let mut requests = SubscribedNamespaceRegistry::default();
        requests.try_insert(1, prefix("tenant/live"), ()).unwrap();
        assert_eq!(
            requests.try_insert(3, prefix("tenant/live/audio"), ()),
            Err(SubscribedNamespaceInsertError::PrefixOverlap)
        );

        assert_eq!(requests.remove(1), Some(()));
        requests
            .try_insert(3, prefix("tenant/live/audio"), ())
            .unwrap();
    }

    #[test]
    fn namespace_registry_rejects_duplicate_request_ids() {
        let mut requests = SubscribedNamespaceRegistry::default();
        requests.try_insert(1, prefix("tenant/live"), ()).unwrap();
        assert_eq!(
            requests.try_insert(1, prefix("other/live"), ()),
            Err(SubscribedNamespaceInsertError::DuplicateRequestId)
        );
    }
}
