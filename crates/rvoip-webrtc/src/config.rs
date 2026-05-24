use rvoip_core::capability::CapabilityDescriptor;
use serde::{Deserialize, Serialize};

/// STUN/TURN server entry with optional long-term credentials.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct IceServerConfig {
    pub urls: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
}

impl IceServerConfig {
    pub fn stun(url: impl Into<String>) -> Self {
        Self {
            urls: vec![url.into()],
            username: None,
            credential: None,
        }
    }

    pub fn turn(url: impl Into<String>, username: impl Into<String>, credential: impl Into<String>) -> Self {
        Self {
            urls: vec![url.into()],
            username: Some(username.into()),
            credential: Some(credential.into()),
        }
    }
}

/// ICE / media configuration shared by peer connections and the adapter.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WebRtcConfig {
    /// UDP bind address passed to `PeerConnectionBuilder::with_udp_addrs`.
    /// Use `"0.0.0.0:0"` or `"127.0.0.1:0"` for ephemeral ports.
    pub udp_bind: String,

    /// STUN/TURN servers (username/credential for TURN relay).
    #[serde(default)]
    pub ice_servers: Vec<IceServerConfig>,

    /// Maximum time to wait for ICE gathering to complete (full SDP, no trickle).
    #[serde(default = "default_gather_timeout_secs")]
    pub gather_timeout_secs: u64,

    /// Default capabilities advertised by [`crate::adapter::WebRtcAdapter`].
    #[serde(default = "default_capabilities")]
    pub capabilities: CapabilityDescriptor,
}

fn default_gather_timeout_secs() -> u64 {
    5
}

fn default_capabilities() -> CapabilityDescriptor {
    crate::sdp::capability::default_webrtc_capabilities()
}

impl Default for WebRtcConfig {
    fn default() -> Self {
        Self {
            udp_bind: "0.0.0.0:0".into(),
            ice_servers: vec![IceServerConfig::stun("stun:stun.l.google.com:19302")],
            gather_timeout_secs: default_gather_timeout_secs(),
            capabilities: default_capabilities(),
        }
    }
}

impl WebRtcConfig {
    pub fn loopback() -> Self {
        Self {
            udp_bind: "127.0.0.1:0".into(),
            ice_servers: vec![],
            gather_timeout_secs: 5,
            capabilities: crate::sdp::capability::default_webrtc_capabilities(),
        }
    }

    /// Configure a TURN relay (external server — this crate does not host TURN).
    pub fn with_turn(
        mut self,
        url: impl Into<String>,
        username: impl Into<String>,
        credential: impl Into<String>,
    ) -> Self {
        self.ice_servers
            .push(IceServerConfig::turn(url, username, credential));
        self
    }
}
