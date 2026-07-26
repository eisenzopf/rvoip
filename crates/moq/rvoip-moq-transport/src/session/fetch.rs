// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Draft-19 Joining FETCH request lifecycles.
//!
//! The first supported profile is deliberately narrow: Relative Joining
//! FETCH with offset zero, ascending Group Order, and subgroup Objects.  The
//! narrow profile gives callers a gap-free handoff from retained media to an
//! established live subscription without pretending that the other FETCH
//! variants are interoperable yet.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use bytes::{Bytes, BytesMut};

use crate::{
    coding::{KeyValuePairs, Location, ReasonPhrase},
    data::{
        FetchForwardingPreference, FetchHeader, FetchItem, FetchObject as WireFetchObject,
        FetchObjectEncoder, StreamHeaderType,
    },
    message::{self, FetchType, GroupOrder, RequestErrorCode},
    serve::{
        RetainedObject, RetainedRange, RetainedSnapshot, RetentionError, RetentionLimit, ServeError,
    },
    watch::{Queue, State},
};

use super::{Publisher, RequestLease, Session, SessionError, Subscriber, Writer};

const FETCH_OBJECT_QUEUE_CAPACITY: usize = 1024;
const FETCH_RETENTION_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
pub(super) const FETCH_OVERLOAD_RETRY_INTERVAL: u64 = 1001;

/// The supported Joining FETCH profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JoiningFetchProfile {
    pub joining_start: u64,
    pub group_order: GroupOrder,
}

impl Default for JoiningFetchProfile {
    fn default() -> Self {
        Self {
            joining_start: 0,
            group_order: GroupOrder::Ascending,
        }
    }
}

/// One complete Object received on a FETCH data stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EndOfGroupState {
    /// A live SUBGROUP_HEADER explicitly carried END_OF_GROUP.
    Signaled,
    /// A live SUBGROUP_HEADER did not carry END_OF_GROUP.
    NotSignaled,
    /// Draft-19 FETCH serialization cannot carry END_OF_GROUP.
    UnknownFromFetch,
}

impl EndOfGroupState {
    pub const fn from_live_header(signaled: bool) -> Self {
        if signaled {
            Self::Signaled
        } else {
            Self::NotSignaled
        }
    }

    pub const fn is_signaled(self) -> bool {
        matches!(self, Self::Signaled)
    }
}

/// One complete Object received on a FETCH data stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FetchedObject {
    pub location: Location,
    pub subgroup_id: u64,
    pub publisher_priority: u8,
    pub properties: crate::data::ExtensionHeaders,
    pub payload: Bytes,
    /// Always [`EndOfGroupState::UnknownFromFetch`] for standards-compliant
    /// draft-19 FETCH. This prevents callers from mistaking range completion
    /// for an END_OF_GROUP assertion.
    pub group_end: EndOfGroupState,
}

#[derive(Clone, Debug)]
struct FetchState {
    response: Option<Result<message::FetchOk, ServeError>>,
    data_started: bool,
    data_finished: bool,
    closed: Result<(), ServeError>,
}

impl Default for FetchState {
    fn default() -> Self {
        Self {
            response: None,
            data_started: false,
            data_finished: false,
            closed: Ok(()),
        }
    }
}

/// An outbound Joining FETCH owned by a [`Subscriber`].
#[must_use = "dropping a FETCH cancels its request stream"]
pub struct Fetch {
    state: State<FetchState>,
    objects: Queue<FetchedObject>,
    subscriber: Subscriber,
    response_cancel: Option<tokio::sync::oneshot::Sender<()>>,
    pub id: u64,
    pub joining_request_id: u64,
    pub joining_location: Location,
    _request_lease: Arc<RequestLease>,
}

impl Fetch {
    pub(super) fn new(
        subscriber: Subscriber,
        id: u64,
        joining_request_id: u64,
        joining_location: Location,
        request_lease: Arc<RequestLease>,
    ) -> (Self, FetchRecv) {
        let (state, recv_state) = State::new(FetchState::default()).split();
        let (objects, recv_objects) = Queue::bounded(FETCH_OBJECT_QUEUE_CAPACITY).split();
        (
            Self {
                state,
                objects,
                subscriber,
                response_cancel: None,
                id,
                joining_request_id,
                joining_location,
                _request_lease: request_lease.clone(),
            },
            FetchRecv {
                state: recv_state,
                objects: recv_objects,
                joining_request_id,
                joining_location,
                last_location: None,
                _request_lease: request_lease,
            },
        )
    }

    pub(super) fn attach_response_cancel(
        &mut self,
        response_cancel: tokio::sync::oneshot::Sender<()>,
    ) {
        self.response_cancel = Some(response_cancel);
    }

    /// Wait for FETCH_OK and return the publisher's immutable response.
    pub async fn ok(&self) -> Result<message::FetchOk, ServeError> {
        loop {
            let notified = {
                let state = self.state.lock();
                state.closed.clone()?;
                if let Some(response) = &state.response {
                    return response.clone();
                }
                state.modified().ok_or(ServeError::Done)?
            };
            notified.await;
        }
    }

    /// Return the next fetched Object. `None` follows a clean data-stream FIN.
    pub async fn next_object(&mut self) -> Option<FetchedObject> {
        self.objects.pop().await
    }

    /// Wait for both FETCH_OK and a clean FIN on the single FETCH data stream.
    pub async fn completed(&self) -> Result<(), ServeError> {
        loop {
            let notified = {
                let state = self.state.lock();
                state.closed.clone()?;
                if state.response.as_ref().is_some_and(Result::is_ok) && state.data_finished {
                    return Ok(());
                }
                state.modified().ok_or(ServeError::Done)?
            };
            notified.await;
        }
    }
}

impl Drop for Fetch {
    fn drop(&mut self) {
        if let Some(cancel) = self.response_cancel.take() {
            let _ = cancel.send(());
        }
        self.subscriber.cancel_fetch(self.id, ServeError::Cancel);
        self._request_lease.release();
    }
}

pub(super) struct FetchRecv {
    state: State<FetchState>,
    objects: Queue<FetchedObject>,
    pub(super) joining_request_id: u64,
    joining_location: Location,
    last_location: Option<Location>,
    _request_lease: Arc<RequestLease>,
}

impl FetchRecv {
    pub(super) fn release_request_lease(&self) {
        self._request_lease.release();
    }

    pub(super) fn recv_ok(&mut self, ok: &message::FetchOk) -> Result<(), ServeError> {
        let mut state = self.state.lock_mut().ok_or(ServeError::Cancel)?;
        state.closed.clone()?;
        if state.response.is_some() {
            return Err(ServeError::Duplicate);
        }
        if ok.end_location != self.joining_location {
            return Err(ServeError::internal_ctx(
                "FETCH_OK end location did not match the frozen Joining Location",
            ));
        }
        state.response = Some(Ok(ok.clone()));
        Ok(())
    }

    pub(super) fn start_data(&mut self) -> Result<(), ServeError> {
        let mut state = self.state.lock_mut().ok_or(ServeError::Cancel)?;
        state.closed.clone()?;
        if state.data_started || state.data_finished {
            return Err(ServeError::Duplicate);
        }
        state.data_started = true;
        Ok(())
    }

    pub(super) fn recv_object(&mut self, object: FetchedObject) -> Result<(), ServeError> {
        let state = self.state.lock();
        state.closed.clone()?;
        if !state.data_started || state.data_finished {
            return Err(ServeError::internal_ctx(
                "FETCH Object arrived outside the data-stream lifecycle",
            ));
        }
        drop(state);

        if object.location > self.joining_location
            || self
                .last_location
                .is_some_and(|previous| object.location <= previous)
        {
            return Err(ServeError::internal_ctx(
                "FETCH Objects were outside the immutable ascending range",
            ));
        }
        self.last_location = Some(object.location);
        self.objects
            .push(object)
            .map_err(|_| ServeError::internal_ctx("bounded FETCH Object queue capacity exhausted"))
    }

    pub(super) fn finish_data(&mut self) -> Result<(), ServeError> {
        let mut state = self.state.lock_mut().ok_or(ServeError::Cancel)?;
        state.closed.clone()?;
        if !state.data_started || state.data_finished {
            return Err(ServeError::Duplicate);
        }
        state.data_finished = true;
        self._request_lease.release();
        Ok(())
    }

    pub(super) fn fail(&mut self, error: ServeError) {
        if let Some(mut state) = self.state.lock_mut() {
            if state.response.is_none() {
                state.response = Some(Err(error.clone()));
            }
            state.closed = Err(error);
        }
        self._request_lease.release();
    }

    pub(super) fn response_received(&self) -> bool {
        self.state.lock().response.is_some()
    }

    pub(super) fn is_complete(&self) -> bool {
        let state = self.state.lock();
        state.response.as_ref().is_some_and(Result::is_ok) && state.data_finished
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FetchRequestedPhase {
    Pending,
    Responded,
    Completed,
    Cancelled,
}

#[derive(Debug)]
struct FetchRequestedState {
    phase: FetchRequestedPhase,
}

/// An inbound Relative Joining FETCH surfaced by a [`Publisher`].
#[must_use = "an inbound FETCH must be served or rejected"]
pub struct FetchRequested {
    publisher: Publisher,
    pub request: message::Fetch,
    state: Arc<Mutex<FetchRequestedState>>,
    _request_lease: Arc<RequestLease>,
}

impl FetchRequested {
    pub(super) fn new(
        publisher: Publisher,
        request: message::Fetch,
        request_lease: Arc<RequestLease>,
    ) -> (Self, FetchRequestedRecv) {
        let state = Arc::new(Mutex::new(FetchRequestedState {
            phase: FetchRequestedPhase::Pending,
        }));
        (
            Self {
                publisher,
                request,
                state: state.clone(),
                _request_lease: request_lease.clone(),
            },
            FetchRequestedRecv {
                state,
                _request_lease: request_lease,
            },
        )
    }

    pub fn id(&self) -> u64 {
        self.request.id
    }

    pub fn joining_request_id(&self) -> Option<u64> {
        self.request
            .joining_fetch
            .as_ref()
            .map(|joining| joining.joining_request_id)
    }

    /// Serve the supported Joining FETCH from the referenced subscription's
    /// bounded, immutable retention snapshot.
    pub async fn serve(mut self) -> Result<(), SessionError> {
        let result = self.serve_inner().await;
        if let Err(error) = &result {
            if self.phase() == FetchRequestedPhase::Pending {
                let (code, reason) = request_error_for_session(error);
                self.reject(code, &reason)?;
            }
        }
        result
    }

    async fn serve_inner(&mut self) -> Result<(), SessionError> {
        let joining = validate_joining_request(&self.request)?;
        let (subscription, retained) = tokio::time::timeout(
            FETCH_RETENTION_WAIT,
            self.publisher
                .resolve_joining_subscription(joining.joining_request_id),
        )
        .await
        .map_err(|_| SessionError::Serve(ServeError::Closed(RequestErrorCode::Timeout as u64)))?
        .map_err(SessionError::Serve)?;
        let range = relative_joining_range(subscription.joining_location)?;

        let snapshot = tokio::time::timeout(
            FETCH_RETENTION_WAIT,
            retained.snapshot_when_available(range, GroupOrder::Ascending),
        )
        .await
        .map_err(|_| SessionError::Serve(ServeError::Closed(RequestErrorCode::Timeout as u64)))?
        .map_err(retention_to_session_error)?;
        validate_subgroup_snapshot(&snapshot)?;

        // Open the only data stream before committing the positive response.
        // A local stream-admission failure can still be represented by the
        // request's single REQUEST_ERROR response.
        let stream = self.publisher.open_uni().await?;
        let mut writer = Writer::new(stream);
        writer.reset_on_drop(Session::REQUEST_STREAM_CANCELLED);

        self.mark_responded()?;
        self.publisher
            .send_message_and_wait(message::FetchOk {
                id: self.request.id,
                end_of_track: false,
                end_location: subscription.joining_location,
                params: Default::default(),
                track_extensions: Default::default(),
            })
            .await;

        write_snapshot(self.request.id, &snapshot, &mut writer).await?;
        writer.finish();
        self.mark_completed();
        self._request_lease.release();
        Ok(())
    }

    /// Reject the FETCH with its one terminal REQUEST_ERROR.
    pub fn reject(
        &mut self,
        error_code: RequestErrorCode,
        reason: &str,
    ) -> Result<(), SessionError> {
        self.reject_with_retry(error_code, 0, reason)
    }

    /// Reject the FETCH with an explicit draft-19 retry interval. The value
    /// is the minimum delay in milliseconds plus one (`0` means no retry).
    pub fn reject_with_retry(
        &mut self,
        error_code: RequestErrorCode,
        retry_interval: u64,
        reason: &str,
    ) -> Result<(), SessionError> {
        self.mark_responded()?;
        self.publisher.send_request_error(
            "fetch",
            fetch_request_error(self.request.id, error_code, retry_interval, reason),
        );
        self._request_lease.release();
        Ok(())
    }

    fn phase(&self) -> FetchRequestedPhase {
        self.state
            .lock()
            .map(|state| state.phase)
            .unwrap_or(FetchRequestedPhase::Cancelled)
    }

    fn mark_responded(&self) -> Result<(), SessionError> {
        let mut state = self.state.lock().map_err(|_| SessionError::Internal)?;
        if state.phase != FetchRequestedPhase::Pending {
            return Err(SessionError::ProtocolViolation(
                "FETCH request already received its terminal response".to_string(),
            ));
        }
        state.phase = FetchRequestedPhase::Responded;
        Ok(())
    }

    fn mark_completed(&self) {
        if let Ok(mut state) = self.state.lock() {
            if state.phase == FetchRequestedPhase::Responded {
                state.phase = FetchRequestedPhase::Completed;
            }
        }
    }
}

fn fetch_request_error(
    id: u64,
    error_code: RequestErrorCode,
    retry_interval: u64,
    reason: &str,
) -> message::RequestError {
    message::RequestError {
        id,
        error_code: error_code as u64,
        retry_interval,
        reason: ReasonPhrase(reason.to_string()),
        redirect: None,
    }
}

impl Drop for FetchRequested {
    fn drop(&mut self) {
        self._request_lease.release();
        if self.phase() != FetchRequestedPhase::Pending {
            return;
        }
        let _ = self.reject(RequestErrorCode::Uninterested, "FETCH request dropped");
    }
}

pub(super) struct FetchRequestedRecv {
    state: Arc<Mutex<FetchRequestedState>>,
    _request_lease: Arc<RequestLease>,
}

impl FetchRequestedRecv {
    pub(super) fn cancel(&self) {
        if let Ok(mut state) = self.state.lock() {
            if state.phase == FetchRequestedPhase::Pending {
                state.phase = FetchRequestedPhase::Cancelled;
            }
        }
        self._request_lease.release();
    }
}

pub(super) fn validate_joining_request(
    request: &message::Fetch,
) -> Result<&message::JoiningFetch, SessionError> {
    if request.fetch_type != FetchType::RelativeJoining
        || request.standalone_fetch.is_some()
        || request.joining_fetch.is_none()
    {
        return Err(SessionError::Serve(ServeError::NotImplemented(
            "only Relative Joining FETCH is supported".to_string(),
        )));
    }
    let joining = request.joining_fetch.as_ref().expect("checked above");
    if joining.joining_start != 0 {
        return Err(SessionError::Serve(ServeError::NotImplemented(
            "only Relative Joining FETCH offset zero is supported".to_string(),
        )));
    }
    if request.params.group_order()? != Some(GroupOrder::Ascending) {
        return Err(SessionError::Serve(ServeError::NotImplemented(
            "Joining FETCH requires explicit ascending Group Order".to_string(),
        )));
    }
    Ok(joining)
}

pub(super) fn relative_joining_range(
    joining_location: Location,
) -> Result<RetainedRange, SessionError> {
    let end = if let Some(object_id) = joining_location.object_id.checked_add(1) {
        Location::new(joining_location.group_id, object_id)
    } else {
        Location::new(
            joining_location.group_id.checked_add(1).ok_or_else(|| {
                SessionError::Serve(ServeError::Closed(RequestErrorCode::InvalidRange as u64))
            })?,
            0,
        )
    };
    Ok(RetainedRange::new(
        Location::new(joining_location.group_id, 0),
        end,
    ))
}

fn validate_subgroup_snapshot(snapshot: &RetainedSnapshot) -> Result<(), SessionError> {
    let mut next_object = HashMap::new();
    for object in snapshot.iter() {
        let subgroup_id = object.subgroup_id().ok_or_else(|| {
            SessionError::Serve(ServeError::NotImplemented(
                "Joining FETCH currently supports subgroup Objects only".to_string(),
            ))
        })?;
        let expected = next_object
            .entry((object.group_id(), subgroup_id))
            .or_insert(0);
        if object.object_id() != *expected {
            return Err(SessionError::Serve(ServeError::NotImplemented(
                "sparse subgroup FETCH handoff is not supported".to_string(),
            )));
        }
        *expected = expected.checked_add(1).ok_or_else(|| {
            SessionError::Serve(ServeError::Closed(RequestErrorCode::InvalidRange as u64))
        })?;
    }
    Ok(())
}

async fn write_snapshot(
    request_id: u64,
    snapshot: &RetainedSnapshot,
    writer: &mut Writer,
) -> Result<(), SessionError> {
    writer
        .encode(&FetchHeader {
            header_type: StreamHeaderType::Fetch,
            request_id,
        })
        .await?;
    let mut encoder = FetchObjectEncoder::new(GroupOrder::Ascending)?;
    for retained in snapshot.iter() {
        write_retained_object(&mut encoder, retained, writer).await?;
    }
    Ok(())
}

async fn write_retained_object(
    encoder: &mut FetchObjectEncoder,
    retained: &RetainedObject,
    writer: &mut Writer,
) -> Result<(), SessionError> {
    // Draft-19 FETCH has no END_OF_GROUP field. Keep the assertion in the
    // retention model, but do not fabricate a private wire property here.
    let object = WireFetchObject {
        group_id: retained.group_id(),
        object_id: retained.object_id(),
        forwarding_preference: FetchForwardingPreference::Subgroup(
            retained.subgroup_id().ok_or_else(|| {
                SessionError::Serve(ServeError::NotImplemented(
                    "datagram Object in subgroup-only FETCH snapshot".to_string(),
                ))
            })?,
        ),
        publisher_priority: retained.publisher_priority(),
        properties: retained.properties().clone(),
        payload_length: retained.payload().len(),
    };
    let mut header = BytesMut::new();
    encoder.encode(&FetchItem::Object(object), &mut header)?;
    writer.write(&header).await?;
    writer.write(retained.payload()).await?;
    Ok(())
}

fn retention_to_session_error(error: RetentionError) -> SessionError {
    let code = match error {
        RetentionError::ExcessiveLoad(
            RetentionLimit::ActiveSnapshots | RetentionLimit::PinnedGroups,
        ) => RequestErrorCode::ExcessiveLoad,
        RetentionError::ExcessiveLoad(_) => RequestErrorCode::ExcessiveLoad,
        RetentionError::InvalidRange(_) => RequestErrorCode::InvalidRange,
    };
    SessionError::Serve(ServeError::Closed(code as u64))
}

fn request_error_for_session(error: &SessionError) -> (RequestErrorCode, String) {
    let code = match error {
        SessionError::Serve(ServeError::Closed(code))
            if *code == RequestErrorCode::Timeout as u64 =>
        {
            RequestErrorCode::Timeout
        }
        SessionError::Serve(ServeError::Closed(code))
            if *code == RequestErrorCode::InvalidJoiningRequestId as u64 =>
        {
            RequestErrorCode::InvalidJoiningRequestId
        }
        SessionError::Serve(ServeError::Closed(code))
            if *code == RequestErrorCode::InvalidRange as u64 =>
        {
            RequestErrorCode::InvalidRange
        }
        SessionError::Serve(ServeError::Closed(code))
            if *code == RequestErrorCode::ExcessiveLoad as u64 =>
        {
            RequestErrorCode::ExcessiveLoad
        }
        SessionError::Serve(ServeError::NotImplemented(_))
        | SessionError::Serve(ServeError::NotImplementedWithId(_, _)) => {
            RequestErrorCode::NotSupported
        }
        _ => RequestErrorCode::InternalError,
    };
    (code, error.to_string())
}

pub(super) fn outbound_joining_message(id: u64, joining_request_id: u64) -> message::Fetch {
    let mut params = KeyValuePairs::default();
    params.set_group_order(GroupOrder::Ascending);
    message::Fetch {
        id,
        fetch_type: FetchType::RelativeJoining,
        standalone_fetch: None,
        joining_fetch: Some(message::JoiningFetch {
            joining_request_id,
            joining_start: 0,
        }),
        params,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        coding::{Decode, Encode},
        serve::RetentionRangeError,
    };

    fn fetch_recv(
        joining_location: Location,
    ) -> (State<FetchState>, Queue<FetchedObject>, FetchRecv) {
        let (state, recv_state) = State::new(FetchState::default()).split();
        let (objects, recv_objects) = Queue::bounded(8).split();
        (
            state,
            objects,
            FetchRecv {
                state: recv_state,
                objects: recv_objects,
                joining_request_id: 2,
                joining_location,
                last_location: None,
                _request_lease: crate::session::test_request_lease(
                    crate::session::RequestDirection::Outbound,
                    crate::session::RequestClass::Fetch,
                ),
            },
        )
    }

    fn fetched(group_id: u64, object_id: u64) -> FetchedObject {
        FetchedObject {
            location: Location::new(group_id, object_id),
            subgroup_id: 0,
            publisher_priority: 7,
            properties: Default::default(),
            payload: Bytes::from_static(b"opus"),
            group_end: EndOfGroupState::UnknownFromFetch,
        }
    }

    #[test]
    fn joining_range_is_inclusive_on_the_wire_and_half_open_in_cache() {
        assert_eq!(
            relative_joining_range(Location::new(7, 9)).unwrap(),
            RetainedRange::new(Location::new(7, 0), Location::new(7, 10))
        );
        assert_eq!(
            relative_joining_range(Location::new(7, u64::MAX)).unwrap(),
            RetainedRange::new(Location::new(7, 0), Location::new(8, 0))
        );
        assert!(relative_joining_range(Location::new(u64::MAX, u64::MAX)).is_err());
    }

    #[test]
    fn fetched_objects_expose_group_end_as_unknown() {
        assert_eq!(fetched(1, 0).group_end, EndOfGroupState::UnknownFromFetch);
    }

    #[test]
    fn outbound_profile_has_stable_wire_and_rejects_unsupported_variants() {
        let request = outbound_joining_message(8, 6);
        validate_joining_request(&request).unwrap();
        let mut wire = BytesMut::new();
        request.encode(&mut wire).unwrap();
        let decoded = message::Fetch::decode(&mut wire).unwrap();
        assert_eq!(decoded, request);

        let mut nonzero = request.clone();
        nonzero.joining_fetch.as_mut().unwrap().joining_start = 1;
        assert!(matches!(
            validate_joining_request(&nonzero),
            Err(SessionError::Serve(ServeError::NotImplemented(_)))
        ));

        let mut descending = request;
        descending.params.set_group_order(GroupOrder::Descending);
        assert!(validate_joining_request(&descending).is_err());
    }

    #[test]
    fn unsupported_fetch_types_are_not_silently_reinterpreted() {
        let mut request = outbound_joining_message(8, 6);
        request.fetch_type = FetchType::AbsoluteJoining;
        assert!(validate_joining_request(&request).is_err());
        request.fetch_type = FetchType::Standalone;
        request.joining_fetch = None;
        request.standalone_fetch = Some(message::StandaloneFetch {
            track_namespace: crate::coding::TrackNamespace::from_utf8_path("test"),
            track_name: "audio".into(),
            start_location: Location::new(0, 0),
            end_location: Location::new(0, 1),
        });
        assert!(validate_joining_request(&request).is_err());
    }

    #[test]
    fn sparse_or_datagram_snapshots_are_rejected_before_fetch_ok() {
        for (subgroup_id, object_id) in [(Some(0), 1), (None, 0)] {
            let track =
                crate::serve::RetainedTrack::new(crate::serve::RetentionLimits::default()).unwrap();
            track
                .commit(
                    crate::serve::RetainedObject::new(
                        crate::serve::RetainedObjectMetadata {
                            location: Location::new(1, object_id),
                            subgroup_id,
                            publisher_priority: 7,
                            properties: Default::default(),
                            end_of_group: true,
                        },
                        Bytes::from_static(b"opus"),
                    )
                    .unwrap(),
                )
                .unwrap();
            track.complete_group(1).unwrap();
            let snapshot = track
                .snapshot(
                    RetainedRange::new(Location::new(1, 0), Location::new(1, object_id + 1)),
                    GroupOrder::Ascending,
                )
                .unwrap();
            assert!(matches!(
                validate_subgroup_snapshot(&snapshot),
                Err(SessionError::Serve(ServeError::NotImplemented(_)))
            ));
        }
    }

    #[test]
    fn retention_errors_map_to_request_scope_not_session_shutdown() {
        for error in [
            RetentionError::ExcessiveLoad(RetentionLimit::ActiveSnapshots),
            RetentionError::ExcessiveLoad(RetentionLimit::PinnedGroups),
            RetentionError::InvalidRange(RetentionRangeError::NotRetained),
        ] {
            assert!(matches!(
                retention_to_session_error(error),
                SessionError::Serve(ServeError::Closed(_))
            ));
        }
    }

    #[test]
    fn fetch_overload_rejection_used_by_queue_is_retryable() {
        let error = fetch_request_error(
            9,
            RequestErrorCode::ExcessiveLoad,
            FETCH_OVERLOAD_RETRY_INTERVAL,
            "capacity exhausted",
        );
        assert_eq!(error.id, 9);
        assert_eq!(error.error_code, RequestErrorCode::ExcessiveLoad as u64);
        assert_eq!(error.retry_interval, FETCH_OVERLOAD_RETRY_INTERVAL);
        assert_eq!(error.reason.0, "capacity exhausted");
    }

    #[tokio::test]
    async fn outbound_lifecycle_accepts_one_ok_one_stream_and_ascending_objects() {
        let (state, mut objects, mut recv) = fetch_recv(Location::new(3, 2));
        recv.start_data().unwrap();
        recv.recv_object(fetched(3, 0)).unwrap();
        recv.recv_object(fetched(3, 1)).unwrap();
        assert!(recv.recv_object(fetched(3, 1)).is_err());
        recv.recv_object(fetched(3, 2)).unwrap();
        recv.finish_data().unwrap();
        assert!(!recv.is_complete(), "data FIN alone cannot complete FETCH");

        recv.recv_ok(&message::FetchOk {
            id: 4,
            end_of_track: false,
            end_location: Location::new(3, 2),
            params: Default::default(),
            track_extensions: Default::default(),
        })
        .unwrap();
        assert!(recv.is_complete());
        assert!(recv
            .recv_ok(&message::FetchOk {
                id: 4,
                end_of_track: false,
                end_location: Location::new(3, 2),
                params: Default::default(),
                track_extensions: Default::default(),
            })
            .is_err());
        assert_eq!(objects.pop().await.unwrap().location, Location::new(3, 0));
        assert_eq!(objects.pop().await.unwrap().location, Location::new(3, 1));
        assert_eq!(objects.pop().await.unwrap().location, Location::new(3, 2));
        assert!(state.lock().data_finished);
    }

    #[test]
    fn empty_fetch_stream_completes_only_after_ok_and_fin() {
        let (_state, _objects, mut recv) = fetch_recv(Location::new(0, 0));
        recv.start_data().unwrap();
        recv.finish_data().unwrap();
        recv.recv_ok(&message::FetchOk {
            id: 4,
            end_of_track: false,
            end_location: Location::new(0, 0),
            params: Default::default(),
            track_extensions: Default::default(),
        })
        .unwrap();
        assert!(recv.is_complete());
        assert!(recv.start_data().is_err());
        assert!(recv.finish_data().is_err());
    }

    #[test]
    fn mismatched_fetch_ok_and_object_past_cutoff_are_rejected() {
        let (_state, _objects, mut recv) = fetch_recv(Location::new(1, 1));
        assert!(recv
            .recv_ok(&message::FetchOk {
                id: 4,
                end_of_track: false,
                end_location: Location::new(1, 2),
                params: Default::default(),
                track_extensions: Default::default(),
            })
            .is_err());
        recv.start_data().unwrap();
        assert!(recv.recv_object(fetched(1, 2)).is_err());
    }

    #[test]
    fn cancellation_is_terminal_and_cannot_be_mistaken_for_completion() {
        let (state, _objects, mut recv) = fetch_recv(Location::new(1, 1));
        recv.fail(ServeError::Cancel);
        let state = state.lock();
        assert!(matches!(&state.closed, Err(ServeError::Cancel)));
        assert!(matches!(&state.response, Some(Err(ServeError::Cancel))));
        assert!(!state.data_finished);

        let requested_state = Arc::new(Mutex::new(FetchRequestedState {
            phase: FetchRequestedPhase::Pending,
        }));
        let requested = FetchRequestedRecv {
            state: requested_state.clone(),
            _request_lease: crate::session::test_request_lease(
                crate::session::RequestDirection::Inbound,
                crate::session::RequestClass::Fetch,
            ),
        };
        requested.cancel();
        assert_eq!(
            requested_state.lock().unwrap().phase,
            FetchRequestedPhase::Cancelled
        );
    }
}
