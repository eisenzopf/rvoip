// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-FileCopyrightText: 2023-2024 Luke Curley and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::collections::HashMap;
use std::ops;
use std::sync::{Arc, Mutex};

use bytes::{Bytes, BytesMut};
use futures::stream::FuturesUnordered;
use futures::StreamExt;

use crate::coding::{Encode, KeyValuePairs, Location, ReasonPhrase, TrackName, TrackNamespace};
use crate::message::RequestErrorCode;
use crate::mlog;
#[cfg(test)]
use crate::serve::RetentionLimits;
use crate::serve::{
    RetainedObject, RetainedObjectMetadata, RetainedTrack, ServeError, TrackReaderMode,
};
use crate::watch::State;
use crate::{data, message, serve};

use super::{DeliveryFilter, Publisher, RequestLease, SessionError, SubscribeInfo, Writer};

// This file defines Publisher handling of inbound Subscriptions

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SubscriptionPhase {
    Pending,
    Established,
    Terminated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SubscriptionOperation {
    Establish,
    ObserveObject,
    UpdateForward,
    Terminate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
enum SubscriptionStateError {
    #[error("cannot {operation:?} a subscription in phase {phase:?}")]
    InvalidTransition {
        phase: SubscriptionPhase,
        operation: SubscriptionOperation,
    },
}

impl From<SubscriptionStateError> for ServeError {
    fn from(error: SubscriptionStateError) -> Self {
        ServeError::internal_ctx(error.to_string())
    }
}

#[derive(Debug)]
struct SubscribedState {
    largest_location: Option<Location>,
    joining_location: Option<Location>,
    stream_count: u64,
    phase: SubscriptionPhase,
    forward: bool,
    peer_rejected: bool,
    closed: Result<(), ServeError>,
}

impl SubscribedState {
    fn record_stream_opened(&mut self) {
        self.stream_count = self.stream_count.saturating_add(1);
    }

    fn ensure_phase(
        &self,
        expected: SubscriptionPhase,
        operation: SubscriptionOperation,
    ) -> Result<(), SubscriptionStateError> {
        if self.phase == expected {
            Ok(())
        } else {
            Err(SubscriptionStateError::InvalidTransition {
                phase: self.phase,
                operation,
            })
        }
    }

    fn establish(
        &mut self,
        communicated_largest: Option<Location>,
    ) -> Result<(), SubscriptionStateError> {
        self.ensure_phase(SubscriptionPhase::Pending, SubscriptionOperation::Establish)?;
        if let Some(location) = communicated_largest {
            self.observe_largest_location(location)?;
        }

        self.phase = SubscriptionPhase::Established;
        if self.forward {
            self.joining_location = self.largest_location;
        }

        Ok(())
    }

    fn observe_largest_location(
        &mut self,
        location: Location,
    ) -> Result<(), SubscriptionStateError> {
        if self.phase == SubscriptionPhase::Terminated {
            return Err(SubscriptionStateError::InvalidTransition {
                phase: self.phase,
                operation: SubscriptionOperation::ObserveObject,
            });
        }

        if self
            .largest_location
            .is_none_or(|current| location > current)
        {
            self.largest_location = Some(location);
        }

        Ok(())
    }

    fn update_largest_location(&mut self, group_id: u64, object_id: u64) -> Result<(), ServeError> {
        self.observe_largest_location(Location::new(group_id, object_id))?;

        Ok(())
    }

    fn update_forward(
        &mut self,
        forward: bool,
        communicated_largest: Option<Location>,
    ) -> Result<(), SubscriptionStateError> {
        self.ensure_phase(
            SubscriptionPhase::Established,
            SubscriptionOperation::UpdateForward,
        )?;

        if let Some(location) = communicated_largest {
            self.observe_largest_location(location)?;
        }
        if !self.forward && forward {
            self.joining_location = self.largest_location;
        }
        self.forward = forward;

        Ok(())
    }

    fn terminate(&mut self) -> Result<(), SubscriptionStateError> {
        if self.phase == SubscriptionPhase::Terminated {
            return Err(SubscriptionStateError::InvalidTransition {
                phase: self.phase,
                operation: SubscriptionOperation::Terminate,
            });
        }
        self.phase = SubscriptionPhase::Terminated;

        Ok(())
    }
}

impl Default for SubscribedState {
    fn default() -> Self {
        Self {
            largest_location: None,
            joining_location: None,
            stream_count: 0,
            phase: SubscriptionPhase::Pending,
            forward: true,
            peer_rejected: false,
            closed: Ok(()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct JoiningSubscription {
    pub request_id: u64,
    pub track_namespace: TrackNamespace,
    pub track_name: TrackName,
    pub joining_location: Location,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum JoiningSubscriptionLookup {
    Pending,
    Established(JoiningSubscription),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(super) enum JoiningSubscriptionLookupError {
    #[error("joining request ID does not identify a live subscriber-initiated subscription")]
    InvalidJoiningRequestId,
    #[error("joining subscription has Forward disabled")]
    ForwardDisabled,
    #[error("joining subscription has no saved Joining Location")]
    NoJoiningLocation,
    #[error("joining subscription registry is unavailable")]
    #[allow(dead_code)] // Constructed by Publisher's next-stage FETCH lookup entry point.
    Internal,
}

impl JoiningSubscriptionLookupError {
    #[allow(dead_code)] // Used by the next-stage FETCH response mapping.
    pub(super) fn request_error_code(self) -> message::RequestErrorCode {
        match self {
            Self::InvalidJoiningRequestId => message::RequestErrorCode::InvalidJoiningRequestId,
            Self::ForwardDisabled | Self::NoJoiningLocation => {
                message::RequestErrorCode::InvalidRange
            }
            Self::Internal => message::RequestErrorCode::InternalError,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SubscriptionInitiator {
    Subscriber,
    Publisher,
}

pub struct Subscribed {
    /// The sessions Publisher manager, used to send control messages,
    /// create new QUIC streams, and send datagrams
    publisher: Publisher,

    /// The tracknamespace and trackname for the subscription.
    pub info: SubscribeInfo,

    state: State<SubscribedState>,

    /// Bounded history for a Relative Joining FETCH that references this
    /// exact subscriber-initiated subscription.
    retained: Option<RetainedTrack>,

    /// Tracks if SubscribeOk has been sent yet or not. Used to send
    /// PUBLISH_DONE vs REQUEST_ERROR on drop.
    ok: bool,

    initiator: SubscriptionInitiator,

    /// Optional mlog writer for logging transport events
    mlog: Option<Arc<Mutex<mlog::MlogWriter>>>,
    _request_lease: Arc<RequestLease>,
}

enum SubgroupStreamFactory {
    Network(Box<Publisher>),
    #[cfg(test)]
    Recording(RecordingSubgroupStream),
}

impl SubgroupStreamFactory {
    async fn open(&mut self, priority: u8) -> Result<SubgroupStreamOutput, SessionError> {
        match self {
            Self::Network(publisher) => {
                let mut send_stream = publisher.open_uni().await?;
                send_stream.set_priority(priority as i32);

                let mut writer = Writer::new(send_stream);
                writer.reset_on_drop(super::Session::REQUEST_STREAM_CANCELLED);
                Ok(SubgroupStreamOutput::Network(writer))
            }
            #[cfg(test)]
            Self::Recording(stream) => {
                {
                    let mut state = stream.0.lock().map_err(|_| SessionError::Internal)?;
                    state.open_count += 1;
                    state.priority = Some(priority);
                }
                Ok(SubgroupStreamOutput::Recording(stream.clone()))
            }
        }
    }
}

enum SubgroupStreamOutput {
    Network(Writer),
    #[cfg(test)]
    Recording(RecordingSubgroupStream),
}

impl SubgroupStreamOutput {
    async fn encode<T: Encode>(&mut self, message: &T) -> Result<(), SessionError> {
        match self {
            Self::Network(writer) => writer.encode(message).await,
            #[cfg(test)]
            Self::Recording(stream) => {
                let mut state = stream.0.lock().map_err(|_| SessionError::Internal)?;
                message.encode(&mut state.bytes)?;
                Ok(())
            }
        }
    }

    async fn write(&mut self, payload: &[u8]) -> Result<(), SessionError> {
        match self {
            Self::Network(writer) => writer.write(payload).await,
            #[cfg(test)]
            Self::Recording(stream) => {
                let mut state = stream.0.lock().map_err(|_| SessionError::Internal)?;
                state.bytes.extend_from_slice(payload);
                Ok(())
            }
        }
    }

    fn finish(&mut self) -> Result<(), SessionError> {
        match self {
            Self::Network(writer) => {
                writer.finish();
                Ok(())
            }
            #[cfg(test)]
            Self::Recording(stream) => {
                stream
                    .0
                    .lock()
                    .map_err(|_| SessionError::Internal)?
                    .finished = true;
                Ok(())
            }
        }
    }
}

#[cfg(test)]
#[derive(Clone, Default)]
struct RecordingSubgroupStream(Arc<Mutex<RecordingSubgroupStreamState>>);

#[cfg(test)]
#[derive(Default)]
struct RecordingSubgroupStreamState {
    bytes: bytes::BytesMut,
    open_count: usize,
    priority: Option<u8>,
    finished: bool,
}

impl Subscribed {
    fn subgroup_header_type(first_object: bool, end_of_group: bool) -> data::StreamHeaderType {
        data::StreamHeaderType::subgroup(
            true,
            data::SubgroupIdMode::Explicit,
            end_of_group,
            false,
            first_object,
        )
    }

    pub(super) fn new(
        publisher: Publisher,
        msg: message::Subscribe,
        mlog: Option<Arc<Mutex<mlog::MlogWriter>>>,
        request_lease: Arc<RequestLease>,
    ) -> Result<(Self, SubscribedRecv), SessionError> {
        let info = SubscribeInfo::new_from_subscribe(&msg)?;
        let initial = SubscribedState {
            forward: info.forward,
            ..Default::default()
        };
        let (send, recv) = State::new(initial).split();
        let retained = RetainedTrack::new_with_budget(
            publisher.retention_track_limits(),
            publisher.retention_budget(),
        )
        .map_err(|error| SessionError::Serve(ServeError::internal_ctx(error.to_string())))?;
        let recv_info = info.clone();
        let send = Self {
            publisher,
            state: send,
            retained: Some(retained.clone()),
            info,
            ok: false,
            initiator: SubscriptionInitiator::Subscriber,
            mlog,
            _request_lease: request_lease.clone(),
        };

        // Prevents updates after being closed
        let recv = SubscribedRecv {
            state: recv,
            info: recv_info,
            retained: Some(retained),
            _request_lease: request_lease,
        };

        Ok((send, recv))
    }

    /// Build the data-plane state for an outbound PUBLISH request.
    pub(super) fn new_published(
        publisher: Publisher,
        msg: &message::Publish,
        mlog: Option<Arc<Mutex<mlog::MlogWriter>>>,
        request_lease: Arc<RequestLease>,
    ) -> Result<(Self, SubscribedRecv), SessionError> {
        let synthetic = message::Subscribe {
            id: msg.id,
            track_namespace: msg.track_namespace.clone(),
            track_name: msg.track_name.clone(),
            params: msg.params.clone(),
        };
        let info = SubscribeInfo::new_from_subscribe(&synthetic)?;
        let forward = msg.params.forward()?.unwrap_or(true);
        let initial = SubscribedState {
            largest_location: msg.params.largest_object()?,
            forward,
            ..Default::default()
        };
        let (send, recv) = State::new(initial).split();
        let recv_info = info.clone();
        let published = Self {
            publisher,
            state: send,
            retained: None,
            info,
            ok: false,
            initiator: SubscriptionInitiator::Publisher,
            mlog,
            _request_lease: request_lease.clone(),
        };
        Ok((
            published,
            SubscribedRecv {
                state: recv,
                info: recv_info,
                retained: None,
                _request_lease: request_lease,
            },
        ))
    }

    pub async fn serve(mut self, track: serve::TrackReader) -> Result<(), SessionError> {
        let res = self.serve_inner(track).await;
        if let Err(err) = &res {
            self.close(err.clone().into())?;
        }

        res
    }

    async fn serve_inner(&mut self, track: serve::TrackReader) -> Result<(), SessionError> {
        // Update largest location before sending SubscribeOk
        let largest_location = track.largest_location();
        // Send SubscribeOk using send_message_and_wait to ensure it is sent at least to the QUIC stack before
        // we start serving the track.  If a subscriber gets the stream before SubscribeOk
        // then they won't recognize the track_alias in the stream header.
        let mut params = KeyValuePairs::default();
        if let Some(largest) = largest_location {
            params
                .set_largest_object(largest)
                .map_err(|_| SessionError::Internal)?;
        }

        self.publisher
            .send_message_and_wait(message::SubscribeOk {
                id: self.info.id,
                track_alias: self.info.id, // use subscription id as track alias
                params,
                track_extensions: Default::default(),
            })
            .await;

        self.state
            .lock_mut()
            .ok_or(ServeError::Cancel)?
            .establish(largest_location)
            .map_err(ServeError::from)?;
        self.ok = true; // So we send SubscribeDone on drop

        let mut delivery_filter = self.info.delivery_filter(largest_location);
        // FORWARD is mutable via REQUEST_UPDATE and is enforced from shared state.
        delivery_filter.forward = true;

        // Serve based on track mode
        let mode = tokio::select! {
            mode = track.mode() => mode?,
            closed = self.closed() => return Ok(closed?),
        };
        match mode {
            // TODO cancel track/datagrams on closed
            TrackReaderMode::Stream(_stream) => panic!("deprecated"),
            TrackReaderMode::Subgroups(subgroups) => {
                self.serve_subgroups(subgroups, delivery_filter).await
            }
            TrackReaderMode::Datagrams(datagrams) => {
                self.serve_datagrams(datagrams, delivery_filter).await
            }
        }
    }

    pub(super) async fn publish_ok(&self) -> Result<(), ServeError> {
        loop {
            {
                let state = self.state.lock();
                state.closed.clone()?;
                match state.phase {
                    SubscriptionPhase::Pending => {}
                    SubscriptionPhase::Established => return Ok(()),
                    SubscriptionPhase::Terminated => return Err(ServeError::Done),
                }
                match state.modified() {
                    Some(notify) => notify,
                    None => return Err(ServeError::Done),
                }
            }
            .await;
        }
    }

    pub(super) async fn serve_published(
        &mut self,
        track: serve::TrackReader,
    ) -> Result<(), SessionError> {
        let result = self.serve_published_inner(track).await;
        if let Err(err) = &result {
            self.close_state(err.clone().into())?;
        }
        result
    }

    async fn serve_published_inner(
        &mut self,
        track: serve::TrackReader,
    ) -> Result<(), SessionError> {
        debug_assert_eq!(self.initiator, SubscriptionInitiator::Publisher);
        self.publish_ok().await?;
        self.ok = true;

        let largest_location = track.largest_location();
        {
            let mut state = self.state.lock_mut().ok_or(ServeError::Cancel)?;
            if let Some(location) = largest_location {
                state
                    .observe_largest_location(location)
                    .map_err(ServeError::from)?;
            }
        }
        let delivery_filter = DeliveryFilter {
            forward: true,
            start_location: None,
            end_group_id: None,
        };

        let mode = tokio::select! {
            mode = track.mode() => mode?,
            closed = self.closed() => return Ok(closed?),
        };
        match mode {
            TrackReaderMode::Stream(_stream) => Err(SessionError::Serve(
                ServeError::not_implemented_ctx("stream track reader mode"),
            )),
            TrackReaderMode::Subgroups(subgroups) => {
                self.serve_subgroups(subgroups, delivery_filter).await
            }
            TrackReaderMode::Datagrams(datagrams) => {
                self.serve_datagrams(datagrams, delivery_filter).await
            }
        }
    }

    pub fn close(self, err: ServeError) -> Result<(), ServeError> {
        self.close_state(err)
    }

    pub(super) fn cancel_request_stream(&mut self) {
        if let Some(mut state) = self.state.lock_mut() {
            state.peer_rejected = true;
            let _ = state.terminate();
            state.closed = Err(ServeError::Cancel);
        }
        self.publisher
            .cancel_request_stream(self.info.id, super::Session::REQUEST_STREAM_CANCELLED);
    }

    fn close_state(&self, err: ServeError) -> Result<(), ServeError> {
        let state = self.state.lock();
        state.closed.clone()?;

        let mut state = state.into_mut().ok_or(ServeError::Done)?;
        state.terminate()?;
        state.closed = Err(err);

        Ok(())
    }

    pub async fn closed(&self) -> Result<(), ServeError> {
        loop {
            {
                let state = self.state.lock();
                state.closed.clone()?;

                match state.modified() {
                    Some(notify) => notify,
                    None => return Ok(()),
                }
            }
            .await;
        }
    }
}

impl ops::Deref for Subscribed {
    type Target = SubscribeInfo;

    fn deref(&self) -> &Self::Target {
        &self.info
    }
}

impl Drop for Subscribed {
    fn drop(&mut self) {
        self._request_lease.release();
        let state = self.state.lock();
        let err = state
            .closed
            .as_ref()
            .err()
            .cloned()
            .unwrap_or(ServeError::Done);
        let stream_count = state.stream_count;
        let peer_rejected = state.peer_rejected;
        if let Some(mut state) = state.into_mut() {
            let _ = state.terminate();
        }

        if peer_rejected {
            if self.initiator == SubscriptionInitiator::Publisher {
                self.publisher.drop_published(self.info.id);
            } else {
                self.publisher.drop_subscribe(self.info.id);
            }
            return;
        }

        if self.initiator == SubscriptionInitiator::Publisher || self.ok {
            self.publisher.send_message(message::PublishDone {
                id: self.info.id,
                status_code: Self::publish_done_code(&err),
                stream_count,
                reason: ReasonPhrase(err.to_string()),
            });
        } else {
            // Draft-16 §9.8: subscription rejection uses REQUEST_ERROR, not the
            // legacy SUBSCRIBE_ERROR.
            self.publisher.send_request_error(
                "subscribe",
                message::RequestError {
                    id: self.info.id,
                    error_code: Self::request_error_code(&err),
                    retry_interval: Self::request_error_retry_interval(&err),
                    reason: ReasonPhrase(err.to_string()),
                    redirect: None,
                },
            );
            self.publisher.drop_subscribe(self.info.id);
        };
    }
}

impl Subscribed {
    fn publish_done_code(err: &ServeError) -> u64 {
        match err {
            ServeError::Done => message::PublishDoneCode::TrackEnded as u64,
            ServeError::Closed(code) => *code,
            _ => message::PublishDoneCode::InternalError as u64,
        }
    }

    fn request_error_code(err: &ServeError) -> u64 {
        match err {
            ServeError::Closed(code) => *code,
            ServeError::NotFound | ServeError::NotFoundWithId(_, _) => {
                RequestErrorCode::DoesNotExist as u64
            }
            // Duplicate is an application policy result in draft-19; the
            // protocol explicitly allows multiple subscriptions per track.
            ServeError::Duplicate => RequestErrorCode::Uninterested as u64,
            ServeError::Cancel | ServeError::Done => RequestErrorCode::Uninterested as u64,
            ServeError::Mode
            | ServeError::Size
            | ServeError::NotImplemented(_)
            | ServeError::NotImplementedWithId(_, _) => RequestErrorCode::NotSupported as u64,
            ServeError::Internal(_) | ServeError::InternalWithId(_, _) => {
                RequestErrorCode::InternalError as u64
            }
        }
    }

    fn request_error_retry_interval(err: &ServeError) -> u64 {
        match err {
            ServeError::Closed(code) if *code == RequestErrorCode::ExcessiveLoad as u64 => {
                // Draft-19 encodes the minimum delay in milliseconds plus one.
                1001
            }
            _ => 0,
        }
    }

    fn is_expected_serve_shutdown(err: &SessionError) -> bool {
        matches!(
            err,
            SessionError::Serve(ServeError::Cancel | ServeError::Done)
        )
    }

    async fn serve_subgroups(
        &mut self,
        mut subgroups: serve::SubgroupsReader,
        delivery_filter: DeliveryFilter,
    ) -> Result<(), SessionError> {
        let mut tasks = FuturesUnordered::new();
        let mut done: Option<Result<(), ServeError>> = None;

        loop {
            tokio::select! {
                res = subgroups.next(), if done.is_none() => match res {
                    Ok(Some(subgroup)) => {
                        let header = data::SubgroupHeader {
                            header_type: Self::subgroup_header_type(
                                subgroup.first_object,
                                subgroup.end_of_group,
                            ),
                            track_alias: self.info.id, // use subscription id as track_alias
                            group_id: subgroup.group_id,
                            subgroup_id: Some(subgroup.subgroup_id),
                            publisher_priority: subgroup.priority,
                        };

                        let publisher = self.publisher.clone();
                        let state = self.state.clone();
                        let retained = self.retained.clone();
                        let info = subgroup.info.clone();
                        let mlog = self.mlog.clone();

                        tasks.push(async move {
                            if let Err(err) = Self::serve_subgroup(header, subgroup, publisher, state, retained, mlog, delivery_filter).await {
                                if Self::is_expected_serve_shutdown(&err) {
                                    tracing::debug!(subgroup_info = ?info, error = %err, "stopped serving subgroup");
                                } else {
                                    tracing::warn!(subgroup_info = ?info, error = %err, "failed to serve subgroup");
                                }
                            }
                        });
                    },
                    Ok(None) => done = Some(Ok(())),
                    Err(err) => return Err(err.into()),
                },
                res = self.closed(), if done.is_none() => return Ok(res?),
                _ = tasks.next(), if !tasks.is_empty() => {},
                else => return Ok(done.unwrap()?),
            }
        }
    }

    async fn wait_until_forward(state: &State<SubscribedState>) -> Result<(), ServeError> {
        loop {
            let notified = {
                let state = state.lock();
                state.closed.clone()?;
                if state.forward {
                    return Ok(());
                }
                state.modified().ok_or(ServeError::Done)?
            };
            notified.await;
        }
    }

    async fn serve_subgroup(
        header: data::SubgroupHeader,
        subgroup_reader: serve::SubgroupReader,
        publisher: Publisher,
        state: State<SubscribedState>,
        retained: Option<RetainedTrack>,
        mlog: Option<Arc<Mutex<mlog::MlogWriter>>>,
        delivery_filter: DeliveryFilter,
    ) -> Result<(), SessionError> {
        Self::serve_subgroup_with_factory(
            header,
            subgroup_reader,
            SubgroupStreamFactory::Network(Box::new(publisher)),
            state,
            retained,
            mlog,
            delivery_filter,
        )
        .await
    }

    async fn serve_subgroup_with_factory(
        header: data::SubgroupHeader,
        mut subgroup_reader: serve::SubgroupReader,
        mut stream_factory: SubgroupStreamFactory,
        state: State<SubscribedState>,
        retained: Option<RetainedTrack>,
        mlog: Option<Arc<Mutex<mlog::MlogWriter>>>,
        delivery_filter: DeliveryFilter,
    ) -> Result<(), SessionError> {
        tracing::trace!(
            "[PUBLISHER] serve_subgroup: starting - group_id={}, subgroup_id={:?}, priority={}",
            subgroup_reader.group_id,
            subgroup_reader.subgroup_id,
            subgroup_reader.priority
        );

        let mut writer: Option<SubgroupStreamOutput> = None;
        let mut object_count = 0;
        let mut retention_valid = true;
        loop {
            Self::wait_until_forward(&state).await?;
            let Some(mut subgroup_object_reader) = subgroup_reader.next().await? else {
                break;
            };
            // FORWARD may have changed while waiting for the next object.
            Self::wait_until_forward(&state).await?;
            let deliver =
                delivery_filter.allows(subgroup_reader.group_id, subgroup_object_reader.object_id);
            if !deliver {
                tracing::trace!(
                    "[PUBLISHER] serve_subgroup: filtered object group_id={}, object_id={}",
                    subgroup_reader.group_id,
                    subgroup_object_reader.object_id
                );
            }

            if deliver && writer.is_none() {
                let mut new_writer = stream_factory.open(subgroup_reader.priority).await?;
                tracing::trace!("[PUBLISHER] serve_subgroup: opened unidirectional stream");

                state
                    .lock_mut()
                    .ok_or(ServeError::Done)?
                    .record_stream_opened();

                tracing::trace!(
                    "[PUBLISHER] serve_subgroup: sending header - track_alias={}, group_id={}, subgroup_id={:?}, priority={}, header_type={:?}",
                    header.track_alias,
                    header.group_id,
                    header.subgroup_id,
                    header.publisher_priority,
                    header.header_type
                );

                new_writer.encode(&header).await?;

                // Log subgroup header created/sent
                if let Some(ref mlog) = mlog {
                    if let Ok(mut mlog_guard) = mlog.lock() {
                        let time = mlog_guard.elapsed_ms();
                        let stream_id = 0; // TODO: Placeholder, need actual QUIC stream ID
                        let event = mlog::subgroup_header_created(time, stream_id, &header);
                        let _ = mlog_guard.add_event(event);
                    }
                }

                writer = Some(new_writer);
            }

            let subgroup_object = data::SubgroupObjectExt {
                // TODO(itzmanish): compute real delta when the receive side uses object IDs
                // for ordering. Both sender and receiver must agree on the same prev tracking
                // semantics before this is meaningful.
                object_id_delta: 0,
                extension_headers: subgroup_object_reader.extension_headers.clone(), // Pass through extension headers
                payload_length: subgroup_object_reader.size,
                status: if subgroup_object_reader.size == 0 {
                    // Only set status if payload length is zero
                    Some(subgroup_object_reader.status)
                } else {
                    None
                },
            };

            tracing::trace!(
                "[PUBLISHER] serve_subgroup: sending object #{} - object_id={}, object_id_delta={}, payload_length={}, status={:?}, extension_headers={:?}",
                object_count + 1,
                subgroup_object_reader.object_id,
                subgroup_object.object_id_delta,
                subgroup_object.payload_length,
                subgroup_object.status,
                subgroup_object.extension_headers
            );

            if deliver {
                writer
                    .as_mut()
                    .ok_or(SessionError::Internal)?
                    .encode(&subgroup_object)
                    .await?;
            }

            // Log subgroup object created/sent
            if let Some(ref mlog) = mlog {
                if let Ok(mut mlog_guard) = mlog.lock() {
                    let time = mlog_guard.elapsed_ms();
                    let stream_id = 0; // TODO: Placeholder, need actual QUIC stream ID
                    let event = mlog::subgroup_object_ext_created(
                        time,
                        stream_id,
                        subgroup_reader.group_id,
                        subgroup_reader.subgroup_id,
                        subgroup_object_reader.object_id,
                        &subgroup_object,
                    );
                    let _ = mlog_guard.add_event(event);
                }
            }

            if deliver {
                state
                    .lock_mut()
                    .ok_or(ServeError::Done)?
                    .update_largest_location(
                        subgroup_reader.group_id,
                        subgroup_object_reader.object_id,
                    )?;
            }

            let mut chunks_sent = 0;
            let mut bytes_sent = 0;
            let retention_object_limit = retained
                .as_ref()
                .map(RetainedTrack::limits)
                .map(|limits| limits.max_object_bytes)
                .unwrap_or(0);
            let retain_object = retained.is_some()
                && subgroup_object_reader.status == data::ObjectStatus::NormalObject
                && subgroup_object_reader.size <= retention_object_limit;
            let mut retained_payload =
                retain_object.then(|| BytesMut::with_capacity(subgroup_object_reader.size));
            while let Some(chunk) = subgroup_object_reader.read().await? {
                tracing::trace!(
                    "[PUBLISHER] serve_subgroup: sending payload chunk #{} for object #{} ({} bytes)",
                    chunks_sent + 1,
                    object_count + 1,
                    chunk.len()
                );
                bytes_sent += chunk.len();
                if let Some(payload) = retained_payload.as_mut() {
                    payload.extend_from_slice(&chunk);
                }
                if deliver {
                    writer
                        .as_mut()
                        .ok_or(SessionError::Internal)?
                        .write(&chunk)
                        .await?;
                }
                chunks_sent += 1;
            }

            if let Some(payload) = retained_payload {
                let retained_object = RetainedObject::new(
                    RetainedObjectMetadata {
                        location: Location::new(
                            subgroup_reader.group_id,
                            subgroup_object_reader.object_id,
                        ),
                        subgroup_id: Some(subgroup_reader.subgroup_id),
                        publisher_priority: subgroup_reader.priority,
                        properties: subgroup_object_reader.extension_headers.clone(),
                        end_of_group: subgroup_reader.end_of_group,
                    },
                    Bytes::from(payload),
                );
                let retained = retained
                    .as_ref()
                    .expect("retained payloads require a configured cache");
                match retained_object.and_then(|object| retained.commit(object)) {
                    Ok(()) => {}
                    Err(error) => {
                        retention_valid = false;
                        tracing::debug!(
                            group_id = subgroup_reader.group_id,
                            object_id = subgroup_object_reader.object_id,
                            %error,
                            "Object remains live but is unavailable to Joining FETCH retention"
                        );
                    }
                }
            } else if retained.is_some() {
                retention_valid = false;
            }

            tracing::trace!(
                "[PUBLISHER] serve_subgroup: completed object #{} ({} chunks, {} bytes total)",
                object_count + 1,
                chunks_sent,
                bytes_sent
            );
            object_count += 1;
        }

        tracing::trace!(
            "[PUBLISHER] serve_subgroup: completed subgroup (group_id={}, subgroup_id={:?}, {} objects sent)",
            subgroup_reader.group_id,
            subgroup_reader.subgroup_id,
            object_count
        );

        if let Some(mut writer) = writer {
            writer.finish()?;
        }

        if let Some(retained) = retained {
            if subgroup_reader.end_of_group && retention_valid {
                if let Err(error) = retained.complete_group(subgroup_reader.group_id) {
                    tracing::debug!(
                        group_id = subgroup_reader.group_id,
                        %error,
                        "completed live Group was not promoted into Joining FETCH retention"
                    );
                    retained.discard_pending(subgroup_reader.group_id);
                }
            } else if !retention_valid {
                retained.discard_pending(subgroup_reader.group_id);
            }
        }

        Ok(())
    }

    async fn serve_datagrams(
        &mut self,
        mut datagrams: serve::DatagramsReader,
        delivery_filter: DeliveryFilter,
    ) -> Result<(), SessionError> {
        tracing::debug!("[PUBLISHER] serve_datagrams: starting");

        let mut datagram_count = 0;
        loop {
            Self::wait_until_forward(&self.state).await?;
            let next = tokio::select! {
                value = datagrams.read() => value,
                closed = self.closed() => return Ok(closed?),
            }?;
            let Some(datagram) = next else {
                break;
            };
            // FORWARD may have changed while waiting for the next datagram.
            Self::wait_until_forward(&self.state).await?;
            if !delivery_filter.allows(datagram.group_id, datagram.object_id) {
                tracing::trace!(
                    "[PUBLISHER] serve_datagrams: filtered datagram group_id={}, object_id={}",
                    datagram.group_id,
                    datagram.object_id
                );
                continue;
            }

            // Determine datagram type based on extension headers presence
            let has_extension_headers = !datagram.extension_headers.is_empty();
            let datagram_type = if has_extension_headers {
                data::DatagramType::ObjectIdPayloadExt
            } else {
                data::DatagramType::ObjectIdPayload
            };

            let encoded_datagram = data::Datagram {
                datagram_type,
                track_alias: self.info.id, // use subscription id as track_alias
                group_id: datagram.group_id,
                object_id: Some(datagram.object_id),
                publisher_priority: datagram.priority,
                extension_headers: if has_extension_headers {
                    Some(datagram.extension_headers.clone())
                } else {
                    None
                },
                status: None,
                payload: Some(datagram.payload),
            };

            let payload_len = encoded_datagram
                .payload
                .as_ref()
                .map(|p| p.len())
                .unwrap_or(0);
            let mut buffer = bytes::BytesMut::with_capacity(payload_len + 100);
            encoded_datagram.encode(&mut buffer)?;

            tracing::trace!(
                "[PUBLISHER] serve_datagrams: sending datagram #{} - track_alias={}, group_id={}, object_id={}, priority={}, payload_len={}, extension_headers={:?}, total_encoded_len={}",
                datagram_count + 1,
                encoded_datagram.track_alias,
                encoded_datagram.group_id,
                encoded_datagram.object_id.unwrap(),
                encoded_datagram.publisher_priority,
                payload_len,
                encoded_datagram.extension_headers,
                buffer.len()
            );

            // Create mlog event for datagram created
            if let Some(ref mlog) = self.mlog {
                if let Ok(mut mlog_guard) = mlog.lock() {
                    let time = mlog_guard.elapsed_ms();
                    let stream_id = 0; // TODO: Placeholder, need actual QUIC stream ID
                    let _ = mlog_guard.add_event(mlog::object_datagram_created(
                        time,
                        stream_id,
                        &encoded_datagram,
                    ));
                }
            }

            self.publisher.send_datagram(buffer.into()).await?;

            self.state
                .lock_mut()
                .ok_or(ServeError::Done)?
                .update_largest_location(
                    encoded_datagram.group_id,
                    encoded_datagram.object_id.unwrap(),
                )?;

            datagram_count += 1;
        }

        tracing::trace!(
            "[PUBLISHER] serve_datagrams: completed ({} datagrams sent)",
            datagram_count
        );

        Ok(())
    }
}

#[derive(Clone)]
pub(super) struct SubscribedRecv {
    state: State<SubscribedState>,
    info: SubscribeInfo,
    retained: Option<RetainedTrack>,
    _request_lease: Arc<RequestLease>,
}

impl SubscribedRecv {
    pub(super) fn release_request_lease(&self) {
        self._request_lease.release();
    }

    pub fn recv_publish_ok(&mut self, msg: &message::RequestOk) -> Result<(), ServeError> {
        let forward = msg
            .params
            .forward()
            .map_err(|_| ServeError::internal_ctx("invalid FORWARD in PUBLISH_OK"))?;
        let mut state = self.state.lock_mut().ok_or(ServeError::Done)?;
        state.closed.clone()?;
        state.ensure_phase(SubscriptionPhase::Pending, SubscriptionOperation::Establish)?;
        if let Some(forward) = forward {
            state.forward = forward;
        }
        state.establish(None)?;
        Ok(())
    }

    pub fn recv_error(&mut self, err: ServeError) -> Result<(), ServeError> {
        let mut state = self.state.lock_mut().ok_or(ServeError::Done)?;
        state.closed.clone()?;
        state.terminate()?;
        state.peer_rejected = true;
        state.closed = Err(err);
        Ok(())
    }

    pub fn recv_update_failed(&mut self) -> Result<(), ServeError> {
        let state = self.state.lock();
        state.closed.clone()?;

        if let Some(mut state) = state.into_mut() {
            state.terminate()?;
            state.closed = Err(ServeError::Closed(
                message::PublishDoneCode::UpdateFailed as u64,
            ));
        }
        Ok(())
    }

    pub fn recv_forward_update(&mut self, forward: bool) -> Result<(), ServeError> {
        let mut state = self.state.lock_mut().ok_or(ServeError::Done)?;
        state.closed.clone()?;
        state.update_forward(forward, None)?;
        Ok(())
    }

    fn joining_fetch_state(
        &self,
    ) -> Result<JoiningSubscriptionLookup, JoiningSubscriptionLookupError> {
        let state = self.state.lock();
        joining_fetch_state(&state, &self.info)
    }

    pub(super) async fn wait_for_joining_fetch(
        &self,
    ) -> Result<(JoiningSubscription, RetainedTrack), JoiningSubscriptionLookupError> {
        loop {
            let notified = {
                let state = self.state.lock();
                match joining_fetch_state(&state, &self.info)? {
                    JoiningSubscriptionLookup::Established(subscription) => {
                        return Ok((
                            subscription,
                            self.retained
                                .clone()
                                .ok_or(JoiningSubscriptionLookupError::Internal)?,
                        ));
                    }
                    JoiningSubscriptionLookup::Pending => state
                        .modified()
                        .ok_or(JoiningSubscriptionLookupError::InvalidJoiningRequestId)?,
                }
            };
            notified.await;
        }
    }
}

fn joining_fetch_state(
    state: &SubscribedState,
    info: &SubscribeInfo,
) -> Result<JoiningSubscriptionLookup, JoiningSubscriptionLookupError> {
    match state.phase {
        SubscriptionPhase::Pending => Ok(JoiningSubscriptionLookup::Pending),
        SubscriptionPhase::Terminated => {
            Err(JoiningSubscriptionLookupError::InvalidJoiningRequestId)
        }
        SubscriptionPhase::Established if !state.forward => {
            Err(JoiningSubscriptionLookupError::ForwardDisabled)
        }
        SubscriptionPhase::Established => {
            let joining_location = state
                .joining_location
                .ok_or(JoiningSubscriptionLookupError::NoJoiningLocation)?;
            Ok(JoiningSubscriptionLookup::Established(
                JoiningSubscription {
                    request_id: info.id,
                    track_namespace: info.track_namespace.clone(),
                    track_name: info.track_name.clone(),
                    joining_location,
                },
            ))
        }
    }
}

pub(super) fn lookup_joining_subscription(
    subscriptions: &HashMap<u64, SubscribedRecv>,
    request_id: u64,
) -> Result<JoiningSubscriptionLookup, JoiningSubscriptionLookupError> {
    subscriptions
        .get(&request_id)
        .ok_or(JoiningSubscriptionLookupError::InvalidJoiningRequestId)?
        .joining_fetch_state()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coding::Decode;

    fn subscribe_info(request_id: u64) -> SubscribeInfo {
        SubscribeInfo::new_from_subscribe(&message::Subscribe {
            id: request_id,
            track_namespace: TrackNamespace::from_utf8_path("test/session"),
            track_name: "audio".into(),
            params: KeyValuePairs::default(),
        })
        .unwrap()
    }

    fn recv_pair(
        initial: SubscribedState,
        request_id: u64,
    ) -> (State<SubscribedState>, SubscribedRecv) {
        let (send, recv) = State::new(initial).split();
        (
            send,
            SubscribedRecv {
                state: recv,
                info: subscribe_info(request_id),
                retained: Some(RetainedTrack::new(RetentionLimits::default()).unwrap()),
                _request_lease: crate::session::test_request_lease(
                    crate::session::RequestDirection::Inbound,
                    crate::session::RequestClass::Subscribe,
                ),
            },
        )
    }

    #[test]
    fn subscribed_state_counts_opened_streams() {
        let mut state = SubscribedState::default();
        assert_eq!(state.stream_count, 0);

        state.record_stream_opened();
        assert_eq!(state.stream_count, 1);

        state.record_stream_opened();
        assert_eq!(state.stream_count, 2);
    }

    #[test]
    fn publish_ok_updates_forward_and_acceptance() {
        let (_send, mut recv) = recv_pair(SubscribedState::default(), 7);
        let mut params = KeyValuePairs::default();
        params.set_forward(false);
        recv.recv_publish_ok(&message::RequestOk {
            id: 7,
            params,
            track_properties: Default::default(),
        })
        .unwrap();
        let state = recv.state.lock();
        assert_eq!(state.phase, SubscriptionPhase::Established);
        assert!(!state.forward);
        assert_eq!(state.joining_location, None);
    }

    #[test]
    fn reverse_request_update_changes_live_forward_state() {
        let mut initial = SubscribedState::default();
        initial.establish(Some(Location::new(3, 4))).unwrap();
        let (_send, mut recv) = recv_pair(initial, 7);
        recv.recv_forward_update(false).unwrap();
        assert!(!recv.state.lock().forward);
        recv.recv_forward_update(true).unwrap();
        assert!(recv.state.lock().forward);
    }

    #[test]
    fn failed_reverse_update_marks_terminal_state_without_snapshotting_early() {
        let mut initial = SubscribedState::default();
        initial.record_stream_opened();
        let (_send, mut recv) = recv_pair(initial, 7);

        recv.recv_update_failed().unwrap();
        let state = recv.state.lock();
        assert_eq!(state.phase, SubscriptionPhase::Terminated);
        assert_eq!(state.stream_count, 1);
        assert!(matches!(
            state.closed,
            Err(ServeError::Closed(code))
                if code == message::PublishDoneCode::UpdateFailed as u64
        ));
    }

    #[test]
    fn subgroup_header_preserves_draft19_first_and_end_semantics() {
        for first_object in [false, true] {
            for end_of_group in [false, true] {
                let header_type = Subscribed::subgroup_header_type(first_object, end_of_group);
                assert_eq!(header_type.is_first_object(), first_object);
                assert_eq!(header_type.contains_end_of_group(), end_of_group);
            }
        }

        let header_type = Subscribed::subgroup_header_type(true, true);
        assert_eq!(header_type.value(), 0x5d);
        let header = data::SubgroupHeader {
            header_type,
            track_alias: 2,
            group_id: 3,
            subgroup_id: Some(4),
            publisher_priority: 5,
        };
        let mut wire = bytes::BytesMut::new();
        header.encode(&mut wire).unwrap();
        assert_eq!(wire.as_ref(), &[0x5d, 0x02, 0x03, 0x04, 0x05]);
    }

    #[tokio::test]
    async fn one_object_complete_group_sets_both_bits_and_finishes_stream() {
        let subgroup = serve::SubgroupInfo {
            track: Arc::new(serve::Track::new(
                TrackNamespace::from_utf8_path("test/session"),
                "audio",
            )),
            group_id: 3,
            subgroup_id: 4,
            priority: 5,
            first_object: true,
            end_of_group: true,
        };
        let (mut subgroup_writer, subgroup_reader) = subgroup.produce();
        subgroup_writer
            .write(bytes::Bytes::from_static(b"opus"))
            .unwrap();
        drop(subgroup_writer);

        let header_type = Subscribed::subgroup_header_type(
            subgroup_reader.first_object,
            subgroup_reader.end_of_group,
        );
        let header = data::SubgroupHeader {
            header_type,
            track_alias: 2,
            group_id: subgroup_reader.group_id,
            subgroup_id: Some(subgroup_reader.subgroup_id),
            publisher_priority: subgroup_reader.priority,
        };
        let recording = RecordingSubgroupStream::default();
        let subscribed_state = State::default();
        let retained = RetainedTrack::new(RetentionLimits::default()).unwrap();

        Subscribed::serve_subgroup_with_factory(
            header,
            subgroup_reader,
            SubgroupStreamFactory::Recording(recording.clone()),
            subscribed_state.clone(),
            Some(retained.clone()),
            None,
            DeliveryFilter {
                forward: true,
                start_location: None,
                end_group_id: None,
            },
        )
        .await
        .unwrap();

        let state = recording.0.lock().unwrap();
        assert_eq!(state.open_count, 1);
        assert_eq!(state.priority, Some(5));
        assert!(state.finished, "a complete subgroup must send stream FIN");

        let mut wire = state.bytes.clone();
        drop(state);
        let decoded_type = data::StreamHeaderType::decode(&mut wire).unwrap();
        assert_eq!(decoded_type.value(), 0x5d);
        assert!(decoded_type.is_first_object());
        assert!(decoded_type.contains_end_of_group());

        let decoded_header = data::SubgroupHeader::decode(decoded_type, &mut wire).unwrap();
        assert_eq!(decoded_header.track_alias, 2);
        assert_eq!(decoded_header.group_id, 3);
        assert_eq!(decoded_header.subgroup_id, Some(4));
        assert_eq!(decoded_header.publisher_priority, 5);

        let object = data::SubgroupObjectExt::decode(&mut wire).unwrap();
        assert_eq!(object.object_id_delta, 0);
        assert_eq!(object.payload_length, 4);
        assert_eq!(wire.as_ref(), b"opus");

        let state = subscribed_state.lock();
        assert_eq!(state.stream_count, 1);
        assert_eq!(state.largest_location, Some(Location::new(3, 0)));
        drop(state);
        let snapshot = retained
            .snapshot(
                crate::serve::RetainedRange::new(Location::new(3, 0), Location::new(3, 1)),
                message::GroupOrder::Ascending,
            )
            .unwrap();
        let object = snapshot.iter().next().unwrap();
        assert_eq!(object.subgroup_id(), Some(4));
        assert_eq!(object.payload().as_ref(), b"opus");
        assert!(object.end_of_group());
    }

    #[test]
    fn publish_rejection_is_terminal_before_acceptance() {
        let (_send, mut recv) = recv_pair(SubscribedState::default(), 7);
        recv.recv_error(ServeError::Closed(RequestErrorCode::Uninterested as u64))
            .unwrap();
        let state = recv.state.lock();
        assert_eq!(state.phase, SubscriptionPhase::Terminated);
        assert!(state.peer_rejected);
        assert!(matches!(
            state.closed,
            Err(ServeError::Closed(code)) if code == RequestErrorCode::Uninterested as u64
        ));
    }

    #[test]
    fn peer_cancellation_closes_shared_media_state() {
        let (_send, mut recv) = recv_pair(SubscribedState::default(), 7);
        recv.recv_error(ServeError::Cancel).unwrap();
        let state = recv.state.lock();
        assert_eq!(state.phase, SubscriptionPhase::Terminated);
        assert!(state.peer_rejected);
        assert!(matches!(state.closed, Err(ServeError::Cancel)));
    }

    #[test]
    fn excessive_load_subscribe_rejection_is_retryable() {
        let excessive = ServeError::Closed(RequestErrorCode::ExcessiveLoad as u64);
        assert_eq!(
            Subscribed::request_error_code(&excessive),
            RequestErrorCode::ExcessiveLoad as u64
        );
        assert_eq!(Subscribed::request_error_retry_interval(&excessive), 1001);
        assert_eq!(
            Subscribed::request_error_retry_interval(&ServeError::Cancel),
            0
        );
    }

    #[test]
    fn pending_establishes_or_terminates_explicitly() {
        let mut established = SubscribedState::default();
        assert_eq!(established.phase, SubscriptionPhase::Pending);
        established.establish(Some(Location::new(4, 8))).unwrap();
        assert_eq!(established.phase, SubscriptionPhase::Established);
        assert_eq!(established.joining_location, Some(Location::new(4, 8)));
        established.terminate().unwrap();
        assert_eq!(established.phase, SubscriptionPhase::Terminated);

        let mut rejected = SubscribedState::default();
        rejected.terminate().unwrap();
        assert_eq!(rejected.phase, SubscriptionPhase::Terminated);
    }

    #[tokio::test]
    async fn pending_joining_fetch_waiter_resolves_from_frozen_subscription_state() {
        let (state, recv) = recv_pair(SubscribedState::default(), 7);
        let waiter = tokio::spawn(async move { recv.wait_for_joining_fetch().await });
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());

        state
            .lock_mut()
            .unwrap()
            .establish(Some(Location::new(5, 6)))
            .unwrap();
        let (joining, _retained) = waiter.await.unwrap().unwrap();
        assert_eq!(joining.request_id, 7);
        assert_eq!(joining.joining_location, Location::new(5, 6));
    }

    #[test]
    fn first_object_initializes_empty_largest_location() {
        let mut state = SubscribedState::default();
        state.establish(None).unwrap();
        assert_eq!(state.largest_location, None);

        state.observe_largest_location(Location::new(0, 0)).unwrap();
        assert_eq!(state.largest_location, Some(Location::new(0, 0)));
        // The Joining Location was frozen when the subscription became
        // established, before the first object existed.
        assert_eq!(state.joining_location, None);
    }

    #[test]
    fn joining_location_is_frozen_while_live_progress_advances() {
        let mut state = SubscribedState::default();
        state.establish(Some(Location::new(5, 2))).unwrap();

        state.observe_largest_location(Location::new(5, 9)).unwrap();
        state
            .observe_largest_location(Location::new(4, 99))
            .unwrap();

        assert_eq!(state.largest_location, Some(Location::new(5, 9)));
        assert_eq!(state.joining_location, Some(Location::new(5, 2)));
    }

    #[test]
    fn invalid_phase_transitions_are_deterministic() {
        let mut pending = SubscribedState::default();
        assert_eq!(
            pending.update_forward(false, None),
            Err(SubscriptionStateError::InvalidTransition {
                phase: SubscriptionPhase::Pending,
                operation: SubscriptionOperation::UpdateForward,
            })
        );

        pending.establish(None).unwrap();
        assert_eq!(
            pending.establish(None),
            Err(SubscriptionStateError::InvalidTransition {
                phase: SubscriptionPhase::Established,
                operation: SubscriptionOperation::Establish,
            })
        );
        pending.terminate().unwrap();
        assert_eq!(
            pending.observe_largest_location(Location::new(1, 0)),
            Err(SubscriptionStateError::InvalidTransition {
                phase: SubscriptionPhase::Terminated,
                operation: SubscriptionOperation::ObserveObject,
            })
        );
        assert_eq!(
            pending.terminate(),
            Err(SubscriptionStateError::InvalidTransition {
                phase: SubscriptionPhase::Terminated,
                operation: SubscriptionOperation::Terminate,
            })
        );
    }

    #[test]
    fn forward_zero_to_one_captures_a_new_joining_location_once() {
        let mut state = SubscribedState {
            forward: false,
            ..Default::default()
        };
        state.establish(Some(Location::new(7, 3))).unwrap();
        assert_eq!(state.joining_location, None);

        state
            .update_forward(true, Some(Location::new(7, 8)))
            .unwrap();
        assert_eq!(state.joining_location, Some(Location::new(7, 8)));
        state.observe_largest_location(Location::new(8, 1)).unwrap();
        state
            .update_forward(true, Some(Location::new(8, 2)))
            .unwrap();
        assert_eq!(state.joining_location, Some(Location::new(7, 8)));

        state.update_forward(false, None).unwrap();
        state
            .update_forward(true, Some(Location::new(9, 0)))
            .unwrap();
        assert_eq!(state.joining_location, Some(Location::new(9, 0)));
    }

    #[test]
    fn joining_lookup_is_session_scoped_and_rejects_terminal_state() {
        let (state, recv) = recv_pair(SubscribedState::default(), 41);
        let mut session = HashMap::from([(41, recv)]);
        let other_session = HashMap::new();

        assert_eq!(
            lookup_joining_subscription(&session, 41),
            Ok(JoiningSubscriptionLookup::Pending)
        );
        assert_eq!(
            lookup_joining_subscription(&other_session, 41),
            Err(JoiningSubscriptionLookupError::InvalidJoiningRequestId)
        );

        state
            .lock_mut()
            .unwrap()
            .establish(Some(Location::new(2, 6)))
            .unwrap();
        assert_eq!(
            lookup_joining_subscription(&session, 41),
            Ok(JoiningSubscriptionLookup::Established(
                JoiningSubscription {
                    request_id: 41,
                    track_namespace: TrackNamespace::from_utf8_path("test/session"),
                    track_name: "audio".into(),
                    joining_location: Location::new(2, 6),
                }
            ))
        );

        state.lock_mut().unwrap().terminate().unwrap();
        assert_eq!(
            lookup_joining_subscription(&session, 41),
            Err(JoiningSubscriptionLookupError::InvalidJoiningRequestId)
        );
        assert_eq!(
            session.remove(&41).unwrap().joining_fetch_state(),
            Err(JoiningSubscriptionLookupError::InvalidJoiningRequestId)
        );
    }

    #[test]
    fn joining_lookup_validates_forward_and_saved_location() {
        let mut no_forward = SubscribedState {
            forward: false,
            ..Default::default()
        };
        no_forward.establish(Some(Location::new(1, 1))).unwrap();
        let (_state, recv) = recv_pair(no_forward, 1);
        assert_eq!(
            recv.joining_fetch_state(),
            Err(JoiningSubscriptionLookupError::ForwardDisabled)
        );

        let mut no_location = SubscribedState::default();
        no_location.establish(None).unwrap();
        let (_state, recv) = recv_pair(no_location, 2);
        assert_eq!(
            recv.joining_fetch_state(),
            Err(JoiningSubscriptionLookupError::NoJoiningLocation)
        );
        assert_eq!(
            JoiningSubscriptionLookupError::ForwardDisabled.request_error_code(),
            RequestErrorCode::InvalidRange
        );
        assert_eq!(
            JoiningSubscriptionLookupError::InvalidJoiningRequestId.request_error_code(),
            RequestErrorCode::InvalidJoiningRequestId
        );
    }

    #[test]
    fn publish_done_code_maps_done_to_track_ended() {
        assert_eq!(
            Subscribed::publish_done_code(&ServeError::Done),
            message::PublishDoneCode::TrackEnded as u64
        );
    }

    #[test]
    fn publish_done_code_passes_through_closed_code() {
        assert_eq!(
            Subscribed::publish_done_code(&ServeError::Closed(0x12)),
            0x12
        );
    }

    #[test]
    fn publish_done_code_maps_other_errors_to_internal() {
        assert_eq!(
            Subscribed::publish_done_code(&ServeError::internal_ctx("test")),
            message::PublishDoneCode::InternalError as u64
        );
    }

    #[test]
    fn request_error_code_maps_rejection_reasons() {
        assert_eq!(
            Subscribed::request_error_code(&ServeError::NotFound),
            RequestErrorCode::DoesNotExist as u64
        );
        assert_eq!(
            Subscribed::request_error_code(&ServeError::Duplicate),
            RequestErrorCode::Uninterested as u64
        );
        assert_eq!(
            Subscribed::request_error_code(&ServeError::NotImplemented("fetch".to_string())),
            RequestErrorCode::NotSupported as u64
        );
        assert_eq!(
            Subscribed::request_error_code(&ServeError::Cancel),
            RequestErrorCode::Uninterested as u64
        );
        assert_eq!(
            Subscribed::request_error_code(&ServeError::Closed(0x42)),
            0x42
        );
    }

    #[test]
    fn expected_serve_shutdown_is_only_cancel_or_done() {
        assert!(Subscribed::is_expected_serve_shutdown(
            &SessionError::Serve(ServeError::Cancel)
        ));
        assert!(Subscribed::is_expected_serve_shutdown(
            &SessionError::Serve(ServeError::Done)
        ));
        assert!(!Subscribed::is_expected_serve_shutdown(
            &SessionError::Serve(ServeError::NotFound)
        ));
        assert!(!Subscribed::is_expected_serve_shutdown(
            &SessionError::Internal
        ));
    }
}
