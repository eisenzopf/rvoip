//! SDP tests for session-core public API
//!
//! This file contains tests for SDP functionality through the public
//! session-core API rather than internal implementation details.

use std::sync::Arc;
use anyhow::Result;

use rvoip_session_core::sdp::{
    SdpContext, NegotiationState, extract_media_config_from_sdp,
    create_default_media_config, update_sdp_for_reinvite
};

#[test]
fn test_sdp_context_workflow() -> Result<()> {
    let mut context = SdpContext::new();
    
    // Create test SDP
    let sdp = rvoip_sip_core::sdp::SdpBuilder::new("Test Session")
        .origin("-", "123456", "1", "IN", "IP4", "127.0.0.1")
        .connection("IN", "IP4", "127.0.0.1")
        .time("0", "0")
        .media_audio(10000, "RTP/AVP")
            .formats(&["0"])
            .rtpmap("0", "PCMU/8000")
            .done()
        .build()?;
    
    // Test offer-answer flow
    context.set_local_offer(sdp.clone())?;
    assert_eq!(context.state, NegotiationState::OfferSent);
    
    context.set_remote_answer(sdp.clone())?;
    assert_eq!(context.state, NegotiationState::Complete);
    assert!(context.is_complete());
    
    Ok(())
}

#[test]
fn test_media_config_extraction() -> Result<()> {
    // Create test SDP with media info
    let sdp = rvoip_sip_core::sdp::SdpBuilder::new("Media Test")
        .origin("-", "123456", "1", "IN", "IP4", "192.168.1.100")
        .connection("IN", "IP4", "192.168.1.100")
        .time("0", "0")
        .media_audio(12000, "RTP/AVP")
            .formats(&["0", "8"])
            .rtpmap("0", "PCMU/8000")
            .rtpmap("8", "PCMA/8000")
            .direction(rvoip_sip_core::sdp::attributes::MediaDirection::SendRecv)
            .done()
        .build()?;
    
    let local_addr = "127.0.0.1:10000".parse().unwrap();
    let media_config = extract_media_config_from_sdp(&sdp, local_addr, false)?;
    
    assert_eq!(media_config.local_addr, local_addr);
    assert!(media_config.remote_addr.is_some());
    assert_eq!(media_config.payload_type, 0); // First format (PCMU)
    assert_eq!(media_config.clock_rate, 8000);
    
    Ok(())
}

#[test]
fn test_sdp_renegotiation() -> Result<()> {
    // Create initial SDP
    let initial_sdp = rvoip_sip_core::sdp::SdpBuilder::new("Initial")
        .origin("-", "123456", "1", "IN", "IP4", "127.0.0.1")
        .connection("IN", "IP4", "127.0.0.1")
        .time("0", "0")
        .media_audio(10000, "RTP/AVP")
            .formats(&["0"])
            .rtpmap("0", "PCMU/8000")
            .done()
        .build()?;
    
    let media_config = create_default_media_config("127.0.0.1:10000".parse().unwrap());
    
    // Test SDP update for re-invite
    let updated_sdp = update_sdp_for_reinvite(&initial_sdp, &media_config)?;
    
    // Verify it's a valid SDP
    assert!(!updated_sdp.session_name.is_empty());
    assert!(!updated_sdp.media_descriptions.is_empty());
    
    Ok(())
} 