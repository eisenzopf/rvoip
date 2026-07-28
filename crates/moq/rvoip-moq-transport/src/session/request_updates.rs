// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::message::Message;

use super::{RequestClass, SessionError};

/// The request that owns a bidirectional request stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RequestKind {
    Subscribe,
    Publish,
    Fetch,
    PublishNamespace,
    SubscribeNamespace,
    SubscribeTracks,
    TrackStatus,
}

impl RequestKind {
    pub(super) fn request_class(self) -> Option<RequestClass> {
        match self {
            Self::Subscribe => Some(RequestClass::Subscribe),
            Self::Publish => Some(RequestClass::Publish),
            Self::Fetch => Some(RequestClass::Fetch),
            Self::PublishNamespace => Some(RequestClass::PublishNamespace),
            Self::TrackStatus => Some(RequestClass::TrackStatus),
            // Namespace discovery shares the inbound subscription budget. A
            // separate class can be introduced if operators need independent
            // limits, but it must never bypass logical request admission.
            Self::SubscribeNamespace => Some(RequestClass::Subscribe),
            Self::SubscribeTracks => None,
        }
    }

    pub(super) fn from_first_message(message: &Message) -> Result<Self, SessionError> {
        match message {
            Message::Subscribe(_) => Ok(Self::Subscribe),
            Message::Publish(_) => Ok(Self::Publish),
            Message::Fetch(_) => Ok(Self::Fetch),
            Message::PublishNamespace(_) => Ok(Self::PublishNamespace),
            Message::SubscribeNamespace(_) => Ok(Self::SubscribeNamespace),
            Message::SubscribeTracks(_) => Ok(Self::SubscribeTracks),
            Message::TrackStatus(_) => Ok(Self::TrackStatus),
            Message::RequestUpdate(_) => Err(SessionError::ProtocolViolation(
                "REQUEST_UPDATE cannot be the first message on a request stream".to_string(),
            )),
            other => Err(SessionError::ProtocolViolation(format!(
                "{} cannot be the first message on a request stream",
                other.name()
            ))),
        }
    }

    pub(super) fn accepts_request_updates(self) -> bool {
        // The subscriber, not the requester/publisher, updates a PUBLISH.
        // Reverse-direction update decoding is a later PUBLISH tranche.
        !matches!(self, Self::TrackStatus | Self::Publish)
    }

    pub(super) fn is_publisher_message(self) -> bool {
        matches!(self, Self::Publish | Self::PublishNamespace)
    }

    pub(super) fn is_namespace_scoped(self) -> bool {
        matches!(
            self,
            Self::PublishNamespace | Self::SubscribeNamespace | Self::SubscribeTracks
        )
    }
}

/// Per-stream accounting for unacknowledged REQUEST_UPDATE messages.
///
/// The receiver acquires one credit when it accepts an update from the wire
/// and releases that credit only after REQUEST_OK or REQUEST_ERROR is written
/// on the response direction. A limit of zero is unlimited.
#[derive(Debug)]
pub(super) struct RequestUpdateCredits {
    limit: u64,
    outstanding: u64,
}

impl RequestUpdateCredits {
    pub(super) fn new(limit: u64) -> Self {
        Self {
            limit,
            outstanding: 0,
        }
    }

    pub(super) fn receive(&mut self) -> Result<(), SessionError> {
        if self.limit != 0 && self.outstanding >= self.limit {
            return Err(SessionError::TooManyRequestUpdates);
        }
        self.outstanding = self.outstanding.saturating_add(1);
        Ok(())
    }

    pub(super) fn respond(&mut self) {
        self.outstanding = self.outstanding.saturating_sub(1);
    }

    #[cfg(test)]
    fn outstanding(&self) -> u64 {
        self.outstanding
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coding::{KeyValuePairs, TrackName, TrackNamespace};
    use crate::message::{RequestUpdate, TrackStatus};

    #[test]
    fn request_update_cannot_open_a_stream() {
        let first = Message::RequestUpdate(RequestUpdate {
            id: 0,
            params: KeyValuePairs::default(),
        });
        assert!(matches!(
            RequestKind::from_first_message(&first),
            Err(SessionError::ProtocolViolation(_))
        ));
    }

    #[test]
    fn track_status_cannot_be_updated() {
        let first = Message::TrackStatus(TrackStatus {
            id: 0,
            track_namespace: TrackNamespace::from_utf8_path("live"),
            track_name: TrackName::from("audio"),
            params: KeyValuePairs::default(),
        });
        let kind = RequestKind::from_first_message(&first).unwrap();
        assert!(!kind.accepts_request_updates());
    }

    #[test]
    fn publish_requester_cannot_send_request_update() {
        let first = Message::Publish(crate::message::Publish {
            id: 0,
            track_namespace: TrackNamespace::from_utf8_path("live"),
            track_name: TrackName::from("audio"),
            track_alias: 0,
            params: KeyValuePairs::default(),
            track_extensions: Default::default(),
        });
        let kind = RequestKind::from_first_message(&first).unwrap();
        assert!(!kind.accepts_request_updates());
    }

    #[test]
    fn finite_credit_limit_rejects_pipelined_overflow() {
        let mut credits = RequestUpdateCredits::new(2);
        credits.receive().unwrap();
        credits.receive().unwrap();
        assert!(matches!(
            credits.receive(),
            Err(SessionError::TooManyRequestUpdates)
        ));
        assert_eq!(credits.outstanding(), 2);

        credits.respond();
        credits.receive().unwrap();
        assert_eq!(credits.outstanding(), 2);
    }

    #[test]
    fn zero_credit_limit_is_unlimited() {
        let mut credits = RequestUpdateCredits::new(0);
        for _ in 0..1_000 {
            credits.receive().unwrap();
        }
        assert_eq!(credits.outstanding(), 1_000);
    }

    #[test]
    fn request_classes_cover_retained_families_and_reserve_fetch() {
        assert_eq!(
            RequestKind::PublishNamespace.request_class(),
            Some(RequestClass::PublishNamespace)
        );
        assert_eq!(
            RequestKind::Subscribe.request_class(),
            Some(RequestClass::Subscribe)
        );
        assert_eq!(
            RequestKind::Publish.request_class(),
            Some(RequestClass::Publish)
        );
        assert_eq!(
            RequestKind::TrackStatus.request_class(),
            Some(RequestClass::TrackStatus)
        );
        assert_eq!(
            RequestKind::Fetch.request_class(),
            Some(RequestClass::Fetch)
        );
        assert_eq!(
            RequestKind::SubscribeNamespace.request_class(),
            Some(RequestClass::Subscribe)
        );
        assert_eq!(RequestKind::SubscribeTracks.request_class(), None);
    }
}
