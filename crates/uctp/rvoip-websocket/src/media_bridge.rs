//! `WebRtcMediaBridge` — co-located WebRTC media per UCTP Connection.
//!
//! With the `media-webrtc` feature, delegates ICE/DTLS-SRTP and RTP bridging
//! to `rvoip_webrtc`. Without it, substrate setup methods return a documented
//! error directing callers to enable the feature.

use crate::errors::{Result, UctpWsError};
use rvoip_uctp::payloads::connection::{IceCandidateInit, WebRtcSubstrateSetup};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BridgeRole {
    Offerer,
    Answerer,
}

// ---------------------------------------------------------------------------
// Stub (default — no `media-webrtc` feature)
// ---------------------------------------------------------------------------

#[cfg(not(feature = "media-webrtc"))]
pub struct WebRtcMediaBridge {
    role: BridgeRole,
}

#[cfg(not(feature = "media-webrtc"))]
impl WebRtcMediaBridge {
    pub fn new_offerer() -> Self {
        Self {
            role: BridgeRole::Offerer,
        }
    }

    pub fn new_answerer() -> Self {
        Self {
            role: BridgeRole::Answerer,
        }
    }

    pub fn role(&self) -> BridgeRole {
        self.role
    }

    pub async fn local_substrate_setup(&self) -> Result<WebRtcSubstrateSetup> {
        Err(UctpWsError::WebRtc(
            "enable the `media-webrtc` feature on rvoip-websocket".into(),
        ))
    }

    pub async fn set_remote_substrate_setup(&self, _setup: WebRtcSubstrateSetup) -> Result<()> {
        Err(UctpWsError::WebRtc(
            "enable the `media-webrtc` feature on rvoip-websocket".into(),
        ))
    }

    pub async fn next_local_ice_candidate(&self) -> Option<IceCandidateInit> {
        None
    }

    pub async fn add_remote_ice_candidate(&self, _init: IceCandidateInit) -> Result<()> {
        Err(UctpWsError::WebRtc(
            "enable the `media-webrtc` feature on rvoip-websocket".into(),
        ))
    }
}

// ---------------------------------------------------------------------------
// Real bridge (`media-webrtc` feature)
// ---------------------------------------------------------------------------

#[cfg(feature = "media-webrtc")]
mod bridge {
    use std::sync::Arc;
    use std::time::Duration;

    use parking_lot::Mutex;
    use rvoip_core::capability::CodecInfo;
    use rvoip_core::ids::StreamId;
    use rvoip_core::stream::MediaStream;
    use rvoip_uctp::payloads::connection::{IceCandidateInit, WebRtcSubstrateSetup};
    use rvoip_webrtc::config::WebRtcConfig;
    use rvoip_webrtc::media::{from_tracks, WebRtcMediaStream};
    use rvoip_webrtc::peer::{PeerRole, RvoipPeerConnection};
    use rvoip_webrtc::sdp::default_webrtc_capabilities;
    use rvoip_webrtc::RTCIceCandidateInit;

    use super::{BridgeRole, Result, UctpWsError};

    pub struct WebRtcMediaBridge {
        peer: Arc<RvoipPeerConnection>,
        role: BridgeRole,
        stream_id: StreamId,
        codec: CodecInfo,
        media: Mutex<Option<Arc<WebRtcMediaStream>>>,
        remote_offer_applied: Mutex<bool>,
    }

    impl WebRtcMediaBridge {
        pub async fn new_offerer() -> Result<Self> {
            Self::with_config(WebRtcConfig::loopback(), BridgeRole::Offerer).await
        }

        pub async fn new_answerer() -> Result<Self> {
            Self::with_config(WebRtcConfig::loopback(), BridgeRole::Answerer).await
        }

        pub async fn with_config(config: WebRtcConfig, role: BridgeRole) -> Result<Self> {
            let peer_role = match role {
                BridgeRole::Offerer => PeerRole::Offerer,
                BridgeRole::Answerer => PeerRole::Answerer,
            };
            let peer = RvoipPeerConnection::new(&config, peer_role).await?;
            let codec = default_webrtc_capabilities()
                .audio_codecs
                .into_iter()
                .next()
                .unwrap_or(CodecInfo {
                    name: "opus".into(),
                    clock_rate_hz: 48000,
                    channels: 2,
                    fmtp: None,
                });

            Ok(Self {
                peer,
                role,
                stream_id: StreamId::new(),
                codec,
                media: Mutex::new(None),
                remote_offer_applied: Mutex::new(false),
            })
        }

        pub fn role(&self) -> BridgeRole {
            self.role
        }

        pub fn peer(&self) -> &Arc<RvoipPeerConnection> {
            &self.peer
        }

        /// Produce local SDP wrapped in [`WebRtcSubstrateSetup`].
        pub async fn local_substrate_setup(&self) -> Result<WebRtcSubstrateSetup> {
            let sdp = match self.role {
                BridgeRole::Offerer => self.peer.create_offer_and_gather().await?,
                BridgeRole::Answerer => {
                    if !*self.remote_offer_applied.lock() {
                        return Err(UctpWsError::WebRtc(
                            "answerer must call set_remote_substrate_setup before local_substrate_setup"
                                .into(),
                        ));
                    }
                    self.peer.create_answer_and_gather().await?
                }
            };

            self.ensure_media_stream().await?;
            Ok(WebRtcSubstrateSetup::new(sdp))
        }

        /// Gap plan §4.2 v1 punch list — drive a mid-call SDP
        /// renegotiation. Returns the new local SDP wrapped in a
        /// `WebRtcSubstrateSetup` ready to ride in a
        /// `connection.update` envelope.
        ///
        /// Offerer side: re-creates an offer (ICE restart semantics
        /// from webrtc-rs). Answerer side: requires the peer to have
        /// sent a new remote offer first; pass it as `remote_sdp` so
        /// the bridge applies it before generating the answer.
        pub async fn renegotiate_codec(
            &self,
            remote_sdp: Option<&str>,
        ) -> Result<WebRtcSubstrateSetup> {
            let sdp = match self.role {
                BridgeRole::Offerer => self.peer.renegotiate_as_offerer().await?,
                BridgeRole::Answerer => {
                    let remote = remote_sdp.ok_or_else(|| {
                        UctpWsError::WebRtc(
                            "answerer renegotiate_codec requires the peer's new offer SDP".into(),
                        )
                    })?;
                    self.peer.renegotiate_as_answerer(remote).await?
                }
            };
            Ok(WebRtcSubstrateSetup::new(sdp))
        }

        /// Apply peer SDP from `substrate_setup`.
        pub async fn set_remote_substrate_setup(&self, setup: WebRtcSubstrateSetup) -> Result<()> {
            if setup.kind != "websocket+webrtc" {
                return Err(UctpWsError::WebRtc(format!(
                    "expected substrate kind websocket+webrtc, got {}",
                    setup.kind
                )));
            }

            match self.role {
                BridgeRole::Offerer => {
                    self.peer.set_remote_answer(&setup.sdp).await?;
                }
                BridgeRole::Answerer => {
                    self.peer.apply_remote_offer(&setup.sdp).await?;
                    *self.remote_offer_applied.lock() = true;
                }
            }
            Ok(())
        }

        /// Gap plan §4.1 — block until the next locally-gathered ICE
        /// candidate is available, return it as an [`IceCandidateInit`]
        /// ready to drop into a `connection.ice-candidate` envelope.
        /// Returns `None` when the bridge is closed or the underlying
        /// peer connection's local-ICE channel closes (e.g. gathering
        /// complete in non-trickle mode).
        pub async fn next_local_ice_candidate(&self) -> Option<IceCandidateInit> {
            let cand = self.peer.recv_local_ice().await?;
            let init = match cand.to_json() {
                Ok(i) => i,
                Err(_) => return None,
            };
            Some(IceCandidateInit {
                candidate: init.candidate,
                sdp_m_line_index: init.sdp_mline_index.unwrap_or(0),
                sdp_mid: init.sdp_mid.unwrap_or_default(),
            })
        }

        /// Gap plan §4.1 — apply an inbound remote ICE candidate. An
        /// empty `candidate` string is the spec's end-of-candidates
        /// marker and is silently dropped (webrtc-rs has no explicit
        /// API for it; the gathering state on the remote side has the
        /// information).
        pub async fn add_remote_ice_candidate(&self, init: IceCandidateInit) -> Result<()> {
            if init.is_end_of_candidates() {
                return Ok(());
            }
            let rtc_init = RTCIceCandidateInit {
                candidate: init.candidate,
                sdp_mid: Some(init.sdp_mid),
                sdp_mline_index: Some(init.sdp_m_line_index),
                username_fragment: None,
                url: None,
            };
            self.peer.add_remote_ice_candidate(rtc_init).await?;
            Ok(())
        }

        /// Wait for ICE/DTLS to reach connected, then ensure media pumps are wired.
        pub async fn wait_connected(&self, timeout: Duration) -> Result<()> {
            self.peer.wait_connected(timeout).await?;
            self.ensure_media_stream().await?;
            self.attach_remote_if_ready(Duration::from_secs(2)).await;
            Ok(())
        }

        /// Access the bridged voip-3 media stream (after setup + connect).
        pub fn media_stream(&self) -> Option<Arc<WebRtcMediaStream>> {
            self.media.lock().clone()
        }

        pub async fn close(&self) -> Result<()> {
            // Drop the parking_lot guard before any .await — guards are
            // !Send and would prevent close() from being .spawn()ed.
            let media = self.media.lock().take();
            if let Some(stream) = media {
                stream.close().await.ok();
            }
            self.peer.close().await?;
            Ok(())
        }

        async fn ensure_media_stream(&self) -> Result<()> {
            // Take any potentially-blocking work (track waits) outside the
            // sync mutex; only hold the guard for the actual slot mutation.
            if self.media.lock().is_some() {
                return Ok(());
            }

            let local = self
                .peer
                .local_audio_track()
                .ok_or_else(|| UctpWsError::WebRtc("no local audio track".into()))?;
            let local_ssrc = self
                .peer
                .local_audio_ssrc()
                .ok_or_else(|| UctpWsError::WebRtc("no local audio ssrc".into()))?;

            let remote = self
                .peer
                .wait_remote_track(Duration::from_millis(500))
                .await
                .or(self.peer.try_recv_remote_track().await);

            // PT 111 is the Opus default carried by `default_webrtc_capabilities`
            // and registered in `peer::builder`; aligned with the QUIC/WT
            // adapters' codec_to_pt mapping.
            const OPUS_PT: u8 = 111;

            let mut guard = self.media.lock();
            if guard.is_some() {
                return Ok(());
            }
            *guard = Some(from_tracks(
                self.stream_id.clone(),
                self.codec.clone(),
                local,
                local_ssrc,
                OPUS_PT,
                remote,
            ));
            Ok(())
        }

        async fn attach_remote_if_ready(&self, timeout: Duration) {
            let remote = self
                .peer
                .wait_remote_track(timeout)
                .await
                .or(self.peer.try_recv_remote_track().await);

            let Some(remote) = remote else {
                return;
            };

            if let Some(stream) = self.media.lock().as_ref() {
                stream.attach_remote(remote);
            }
        }
    }
}

#[cfg(feature = "media-webrtc")]
pub use bridge::WebRtcMediaBridge;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_role_construction() {
        #[cfg(not(feature = "media-webrtc"))]
        {
            assert_eq!(WebRtcMediaBridge::new_offerer().role(), BridgeRole::Offerer);
            assert_eq!(
                WebRtcMediaBridge::new_answerer().role(),
                BridgeRole::Answerer
            );
        }
    }

    #[cfg(not(feature = "media-webrtc"))]
    #[tokio::test]
    async fn stub_methods_return_documented_error() {
        let bridge = WebRtcMediaBridge::new_offerer();
        let err = bridge.local_substrate_setup().await.unwrap_err();
        assert!(format!("{}", err).contains("media-webrtc"));
    }
}
