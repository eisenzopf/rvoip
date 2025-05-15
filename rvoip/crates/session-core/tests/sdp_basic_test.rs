use std::net::IpAddr;
use std::str::FromStr;

use rvoip_session_core::{
    media::AudioCodecType,
    sdp::{self, SessionDescription, NegotiationState},
};

#[test]
fn test_sdp_offer_answer_negotiation() {
    // Create an SDP offer
    let local_addr = IpAddr::from_str("192.168.1.1").unwrap();
    let local_port = 10000;
    let supported_codecs = vec![AudioCodecType::PCMU, AudioCodecType::PCMA];
    
    let sdp_offer = sdp::create_audio_offer(
        local_addr,
        local_port,
        &supported_codecs
    ).unwrap();
    
    println!("SDP Offer:\n{}", sdp_offer);
    
    // Verify the SDP offer contains the expected codecs
    let audio_media = sdp_offer.media_descriptions.iter()
        .find(|m| m.media == "audio")
        .expect("No audio media found in offer");
    
    assert_eq!(audio_media.port, local_port);
    assert!(audio_media.formats.contains(&"0".to_string()), "PCMU codec not found");
    assert!(audio_media.formats.contains(&"8".to_string()), "PCMA codec not found");
    
    // Create an SDP answer
    let remote_addr = IpAddr::from_str("192.168.2.1").unwrap();
    let remote_port = 20000;
    let remote_supported_codecs = vec![AudioCodecType::PCMU]; // Only supporting PCMU
    
    let sdp_answer = sdp::create_audio_answer(
        &sdp_offer,
        remote_addr,
        remote_port,
        &remote_supported_codecs
    ).unwrap();
    
    println!("SDP Answer:\n{}", sdp_answer);
    
    // Verify the SDP answer contains only the negotiated codec
    let answer_audio_media = sdp_answer.media_descriptions.iter()
        .find(|m| m.media == "audio")
        .expect("No audio media found in answer");
    
    assert_eq!(answer_audio_media.port, remote_port);
    assert!(answer_audio_media.formats.contains(&"0".to_string()), "PCMU codec not found");
    assert!(!answer_audio_media.formats.contains(&"8".to_string()), "PCMA codec shouldn't be in answer");
    
    // Create an SDP context and simulate offer/answer exchange
    let mut sdp_context = sdp::SdpContext::new();
    
    // Update with local offer
    sdp_context.update_with_local_offer(sdp_offer.clone());
    assert_eq!(sdp_context.state, NegotiationState::OfferSent);
    
    // Update with remote answer
    sdp_context.update_with_remote_answer(sdp_answer.clone());
    assert_eq!(sdp_context.state, NegotiationState::Complete);
    
    // Extract media config
    let media_config = sdp::extract_media_config(
        sdp_context.local_sdp.as_ref().unwrap(),
        sdp_context.remote_sdp.as_ref().unwrap()
    ).unwrap();
    
    // Verify media config
    assert_eq!(media_config.local_addr.ip(), local_addr);
    assert_eq!(media_config.local_addr.port(), local_port);
    assert_eq!(media_config.remote_addr.unwrap().ip(), remote_addr);
    assert_eq!(media_config.remote_addr.unwrap().port(), remote_port);
    assert_eq!(media_config.audio_codec, AudioCodecType::PCMU);
    
    // Test re-INVITE for hold
    let hold_sdp = sdp::update_sdp_for_reinvite(
        sdp_context.local_sdp.as_ref().unwrap(),
        None, // Same port
        Some(rvoip_sip_core::sdp::attributes::MediaDirection::SendOnly) // Hold
    ).unwrap();
    
    // Verify the hold SDP has sendonly direction
    let hold_media = hold_sdp.media_descriptions.iter()
        .find(|m| m.media == "audio")
        .expect("No audio media found in hold SDP");
    
    assert_eq!(hold_media.direction, Some(rvoip_sip_core::sdp::attributes::MediaDirection::SendOnly),
        "Hold SDP should have sendonly direction");
    
    // Reset negotiation state
    sdp_context.reset_for_renegotiation();
    assert_eq!(sdp_context.state, NegotiationState::Initial);
    
    // Update with hold offer
    sdp_context.update_with_local_offer(hold_sdp.clone());
    assert_eq!(sdp_context.state, NegotiationState::OfferSent);
    
    // Create a hold answer
    let hold_answer = sdp::update_sdp_for_reinvite(
        sdp_context.remote_sdp.as_ref().unwrap(),
        None, // Same port
        Some(rvoip_sip_core::sdp::attributes::MediaDirection::RecvOnly) // Hold response
    ).unwrap();
    
    // Verify the hold answer has recvonly direction
    let hold_answer_media = hold_answer.media_descriptions.iter()
        .find(|m| m.media == "audio")
        .expect("No audio media found in hold answer SDP");
    
    assert_eq!(hold_answer_media.direction, Some(rvoip_sip_core::sdp::attributes::MediaDirection::RecvOnly),
        "Hold answer SDP should have recvonly direction");
    
    // Update with hold answer
    sdp_context.update_with_remote_answer(hold_answer.clone());
    assert_eq!(sdp_context.state, NegotiationState::Complete);
    
    println!("SDP Negotiation Test Completed Successfully");
} 