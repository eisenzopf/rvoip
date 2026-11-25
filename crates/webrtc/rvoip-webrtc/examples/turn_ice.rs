//! Real TURN/ICE qualification. See docs/TURN_ICE.md at the repository root.
#[path = "../tests/support/coturn_fixture.rs"]
mod local_turn;

use std::{error::Error, sync::Arc, time::Duration};

use rvoip_webrtc::{
    config::IceTransportPolicy,
    media::silent_opus_payload,
    peer::{PeerRole, RvoipPeerConnection},
    WebRtcConfig,
};
use webrtc::media_stream::{track_local::TrackLocal, track_remote::TrackRemoteEvent};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[tokio::main]
async fn main() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let args: Vec<_> = std::env::args().skip(1).collect();
    let mut fixture = None;
    let config = match args.as_slice() {
        [] => {
            let server = local_turn::CoturnFixture::start().await?;
            let mut config = WebRtcConfig::loopback();
            config.ice_servers = vec![server.ice_config()];
            config.ice_transport_policy = IceTransportPolicy::Relay;
            fixture = Some(server);
            config
        }
        [flag] if flag == "--direct" => WebRtcConfig::loopback(),
        [flag, path] if flag == "--config" => {
            let contents = std::fs::read(path).map_err(|_| "cannot read configuration file")?;
            let config: WebRtcConfig = serde_json::from_slice(&contents)
                .map_err(|_| "invalid WebRtcConfig JSON; see docs/TURN_ICE.md")?;
            if config.ice_transport_policy != IceTransportPolicy::Relay {
                return Err(
                    "custom TURN qualification requires ice_transport_policy: relay".into(),
                );
            }
            config
        }
        _ => return Err("usage: turn_ice [--direct | --config PATH]".into()),
    };
    let expected = if config.ice_transport_policy == IceTransportPolicy::Relay {
        "relay"
    } else {
        "host"
    };

    // Retain every created peer so errors also run the close path.
    let mut peers = Vec::new();
    let result = qualify(&config, expected, &mut peers).await;
    let allocations_ok = match &fixture {
        Some(server) => server.allocation_count().await == 2,
        None => true,
    };
    let mut close_ok = true;
    for peer in peers {
        close_ok &= peer.close().await.is_ok();
    }
    if let Some(server) = fixture {
        server.wait_for_no_allocations().await;
        server.close().await?;
    }
    result?;
    if !allocations_ok || !close_ok {
        return Err("allocation count or peer cleanup failed".into());
    }
    println!("PASS: {expected}/{expected} ICE nominated; exact audio packets received both ways; peers closed");
    if args.is_empty() {
        println!("PASS: two TURN allocations created and both released; no public STUN used");
    }
    Ok(())
}

async fn qualify(
    config: &WebRtcConfig,
    expected: &str,
    peers: &mut Vec<Arc<RvoipPeerConnection>>,
) -> Result<()> {
    for role in [PeerRole::Offerer, PeerRole::Answerer] {
        let peer = RvoipPeerConnection::new(config, role)
            .await
            .map_err(|_| "peer setup failed: check TURN credentials, reachability and timeouts")?;
        peers.push(peer);
    }
    let (offerer, answerer) = (&peers[0], &peers[1]);
    for peer in peers.iter() {
        peer.add_local_audio_track()
            .await
            .map_err(|_| "audio setup failed")?;
    }
    let offer = offerer
        .create_offer_and_gather()
        .await
        .map_err(|_| "offer gathering failed")?;
    check_candidates(&offer, expected)?;
    let answer = answerer
        .accept_offer_and_gather(&offer)
        .await
        .map_err(|_| "answer gathering failed")?;
    check_candidates(&answer, expected)?;
    offerer
        .set_remote_answer(&answer)
        .await
        .map_err(|_| "answer application failed")?;
    let timeout = Duration::from_secs(config.connection_timeout_secs);
    let (a, b) = tokio::join!(
        offerer.wait_connected(timeout),
        answerer.wait_connected(timeout)
    );
    a.map_err(|_| "offerer ICE/DTLS connection failed")?;
    b.map_err(|_| "answerer ICE/DTLS connection failed")?;
    // Verify the local side at BOTH endpoints; the core may omit the remote
    // candidate-type stats. The SDP check independently excludes direct paths.
    for peer in peers.iter() {
        let pair = peer
            .selected_candidate_pair()
            .await
            .ok_or("no selected ICE pair")?;
        if !pair.nominated || pair.local_candidate_type != expected {
            return Err(format!(
                "ICE selected an unexpected candidate pair: local={}, remote={}, nominated={}",
                pair.local_candidate_type, pair.remote_candidate_type, pair.nominated
            )
            .into());
        }
    }
    verify_audio(offerer, answerer, 123_456, timeout).await?;
    verify_audio(answerer, offerer, 654_321, timeout).await?;
    Ok(())
}

fn check_candidates(sdp: &str, expected: &str) -> Result<()> {
    let candidates: Vec<_> = sdp
        .lines()
        .filter(|line| line.starts_with("a=candidate:"))
        .collect();
    if candidates.is_empty()
        || candidates.iter().any(|line| {
            !line
                .split_whitespace()
                .collect::<Vec<_>>()
                .windows(2)
                .any(|pair| pair == ["typ", expected])
        })
    {
        return Err("SDP has missing or unexpected ICE candidates".into());
    }
    Ok(())
}

async fn verify_audio(
    sender: &Arc<RvoipPeerConnection>,
    receiver: &Arc<RvoipPeerConnection>,
    marker: u32,
    timeout: Duration,
) -> Result<()> {
    let remote = RvoipPeerConnection::prime_remote_track(sender, receiver, timeout)
        .await
        .ok_or("remote audio track did not arrive")?;
    let sequence = tokio::time::timeout(timeout, async {
        while let Some(event) = remote.poll().await {
            if let TrackRemoteEvent::OnRtpPacket(packet) = event {
                // Continue beyond the short priming burst in the same RTP sequence
                // space; a fixed sequence could fall behind its replay window.
                return Some(packet.header.sequence_number.wrapping_add(1024));
            }
        }
        None
    })
    .await
    .map_err(|_| "priming packet timed out")?
    .ok_or("priming track ended")?;
    let payload = silent_opus_payload();
    let packet = rtc::rtp::Packet {
        header: rtc::rtp::Header {
            version: 2,
            payload_type: 111,
            sequence_number: sequence,
            timestamp: marker,
            ssrc: sender.local_audio_ssrc().ok_or("missing audio SSRC")?,
            ..Default::default()
        },
        payload: payload.clone(),
    };
    sender
        .local_audio_track()
        .ok_or("missing local audio track")?
        .write_rtp(packet)
        .await
        .map_err(|_| "audio send failed")?;
    // Priming packets cannot satisfy this assertion: require our exact timestamp
    // and payload after the full encrypted media path, in each direction.
    tokio::time::timeout(timeout, async {
        while let Some(event) = remote.poll().await {
            if let TrackRemoteEvent::OnRtpPacket(packet) = event {
                if packet.header.timestamp == marker && packet.payload == payload {
                    return Ok(());
                }
            }
        }
        Err("remote audio track ended before the expected packet")
    })
    .await
    .map_err(|_| "expected audio packet timed out")??;
    Ok(())
}
