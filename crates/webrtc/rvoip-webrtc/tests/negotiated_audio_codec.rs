//! The local audio sender is bound to the codec an offer/answer selected.
//!
//! The RTC sender stamps each packet with the negotiated codec that matches
//! its SSRC's encoding and drops packets whose encoding matches none. A local
//! track whose encoding is not one of the negotiated codecs therefore sends
//! nothing, while the track wrapper has already reported the write as
//! accepted. These tests drive real peer connections through offer/answer
//! and inspect the actual sender parameters.
//!
//! The SIP/WebRTC wire journeys for the same profiles (PCMU-only, Opus,
//! mixed offer with a G.711-only answer, receive-only WHEP) live in
//! `sip_webrtc_acceptance.rs`.

use rvoip_webrtc::config::WebRtcConfig;
use rvoip_webrtc::peer::{PeerRole, RvoipPeerConnection};
use webrtc::media_stream::Track;
use webrtc::rtp_transceiver::RTCRtpTransceiverDirection;

const SDP_PREFIX: &str = "v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\n";

/// `WebRtcConfig::loopback()` narrowed to one primary audio codec, or left
/// with its default Opus + G.711 list for `"mixed"`.
fn codec_config(name: &str) -> WebRtcConfig {
    let mut config = WebRtcConfig::loopback();
    if name != "mixed" {
        config
            .capabilities
            .audio_codecs
            .retain(|codec| codec.name == name);
        assert_eq!(config.capabilities.audio_codecs.len(), 1);
    }
    config
}

/// Mime type and clock of the encoding the local audio SSRC is sent with.
async fn local_audio_encoding(peer: &RvoipPeerConnection) -> (String, u32) {
    let track = peer.local_audio_track().expect("local audio track");
    let ssrc = peer.local_audio_ssrc().expect("local audio SSRC");
    let codec = track.codec(ssrc).await.expect("local audio encoding");
    (codec.mime_type.to_ascii_lowercase(), codec.clock_rate)
}

/// Mime type, clock and payload type the negotiated sender binds the local
/// audio SSRC to, read from the real sender parameters.
async fn bound_sender_codec(peer: &RvoipPeerConnection) -> (String, u32, u8) {
    let ssrc = peer.local_audio_ssrc().expect("local audio SSRC");
    for sender in peer.peer_connection().get_senders().await {
        let params = sender.get_parameters().await.expect("sender parameters");
        let Some(encoding) = params
            .encodings
            .iter()
            .find(|encoding| encoding.rtp_coding_parameters.ssrc == Some(ssrc))
        else {
            continue;
        };
        let negotiated = params
            .rtp_parameters
            .codecs
            .iter()
            .find(|codec| {
                codec
                    .rtp_codec
                    .mime_type
                    .eq_ignore_ascii_case(&encoding.codec.mime_type)
                    && codec.rtp_codec.clock_rate == encoding.codec.clock_rate
            })
            .unwrap_or_else(|| {
                panic!(
                    "local audio encoding {} is not among the negotiated codecs",
                    encoding.codec.mime_type
                )
            });
        return (
            encoding.codec.mime_type.to_ascii_lowercase(),
            encoding.codec.clock_rate,
            negotiated.payload_type,
        );
    }
    panic!("no sender carries the local audio SSRC");
}

#[tokio::test]
async fn pcmu_only_answerer_builds_a_pcmu_track_and_refuses_a_codec_change() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let peer = RvoipPeerConnection::new(&codec_config("g.711-mu"), PeerRole::Answerer)
        .await
        .expect("answerer");
    // A static PT 0 offer with no rtpmap, as a SIP-originated offer may send.
    let pcmu_offer = format!("{SDP_PREFIX}m=audio 9 UDP/TLS/RTP/SAVPF 0\r\nc=IN IP4 127.0.0.1\r\n");
    peer.prepare_answerer_media_for_offer(&pcmu_offer)
        .await
        .expect("PCMU answerer media");
    assert_eq!(
        local_audio_encoding(&peer).await,
        ("audio/pcmu".to_owned(), 8_000),
        "a PCMU-only route must not send an Opus/48000 track"
    );

    let opus_offer = format!(
        "{SDP_PREFIX}m=audio 9 UDP/TLS/RTP/SAVPF 111\r\nc=IN IP4 127.0.0.1\r\na=rtpmap:111 opus/48000/2\r\n"
    );
    assert!(
        peer.prepare_answerer_media_for_offer(&opus_offer)
            .await
            .is_err(),
        "a renegotiation may not silently keep a track whose encoding changed"
    );
    peer.close().await.expect("close peer");
}

#[tokio::test]
async fn offer_answer_binds_the_sender_to_the_answered_codec() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    // (offerer profile, answerer profile, reverse the offer's audio formats,
    //  expected mime, clock, payload type)
    for (offer_profile, answer_profile, reverse, mime, clock, pt) in [
        ("g.711-mu", "g.711-mu", false, "audio/pcmu", 8_000, 0),
        ("opus", "opus", false, "audio/opus", 48_000, 111),
        // Opus-first mixed offer answered by a G.711-only peer: the offerer's
        // first local capability (Opus) is not what the answer selected.
        ("mixed", "g.711-mu", false, "audio/pcmu", 8_000, 0),
        ("mixed", "g.711-mu", true, "audio/pcmu", 8_000, 0),
    ] {
        let case = format!("offer={offer_profile} answer={answer_profile} reverse={reverse}");
        let offerer = RvoipPeerConnection::new(&codec_config(offer_profile), PeerRole::Offerer)
            .await
            .expect("offerer");
        let mut offer = offerer.create_offer_and_gather().await.expect("offer");
        if reverse {
            offer = reverse_audio_formats(&offer);
        }
        let answerer = RvoipPeerConnection::new(&codec_config(answer_profile), PeerRole::Answerer)
            .await
            .expect("answerer");
        let answer = answerer
            .accept_offer_and_gather(&offer)
            .await
            .unwrap_or_else(|error| panic!("{case}: answer: {error}"));
        offerer
            .set_remote_answer(&answer)
            .await
            .unwrap_or_else(|error| panic!("{case}: set_remote_answer: {error}"));

        for (side, peer) in [("offerer", &offerer), ("answerer", &answerer)] {
            assert_eq!(
                bound_sender_codec(peer).await,
                (mime.to_owned(), clock, pt),
                "{case}: {side} sender"
            );
        }
        offerer.close().await.expect("close offerer");
        answerer.close().await.expect("close answerer");
    }
}

#[tokio::test]
async fn receive_only_offer_needs_no_sender_binding() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let offerer = RvoipPeerConnection::new(&codec_config("mixed"), PeerRole::Offerer)
        .await
        .expect("offerer");
    offerer
        .prepare_receive_only_offer()
        .await
        .expect("receive-only offer");
    let offer = offerer.create_offer_and_gather().await.expect("offer");
    let answerer = RvoipPeerConnection::new(&codec_config("g.711-mu"), PeerRole::Answerer)
        .await
        .expect("answerer");
    let answer = answerer
        .accept_offer_and_gather(&offer)
        .await
        .expect("PCMU answer");
    // The RTC core exposes no RTP sender for a transceiver without a send
    // direction, so binding must not require one.
    offerer
        .set_remote_answer(&answer)
        .await
        .expect("receive-only answer is accepted without a sender binding");

    let transceivers = offerer.peer_connection().get_transceivers().await;
    assert!(!transceivers.is_empty(), "receive-only keeps a transceiver");
    for transceiver in transceivers {
        let direction = transceiver.direction().await.expect("direction");
        assert_eq!(direction, RTCRtpTransceiverDirection::Recvonly);
        assert_eq!(
            transceiver
                .current_direction()
                .await
                .expect("current direction"),
            RTCRtpTransceiverDirection::Recvonly
        );
    }
    offerer.close().await.expect("close offerer");
    answerer.close().await.expect("close answerer");
}

/// Reverse the payload-type order of every `m=audio` line.
fn reverse_audio_formats(sdp: &str) -> String {
    sdp.lines()
        .map(|line| {
            if line.starts_with("m=audio ") {
                let fields = line.split_whitespace().collect::<Vec<_>>();
                let mut formats = fields[3..].to_vec();
                formats.reverse();
                format!("{} {}", fields[..3].join(" "), formats.join(" "))
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\r\n")
        + "\r\n"
}
