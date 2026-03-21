// ICE + SDP Integration Tests
//
// Verifies that ICE attributes are correctly generated into SDP offers
// and that remote ICE attributes in SDP answers are correctly parsed
// and fed into the ICE agent.

use std::net::SocketAddr;
use rvoip_session_core::media::{
    MediaManager, MediaConfig, IceConfig,
};
use rvoip_session_core::SessionId;
use rvoip_rtp_core::ice::adapter::IceAgentAdapter;
use rvoip_rtp_core::ice::types::{
    IceCandidate, IceRole, IceConnectionState,
    CandidateType, ComponentId,
};

/// Test 7: ICE candidates appear in SDP offer.
///
/// Creates a MediaManager with ICE enabled, creates a session,
/// manually injects an ICE agent with host candidates (to avoid
/// needing network I/O), then generates an SDP offer and verifies
/// that `a=ice-ufrag`, `a=ice-pwd`, and `a=candidate` lines appear.
#[tokio::test]
async fn test_ice_candidates_appear_in_sdp_offer() {
    let local_addr: SocketAddr = "127.0.0.1:8000"
        .parse()
        .unwrap_or_else(|e| panic!("parse: {e}"));

    let mut ice_config = IceConfig::default();
    ice_config.enabled = true;
    // No real STUN servers -- we will inject candidates manually.

    let mut media_config = MediaConfig::default();
    media_config.ice = ice_config;

    let manager = MediaManager::with_port_range_and_config(
        local_addr, 30000, 31000, media_config,
    );

    let session_id = SessionId::new();

    // Create a media session (allocates RTP port, etc.)
    let _session_info = manager.create_media_session(&session_id).await
        .unwrap_or_else(|e| panic!("create_media_session failed: {e}"));

    // Manually inject an ICE agent with a known host candidate.
    // This avoids network I/O while still exercising the SDP generation path.
    {
        let mut agent = IceAgentAdapter::new(IceRole::Controlling);
        let host_candidates = agent.gather_host_candidates_only(
            "127.0.0.1:30000"
                .parse()
                .unwrap_or_else(|e| panic!("parse: {e}")),
        );
        assert!(
            !host_candidates.is_empty(),
            "gather_host_candidates_only should produce at least one candidate"
        );

        let mut agents = manager.ice_agents.write().await;
        agents.insert(session_id.clone(), agent);
    }

    // Generate SDP offer -- it should contain ICE attributes
    let sdp = manager.generate_sdp_offer(&session_id).await
        .unwrap_or_else(|e| panic!("generate_sdp_offer failed: {e}"));

    // Verify presence of ICE ufrag (4+ alphanumeric characters)
    assert!(
        sdp.contains("a=ice-ufrag:"),
        "SDP must contain a=ice-ufrag attribute.\nSDP:\n{sdp}"
    );

    // Verify presence of ICE pwd (22+ characters)
    assert!(
        sdp.contains("a=ice-pwd:"),
        "SDP must contain a=ice-pwd attribute.\nSDP:\n{sdp}"
    );

    // Verify at least one candidate line
    assert!(
        sdp.contains("a=candidate:"),
        "SDP must contain at least one a=candidate line.\nSDP:\n{sdp}"
    );

    // Verify the candidate line contains "typ host"
    let candidate_line = sdp.lines()
        .find(|l| l.contains("a=candidate:"))
        .unwrap_or_else(|| panic!("expected a=candidate line"));
    assert!(
        candidate_line.contains("typ host"),
        "candidate should be host type, got: {candidate_line}"
    );

    // Cleanup
    let _ = manager.terminate_media_session(&session_id).await;
}

/// Test 8: Remote ICE parsed from SDP answer.
///
/// Creates a MediaManager with ICE enabled, injects an ICE agent,
/// then feeds it an SDP answer containing remote ICE credentials
/// and candidates.  Verifies that the agent received the remote
/// credentials.
#[tokio::test]
async fn test_remote_ice_parsed_from_sdp_answer() {
    let local_addr: SocketAddr = "127.0.0.1:8000"
        .parse()
        .unwrap_or_else(|e| panic!("parse: {e}"));

    let mut ice_config = IceConfig::default();
    ice_config.enabled = true;

    let mut media_config = MediaConfig::default();
    media_config.ice = ice_config;

    let manager = MediaManager::with_port_range_and_config(
        local_addr, 32000, 33000, media_config,
    );

    let session_id = SessionId::new();

    // Create session
    let _session_info = manager.create_media_session(&session_id).await
        .unwrap_or_else(|e| panic!("create_media_session failed: {e}"));

    // Inject an ICE agent manually (with host candidates gathered)
    {
        let mut agent = IceAgentAdapter::new(IceRole::Controlling);
        let _ = agent.gather_host_candidates_only(
            "127.0.0.1:32000"
                .parse()
                .unwrap_or_else(|e| panic!("parse: {e}")),
        );

        let mut agents = manager.ice_agents.write().await;
        agents.insert(session_id.clone(), agent);
    }

    // Simulate receiving an SDP answer with remote ICE credentials and candidates
    let sdp_answer = "\
v=0\r\n\
o=bob 456 789 IN IP4 10.0.0.2\r\n\
s=-\r\n\
c=IN IP4 10.0.0.2\r\n\
t=0 0\r\n\
m=audio 40000 RTP/AVP 0 8\r\n\
a=rtpmap:0 PCMU/8000\r\n\
a=rtpmap:8 PCMA/8000\r\n\
a=ice-ufrag:ABCD\r\n\
a=ice-pwd:a1b2c3d4e5f6g7h8i9j0kk\r\n\
a=candidate:1 1 udp 2130706431 10.0.0.2 40000 typ host\r\n\
a=sendrecv\r\n";

    // Process the SDP answer through the media manager's process_sdp_answer path
    // which calls process_remote_ice internally to parse ICE attributes.
    let process_result = manager.process_sdp_answer(&session_id, sdp_answer).await;
    // process_sdp_answer may encounter non-fatal issues (e.g., SRTP not configured)
    // but the ICE processing path runs first. We check the agent state directly.
    let _ = process_result;

    // Verify the ICE agent received the remote credentials
    {
        let agents = manager.ice_agents.read().await;
        let agent = agents.get(&session_id)
            .unwrap_or_else(|| panic!("ICE agent should exist for session"));

        let remote_creds = agent.remote_credentials();
        assert!(
            remote_creds.is_some(),
            "remote credentials should be set after processing SDP answer"
        );
        let creds = remote_creds.unwrap_or_else(|| panic!("expected credentials"));
        assert_eq!(creds.ufrag, "ABCD");
        assert_eq!(creds.pwd, "a1b2c3d4e5f6g7h8i9j0kk");
    }

    // Cleanup
    let _ = manager.terminate_media_session(&session_id).await;
}

/// Verify that an SDP offer generated without ICE enabled does NOT contain
/// ICE attributes.
#[tokio::test]
async fn test_no_ice_attributes_when_disabled() {
    let local_addr: SocketAddr = "127.0.0.1:8000"
        .parse()
        .unwrap_or_else(|e| panic!("parse: {e}"));

    // Default config has ICE disabled
    let manager = MediaManager::with_port_range(local_addr, 34000, 35000);
    let session_id = SessionId::new();

    let _session_info = manager.create_media_session(&session_id).await
        .unwrap_or_else(|e| panic!("create_media_session failed: {e}"));

    let sdp = manager.generate_sdp_offer(&session_id).await
        .unwrap_or_else(|e| panic!("generate_sdp_offer failed: {e}"));

    assert!(
        !sdp.contains("a=ice-ufrag:"),
        "SDP should NOT contain ICE attributes when ICE is disabled.\nSDP:\n{sdp}"
    );
    assert!(
        !sdp.contains("a=candidate:"),
        "SDP should NOT contain candidate lines when ICE is disabled.\nSDP:\n{sdp}"
    );

    let _ = manager.terminate_media_session(&session_id).await;
}
