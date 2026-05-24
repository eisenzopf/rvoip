//! ICE capability surface and local candidate bookkeeping.

use std::sync::Arc;

use parking_lot::Mutex;

/// Features intentionally supported or deferred in this crate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WebRtcFeatureSupport {
    /// Trickle ICE over signaling (UCTP `connection.ice-candidate` / WS `ice-candidate`).
    pub trickle_ice_signaling: bool,
    /// Simulcast / SVC encodings on a single video sender.
    pub simulcast: bool,
    /// Hosted TURN relay server inside rvoip-webrtc.
    pub turn_relay_server: bool,
}

impl Default for WebRtcFeatureSupport {
    fn default() -> Self {
        Self {
            trickle_ice_signaling: false,
            simulcast: false,
            turn_relay_server: false,
        }
    }
}

/// Shared buffer for ICE candidates gathered during full SDP exchange.
#[derive(Clone, Default)]
pub struct IceCandidateLog {
    inner: Arc<Mutex<Vec<String>>>,
}

impl IceCandidateLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&self, summary: impl Into<String>) {
        self.inner.lock().push(summary.into());
    }

    pub fn candidates(&self) -> Vec<String> {
        self.inner.lock().clone()
    }

    pub fn len(&self) -> usize {
        self.inner.lock().len()
    }
}
