//! Control plane — placing an inbound WebRTC contact into Amazon Connect.
//!
//! The adapter depends on the [`ConnectContactStarter`] trait rather than the
//! AWS SDK directly. This keeps the crate (and its unit tests) buildable with
//! zero AWS dependencies, lets tests inject a mock, and isolates the AWS
//! `aws-lc-rs` crypto provider behind the `aws-control` feature so it never
//! clashes with the workspace's `ring` rustls provider unless explicitly
//! opted in.

use std::collections::BTreeMap;

use async_trait::async_trait;

use crate::errors::Result;

/// A request to start an inbound WebRTC contact (maps to `StartWebRTCContact`).
#[derive(Clone, Debug)]
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

/// The subset of `StartWebRTCContact`'s `ConnectionData` the media plane needs
/// to join the Amazon Chime SDK meeting.
#[derive(Clone, Debug)]
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

/// Chime meeting media endpoints (subset of the AWS `MediaPlacement` shape).
#[derive(Clone, Debug, Default)]
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

/// Abstraction over the Amazon Connect control plane. Implemented for real by
/// [`AwsConnectStarter`] (feature `aws-control`); tests inject a mock.
#[async_trait]
pub trait ConnectContactStarter: Send + Sync {
    /// Place an inbound WebRTC contact and return the data needed to join the
    /// Chime meeting.
    async fn start_webrtc_contact(&self, request: StartContactRequest) -> Result<ConnectionData>;
}

#[cfg(feature = "aws-control")]
mod aws {
    use super::*;
    use crate::errors::ConnectError;

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
            self.client
                .stop_contact()
                .contact_id(contact_id)
                .instance_id(instance_id)
                .send()
                .await
                .map_err(|e| ConnectError::Control(format!("StopContact: {e:?}")))?;
            Ok(())
        }
    }

    #[async_trait]
    impl ConnectContactStarter for AwsConnectStarter {
        async fn start_webrtc_contact(
            &self,
            request: StartContactRequest,
        ) -> Result<ConnectionData> {
            use aws_sdk_connect::types::ParticipantDetails;

            let mut builder = self
                .client
                .start_web_rtc_contact()
                .instance_id(request.instance_id)
                .contact_flow_id(request.contact_flow_id)
                .participant_details(
                    ParticipantDetails::builder()
                        .display_name(request.display_name)
                        .build()
                        .map_err(|e| ConnectError::Control(e.to_string()))?,
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

            let out = builder
                .send()
                .await
                .map_err(|e| ConnectError::Control(format!("{e:?}")))?;

            map_response(out)
        }
    }

    fn map_response(
        out: aws_sdk_connect::operation::start_web_rtc_contact::StartWebRtcContactOutput,
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

        Ok(ConnectionData {
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
        })
    }
}

#[cfg(feature = "aws-control")]
pub use aws::AwsConnectStarter;
