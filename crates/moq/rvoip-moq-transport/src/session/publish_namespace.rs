// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-FileCopyrightText: 2023-2024 Luke Curley and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{collections::VecDeque, ops, sync::Arc, time::Duration};

use crate::coding::TrackNamespace;
use crate::watch::State;
use crate::{
    message,
    serve::{ServeError, TracksReader},
};

use super::{
    Publisher, RequestLease, Session, SessionError, Subscribed, TrackStatusRequested, Writer,
};

/// Default time allowed for a peer to accept `PUBLISH_NAMESPACE`.
pub const DEFAULT_PUBLISH_NAMESPACE_ACCEPTANCE_TIMEOUT: Duration = Duration::from_secs(10);

/// Wire details retained when a peer rejects `PUBLISH_NAMESPACE`.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PublishNamespaceRejection {
    pub error_code: u64,
    pub retry_interval: u64,
    pub reason: crate::coding::ReasonPhrase,
    pub redirect: Option<message::Redirect>,
}

impl From<message::RequestError> for PublishNamespaceRejection {
    fn from(error: message::RequestError) -> Self {
        Self {
            error_code: error.error_code,
            retry_interval: error.retry_interval,
            reason: error.reason,
            redirect: error.redirect,
        }
    }
}

/// Observable acceptance state for an outbound `PUBLISH_NAMESPACE`.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum PublishNamespaceAcceptance {
    /// No `REQUEST_OK` or `REQUEST_ERROR` has arrived yet.
    Pending,
    /// The peer sent `REQUEST_OK` on the request stream.
    Accepted,
    /// The peer sent `REQUEST_ERROR` on the request stream.
    Rejected(PublishNamespaceRejection),
    /// The peer response direction or session closed before acceptance.
    ResponseStreamClosed,
}

/// Failure while waiting for explicit namespace acceptance.
#[derive(thiserror::Error, Debug, Clone, Eq, PartialEq)]
pub enum PublishNamespaceAcceptanceError {
    #[error("PUBLISH_NAMESPACE rejected: {0:?}")]
    Rejected(PublishNamespaceRejection),
    #[error("PUBLISH_NAMESPACE response stream closed before acceptance")]
    ResponseStreamClosed,
    #[error("PUBLISH_NAMESPACE acceptance timed out after {timeout:?}")]
    TimedOut { timeout: Duration },
}

impl From<PublishNamespaceAcceptanceError> for ServeError {
    fn from(error: PublishNamespaceAcceptanceError) -> Self {
        match error {
            PublishNamespaceAcceptanceError::Rejected(rejection) => {
                ServeError::Closed(rejection.error_code)
            }
            PublishNamespaceAcceptanceError::ResponseStreamClosed => ServeError::Cancel,
            PublishNamespaceAcceptanceError::TimedOut { timeout } => ServeError::Internal(format!(
                "PUBLISH_NAMESPACE acceptance timed out after {timeout:?}"
            )),
        }
    }
}

/// Information about an outbound PUBLISH_NAMESPACE request.
#[derive(Debug, Clone)]
pub struct PublishNamespaceInfo {
    pub request_id: u64,
    pub namespace: TrackNamespace,
}

struct PublishNamespaceState {
    subscribers: VecDeque<Subscribed>,
    track_statuses_requested: VecDeque<TrackStatusRequested>,
    acceptance: PublishNamespaceAcceptance,
    closed: Result<(), ServeError>,
}

impl Default for PublishNamespaceState {
    fn default() -> Self {
        Self {
            subscribers: Default::default(),
            track_statuses_requested: Default::default(),
            acceptance: PublishNamespaceAcceptance::Pending,
            closed: Ok(()),
        }
    }
}

impl Drop for PublishNamespaceState {
    fn drop(&mut self) {
        for subscriber in self.subscribers.drain(..) {
            subscriber
                .close(ServeError::not_found_ctx(
                    "publish_namespace dropped before subscription handled",
                ))
                .ok();
        }
        self.track_statuses_requested.clear();
    }
}

/// Represents an outbound PUBLISH_NAMESPACE sent by a publisher.
///
/// The request remains active for the lifetime of its bidirectional stream.
#[must_use = "keep the PUBLISH_NAMESPACE request stream alive"]
pub struct PublishNamespace {
    publisher: Publisher,
    state: State<PublishNamespaceState>,
    request_writer: Option<Writer>,
    response_cancel: Option<tokio::sync::oneshot::Sender<()>>,
    _request_lease: Arc<RequestLease>,

    pub info: PublishNamespaceInfo,
}

impl PublishNamespace {
    pub(super) fn publisher(&self) -> Publisher {
        self.publisher.clone()
    }

    /// Create a PublishNamespace without sending on the control stream.
    /// The caller sends via a bidi request stream (draft-19).
    pub(super) fn new(
        publisher: Publisher,
        request_id: u64,
        namespace: TrackNamespace,
        request_lease: Arc<RequestLease>,
    ) -> (PublishNamespace, PublishNamespaceRecv) {
        let info = PublishNamespaceInfo {
            request_id,
            namespace: namespace.clone(),
        };
        Self::from_parts(publisher, info, request_id, request_lease)
    }

    /// Return the wire message to send on the request stream.
    pub(super) fn wire_message(&self) -> message::PublishNamespace {
        message::PublishNamespace {
            id: self.info.request_id,
            track_namespace: self.info.namespace.clone(),
            params: Default::default(),
        }
    }

    fn from_parts(
        publisher: Publisher,
        info: PublishNamespaceInfo,
        request_id: u64,
        request_lease: Arc<RequestLease>,
    ) -> (PublishNamespace, PublishNamespaceRecv) {
        let (send, recv) = State::default().split();

        let send = Self {
            publisher,
            info,
            state: send,
            request_writer: None,
            response_cancel: None,
            _request_lease: request_lease.clone(),
        };
        let recv = PublishNamespaceRecv {
            state: recv,
            request_id,
            _request_lease: request_lease,
        };

        (send, recv)
    }

    pub(super) fn attach_request_stream(
        &mut self,
        writer: Writer,
        response_cancel: tokio::sync::oneshot::Sender<()>,
    ) {
        self.request_writer = Some(writer);
        self.response_cancel = Some(response_cancel);
    }

    fn observed_acceptance(state: &State<PublishNamespaceState>) -> PublishNamespaceAcceptance {
        let state = state.lock();
        let acceptance = state.acceptance.clone();
        if acceptance == PublishNamespaceAcceptance::Pending && state.modified().is_none() {
            PublishNamespaceAcceptance::ResponseStreamClosed
        } else {
            acceptance
        }
    }

    async fn wait_for_acceptance(
        state: &State<PublishNamespaceState>,
    ) -> Result<(), PublishNamespaceAcceptanceError> {
        loop {
            let modified = {
                let state = state.lock();
                match &state.acceptance {
                    PublishNamespaceAcceptance::Pending => state.modified(),
                    PublishNamespaceAcceptance::Accepted => return Ok(()),
                    PublishNamespaceAcceptance::Rejected(rejection) => {
                        return Err(PublishNamespaceAcceptanceError::Rejected(rejection.clone()))
                    }
                    PublishNamespaceAcceptance::ResponseStreamClosed => {
                        return Err(PublishNamespaceAcceptanceError::ResponseStreamClosed)
                    }
                }
            };

            let Some(modified) = modified else {
                return Err(PublishNamespaceAcceptanceError::ResponseStreamClosed);
            };
            modified.await;
        }
    }

    async fn wait_for_acceptance_with_timeout(
        state: &State<PublishNamespaceState>,
        timeout: Duration,
    ) -> Result<(), PublishNamespaceAcceptanceError> {
        match tokio::time::timeout(timeout, Self::wait_for_acceptance(state)).await {
            Ok(result) => result,
            Err(_) => match Self::observed_acceptance(state) {
                PublishNamespaceAcceptance::Accepted => Ok(()),
                PublishNamespaceAcceptance::Rejected(rejection) => {
                    Err(PublishNamespaceAcceptanceError::Rejected(rejection))
                }
                PublishNamespaceAcceptance::ResponseStreamClosed => {
                    Err(PublishNamespaceAcceptanceError::ResponseStreamClosed)
                }
                PublishNamespaceAcceptance::Pending => {
                    Err(PublishNamespaceAcceptanceError::TimedOut { timeout })
                }
            },
        }
    }

    /// Return the current acceptance state without waiting.
    pub fn acceptance_state(&self) -> PublishNamespaceAcceptance {
        Self::observed_acceptance(&self.state)
    }

    /// Wait until the peer explicitly accepts or rejects the request.
    pub async fn accepted(&self) -> Result<(), PublishNamespaceAcceptanceError> {
        Self::wait_for_acceptance(&self.state).await
    }

    /// Wait for explicit acceptance with a caller-selected deadline.
    pub async fn accepted_with_timeout(
        &self,
        timeout: Duration,
    ) -> Result<(), PublishNamespaceAcceptanceError> {
        Self::wait_for_acceptance_with_timeout(&self.state, timeout).await
    }

    /// Serve subscriptions and track-status requests for the accepted namespace.
    pub async fn serve(mut self, tracks: TracksReader) -> Result<(), SessionError> {
        self.accepted().await.map_err(ServeError::from)?;
        if Publisher::serve_publish_namespace(&self, tracks).await? {
            self.finish_request_stream().await?;
        }
        Ok(())
    }

    /// Gracefully finish the request and wait until the peer closes its
    /// response direction after observing our FIN.
    async fn finish_request_stream(&mut self) -> Result<(), SessionError> {
        let mut writer = self.request_writer.take().ok_or(SessionError::Internal)?;
        writer.finish();
        tokio::task::yield_now().await;
        self.closed().await?;
        // The response task has already exited (it owns the peer-facing state
        // whose closure woke `closed`), so dropping this sender cannot cancel
        // a live response direction.
        self.response_cancel.take();
        Ok(())
    }

    /// Wait until the namespace publish is closed (error or peer disconnect).
    pub async fn closed(&self) -> Result<(), ServeError> {
        loop {
            {
                let state = self.state.lock();
                state.closed.clone()?;

                match state.modified() {
                    Some(notified) => notified,
                    None => return Ok(()),
                }
            }
            .await;
        }
    }

    /// Wait until a subscriber arrives for this namespace.
    pub async fn subscribed(&self) -> Result<Option<Subscribed>, ServeError> {
        loop {
            {
                let state = self.state.lock();
                if !state.subscribers.is_empty() {
                    return Ok(state
                        .into_mut()
                        .and_then(|mut state| state.subscribers.pop_front()));
                }

                state.closed.clone()?;
                match state.modified() {
                    Some(notified) => notified,
                    None => return Ok(None),
                }
            }
            .await;
        }
    }

    /// Wait until a TRACK_STATUS request arrives for this namespace.
    pub async fn track_status_requested(&self) -> Result<Option<TrackStatusRequested>, ServeError> {
        loop {
            {
                let state = self.state.lock();
                if !state.track_statuses_requested.is_empty() {
                    return Ok(state
                        .into_mut()
                        .and_then(|mut state| state.track_statuses_requested.pop_front()));
                }

                state.closed.clone()?;
                match state.modified() {
                    Some(notified) => notified,
                    None => return Ok(None),
                }
            }
            .await;
        }
    }

    /// Wait until the peer has sent REQUEST_OK for this namespace.
    pub async fn ok(&self) -> Result<(), ServeError> {
        self.accepted().await.map_err(ServeError::from)
    }
}

impl Drop for PublishNamespace {
    fn drop(&mut self) {
        // Draft-19 removed PUBLISH_NAMESPACE_DONE. Completion/cancellation is
        // represented by closing or resetting the owning request stream.
        if let Some(cancel) = self.response_cancel.take() {
            let _ = cancel.send(());
        }
        if let Some(writer) = self.request_writer.as_mut() {
            writer.reset(Session::REQUEST_STREAM_CANCELLED);
        }
        let _ = self.publisher.drop_publish_namespace(self.info.request_id);
        self._request_lease.release();
    }
}

impl ops::Deref for PublishNamespace {
    type Target = PublishNamespaceInfo;

    fn deref(&self) -> &Self::Target {
        &self.info
    }
}

/// Peer-facing handle for tracking a PUBLISH_NAMESPACE request.
pub(super) struct PublishNamespaceRecv {
    state: State<PublishNamespaceState>,
    /// Request ID of the outbound PUBLISH_NAMESPACE.
    // Namespace lookup alone is insufficient: both request_id and namespace
    // are needed, so Publisher holds a second index by request_id.
    pub request_id: u64,
    _request_lease: Arc<RequestLease>,
}

impl PublishNamespaceRecv {
    pub(super) fn release_request_lease(&self) {
        self._request_lease.release();
    }

    pub fn recv_ok(&mut self) -> Result<(), ServeError> {
        if let Some(mut state) = self.state.lock_mut() {
            match state.acceptance.clone() {
                PublishNamespaceAcceptance::Pending => {
                    state.acceptance = PublishNamespaceAcceptance::Accepted;
                }
                PublishNamespaceAcceptance::Accepted => return Err(ServeError::Duplicate),
                PublishNamespaceAcceptance::Rejected(_)
                | PublishNamespaceAcceptance::ResponseStreamClosed => return Err(ServeError::Done),
            }
        }

        Ok(())
    }

    pub fn recv_rejected(self, rejection: PublishNamespaceRejection) -> Result<(), ServeError> {
        let state = self.state.lock();
        state.closed.clone()?;

        let mut state = state.into_mut().ok_or(ServeError::Done)?;
        state.closed = Err(ServeError::Closed(rejection.error_code));
        state.acceptance = PublishNamespaceAcceptance::Rejected(rejection);

        Ok(())
    }

    pub fn recv_response_stream_closed(&mut self) {
        if let Some(mut state) = self.state.lock_mut() {
            if state.acceptance == PublishNamespaceAcceptance::Pending {
                state.acceptance = PublishNamespaceAcceptance::ResponseStreamClosed;
            }
        }
    }

    pub fn recv_subscribe(&mut self, subscriber: Subscribed) -> Result<(), ServeError> {
        let mut state = self.state.lock_mut().ok_or(ServeError::Done)?;
        state.subscribers.push_back(subscriber);

        Ok(())
    }

    pub fn recv_track_status_requested(
        &mut self,
        track_status_requested: TrackStatusRequested,
    ) -> Result<(), ServeError> {
        let mut state = self.state.lock_mut().ok_or(ServeError::Done)?;
        state
            .track_statuses_requested
            .push_back(track_status_requested);
        Ok(())
    }

    pub fn remove_subscribe(&mut self, request_id: u64) {
        if let Some(mut state) = self.state.lock_mut() {
            state
                .subscribers
                .retain(|subscriber| subscriber.info.id != request_id);
        }
    }

    pub fn remove_track_status(&mut self, request_id: u64) {
        if let Some(mut state) = self.state.lock_mut() {
            state
                .track_statuses_requested
                .retain(|request| request.request_msg.id != request_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_pair() -> (State<PublishNamespaceState>, PublishNamespaceRecv) {
        let (send, recv) = State::default().split();
        (
            send,
            PublishNamespaceRecv {
                state: recv,
                request_id: 0,
                _request_lease: crate::session::test_request_lease(
                    crate::session::RequestDirection::Outbound,
                    crate::session::RequestClass::PublishNamespace,
                ),
            },
        )
    }

    fn rejection() -> PublishNamespaceRejection {
        message::RequestError {
            id: 0,
            error_code: message::RequestErrorCode::Unauthorized as u64,
            retry_interval: 250,
            reason: crate::coding::ReasonPhrase("denied".to_string()),
            redirect: None,
        }
        .into()
    }

    #[tokio::test]
    async fn explicit_request_ok_transitions_pending_to_accepted() {
        let (state, mut recv) = state_pair();
        assert_eq!(
            PublishNamespace::observed_acceptance(&state),
            PublishNamespaceAcceptance::Pending
        );

        recv.recv_ok().unwrap();

        assert_eq!(
            PublishNamespace::observed_acceptance(&state),
            PublishNamespaceAcceptance::Accepted
        );
        PublishNamespace::wait_for_acceptance(&state).await.unwrap();
    }

    #[tokio::test]
    async fn request_error_retains_rejection_details() {
        let (state, recv) = state_pair();
        let rejection = rejection();
        recv.recv_rejected(rejection.clone()).unwrap();

        assert_eq!(
            PublishNamespace::observed_acceptance(&state),
            PublishNamespaceAcceptance::Rejected(rejection.clone())
        );
        assert_eq!(
            PublishNamespace::wait_for_acceptance(&state)
                .await
                .unwrap_err(),
            PublishNamespaceAcceptanceError::Rejected(rejection)
        );
    }

    #[tokio::test]
    async fn silent_peer_remains_pending_until_typed_timeout() {
        let (state, _recv) = state_pair();
        assert_eq!(
            PublishNamespace::observed_acceptance(&state),
            PublishNamespaceAcceptance::Pending
        );

        let timeout = Duration::ZERO;
        assert_eq!(
            PublishNamespace::wait_for_acceptance_with_timeout(&state, timeout)
                .await
                .unwrap_err(),
            PublishNamespaceAcceptanceError::TimedOut { timeout }
        );
        assert_eq!(
            PublishNamespace::observed_acceptance(&state),
            PublishNamespaceAcceptance::Pending
        );
    }

    #[tokio::test]
    async fn response_stream_disconnect_before_acceptance_is_not_success() {
        let (state, recv) = state_pair();
        drop(recv);

        assert_eq!(
            PublishNamespace::observed_acceptance(&state),
            PublishNamespaceAcceptance::ResponseStreamClosed
        );
        assert_eq!(
            PublishNamespace::wait_for_acceptance(&state)
                .await
                .unwrap_err(),
            PublishNamespaceAcceptanceError::ResponseStreamClosed
        );
    }

    #[tokio::test]
    async fn response_fin_after_acceptance_preserves_acceptance() {
        let (state, mut recv) = state_pair();
        recv.recv_ok().unwrap();
        recv.recv_response_stream_closed();

        assert_eq!(
            PublishNamespace::observed_acceptance(&state),
            PublishNamespaceAcceptance::Accepted
        );
        PublishNamespace::wait_for_acceptance(&state).await.unwrap();
    }
}
