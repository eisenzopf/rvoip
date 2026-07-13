//! Control plane — placing an inbound WebRTC contact into Amazon Connect.
//!
//! The adapter depends on the [`ConnectContactStarter`] trait rather than the
//! AWS SDK directly. This keeps the crate (and its unit tests) buildable with
//! zero AWS dependencies, lets tests inject a mock, and isolates the AWS
//! `aws-lc-rs` crypto provider behind the `aws-control` feature so it never
//! clashes with the workspace's `ring` rustls provider unless explicitly
//! opted in.

use std::collections::BTreeMap;
use std::fmt;

use async_trait::async_trait;
use zeroize::Zeroize;

use crate::errors::Result;

const MAX_CONNECT_RESPONSE_ID_BYTES: usize = 2_048;
const MAX_CONNECT_RESPONSE_REGION_BYTES: usize = 128;
const MAX_CONNECT_RESPONSE_TOKEN_BYTES: usize = 65_536;
const MAX_CONNECT_RESPONSE_URL_BYTES: usize = 8_192;

/// A request to start an inbound WebRTC contact (maps to `StartWebRTCContact`).
#[derive(Clone)]
pub struct StartContactRequest {
    /// Amazon Connect instance id.
    pub instance_id: String,
    /// Contact-flow id to run.
    pub contact_flow_id: String,
    /// Display name shown to the agent in the CCP.
    pub display_name: String,
    /// Contact attributes — the screen-pop channel. These become standard
    /// Connect contact attributes, readable in the flow and via
    /// `contact.getAttributes()`.
    pub attributes: BTreeMap<String, String>,
    /// Optional task description shown in the CCP.
    pub description: Option<String>,
    /// Idempotency token (maps to `ClientToken`). When `None` the SDK
    /// generates one.
    pub client_token: Option<String>,
}

impl StartContactRequest {
    /// Strict validation used by the new typed generic path. Legacy
    /// `originate_contact*` wrappers deliberately keep their historic
    /// empty-attributes and `client_token=None` semantics and do not call it.
    pub fn validate(&self) -> std::result::Result<(), crate::AmazonConnectOriginateContextError> {
        crate::originate::validate_start_contact_request(self)
    }

    /// Best-effort clearing for callers that retain the legacy public request
    /// after a failed operation. The new typed context clears itself on drop.
    pub fn zeroize_sensitive(&mut self) {
        self.instance_id.zeroize();
        self.contact_flow_id.zeroize();
        self.display_name.zeroize();
        if let Some(description) = self.description.as_mut() {
            description.zeroize();
        }
        if let Some(client_token) = self.client_token.as_mut() {
            client_token.zeroize();
        }
        for (mut key, mut value) in std::mem::take(&mut self.attributes) {
            key.zeroize();
            value.zeroize();
        }
    }
}

impl fmt::Debug for StartContactRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StartContactRequest")
            .field("instance_id", &"[redacted]")
            .field("contact_flow_id", &"[redacted]")
            .field("display_name_present", &!self.display_name.is_empty())
            .field("attribute_count", &self.attributes.len())
            .field("description_present", &self.description.is_some())
            .field("client_token_present", &self.client_token.is_some())
            .finish()
    }
}

/// Idempotent request to terminate a previously started Connect contact.
#[derive(Clone, PartialEq, Eq)]
pub struct StopContactRequest {
    /// Amazon Connect instance that owns the contact.
    pub instance_id: String,
    /// Contact id returned by `StartWebRTCContact`.
    pub contact_id: String,
}

impl fmt::Debug for StopContactRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StopContactRequest")
            .field("instance_id", &"[redacted]")
            .field("contact_id", &"[redacted]")
            .finish()
    }
}

/// The subset of `StartWebRTCContact`'s `ConnectionData` the media plane needs
/// to join the Amazon Chime SDK meeting.
#[derive(Clone)]
pub struct ConnectionData {
    /// Connect contact id (one per call).
    pub contact_id: String,
    /// Participant id (stable for the participant's lifetime).
    pub participant_id: String,
    /// Participant token (for `CreateParticipantConnection`, if needed).
    pub participant_token: String,
    /// Chime meeting id.
    pub meeting_id: String,
    /// AWS region the meeting lives in.
    pub media_region: String,
    /// Chime attendee id for our (the customer) participant.
    pub attendee_id: String,
    /// Attendee join token — authenticates the Chime signaling websocket.
    pub join_token: String,
    /// Media endpoints for the meeting.
    pub media_placement: MediaPlacement,
}

impl ConnectionData {
    /// Validate every response field required by the adapter before Chime,
    /// ICE, DTLS, or media work begins.
    pub fn validate(&self) -> Result<()> {
        validate_response_field(&self.contact_id, "ContactId", MAX_CONNECT_RESPONSE_ID_BYTES)?;
        validate_response_field(
            &self.participant_id,
            "ParticipantId",
            MAX_CONNECT_RESPONSE_ID_BYTES,
        )?;
        validate_response_field(
            &self.participant_token,
            "ParticipantToken",
            MAX_CONNECT_RESPONSE_TOKEN_BYTES,
        )?;
        validate_response_field(&self.meeting_id, "MeetingId", MAX_CONNECT_RESPONSE_ID_BYTES)?;
        validate_response_field(
            &self.media_region,
            "MediaRegion",
            MAX_CONNECT_RESPONSE_REGION_BYTES,
        )?;
        validate_response_field(
            &self.attendee_id,
            "AttendeeId",
            MAX_CONNECT_RESPONSE_ID_BYTES,
        )?;
        validate_response_field(
            &self.join_token,
            "JoinToken",
            MAX_CONNECT_RESPONSE_TOKEN_BYTES,
        )?;
        self.media_placement.validate()
    }

    pub(crate) fn validate_cleanup_identity(&self) -> Result<()> {
        validate_response_field(&self.contact_id, "ContactId", MAX_CONNECT_RESPONSE_ID_BYTES)
    }

    /// Best-effort clearing once connection data has been copied into its
    /// separately owned route/session resources.
    pub fn zeroize_sensitive(&mut self) {
        self.contact_id.zeroize();
        self.participant_id.zeroize();
        self.participant_token.zeroize();
        self.meeting_id.zeroize();
        self.media_region.zeroize();
        self.attendee_id.zeroize();
        self.join_token.zeroize();
        self.media_placement.zeroize_sensitive();
    }
}

impl fmt::Debug for ConnectionData {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConnectionData")
            .field("contact_id_present", &!self.contact_id.is_empty())
            .field("participant_id_present", &!self.participant_id.is_empty())
            .field(
                "participant_token_present",
                &!self.participant_token.is_empty(),
            )
            .field("meeting_id_present", &!self.meeting_id.is_empty())
            .field("media_region_present", &!self.media_region.is_empty())
            .field("attendee_id_present", &!self.attendee_id.is_empty())
            .field("join_token_present", &!self.join_token.is_empty())
            .field("media_placement", &self.media_placement)
            .finish()
    }
}

/// Chime meeting media endpoints (subset of the AWS `MediaPlacement` shape).
#[derive(Clone, Default)]
pub struct MediaPlacement {
    /// Secure-WebSocket signaling URL (the Chime protobuf signaling endpoint).
    pub signaling_url: String,
    /// Audio host URL (used as `audio_host` in the SUBSCRIBE frame).
    pub audio_host_url: String,
    /// TURN control URL (legacy TURN provisioning; modern joins receive TURN
    /// creds in the JOIN_ACK instead).
    pub turn_control_url: Option<String>,
    /// Audio fallback URL.
    pub audio_fallback_url: Option<String>,
    /// Client-event ingestion URL.
    pub event_ingestion_url: Option<String>,
}

impl MediaPlacement {
    fn validate(&self) -> Result<()> {
        validate_response_field(
            &self.signaling_url,
            "SignalingUrl",
            MAX_CONNECT_RESPONSE_URL_BYTES,
        )?;
        validate_response_field(
            &self.audio_host_url,
            "AudioHostUrl",
            MAX_CONNECT_RESPONSE_URL_BYTES,
        )?;
        validate_optional_response_field(
            self.turn_control_url.as_deref(),
            "TurnControlUrl",
            MAX_CONNECT_RESPONSE_URL_BYTES,
        )?;
        validate_optional_response_field(
            self.audio_fallback_url.as_deref(),
            "AudioFallbackUrl",
            MAX_CONNECT_RESPONSE_URL_BYTES,
        )?;
        validate_optional_response_field(
            self.event_ingestion_url.as_deref(),
            "EventIngestionUrl",
            MAX_CONNECT_RESPONSE_URL_BYTES,
        )
    }

    /// Best-effort clearing of Chime endpoint material.
    pub fn zeroize_sensitive(&mut self) {
        self.signaling_url.zeroize();
        self.audio_host_url.zeroize();
        if let Some(url) = self.turn_control_url.as_mut() {
            url.zeroize();
        }
        if let Some(url) = self.audio_fallback_url.as_mut() {
            url.zeroize();
        }
        if let Some(url) = self.event_ingestion_url.as_mut() {
            url.zeroize();
        }
    }
}

impl fmt::Debug for MediaPlacement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MediaPlacement")
            .field("signaling_url_present", &!self.signaling_url.is_empty())
            .field("audio_host_url_present", &!self.audio_host_url.is_empty())
            .field("turn_control_url_present", &self.turn_control_url.is_some())
            .field(
                "audio_fallback_url_present",
                &self.audio_fallback_url.is_some(),
            )
            .field(
                "event_ingestion_url_present",
                &self.event_ingestion_url.is_some(),
            )
            .finish()
    }
}

fn validate_response_field(value: &str, field: &'static str, max_bytes: usize) -> Result<()> {
    if value.is_empty() || value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(crate::errors::ConnectError::MissingConnectionData(field));
    }
    Ok(())
}

fn validate_optional_response_field(
    value: Option<&str>,
    field: &'static str,
    max_bytes: usize,
) -> Result<()> {
    match value {
        Some(value) => validate_response_field(value, field, max_bytes),
        None => Ok(()),
    }
}

/// Abstraction over the Amazon Connect control plane. Implemented for real by
/// [`AwsConnectStarter`] (feature `aws-control`); tests inject a mock.
#[async_trait]
pub trait ConnectContactStarter: Send + Sync {
    /// Place an inbound WebRTC contact and return the data needed to join the
    /// Chime meeting.
    async fn start_webrtc_contact(&self, request: StartContactRequest) -> Result<ConnectionData>;

    /// Terminate a contact started by this control plane. Implementations must
    /// be idempotent: stopping an already-ended contact is success.
    ///
    /// The default preserves source compatibility for existing custom
    /// starters, but reports the missing capability rather than silently
    /// leaking a started contact.
    async fn stop_contact(&self, _request: StopContactRequest) -> Result<()> {
        Err(crate::errors::ConnectError::Control(
            "contact termination is not implemented by this starter".into(),
        ))
    }
}

#[cfg(feature = "aws-control")]
mod aws {
    use super::*;
    use crate::errors::ConnectError;
    use aws_sdk_connect::error::SdkError;
    use aws_sdk_connect::operation::start_web_rtc_contact::StartWebRTCContactError;
    use aws_sdk_connect::operation::stop_contact::StopContactError;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum StopErrorDisposition {
        AlreadyEnded,
        Retryable,
        Permanent,
    }

    /// Real control-plane implementation backed by `aws-sdk-connect`.
    pub struct AwsConnectStarter {
        client: aws_sdk_connect::Client,
    }

    impl AwsConnectStarter {
        /// Build from an explicit AWS SDK config.
        pub fn new(conf: &aws_config::SdkConfig) -> Self {
            Self {
                client: aws_sdk_connect::Client::new(conf),
            }
        }

        /// Resolve AWS configuration from the standard environment/profile
        /// chain (optionally pinning a region) and build the client.
        pub async fn from_env(region: Option<String>) -> Self {
            let mut loader = aws_config::defaults(aws_config::BehaviorVersion::latest());
            if let Some(region) = region {
                loader = loader.region(aws_config::Region::new(region));
            }
            let conf = loader.load().await;
            Self::new(&conf)
        }

        /// End a contact (Connect `StopContact`). Used to clean up after a
        /// probe/test so repeated runs don't leave contacts ringing agents.
        pub async fn stop_contact(
            &self,
            contact_id: impl Into<String>,
            instance_id: impl Into<String>,
        ) -> Result<()> {
            match self
                .client
                .stop_contact()
                .contact_id(contact_id)
                .instance_id(instance_id)
                .send()
                .await
            {
                Ok(_) => Ok(()),
                Err(error) => match classify_stop_error(&error) {
                    StopErrorDisposition::AlreadyEnded => Ok(()),
                    StopErrorDisposition::Retryable => Err(ConnectError::TransientControl(
                        "typed AWS StopContact transient failure".into(),
                    )),
                    StopErrorDisposition::Permanent => Err(ConnectError::Control(
                        "typed AWS StopContact permanent failure".into(),
                    )),
                },
            }
        }
    }

    #[async_trait]
    impl ConnectContactStarter for AwsConnectStarter {
        async fn start_webrtc_contact(
            &self,
            request: StartContactRequest,
        ) -> Result<ConnectionData> {
            use aws_sdk_connect::types::ParticipantDetails;

            let instance_id_for_cleanup = request.instance_id.clone();

            let mut builder = self
                .client
                .start_web_rtc_contact()
                .instance_id(request.instance_id)
                .contact_flow_id(request.contact_flow_id)
                .participant_details(
                    ParticipantDetails::builder()
                        .display_name(request.display_name)
                        .build()
                        .map_err(|_| {
                            ConnectError::Control(
                                "AWS participant-details construction failed".into(),
                            )
                        })?,
                );

            for (k, v) in request.attributes {
                builder = builder.attributes(k, v);
            }
            if let Some(desc) = request.description {
                builder = builder.description(desc);
            }
            if let Some(token) = request.client_token {
                builder = builder.client_token(token);
            }

            let out = builder.send().await.map_err(classify_start_error)?;

            match map_response(&out) {
                Ok(data) => Ok(data),
                Err(response_error) => {
                    // If AWS returned a usable ContactId with an otherwise
                    // invalid response, the same starter that selected the
                    // account/region owns best-effort compensation.
                    if let Some(contact_id) = out.contact_id().filter(|value| !value.is_empty()) {
                        let _cleanup_result = self
                            .stop_contact(contact_id.to_owned(), instance_id_for_cleanup)
                            .await;
                    }
                    Err(response_error)
                }
            }
        }

        async fn stop_contact(&self, request: StopContactRequest) -> Result<()> {
            self.stop_contact(request.contact_id, request.instance_id)
                .await
        }
    }

    fn classify_start_error(error: SdkError<StartWebRTCContactError>) -> ConnectError {
        match error {
            SdkError::TimeoutError(_)
            | SdkError::DispatchFailure(_)
            | SdkError::ResponseError(_) => ConnectError::TransientControl(
                "typed AWS StartWebRTCContact ambiguous failure".into(),
            ),
            SdkError::ServiceError(service) if service.err().is_internal_service_exception() => {
                ConnectError::TransientControl(
                    "typed AWS StartWebRTCContact service failure".into(),
                )
            }
            SdkError::ConstructionFailure(_) | SdkError::ServiceError(_) => {
                ConnectError::Control("typed AWS StartWebRTCContact permanent failure".into())
            }
            _ => ConnectError::Control("typed AWS StartWebRTCContact failure".into()),
        }
    }

    fn classify_stop_error(error: &SdkError<StopContactError>) -> StopErrorDisposition {
        match error {
            SdkError::ServiceError(service) => classify_stop_service_error(service.err()),
            SdkError::TimeoutError(_)
            | SdkError::DispatchFailure(_)
            | SdkError::ResponseError(_) => StopErrorDisposition::Retryable,
            SdkError::ConstructionFailure(_) => StopErrorDisposition::Permanent,
            _ => StopErrorDisposition::Permanent,
        }
    }

    fn classify_stop_service_error(error: &StopContactError) -> StopErrorDisposition {
        if error.is_contact_not_found_exception() || error.is_resource_not_found_exception() {
            StopErrorDisposition::AlreadyEnded
        } else if error.is_internal_service_exception() {
            StopErrorDisposition::Retryable
        } else {
            StopErrorDisposition::Permanent
        }
    }

    fn map_response(
        out: &aws_sdk_connect::operation::start_web_rtc_contact::StartWebRtcContactOutput,
    ) -> Result<ConnectionData> {
        let contact_id = out.contact_id().unwrap_or_default().to_string();
        let participant_id = out.participant_id().unwrap_or_default().to_string();
        let participant_token = out.participant_token().unwrap_or_default().to_string();

        let conn = out
            .connection_data()
            .ok_or(ConnectError::MissingConnectionData("ConnectionData"))?;
        let attendee = conn
            .attendee()
            .ok_or(ConnectError::MissingConnectionData("Attendee"))?;
        let meeting = conn
            .meeting()
            .ok_or(ConnectError::MissingConnectionData("Meeting"))?;
        let placement = meeting
            .media_placement()
            .ok_or(ConnectError::MissingConnectionData("MediaPlacement"))?;

        let signaling_url = placement
            .signaling_url()
            .ok_or(ConnectError::MissingConnectionData("SignalingUrl"))?
            .to_string();
        let join_token = attendee
            .join_token()
            .ok_or(ConnectError::MissingConnectionData("JoinToken"))?
            .to_string();

        let data = ConnectionData {
            contact_id,
            participant_id,
            participant_token,
            meeting_id: meeting.meeting_id().unwrap_or_default().to_string(),
            media_region: meeting.media_region().unwrap_or_default().to_string(),
            attendee_id: attendee.attendee_id().unwrap_or_default().to_string(),
            join_token,
            media_placement: MediaPlacement {
                signaling_url,
                audio_host_url: placement.audio_host_url().unwrap_or_default().to_string(),
                turn_control_url: placement.turn_control_url().map(str::to_string),
                audio_fallback_url: placement.audio_fallback_url().map(str::to_string),
                event_ingestion_url: placement.event_ingestion_url().map(str::to_string),
            },
        };
        data.validate()?;
        Ok(data)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn stop_service_errors_use_typed_variants_not_rendered_messages() {
            let not_found = StopContactError::ContactNotFoundException(
                aws_sdk_connect::types::error::ContactNotFoundException::builder()
                    .message("arbitrary secret message")
                    .build(),
            );
            assert_eq!(
                classify_stop_service_error(&not_found),
                StopErrorDisposition::AlreadyEnded
            );

            let internal = StopContactError::InternalServiceException(
                aws_sdk_connect::types::error::InternalServiceException::builder()
                    .message("arbitrary secret message")
                    .build(),
            );
            assert_eq!(
                classify_stop_service_error(&internal),
                StopErrorDisposition::Retryable
            );

            let invalid = StopContactError::InvalidRequestException(
                aws_sdk_connect::types::error::InvalidRequestException::builder()
                    .message("already ended but still a permanent typed variant")
                    .build(),
            );
            assert_eq!(
                classify_stop_service_error(&invalid),
                StopErrorDisposition::Permanent
            );
        }

        #[test]
        fn transport_timeout_is_typed_retryable() {
            let timeout: SdkError<StopContactError> = SdkError::timeout_error(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "secret timeout detail",
            ));
            assert_eq!(
                classify_stop_error(&timeout),
                StopErrorDisposition::Retryable
            );
        }
    }
}

#[cfg(feature = "aws-control")]
pub use aws::AwsConnectStarter;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_and_response_diagnostics_are_metadata_only() {
        let request = StartContactRequest {
            instance_id: "instance-secret".into(),
            contact_flow_id: "flow-secret".into(),
            display_name: "display-secret".into(),
            attributes: BTreeMap::from([("key-secret".into(), "value-secret".into())]),
            description: Some("description-secret".into()),
            client_token: Some("token-secret".into()),
        };
        let data = ConnectionData {
            contact_id: "contact-secret".into(),
            participant_id: "participant-secret".into(),
            participant_token: "participant-token-secret".into(),
            meeting_id: "meeting-secret".into(),
            media_region: "region-secret".into(),
            attendee_id: "attendee-secret".into(),
            join_token: "join-token-secret".into(),
            media_placement: MediaPlacement {
                signaling_url: "wss://signal-secret".into(),
                audio_host_url: "audio-secret".into(),
                turn_control_url: Some("turn-secret".into()),
                audio_fallback_url: Some("fallback-secret".into()),
                event_ingestion_url: Some("event-secret".into()),
            },
        };
        let diagnostics = format!("{request:?} {data:?}");
        for secret in [
            "instance-secret",
            "flow-secret",
            "display-secret",
            "key-secret",
            "value-secret",
            "description-secret",
            "token-secret",
            "contact-secret",
            "meeting-secret",
            "join-token-secret",
            "signal-secret",
        ] {
            assert!(!diagnostics.contains(secret), "leaked {secret}");
        }
    }

    #[test]
    fn response_validation_rejects_every_required_field() {
        let valid = || ConnectionData {
            contact_id: "contact".into(),
            participant_id: "participant".into(),
            participant_token: "participant-token".into(),
            meeting_id: "meeting".into(),
            media_region: "us-west-2".into(),
            attendee_id: "attendee".into(),
            join_token: "join-token".into(),
            media_placement: MediaPlacement {
                signaling_url: "wss://signal.example".into(),
                audio_host_url: "audio.example".into(),
                ..Default::default()
            },
        };
        assert!(valid().validate().is_ok());

        let mut fields: Vec<(&'static str, Box<dyn Fn(&mut ConnectionData)>)> = vec![
            ("ContactId", Box::new(|data| data.contact_id.clear())),
            (
                "ParticipantId",
                Box::new(|data| data.participant_id.clear()),
            ),
            (
                "ParticipantToken",
                Box::new(|data| data.participant_token.clear()),
            ),
            ("MeetingId", Box::new(|data| data.meeting_id.clear())),
            ("MediaRegion", Box::new(|data| data.media_region.clear())),
            ("AttendeeId", Box::new(|data| data.attendee_id.clear())),
            ("JoinToken", Box::new(|data| data.join_token.clear())),
            (
                "SignalingUrl",
                Box::new(|data| data.media_placement.signaling_url.clear()),
            ),
            (
                "AudioHostUrl",
                Box::new(|data| data.media_placement.audio_host_url.clear()),
            ),
        ];
        for (field, clear) in fields.drain(..) {
            let mut data = valid();
            clear(&mut data);
            assert!(matches!(
                data.validate(),
                Err(crate::errors::ConnectError::MissingConnectionData(actual)) if actual == field
            ));
        }
    }
}
