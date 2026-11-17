use rtc::peer_connection::RTCPeerConnectionBuilder;
use rtc::peer_connection::configuration::media_engine::MediaEngine;
use rtc::rtp_transceiver::rtp_sender::RtpCodecKind;
use std::time::Instant;

fn audio_formats(sdp: &str) -> Vec<String> {
    sdp.lines()
        .find(|line| line.starts_with("m=audio "))
        .expect("SDP contains an audio media section")
        .split_whitespace()
        .skip(3)
        .map(str::to_owned)
        .collect()
}

#[test]
fn answer_preserves_remote_audio_codec_preference_order() {
    let mut offer_media = MediaEngine::default();
    offer_media
        .register_default_codecs()
        .expect("register offer codecs");
    let mut offerer = RTCPeerConnectionBuilder::new()
        .with_media_engine(offer_media)
        .build(Instant::now())
        .expect("build offerer");
    offerer
        .add_transceiver_from_kind(RtpCodecKind::Audio, None)
        .expect("add offerer audio transceiver");

    let mut answer_media = MediaEngine::default();
    answer_media
        .register_default_codecs()
        .expect("register answer codecs");
    let mut answerer = RTCPeerConnectionBuilder::new()
        .with_media_engine(answer_media)
        .build(Instant::now())
        .expect("build answerer");

    let offer = offerer.create_offer(None).expect("create offer");
    let offered_formats = audio_formats(&offer.sdp);
    offerer
        .set_local_description(Instant::now(), offer.clone())
        .expect("set offer locally");
    answerer
        .set_remote_description(Instant::now(), offer)
        .expect("set remote offer");

    let answer = answerer.create_answer(None).expect("create answer");
    let answered_formats = audio_formats(&answer.sdp);

    assert_eq!(answered_formats, offered_formats);
    assert_eq!(answered_formats.first().map(String::as_str), Some("111"));
}
