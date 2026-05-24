use rvoip_core::capability::CapabilityDescriptor;
use serde::{Deserialize, Serialize};

/// ICE / media configuration shared by peer connections and the adapter.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WebRtcConfig {
    /// UDP bind address passed to `PeerConnectionBuilder::with_udp_addrs`.
    /// Use `"0.0.0.0:0"` or `"127.0.0.1:0"` for ephemeral ports.
    pub udp_bind: String,

    /// STUN/TURN URLs (e.g. `stun:stun.l.google.com:19302`).
    #[serde(default)]
    pub ice_servers: Vec<String>,

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
            ice_servers: vec!["stun:stun.l.google.com:19302".into()],
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
}
