// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-FileCopyrightText: 2023-2024 Luke Curley and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{
    collections::{hash_map, HashMap},
    io,
    sync::{Arc, Mutex},
    time::Duration,
};

use tokio::sync::Notify;

use crate::{
    coding::{Decode, TrackName, TrackNamespace},
    data,
    message::{self, Message},
    mlog,
    serve::{self, ServeError},
};

use crate::watch::Queue;

use super::{
    BidiCommand, BidiResponseMap, BufferedJoinObject, EndOfGroupState, Fetch, FetchRecv,
    FetchedObject, PublishReceived, PublishReceivedRecv, PublishedNamespace,
    PublishedNamespaceRecv, Reader, RequestClass, RequestDirection, RequestId, RequestLease,
    Session, SessionError, SessionRequestCapacity, Subscribe, SubscribeOptions, SubscribeRecv,
    Writer,
};

// Default timeout for waiting for subscribe aliases to become available via SUBSCRIBE_OK (1 second)
const DEFAULT_ALIAS_WAIT_TIME_MS: u64 = 1000;
const MAX_STREAMS_PER_PUBLICATION: u64 = 64;
const PUBLISH_DONE_CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AliasBinding {
    Subscribe(u64),
    Publish(u64),
}

#[derive(Clone, Debug)]
struct AliasEntry {
    full_name: serve::FullTrackName,
    bindings: Vec<AliasBinding>,
}

struct SubscribeResponseGuard {
    subscriber: Subscriber,
    request_id: u64,
}

struct FetchResponseGuard {
    subscriber: Subscriber,
    request_id: u64,
}

impl Drop for FetchResponseGuard {
    fn drop(&mut self) {
        self.subscriber
            .fetch_response_stream_closed(self.request_id);
    }
}

impl Drop for SubscribeResponseGuard {
    fn drop(&mut self) {
        self.subscriber.cleanup_outbound_subscribe(self.request_id);
    }
}

// TODO remove Clone.
#[derive(Clone)]
pub struct Subscriber {
    /// Active inbound PUBLISH_NAMESPACE messages, keyed by namespace.
    published_namespaces: Arc<Mutex<HashMap<TrackNamespace, PublishedNamespaceRecv>>>,

    /// Queue of inbound PUBLISH_NAMESPACE events waiting to be consumed by the application.
    published_namespace_queue: Queue<PublishedNamespace>,

    /// The currently active outbound subscribes, keyed by request id.
    subscribes: Arc<Mutex<HashMap<u64, SubscribeRecv>>>,

    /// Active outbound Joining FETCH requests, keyed by request ID.
    fetches: Arc<Mutex<HashMap<u64, FetchRecv>>>,

    /// Session-scoped aliases. One alias may fan out to multiple requests only
    /// when every request names the exact same track.
    alias_map: Arc<Mutex<HashMap<u64, AliasEntry>>>,

    /// Notify when subscribe alias map is updated
    subscribe_alias_notify: Arc<Notify>,

    /// Active inbound PUBLISH requests, keyed by request ID.
    publishes_received: Arc<Mutex<HashMap<u64, PublishReceivedRecv>>>,

    /// Inbound publications waiting for application policy and registration.
    publish_received_queue: Queue<PublishReceived>,

    /// Shared queue for request-stream responses and session-level messages.
    /// The session send task enforces the draft-19 stream placement.
    outgoing: Queue<Message>,

    /// WebTransport session, used to open bidi streams for requests (draft-19).
    webtransport: web_transport::Session,

    /// Shared with Publisher so all requests within a session use unique IDs.
    /// When we need a new Request Id for sending a request, we can get it from here.
    /// The manager is shared with the Publisher, so the session uses unique request ids
    /// for all requests generated.  If we initiated the QUIC connection then request
    /// IDs start at 0 and increment by 2 (even numbers).  If we accepted an inbound
    /// QUIC connection then request IDs start at 1 and increment by 2 (odd numbers).
    request_id: RequestId,

    /// Optional mlog writer for logging transport events
    mlog: Option<Arc<Mutex<mlog::MlogWriter>>>,

    /// Channel for sending spawned bidi reader task handles to Session::run.
    bidi_task_tx: super::BidiTaskSender,

    /// Request-stream command channels, used for cancellation and reverse updates.
    bidi_response_map: BidiResponseMap,

    /// Shared fail-fast ownership for logical inbound and outbound requests.
    request_capacity: SessionRequestCapacity,
}

impl Subscriber {
    fn same_session(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.subscribes, &other.subscribes)
            && Arc::ptr_eq(&self.fetches, &other.fetches)
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
            published_namespaces: Default::default(),
            published_namespace_queue: Queue::bounded(limits.session_inbound.publish_namespace),
            subscribes: Default::default(),
            fetches: Default::default(),
            alias_map: Default::default(),
            publishes_received: Default::default(),
            publish_received_queue: Queue::bounded(limits.session_inbound.publish),
            outgoing,
            webtransport,
            request_id,
            mlog,
            subscribe_alias_notify: Arc::new(Notify::new()),
            bidi_task_tx,
            bidi_response_map,
            request_capacity,
        }
    }

    pub(super) fn cancel_publish_received(&mut self, request_id: u64, code: u32) {
        self.fail_publish_received(request_id, ServeError::Cancel);
        self.cancel_request_stream(request_id, code);
    }

    pub(super) fn cancel_request_stream(&mut self, request_id: u64, code: u32) {
        let command = self
            .bidi_response_map
            .lock()
            .ok()
            .and_then(|streams| streams.get(&request_id).cloned());
        if let Some(command) = command {
            let _ = command.try_send(BidiCommand::Cancel(code));
        }
    }

    pub(super) async fn update_publish_received(
        &mut self,
        request_id: u64,
        forward: bool,
    ) -> Result<(), SessionError> {
        let update_id = self.request_id.allocate()?;
        let mut params = crate::coding::KeyValuePairs::default();
        params.set_forward(forward);
        let update = message::RequestUpdate {
            id: update_id,
            params,
        };
        let command = self
            .bidi_response_map
            .lock()
            .map_err(|_| SessionError::Internal)?
            .get(&request_id)
            .cloned()
            .ok_or_else(|| {
                SessionError::ProtocolViolation(format!(
                    "PUBLISH request stream {request_id} is no longer active"
                ))
            })?;
        let (completion, completed) = tokio::sync::oneshot::channel();
        command
            .try_send(BidiCommand::RequestUpdate {
                update,
                forward,
                completion,
            })
            .map_err(|_| SessionError::TooManyRequestUpdates)?;
        completed.await.map_err(|_| SessionError::Internal)?
    }

    pub(super) fn set_publish_forward(
        &mut self,
        request_id: u64,
        forward: bool,
    ) -> Result<(), SessionError> {
        let mut publishes = self
            .publishes_received
            .lock()
            .map_err(|_| SessionError::Internal)?;
        let publish = publishes.get_mut(&request_id).ok_or_else(|| {
            SessionError::ProtocolViolation(format!(
                "PUBLISH request stream {request_id} is no longer active"
            ))
        })?;
        publish.set_forward(forward);
        Ok(())
    }

    /// Create an inbound/server QUIC connection, by accepting a bi-directional QUIC stream for control messages.
    pub async fn accept(
        session: web_transport::Session,
        negotiated: super::NegotiatedTransport,
    ) -> Result<(Session, Self), SessionError> {
        let (session, _, subscriber) = Session::accept(session, None, negotiated).await?;
        Ok((session, subscriber.unwrap()))
    }

    pub async fn accept_with_capacity(
        session: web_transport::Session,
        negotiated: super::NegotiatedTransport,
        request_capacity: &super::RequestCapacity,
    ) -> Result<(Session, Self), SessionError> {
        let (session, _, subscriber) =
            Session::accept_with_capacity(session, None, negotiated, request_capacity).await?;
        Ok((session, subscriber.unwrap()))
    }

    /// Create an outbound/client QUIC connection, by opening a bi-directional QUIC stream for control messages.
    pub async fn connect(
        session: web_transport::Session,
        negotiated: super::NegotiatedTransport,
    ) -> Result<(Session, Self), SessionError> {
        let (session, _, subscriber) = Session::connect(session, None, negotiated).await?;
        Ok((session, subscriber))
    }

    pub async fn connect_with_capacity(
        session: web_transport::Session,
        negotiated: super::NegotiatedTransport,
        request_capacity: &super::RequestCapacity,
    ) -> Result<(Session, Self), SessionError> {
        let (session, _, subscriber) =
            Session::connect_with_capacity(session, None, negotiated, request_capacity).await?;
        Ok((session, subscriber))
    }

    /// Wait for the next inbound PUBLISH_NAMESPACE from the peer, if any.
    pub async fn published_namespace(&mut self) -> Option<PublishedNamespace> {
        self.published_namespace_queue.pop().await
    }

    /// Wait for an inbound publisher-initiated subscription.
    pub async fn publish_received(&mut self) -> Option<PublishReceived> {
        self.publish_received_queue.pop().await
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

    /// Allocate the next outbound request ID.
    fn get_next_request_id(&mut self) -> Result<u64, SessionError> {
        self.request_id.allocate()
    }

    /// Open a bidirectional request stream (draft-19 §10), send a request
    /// message, and return a Reader for reading the response on the same stream.
    async fn open_request_stream(&self, msg: &message::Message) -> Result<Reader, SessionError> {
        let (send_stream, recv_stream) = self.webtransport.open_bi().await?;
        let mut writer = Writer::new(send_stream);
        writer.encode(msg).await?;
        Ok(Reader::new(recv_stream))
    }

    fn new_track_status_request(
        &mut self,
        track_namespace: &TrackNamespace,
        track_name: TrackName,
    ) -> Result<(message::TrackStatus, Arc<RequestLease>), SessionError> {
        let request_lease = Arc::new(
            self.request_capacity
                .try_acquire(RequestDirection::Outbound, RequestClass::TrackStatus)?,
        );
        Ok((
            message::TrackStatus {
                id: self.get_next_request_id()?,
                track_namespace: track_namespace.clone(),
                track_name,
                params: Default::default(),
            },
            request_lease,
        ))
    }

    async fn run_track_status_request(
        &self,
        request: message::TrackStatus,
        _request_lease: Arc<RequestLease>,
    ) -> Result<message::RequestOk, SessionError> {
        let request_id = request.id;
        let (send_stream, recv_stream) = self.webtransport.open_bi().await?;
        let mut writer = Writer::new(send_stream);
        writer.encode(&Message::TrackStatus(request)).await?;

        // TRACK_STATUS cannot be updated, so the requester has no further
        // messages to send and can close its direction immediately.
        writer.finish();

        let mut reader = Reader::new(recv_stream);
        let response =
            Session::decode_bidi_response(&mut reader, request_id, super::RequestKind::TrackStatus)
                .await?;

        let result = match response {
            Message::RequestOk(ok) => {
                self.log_request_ok_parsed("track_status", &ok);
                Ok(ok)
            }
            Message::RequestError(error) => {
                self.log_request_error_parsed("track_status", &error);
                Err(SessionError::Serve(Self::request_error_to_serve_error(
                    &error,
                )))
            }
            other => Err(SessionError::ProtocolViolation(format!(
                "unexpected {} on TRACK_STATUS response stream",
                other.name()
            ))),
        };

        // A TRACK_STATUS responder has no subsequent messages. Require the
        // response direction to close so a successful query cannot leave an
        // orphaned request stream behind.
        if !reader.done().await? {
            return Err(SessionError::ProtocolViolation(
                "TRACK_STATUS response stream contained additional messages".to_string(),
            ));
        }

        result
    }

    /// Query the status of a track on a dedicated bidirectional request stream.
    pub async fn track_status_query(
        &mut self,
        track_namespace: &TrackNamespace,
        track_name: impl Into<TrackName>,
    ) -> Result<message::RequestOk, SessionError> {
        let (request, request_lease) =
            self.new_track_status_request(track_namespace, track_name.into())?;
        self.run_track_status_request(request, request_lease).await
    }

    /// Send a fire-and-forget TRACK_STATUS request for source compatibility.
    ///
    /// New callers should use [`Self::track_status_query`] so transport and
    /// request errors are observable.
    pub fn track_status(
        &mut self,
        track_namespace: &TrackNamespace,
        track_name: impl Into<TrackName>,
    ) {
        let (request, request_lease) =
            match self.new_track_status_request(track_namespace, track_name.into()) {
                Ok(request) => request,
                Err(error) => {
                    tracing::warn!(%error, "could not allocate TRACK_STATUS request");
                    return;
                }
            };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::warn!("could not send TRACK_STATUS outside a Tokio runtime");
            return;
        };

        let subscriber = self.clone();
        let request_id = request.id;
        let task = runtime.spawn(async move {
            if let Err(error) = subscriber
                .run_track_status_request(request, request_lease)
                .await
            {
                tracing::warn!(%error, request_id, "TRACK_STATUS request failed");
            }
        });
        if self.bidi_task_tx.send(task).is_err() {
            tracing::warn!(request_id, "TRACK_STATUS task collector is closed");
        }
    }

    /// Subscribe to a track by creating a new subscribe request to the publisher.  Block until subscription is closed.
    pub async fn subscribe(&mut self, track: serve::TrackWriter) -> Result<(), ServeError> {
        let subscribe = self.subscribe_open(track).await?;
        subscribe.closed().await
    }

    /// Subscribe to a track and wait until the publisher acknowledges it.
    ///
    /// Draft-19: sends SUBSCRIBE on a new bidi request stream and reads
    /// the response (REQUEST_OK / REQUEST_ERROR) from the same stream.
    pub async fn subscribe_open(
        &mut self,
        track: serve::TrackWriter,
    ) -> Result<Subscribe, ServeError> {
        self.subscribe_open_with(track, SubscribeOptions::default())
            .await
    }

    /// Subscribe with explicit draft-19 request parameters and wait for the
    /// publisher to acknowledge the request.
    pub async fn subscribe_open_with(
        &mut self,
        track: serve::TrackWriter,
        options: SubscribeOptions,
    ) -> Result<Subscribe, ServeError> {
        self.subscribe_open_with_barrier(track, options, false)
            .await
    }

    async fn subscribe_open_with_barrier(
        &mut self,
        track: serve::TrackWriter,
        options: SubscribeOptions,
        joining_fetch: bool,
    ) -> Result<Subscribe, ServeError> {
        let request_lease = Arc::new(
            self.request_capacity
                .try_acquire(RequestDirection::Outbound, RequestClass::Subscribe)
                .map_err(|error| ServeError::internal_ctx(error.to_string()))?,
        );
        let request_id = self
            .get_next_request_id()
            .map_err(|e| ServeError::internal_ctx(format!("request ID limit: {}", e)))?;
        let (mut send, mut recv) =
            Subscribe::new_with_options(self.clone(), request_id, track, options, request_lease)?;
        if joining_fetch {
            recv.begin_joining_fetch()?;
        }

        // Open a bidi stream and send the SUBSCRIBE message BEFORE
        // registering in the subscribes map — avoids a leaked entry if
        // open_request_stream fails.
        let subscribe_msg: Message = send.wire_message().into();
        let mut response_reader = self
            .open_request_stream(&subscribe_msg)
            .await
            .map_err(|e| {
                ServeError::internal_ctx(format!("failed to open request stream: {}", e))
            })?;

        self.subscribes
            .lock()
            .map_err(|_| {
                tracing::warn!(
                    request_id,
                    "subscribes lock poisoned after bidi stream open; stream will be dropped"
                );
                ServeError::internal_ctx("subscribe lock poisoned")
            })?
            .insert(request_id, recv);

        let (response_cancel, mut response_cancelled) = tokio::sync::oneshot::channel();
        send.attach_response_cancel(response_cancel);

        // Spawn a reader task for bidi stream responses (draft-19).
        // Handle is sent to Session::run via bidi_task_tx; dropped on session exit.
        let mut subscriber_clone = self.clone();
        let response_guard = SubscribeResponseGuard {
            subscriber: subscriber_clone.clone(),
            request_id,
        };
        let handle = tokio::spawn(async move {
            let _response_guard = response_guard;
            loop {
                let response = tokio::select! {
                    _ = &mut response_cancelled => {
                        response_reader.stop(Session::REQUEST_STREAM_CANCELLED);
                        break;
                    }
                    response = Session::decode_bidi_response(
                        &mut response_reader,
                        request_id,
                        super::RequestKind::Subscribe,
                    ) => response,
                };
                match response {
                    Ok(msg) => {
                        if let Ok(pub_msg) = TryInto::<message::Publisher>::try_into(msg) {
                            let terminal = matches!(pub_msg, message::Publisher::PublishDone(_));
                            if let Err(e) = subscriber_clone.recv_message(pub_msg) {
                                tracing::warn!(error = %e, "error handling bidi response");
                                break;
                            }
                            if terminal {
                                subscriber_clone
                                    .await_subscribe_done_cleanup(request_id)
                                    .await;
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::debug!(error = %e, request_id, "bidi response reader ended");
                        break;
                    }
                }
            }
        });
        if let Err(error) = self.bidi_task_tx.send(handle) {
            error.abort_and_wait().await;
            self.remove_subscribe(request_id);
            return Err(ServeError::internal_ctx(
                "session request task collector closed",
            ));
        }

        send.ok().await?;
        Ok(send)
    }

    /// Subscribe to the live edge and atomically join it to a Relative
    /// Joining FETCH. Fetched Objects are emitted first, followed by live
    /// Objects buffered behind the frozen Joining Location.
    pub async fn subscribe_joining(
        &mut self,
        track: serve::TrackWriter,
    ) -> Result<(Subscribe, Fetch), ServeError> {
        let options = SubscribeOptions::default()
            .with_forward(true)
            .with_filter(message::SubscriptionFilter::largest_object())
            .with_group_order(message::GroupOrder::Ascending);
        let subscribe = self
            .subscribe_open_with_barrier(track, options, true)
            .await?;
        let fetch = self.fetch_joining(&subscribe).await?;
        Ok((subscribe, fetch))
    }

    /// Subscribe at the live edge and join retained Objects when the peer
    /// reports an existing Largest Object.
    ///
    /// A newly activated upstream can legitimately acknowledge SUBSCRIBE
    /// before it has observed any Object. In that case there is no valid
    /// Relative Joining FETCH range. The live barrier is released atomically
    /// and `None` is returned instead of failing an otherwise healthy
    /// subscription.
    pub async fn subscribe_joining_or_live(
        &mut self,
        track: serve::TrackWriter,
    ) -> Result<(Subscribe, Option<Fetch>), ServeError> {
        let options = SubscribeOptions::default()
            .with_forward(true)
            .with_filter(message::SubscriptionFilter::largest_object())
            .with_group_order(message::GroupOrder::Ascending);
        let subscribe = self
            .subscribe_open_with_barrier(track, options, true)
            .await?;
        let joining_request_id = subscribe.info.id;
        let has_joining_location = self
            .subscribes
            .lock()
            .map_err(|_| ServeError::internal_ctx("subscribe registry unavailable"))?
            .get(&joining_request_id)
            .ok_or_else(|| {
                ServeError::internal_ctx(
                    "Joining FETCH reference is not active in this Subscriber session",
                )
            })?
            .joining_location()
            .is_some();
        if has_joining_location {
            let fetch = self.fetch_joining(&subscribe).await?;
            return Ok((subscribe, Some(fetch)));
        }

        self.subscribes
            .lock()
            .map_err(|_| ServeError::internal_ctx("subscribe registry unavailable"))?
            .get_mut(&joining_request_id)
            .ok_or_else(|| {
                ServeError::internal_ctx(
                    "Joining FETCH reference is not active in this Subscriber session",
                )
            })?
            .fall_back_to_live_without_fetch()?;
        Ok((subscribe, None))
    }

    /// Start the supported Relative Joining FETCH for an established
    /// subscription in this same session.
    pub async fn fetch_joining(&mut self, joining: &Subscribe) -> Result<Fetch, ServeError> {
        if !self.same_session(joining.subscriber()) {
            return Err(ServeError::internal_ctx(
                "Joining FETCH cannot reference a subscription from another session",
            ));
        }
        let joining_request_id = joining.info.id;
        let joining_location = {
            let mut subscribes = self
                .subscribes
                .lock()
                .map_err(|_| ServeError::internal_ctx("subscribe registry unavailable"))?;
            let subscribe = subscribes.get_mut(&joining_request_id).ok_or_else(|| {
                ServeError::internal_ctx(
                    "Joining FETCH reference is not active in this Subscriber session",
                )
            })?;
            if !subscribe.has_joining_barrier() {
                subscribe.begin_joining_fetch()?;
            }
            subscribe
                .joining_location()
                .ok_or_else(|| ServeError::internal_ctx("subscription has no Joining Location"))?
        };

        let request_lease = Arc::new(
            self.request_capacity
                .try_acquire(RequestDirection::Outbound, RequestClass::Fetch)
                .map_err(|error| ServeError::internal_ctx(error.to_string()))?,
        );
        let request_id = self
            .get_next_request_id()
            .map_err(|error| ServeError::internal_ctx(error.to_string()))?;
        let request = super::fetch::outbound_joining_message(request_id, joining_request_id);
        let (mut fetch, recv) = Fetch::new(
            self.clone(),
            request_id,
            joining_request_id,
            joining_location,
            request_lease,
        );

        let (send_stream, recv_stream) = self
            .webtransport
            .open_bi()
            .await
            .map_err(|error| ServeError::internal_ctx(error.to_string()))?;
        self.fetches
            .lock()
            .map_err(|_| ServeError::internal_ctx("FETCH registry unavailable"))?
            .insert(request_id, recv);
        let mut writer = Writer::new(send_stream);
        if let Err(error) = writer.encode(&Message::Fetch(request)).await {
            let error = ServeError::internal_ctx(error.to_string());
            self.fail_fetch(request_id, error.clone());
            return Err(error);
        }
        writer.finish();
        let (response_cancel, mut response_cancelled) = tokio::sync::oneshot::channel();
        fetch.attach_response_cancel(response_cancel);

        let mut subscriber = self.clone();
        let response_guard = FetchResponseGuard {
            subscriber: subscriber.clone(),
            request_id,
        };
        let handle = tokio::spawn(async move {
            let _response_guard = response_guard;
            let mut reader = Reader::new(recv_stream);
            let response = tokio::select! {
                _ = &mut response_cancelled => {
                    reader.stop(Session::REQUEST_STREAM_CANCELLED);
                    return;
                }
                response = Session::decode_bidi_response(
                    &mut reader,
                    request_id,
                    super::RequestKind::Fetch,
                ) => response,
            };
            match response {
                Ok(message) => match TryInto::<message::Publisher>::try_into(message) {
                    Ok(message) => {
                        if let Err(error) = subscriber.recv_message(message) {
                            subscriber.fail_fetch(
                                request_id,
                                ServeError::internal_ctx(error.to_string()),
                            );
                        }
                    }
                    Err(message) => subscriber.fail_fetch(
                        request_id,
                        ServeError::internal_ctx(format!(
                            "unexpected {} on FETCH response stream",
                            message.name()
                        )),
                    ),
                },
                Err(error) => subscriber.fail_fetch(
                    request_id,
                    ServeError::internal_ctx(format!("FETCH response failed: {error}")),
                ),
            }
        });
        if let Err(error) = self.bidi_task_tx.send(handle) {
            error.abort_and_wait().await;
            self.cancel_fetch(request_id, ServeError::Cancel);
            return Err(ServeError::internal_ctx(
                "session request task collector closed",
            ));
        }

        fetch.ok().await?;
        Ok(fetch)
    }

    /// Enqueue a response for routing to its owning request stream.
    pub(super) fn send_message<M: Into<message::Subscriber>>(&mut self, msg: M) {
        let msg = msg.into();

        // TODO report dropped messages?
        let _ = self.outgoing.push(msg.into());
    }

    /// Receive a publisher message from a request stream.
    pub(super) fn recv_message(&mut self, msg: message::Publisher) -> Result<(), SessionError> {
        match &msg {
            message::Publisher::PublishNamespace(msg) => {
                let lease = Arc::new(
                    self.request_capacity
                        .try_acquire(RequestDirection::Inbound, RequestClass::PublishNamespace)?,
                );
                self.recv_publish_namespace(msg, lease)?;
            }
            message::Publisher::Publish(msg) => {
                let lease = Arc::new(
                    self.request_capacity
                        .try_acquire(RequestDirection::Inbound, RequestClass::Publish)?,
                );
                self.recv_publish(msg, lease)?;
            }
            message::Publisher::RequestUpdate(_) => {
                return Err(SessionError::ProtocolViolation(
                    "REQUEST_UPDATE was not associated with a request stream".to_string(),
                ));
            }
            message::Publisher::PublishDone(msg) => self.recv_publish_done(msg)?,
            // PUBLISH_SKIPPED is scoped to a SUBSCRIBE_TRACKS response stream.
            // The draft-19 wire shape is supported; request lifecycle routing
            // lands with SUBSCRIBE_TRACKS session support.
            message::Publisher::PublishSkipped(msg) => {
                tracing::debug!(
                    target: "moq_transport::control",
                    namespace_suffix = %msg.track_namespace_suffix,
                    track_name = %msg.track_name,
                    "received PUBLISH_SKIPPED for unsupported SUBSCRIBE_TRACKS"
                );
            }
            message::Publisher::SubscribeOk(msg) => self.recv_subscribe_ok(msg)?,
            // Draft-16 shared responses (REQUEST_OK / REQUEST_ERROR).
            message::Publisher::RequestOk(msg) => self.recv_request_ok(msg)?,
            message::Publisher::RequestError(msg) => self.recv_request_error(msg)?,
            message::Publisher::FetchOk(msg) => self.recv_fetch_ok(msg)?,
        }

        Ok(())
    }

    /// Dispatch the first message on a peer-opened request stream while
    /// attaching its already-acquired logical request lease to retained state.
    pub(super) fn recv_request_message(
        &mut self,
        msg: message::Publisher,
        request_lease: Arc<RequestLease>,
    ) -> Result<(), SessionError> {
        match msg {
            message::Publisher::PublishNamespace(msg) => {
                self.recv_publish_namespace(&msg, request_lease)
            }
            message::Publisher::Publish(msg) => self.recv_publish(&msg, request_lease),
            other => self.recv_message(other),
        }
    }

    pub(crate) fn recv_request_update(
        &mut self,
        initial_request_id: u64,
        update: message::RequestUpdate,
    ) -> Result<(), SessionError> {
        tracing::debug!(
            target: "moq_transport::control",
            initial_request_id,
            update_request_id = update.id,
            "rejecting unsupported update of publisher-initiated request"
        );
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
                error_code: crate::message::RequestErrorCode::NotSupported as u64,
                retry_interval: 0,
                reason: crate::coding::ReasonPhrase("not supported".to_string()),
                redirect: None,
            },
        );
    }

    /// Handle reception of an inbound PUBLISH_NAMESPACE from the publisher.
    fn recv_publish_namespace(
        &mut self,
        msg: &message::PublishNamespace,
        request_lease: Arc<RequestLease>,
    ) -> Result<(), SessionError> {
        let mut published_namespaces = self
            .published_namespaces
            .lock()
            .map_err(|_| SessionError::Internal)?;

        // Duplicate PUBLISH_NAMESPACE for the same namespace within a session is invalid.
        let entry = match published_namespaces.entry(msg.track_namespace.clone()) {
            hash_map::Entry::Occupied(_) => return Err(SessionError::Duplicate),
            hash_map::Entry::Vacant(entry) => entry,
        };

        let (published_ns, recv) = PublishedNamespace::new(
            self.clone(),
            msg.id,
            msg.track_namespace.clone(),
            request_lease,
        );
        if let Err(published_ns) = self.published_namespace_queue.push(published_ns) {
            published_ns.close(ServeError::Cancel)?;
            return Ok(());
        }
        entry.insert(recv);

        Ok(())
    }

    /// Handle the reception of a SubscribeOk message from the publisher.
    fn recv_subscribe_ok(&mut self, msg: &message::SubscribeOk) -> Result<(), SessionError> {
        let full_name = self
            .subscribes
            .lock()
            .map_err(|_| SessionError::Internal)?
            .get(&msg.id)
            .map(SubscribeRecv::full_name);
        let Some(full_name) = full_name else {
            return Ok(());
        };

        self.register_alias(msg.track_alias, full_name, AliasBinding::Subscribe(msg.id))?;
        let result = self
            .subscribes
            .lock()
            .map_err(|_| SessionError::Internal)?
            .get_mut(&msg.id)
            .ok_or(SessionError::Internal)?
            .ok(msg);
        if let Err(err) = result {
            self.unregister_alias_binding(msg.track_alias, AliasBinding::Subscribe(msg.id));
            return Err(err.into());
        }

        self.subscribe_alias_notify.notify_waiters();

        Ok(())
    }

    fn recv_fetch_ok(&mut self, msg: &message::FetchOk) -> Result<(), SessionError> {
        let complete = {
            let mut fetches = self.fetches.lock().map_err(|_| SessionError::Internal)?;
            let Some(fetch) = fetches.get_mut(&msg.id) else {
                return Ok(());
            };
            fetch.recv_ok(msg)?;
            fetch.is_complete()
        };
        if complete {
            self.remove_fetch(msg.id);
        }
        Ok(())
    }

    /// Remove a subscribe from our map of active subscribes, and the alias map if present.
    pub(super) fn remove_subscribe(&mut self, id: u64) -> Option<SubscribeRecv> {
        let subscribe = self.subscribes.lock().ok().and_then(|mut s| s.remove(&id));
        if let Some(ref sub) = subscribe {
            sub.release_request_lease();
            if let Some(track_alias) = sub.track_alias() {
                self.unregister_alias_binding(track_alias, AliasBinding::Subscribe(id));
            }
        }
        subscribe
    }

    fn cleanup_outbound_subscribe(&mut self, id: u64) {
        if let Some(subscribe) = self.remove_subscribe(id) {
            let _ = subscribe.error(ServeError::Cancel);
        }
    }

    fn remove_fetch(&mut self, id: u64) -> Option<FetchRecv> {
        let fetch = self.fetches.lock().ok()?.remove(&id);
        if let Some(fetch) = &fetch {
            fetch.release_request_lease();
        }
        fetch
    }

    pub(super) fn fail_fetch(&mut self, id: u64, error: ServeError) {
        let joining_request_id = self
            .fetches
            .lock()
            .ok()
            .and_then(|fetches| fetches.get(&id).map(|fetch| fetch.joining_request_id));
        let abort_failed = joining_request_id.is_some_and(|joining_request_id| {
            self.subscribes
                .lock()
                .ok()
                .and_then(|mut subscribes| {
                    subscribes
                        .get_mut(&joining_request_id)
                        .map(SubscribeRecv::abort_joining_fetch)
                })
                .is_some_and(|result| result.is_err())
        });
        if abort_failed {
            if let Some(subscribe) = joining_request_id.and_then(|id| self.remove_subscribe(id)) {
                let _ = subscribe.error(error.clone());
            }
        }
        if let Some(mut fetch) = self.remove_fetch(id) {
            fetch.fail(error);
        }
    }

    pub(super) fn cancel_fetch(&mut self, id: u64, error: ServeError) {
        self.cancel_request_stream(id, Session::REQUEST_STREAM_CANCELLED);
        self.fail_fetch(id, error);
    }

    fn fetch_response_stream_closed(&mut self, id: u64) {
        let response_received = self
            .fetches
            .lock()
            .ok()
            .and_then(|fetches| fetches.get(&id).map(FetchRecv::response_received));
        if response_received == Some(false) {
            self.fail_fetch(
                id,
                ServeError::internal_ctx("FETCH response stream closed before a response"),
            );
        }
    }

    fn recv_publish(
        &mut self,
        msg: &message::Publish,
        request_lease: Arc<RequestLease>,
    ) -> Result<(), SessionError> {
        // The serve model cannot yet retain Track Properties. Rejecting them
        // avoids silently stripping relay-visible metadata.
        if !msg.track_extensions.is_empty() {
            self.send_request_error(
                "publish",
                message::RequestError {
                    id: msg.id,
                    error_code: message::RequestErrorCode::NotSupported as u64,
                    retry_interval: 0,
                    reason: crate::coding::ReasonPhrase(
                        "track properties are not supported by this media model".to_string(),
                    ),
                    redirect: None,
                },
            );
            return Ok(());
        }

        let initial_forward = msg.params.forward()?.unwrap_or(true);
        let largest_location = msg.params.largest_object()?;
        let publish = {
            let mut publications = self
                .publishes_received
                .lock()
                .map_err(|_| SessionError::Internal)?;
            if publications.contains_key(&msg.id) {
                return Err(SessionError::InvalidRequestId);
            }

            let (writer, reader) =
                serve::Track::new(msg.track_namespace.clone(), msg.track_name.clone()).produce();
            let (publish, recv) = PublishReceivedRecv::produce(
                self.clone(),
                msg.id,
                msg.track_alias,
                msg.track_namespace.clone(),
                msg.track_name.clone(),
                initial_forward,
                largest_location,
                writer,
                reader,
                request_lease,
            );
            self.register_alias(
                msg.track_alias,
                recv.full_name(),
                AliasBinding::Publish(msg.id),
            )?;
            publications.insert(msg.id, recv);
            publish
        };

        self.subscribe_alias_notify.notify_waiters();

        if let Err(publish) = self.publish_received_queue.push(publish) {
            drop(publish);
        }
        Ok(())
    }

    /// Handle the reception of a PublishDone message from the publisher.
    fn recv_publish_done(&mut self, msg: &message::PublishDone) -> Result<(), SessionError> {
        let subscribe_complete = {
            let mut subscribes = self.subscribes.lock().map_err(|_| SessionError::Internal)?;
            match subscribes.get_mut(&msg.id) {
                Some(subscribe) => Some(subscribe.recv_done(msg.status_code, msg.stream_count)?),
                None => None,
            }
        };
        if let Some(complete) = subscribe_complete {
            if complete {
                self.remove_subscribe(msg.id);
            }
            return Ok(());
        }

        let complete = {
            let mut publications = self
                .publishes_received
                .lock()
                .map_err(|_| SessionError::Internal)?;
            match publications.get_mut(&msg.id) {
                Some(publish) => publish.recv_done(msg.status_code, msg.stream_count)?,
                None => false,
            }
        };
        if complete {
            self.remove_publish_received(msg.id);
        }

        Ok(())
    }

    /// Keep the SUBSCRIBE response task alive until every subgroup stream
    /// declared by PUBLISH_DONE has been received, with a hard upper bound.
    async fn await_subscribe_done_cleanup(&mut self, request_id: u64) {
        let deadline = tokio::time::sleep(PUBLISH_DONE_CLEANUP_TIMEOUT);
        tokio::pin!(deadline);
        loop {
            let progress = self.subscribes.lock().ok().and_then(|subscribes| {
                subscribes.get(&request_id).and_then(|subscribe| {
                    subscribe.awaiting_streams().then(|| subscribe.progress())
                })
            });
            let Some(progress) = progress else {
                return;
            };
            tokio::select! {
                _ = progress.notified() => {}
                _ = &mut deadline => {
                    if let Some(subscribe) = self.remove_subscribe(request_id) {
                        let _ = subscribe.error(ServeError::internal_ctx(
                            "timed out waiting for declared SUBSCRIBE streams",
                        ));
                    }
                    return;
                }
            }
        }
    }

    /// Keep the supervised request-stream task alive until every stream
    /// declared by PUBLISH_DONE is accounted for, with a hard upper bound.
    pub(super) async fn await_publish_done_cleanup(&mut self, request_id: u64) {
        let deadline = tokio::time::sleep(PUBLISH_DONE_CLEANUP_TIMEOUT);
        tokio::pin!(deadline);
        loop {
            let progress = self.publishes_received.lock().ok().and_then(|publishes| {
                publishes
                    .get(&request_id)
                    .and_then(|publish| publish.awaiting_streams().then(|| publish.progress()))
            });
            let Some(progress) = progress else {
                return;
            };
            tokio::select! {
                _ = progress.notified() => {}
                _ = &mut deadline => {
                    self.fail_publish_received(
                        request_id,
                        ServeError::internal_ctx("timed out waiting for declared PUBLISH streams"),
                    );
                    return;
                }
            }
        }
    }

    /// Handle REQUEST_OK from the publisher.
    ///
    /// REQUEST_OK is the shared positive response for REQUEST_UPDATE, TRACK_STATUS,
    /// SUBSCRIBE_NAMESPACE, and PUBLISH_NAMESPACE.  SUBSCRIBE uses its own dedicated
    /// SUBSCRIBE_OK message (§9.10) and does not come through this handler.
    /// Full routing for the other request types is wired up (TODO itzmanish).
    fn recv_request_ok(&mut self, msg: &message::RequestOk) -> Result<(), SessionError> {
        self.log_request_ok_parsed("unknown", msg);
        tracing::debug!(
            target: "moq_transport::control",
            request_id = msg.id,
            "received REQUEST_OK"
        );
        // TODO(itzmanish): route to the correct pending request type by ID.
        Ok(())
    }

    /// Handle REQUEST_ERROR from the publisher.
    ///
    /// Routes to the matching active subscribe (via request ID) if one
    /// exists, otherwise logs and ignores.  Full per-flow routing is
    /// wired up (TODO itzmanish).
    fn recv_request_error(&mut self, msg: &message::RequestError) -> Result<(), SessionError> {
        if self
            .fetches
            .lock()
            .map_err(|_| SessionError::Internal)?
            .contains_key(&msg.id)
        {
            self.log_request_error_parsed("fetch", msg);
            self.fail_fetch(msg.id, Self::request_error_to_serve_error(msg));
            return Ok(());
        }
        // Route to a matching subscribe if present.
        if let Some(subscribe) = self.remove_subscribe(msg.id) {
            self.log_request_error_parsed("subscribe", msg);
            let err = Self::request_error_to_serve_error(msg);
            subscribe.error(err)?;
        } else {
            self.log_request_error_parsed("unknown", msg);
        }

        tracing::debug!(
            target: "moq_transport::control",
            request_id = msg.id,
            error_code = msg.error_code,
            retry_interval = msg.retry_interval,
            reason = %msg.reason.0,
            "received REQUEST_ERROR"
        );
        Ok(())
    }

    /// Map a REQUEST_ERROR to a semantic ServeError so callers see
    /// meaningful variants (e.g. NotFound) instead of opaque error codes.
    fn request_error_to_serve_error(msg: &message::RequestError) -> ServeError {
        use message::RequestErrorCode;
        match msg.error_code {
            c if c == RequestErrorCode::DoesNotExist as u64 => {
                ServeError::not_found_ctx(msg.reason.0.clone())
            }
            c if c == RequestErrorCode::InternalError as u64 => {
                ServeError::internal_ctx(msg.reason.0.clone())
            }
            c if c == RequestErrorCode::NotSupported as u64 => {
                ServeError::NotImplemented(msg.reason.0.clone())
            }
            code => ServeError::Closed(code),
        }
    }

    pub(super) fn drop_publish_namespace(&mut self, id: u64) -> Option<PublishedNamespaceRecv> {
        if let Ok(mut ns) = self.published_namespaces.lock() {
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

    /// Remove every retained representation of a peer-opened
    /// PUBLISH_NAMESPACE and mark any application handle complete.
    pub(super) fn cleanup_inbound_publish_namespace(&mut self, id: u64) {
        if let Some(mut recv) = self.drop_publish_namespace(id) {
            let _ = recv.recv_done();
        }
        self.published_namespace_queue
            .remove_where(|namespace| namespace.info.request_id == id);
    }

    /// Mark an inbound namespace request complete, then wait until the
    /// application has observed that completion and released its handle.
    pub(super) async fn finish_inbound_publish_namespace(
        &mut self,
        id: u64,
    ) -> Result<(), SessionError> {
        let mut recv = self.drop_publish_namespace(id).ok_or_else(|| {
            SessionError::ProtocolViolation(format!(
                "PUBLISH_NAMESPACE request {id} completed without retained state"
            ))
        })?;
        recv.recv_done()?;
        self.published_namespace_queue
            .remove_where(|namespace| namespace.info.request_id == id);
        recv.acknowledged().await;
        Ok(())
    }

    /// Remove every retained representation of a peer-opened PUBLISH and
    /// notify an already-consumed application handle that its stream ended.
    pub(super) fn cleanup_inbound_publish(&mut self, id: u64) {
        self.fail_publish_received(id, ServeError::Cancel);
        self.publish_received_queue
            .remove_where(|publish| publish.request_id() == id);
    }

    pub(super) fn remove_publish_received(&mut self, id: u64) {
        if let Ok(mut publishes) = self.publishes_received.lock() {
            publishes.remove(&id);
        }
        self.remove_publish_indexes(id);
    }

    pub(super) fn fail_publish_received(&mut self, id: u64, err: ServeError) {
        let publish = self
            .publishes_received
            .lock()
            .ok()
            .and_then(|mut publishes| publishes.remove(&id));
        if let Some(mut publish) = publish {
            publish.recv_stream_error(err);
        }
        self.remove_publish_indexes(id);
    }

    fn remove_publish_indexes(&self, id: u64) {
        if let Ok(mut aliases) = self.alias_map.lock() {
            aliases.retain(|_, entry| {
                entry
                    .bindings
                    .retain(|binding| *binding != AliasBinding::Publish(id));
                !entry.bindings.is_empty()
            });
        }
    }

    fn finish_publish_stream(&mut self, id: u64) -> Result<(), SessionError> {
        let complete = {
            let mut publications = self
                .publishes_received
                .lock()
                .map_err(|_| SessionError::Internal)?;
            match publications.get_mut(&id) {
                Some(publish) => publish.finish_stream()?,
                None => false,
            }
        };
        if complete {
            self.remove_publish_received(id);
        }
        Ok(())
    }

    fn finish_subscribe_stream(&mut self, id: u64) -> Result<(), SessionError> {
        let complete = {
            let mut subscribes = self.subscribes.lock().map_err(|_| SessionError::Internal)?;
            match subscribes.get_mut(&id) {
                Some(subscribe) => subscribe.finish_stream()?,
                None => false,
            }
        };
        if complete {
            self.remove_subscribe(id);
        }
        Ok(())
    }

    fn begin_publish_stream(&mut self, id: u64) -> Result<(), SessionError> {
        self.publishes_received
            .lock()
            .map_err(|_| SessionError::Internal)?
            .get_mut(&id)
            .ok_or_else(|| {
                SessionError::Serve(ServeError::not_found_ctx(format!(
                    "publish_id={id} not found"
                )))
            })?
            .begin_stream(MAX_STREAMS_PER_PUBLICATION)?;
        Ok(())
    }

    fn begin_subscribe_stream(&mut self, id: u64) -> Result<(), SessionError> {
        self.subscribes
            .lock()
            .map_err(|_| SessionError::Internal)?
            .get_mut(&id)
            .ok_or_else(|| {
                SessionError::Serve(ServeError::not_found_ctx(format!(
                    "subscribe_id={id} not found"
                )))
            })?
            .begin_stream(MAX_STREAMS_PER_PUBLICATION)?;
        Ok(())
    }

    fn fail_subscribe_stream(&mut self, id: u64, error: ServeError) {
        if let Some(mut subscribe) = self.remove_subscribe(id) {
            subscribe.recv_stream_error(error);
        }
    }

    fn register_alias(
        &self,
        track_alias: u64,
        full_name: serve::FullTrackName,
        binding: AliasBinding,
    ) -> Result<(), SessionError> {
        let mut aliases = self.alias_map.lock().map_err(|_| SessionError::Internal)?;
        Self::insert_alias(&mut aliases, track_alias, full_name, binding)
    }

    fn insert_alias(
        aliases: &mut HashMap<u64, AliasEntry>,
        track_alias: u64,
        full_name: serve::FullTrackName,
        binding: AliasBinding,
    ) -> Result<(), SessionError> {
        match aliases.entry(track_alias) {
            hash_map::Entry::Vacant(entry) => {
                entry.insert(AliasEntry {
                    full_name,
                    bindings: vec![binding],
                });
            }
            hash_map::Entry::Occupied(mut entry) => {
                if entry.get().full_name != full_name {
                    return Err(SessionError::Duplicate);
                }
                if entry.get().bindings.contains(&binding) {
                    return Err(SessionError::Duplicate);
                }
                entry.get_mut().bindings.push(binding);
            }
        }
        Ok(())
    }

    fn unregister_alias_binding(&self, track_alias: u64, binding: AliasBinding) {
        if let Ok(mut aliases) = self.alias_map.lock() {
            Self::remove_alias_binding(&mut aliases, track_alias, binding);
        }
    }

    fn remove_alias_binding(
        aliases: &mut HashMap<u64, AliasEntry>,
        track_alias: u64,
        binding: AliasBinding,
    ) {
        if let hash_map::Entry::Occupied(mut entry) = aliases.entry(track_alias) {
            entry.get_mut().bindings.retain(|value| *value != binding);
            if entry.get().bindings.is_empty() {
                entry.remove();
            }
        }
    }

    /// Resolve every request sharing a session-scoped exact-track alias.
    async fn resolve_alias(
        &self,
        track_alias: u64,
        timeout_ms: Option<u64>,
    ) -> Result<Option<Vec<AliasBinding>>, SessionError> {
        let lookup = || -> Result<Option<Vec<AliasBinding>>, SessionError> {
            Ok(self
                .alias_map
                .lock()
                .map_err(|_| SessionError::Internal)?
                .get(&track_alias)
                .map(|entry| entry.bindings.clone()))
        };

        let timeout_ms = match timeout_ms {
            Some(ms) => ms,
            None => return lookup(),
        };

        let timeout_duration = Duration::from_millis(timeout_ms);
        tokio::time::timeout(timeout_duration, async {
            loop {
                let notified = self.subscribe_alias_notify.notified();
                if let Some(binding) = lookup()? {
                    return Ok(Some(binding));
                }
                notified.await;
            }
        })
        .await
        .unwrap_or(Ok(None))
    }

    /// Handle reception of a new stream from the QUIC session.
    pub(super) async fn recv_stream(
        mut self,
        stream: web_transport::RecvStream,
    ) -> Result<(), SessionError> {
        tracing::trace!("[SUBSCRIBER] recv_stream: new stream received, decoding header");
        let mut reader = Reader::new(stream);

        // Decode the stream header
        let stream_header: data::StreamHeader = reader.decode().await?;
        tracing::trace!(
            "[SUBSCRIBER] recv_stream: decoded stream header type={:?}",
            stream_header.header_type
        );

        if stream_header.header_type.is_fetch() {
            let fetch_header = stream_header.fetch_header.ok_or_else(|| {
                SessionError::ProtocolViolation("FETCH stream omitted its header".to_string())
            })?;
            return self.recv_fetch_stream(reader, fetch_header).await;
        }

        if !stream_header.header_type.is_subgroup() {
            return Err(SessionError::unimplemented("non-SUBGROUP stream types"));
        }

        // Log subgroup header parsed/received
        if let Some(ref subgroup_header) = stream_header.subgroup_header {
            if let Some(ref mlog) = self.mlog {
                if let Ok(mut mlog_guard) = mlog.lock() {
                    let time = mlog_guard.elapsed_ms();
                    let stream_id = 0; // TODO: Placeholder, need actual QUIC stream ID
                    let event = mlog::subgroup_header_parsed(time, stream_id, subgroup_header);
                    let _ = mlog_guard.add_event(event);
                }
            }
        }

        let track_alias = stream_header.subgroup_header.as_ref().unwrap().track_alias;
        tracing::trace!(
            "[SUBSCRIBER] recv_stream: stream for subscription track_alias={}",
            track_alias
        );

        let mlog = self.mlog.clone();
        let res = self.recv_stream_inner(reader, stream_header, mlog).await;
        if let Err(error) = &res {
            let err = match error {
                SessionError::Serve(err) => err.clone(),
                _ => ServeError::internal_ctx(error.to_string()),
            };
            tracing::warn!(
                "[SUBSCRIBER] recv_stream: stream processing error for track_alias={}: {:?}",
                track_alias,
                err
            );
            // The writer is closed, so we should terminate.
            // TODO it would be nice to do this immediately when the Writer is closed.
            for binding in self
                .resolve_alias(track_alias, None)
                .await?
                .unwrap_or_default()
            {
                match binding {
                    AliasBinding::Subscribe(id) => {
                        if let Some(subscribe) = self.remove_subscribe(id) {
                            subscribe.error(err.clone())?;
                        }
                    }
                    AliasBinding::Publish(id) => {
                        self.fail_publish_received(id, err.clone());
                    }
                }
            }
        }

        res
    }

    async fn recv_fetch_stream(
        &mut self,
        mut reader: Reader,
        header: data::FetchHeader,
    ) -> Result<(), SessionError> {
        let request_id = header.request_id;
        {
            let mut fetches = self.fetches.lock().map_err(|_| SessionError::Internal)?;
            let fetch = fetches.get_mut(&request_id).ok_or_else(|| {
                SessionError::ProtocolViolation(format!(
                    "FETCH data stream references inactive request {request_id}"
                ))
            })?;
            fetch.start_data()?;
        }

        let result = async {
            let mut decoder = data::FetchObjectDecoder::new(message::GroupOrder::Ascending)?;
            loop {
                let Some(item) = reader.decode_with(|cursor| decoder.decode(cursor)).await? else {
                    break;
                };
                let data::FetchItem::Object(object) = item else {
                    return Err(SessionError::unimplemented(
                        "FETCH non-existent/unknown range markers",
                    ));
                };
                let subgroup_id = match object.forwarding_preference {
                    data::FetchForwardingPreference::Subgroup(id) => id,
                    data::FetchForwardingPreference::Datagram => {
                        return Err(SessionError::unimplemented(
                            "datagram forwarding preference on Joining FETCH",
                        ));
                    }
                };
                if object.payload_length > serve::RetentionLimits::default().max_object_bytes {
                    return Err(SessionError::Serve(ServeError::internal_ctx(
                        "FETCH Object exceeds the bounded receive limit",
                    )));
                }
                let mut payload = bytes::BytesMut::with_capacity(object.payload_length);
                while payload.len() < object.payload_length {
                    let remaining = object.payload_length - payload.len();
                    let chunk = reader.read_chunk(remaining).await?.ok_or_else(|| {
                        SessionError::ProtocolViolation(
                            "FETCH stream ended inside an Object payload".to_string(),
                        )
                    })?;
                    payload.extend_from_slice(&chunk);
                }
                let fetched = FetchedObject {
                    location: object.location(),
                    subgroup_id,
                    publisher_priority: object.publisher_priority,
                    properties: object.properties,
                    payload: payload.freeze(),
                    group_end: EndOfGroupState::UnknownFromFetch,
                };
                self.recv_fetch_object(request_id, fetched)?;
            }
            self.finish_fetch_data(request_id)
        }
        .await;

        if let Err(error) = &result {
            self.fail_fetch(request_id, ServeError::internal_ctx(error.to_string()));
        }
        result
    }

    fn recv_fetch_object(
        &mut self,
        request_id: u64,
        object: FetchedObject,
    ) -> Result<(), SessionError> {
        let joining_request_id = self
            .fetches
            .lock()
            .map_err(|_| SessionError::Internal)?
            .get(&request_id)
            .ok_or_else(|| {
                SessionError::ProtocolViolation(format!(
                    "FETCH Object references inactive request {request_id}"
                ))
            })?
            .joining_request_id;
        // Validate ordering/range and reserve bounded observation capacity
        // before mutating the application-visible handoff track.
        self.fetches
            .lock()
            .map_err(|_| SessionError::Internal)?
            .get_mut(&request_id)
            .ok_or_else(|| {
                SessionError::ProtocolViolation(format!(
                    "FETCH Object references inactive request {request_id}"
                ))
            })?
            .recv_object(object.clone())?;
        self.subscribes
            .lock()
            .map_err(|_| SessionError::Internal)?
            .get_mut(&joining_request_id)
            .ok_or_else(|| {
                SessionError::ProtocolViolation(format!(
                    "Joining FETCH references inactive subscription {joining_request_id}"
                ))
            })?
            .recv_fetched_object(BufferedJoinObject {
                location: object.location,
                subgroup_id: object.subgroup_id,
                publisher_priority: object.publisher_priority,
                properties: object.properties.clone(),
                payload: object.payload.clone(),
                first_object: object.location.object_id == 0,
                group_end: object.group_end,
            })?;
        Ok(())
    }

    fn finish_fetch_data(&mut self, request_id: u64) -> Result<(), SessionError> {
        let joining_request_id = self
            .fetches
            .lock()
            .map_err(|_| SessionError::Internal)?
            .get(&request_id)
            .ok_or_else(|| {
                SessionError::ProtocolViolation(format!(
                    "FETCH FIN references inactive request {request_id}"
                ))
            })?
            .joining_request_id;
        self.subscribes
            .lock()
            .map_err(|_| SessionError::Internal)?
            .get_mut(&joining_request_id)
            .ok_or_else(|| {
                SessionError::ProtocolViolation(format!(
                    "Joining FETCH references inactive subscription {joining_request_id}"
                ))
            })?
            .finish_joining_fetch()?;
        let complete = {
            let mut fetches = self.fetches.lock().map_err(|_| SessionError::Internal)?;
            let fetch = fetches.get_mut(&request_id).ok_or_else(|| {
                SessionError::ProtocolViolation(format!(
                    "FETCH FIN references inactive request {request_id}"
                ))
            })?;
            fetch.finish_data()?;
            fetch.is_complete()
        };
        if complete {
            self.remove_fetch(request_id);
        }
        Ok(())
    }

    /// Continue handling the reception of a new stream from the QUIC session.
    async fn recv_stream_inner(
        &mut self,
        mut reader: Reader,
        stream_header: data::StreamHeader,
        mlog: Option<Arc<Mutex<mlog::MlogWriter>>>,
    ) -> Result<(), SessionError> {
        let track_alias = stream_header.subgroup_header.as_ref().unwrap().track_alias;
        tracing::trace!(
            "[SUBSCRIBER] recv_stream_inner: processing stream for track_alias={}",
            track_alias
        );

        let Some(bindings) = self
            .resolve_alias(track_alias, Some(DEFAULT_ALIAS_WAIT_TIME_MS))
            .await?
        else {
            return Err(SessionError::Serve(ServeError::not_found_ctx(format!(
                "subscription track_alias={} not found",
                track_alias
            ))));
        };

        tracing::trace!("[SUBSCRIBER] recv_stream_inner: receiving subgroup data");
        let mut active_bindings = Vec::with_capacity(bindings.len());
        let mut subscribe_ids = Vec::new();
        let mut publish_ids = Vec::new();
        for binding in bindings {
            match binding {
                AliasBinding::Subscribe(id) => {
                    if let Err(error) = self.begin_subscribe_stream(id) {
                        self.fail_subscribe_stream(id, ServeError::internal_ctx(error.to_string()));
                        continue;
                    }
                    subscribe_ids.push(id);
                }
                AliasBinding::Publish(id) => {
                    if self.begin_publish_stream(id).is_err() {
                        self.cancel_publish_received(
                            id,
                            message::RequestErrorCode::ExcessiveLoad as u32,
                        );
                        continue;
                    }
                    publish_ids.push(id);
                }
            }
            active_bindings.push(binding);
        }
        if active_bindings.is_empty() {
            reader.stop(message::RequestErrorCode::ExcessiveLoad as u32);
            return Ok(());
        }

        let result = self
            .recv_subgroup(
                stream_header.header_type,
                stream_header.subgroup_header.unwrap(),
                &active_bindings,
                reader,
                mlog,
            )
            .await;

        match result {
            Ok(()) => {
                for id in subscribe_ids {
                    self.finish_subscribe_stream(id)?;
                }
                for id in publish_ids {
                    self.finish_publish_stream(id)?;
                }
            }
            Err(error) => {
                for id in subscribe_ids {
                    self.fail_subscribe_stream(id, ServeError::internal_ctx(error.to_string()));
                }
                for id in publish_ids {
                    self.fail_publish_received(id, ServeError::internal_ctx(error.to_string()));
                }
                return Err(error);
            }
        }

        tracing::trace!(
            "[SUBSCRIBER] recv_stream_inner: completed processing stream for track_alias={}",
            track_alias
        );
        Ok(())
    }

    fn binding_claims_object(
        &mut self,
        binding: AliasBinding,
        group_id: u64,
        object_id: u64,
    ) -> Result<bool, SessionError> {
        match binding {
            AliasBinding::Subscribe(id) => Ok(self
                .subscribes
                .lock()
                .map_err(|_| SessionError::Internal)?
                .get_mut(&id)
                .is_some_and(|subscribe| subscribe.claim_object(group_id, object_id))),
            AliasBinding::Publish(id) => Ok(self
                .publishes_received
                .lock()
                .map_err(|_| SessionError::Internal)?
                .get_mut(&id)
                .is_some_and(|publish| publish.claim_object(group_id, object_id))),
        }
    }

    fn fail_alias_binding(&mut self, binding: AliasBinding, err: ServeError) {
        match binding {
            AliasBinding::Subscribe(id) => {
                if let Some(subscribe) = self.remove_subscribe(id) {
                    let _ = subscribe.error(err);
                }
            }
            AliasBinding::Publish(id) => {
                self.cancel_publish_received(id, Session::REQUEST_STREAM_CANCELLED);
            }
        }
    }

    fn binding_subgroup(
        &mut self,
        binding: AliasBinding,
        header: data::SubgroupHeader,
    ) -> Result<Option<serve::SubgroupWriter>, SessionError> {
        match binding {
            AliasBinding::Subscribe(id) => self
                .subscribes
                .lock()
                .map_err(|_| SessionError::Internal)?
                .get_mut(&id)
                .map(|subscribe| subscribe.subgroup(header).map_err(SessionError::from))
                .transpose(),
            AliasBinding::Publish(id) => self
                .publishes_received
                .lock()
                .map_err(|_| SessionError::Internal)?
                .get_mut(&id)
                .map(|publish| publish.subgroup(header).map_err(SessionError::from))
                .transpose(),
        }
    }

    fn binding_has_joining_barrier(&self, binding: AliasBinding) -> Result<bool, SessionError> {
        match binding {
            AliasBinding::Subscribe(id) => Ok(self
                .subscribes
                .lock()
                .map_err(|_| SessionError::Internal)?
                .get(&id)
                .is_some_and(SubscribeRecv::has_joining_barrier)),
            AliasBinding::Publish(_) => Ok(false),
        }
    }

    fn binding_recv_joining_live_object(
        &mut self,
        binding: AliasBinding,
        object: BufferedJoinObject,
    ) -> Result<(), SessionError> {
        let AliasBinding::Subscribe(id) = binding else {
            return Err(SessionError::Internal);
        };
        self.subscribes
            .lock()
            .map_err(|_| SessionError::Internal)?
            .get_mut(&id)
            .ok_or_else(|| {
                SessionError::Serve(ServeError::not_found_ctx(format!(
                    "subscription {id} closed during Joining FETCH handoff"
                )))
            })?
            .recv_joining_live_object(object)?;
        Ok(())
    }

    /// If new stream is a Subgroup stream, handle reception of subgroup objects and payloads.
    async fn recv_subgroup(
        &mut self,
        stream_header_type: data::StreamHeaderType,
        mut subgroup_header: data::SubgroupHeader,
        bindings: &[AliasBinding],
        mut reader: Reader,
        mlog: Option<Arc<Mutex<mlog::MlogWriter>>>,
    ) -> Result<(), SessionError> {
        tracing::trace!(
            "[SUBSCRIBER] recv_subgroup: starting - group_id={}, subgroup_id={:?}, priority={}",
            subgroup_header.group_id,
            subgroup_header.subgroup_id,
            subgroup_header.publisher_priority
        );

        let mut object_count = 0;
        let mut previous_object_id: Option<u64> = None;
        let mut subgroup_writers: Vec<(AliasBinding, serve::SubgroupWriter)> = Vec::new();
        while !reader.done().await? {
            tracing::trace!(
                "[SUBSCRIBER] recv_subgroup: reading object #{} (has_ext_headers={})",
                object_count + 1,
                stream_header_type.has_extension_headers()
            );

            // Need to be able to decode the subgroup object conditionally based on the stream header type
            // read the object payload length into remaining_bytes
            let (mut remaining_bytes, object_id_delta, status, decoded_object) =
                match stream_header_type.has_extension_headers() {
                    true => {
                        let object = reader.decode::<data::SubgroupObjectExt>().await?;
                        tracing::trace!(
                        "[SUBSCRIBER] recv_subgroup: object #{} with extension headers - object_id_delta={}, payload_length={}, status={:?}, extension_headers={:?}",
                        object_count + 1,
                        object.object_id_delta,
                        object.payload_length,
                        object.status,
                        object.extension_headers
                    );

                        // Check for known draft-14 extension types

                        // Check for Immutable Extensions (type 0xB = 11)
                        if object.extension_headers.has(0xB) {
                            tracing::trace!(
                                "[SUBSCRIBER] recv_subgroup: object #{} contains IMMUTABLE EXTENSIONS (type 0xB) - will be forwarded",
                                object_count + 1
                            );
                            if let Some(immutable_ext) = object.extension_headers.get(0xB) {
                                tracing::trace!(
                                    "[SUBSCRIBER] recv_subgroup: immutable extension details: {:?}",
                                    immutable_ext
                                );
                            }
                        }

                        // Check for Prior Group ID Gap (type 0x3C = 60)
                        if object.extension_headers.has(0x3C) {
                            tracing::trace!(
                                "[SUBSCRIBER] recv_subgroup: object #{} contains PRIOR GROUP ID GAP (type 0x3C)",
                                object_count + 1
                            );
                            if let Some(gap_ext) = object.extension_headers.get(0x3C) {
                                tracing::trace!(
                                    "[SUBSCRIBER] recv_subgroup: prior group id gap details: {:?}",
                                    gap_ext
                                );
                            }
                        }

                        let obj_copy = object.clone();
                        (
                            object.payload_length,
                            object.object_id_delta,
                            object.status,
                            Some(obj_copy),
                        )
                    }
                    false => {
                        let object = reader.decode::<data::SubgroupObject>().await?;
                        tracing::trace!(
                        "[SUBSCRIBER] recv_subgroup: object #{} - object_id_delta={}, payload_length={}, status={:?}",
                        object_count + 1,
                        object.object_id_delta,
                        object.payload_length,
                        object.status
                    );
                        (
                            object.payload_length,
                            object.object_id_delta,
                            object.status,
                            None,
                        )
                    }
                };

            let current_object_id = match previous_object_id {
                Some(previous) => previous
                    .checked_add(object_id_delta)
                    .and_then(|value| value.checked_add(1))
                    .ok_or_else(|| {
                        SessionError::ProtocolViolation("subgroup object id overflow".to_string())
                    })?,
                None => object_id_delta,
            };
            previous_object_id = Some(current_object_id);

            // Extract extension headers if present
            let extension_headers = decoded_object
                .as_ref()
                .map(|obj| obj.extension_headers.clone());

            if status.is_some_and(|status| status != data::ObjectStatus::NormalObject)
                && extension_headers
                    .as_ref()
                    .is_some_and(|headers| !headers.is_empty())
            {
                return Err(SessionError::ProtocolViolation(
                    "non-normal object status with extension headers".to_string(),
                ));
            }

            if stream_header_type.uses_first_object_id_as_subgroup_id()
                && subgroup_header.subgroup_id.is_none()
            {
                subgroup_header.subgroup_id = Some(current_object_id);
            }

            let mut claimed_bindings = Vec::new();
            let mut barrier_bindings = Vec::new();
            for binding in bindings {
                if !self.binding_claims_object(
                    *binding,
                    subgroup_header.group_id,
                    current_object_id,
                )? {
                    continue;
                }
                claimed_bindings.push(*binding);
                if self.binding_has_joining_barrier(*binding)? {
                    barrier_bindings.push(*binding);
                    continue;
                }
                if subgroup_writers
                    .iter()
                    .any(|(existing, _)| existing == binding)
                {
                    continue;
                }
                match self.binding_subgroup(*binding, subgroup_header.clone()) {
                    Ok(Some(writer)) => subgroup_writers.push((*binding, writer)),
                    Ok(None) => {}
                    Err(SessionError::Serve(err)) => {
                        self.fail_alias_binding(*binding, err);
                    }
                    Err(error) => return Err(error),
                }
            }

            // Log subgroup object parsed/received
            if let Some(ref mlog) = mlog {
                if let Ok(mut mlog_guard) = mlog.lock() {
                    let time = mlog_guard.elapsed_ms();
                    let stream_id = 0; // TODO: Placeholder, need actual QUIC stream ID
                    let event = if let Some(obj_ext) = decoded_object {
                        mlog::subgroup_object_ext_parsed(
                            time,
                            stream_id,
                            subgroup_header.group_id,
                            subgroup_header.subgroup_id.unwrap_or(0),
                            current_object_id,
                            &obj_ext,
                        )
                    } else {
                        // For non-extension objects, create a temporary SubgroupObject for logging
                        let temp_obj = data::SubgroupObject {
                            object_id_delta,
                            payload_length: remaining_bytes,
                            status,
                        };
                        mlog::subgroup_object_parsed(
                            time,
                            stream_id,
                            subgroup_header.group_id,
                            subgroup_header.subgroup_id.unwrap_or(0),
                            current_object_id,
                            &temp_obj,
                        )
                    };
                    let _ = mlog_guard.add_event(event);
                }
            }

            // Pass extension headers through to the serve layer
            // TODO SLG - object_id_delta and object status are still being ignored

            let mut object_writers = Vec::with_capacity(claimed_bindings.len());
            let mut failed_bindings = Vec::new();
            for binding in claimed_bindings {
                let Some((_, subgroup_writer)) = subgroup_writers
                    .iter_mut()
                    .find(|(existing, _)| *existing == binding)
                else {
                    continue;
                };
                match subgroup_writer.create(remaining_bytes, extension_headers.clone()) {
                    Ok(writer) => object_writers.push((binding, writer)),
                    Err(err) => failed_bindings.push((binding, err)),
                }
            }
            for (binding, err) in failed_bindings {
                subgroup_writers.retain(|(existing, _)| *existing != binding);
                self.fail_alias_binding(binding, err);
            }
            tracing::trace!(
                "[SUBSCRIBER] recv_subgroup: reading payload for object #{} ({} bytes)",
                object_count + 1,
                remaining_bytes
            );

            if !barrier_bindings.is_empty()
                && remaining_bytes > serve::RetentionLimits::default().max_object_bytes
            {
                for binding in barrier_bindings.drain(..) {
                    self.fail_alias_binding(
                        binding,
                        ServeError::Closed(message::RequestErrorCode::ExcessiveLoad as u64),
                    );
                }
            }
            let mut chunks_read = 0;
            let mut joining_payload = (!barrier_bindings.is_empty())
                .then(|| bytes::BytesMut::with_capacity(remaining_bytes));
            while remaining_bytes > 0 {
                let data = reader
                    .read_chunk(remaining_bytes)
                    .await?
                    .ok_or_else(|| {
                        tracing::error!(
                            "[SUBSCRIBER] recv_subgroup: ERROR - stream ended with {} bytes remaining for object #{}",
                            remaining_bytes,
                            object_count + 1
                        );
                        SessionError::WrongSize
                    })?;
                tracing::trace!(
                    "[SUBSCRIBER] recv_subgroup: received payload chunk #{} for object #{} ({} bytes, {} remaining)",
                    chunks_read + 1,
                    object_count + 1,
                    data.len(),
                    remaining_bytes - data.len()
                );
                remaining_bytes -= data.len();
                if let Some(payload) = joining_payload.as_mut() {
                    payload.extend_from_slice(&data);
                }
                let mut failed_bindings = Vec::new();
                for (binding, object_writer) in &mut object_writers {
                    if let Err(err) = object_writer.write(data.clone()) {
                        failed_bindings.push((*binding, err));
                    }
                }
                for (binding, err) in failed_bindings {
                    object_writers.retain(|(existing, _)| *existing != binding);
                    subgroup_writers.retain(|(existing, _)| *existing != binding);
                    self.fail_alias_binding(binding, err);
                }
                chunks_read += 1;
            }

            if let Some(payload) = joining_payload {
                let payload = payload.freeze();
                let mut failed_bindings = Vec::new();
                for binding in barrier_bindings {
                    let object = BufferedJoinObject {
                        location: crate::coding::Location::new(
                            subgroup_header.group_id,
                            current_object_id,
                        ),
                        subgroup_id: subgroup_header.subgroup_id.unwrap_or(0),
                        publisher_priority: subgroup_header.publisher_priority,
                        properties: extension_headers.clone().unwrap_or_default(),
                        payload: payload.clone(),
                        first_object: stream_header_type.is_first_object(),
                        group_end: EndOfGroupState::from_live_header(
                            stream_header_type.contains_end_of_group(),
                        ),
                    };
                    if let Err(error) = self.binding_recv_joining_live_object(binding, object) {
                        failed_bindings.push((binding, error));
                    }
                }
                for (binding, error) in failed_bindings {
                    let serve_error = match error {
                        SessionError::Serve(error) => error,
                        other => ServeError::internal_ctx(other.to_string()),
                    };
                    self.fail_alias_binding(binding, serve_error);
                }
            }

            tracing::trace!(
                "[SUBSCRIBER] recv_subgroup: completed object #{} ({} chunks)",
                object_count + 1,
                chunks_read
            );
            object_count += 1;
        }

        tracing::trace!(
            "[SUBSCRIBER] recv_subgroup: completed subgroup (group_id={}, subgroup_id={}, {} objects received)",
            subgroup_header.group_id,
            subgroup_header.subgroup_id.unwrap_or(0),
            object_count
        );

        Ok(())
    }

    /// Handle reception of a datagram from the QUIC session.
    pub async fn recv_datagram(&mut self, datagram: bytes::Bytes) -> Result<(), SessionError> {
        let mut cursor = io::Cursor::new(datagram);
        let datagram = data::Datagram::decode(&mut cursor)?;

        if let Some(ref mlog) = self.mlog {
            if let Ok(mut mlog_guard) = mlog.lock() {
                let time = mlog_guard.elapsed_ms();
                let stream_id = 0; // TODO: Placeholder, need actual QUIC stream ID
                let _ =
                    mlog_guard.add_event(mlog::object_datagram_parsed(time, stream_id, &datagram));
            }
        }

        // Check for extension headers in the datagram
        if let Some(ref ext_headers) = datagram.extension_headers {
            tracing::trace!(
                "[SUBSCRIBER] recv_datagram: datagram contains extension headers: {:?}",
                ext_headers
            );

            // Check for known draft-14 extension types

            // Check for Immutable Extensions (type 0xB = 11)
            if ext_headers.has(0xB) {
                tracing::trace!(
                    "[SUBSCRIBER] recv_datagram: datagram contains IMMUTABLE EXTENSIONS (type 0xB)"
                );
                if let Some(immutable_ext) = ext_headers.get(0xB) {
                    tracing::trace!(
                        "[SUBSCRIBER] recv_datagram: immutable extension details: {:?}",
                        immutable_ext
                    );
                }
            }

            // Check for Prior Group ID Gap (type 0x3C = 60)
            if ext_headers.has(0x3C) {
                tracing::trace!(
                    "[SUBSCRIBER] recv_datagram: datagram contains PRIOR GROUP ID GAP (type 0x3C)"
                );
                if let Some(gap_ext) = ext_headers.get(0x3C) {
                    tracing::trace!(
                        "[SUBSCRIBER] recv_datagram: prior group id gap details: {:?}",
                        gap_ext
                    );
                }
            }
        }

        let bindings = self
            .resolve_alias(datagram.track_alias, Some(DEFAULT_ALIAS_WAIT_TIME_MS))
            .await?;
        let Some(bindings) = bindings else {
            tracing::warn!(
                "[SUBSCRIBER] recv_datagram: discarded due to unknown track_alias: track_alias={}, group_id={}, object_id={}, publisher_priority={}, status={}, payload_length={}",
                datagram.track_alias,
                datagram.group_id,
                datagram.object_id.unwrap_or(0),
                datagram.publisher_priority,
                datagram.status.as_ref().map_or("None".to_string(), |s| format!("{:?}", s)),
                datagram.payload.as_ref().map_or(0, |p| p.len()));
            return Ok(());
        };

        let object_id = datagram.object_id.unwrap_or(0);
        for binding in bindings {
            if !self.binding_claims_object(binding, datagram.group_id, object_id)? {
                continue;
            }
            match binding {
                AliasBinding::Subscribe(id) => {
                    let result = self
                        .subscribes
                        .lock()
                        .map_err(|_| SessionError::Internal)?
                        .get_mut(&id)
                        .map(|subscribe| subscribe.datagram(datagram.clone()));
                    if let Some(Err(err)) = result {
                        self.fail_alias_binding(binding, err);
                    }
                }
                AliasBinding::Publish(id) => {
                    let result = self
                        .publishes_received
                        .lock()
                        .map_err(|_| SessionError::Internal)?
                        .get_mut(&id)
                        .map(|publish| publish.datagram(datagram.clone()));
                    if let Some(Err(err)) = result {
                        self.fail_alias_binding(binding, err);
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod alias_tests {
    use super::*;

    fn full(namespace: &str, name: &str) -> serve::FullTrackName {
        serve::FullTrackName {
            namespace: TrackNamespace::from_utf8_path(namespace),
            name: TrackName::from(name),
        }
    }

    #[test]
    fn alias_fans_out_only_for_the_same_full_track_name() {
        let mut aliases = HashMap::new();
        Subscriber::insert_alias(
            &mut aliases,
            7,
            full("live", "audio"),
            AliasBinding::Subscribe(1),
        )
        .unwrap();
        Subscriber::insert_alias(
            &mut aliases,
            7,
            full("live", "audio"),
            AliasBinding::Publish(3),
        )
        .unwrap();

        assert_eq!(aliases[&7].bindings.len(), 2);
        assert!(matches!(
            Subscriber::insert_alias(
                &mut aliases,
                7,
                full("live", "video"),
                AliasBinding::Publish(5),
            ),
            Err(SessionError::Duplicate)
        ));
        assert_eq!(aliases[&7].bindings.len(), 2);
    }

    #[test]
    fn duplicate_request_binding_is_rejected_atomically() {
        let mut aliases = HashMap::new();
        let binding = AliasBinding::Publish(3);
        Subscriber::insert_alias(&mut aliases, 7, full("live", "audio"), binding).unwrap();
        assert!(matches!(
            Subscriber::insert_alias(&mut aliases, 7, full("live", "audio"), binding),
            Err(SessionError::Duplicate)
        ));
        assert_eq!(aliases[&7].bindings, vec![binding]);
    }

    #[test]
    fn removing_one_shared_binding_preserves_the_other_alias_route() {
        let mut aliases = HashMap::new();
        let subscribe = AliasBinding::Subscribe(1);
        let publish = AliasBinding::Publish(3);
        Subscriber::insert_alias(&mut aliases, 7, full("live", "audio"), subscribe).unwrap();
        Subscriber::insert_alias(&mut aliases, 7, full("live", "audio"), publish).unwrap();

        Subscriber::remove_alias_binding(&mut aliases, 7, subscribe);
        assert_eq!(aliases[&7].bindings, vec![publish]);
        Subscriber::remove_alias_binding(&mut aliases, 7, publish);
        assert!(!aliases.contains_key(&7));
    }
}

// TODO: Subscriber unit tests (`dropping_subscribe_removes_recv_state`,
// `remove_subscribe_clears_alias_map`) were removed because constructing
// a `Subscriber` requires a `web_transport::Session`, which in turn
// requires a live Quinn QUIC connection (there is no `Default` or
// mock constructor on `web_transport::Session` — it wraps
// `web_transport_quinn::Session` which holds a `quinn::Connection`).
// The tests verified that `Subscribe::Drop` removes the subscribes-map
// entry, and that `remove_subscribe` clears both `subscribes` and the
// shared exact-track alias registry. To restore them, either:
//   1. Add a `#[cfg(test)] pub fn stub(url: Url) -> Session` constructor
//      to `web_transport` (upstream crate) that creates a disconnected
//      session, or
//   2. Move these tests into an integration test that can spin up a
//      full Quinn loopback (moq-transport already depends on quinn
//      transitively, but the TLS ceremony requires `rustls` + `rcgen`
//      as direct dev-deps, which we want to avoid).
