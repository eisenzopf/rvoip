// SPDX-FileCopyrightText: 2026 Cloudflare Inc.
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Publisher-side handle for a draft-19 `PUBLISH` request.

use std::ops;

use crate::{
    coding::{Location, TrackName, TrackNamespace},
    serve::{ServeError, TrackReader},
};

use super::{SessionError, Subscribed};

#[derive(Clone, Debug)]
pub struct PublishedInfo {
    pub id: u64,
    pub track_namespace: TrackNamespace,
    pub track_name: TrackName,
    pub track_alias: u64,
    pub largest_location: Option<Location>,
}

/// An outbound publisher-initiated subscription.
///
/// Reverse-direction `REQUEST_UPDATE` messages from the subscriber dynamically
/// pause and resume delivery through the shared subscription state.
#[must_use = "serve or drop to finish the PUBLISH request stream"]
pub struct Published {
    subscription: Subscribed,
    track: Option<TrackReader>,
    pub info: PublishedInfo,
}

impl Published {
    pub(super) fn new(subscription: Subscribed, track: TrackReader, info: PublishedInfo) -> Self {
        Self {
            subscription,
            track: Some(track),
            info,
        }
    }

    /// Wait for draft-19 `REQUEST_OK` (`PUBLISH_OK`).
    pub async fn ok(&self) -> Result<(), ServeError> {
        self.subscription.publish_ok().await
    }

    /// Serve the track and send `PUBLISH_DONE` before closing the request stream.
    pub async fn serve(mut self) -> Result<(), SessionError> {
        let track = self.track.take().ok_or(SessionError::Internal)?;
        self.subscription.serve_published(track).await
    }

    pub async fn closed(&self) -> Result<(), ServeError> {
        self.subscription.closed().await
    }

    pub fn close(self, err: ServeError) -> Result<(), ServeError> {
        self.subscription.close(err)
    }

    /// Cancel the request stream with RESET_STREAM and STOP_SENDING.
    pub fn cancel(mut self) {
        self.subscription.cancel_request_stream();
    }
}

impl ops::Deref for Published {
    type Target = PublishedInfo;

    fn deref(&self) -> &Self::Target {
        &self.info
    }
}
