//! H6.3: recorded-browser SDP interop — feed a representative Chrome WHIP
//! offer through `apply_remote_offer` and validate the resulting answer is
//! shaped the way a browser expects.
//!
//! The fixture is a real audio-only offer captured from Chromium 120 with
//! `RTCPeerConnection().createOffer()` + an `addTrack(audio)`. SSRCs, ufrag,
//! and fingerprint were anonymized — they don't need to validate cryptographically
//! since the server only parses/echos them.

#![cfg(feature = "signaling-whip")]

use rvoip_webrtc::media::dtmf::{OutboundDtmfNegotiation, TelephoneEventCodec};
use rvoip_webrtc::peer::{connect_loopback, PeerRole, RvoipPeerConnection};
use rvoip_webrtc::{WebRtcAdapter, WebRtcConfig, WebRtcError};

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

const CHROME_MULTI_CODEC_OFFER_SDP: &str = "v=0\r\n\
o=- 8367589427365485632 2 IN IP4 127.0.0.1\r\n\
s=-\r\n\
t=0 0\r\n\
a=group:BUNDLE 0\r\n\
a=msid-semantic: WMS rvoip-test-stream\r\n\
m=audio 9 UDP/TLS/RTP/SAVPF 111 0 8 110\r\n\
c=IN IP4 0.0.0.0\r\n\
a=rtcp:9 IN IP4 0.0.0.0\r\n\
a=ice-ufrag:Hbgf\r\n\
a=ice-pwd:b9LhzpVk3K8aRn3PiNoYqVtm\r\n\
a=ice-options:trickle\r\n\
a=fingerprint:sha-256 13:14:DD:9E:5F:91:00:46:11:50:6C:90:8B:9E:AA:F2:14:31:F3:18:C9:00:48:6F:1D:34:33:36:8B:DE:F0:23\r\n\
a=setup:actpass\r\n\
a=mid:0\r\n\
a=sendrecv\r\n\
a=msid:rvoip-test-stream rvoip-test-audio\r\n\
a=rtcp-mux\r\n\
a=rtpmap:111 opus/48000/2\r\n\
a=fmtp:111 minptime=10;useinbandfec=1\r\n\
a=rtpmap:0 PCMU/8000\r\n\
a=rtpmap:8 PCMA/8000\r\n\
a=rtpmap:110 telephone-event/48000\r\n\
a=fmtp:110 0-15\r\n\
a=ssrc:1234567890 cname:rvoip-test\r\n\
a=candidate:1 1 udp 2122260223 127.0.0.1 50000 typ host generation 0\r\n\
a=end-of-candidates\r\n";

fn chrome_offer_with_telephone_event(payload_type: u8, clock_rate_hz: u32) -> String {
    CHROME_MULTI_CODEC_OFFER_SDP
        .replace(" 110\r\n", &format!(" {payload_type}\r\n"))
        .replace(
            "a=rtpmap:110 telephone-event/48000",
            &format!("a=rtpmap:{payload_type} telephone-event/{clock_rate_hz}"),
        )
        .replace("a=fmtp:110 0-15", &format!("a=fmtp:{payload_type} 0-15"))
}

fn prefer_48khz_telephone_event(sdp: &str) -> String {
    let mut output = String::with_capacity(sdp.len());
    for line in sdp.lines() {
        if let Some(rest) = line.strip_prefix("m=audio ") {
            let mut fields = rest.split_whitespace().collect::<Vec<_>>();
            if fields.len() >= 2 {
                let mut reordered = fields.drain(..2).collect::<Vec<_>>();
                let mut inserted_dtmf = false;
                for payload in fields {
                    if matches!(payload, "101" | "110" | "126") {
                        if !inserted_dtmf {
                            reordered.extend(["110", "101", "126"]);
                            inserted_dtmf = true;
                        }
                    } else {
                        reordered.push(payload);
                    }
                }
                output.push_str("m=audio ");
                output.push_str(&reordered.join(" "));
                output.push_str("\r\n");
                continue;
            }
        }
        output.push_str(line);
        output.push_str("\r\n");
    }
    output
}

fn retain_only_48khz_telephone_event(sdp: &str) -> String {
    let mut output = String::with_capacity(sdp.len());
    for line in sdp.lines() {
        if let Some(rest) = line.strip_prefix("m=audio ") {
            let mut fields = rest.split_whitespace().collect::<Vec<_>>();
            if fields.len() >= 2 {
                let mut retained = fields.drain(..2).collect::<Vec<_>>();
                retained.extend(
                    fields
                        .into_iter()
                        .filter(|payload| !matches!(*payload, "101" | "126")),
                );
                output.push_str("m=audio ");
                output.push_str(&retained.join(" "));
                output.push_str("\r\n");
                continue;
            }
        }
        if line.starts_with("a=rtpmap:101 ")
            || line.starts_with("a=fmtp:101 ")
            || line.starts_with("a=rtpmap:126 ")
            || line.starts_with("a=fmtp:126 ")
        {
            continue;
        }
        output.push_str(line);
        output.push_str("\r\n");
    }
    output
}

#[tokio::test]
async fn opus_only_profile_answers_a_multi_codec_browser_offer_deterministically() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut config = WebRtcConfig::loopback();
    config
        .capabilities
        .audio_codecs
        .retain(|codec| codec.name.eq_ignore_ascii_case("opus"));
    let adapter = WebRtcAdapter::new(config);
    let conn_id = adapter
        .apply_remote_offer(CHROME_MULTI_CODEC_OFFER_SDP)
        .await
        .expect("Opus-only answer");
    let answer = adapter.local_sdp(&conn_id).expect("local answer");
    let negotiated =
        rvoip_webrtc::sdp::negotiated_single_audio_payload(CHROME_MULTI_CODEC_OFFER_SDP, &answer)
            .expect("exact final codec/PT");
    assert_eq!(negotiated.codec.name, "opus");
    assert_eq!(negotiated.payload_type, 111);
    assert!(!answer.contains(" PCMU/"));
    assert!(!answer.contains(" PCMA/"));
    assert!(
        answer.contains("a=rtpmap:110 telephone-event/48000"),
        "final answer must actually accept Chromium PT 110/48 kHz"
    );
    assert_eq!(
        rvoip_webrtc::sdp::negotiated_telephone_event_codec(CHROME_MULTI_CODEC_OFFER_SDP, &answer,),
        Some(TelephoneEventCodec::new(110, 48_000))
    );

    let route = adapter.routes().get(&conn_id).expect("retained route");
    assert_eq!(route.negotiated.audio.as_ref().unwrap().name, "opus");
    assert_eq!(
        route.peer.negotiated_outbound_dtmf_codec(),
        Some(TelephoneEventCodec::new(110, 48_000))
    );
    assert_eq!(
        route.peer.local_dtmf_codec(),
        Some(TelephoneEventCodec::new(110, 48_000)),
        "the supplemental sender encoding must bind with Chromium's selected clock"
    );
}

#[tokio::test]
async fn eight_khz_dynamic_telephone_event_offer_binds_pt126_sender_clock() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let offer = chrome_offer_with_telephone_event(126, 8_000);
    let adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let conn_id = adapter
        .apply_remote_offer(&offer)
        .await
        .expect("8 kHz telephone-event answer");
    let answer = adapter.local_sdp(&conn_id).expect("local answer");
    assert!(
        answer.contains("a=rtpmap:126 telephone-event/8000"),
        "final answer must actually accept PT 126/8 kHz"
    );
    let route = adapter.routes().get(&conn_id).expect("retained route");
    assert_eq!(
        route.peer.negotiated_outbound_dtmf_codec(),
        Some(TelephoneEventCodec::new(126, 8_000))
    );
    assert_eq!(
        route.peer.local_dtmf_codec(),
        Some(TelephoneEventCodec::new(126, 8_000))
    );
}

#[tokio::test]
async fn failed_clock_rate_renegotiation_preserves_live_dtmf_mapping() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let conn_id = adapter
        .apply_remote_offer(CHROME_MULTI_CODEC_OFFER_SDP)
        .await
        .expect("initial 48 kHz answer");
    let peer = {
        let route = adapter.routes().get(&conn_id).expect("retained route");
        std::sync::Arc::clone(&route.peer)
    };

    let changed_clock = chrome_offer_with_telephone_event(126, 8_000)
        .replace("8367589427365485632 2 IN", "8367589427365485632 3 IN");
    assert!(peer.renegotiate_as_answerer(&changed_clock).await.is_err());
    assert_eq!(
        peer.outbound_dtmf_negotiation(),
        OutboundDtmfNegotiation::Negotiated(TelephoneEventCodec::new(110, 48_000))
    );
    assert_eq!(
        peer.local_dtmf_codec(),
        Some(TelephoneEventCodec::new(110, 48_000))
    );
}

#[tokio::test]
async fn same_clock_pt_remap_omitted_by_final_answer_fails_closed() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let initial = chrome_offer_with_telephone_event(126, 8_000);
    let conn_id = adapter
        .apply_remote_offer(&initial)
        .await
        .expect("initial PT126/8 kHz answer");
    let peer = {
        let route = adapter.routes().get(&conn_id).expect("retained route");
        std::sync::Arc::clone(&route.peer)
    };

    // Both mappings are explicitly registered. The clock stays fixed while
    // only the dynamic payload type changes across the completed exchange.
    let remapped = chrome_offer_with_telephone_event(101, 8_000)
        .replace("8367589427365485632 2 IN", "8367589427365485632 3 IN");
    let answer = peer
        .renegotiate_as_answerer(&remapped)
        .await
        .expect("primary audio renegotiation");
    assert!(
        !answer.contains("a=rtpmap:101 telephone-event/8000"),
        "the alpha engine currently retains its established PT instead of accepting the remap"
    );
    assert_eq!(
        rvoip_webrtc::sdp::negotiated_telephone_event_codec(&remapped, &answer),
        None,
        "a remap omitted by final SDP is not negotiated"
    );
    assert_eq!(
        peer.outbound_dtmf_negotiation(),
        OutboundDtmfNegotiation::Unsupported
    );
    assert_eq!(
        peer.negotiated_outbound_dtmf_codec(),
        None,
        "the old physical track binding must not be presented as negotiated"
    );
    assert!(matches!(
        rvoip_webrtc::media::dtmf::send_dtmf(&peer, "5", 120).await,
        Err(WebRtcError::IncompatibleCapabilities)
    ));
}

#[tokio::test]
async fn preattached_audio_offer_owns_supplemental_dtmf_encodings() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let peer = RvoipPeerConnection::new(&WebRtcConfig::loopback(), PeerRole::Offerer)
        .await
        .expect("offerer");
    peer.add_local_audio_track().await.expect("audio track");
    assert_eq!(
        peer.local_dtmf_codec(),
        Some(TelephoneEventCodec::default())
    );
    let audio = peer.local_audio_track().expect("primary audio handle");
    let dtmf = peer.local_dtmf_track().expect("supplemental DTMF handle");
    assert!(
        std::sync::Arc::ptr_eq(&audio, &dtmf),
        "both clock encodings must share one negotiated sender/track"
    );
    assert_ne!(
        peer.local_audio_ssrc(),
        peer.local_dtmf_ssrc(),
        "primary audio and telephone-event require independent SSRC timelines"
    );

    let offer = peer.create_offer_and_gather().await.expect("offer");
    assert_eq!(
        offer.matches("m=audio ").count(),
        1,
        "Opus and telephone-event belong to one negotiated audio m-section"
    );
    assert_eq!(
        peer.local_dtmf_codec(),
        Some(TelephoneEventCodec::default())
    );
    for mapping in [
        "a=rtpmap:101 telephone-event/8000",
        "a=rtpmap:110 telephone-event/48000",
        "a=rtpmap:126 telephone-event/8000",
    ] {
        assert!(offer.contains(mapping), "offer omitted {mapping}");
    }
}

#[tokio::test]
async fn offerer_accepts_a_final_pt110_48khz_answer_on_its_shared_audio_sender() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = WebRtcConfig::loopback();
    let offerer = RvoipPeerConnection::new(&config, PeerRole::Offerer)
        .await
        .expect("offerer");
    let offer = offerer.create_offer_and_gather().await.expect("offer");
    let eight_khz_ssrc = offerer
        .local_dtmf_ssrc()
        .expect("pre-negotiation 8 kHz encoding");

    // A standards-compliant answer may select any mapping from the offer. Give
    // the answerer the same capabilities with 48 kHz preferred so its physical
    // sender matches the PT110-only answer returned to our offerer.
    let answerer_offer = prefer_48khz_telephone_event(&offer);
    let answerer = RvoipPeerConnection::new(&config, PeerRole::Answerer)
        .await
        .expect("answerer");
    let full_answer = answerer
        .accept_offer_and_gather(&answerer_offer)
        .await
        .expect("answer");
    let answer = retain_only_48khz_telephone_event(&full_answer);
    assert!(answer.contains("a=rtpmap:110 telephone-event/48000"));
    assert!(!answer.contains("a=rtpmap:101 telephone-event/8000"));
    assert!(!answer.contains("a=rtpmap:126 telephone-event/8000"));

    offerer
        .set_remote_answer(&answer)
        .await
        .expect("PT110 final answer");
    assert_eq!(
        offerer.outbound_dtmf_negotiation(),
        OutboundDtmfNegotiation::Negotiated(TelephoneEventCodec::new(110, 48_000))
    );
    assert_ne!(
        offerer.local_dtmf_ssrc(),
        Some(eight_khz_ssrc),
        "final PT110 must select the distinct 48 kHz SSRC encoding"
    );

    offerer.close().await.ok();
    answerer.close().await.ok();
}

#[tokio::test]
async fn receive_only_offer_rejects_outbound_dtmf_before_writing() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let peer = RvoipPeerConnection::new(&WebRtcConfig::loopback(), PeerRole::Offerer)
        .await
        .expect("offerer");
    peer.prepare_receive_only_offer()
        .await
        .expect("receive-only preparation");
    assert_eq!(
        peer.outbound_dtmf_negotiation(),
        OutboundDtmfNegotiation::Unsupported
    );
    assert!(matches!(
        rvoip_webrtc::media::dtmf::send_dtmf(&peer, "5", 120).await,
        Err(WebRtcError::IncompatibleCapabilities)
    ));
}

#[tokio::test]
async fn pending_or_midless_dtmf_fails_closed_before_writing() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let pending = RvoipPeerConnection::new(&WebRtcConfig::loopback(), PeerRole::Offerer)
        .await
        .expect("offerer");
    pending.add_local_audio_track().await.expect("local audio");
    assert_eq!(
        pending.outbound_dtmf_negotiation(),
        OutboundDtmfNegotiation::Pending
    );
    assert_eq!(pending.negotiated_outbound_audio_mid(), None);
    assert!(matches!(
        rvoip_webrtc::media::dtmf::send_dtmf(&pending, "5", 120).await,
        Err(WebRtcError::InvalidState(_))
    ));

    // This fixture negotiates telephone-event but deliberately has no SDES
    // MID extmap. The track is not connected, so receiving the capability
    // error proves rejection happened before a write was attempted.
    let adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
    let conn_id = adapter
        .apply_remote_offer(CHROME_MULTI_CODEC_OFFER_SDP)
        .await
        .expect("telephone-event answer");
    let peer = {
        let route = adapter.routes().get(&conn_id).expect("retained route");
        std::sync::Arc::clone(&route.peer)
    };
    assert_eq!(
        peer.outbound_dtmf_negotiation(),
        OutboundDtmfNegotiation::Negotiated(TelephoneEventCodec::new(110, 48_000))
    );
    assert_eq!(peer.negotiated_outbound_audio_mid(), None);
    assert!(matches!(
        rvoip_webrtc::media::dtmf::send_dtmf(&peer, "5", 120).await,
        Err(WebRtcError::IncompatibleCapabilities)
    ));
}

#[tokio::test]
async fn pending_offerer_renegotiation_clears_the_previous_dtmf_mid_binding() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let (offerer, answerer) = connect_loopback(&WebRtcConfig::loopback())
        .await
        .expect("initial negotiated loopback");
    assert!(matches!(
        offerer.outbound_dtmf_negotiation(),
        OutboundDtmfNegotiation::Negotiated(_)
    ));
    assert!(offerer.negotiated_outbound_audio_mid().is_some());

    offerer
        .renegotiate_as_offerer()
        .await
        .expect("pending local offer");
    assert_eq!(
        offerer.outbound_dtmf_negotiation(),
        OutboundDtmfNegotiation::Pending
    );
    assert_eq!(offerer.negotiated_outbound_audio_mid(), None);
    assert!(matches!(
        rvoip_webrtc::media::dtmf::send_dtmf(&offerer, "5", 120).await,
        Err(WebRtcError::InvalidState(_))
    ));

    offerer.close().await.ok();
    answerer.close().await.ok();
}

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
    assert!(
        answer.contains("a=ice-ufrag:"),
        "answer must carry a fresh ICE ufrag"
    );
    assert!(
        answer.contains("a=ice-pwd:"),
        "answer must carry a fresh ICE pwd"
    );
    assert!(
        answer.contains("a=fingerprint:"),
        "answer must carry DTLS fingerprint"
    );
    assert!(
        answer.contains("a=setup:active") || answer.contains("a=setup:passive"),
        "answer must commit to active or passive setup (got actpass-style answer)"
    );

    // Audio negotiation.
    assert!(
        answer.contains("m=audio "),
        "answer must include the audio m-section"
    );
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

    let peer = {
        let route = adapter.routes().get(&conn_id).expect("retained route");
        std::sync::Arc::clone(&route.peer)
    };
    assert_eq!(peer.negotiated_outbound_dtmf_codec(), None);
    assert_eq!(
        peer.outbound_dtmf_negotiation(),
        OutboundDtmfNegotiation::Unsupported
    );
    assert_eq!(peer.local_dtmf_codec(), None);
    assert!(matches!(
        rvoip_webrtc::media::dtmf::send_dtmf(&peer, "5", 120).await,
        Err(WebRtcError::IncompatibleCapabilities)
    ));
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
