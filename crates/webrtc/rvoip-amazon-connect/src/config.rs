//! Configuration for the Amazon Connect adapter.

use std::time::Duration;

use crate::mapping::AttributeMapping;

/// Static configuration for an [`crate::AmazonConnectAdapter`].
///
/// The AWS instance/flow identifiers and region drive the
/// `StartWebRTCContact` control-plane call; the timeouts and
/// [`AttributeMapping`] govern the per-contact behaviour.
#[derive(Clone, Debug)]
pub struct ConnectConfig {
    /// Amazon Connect instance id (the UUID in the instance ARN).
    pub instance_id: String,
    /// Contact-flow id to run for inbound WebRTC contacts (drives the screen
    /// pop / routing).
    pub contact_flow_id: String,
    /// AWS region of the Connect instance (e.g. `us-west-2`). When `None`, the
    /// region is resolved from the standard AWS environment / profile chain.
    pub region: Option<String>,
    /// Default display name shown to the agent when the inbound leg does not
    /// supply one.
    pub default_display_name: String,
    /// How a SIP custom-header set is translated into Connect contact
    /// attributes (the screen-pop channel).
    pub attribute_mapping: AttributeMapping,
    /// Max time to wait for the Chime signaling handshake (JOIN→JOIN_ACK and
    /// SUBSCRIBE→SUBSCRIBE_ACK).
    pub signaling_timeout: Duration,
    /// Max time to wait for the DTLS/ICE peer connection to reach `Connected`.
    pub media_connect_timeout: Duration,
    /// Interval between Chime `PING_PONG` keepalive frames.
    pub keepalive_interval: Duration,
    /// Reap routes whose peer connection has been `Failed` for at least this
    /// long. Zero disables the reaper.
    pub session_idle_ttl: Duration,
}

impl Default for ConnectConfig {
    fn default() -> Self {
        Self {
            instance_id: String::new(),
            contact_flow_id: String::new(),
            region: None,
            default_display_name: "rvoip".to_string(),
            attribute_mapping: AttributeMapping::default(),
            signaling_timeout: Duration::from_secs(15),
            media_connect_timeout: Duration::from_secs(30),
            keepalive_interval: Duration::from_secs(10),
            session_idle_ttl: Duration::from_secs(120),
        }
    }
}

impl ConnectConfig {
    /// Construct with the required AWS identifiers; everything else takes the
    /// defaults above.
    pub fn new(instance_id: impl Into<String>, contact_flow_id: impl Into<String>) -> Self {
        Self {
            instance_id: instance_id.into(),
            contact_flow_id: contact_flow_id.into(),
            ..Default::default()
        }
    }

    /// Set the AWS region explicitly (builder-style).
    pub fn with_region(mut self, region: impl Into<String>) -> Self {
        self.region = Some(region.into());
        self
    }

    /// Replace the SIP-header → attribute mapping (builder-style).
    pub fn with_attribute_mapping(mut self, mapping: AttributeMapping) -> Self {
        self.attribute_mapping = mapping;
        self
    }
}
