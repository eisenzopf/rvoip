//! Loopback offer/answer using `WebRtcClient` (requires `--features client`).

use std::sync::Arc;
use std::time::Duration;

use rvoip_webrtc::client::{
    Answer, CallTarget, IceCandidate, Offer, SessionHandle, SessionMedium, Signaler, WebRtcClient,
};
use rvoip_webrtc::peer::{PeerRole, RvoipPeerConnection};
use rvoip_webrtc::WebRtcConfig;

struct LoopbackSignaler {
    answerer: Arc<RvoipPeerConnection>,
}

#[async_trait::async_trait]
impl Signaler for LoopbackSignaler {
    async fn send_offer(&self, offer: &Offer) -> rvoip_webrtc::Result<Answer> {
        let answer_sdp = self.answerer.accept_offer_and_gather(&offer.0).await?;
        Ok(Answer(answer_sdp))
    }

    async fn send_answer(&self, _answer: &Answer) -> rvoip_webrtc::Result<()> {
        Ok(())
    }

    async fn send_ice(&self, _candidate: &IceCandidate) -> rvoip_webrtc::Result<()> {
        Ok(())
    }
}

#[tokio::main]
async fn main() -> rvoip_webrtc::Result<()> {
    let config = WebRtcConfig::loopback();
    let client =
        WebRtcClient::connect(config.clone(), "loopback://in-process").await?;
    let answerer = RvoipPeerConnection::new(&config, PeerRole::Answerer).await?;

    let signaler = LoopbackSignaler { answerer };
    let session: SessionHandle = client
        .call(
            &signaler,
            CallTarget::Uri("loopback".into()),
            SessionMedium::Audio,
        )
        .await?;
    session
        .wait_connected(Duration::from_secs(10))
        .await?;
    println!(
        "loopback call established (session {})",
        session.session_id()
    );
    Ok(())
}
