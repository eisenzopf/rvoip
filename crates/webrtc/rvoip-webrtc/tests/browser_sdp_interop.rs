//! H6.3: recorded-browser SDP interop — feed a representative Chrome WHIP
//! offer through `apply_remote_offer` and validate the resulting answer is
//! shaped the way a browser expects.
//!
//! The fixture is a real audio-only offer captured from Chromium 120 with
//! `RTCPeerConnection().createOffer()` + an `addTrack(audio)`. SSRCs, ufrag,
//! and fingerprint were anonymized — they don't need to validate cryptographically
//! since the server only parses/echos them.

#![cfg(feature = "signaling-whip")]

use rvoip_webrtc::{WebRtcAdapter, WebRtcConfig};

const CHROME_OFFER_SDP: &str = "v=0\r\n\
o=- 8367589427365485632 2 IN IP4 127.0.0.1\r\n\
s=-\r\n\
t=0 0\r\n\
a=group:BUNDLE 0\r\n\
a=extmap-allow-mixed\r\n\
a=msid-semantic: WMS rvoip-test-stream\r\n\
m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n\
c=IN IP4 0.0.0.0\r\n\
a=rtcp:9 IN IP4 0.0.0.0\r\n\
a=ice-ufrag:Hbgf\r\n\
a=ice-pwd:b9LhzpVk3K8aRn3PiNoYqVtm\r\n\
a=ice-options:trickle\r\n\
a=fingerprint:sha-256 13:14:DD:9E:5F:91:00:46:11:50:6C:90:8B:9E:AA:F2:14:31:F3:18:C9:00:48:6F:1D:34:33:36:8B:DE:F0:23\r\n\
a=setup:actpass\r\n\
a=mid:0\r\n\
a=extmap:1 urn:ietf:params:rtp-hdrext:ssrc-audio-level\r\n\
a=extmap:2 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01\r\n\
a=sendrecv\r\n\
a=msid:rvoip-test-stream rvoip-test-audio\r\n\
a=rtcp-mux\r\n\
a=rtcp-rsize\r\n\
a=rtpmap:111 opus/48000/2\r\n\
a=rtcp-fb:111 transport-cc\r\n\
a=fmtp:111 minptime=10;useinbandfec=1\r\n\
a=ssrc:1234567890 cname:rvoip-test\r\n\
a=ssrc:1234567890 msid:rvoip-test-stream rvoip-test-audio\r\n\
a=ssrc:1234567890 mslabel:rvoip-test-stream\r\n\
a=ssrc:1234567890 label:rvoip-test-audio\r\n\
a=candidate:1 1 udp 2122260223 127.0.0.1 50000 typ host generation 0\r\n\
a=end-of-candidates\r\n";

#[tokio::test]
async fn chrome_audio_offer_produces_well_formed_answer() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let conn_id = adapter
        .apply_remote_offer(CHROME_OFFER_SDP)
        .await
        .expect("apply_remote_offer for Chrome offer");

    let answer = adapter.local_sdp(&conn_id).expect("local sdp present");

    // Required ICE+DTLS bits.
    assert!(answer.contains("a=ice-ufrag:"), "answer must carry a fresh ICE ufrag");
    assert!(answer.contains("a=ice-pwd:"), "answer must carry a fresh ICE pwd");
    assert!(answer.contains("a=fingerprint:"), "answer must carry DTLS fingerprint");
    assert!(
        answer.contains("a=setup:active") || answer.contains("a=setup:passive"),
        "answer must commit to active or passive setup (got actpass-style answer)"
    );

    // Audio negotiation.
    assert!(answer.contains("m=audio "), "answer must include the audio m-section");
    assert!(
        answer.contains("a=rtpmap:111 opus/48000/2"),
        "Opus PT 111 must be echoed"
    );

    // BUNDLE + rtcp-mux carry through.
    assert!(answer.contains("a=group:BUNDLE"));
    assert!(answer.contains("a=rtcp-mux"));

    // mid must match the offer's mid:0.
    assert!(answer.contains("a=mid:0"));

    // The server should accept a follow-up trickle candidate for this route.
    let init = webrtc::peer_connection::RTCIceCandidateInit {
        candidate: "candidate:2 1 udp 2122260223 127.0.0.2 50001 typ host".to_owned(),
        sdp_mid: Some("0".into()),
        sdp_mline_index: Some(0),
        username_fragment: None,
        url: None,
    };
    adapter
        .apply_trickle_candidate(&conn_id, init)
        .await
        .expect("trickle candidate accepted");

    // G6 — header extensions offered by the client should round-trip into
    // the answer (webrtc-rs negotiates only those the offer advertised).
    for ext_uri in [
        "urn:ietf:params:rtp-hdrext:ssrc-audio-level",
        "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01",
    ] {
        assert!(
            answer.contains(ext_uri),
            "answer should echo extmap:{ext_uri}\n--- answer ---\n{answer}"
        );
    }
}

/// G6 — Safari 17 audio offer: H.264-only video would normally appear but
/// we only exercise the audio path here. Safari emits `extmap` for MID,
/// audio-level, and abs-send-time.
const SAFARI_AUDIO_OFFER: &str = "v=0\r\n\
o=- 2123456789 2 IN IP4 127.0.0.1\r\n\
s=-\r\n\
t=0 0\r\n\
a=group:BUNDLE 0\r\n\
a=msid-semantic: WMS safari-stream\r\n\
m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n\
c=IN IP4 0.0.0.0\r\n\
a=rtcp:9 IN IP4 0.0.0.0\r\n\
a=ice-ufrag:SFRi\r\n\
a=ice-pwd:0123456789abcdef0123456789abcdef\r\n\
a=ice-options:trickle\r\n\
a=fingerprint:sha-256 AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89\r\n\
a=setup:actpass\r\n\
a=mid:0\r\n\
a=extmap:1 urn:ietf:params:rtp-hdrext:ssrc-audio-level\r\n\
a=extmap:3 http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time\r\n\
a=sendrecv\r\n\
a=rtcp-mux\r\n\
a=rtpmap:111 opus/48000/2\r\n\
a=fmtp:111 minptime=10;useinbandfec=1\r\n\
a=ssrc:1112223334 cname:safari-test\r\n";

#[tokio::test]
async fn safari_audio_offer_negotiates_opus_and_echoes_audio_level() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let conn_id = adapter
        .apply_remote_offer(SAFARI_AUDIO_OFFER)
        .await
        .expect("apply Safari offer");
    let answer = adapter.local_sdp(&conn_id).expect("answer");

    assert!(answer.contains("a=rtpmap:111 opus/48000/2"));
    assert!(answer.contains("a=ice-ufrag:"));
    assert!(answer.contains("a=fingerprint:"));
    assert!(
        answer.contains("urn:ietf:params:rtp-hdrext:ssrc-audio-level"),
        "Safari fixture sends audio-level — answer must echo it"
    );
    assert!(
        answer.contains("http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time"),
        "Safari fixture sends abs-send-time — answer must echo it"
    );
}

/// G6 — Firefox 125 audio-only offer (stereo Opus, MID hdrext).
const FIREFOX_AV_OFFER: &str = "v=0\r\n\
o=mozilla...THIS_IS_SDPARTA-99.0 0 0 IN IP4 0.0.0.0\r\n\
s=-\r\n\
t=0 0\r\n\
a=fingerprint:sha-256 11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00\r\n\
a=group:BUNDLE 0\r\n\
a=ice-options:trickle\r\n\
a=msid-semantic:WMS firefox-stream\r\n\
m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n\
c=IN IP4 0.0.0.0\r\n\
a=sendrecv\r\n\
a=extmap:1 urn:ietf:params:rtp-hdrext:ssrc-audio-level\r\n\
a=extmap:3 urn:ietf:params:rtp-hdrext:sdes:mid\r\n\
a=fmtp:111 maxplaybackrate=48000;stereo=1;useinbandfec=1\r\n\
a=ice-pwd:abcdefabcdefabcdefabcdefabcdef00\r\n\
a=ice-ufrag:ffabcd\r\n\
a=mid:0\r\n\
a=msid:firefox-stream firefox-audio\r\n\
a=rtcp-mux\r\n\
a=rtpmap:111 opus/48000/2\r\n\
a=setup:actpass\r\n\
a=ssrc:9876543210 cname:firefox-test\r\n\
a=ssrc:9876543210 msid:firefox-stream firefox-audio\r\n\
a=ssrc:9876543210 mslabel:firefox-stream\r\n\
a=ssrc:9876543210 label:firefox-audio\r\n";

#[tokio::test]
async fn firefox_audio_offer_negotiates_opus_with_mid_hdrext() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let conn_id = adapter
        .apply_remote_offer(FIREFOX_AV_OFFER)
        .await
        .expect("apply Firefox offer");
    let answer = adapter.local_sdp(&conn_id).expect("answer");

    assert!(answer.contains("m=audio "));
    assert!(answer.contains("a=rtpmap:111 opus/48000/2"));
    assert!(answer.contains("a=group:BUNDLE"));
    // Firefox offered MID hdrext — answer must echo it.
    assert!(
        answer.contains("urn:ietf:params:rtp-hdrext:sdes:mid"),
        "Firefox offered MID hdrext; answer must echo it"
    );
}

#[tokio::test]
async fn malformed_sdp_returns_error_not_panic() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let adapter = WebRtcAdapter::new(WebRtcConfig::loopback());

    // Garbage that has no v=0 / m= line.
    let err = adapter
        .apply_remote_offer("not an sdp\r\n")
        .await
        .expect_err("garbage must be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("sdp") || msg.contains("webrtc"),
        "error should be diagnostic: {msg}"
    );
}
