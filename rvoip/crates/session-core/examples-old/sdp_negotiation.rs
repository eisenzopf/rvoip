use std::net::IpAddr;
use std::str::FromStr;
use std::time::Duration;
use tokio::time::sleep;

use rvoip_session_core::{
    media::{AudioCodecType, MediaConfig},
    sdp,
};

use rvoip_sip_core::sdp::attributes::MediaDirection;

/// A simulated RTP media handler that can be used to demonstrate
/// how to handle media after SDP negotiation is complete
struct MediaHandler {
    local_config: Option<MediaConfig>,
    active: bool,
}

impl MediaHandler {
    fn new() -> Self {
        Self {
            local_config: None,
            active: false,
        }
    }
    
    fn configure_media(&mut self, config: MediaConfig) {
        println!("🔊 Configuring media with:");
        println!("   - Local endpoint: {}", config.local_addr);
        println!("   - Remote endpoint: {:?}", config.remote_addr);
        println!("   - Audio codec: {:?} (PT: {}, Rate: {}Hz)", 
                config.audio_codec, config.payload_type, config.clock_rate);
        
        self.local_config = Some(config);
    }
    
    fn start(&mut self) {
        if self.local_config.is_some() {
            self.active = true;
            println!("▶️ Media started");
        } else {
            println!("❌ Cannot start media: no configuration");
        }
    }
    
    fn stop(&mut self) {
        if self.active {
            self.active = false;
            println!("⏹️ Media stopped");
        }
    }
    
    fn place_on_hold(&mut self) {
        if self.active {
            println!("⏸️ Call placed on hold");
        }
    }
    
    fn resume(&mut self) {
        if self.active {
            println!("▶️ Call resumed from hold");
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Starting SDP negotiation example");
    
    // Create a media handler for this example
    let mut media_handler = MediaHandler::new();
    
    // Simulate creating a dialog with SDP offer/answer
    println!("🔄 Simulating SDP offer/answer exchange...");
    
    // Create an SDP offer
    let local_addr = IpAddr::from_str("192.168.1.1")?;
    let local_port = 10000;
    let supported_codecs = vec![AudioCodecType::PCMU, AudioCodecType::PCMA];
    
    let sdp_offer = sdp::create_audio_offer(
        local_addr,
        local_port,
        &supported_codecs
    )?;
    
    println!("📤 Created SDP offer with codecs: {:?}", supported_codecs);
    println!("\nOffer SDP:\n{}\n", sdp_offer);
    
    // Simulate receiving SDP answer from remote
    let remote_addr = IpAddr::from_str("192.168.2.1")?;
    let remote_port = 20000;
    let remote_supported_codecs = vec![AudioCodecType::PCMU]; // Only support one codec
    
    let sdp_answer = sdp::create_audio_answer(
        &sdp_offer,
        remote_addr,
        remote_port,
        &remote_supported_codecs
    )?;
    
    println!("📥 Received SDP answer with codecs: {:?}", remote_supported_codecs);
    println!("\nAnswer SDP:\n{}\n", sdp_answer);
    
    // Extract media config from the negotiated SDP
    let media_config = sdp::extract_media_config(&sdp_offer, &sdp_answer)?;
    
    // Configure and start media
    media_handler.configure_media(media_config);
    media_handler.start();
    
    // Simulate active call for a few seconds
    println!("☎️ Call is now active...");
    sleep(Duration::from_secs(3)).await;
    
    // Simulate putting call on hold (send re-INVITE with SDP sendonly)
    println!("⏸️ Putting call on hold...");
    
    // Create a new SDP offer for hold
    let hold_sdp = sdp::update_sdp_for_reinvite(
        &sdp_offer,
        None, // Same port
        Some(MediaDirection::SendOnly) // Hold
    )?;
    
    println!("📤 Sent re-INVITE with hold SDP (sendonly)");
    println!("\nHold Offer SDP:\n{}\n", hold_sdp);
    
    // Simulate receiving hold response SDP (recvonly)
    let hold_answer = sdp::update_sdp_for_reinvite(
        &sdp_answer,
        None, // Same port
        Some(MediaDirection::RecvOnly) // Hold response
    )?;
    
    println!("📥 Received SDP answer for hold (recvonly)");
    println!("\nHold Answer SDP:\n{}\n", hold_answer);
    
    // Update the media handler with hold state
    media_handler.place_on_hold();
    
    // Simulate call on hold for a few seconds
    sleep(Duration::from_secs(3)).await;
    
    // Simulate resuming call (send re-INVITE with SDP sendrecv)
    println!("▶️ Resuming call...");
    
    // Create a new SDP offer for resume
    let resume_sdp = sdp::update_sdp_for_reinvite(
        &hold_sdp,
        None, // Same port
        Some(MediaDirection::SendRecv) // Resume
    )?;
    
    println!("📤 Sent re-INVITE with resume SDP (sendrecv)");
    println!("\nResume Offer SDP:\n{}\n", resume_sdp);
    
    // Simulate receiving resume response SDP
    let resume_answer = sdp::update_sdp_for_reinvite(
        &hold_answer,
        None, // Same port
        Some(MediaDirection::SendRecv) // Resume response
    )?;
    
    println!("📥 Received SDP answer for resume (sendrecv)");
    println!("\nResume Answer SDP:\n{}\n", resume_answer);
    
    // Update the media handler with resume state
    media_handler.resume();
    
    // Simulate active call for a few more seconds
    sleep(Duration::from_secs(3)).await;
    
    // End the call
    println!("📞 Ending call...");
    media_handler.stop();
    
    println!("✅ SDP negotiation example completed successfully");
    
    Ok(())
} 