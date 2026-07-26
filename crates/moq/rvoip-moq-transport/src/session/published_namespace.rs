// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-FileCopyrightText: 2023-2024 Luke Curley and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{ops, sync::Arc};

use crate::coding::{ReasonPhrase, TrackNamespace};
use crate::message::RequestErrorCode;
use crate::watch::State;
use crate::{message, serve::ServeError};

use super::{PublishNamespaceInfo, RequestLease, Subscriber};

// Tracks whether the publisher has cleanly completed this namespace publish.
#[derive(Default)]
struct PublishedNamespaceState {
    done: bool,
}

/// Represents an inbound PUBLISH_NAMESPACE received by a subscriber.
///
/// On drop, rejects an unaccepted namespace with REQUEST_ERROR. Draft-19
/// cancels an accepted namespace by closing/resetting the request stream.
pub struct PublishedNamespace {
    session: Subscriber,
    state: State<PublishedNamespaceState>,
    _request_lease: Arc<RequestLease>,

    pub info: PublishNamespaceInfo,

    ok: bool,
    error: Option<ServeError>,
}

impl PublishedNamespace {
    pub(super) fn new(
        session: Subscriber,
        request_id: u64,
        namespace: TrackNamespace,
        request_lease: Arc<RequestLease>,
    ) -> (PublishedNamespace, PublishedNamespaceRecv) {
        let info = PublishNamespaceInfo {
            request_id,
            namespace,
        };

        let (send, recv) = State::default().split();
        let send = Self {
            session,
            info,
            ok: false,
            error: None,
            state: send,
            _request_lease: request_lease.clone(),
        };
        let recv = PublishedNamespaceRecv {
            state: recv,
            request_id,
            _request_lease: request_lease,
        };

        (send, recv)
    }

    /// Accept the PUBLISH_NAMESPACE by sending REQUEST_OK (draft-16 §9.7).
    pub fn ok(&mut self) -> Result<(), ServeError> {
        if self.ok {
            return Err(ServeError::Duplicate);
        }

        // Draft-16 §6.2: acceptance is signalled with REQUEST_OK, not the
        // legacy PUBLISH_NAMESPACE_OK.
        self.session.send_request_ok(
            "publish_namespace",
            message::RequestOk {
                id: self.info.request_id,
                params: Default::default(),
                track_properties: Default::default(),
            },
        );

        self.ok = true;

        Ok(())
    }

    /// Wait until the peer closes or cancels the namespace request stream.
    pub async fn closed(&self) -> Result<(), ServeError> {
        loop {
            let modified = {
                let state = self.state.lock();
                if state.done {
                    return Ok(());
                }
                state.modified()
            };
            let Some(modified) = modified else {
                return Ok(());
            };

            modified.await;
        }
    }

    /// Reject the PUBLISH_NAMESPACE; the error is sent on drop.
    pub fn close(mut self, err: ServeError) -> Result<(), ServeError> {
        self.error = Some(err);
        Ok(())
    }
}

impl ops::Deref for PublishedNamespace {
    type Target = PublishNamespaceInfo;

    fn deref(&self) -> &PublishNamespaceInfo {
        &self.info
    }
}

impl Drop for PublishedNamespace {
    fn drop(&mut self) {
        self._request_lease.release();
        let err = self.error.clone().unwrap_or(ServeError::Done);

        if self.state.lock().done {
            return;
        }

        if self.ok {
            self.session.cancel_request_stream(
                self.info.request_id,
                super::Session::REQUEST_STREAM_CANCELLED,
            );
            if let Some(mut recv) = self.session.drop_publish_namespace(self.info.request_id) {
                let _ = recv.recv_done();
            }
        } else {
            // Never accepted: send REQUEST_ERROR (draft-16 §9.8).
            self.session.send_request_error(
                "publish_namespace",
                message::RequestError {
                    id: self.info.request_id,
                    error_code: request_error_code(&err),
                    retry_interval: retry_interval(&err),
                    reason: ReasonPhrase(err.to_string()),
                    redirect: None,
                },
            );
        }
    }
}

fn request_error_code(err: &ServeError) -> u64 {
    match err {
        ServeError::Closed(code) => *code,
        _ => RequestErrorCode::Uninterested as u64,
    }
}

fn retry_interval(err: &ServeError) -> u64 {
    match err {
        ServeError::Closed(code) if *code == RequestErrorCode::ExcessiveLoad as u64 => 1_001,
        _ => 0,
    }
}

pub(super) struct PublishedNamespaceRecv {
    state: State<PublishedNamespaceState>,
    /// Request ID of the corresponding PUBLISH_NAMESPACE, used for O(1)
    /// request-stream lifecycle lookup.
    pub request_id: u64,
    _request_lease: Arc<RequestLease>,
}

impl PublishedNamespaceRecv {
    pub fn recv_done(&mut self) -> Result<(), ServeError> {
        if let Some(mut state) = self.state.lock_mut() {
            state.done = true;
        }
        Ok(())
    }

    /// Wait for the application-facing namespace handle to acknowledge the
    /// completion notification by dropping its producer-side state.
    pub async fn acknowledged(&self) {
        loop {
            let modified = self.state.lock().modified();
            let Some(modified) = modified else { return };
            modified.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recv_done_marks_namespace_done_before_drop() {
        let state = State::<PublishedNamespaceState>::default();
        let (send_state, recv_state) = state.split();
        let recv = PublishedNamespaceRecv {
            state: recv_state,
            request_id: 0,
            _request_lease: crate::session::test_request_lease(
                crate::session::RequestDirection::Inbound,
                crate::session::RequestClass::PublishNamespace,
            ),
        };

        assert!(!send_state.lock().done);

        let mut recv = recv;
        recv.recv_done().unwrap();

        assert!(send_state.lock().done);
        assert!(send_state.lock().modified().is_some());
        drop(recv);
        assert!(send_state.lock().modified().is_none());
    }

    #[test]
    fn excessive_load_rejection_is_retryable() {
        let error = ServeError::Closed(RequestErrorCode::ExcessiveLoad as u64);
        assert_eq!(
            request_error_code(&error),
            RequestErrorCode::ExcessiveLoad as u64
        );
        assert_eq!(retry_interval(&error), 1_001);
        assert_eq!(retry_interval(&ServeError::Cancel), 0);
    }
}
