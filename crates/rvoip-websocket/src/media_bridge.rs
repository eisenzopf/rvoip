//! `WebRtcMediaBridge` — co-located webrtc-rs `RTCPeerConnection` per
//! UCTP Connection.
//!
//! # Status
//!
//! **Public surface only in v0.** The signaling path (WS text frames
//! carrying UCTP envelopes) is fully implemented and tested in
//! `crate::{server, client, adapter}`. The media plane (SDP exchange via
//! `substrate_setup`, ICE/DTLS negotiation, MediaFrame ↔
//! `TrackLocalStaticRTP` bridging) is **stubbed**: the public surface
//! compiles and integrates with the adapter, but the actual
//! `RTCPeerConnection` lifecycle is deferred.
//!
//! # Why deferred
//!
//! The only currently-published `webrtc` crate is `0.20.0-alpha.1` —
//! a pre-release with a redesigned event-handler API
//! (`PeerConnectionEventHandler`, `PeerConnectionBuilder`,
//! `PeerConnection` trait) that's expected to change before 1.0. Writing
//! the bridge against this surface today guarantees a rewrite when the
//! crate stabilizes. Decision: ship the signaling path (the v0 spike's
//! reach goal — "fallback for older browsers and constrained
//! networks" per CONVERSATION_PROTOCOL.md §4.3) and defer the media
//! plane behind a documented stub.
//!
//! # Integration plan when webrtc-rs stabilizes
//!
//! 1. `WebRtcMediaBridge::new(config)` constructs a `PeerConnection` via
//!    `PeerConnectionBuilder::new().with_configuration(rtc_config).build().await`.
//! 2. `add_local_track(stream_id, codec)` creates a `TrackLocalStaticRTP`
//!    and registers it via `pc.add_track(track).await`.
//! 3. `create_offer()` calls `pc.create_offer(None).await` →
//!    `set_local_description(offer)` → wait for ICE gathering via the
//!    `PeerConnectionEventHandler::on_ice_candidate` flow → return
//!    `local_description().await.unwrap().sdp` wrapped in
//!    [`WebRtcSubstrateSetup`].
//! 4. `set_remote_description(setup)` decodes the peer's SDP and calls
//!    `pc.set_remote_description(...)`.
//! 5. The outbound media pump drains the matching `MediaStream`'s
//!    `frames_out_rx` and calls `track.write_rtp(packet)` (parsing the
//!    `MediaFrame.payload` as `rtc::shared::rtp::Packet`).
//! 6. The `on_track` event handler spawns an inbound pump that reads
//!    RTP from `TrackRemote::read_rtp` and pushes `MediaFrame`s into
//!    the matching `inbound_tx`.

use crate::errors::{Result, UctpWsError};
use rvoip_uctp::payloads::connection::WebRtcSubstrateSetup;

/// Per-Connection media bridge. v0: stub. v0.x: backed by a real
/// `webrtc::peer_connection::PeerConnection`.
pub struct WebRtcMediaBridge {
    /// Whether this bridge owns the offer (outbound) or the answer
    /// (inbound). Stored so the integration knows whether to call
    /// `create_offer()` or `create_answer()` when the webrtc-rs
    /// integration lands.
    role: BridgeRole,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BridgeRole {
    Offerer,
    Answerer,
}

impl WebRtcMediaBridge {
    pub fn new_offerer() -> Self {
        Self { role: BridgeRole::Offerer }
    }

    pub fn new_answerer() -> Self {
        Self { role: BridgeRole::Answerer }
    }

    pub fn role(&self) -> BridgeRole {
        self.role
    }

    /// Produce the local SDP wrapped in [`WebRtcSubstrateSetup`].
    ///
    /// v0 stub: returns a placeholder SDP that contains the expected
    /// attributes (ICE credentials, fingerprint, candidates) but is not
    /// a valid offer. v0.x replaces with real `pc.create_offer().await`
    /// + `pc.set_local_description().await` + ICE gathering completion.
    pub async fn local_substrate_setup(&self) -> Result<WebRtcSubstrateSetup> {
        Err(UctpWsError::WebRtc(
            "WebRtcMediaBridge::local_substrate_setup — v0.x integration pending webrtc-rs stable release"
                .into(),
        ))
    }

    /// Apply the peer's SDP from `substrate_setup`.
    ///
    /// v0 stub. v0.x: `pc.set_remote_description(parsed_sdp).await`.
    pub async fn set_remote_substrate_setup(
        &self,
        _setup: WebRtcSubstrateSetup,
    ) -> Result<()> {
        Err(UctpWsError::WebRtc(
            "WebRtcMediaBridge::set_remote_substrate_setup — v0.x integration pending webrtc-rs stable release"
                .into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_role_construction() {
        assert_eq!(WebRtcMediaBridge::new_offerer().role(), BridgeRole::Offerer);
        assert_eq!(WebRtcMediaBridge::new_answerer().role(), BridgeRole::Answerer);
    }

    #[tokio::test]
    async fn stub_methods_return_documented_error() {
        let bridge = WebRtcMediaBridge::new_offerer();
        let err = bridge.local_substrate_setup().await.unwrap_err();
        assert!(format!("{}", err).contains("v0.x"));
    }
}
