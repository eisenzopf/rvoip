// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-FileCopyrightText: 2023-2024 Luke Curley and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    ops,
    sync::Arc,
};

use bytes::BytesMut;
use tokio::sync::Notify;

use crate::{
    coding::{Encode, KeyValuePairs, Location, TrackName, TrackNamespace},
    data,
    message::{self, FilterType, GroupOrder, SubscriptionFilter},
    serve::{self, ServeError, TrackWriter, TrackWriterMode},
};

use crate::watch::State;

use super::Subscriber;
use super::{EndOfGroupState, RequestLease, SessionError};

#[derive(Debug, Clone, Copy)]
pub struct DeliveryFilter {
    pub forward: bool,
    pub start_location: Option<Location>,
    pub end_group_id: Option<u64>,
}

impl DeliveryFilter {
    pub fn allows(&self, group_id: u64, object_id: u64) -> bool {
        if !self.forward {
            return false;
        }

        let location = Location::new(group_id, object_id);
        if let Some(start) = self.start_location {
            if location < start {
                return false;
            }
        }

        if let Some(end_group_id) = self.end_group_id {
            if group_id > end_group_id {
                return false;
            }
        }

        true
    }
}

/// Transport-owned configuration for an outbound SUBSCRIBE request.
///
/// `None` leaves a typed parameter off the wire and therefore uses its MOQT
/// default. This keeps [`Subscriber::subscribe_open`] wire-compatible while
/// allowing callers to explicitly send values such as `Forward=1`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SubscribeOptions {
    pub forward: Option<bool>,
    pub filter: Option<SubscriptionFilter>,
    pub group_order: Option<GroupOrder>,
    pub subscriber_priority: Option<u8>,
    /// Additional request parameters, such as authorization or delivery
    /// policy. Typed fields above may not also appear here.
    pub request_parameters: KeyValuePairs,
}

impl SubscribeOptions {
    pub fn with_forward(mut self, forward: bool) -> Self {
        self.forward = Some(forward);
        self
    }

    pub fn with_filter(mut self, filter: SubscriptionFilter) -> Self {
        self.filter = Some(filter);
        self
    }

    pub fn with_group_order(mut self, group_order: GroupOrder) -> Self {
        self.group_order = Some(group_order);
        self
    }

    pub fn with_subscriber_priority(mut self, subscriber_priority: u8) -> Self {
        self.subscriber_priority = Some(subscriber_priority);
        self
    }

    pub fn with_request_parameters(mut self, request_parameters: KeyValuePairs) -> Self {
        self.request_parameters = request_parameters;
        self
    }

    /// Validate and merge typed and additional parameters without dropping or
    /// overwriting either source.
    pub fn to_request_parameters(&self) -> Result<KeyValuePairs, SubscribeOptionsError> {
        if let Some(filter) = &self.filter {
            validate_subscription_filter(filter)?;
        }
        if self.group_order == Some(GroupOrder::Publisher) {
            return Err(SubscribeOptionsError::InvalidGroupOrder);
        }

        for forbidden in [
            message::parameter_type::EXPIRES,
            message::parameter_type::LARGEST_OBJECT,
        ] {
            if self.request_parameters.has(forbidden) {
                return Err(SubscribeOptionsError::ParameterNotAllowed(forbidden));
            }
        }

        for (present, parameter) in [
            (self.forward.is_some(), message::parameter_type::FORWARD),
            (
                self.filter.is_some(),
                message::parameter_type::SUBSCRIPTION_FILTER,
            ),
            (
                self.group_order.is_some(),
                message::parameter_type::GROUP_ORDER,
            ),
            (
                self.subscriber_priority.is_some(),
                message::parameter_type::SUBSCRIBER_PRIORITY,
            ),
        ] {
            if present && self.request_parameters.has(parameter) {
                return Err(SubscribeOptionsError::ConflictingParameter(parameter));
            }
        }

        let mut parameters = self.request_parameters.clone();
        if let Some(forward) = self.forward {
            parameters.set_forward(forward);
        }
        if let Some(filter) = &self.filter {
            parameters
                .set_subscription_filter(filter)
                .map_err(|_| SubscribeOptionsError::InvalidRequestParameters)?;
        }
        if let Some(group_order) = self.group_order {
            parameters.set_group_order(group_order);
        }
        if let Some(priority) = self.subscriber_priority {
            parameters.set_subscriber_priority(priority);
        }

        validate_request_parameters(&parameters)?;
        Ok(parameters)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SubscribeOptionsError {
    #[error("Publisher group order is an omission sentinel and cannot be sent")]
    InvalidGroupOrder,
    #[error("invalid fields for subscription filter {0:?}")]
    InvalidFilter(FilterType),
    #[error("typed option conflicts with request parameter 0x{0:x}")]
    ConflictingParameter(u64),
    #[error("request parameter 0x{0:x} is not allowed on SUBSCRIBE")]
    ParameterNotAllowed(u64),
    #[error("request parameters are not valid MOQT parameters")]
    InvalidRequestParameters,
}

impl From<SubscribeOptionsError> for ServeError {
    fn from(error: SubscribeOptionsError) -> Self {
        ServeError::Internal(error.to_string())
    }
}

fn validate_subscription_filter(filter: &SubscriptionFilter) -> Result<(), SubscribeOptionsError> {
    let valid = match filter.filter_type {
        FilterType::NextGroupStart | FilterType::LargestObject => {
            filter.start_location.is_none() && filter.end_group_id.is_none()
        }
        FilterType::AbsoluteStart => {
            filter.start_location.is_some() && filter.end_group_id.is_none()
        }
        FilterType::AbsoluteRange => {
            filter.start_location.is_some() && filter.end_group_id.is_some()
        }
    };

    if valid {
        Ok(())
    } else {
        Err(SubscribeOptionsError::InvalidFilter(filter.filter_type))
    }
}

fn validate_request_parameters(parameters: &KeyValuePairs) -> Result<(), SubscribeOptionsError> {
    let mut encoded = BytesMut::new();
    parameters
        .encode(&mut encoded)
        .map_err(|_| SubscribeOptionsError::InvalidRequestParameters)?;
    parameters
        .forward()
        .map_err(|_| SubscribeOptionsError::InvalidRequestParameters)?;
    parameters
        .subscriber_priority()
        .map_err(|_| SubscribeOptionsError::InvalidRequestParameters)?;
    parameters
        .group_order()
        .map_err(|_| SubscribeOptionsError::InvalidRequestParameters)?;
    if let Some(filter) = parameters
        .subscription_filter()
        .map_err(|_| SubscribeOptionsError::InvalidRequestParameters)?
    {
        validate_subscription_filter(&filter)?;
    }

    Ok(())
}

// TODO rename to SubscriptionInfo when used for Publishes as well?
#[derive(Debug, Clone)]
pub struct SubscribeInfo {
    pub id: u64,
    pub track_namespace: TrackNamespace,
    pub track_name: TrackName,

    /// Subscriber Priority
    pub subscriber_priority: u8,
    pub group_order: GroupOrder,

    /// Forward Flag
    pub forward: bool,

    /// Filter type
    pub filter_type: FilterType,

    /// The starting location for this subscription. Only present for "AbsoluteStart" and "AbsoluteRange" filter types.
    pub start_location: Option<Location>,
    /// End group id, inclusive, for the subscription, if applicable. Only present for "AbsoluteRange" filter type.
    pub end_group_id: Option<u64>,

    /// None means the SUBSCRIPTION_FILTER parameter was omitted and the
    /// subscription is unfiltered per draft-16 §9.2.2.5.
    pub filter: Option<SubscriptionFilter>,

    /// Optional parameters
    pub params: KeyValuePairs,

    // Set to true if this is a track_status request only
    pub track_status: bool,
}

impl SubscribeInfo {
    pub fn new_from_subscribe(msg: &message::Subscribe) -> Result<Self, SessionError> {
        let filter = msg.params.subscription_filter()?;
        let filter_type = filter
            .as_ref()
            .map(|filter| filter.filter_type)
            .unwrap_or(FilterType::AbsoluteStart);
        let start_location = filter.as_ref().and_then(|filter| filter.start_location);
        let end_group_id = filter.as_ref().and_then(|filter| filter.end_group_id);

        Ok(Self {
            id: msg.id,
            track_namespace: msg.track_namespace.clone(),
            track_name: msg.track_name.clone(),
            subscriber_priority: msg.params.subscriber_priority()?.unwrap_or(128),
            group_order: msg.params.group_order()?.unwrap_or(GroupOrder::Publisher),
            forward: msg.params.forward()?.unwrap_or(true),
            filter_type,
            start_location,
            end_group_id,
            filter,
            params: msg.params.clone(),
            track_status: false,
        })
    }

    pub fn delivery_filter(&self, largest_location: Option<Location>) -> DeliveryFilter {
        let Some(filter) = &self.filter else {
            return DeliveryFilter {
                forward: self.forward,
                start_location: None,
                end_group_id: None,
            };
        };

        let start_location = match filter.filter_type {
            FilterType::LargestObject => Some(next_object_location(largest_location)),
            FilterType::NextGroupStart => Some(next_group_location(largest_location)),
            FilterType::AbsoluteStart | FilterType::AbsoluteRange => filter.start_location,
        };

        DeliveryFilter {
            forward: self.forward,
            start_location,
            end_group_id: filter.end_group_id,
        }
    }
}

fn next_object_location(largest_location: Option<Location>) -> Location {
    let Some(location) = largest_location else {
        return Location::new(0, 0);
    };

    if let Some(object_id) = location.object_id.checked_add(1) {
        Location::new(location.group_id, object_id)
    } else {
        next_group_location(Some(location))
    }
}

fn next_group_location(largest_location: Option<Location>) -> Location {
    let Some(location) = largest_location else {
        return Location::new(0, 0);
    };

    Location::new(location.group_id.saturating_add(1), 0)
}

struct SubscribeState {
    ok: bool,
    track_alias: Option<u64>,
    closed: Result<(), ServeError>,
}

const JOIN_BARRIER_MAX_OBJECTS: usize = 1024;
const JOIN_BARRIER_MAX_BYTES: usize = 4 * 1024 * 1024;
const JOIN_BARRIER_MAX_SUBGROUPS: usize = 64;

#[derive(Clone, Debug)]
pub(super) struct BufferedJoinObject {
    pub location: Location,
    pub subgroup_id: u64,
    pub publisher_priority: u8,
    pub properties: data::ExtensionHeaders,
    pub payload: bytes::Bytes,
    pub first_object: bool,
    pub group_end: EndOfGroupState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum JoinBarrierPhase {
    AwaitingSubscribeOk,
    Fetching,
    Released,
}

struct JoinBarrier {
    phase: JoinBarrierPhase,
    cutoff: Option<Location>,
    buffered: BTreeMap<Location, BufferedJoinObject>,
    fetched: BTreeMap<Location, BufferedJoinObject>,
    signaled_group_ends: HashSet<(u64, u64)>,
    buffered_bytes: usize,
}

impl JoinBarrier {
    fn new() -> Self {
        Self {
            phase: JoinBarrierPhase::AwaitingSubscribeOk,
            cutoff: None,
            buffered: BTreeMap::new(),
            fetched: BTreeMap::new(),
            signaled_group_ends: HashSet::new(),
            buffered_bytes: 0,
        }
    }

    fn set_cutoff(&mut self, cutoff: Option<Location>, seen: &mut HashSet<(u64, u64)>) {
        self.cutoff = cutoff;
        self.phase = JoinBarrierPhase::Fetching;
        if let Some(cutoff) = cutoff {
            let discarded: Vec<_> = self
                .buffered
                .range(..=cutoff)
                .map(|(location, _)| *location)
                .collect();
            for location in discarded {
                if let Some(object) = self.buffered.remove(&location) {
                    if object.group_end.is_signaled() {
                        self.signaled_group_ends
                            .insert((object.location.group_id, object.subgroup_id));
                    }
                    self.buffered_bytes = self.buffered_bytes.saturating_sub(object.payload.len());
                }
                seen.remove(&(location.group_id, location.object_id));
            }
        }
    }

    fn claims_live(&self, location: Location) -> bool {
        match self.phase {
            JoinBarrierPhase::AwaitingSubscribeOk => true,
            JoinBarrierPhase::Fetching | JoinBarrierPhase::Released => {
                self.cutoff.is_none_or(|cutoff| location > cutoff)
            }
        }
    }

    fn buffer(&mut self, object: BufferedJoinObject) -> Result<(), ServeError> {
        if self.phase == JoinBarrierPhase::Released {
            return Err(ServeError::internal_ctx(
                "released Joining FETCH barrier cannot buffer live media",
            ));
        }
        if self.buffered.contains_key(&object.location)
            || self.fetched.contains_key(&object.location)
        {
            return Err(ServeError::Duplicate);
        }
        let bytes = self
            .buffered_bytes
            .checked_add(object.payload.len())
            .ok_or_else(|| ServeError::internal_ctx("Joining FETCH buffer byte overflow"))?;
        if self.buffered.len() + self.fetched.len() >= JOIN_BARRIER_MAX_OBJECTS
            || bytes > JOIN_BARRIER_MAX_BYTES
        {
            return Err(ServeError::Closed(
                message::RequestErrorCode::ExcessiveLoad as u64,
            ));
        }
        if object.group_end.is_signaled() {
            self.signaled_group_ends
                .insert((object.location.group_id, object.subgroup_id));
        }
        self.buffered_bytes = bytes;
        self.buffered.insert(object.location, object);
        Ok(())
    }

    fn buffer_fetched(&mut self, object: BufferedJoinObject) -> Result<(), ServeError> {
        if self.phase != JoinBarrierPhase::Fetching {
            return Err(ServeError::internal_ctx(
                "fetched Object arrived outside the barrier fetch phase",
            ));
        }
        if self.buffered.contains_key(&object.location)
            || self.fetched.contains_key(&object.location)
        {
            return Err(ServeError::Duplicate);
        }
        let bytes = self
            .buffered_bytes
            .checked_add(object.payload.len())
            .ok_or_else(|| ServeError::internal_ctx("Joining FETCH buffer byte overflow"))?;
        if self.buffered.len() + self.fetched.len() >= JOIN_BARRIER_MAX_OBJECTS
            || bytes > JOIN_BARRIER_MAX_BYTES
        {
            return Err(ServeError::Closed(
                message::RequestErrorCode::ExcessiveLoad as u64,
            ));
        }
        self.buffered_bytes = bytes;
        self.fetched.insert(object.location, object);
        Ok(())
    }

    fn release(&mut self) -> Vec<BufferedJoinObject> {
        self.phase = JoinBarrierPhase::Released;
        self.buffered_bytes = 0;
        let mut objects = std::mem::take(&mut self.fetched);
        objects.append(&mut self.buffered);
        let signaled_group_ends = std::mem::take(&mut self.signaled_group_ends);
        objects
            .into_values()
            .map(|mut object| {
                if signaled_group_ends.contains(&(object.location.group_id, object.subgroup_id)) {
                    object.group_end = EndOfGroupState::Signaled;
                }
                object
            })
            .collect()
    }
}

impl Default for SubscribeState {
    fn default() -> Self {
        Self {
            ok: Default::default(),
            track_alias: None,
            closed: Ok(()),
        }
    }
}

// Held by the application
#[must_use = "unsubscribe on drop"]
pub struct Subscribe {
    state: State<SubscribeState>,
    subscriber: Subscriber,
    response_cancel: Option<tokio::sync::oneshot::Sender<()>>,

    pub info: SubscribeInfo,
    _request_lease: Arc<RequestLease>,
}

impl Subscribe {
    pub(super) fn subscriber(&self) -> &Subscriber {
        &self.subscriber
    }

    fn build_info(
        request_id: u64,
        track: &TrackWriter,
        options: &SubscribeOptions,
    ) -> Result<SubscribeInfo, SubscribeOptionsError> {
        let subscribe_message = message::Subscribe {
            id: request_id,
            track_namespace: track.namespace.clone(),
            track_name: track.name.clone(),
            params: options.to_request_parameters()?,
        };
        SubscribeInfo::new_from_subscribe(&subscribe_message)
            .map_err(|_| SubscribeOptionsError::InvalidRequestParameters)
    }

    /// Create a configured Subscribe without sending on the control stream.
    /// The caller sends it via a bidirectional request stream.
    pub(super) fn new_with_options(
        subscriber: Subscriber,
        request_id: u64,
        track: TrackWriter,
        options: SubscribeOptions,
        request_lease: Arc<RequestLease>,
    ) -> Result<(Subscribe, SubscribeRecv), SubscribeOptionsError> {
        let info = Self::build_info(request_id, &track, &options)?;
        Ok(Self::from_parts(subscriber, info, track, request_lease))
    }

    /// Return the wire message to send on the request stream.
    pub(super) fn wire_message(&self) -> message::Subscribe {
        message::Subscribe {
            id: self.info.id,
            track_namespace: self.info.track_namespace.clone(),
            track_name: self.info.track_name.clone(),
            params: self.info.params.clone(),
        }
    }

    fn from_parts(
        subscriber: Subscriber,
        info: SubscribeInfo,
        track: TrackWriter,
        request_lease: Arc<RequestLease>,
    ) -> (Subscribe, SubscribeRecv) {
        let (send, recv) = State::default().split();

        let send = Subscribe {
            state: send,
            subscriber,
            response_cancel: None,
            info,
            _request_lease: request_lease.clone(),
        };

        let recv = SubscribeRecv {
            state: recv,
            writer: Some(track.into()),
            processed_streams: 0,
            active_streams: 0,
            terminal: None,
            progress: Arc::new(Notify::new()),
            info: send.info.clone(),
            delivery_filter: None,
            joining_location: None,
            join_barrier: None,
            joining_writers: HashMap::new(),
            seen_objects: HashSet::new(),
            _request_lease: request_lease,
        };

        (send, recv)
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

    pub(super) fn attach_response_cancel(
        &mut self,
        response_cancel: tokio::sync::oneshot::Sender<()>,
    ) {
        self.response_cancel = Some(response_cancel);
    }

    pub async fn ok(&self) -> Result<(), ServeError> {
        loop {
            {
                let state = self.state.lock();
                state.closed.clone()?;

                if state.ok {
                    return Ok(());
                }

                match state.modified() {
                    Some(notify) => notify,
                    None => return Err(ServeError::Done),
                }
            }
            .await;
        }
    }
}

impl Drop for Subscribe {
    fn drop(&mut self) {
        // Draft-19 removed UNSUBSCRIBE. The owning request stream is the
        // cancellation boundary; dropping the handle releases local state.
        if let Some(cancel) = self.response_cancel.take() {
            let _ = cancel.send(());
        }
        self.subscriber.remove_subscribe(self.info.id);
        self._request_lease.release();
    }
}

impl ops::Deref for Subscribe {
    type Target = SubscribeInfo;

    fn deref(&self) -> &SubscribeInfo {
        &self.info
    }
}

pub(super) struct SubscribeRecv {
    state: State<SubscribeState>,
    writer: Option<TrackWriterMode>,
    processed_streams: u64,
    active_streams: u64,
    terminal: Option<(u64, u64)>,
    progress: Arc<Notify>,
    info: SubscribeInfo,
    delivery_filter: Option<DeliveryFilter>,
    joining_location: Option<Location>,
    join_barrier: Option<JoinBarrier>,
    joining_writers: HashMap<(u64, u64), serve::SubgroupWriter>,
    seen_objects: HashSet<(u64, u64)>,
    _request_lease: Arc<RequestLease>,
}

impl SubscribeRecv {
    pub(super) fn release_request_lease(&self) {
        self._request_lease.release();
    }

    pub fn ok(&mut self, msg: &message::SubscribeOk) -> Result<(), ServeError> {
        let state = self.state.lock();
        if state.ok {
            return Err(ServeError::Duplicate);
        }

        if let Some(mut state) = state.into_mut() {
            state.ok = true;
            state.track_alias = Some(msg.track_alias);
        }
        let largest = msg
            .params
            .largest_object()
            .map_err(|err| ServeError::internal_ctx(format!("invalid largest object: {err}")))?;
        self.joining_location = largest;
        self.delivery_filter = Some(self.info.delivery_filter(largest));
        if let Some(barrier) = self.join_barrier.as_mut() {
            barrier.set_cutoff(largest, &mut self.seen_objects);
        }

        Ok(())
    }

    pub fn full_name(&self) -> serve::FullTrackName {
        serve::FullTrackName {
            namespace: self.info.track_namespace.clone(),
            name: self.info.track_name.clone(),
        }
    }

    pub(super) fn awaiting_streams(&self) -> bool {
        self.terminal
            .is_some_and(|(_, expected)| self.processed_streams < expected)
    }

    pub(super) fn progress(&self) -> Arc<Notify> {
        self.progress.clone()
    }

    /// Record PUBLISH_DONE without dropping the track writer until every
    /// declared subgroup stream has been received. The request stream can
    /// outrun independently scheduled data streams.
    pub(super) fn recv_done(
        &mut self,
        status_code: u64,
        stream_count: u64,
    ) -> Result<bool, SessionError> {
        if self.processed_streams.saturating_add(self.active_streams) > stream_count {
            return Err(SessionError::ProtocolViolation(format!(
                "PUBLISH_DONE declared {} streams after {} SUBSCRIBE streams were received",
                stream_count,
                self.processed_streams.saturating_add(self.active_streams)
            )));
        }
        self.terminal = Some((status_code, stream_count));
        self.progress.notify_one();
        Ok(self.maybe_finish())
    }

    pub(super) fn begin_stream(&mut self, limit: u64) -> Result<(), ServeError> {
        if self.active_streams >= limit {
            return Err(ServeError::Closed(
                message::RequestErrorCode::ExcessiveLoad as u64,
            ));
        }
        if let Some((_, expected)) = self.terminal {
            if self.processed_streams.saturating_add(self.active_streams) >= expected {
                return Err(ServeError::Size);
            }
        }
        self.active_streams = self.active_streams.saturating_add(1);
        Ok(())
    }

    pub(super) fn finish_stream(&mut self) -> Result<bool, SessionError> {
        self.active_streams = self.active_streams.saturating_sub(1);
        self.processed_streams = self.processed_streams.saturating_add(1);
        self.progress.notify_one();
        if let Some((_, expected)) = self.terminal {
            if self.processed_streams > expected {
                return Err(SessionError::ProtocolViolation(format!(
                    "received more SUBSCRIBE streams than declared: {} > {}",
                    self.processed_streams, expected
                )));
            }
        }
        Ok(self.maybe_finish())
    }

    fn maybe_finish(&mut self) -> bool {
        let Some((status_code, stream_count)) = self.terminal else {
            return false;
        };
        if self.processed_streams < stream_count {
            return false;
        }
        if let Some(mut state) = self.state.lock_mut() {
            state.closed = if status_code == message::PublishDoneCode::TrackEnded as u64 {
                Err(ServeError::Done)
            } else {
                Err(ServeError::Closed(status_code))
            };
        }
        self.joining_writers.clear();
        self.writer = None;
        true
    }

    pub(super) fn recv_stream_error(&mut self, err: ServeError) {
        self.active_streams = self.active_streams.saturating_sub(1);
        self.progress.notify_one();
        if let Some(mut state) = self.state.lock_mut() {
            state.closed = Err(err);
        }
        self.joining_writers.clear();
        self.writer = None;
    }

    pub fn allows(&self, group_id: u64, object_id: u64) -> bool {
        self.delivery_filter
            .unwrap_or_else(|| self.info.delivery_filter(None))
            .allows(group_id, object_id)
    }

    /// Claim an Object for this subscription, applying its filter and
    /// suppressing duplicate wire copies caused by shared Track Aliases.
    pub fn claim_object(&mut self, group_id: u64, object_id: u64) -> bool {
        let location = Location::new(group_id, object_id);
        if self
            .join_barrier
            .as_ref()
            .is_some_and(|barrier| barrier.phase == JoinBarrierPhase::Released)
        {
            let largest_seen_group = self.seen_objects.iter().map(|(group, _)| *group).max();
            if largest_seen_group.is_some_and(|largest| group_id < largest) {
                return false;
            }
            if largest_seen_group.is_some_and(|largest| group_id > largest) {
                self.seen_objects.retain(|(group, _)| *group == group_id);
            }
        }
        let barrier_allows = self
            .join_barrier
            .as_ref()
            .is_none_or(|barrier| barrier.claims_live(location));
        self.allows(group_id, object_id)
            && barrier_allows
            && self.seen_objects.insert((group_id, object_id))
    }

    pub fn track_alias(&self) -> Option<u64> {
        let state = self.state.lock();
        state.track_alias
    }

    pub(super) fn joining_location(&self) -> Option<Location> {
        self.joining_location
    }

    pub(super) fn begin_joining_fetch(&mut self) -> Result<(), ServeError> {
        if self.join_barrier.is_some() {
            return Err(ServeError::Duplicate);
        }
        let mut barrier = JoinBarrier::new();
        if self.joining_location.is_some() {
            barrier.set_cutoff(self.joining_location, &mut self.seen_objects);
        }
        self.join_barrier = Some(barrier);
        Ok(())
    }

    pub(super) fn has_joining_barrier(&self) -> bool {
        self.join_barrier.is_some()
    }

    pub(super) fn recv_joining_live_object(
        &mut self,
        object: BufferedJoinObject,
    ) -> Result<(), ServeError> {
        let Some(barrier) = self.join_barrier.as_mut() else {
            return Err(ServeError::internal_ctx(
                "Joining FETCH barrier is not active",
            ));
        };
        if barrier.phase == JoinBarrierPhase::Released {
            self.write_joining_object(object)
        } else {
            barrier.buffer(object)
        }
    }

    pub(super) fn recv_fetched_object(
        &mut self,
        object: BufferedJoinObject,
    ) -> Result<(), ServeError> {
        let barrier = self
            .join_barrier
            .as_ref()
            .ok_or_else(|| ServeError::internal_ctx("Joining FETCH barrier is not active"))?;
        if barrier.phase != JoinBarrierPhase::Fetching {
            return Err(ServeError::internal_ctx(
                "fetched Object arrived outside the barrier fetch phase",
            ));
        }
        let cutoff = barrier.cutoff.ok_or_else(|| {
            ServeError::internal_ctx("Joining FETCH barrier has no frozen cutoff")
        })?;
        if object.location > cutoff {
            return Err(ServeError::internal_ctx(
                "fetched Object passed the frozen Joining Location",
            ));
        }
        if !self
            .seen_objects
            .insert((object.location.group_id, object.location.object_id))
        {
            return Ok(());
        }
        self.join_barrier
            .as_mut()
            .ok_or_else(|| ServeError::internal_ctx("Joining FETCH barrier is not active"))?
            .buffer_fetched(object)
    }

    pub(super) fn finish_joining_fetch(&mut self) -> Result<(), ServeError> {
        let buffered = self
            .join_barrier
            .as_mut()
            .ok_or_else(|| ServeError::internal_ctx("Joining FETCH barrier is not active"))?
            .release();
        for object in buffered {
            self.write_joining_object(object)?;
        }
        if let Some(largest_group) = self.seen_objects.iter().map(|(group, _)| *group).max() {
            self.seen_objects
                .retain(|(group, _)| *group == largest_group);
        }
        Ok(())
    }

    /// Release a Joining barrier when SUBSCRIBE_OK reported no Largest
    /// Object, so there is no valid Relative Joining FETCH range.
    ///
    /// When no live Object raced the response, remove the barrier entirely
    /// and resume the ordinary zero-copy receive path. If an Object did race,
    /// preserve the released barrier long enough to flush its bounded payload
    /// before continuing through the joining writer path.
    pub(super) fn fall_back_to_live_without_fetch(&mut self) -> Result<(), ServeError> {
        let barrier = self
            .join_barrier
            .as_ref()
            .ok_or_else(|| ServeError::internal_ctx("Joining FETCH barrier is not active"))?;
        if barrier.cutoff.is_some() {
            return Err(ServeError::internal_ctx(
                "cannot skip Joining FETCH with a frozen cutoff",
            ));
        }
        if barrier.buffered.is_empty() && barrier.fetched.is_empty() {
            self.join_barrier.take();
            return Ok(());
        }
        self.finish_joining_fetch()
    }

    pub(super) fn abort_joining_fetch(&mut self) -> Result<(), ServeError> {
        if self.join_barrier.is_none() {
            return Ok(());
        }
        self.finish_joining_fetch()
    }

    fn write_joining_object(&mut self, object: BufferedJoinObject) -> Result<(), ServeError> {
        let key = (object.location.group_id, object.subgroup_id);
        if !self.joining_writers.contains_key(&key) {
            if let Some(largest_group) = self
                .joining_writers
                .keys()
                .map(|(group_id, _)| *group_id)
                .max()
            {
                if object.location.group_id < largest_group {
                    return Err(ServeError::internal_ctx(
                        "Joining FETCH handoff regressed to an older Group",
                    ));
                }
                if object.location.group_id > largest_group {
                    self.joining_writers
                        .retain(|(group_id, _), _| *group_id == object.location.group_id);
                }
            }
            if self.joining_writers.len() >= JOIN_BARRIER_MAX_SUBGROUPS {
                return Err(ServeError::Closed(
                    message::RequestErrorCode::ExcessiveLoad as u64,
                ));
            }
            let writer = self.writer.take().ok_or(ServeError::Done)?;
            let mut subgroups = match writer {
                TrackWriterMode::Track(track) => track.subgroups()?,
                TrackWriterMode::Subgroups(subgroups) => subgroups,
                other => {
                    self.writer = Some(other);
                    return Err(ServeError::Mode);
                }
            };
            let subgroup = subgroups.create(
                serve::Subgroup::new(
                    object.location.group_id,
                    object.subgroup_id,
                    object.publisher_priority,
                )
                .with_first_object(object.first_object)
                .with_end_of_group(object.group_end.is_signaled()),
            )?;
            self.writer = Some(subgroups.into());
            self.joining_writers.insert(key, subgroup);
        }

        let subgroup = self.joining_writers.get_mut(&key).ok_or(ServeError::Done)?;
        let next_object_id = u64::try_from(subgroup.len()).map_err(|_| ServeError::Size)?;
        if next_object_id != object.location.object_id {
            return Err(ServeError::internal_ctx(format!(
                "Joining FETCH subgroup expected Object {next_object_id}, received {}",
                object.location.object_id
            )));
        }
        let mut writer = subgroup.create(object.payload.len(), Some(object.properties))?;
        writer.write(object.payload)?;
        Ok(())
    }

    pub fn error(mut self, err: ServeError) -> Result<(), ServeError> {
        if let Some(writer) = self.writer.take() {
            writer.close(err.clone())?;
        }

        let state = self.state.lock();
        state.closed.clone()?;

        let mut state = state.into_mut().ok_or(ServeError::Cancel)?;
        state.closed = Err(err);

        Ok(())
    }

    pub fn subgroup(
        &mut self,
        header: data::SubgroupHeader,
    ) -> Result<serve::SubgroupWriter, ServeError> {
        let writer = self.writer.take().ok_or(ServeError::Done)?;

        let mut subgroups = match writer {
            // TODO SLG - understand why both of these are needed, clock demo won't run if I comment out TrackWriteMode::Track
            TrackWriterMode::Track(track) => track.subgroups()?,
            TrackWriterMode::Subgroups(subgroups) => subgroups,
            _ => return Err(ServeError::Mode),
        };

        let writer = subgroups.create(serve::Subgroup::from_header(&header)?)?;

        self.writer = Some(subgroups.into());

        Ok(writer)
    }

    pub fn datagram(&mut self, datagram: data::Datagram) -> Result<(), ServeError> {
        let writer = self.writer.take().ok_or(ServeError::Done)?;

        match writer {
            TrackWriterMode::Track(track) => {
                // convert Track -> Datagrams writer, write, then put Datagrams back
                let mut datagrams = track.datagrams()?;
                datagrams.write(serve::Datagram {
                    group_id: datagram.group_id,
                    object_id: datagram.object_id.unwrap_or(0),
                    priority: datagram.publisher_priority,
                    payload: datagram.payload.unwrap_or_default(),
                    extension_headers: datagram.extension_headers.unwrap_or_default(),
                })?;
                self.writer = Some(TrackWriterMode::Datagrams(datagrams));
                Ok(())
            }
            TrackWriterMode::Datagrams(mut datagrams) => {
                datagrams.write(serve::Datagram {
                    group_id: datagram.group_id,
                    object_id: datagram.object_id.unwrap_or(0),
                    priority: datagram.publisher_priority,
                    payload: datagram.payload.unwrap_or_default(),
                    extension_headers: datagram.extension_headers.unwrap_or_default(),
                })?;
                self.writer = Some(TrackWriterMode::Datagrams(datagrams));
                Ok(())
            }
            other => {
                // preserve whatever unexpected mode was present, then report error
                self.writer = Some(other);
                Err(ServeError::Mode)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track_writer() -> TrackWriter {
        let (writer, _reader) =
            serve::Track::new(TrackNamespace::from_utf8_path("test/session"), "audio").produce();
        writer
    }

    fn buffered(group_id: u64, object_id: u64, payload: &'static [u8]) -> BufferedJoinObject {
        BufferedJoinObject {
            location: Location::new(group_id, object_id),
            subgroup_id: 0,
            publisher_priority: 7,
            properties: Default::default(),
            payload: bytes::Bytes::from_static(payload),
            first_object: object_id == 0,
            group_end: EndOfGroupState::NotSignaled,
        }
    }

    fn joining_recv() -> (SubscribeRecv, serve::TrackReader) {
        let (writer, reader) =
            serve::Track::new(TrackNamespace::from_utf8_path("test/session"), "audio").produce();
        let state = State::default();
        (
            SubscribeRecv {
                state,
                writer: Some(writer.into()),
                processed_streams: 0,
                active_streams: 0,
                terminal: None,
                progress: Arc::new(Notify::new()),
                info: subscribe_info_with(KeyValuePairs::default()),
                delivery_filter: None,
                joining_location: None,
                join_barrier: None,
                joining_writers: HashMap::new(),
                seen_objects: HashSet::new(),
                _request_lease: crate::session::test_request_lease(
                    crate::session::RequestDirection::Outbound,
                    crate::session::RequestClass::Subscribe,
                ),
            },
            reader,
        )
    }

    fn terminal_recv() -> (State<SubscribeState>, SubscribeRecv, serve::TrackReader) {
        let (writer, reader) =
            serve::Track::new(TrackNamespace::from_utf8_path("test/session"), "audio").produce();
        let (app_state, transport_state) = State::default().split();
        (
            app_state,
            SubscribeRecv {
                state: transport_state,
                writer: Some(writer.into()),
                processed_streams: 0,
                active_streams: 0,
                terminal: None,
                progress: Arc::new(Notify::new()),
                info: subscribe_info_with(KeyValuePairs::default()),
                delivery_filter: None,
                joining_location: None,
                join_barrier: None,
                joining_writers: HashMap::new(),
                seen_objects: HashSet::new(),
                _request_lease: crate::session::test_request_lease(
                    crate::session::RequestDirection::Outbound,
                    crate::session::RequestClass::Subscribe,
                ),
            },
            reader,
        )
    }

    #[test]
    fn publish_done_retains_subscribe_until_every_declared_stream_finishes() {
        let (app_state, mut recv, reader) = terminal_recv();

        assert!(!recv
            .recv_done(message::PublishDoneCode::TrackEnded as u64, 1)
            .unwrap());
        assert!(app_state.lock().closed.is_ok());
        assert!(!reader.is_closed());

        recv.begin_stream(4).unwrap();
        assert!(recv.finish_stream().unwrap());
        assert!(matches!(app_state.lock().closed, Err(ServeError::Done)));
        assert!(reader.is_closed());
    }

    #[test]
    fn publish_done_rejects_a_stream_count_smaller_than_observed() {
        let (_app_state, mut recv, _reader) = terminal_recv();
        recv.begin_stream(4).unwrap();
        assert!(!recv.finish_stream().unwrap());
        assert!(matches!(
            recv.recv_done(message::PublishDoneCode::TrackEnded as u64, 0),
            Err(SessionError::ProtocolViolation(_))
        ));
    }

    fn subscribe_info_with(params: KeyValuePairs) -> SubscribeInfo {
        SubscribeInfo::new_from_subscribe(&message::Subscribe {
            id: 0,
            track_namespace: TrackNamespace::from_utf8_path("test"),
            track_name: "track".into(),
            params,
        })
        .unwrap()
    }

    #[test]
    fn omitted_subscription_filter_is_unfiltered() {
        let info = subscribe_info_with(KeyValuePairs::default());
        let filter = info.delivery_filter(Some(Location::new(10, 20)));

        assert!(info.filter.is_none());
        assert!(filter.allows(0, 0));
        assert!(filter.allows(10, 20));
        assert!(filter.allows(100, 0));
    }

    #[test]
    fn largest_object_filter_starts_after_largest_object() {
        let mut params = KeyValuePairs::default();
        params
            .set_subscription_filter(&SubscriptionFilter::largest_object())
            .unwrap();
        let info = subscribe_info_with(params);
        let filter = info.delivery_filter(Some(Location::new(2, 3)));

        assert!(!filter.allows(2, 3));
        assert!(filter.allows(2, 4));
        assert!(filter.allows(3, 0));
    }

    #[test]
    fn absolute_range_filter_limits_start_and_end_group() {
        let mut params = KeyValuePairs::default();
        params
            .set_subscription_filter(&SubscriptionFilter {
                filter_type: FilterType::AbsoluteRange,
                start_location: Some(Location::new(2, 3)),
                end_group_id: Some(4),
            })
            .unwrap();
        let info = subscribe_info_with(params);
        let filter = info.delivery_filter(None);

        assert!(!filter.allows(2, 2));
        assert!(filter.allows(2, 3));
        assert!(filter.allows(4, 10));
        assert!(!filter.allows(5, 0));
    }

    #[test]
    fn forward_false_blocks_delivery() {
        let mut params = KeyValuePairs::default();
        params.set_forward(false);
        let info = subscribe_info_with(params);
        let filter = info.delivery_filter(None);

        assert!(!filter.allows(0, 0));
        assert!(!filter.allows(100, 100));
    }

    #[test]
    fn explicit_forward_one_and_largest_object_have_stable_wire_and_model() {
        let options = SubscribeOptions::default()
            .with_forward(true)
            .with_filter(SubscriptionFilter::largest_object());
        let parameters = options.to_request_parameters().unwrap();
        let mut wire = BytesMut::new();
        parameters.encode(&mut wire).unwrap();

        // count=2, FORWARD(type=0x10,value=1), then LOCATION_FILTER
        // (delta=0x11,length=1,LargestObject=0x2).
        assert_eq!(wire.as_ref(), &[0x02, 0x10, 0x01, 0x11, 0x01, 0x02]);
        let info = Subscribe::build_info(8, &track_writer(), &options).unwrap();
        assert_eq!(info.id, 8);
        assert!(info.forward);
        assert_eq!(info.filter, Some(SubscriptionFilter::largest_object()));
        assert_eq!(info.params, parameters);
    }

    #[test]
    fn default_options_preserve_legacy_empty_parameters_and_semantics() {
        let options = SubscribeOptions::default();
        assert!(options.to_request_parameters().unwrap().0.is_empty());

        let info = Subscribe::build_info(10, &track_writer(), &options).unwrap();
        assert!(info.params.0.is_empty());
        assert!(info.forward);
        assert_eq!(info.subscriber_priority, 128);
        assert_eq!(info.group_order, GroupOrder::Publisher);
        assert_eq!(info.filter, None);
    }

    #[test]
    fn explicit_priority_and_group_order_round_trip_into_model() {
        let options = SubscribeOptions::default()
            .with_subscriber_priority(7)
            .with_group_order(GroupOrder::Descending);
        let parameters = options.to_request_parameters().unwrap();
        assert_eq!(parameters.subscriber_priority().unwrap(), Some(7));
        assert_eq!(
            parameters.group_order().unwrap(),
            Some(GroupOrder::Descending)
        );

        let info = Subscribe::build_info(12, &track_writer(), &options).unwrap();
        assert_eq!(info.subscriber_priority, 7);
        assert_eq!(info.group_order, GroupOrder::Descending);
    }

    #[test]
    fn invalid_option_combinations_are_rejected_before_wire_encoding() {
        assert_eq!(
            SubscribeOptions::default()
                .with_group_order(GroupOrder::Publisher)
                .to_request_parameters(),
            Err(SubscribeOptionsError::InvalidGroupOrder)
        );

        for filter in [
            SubscriptionFilter {
                filter_type: FilterType::LargestObject,
                start_location: Some(Location::new(1, 0)),
                end_group_id: None,
            },
            SubscriptionFilter {
                filter_type: FilterType::AbsoluteStart,
                start_location: None,
                end_group_id: None,
            },
            SubscriptionFilter {
                filter_type: FilterType::AbsoluteRange,
                start_location: Some(Location::new(1, 0)),
                end_group_id: None,
            },
        ] {
            let filter_type = filter.filter_type;
            assert_eq!(
                SubscribeOptions::default()
                    .with_filter(filter)
                    .to_request_parameters(),
                Err(SubscribeOptionsError::InvalidFilter(filter_type))
            );
        }

        let mut collision = KeyValuePairs::default();
        collision.set_forward(false);
        assert_eq!(
            SubscribeOptions::default()
                .with_forward(true)
                .with_request_parameters(collision)
                .to_request_parameters(),
            Err(SubscribeOptionsError::ConflictingParameter(
                message::parameter_type::FORWARD
            ))
        );

        let mut response_only = KeyValuePairs::default();
        response_only.set_intvalue(message::parameter_type::EXPIRES, 1);
        assert_eq!(
            SubscribeOptions::default()
                .with_request_parameters(response_only)
                .to_request_parameters(),
            Err(SubscribeOptionsError::ParameterNotAllowed(
                message::parameter_type::EXPIRES
            ))
        );

        let invalid_wire = KeyValuePairs(vec![crate::coding::KeyValuePair::new_bytes(
            message::parameter_type::DELIVERY_TIMEOUT,
            vec![1],
        )]);
        assert_eq!(
            SubscribeOptions::default()
                .with_request_parameters(invalid_wire)
                .to_request_parameters(),
            Err(SubscribeOptionsError::InvalidRequestParameters)
        );
    }

    #[test]
    fn additional_request_parameters_are_preserved_without_silent_overwrite() {
        let mut request_parameters = KeyValuePairs::default();
        request_parameters.set_intvalue(message::parameter_type::DELIVERY_TIMEOUT, 250);
        request_parameters.set_bytesvalue(0x41, vec![1, 2, 3]);
        let original = request_parameters.clone();

        let merged = SubscribeOptions::default()
            .with_forward(true)
            .with_subscriber_priority(9)
            .with_request_parameters(request_parameters)
            .to_request_parameters()
            .unwrap();

        assert_eq!(
            merged.get(message::parameter_type::DELIVERY_TIMEOUT),
            original.get(message::parameter_type::DELIVERY_TIMEOUT)
        );
        assert_eq!(merged.get(0x41), original.get(0x41));
        assert_eq!(merged.forward().unwrap(), Some(true));
        assert_eq!(merged.subscriber_priority().unwrap(), Some(9));
        assert_eq!(merged.0.len(), original.0.len() + 2);
    }

    #[test]
    fn valid_typed_request_parameter_is_preserved_when_option_is_omitted() {
        let mut request_parameters = KeyValuePairs::default();
        request_parameters.set_forward(false);
        let parameters = SubscribeOptions::default()
            .with_request_parameters(request_parameters)
            .to_request_parameters()
            .unwrap();
        assert_eq!(parameters.forward().unwrap(), Some(false));

        let info = Subscribe::build_info(
            14,
            &track_writer(),
            &SubscribeOptions::default().with_request_parameters(parameters),
        )
        .unwrap();
        assert!(!info.forward);
    }

    #[test]
    fn relay_receive_path_preserves_first_object_and_end_of_group() {
        let state = State::default();
        let (writer, _reader) =
            serve::Track::new(TrackNamespace::from_utf8_path("test/session"), "audio").produce();
        let mut recv = SubscribeRecv {
            state,
            writer: Some(writer.into()),
            processed_streams: 0,
            active_streams: 0,
            terminal: None,
            progress: Arc::new(Notify::new()),
            info: subscribe_info_with(KeyValuePairs::default()),
            delivery_filter: None,
            joining_location: None,
            join_barrier: None,
            joining_writers: HashMap::new(),
            seen_objects: HashSet::new(),
            _request_lease: crate::session::test_request_lease(
                crate::session::RequestDirection::Outbound,
                crate::session::RequestClass::Subscribe,
            ),
        };
        let header_type = data::StreamHeaderType::subgroup(
            true,
            data::SubgroupIdMode::Explicit,
            true,
            false,
            true,
        );

        let subgroup = recv
            .subgroup(data::SubgroupHeader {
                header_type,
                track_alias: 1,
                group_id: 2,
                subgroup_id: Some(3),
                publisher_priority: 4,
            })
            .unwrap();

        assert!(subgroup.first_object);
        assert!(subgroup.end_of_group);
    }

    #[tokio::test]
    async fn joining_barrier_emits_fetch_then_buffered_live_without_duplicates() {
        let (mut recv, reader) = joining_recv();
        recv.begin_joining_fetch().unwrap();

        // A raced live copy at or before the subsequently frozen cutoff is
        // discarded so the FETCH stream remains the sole source for it.
        assert!(recv.claim_object(0, 0));
        recv.recv_joining_live_object(buffered(0, 0, b"raced"))
            .unwrap();
        let mut params = KeyValuePairs::default();
        params.set_largest_object(Location::new(0, 1)).unwrap();
        recv.ok(&message::SubscribeOk {
            id: 0,
            track_alias: 0,
            params,
            track_extensions: Default::default(),
        })
        .unwrap();
        assert!(!recv.claim_object(0, 1));

        assert!(recv.claim_object(0, 2));
        let mut live = buffered(0, 2, b"live");
        live.group_end = EndOfGroupState::Signaled;
        recv.recv_joining_live_object(live).unwrap();
        let mut fetch_zero = buffered(0, 0, b"fetch-0");
        fetch_zero.group_end = EndOfGroupState::UnknownFromFetch;
        recv.recv_fetched_object(fetch_zero).unwrap();
        let mut fetch_one = buffered(0, 1, b"fetch-1");
        fetch_one.group_end = EndOfGroupState::UnknownFromFetch;
        recv.recv_fetched_object(fetch_one).unwrap();
        // Duplicate FETCH copies are ignored by the shared location set.
        recv.recv_fetched_object(buffered(0, 1, b"duplicate"))
            .unwrap();
        recv.finish_joining_fetch().unwrap();

        let serve::TrackReaderMode::Subgroups(mut groups) = reader.mode().await.unwrap() else {
            panic!("Joining FETCH must preserve subgroup delivery");
        };
        let mut subgroup = groups.next().await.unwrap().unwrap();
        assert_eq!(subgroup.len(), 3);
        assert!(subgroup.end_of_group);
        let mut payloads = Vec::new();
        for _ in 0..3 {
            payloads.push(
                subgroup
                    .next()
                    .await
                    .unwrap()
                    .unwrap()
                    .read_all()
                    .await
                    .unwrap(),
            );
        }
        assert_eq!(
            payloads,
            [
                bytes::Bytes::from_static(b"fetch-0"),
                bytes::Bytes::from_static(b"fetch-1"),
                bytes::Bytes::from_static(b"live"),
            ]
        );
    }

    #[tokio::test]
    async fn late_fetch_without_live_evidence_keeps_end_of_group_unknown() {
        let (mut recv, reader) = joining_recv();
        recv.begin_joining_fetch().unwrap();
        let mut params = KeyValuePairs::default();
        params.set_largest_object(Location::new(0, 1)).unwrap();
        recv.ok(&message::SubscribeOk {
            id: 0,
            track_alias: 0,
            params,
            track_extensions: Default::default(),
        })
        .unwrap();

        for (object_id, payload) in [(0, b"fetch-0" as &[u8]), (1, b"fetch-1")] {
            let mut object = buffered(0, object_id, payload);
            object.group_end = EndOfGroupState::UnknownFromFetch;
            recv.recv_fetched_object(object).unwrap();
        }
        recv.finish_joining_fetch().unwrap();

        let serve::TrackReaderMode::Subgroups(mut groups) = reader.mode().await.unwrap() else {
            panic!("Joining FETCH must preserve subgroup delivery");
        };
        let subgroup = groups.next().await.unwrap().unwrap();
        assert!(!subgroup.end_of_group);
    }

    #[test]
    fn cold_join_without_a_cutoff_resumes_the_ordinary_live_path() {
        let (mut recv, _reader) = joining_recv();
        recv.begin_joining_fetch().unwrap();
        recv.ok(&message::SubscribeOk {
            id: 0,
            track_alias: 0,
            params: KeyValuePairs::default(),
            track_extensions: Default::default(),
        })
        .unwrap();

        recv.fall_back_to_live_without_fetch().unwrap();
        assert!(!recv.has_joining_barrier());
    }

    #[tokio::test]
    async fn cold_join_flushes_a_live_object_that_raced_subscribe_ok() {
        let (mut recv, reader) = joining_recv();
        recv.begin_joining_fetch().unwrap();
        assert!(recv.claim_object(0, 0));
        let mut raced = buffered(0, 0, b"raced-live");
        raced.group_end = EndOfGroupState::Signaled;
        recv.recv_joining_live_object(raced).unwrap();
        recv.ok(&message::SubscribeOk {
            id: 0,
            track_alias: 0,
            params: KeyValuePairs::default(),
            track_extensions: Default::default(),
        })
        .unwrap();

        recv.fall_back_to_live_without_fetch().unwrap();
        let serve::TrackReaderMode::Subgroups(mut groups) = reader.mode().await.unwrap() else {
            panic!("cold Joining fallback must preserve subgroup delivery");
        };
        let mut subgroup = groups.next().await.unwrap().unwrap();
        assert!(subgroup.end_of_group);
        assert_eq!(
            subgroup
                .next()
                .await
                .unwrap()
                .unwrap()
                .read_all()
                .await
                .unwrap(),
            bytes::Bytes::from_static(b"raced-live")
        );
    }

    #[test]
    fn joining_barrier_is_hard_bounded() {
        let mut barrier = JoinBarrier::new();
        for object_id in 0..JOIN_BARRIER_MAX_OBJECTS {
            barrier.buffer(buffered(1, object_id as u64, b"")).unwrap();
        }
        assert!(matches!(
            barrier.buffer(buffered(1, JOIN_BARRIER_MAX_OBJECTS as u64, b"")),
            Err(ServeError::Closed(code))
                if code == message::RequestErrorCode::ExcessiveLoad as u64
        ));
        assert_eq!(barrier.buffered.len(), JOIN_BARRIER_MAX_OBJECTS);

        let mut bytes = JoinBarrier::new();
        assert!(matches!(
            bytes.buffer(BufferedJoinObject {
                payload: bytes::Bytes::from(vec![0; JOIN_BARRIER_MAX_BYTES + 1]),
                ..buffered(1, 0, b"")
            }),
            Err(ServeError::Closed(_))
        ));
    }
}
