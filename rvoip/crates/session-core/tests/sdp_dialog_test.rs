use std::sync::Arc;
use std::net::IpAddr;
use std::str::FromStr;

use rvoip_session_core::{
    dialog::{DialogManager, DialogId},
    media::AudioCodecType,
    session::{SessionManager, SessionId, SessionConfig},
    events::EventBus,
    sdp::{self, SessionDescription},
    helpers
};

use rvoip_transaction_core::{TransactionManager, TransactionKey};
use rvoip_sip_core::{
    Request, Response, Method, StatusCode, Uri, TypedHeader, Address, From
};
use rvoip_sip_core::types::from::From as FromHeaderType;
use rvoip_sip_core::types::to::To as ToHeader;
use rvoip_sip_core::types::call_id::CallId;
use rvoip_sip_core::types::content_type::ContentType;
use rvoip_sip_core::types::cseq::CSeq;
use rvoip_sip_core::types::via::Via;
use rvoip_sip_core::types::contact::Contact;
use rvoip_sip_core::prelude::Param;

use tokio::sync::mpsc;
use bytes::Bytes;
use std::time::Duration;

// Create a mock transport for testing
#[derive(Debug)]
struct MockTransport {
    local_addr: std::net::SocketAddr,
}

impl MockTransport {
    fn new(addr: &str) -> Self {
        Self {
            local_addr: std::net::SocketAddr::from_str(addr).unwrap(),
        }
    }
}

#[async_trait::async_trait]
impl rvoip_sip_transport::Transport for MockTransport {
    async fn send_message(
        &self,
        _message: rvoip_sip_core::Message,
        _destination: std::net::SocketAddr,
    ) -> std::result::Result<(), rvoip_sip_transport::Error> {
        Ok(()) // Just pretend we sent it
    }

    fn local_addr(&self) -> std::result::Result<std::net::SocketAddr, rvoip_sip_transport::Error> {
        Ok(self.local_addr)
    }

    async fn close(&self) -> std::result::Result<(), rvoip_sip_transport::Error> {
        Ok(())
    }

    fn is_closed(&self) -> bool {
        false
    }
}

#[tokio::test]
async fn test_sdp_integration_with_dialog() {
    // Setup transaction manager and dialog manager
    let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
    let (transport_tx, transport_rx) = mpsc::channel(100);
    
    let (transaction_manager, _) = TransactionManager::new(transport.clone(), transport_rx, None).await.unwrap();
    let transaction_manager = Arc::new(transaction_manager);
    
    let event_bus = EventBus::new(100);
    let dialog_manager = Arc::new(DialogManager::new(transaction_manager.clone(), event_bus.clone()));
    
    // Create session config
    let session_config = SessionConfig::default();
    
    // Create a session manager
    let session_manager = Arc::new(SessionManager::new(
        transaction_manager.clone(),
        session_config,
        event_bus.clone()
    ));
    
    // Start the dialog manager
    let _transaction_events = dialog_manager.start().await;
    
    // Create a test INVITE request with SDP offer
    let uri = Uri::from_str("sip:bob@example.com").unwrap();
    let mut invite_request = Request::new(Method::Invite, uri.clone());
    
    // Add required headers
    let alice_addr = Address::new(Uri::from_str("sip:alice@example.com").unwrap());
    let bob_addr = Address::new(Uri::from_str("sip:bob@example.com").unwrap());
    
    invite_request.headers.push(TypedHeader::From(
        From(alice_addr.clone())
    ));
    invite_request.headers.push(TypedHeader::To(
        ToHeader::new(bob_addr.clone())
    ));
    invite_request.headers.push(TypedHeader::CallId(
        CallId::new("test-call-id-123")
    ));
    invite_request.headers.push(TypedHeader::CSeq(
        CSeq::new(1, Method::Invite)
    ));
    
    // Creating a Via header with the updated signature
    let via_params = vec![
        Param::new("branch".to_string(), Some("z9hG4bK123".to_string())),
    ];
    
    let via = Via::new(
        "SIP/2.0".to_string(), 
        "UDP".to_string(), 
        "192.168.1.1".to_string(), 
        "5060".to_string(), 
        None,
        via_params
    ).expect("Failed to create Via header");
    
    invite_request.headers.push(TypedHeader::Via(via.clone()));
    invite_request.headers.push(TypedHeader::Contact(
        Contact::from_str("<sip:alice@192.168.1.1:5060>").unwrap()
    ));
    
    // Create an SDP offer
    let local_addr = IpAddr::from_str("192.168.1.1").unwrap();
    let local_port = 10000;
    let supported_codecs = vec![AudioCodecType::PCMU, AudioCodecType::PCMA];
    let sdp_offer = sdp::create_audio_offer(
        local_addr,
        local_port,
        &supported_codecs
    ).unwrap();
    
    // Add SDP body to INVITE
    invite_request.body = Bytes::from(sdp_offer.to_string().into_bytes());
    invite_request.headers.push(TypedHeader::ContentType(
        ContentType::from_str("application/sdp").unwrap()
    ));
    
    // Create a test 200 OK response with SDP answer
    let mut ok_response = Response::new(StatusCode::Ok);
    
    // Add required headers
    // Create the From header with tag
    let mut alice_addr_with_tag = alice_addr.clone();
    alice_addr_with_tag.set_tag("alice-tag-123".to_string());
    ok_response.headers.push(TypedHeader::From(
        From(alice_addr_with_tag)
    ));
    
    let to_with_tag = ToHeader::new(bob_addr.clone())
        .with_tag("bob-tag-456".to_string());
    ok_response.headers.push(TypedHeader::To(to_with_tag));
    
    ok_response.headers.push(TypedHeader::CallId(
        CallId::new("test-call-id-123")
    ));
    ok_response.headers.push(TypedHeader::CSeq(
        CSeq::new(1, Method::Invite)
    ));
    
    // Create the same Via header for the response
    ok_response.headers.push(TypedHeader::Via(via));
    ok_response.headers.push(TypedHeader::Contact(
        Contact::from_str("<sip:bob@192.168.2.1:5060>").unwrap()
    ));
    
    // Create an SDP answer
    let remote_addr = IpAddr::from_str("192.168.2.1").unwrap();
    let remote_port = 20000;
    let remote_supported_codecs = vec![AudioCodecType::PCMU]; // Only supporting one codec
    let sdp_answer = sdp::create_audio_answer(
        &sdp_offer,
        remote_addr,
        remote_port,
        &remote_supported_codecs
    ).unwrap();
    
    // Add SDP body to response
    ok_response.body = Bytes::from(sdp_answer.to_string().into_bytes());
    ok_response.headers.push(TypedHeader::ContentType(
        ContentType::from_str("application/sdp").unwrap()
    ));
    
    // Create a Transaction Key
    let transaction_key = TransactionKey::new(
        "test-transaction-id".to_string(),
        Method::Invite,
        false // client transaction
    );
    
    // Create the dialog from the request and response
    let dialog_id_opt = dialog_manager.create_dialog_from_transaction(
        &transaction_key, 
        &invite_request, 
        &ok_response, 
        true // We're the initiator (UAC)
    ).await;
    
    assert!(dialog_id_opt.is_some(), "Failed to create dialog");
    let dialog_id = dialog_id_opt.unwrap();
    
    // Associate with a session
    let session_id = SessionId::new();
    dialog_manager.associate_with_session(&dialog_id, &session_id).unwrap();
    
    // Get the dialog
    let dialog = dialog_manager.get_dialog(&dialog_id).unwrap();
    
    // Verify SDP context is complete
    assert_eq!(dialog.sdp_context.state, sdp::NegotiationState::Complete, 
        "SDP negotiation should be complete");
    
    // Verify local and remote SDP are set
    assert!(dialog.sdp_context.local_sdp.is_some(), "Local SDP should be set");
    assert!(dialog.sdp_context.remote_sdp.is_some(), "Remote SDP should be set");
    
    // Get media config from the dialog
    let media_config = helpers::get_dialog_media_config(&dialog_manager, &dialog_id).unwrap();
    
    // Verify media config
    assert_eq!(media_config.local_addr.ip(), local_addr);
    assert_eq!(media_config.local_addr.port(), local_port);
    assert_eq!(media_config.remote_addr.unwrap().ip(), remote_addr);
    assert_eq!(media_config.remote_addr.unwrap().port(), remote_port);
    assert_eq!(media_config.audio_codec, AudioCodecType::PCMU); // The negotiated codec
    
    // Test SDP offer for re-INVITE (e.g., for hold)
    // First, prepare for renegotiation
    dialog_manager.prepare_dialog_sdp_renegotiation(&dialog_id).await.unwrap();
    
    // Get dialog again
    let dialog = dialog_manager.get_dialog(&dialog_id).unwrap();
    
    // Verify negotiation state is reset
    assert_eq!(dialog.sdp_context.state, sdp::NegotiationState::Initial,
        "Negotiation state should be reset");
        
    // Create a new SDP offer for hold (sendonly)
    let new_sdp_offer = sdp::update_sdp_for_reinvite(
        dialog.sdp_context.local_sdp.as_ref().unwrap(),
        None, // Same port
        Some(rvoip_sip_core::sdp::attributes::MediaDirection::SendOnly) // Hold
    ).unwrap();
    
    // Update dialog with new SDP offer
    dialog_manager.update_dialog_with_local_sdp_offer(&dialog_id, new_sdp_offer.clone()).await.unwrap();
    
    // Get dialog again
    let dialog = dialog_manager.get_dialog(&dialog_id).unwrap();
    
    // Verify negotiation state is OfferSent
    assert_eq!(dialog.sdp_context.state, sdp::NegotiationState::OfferSent,
        "Negotiation state should be OfferSent");
        
    // Create a response SDP answer (recvonly for hold)
    let new_sdp_answer = sdp::update_sdp_for_reinvite(
        dialog.sdp_context.remote_sdp.as_ref().unwrap(),
        None, // Same port
        Some(rvoip_sip_core::sdp::attributes::MediaDirection::RecvOnly) // Hold response
    ).unwrap();
    
    // Simulate receiving the answer
    let mut dialog = dialog_manager.get_dialog(&dialog_id).unwrap();
    dialog.sdp_context.update_with_remote_answer(new_sdp_answer);
    
    // Verify negotiation state is Complete
    assert_eq!(dialog.sdp_context.state, sdp::NegotiationState::Complete,
        "Negotiation state should be Complete after receiving answer");
        
    // Verify the local SDP direction is sendonly (hold)
    if let Some(local_sdp) = &dialog.sdp_context.local_sdp {
        let media = &local_sdp.media_descriptions[0];
        assert_eq!(media.direction, Some(rvoip_sip_core::sdp::attributes::MediaDirection::SendOnly),
            "Local media direction should be sendonly (hold)");
    } else {
        panic!("Local SDP is missing");
    }
    
    // Verify the remote SDP direction is recvonly (hold response)
    if let Some(remote_sdp) = &dialog.sdp_context.remote_sdp {
        let media = &remote_sdp.media_descriptions[0];
        assert_eq!(media.direction, Some(rvoip_sip_core::sdp::attributes::MediaDirection::RecvOnly),
            "Remote media direction should be recvonly (hold response)");
    } else {
        panic!("Remote SDP is missing");
    }
} 