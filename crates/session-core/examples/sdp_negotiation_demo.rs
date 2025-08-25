//! SDP Negotiation Demo
//!
//! This example demonstrates how client-core and call-engine can use
//! session-core's SDP negotiation with media preferences.

use rvoip_session_core::api::*;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

/// Example call handler that defers incoming calls for async processing
#[derive(Debug)]
struct DeferringHandler {
    tx: tokio::sync::mpsc::Sender<IncomingCall>,
}

#[async_trait::async_trait]
impl CallHandler for DeferringHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        // Store the call for async processing
        let _ = self.tx.send(call).await;
        CallDecision::Defer
    }
    
    async fn on_call_established(&self, call: CallSession, local_sdp: Option<String>, remote_sdp: Option<String>) {
        println!("Call established: {}", call.id());
        if let Some(sdp) = local_sdp {
            println!("Local SDP: {} bytes", sdp.len());
        }
        if let Some(sdp) = remote_sdp {
            println!("Remote SDP: {} bytes", sdp.len());
        }
    }
    
    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        println!("Call ended: {} - {}", call.id(), reason);
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    // Create a channel for deferred call processing
    let (tx, mut rx) = tokio::sync::mpsc::channel(100);
    
    // Build session coordinator with media preferences
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(5060)
        .with_local_address("sip:demo@localhost:5060")
        .with_media_config(MediaConfig {
            // Prefer Opus and G722 over PCMU/PCMA
            preferred_codecs: vec![
                "opus".to_string(),
                "G722".to_string(), 
                "PCMU".to_string(),
                "PCMA".to_string(),
            ],
            dtmf_support: true,
            echo_cancellation: true,
            noise_suppression: true,
            auto_gain_control: true,
            max_bandwidth_kbps: Some(64),
            preferred_ptime: Some(20),
            ..Default::default()
        })
        .with_handler(Arc::new(DeferringHandler { tx }))
        .build()
        .await?;
    
    println!("Session coordinator started on {}", coordinator.get_bound_address());
    
    // Spawn task to process deferred calls
    let coordinator_clone = coordinator.clone();
    tokio::spawn(async move {
        while let Some(call) = rx.recv().await {
            println!("\nProcessing deferred call from {} to {}", call.from, call.to);
            
            // Check if caller is authorized (async database lookup, etc.)
            sleep(Duration::from_millis(100)).await; // Simulate async work
            
            if let Some(their_offer) = &call.sdp {
                println!("Received SDP offer: {} bytes", their_offer.len());
                
                // Generate SDP answer based on our media preferences
                match MediaControl::generate_sdp_answer(&coordinator_clone, &call.id, their_offer).await {
                    Ok(our_answer) => {
                        println!("Generated SDP answer: {} bytes", our_answer.len());
                        
                        // Accept the call with our answer
                        match SessionControl::accept_incoming_call(
                            &coordinator_clone,
                            &call,
                            Some(our_answer)
                        ).await {
                            Ok(_) => println!("Call accepted successfully"),
                            Err(e) => tracing::error!("Failed to accept call: {}", e),
                        }
                        
                        // After negotiation, check what was actually negotiated
                        sleep(Duration::from_millis(500)).await; // Wait for negotiation
                        
                        if let Ok(Some(media_info)) = MediaControl::get_media_info(
                            &coordinator_clone,
                            &call.id
                        ).await {
                            println!("\n=== Negotiated Media Configuration ===");
                            if let Some(codec) = media_info.codec {
                                println!("Codec: {}", codec);
                            }
                            if let Some(local_port) = media_info.local_rtp_port {
                                println!("Local RTP port: {}", local_port);
                            }
                            if let Some(remote_port) = media_info.remote_rtp_port {
                                println!("Remote RTP port: {}", remote_port);
                            }
                            if let Some(local_sdp) = &media_info.local_sdp {
                                println!("Local SDP: {} bytes", local_sdp.len());
                            }
                            if let Some(remote_sdp) = &media_info.remote_sdp {
                                println!("Remote SDP: {} bytes", remote_sdp.len());
                            }
                            println!("=====================================\n");
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to generate SDP answer: {}", e);
                        let _ = SessionControl::reject_incoming_call(
                            &coordinator_clone,
                            &call,
                            "SDP negotiation failed"
                        ).await;
                    }
                }
            } else {
                // No SDP in offer - reject
                let _ = SessionControl::reject_incoming_call(
                    &coordinator_clone,
                    &call,
                    "No SDP in INVITE"
                ).await;
            }
        }
    });
    
    // Also demonstrate outgoing call with SDP negotiation
    println!("\nMaking outgoing call with media preferences...");
    
    let outgoing = SessionControl::create_outgoing_call(
        &coordinator,
        "sip:demo@localhost:5060",
        "sip:target@example.com:5060",
        None  // SDP will be generated based on preferences
    ).await?;
    
    println!("Outgoing call created: {}", outgoing.id());
    
    // Simulate running for a while
    println!("\nListening for incoming calls. Press Ctrl-C to stop.");
    tokio::signal::ctrl_c().await?;
    
    // Clean shutdown
    SessionControl::stop(&coordinator).await?;
    println!("Shutdown complete");
    
    Ok(())
} 