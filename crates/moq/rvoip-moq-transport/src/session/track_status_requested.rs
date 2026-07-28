// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::sync::Arc;

use super::{Publisher, RequestLease, SessionError};
use crate::coding::{KeyValuePairs, ReasonPhrase};
use crate::message;
use crate::message::RequestOk;
use crate::serve;

pub struct TrackStatusRequested {
    publisher: Publisher,
    pub request_msg: message::TrackStatus,
    _request_lease: Arc<RequestLease>,
    responded: bool,
}

impl TrackStatusRequested {
    pub(super) fn new(
        publisher: Publisher,
        request_msg: message::TrackStatus,
        request_lease: Arc<RequestLease>,
    ) -> Self {
        Self {
            publisher,
            request_msg,
            _request_lease: request_lease,
            responded: false,
        }
    }

    /// Reject the TRACK_STATUS request with REQUEST_ERROR (draft-16 §9.8).
    pub fn respond_error(
        &mut self,
        error_code: u64,
        error_message: &str,
    ) -> Result<(), SessionError> {
        self.respond_error_with_retry(error_code, 0, error_message)
    }

    /// Reject a TRACK_STATUS request and advertise when a retry is allowed.
    pub fn respond_error_with_retry(
        &mut self,
        error_code: u64,
        retry_interval: u64,
        error_message: &str,
    ) -> Result<(), SessionError> {
        self.publisher.send_request_error(
            "track_status",
            message::RequestError {
                id: self.request_msg.id,
                error_code,
                retry_interval,
                reason: ReasonPhrase(error_message.to_string()),
                redirect: None,
            },
        );
        self.responded = true;
        self._request_lease.release();
        Ok(())
    }

    /// Accept the TRACK_STATUS request with REQUEST_OK (draft-16 §9.7).
    ///
    /// The response includes LARGEST_OBJECT when objects have been published.
    /// No Track Alias is included — draft-16 §9.19 does not use one for
    /// TRACK_STATUS responses.
    pub fn respond_ok(mut self, track: &serve::TrackReader) -> Result<(), SessionError> {
        let mut params = KeyValuePairs::default();

        if let Some(largest) = track.largest_location() {
            params
                .set_largest_object(largest)
                .map_err(|_| SessionError::Internal)?;
        }

        self.publisher.send_request_ok(
            "track_status",
            RequestOk {
                id: self.request_msg.id,
                params,
                track_properties: Default::default(),
            },
        );
        self.responded = true;
        self._request_lease.release();

        Ok(())
    }
}

impl Drop for TrackStatusRequested {
    fn drop(&mut self) {
        self._request_lease.release();
        if self.responded {
            return;
        }
        self.publisher.send_request_error(
            "track_status",
            message::RequestError {
                id: self.request_msg.id,
                error_code: message::RequestErrorCode::Uninterested as u64,
                retry_interval: 0,
                reason: ReasonPhrase("track status request dropped".to_string()),
                redirect: None,
            },
        );
        self.responded = true;
    }
}
