// SPDX-FileCopyrightText: 2026 Cloudflare Inc.
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Subscriber-side handling for an inbound draft-19 `PUBLISH` request.

use std::{collections::HashSet, sync::Arc};
use tokio::sync::Notify;

use crate::{
    coding::{KeyValuePairs, Location, ReasonPhrase, TrackName, TrackNamespace},
    data,
    message::{self, RequestErrorCode},
    serve::{self, ServeError, TrackReader, TrackWriterMode},
    watch::State,
};

use super::{RequestLease, Session, SessionError, Subscriber};

#[derive(Debug)]
struct PublishReceivedState {
    closed: Result<(), ServeError>,
    forward: bool,
}

impl Default for PublishReceivedState {
    fn default() -> Self {
        Self {
            closed: Ok(()),
            forward: true,
        }
    }
}

/// An inbound publisher-initiated subscription.
///
/// The application must call [`accept`](Self::accept) after it has registered
/// the returned media track. Dropping an unaccepted request rejects it with a
/// `REQUEST_ERROR` on the request stream.
#[must_use = "accept or reject the inbound PUBLISH"]
pub struct PublishReceived {
    subscriber: Subscriber,
    state: State<PublishReceivedState>,
    reader: Option<TrackReader>,
    request_id: u64,
    track_alias: u64,
    namespace: TrackNamespace,
    name: TrackName,
    initial_forward: bool,
    largest_location: Option<Location>,
    accepted: bool,
    rejection: Option<ServeError>,
    _request_lease: Arc<RequestLease>,
}

impl PublishReceived {
    #[allow(clippy::too_many_arguments)]
    fn new(
        subscriber: Subscriber,
        state: State<PublishReceivedState>,
        reader: TrackReader,
        request_id: u64,
        track_alias: u64,
        namespace: TrackNamespace,
        name: TrackName,
        initial_forward: bool,
        largest_location: Option<Location>,
        request_lease: Arc<RequestLease>,
    ) -> Self {
        Self {
            subscriber,
            state,
            reader: Some(reader),
            request_id,
            track_alias,
            namespace,
            name,
            initial_forward,
            largest_location,
            accepted: false,
            rejection: None,
            _request_lease: request_lease,
        }
    }

    /// Move out the media reader before accepting the request.
    ///
    /// Relays use this ordering to register the exact full track name before
    /// sending `PUBLISH_OK`, avoiding a window where downstream subscriptions
    /// cannot resolve an already-accepted publication.
    pub fn take_reader(&mut self) -> Result<TrackReader, ServeError> {
        self.reader.take().ok_or(ServeError::Done)
    }

    /// Accept the publication with draft-19 `REQUEST_OK` (`PUBLISH_OK`).
    /// The Forward value can later be changed with [`set_forward`](Self::set_forward).
    pub fn accept(&mut self, forward: bool) -> Result<(), ServeError> {
        if self.accepted {
            return Err(ServeError::Duplicate);
        }

        if let Some(mut state) = self.state.lock_mut() {
            state.forward = forward;
        }
        self.subscriber
            .set_publish_forward(self.request_id, forward)
            .map_err(ServeError::from)?;

        let mut params = KeyValuePairs::default();
        params.set_forward(forward);
        self.subscriber.send_request_ok(
            "publish",
            message::RequestOk {
                id: self.request_id,
                params,
                track_properties: Default::default(),
            },
        );
        self.accepted = true;
        Ok(())
    }

    /// Update the accepted PUBLISH Forward state on the request's response direction.
    pub async fn set_forward(&mut self, forward: bool) -> Result<(), ServeError> {
        if !self.accepted {
            return Err(ServeError::Cancel);
        }
        self.subscriber
            .update_publish_received(self.request_id, forward)
            .await
            .map_err(ServeError::from)?;
        if let Some(mut state) = self.state.lock_mut() {
            state.forward = forward;
        }
        Ok(())
    }

    /// Cancel an accepted request with RESET_STREAM and STOP_SENDING.
    pub fn cancel(mut self) {
        self.subscriber
            .cancel_publish_received(self.request_id, Session::REQUEST_STREAM_CANCELLED);
        self.accepted = true;
    }

    /// Accept and return the media reader in one operation.
    pub fn ok(&mut self, forward: bool) -> Result<TrackReader, ServeError> {
        let reader = self.take_reader()?;
        self.accept(forward)?;
        Ok(reader)
    }

    /// Select the error returned when this unaccepted request is dropped.
    pub fn close(mut self, err: ServeError) {
        self.rejection = Some(err);
    }

    /// Wait for `PUBLISH_DONE` or request-stream failure.
    ///
    /// `TRACK_ENDED` is a clean completion and therefore returns `Ok(())`.
    pub async fn closed(&self) -> Result<(), ServeError> {
        loop {
            {
                let state = self.state.lock();
                match state.closed.clone() {
                    Ok(()) => {}
                    Err(ServeError::Done) => return Ok(()),
                    Err(err) => return Err(err),
                }
                match state.modified() {
                    Some(notify) => notify,
                    None => return Ok(()),
                }
            }
            .await;
        }
    }

    pub fn request_id(&self) -> u64 {
        self.request_id
    }

    pub fn namespace(&self) -> &TrackNamespace {
        &self.namespace
    }

    pub fn name(&self) -> &TrackName {
        &self.name
    }

    pub fn track_alias(&self) -> u64 {
        self.track_alias
    }

    pub fn initial_forward(&self) -> bool {
        self.initial_forward
    }

    pub fn largest_location(&self) -> Option<Location> {
        self.largest_location
    }
}

impl Drop for PublishReceived {
    fn drop(&mut self) {
        self._request_lease.release();
        if self.accepted {
            if self.state.lock().closed.is_ok() {
                self.subscriber
                    .cancel_publish_received(self.request_id, Session::REQUEST_STREAM_CANCELLED);
            }
            return;
        }

        if self.state.lock().closed.is_err() {
            self.subscriber.remove_publish_received(self.request_id);
            return;
        }

        let err = self.rejection.clone().unwrap_or(ServeError::Cancel);
        self.subscriber.send_request_error(
            "publish",
            message::RequestError {
                id: self.request_id,
                error_code: request_error_code(&err),
                retry_interval: retry_interval(&err),
                reason: ReasonPhrase(err.to_string()),
                redirect: None,
            },
        );
        self.subscriber.remove_publish_received(self.request_id);
    }
}

fn request_error_code(err: &ServeError) -> u64 {
    match err {
        ServeError::Cancel | ServeError::Done => RequestErrorCode::Uninterested as u64,
        ServeError::Duplicate => RequestErrorCode::Uninterested as u64,
        ServeError::NotFound | ServeError::NotFoundWithId(_, _) => {
            RequestErrorCode::DoesNotExist as u64
        }
        ServeError::Mode
        | ServeError::Size
        | ServeError::NotImplemented(_)
        | ServeError::NotImplementedWithId(_, _) => RequestErrorCode::NotSupported as u64,
        ServeError::Internal(_) | ServeError::InternalWithId(_, _) => {
            RequestErrorCode::InternalError as u64
        }
        ServeError::Closed(code) => *code,
    }
}

fn retry_interval(err: &ServeError) -> u64 {
    match err {
        ServeError::Closed(code) if *code == RequestErrorCode::ExcessiveLoad as u64 => 1_001,
        _ => 0,
    }
}

pub(crate) struct PublishReceivedRecv {
    state: State<PublishReceivedState>,
    writer: Option<TrackWriterMode>,
    processed_streams: u64,
    active_streams: u64,
    terminal: Option<(u64, u64)>,
    forward: bool,
    full_name: serve::FullTrackName,
    progress: Arc<Notify>,
    seen_objects: HashSet<(u64, u64)>,
    _request_lease: Arc<RequestLease>,
}

impl PublishReceivedRecv {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn produce(
        subscriber: Subscriber,
        request_id: u64,
        track_alias: u64,
        namespace: TrackNamespace,
        name: TrackName,
        initial_forward: bool,
        largest_location: Option<Location>,
        writer: serve::TrackWriter,
        reader: TrackReader,
        request_lease: Arc<RequestLease>,
    ) -> (PublishReceived, Self) {
        let full_name = serve::FullTrackName {
            namespace: namespace.clone(),
            name: name.clone(),
        };
        let (app_state, transport_state) = State::new(PublishReceivedState {
            closed: Ok(()),
            forward: initial_forward,
        })
        .split();
        let app = PublishReceived::new(
            subscriber,
            app_state,
            reader,
            request_id,
            track_alias,
            namespace,
            name,
            initial_forward,
            largest_location,
            request_lease.clone(),
        );
        let recv = Self {
            state: transport_state,
            writer: Some(writer.into()),
            processed_streams: 0,
            active_streams: 0,
            terminal: None,
            forward: initial_forward,
            full_name,
            progress: Arc::new(Notify::new()),
            seen_objects: HashSet::new(),
            _request_lease: request_lease,
        };
        (app, recv)
    }

    pub fn subgroup(
        &mut self,
        header: data::SubgroupHeader,
    ) -> Result<serve::SubgroupWriter, ServeError> {
        let writer = self.writer.take().ok_or(ServeError::Done)?;
        let mut subgroups = match writer {
            TrackWriterMode::Track(track) => track.subgroups()?,
            TrackWriterMode::Subgroups(subgroups) => subgroups,
            other => {
                self.writer = Some(other);
                return Err(ServeError::Mode);
            }
        };
        let subgroup = subgroups.create(serve::Subgroup::from_header(&header)?)?;
        self.writer = Some(subgroups.into());
        Ok(subgroup)
    }

    pub fn datagram(&mut self, datagram: data::Datagram) -> Result<(), ServeError> {
        let writer = self.writer.take().ok_or(ServeError::Done)?;
        let value = serve::Datagram {
            group_id: datagram.group_id,
            object_id: datagram.object_id.unwrap_or(0),
            priority: datagram.publisher_priority,
            payload: datagram.payload.unwrap_or_default(),
            extension_headers: datagram.extension_headers.unwrap_or_default(),
        };
        match writer {
            TrackWriterMode::Track(track) => {
                let mut datagrams = track.datagrams()?;
                datagrams.write(value)?;
                self.writer = Some(datagrams.into());
                Ok(())
            }
            TrackWriterMode::Datagrams(mut datagrams) => {
                datagrams.write(value)?;
                self.writer = Some(datagrams.into());
                Ok(())
            }
            other => {
                self.writer = Some(other);
                Err(ServeError::Mode)
            }
        }
    }

    pub fn set_forward(&mut self, forward: bool) {
        self.forward = forward;
        if let Some(mut state) = self.state.lock_mut() {
            state.forward = forward;
        }
    }

    pub fn allows(&self, _group_id: u64, _object_id: u64) -> bool {
        self.forward
    }

    /// Claim an Object for this publisher-initiated subscription, suppressing
    /// duplicate copies when a Track Alias is shared by overlapping requests.
    pub fn claim_object(&mut self, group_id: u64, object_id: u64) -> bool {
        self.allows(group_id, object_id) && self.seen_objects.insert((group_id, object_id))
    }

    pub fn full_name(&self) -> serve::FullTrackName {
        self.full_name.clone()
    }

    pub fn awaiting_streams(&self) -> bool {
        self.terminal
            .is_some_and(|(_, expected)| self.processed_streams < expected)
    }

    pub fn progress(&self) -> Arc<Notify> {
        self.progress.clone()
    }

    /// Record PUBLISH_DONE, retaining receive state until every announced
    /// object stream has been processed. Request streams can outrun lower
    /// priority object streams, so immediate cleanup would lose late media.
    pub fn recv_done(&mut self, status_code: u64, stream_count: u64) -> Result<bool, SessionError> {
        if self.processed_streams.saturating_add(self.active_streams) > stream_count {
            return Err(SessionError::ProtocolViolation(format!(
                "PUBLISH_DONE declared {} streams after {} were received",
                stream_count,
                self.processed_streams.saturating_add(self.active_streams)
            )));
        }
        self.terminal = Some((status_code, stream_count));
        self.progress.notify_one();
        Ok(self.maybe_finish())
    }

    pub fn begin_stream(&mut self, limit: u64) -> Result<(), ServeError> {
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

    pub fn finish_stream(&mut self) -> Result<bool, SessionError> {
        self.active_streams = self.active_streams.saturating_sub(1);
        self.processed_streams = self.processed_streams.saturating_add(1);
        self.progress.notify_one();
        if let Some((_, expected)) = self.terminal {
            if self.processed_streams > expected {
                return Err(SessionError::ProtocolViolation(format!(
                    "received more PUBLISH streams than declared: {} > {}",
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
        self.writer = None;
        true
    }

    pub fn recv_stream_error(&mut self, err: ServeError) {
        self.active_streams = self.active_streams.saturating_sub(1);
        self.progress.notify_one();
        if let Some(mut state) = self.state.lock_mut() {
            state.closed = Err(err);
        }
        self.writer = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recv() -> (State<PublishReceivedState>, PublishReceivedRecv) {
        let state = State::<PublishReceivedState>::default();
        let (app, transport) = state.split();
        (
            app,
            PublishReceivedRecv {
                state: transport,
                writer: None,
                processed_streams: 0,
                active_streams: 0,
                terminal: None,
                forward: true,
                full_name: serve::FullTrackName {
                    namespace: TrackNamespace::from_utf8_path("test"),
                    name: TrackName::from("track"),
                },
                progress: Arc::new(Notify::new()),
                seen_objects: HashSet::new(),
                _request_lease: crate::session::test_request_lease(
                    crate::session::RequestDirection::Inbound,
                    crate::session::RequestClass::Publish,
                ),
            },
        )
    }

    #[test]
    fn rejection_codes_are_semantic() {
        assert_eq!(
            request_error_code(&ServeError::Duplicate),
            RequestErrorCode::Uninterested as u64
        );
        assert_eq!(
            request_error_code(&ServeError::NotFound),
            RequestErrorCode::DoesNotExist as u64
        );
        let excessive = ServeError::Closed(RequestErrorCode::ExcessiveLoad as u64);
        assert_eq!(
            request_error_code(&excessive),
            RequestErrorCode::ExcessiveLoad as u64
        );
        assert_eq!(retry_interval(&excessive), 1_001);
        assert_eq!(retry_interval(&ServeError::Internal("fatal".into())), 0);
    }

    #[test]
    fn track_ended_is_recorded_as_clean_terminal_state() {
        let (_app, mut recv) = recv();
        assert!(recv
            .recv_done(message::PublishDoneCode::TrackEnded as u64, 0)
            .unwrap());
        assert!(matches!(recv.state.lock().closed, Err(ServeError::Done)));
    }

    #[test]
    fn publish_done_waits_for_late_declared_streams() {
        let (_app, mut recv) = recv();
        assert!(!recv
            .recv_done(message::PublishDoneCode::TrackEnded as u64, 1)
            .unwrap());
        assert!(matches!(recv.state.lock().closed, Ok(())));
        assert!(recv.finish_stream().unwrap());
        assert!(matches!(recv.state.lock().closed, Err(ServeError::Done)));
    }

    #[test]
    fn stream_admission_is_bounded_and_done_counts_active_streams() {
        let (_app, mut recv) = recv();
        for _ in 0..64 {
            recv.begin_stream(64).unwrap();
        }
        assert!(matches!(
            recv.begin_stream(64),
            Err(ServeError::Closed(code))
                if code == RequestErrorCode::ExcessiveLoad as u64
        ));
        assert!(matches!(
            recv.recv_done(message::PublishDoneCode::TrackEnded as u64, 63),
            Err(SessionError::ProtocolViolation(_))
        ));
    }

    #[test]
    fn maximum_declared_stream_count_still_requires_bounded_cleanup() {
        let (_app, mut recv) = recv();
        assert!(!recv
            .recv_done(
                message::PublishDoneCode::TrackEnded as u64,
                (1_u64 << 62) - 1,
            )
            .unwrap());
        assert!(recv.awaiting_streams());
    }

    #[test]
    fn stream_failure_releases_active_accounting_and_closes_publication() {
        let (_app, mut recv) = recv();
        recv.begin_stream(64).unwrap();
        recv.recv_stream_error(ServeError::Size);
        assert_eq!(recv.active_streams, 0);
        assert!(matches!(recv.state.lock().closed, Err(ServeError::Size)));
    }

    #[test]
    fn shared_alias_object_claims_are_deduplicated_per_publication() {
        let (_app, mut recv) = recv();
        assert!(recv.claim_object(7, 9));
        assert!(!recv.claim_object(7, 9));
        assert!(recv.claim_object(7, 10));

        recv.set_forward(false);
        assert!(!recv.claim_object(8, 0));
        recv.set_forward(true);
        assert!(recv.claim_object(8, 0));
    }

    #[test]
    fn reverse_publish_receive_path_preserves_first_object_and_end_of_group() {
        let (_app, mut recv) = recv();
        let (writer, _reader) =
            serve::Track::new(TrackNamespace::from_utf8_path("test"), "audio").produce();
        recv.writer = Some(writer.into());
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
}
