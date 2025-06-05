//! Helper tests for session-core public API
//!
//! This file contains tests for helper functions and public API methods
//! that are part of the session-core interface.

use std::sync::Arc;
use anyhow::Result;

use rvoip_session_core::sdp::{SdpContext, NegotiationState, create_default_media_config};
use rvoip_session_core::media::SessionMediaType;

#[test]
fn test_sdp_context_creation() {
    let context = SdpContext::new();
    assert_eq!(context.state, NegotiationState::Initial);
    assert!(context.local_sdp.is_none());
    assert!(context.remote_sdp.is_none());
    assert!(context.media_config.is_none());
}

#[test]
fn test_sdp_context_state_transitions() {
    let mut context = SdpContext::new();
    
    // Test initial state
    assert_eq!(context.state, NegotiationState::Initial);
    assert!(!context.is_complete());
    
    // Create a simple SDP session for testing
    let sdp_builder = rvoip_sip_core::sdp::SdpBuilder::new("Test Session")
        .origin("-", "123456", "1", "IN", "IP4", "127.0.0.1")
        .connection("IN", "IP4", "127.0.0.1")
        .time("0", "0")
        .media_audio(10000, "RTP/AVP")
            .formats(&["0"])
            .rtpmap("0", "PCMU/8000")
            .done();
    
    let sdp = sdp_builder.build().unwrap();
    
    // Test setting local offer
    context.set_local_offer(sdp.clone()).unwrap();
    assert_eq!(context.state, NegotiationState::OfferSent);
    assert!(context.local_sdp.is_some());
    
    // Reset and test remote offer
    context.reset_for_renegotiation();
    context.set_remote_offer(sdp.clone()).unwrap();
    assert_eq!(context.state, NegotiationState::OfferReceived);
    assert!(context.remote_sdp.is_some());
    
    // Test completing negotiation
    context.set_local_answer(sdp.clone()).unwrap();
    assert_eq!(context.state, NegotiationState::Complete);
    assert!(context.is_complete());
}

#[test] 
fn test_default_media_config_creation() {
    let local_addr = "127.0.0.1:10000".parse().unwrap();
    let config = create_default_media_config(local_addr);
    
    assert_eq!(config.local_addr, local_addr);
    assert!(config.remote_addr.is_none());
    assert_eq!(config.media_type, SessionMediaType::Audio);
    assert_eq!(config.payload_type, 0); // PCMU
    assert_eq!(config.clock_rate, 8000);
}

#[test]
fn test_negotiation_state_display() {
    assert_eq!(format!("{:?}", NegotiationState::Initial), "Initial");
    assert_eq!(format!("{:?}", NegotiationState::OfferSent), "OfferSent");
    assert_eq!(format!("{:?}", NegotiationState::OfferReceived), "OfferReceived");
    assert_eq!(format!("{:?}", NegotiationState::Complete), "Complete");
} 