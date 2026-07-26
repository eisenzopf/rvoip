// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{ops, sync::Arc};

use crate::{
    coding::{ReasonPhrase, TrackNamespacePrefix},
    message,
    serve::ServeError,
    watch::State,
};

use super::{Publisher, RequestLease, Session, SessionError};

#[derive(Default)]
struct SubscribedNamespaceState {
    closed: bool,
}

/// Information retained for an inbound `SUBSCRIBE_NAMESPACE` request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubscribedNamespaceInfo {
    pub request_id: u64,
    pub prefix: TrackNamespacePrefix,
}

/// A long-lived inbound `SUBSCRIBE_NAMESPACE` request.
///
/// The application accepts the request, emits zero or more `NAMESPACE` /
/// `NAMESPACE_DONE` updates, and keeps this handle alive until the peer closes
/// the request stream. Dropping an accepted handle cancels the request stream;
/// dropping an unaccepted handle sends `REQUEST_ERROR`.
#[must_use = "keep the SUBSCRIBE_NAMESPACE request stream alive"]
pub struct SubscribedNamespace {
    publisher: Publisher,
    state: State<SubscribedNamespaceState>,
    _request_lease: Arc<RequestLease>,

    pub info: SubscribedNamespaceInfo,

    accepted: bool,
    error: Option<ServeError>,
}

impl SubscribedNamespace {
    pub(super) fn new(
        publisher: Publisher,
        request: message::SubscribeNamespace,
        request_lease: Arc<RequestLease>,
    ) -> (Self, SubscribedNamespaceRecv) {
        let info = SubscribedNamespaceInfo {
            request_id: request.id,
            prefix: request.track_namespace_prefix,
        };
        let (send, recv) = State::default().split();
        (
            Self {
                publisher,
                state: send,
                _request_lease: request_lease.clone(),
                info,
                accepted: false,
                error: None,
            },
            SubscribedNamespaceRecv {
                state: recv,
                _request_lease: request_lease,
            },
        )
    }

    /// Accept this request with `REQUEST_OK`.
    pub fn ok(&mut self) -> Result<(), SessionError> {
        if self.accepted {
            return Err(SessionError::Serve(ServeError::Duplicate));
        }
        // Route acceptance directly to the request stream. NAMESPACE updates
        // use the same channel, which preserves the required OK-before-update
        // ordering and prevents the shared outgoing queue from being raced.
        self.publisher.send_associated_message(
            self.info.request_id,
            message::RequestOk {
                id: self.info.request_id,
                params: Default::default(),
                track_properties: Default::default(),
            }
            .into(),
        )?;
        self.accepted = true;
        Ok(())
    }

    /// Announce a namespace suffix on this request's response stream.
    pub fn namespace(&mut self, suffix: TrackNamespacePrefix) -> Result<(), SessionError> {
        self.ensure_accepted()?;
        self.publisher.send_associated_message(
            self.info.request_id,
            message::Namespace {
                track_namespace_suffix: suffix,
            }
            .into(),
        )
    }

    /// Withdraw a previously announced namespace suffix.
    pub fn namespace_done(&mut self, suffix: TrackNamespacePrefix) -> Result<(), SessionError> {
        self.ensure_accepted()?;
        self.publisher.send_associated_message(
            self.info.request_id,
            message::NamespaceDone {
                track_namespace_suffix: suffix,
            }
            .into(),
        )
    }

    /// Wait until the peer closes or cancels the request stream.
    pub async fn closed(&self) {
        loop {
            let modified = {
                let state = self.state.lock();
                if state.closed {
                    return;
                }
                state.modified()
            };
            let Some(modified) = modified else { return };
            modified.await;
        }
    }

    /// Reject this request. The terminal `REQUEST_ERROR` is sent on drop.
    pub fn close(mut self, error: ServeError) {
        self.error = Some(error);
    }

    fn ensure_accepted(&self) -> Result<(), SessionError> {
        if self.accepted {
            Ok(())
        } else {
            Err(SessionError::ProtocolViolation(
                "NAMESPACE sent before SUBSCRIBE_NAMESPACE was accepted".to_string(),
            ))
        }
    }
}

impl ops::Deref for SubscribedNamespace {
    type Target = SubscribedNamespaceInfo;

    fn deref(&self) -> &Self::Target {
        &self.info
    }
}

impl Drop for SubscribedNamespace {
    fn drop(&mut self) {
        self._request_lease.release();
        if self.state.lock().closed {
            return;
        }

        if self.accepted {
            self.publisher
                .cancel_request_stream(self.info.request_id, Session::REQUEST_STREAM_CANCELLED);
        } else {
            let error = self.error.clone().unwrap_or(ServeError::Done);
            self.publisher.send_request_error(
                "subscribe_namespace",
                message::RequestError {
                    id: self.info.request_id,
                    error_code: request_error_code(&error),
                    retry_interval: retry_interval(&error),
                    reason: ReasonPhrase(error.to_string()),
                    redirect: None,
                },
            );
        }
    }
}

fn request_error_code(error: &ServeError) -> u64 {
    match error {
        ServeError::Closed(code) => *code,
        _ => message::RequestErrorCode::Uninterested as u64,
    }
}

fn retry_interval(error: &ServeError) -> u64 {
    match error {
        ServeError::Closed(code) if *code == message::RequestErrorCode::ExcessiveLoad as u64 => {
            1_001
        }
        _ => 0,
    }
}

pub(super) struct SubscribedNamespaceRecv {
    state: State<SubscribedNamespaceState>,
    _request_lease: Arc<RequestLease>,
}

impl SubscribedNamespaceRecv {
    pub fn recv_closed(&mut self) {
        if let Some(mut state) = self.state.lock_mut() {
            state.closed = true;
        }
    }
}
