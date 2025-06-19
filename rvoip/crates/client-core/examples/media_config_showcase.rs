//! Showcase of media configuration options in client-core
//! 
//! This example demonstrates all the ways to configure media preferences
//! that will be passed to session-core for SDP generation.

use std::collections::HashMap;

fn main() {
    println!("=== Media Configuration Showcase ===\n");
    
    // Show the default MediaConfig
    println!("1. Default MediaConfig:");
    println!("   preferred_codecs: [\"PCMU\", \"PCMA\"]");
    println!("   echo_cancellation: true");
    println!("   noise_suppression: true");
    println!("   auto_gain_control: true");
    println!("   dtmf_enabled: true");
    println!("   max_bandwidth_kbps: None");
    println!("   preferred_ptime: Some(20)");
    println!("   custom_sdp_attributes: {}");
    
    // Show MediaConfigBuilder usage
    println!("\n2. Using MediaConfigBuilder:");
    println!("   MediaConfigBuilder::new()");
    println!("       .codecs(vec![\"opus\", \"G722\", \"PCMU\"])");
    println!("       .echo_cancellation(true)");
    println!("       .noise_suppression(true)");
    println!("       .auto_gain_control(false)");
    println!("       .dtmf(true)");
    println!("       .max_bandwidth_kbps(256)");
    println!("       .ptime(30)");
    println!("       .custom_attribute(\"a=tool\", \"rvoip\")");
    println!("       .build()");
    
    // Show ClientBuilder integration
    println!("\n3. ClientBuilder with media config:");
    println!("   ClientBuilder::new()");
    println!("       .local_address(\"127.0.0.1:5060\".parse()?)");
    println!("       .with_media(|m| m");
    println!("           .codecs(vec![\"opus\", \"PCMU\"])");
    println!("           .audio_processing(true)");
    println!("       )");
    println!("       .build()");
    println!("       .await?");
    
    // Show presets
    println!("\n4. Media Presets:");
    println!("   - VoiceOptimized: Standard codecs, echo/noise suppression");
    println!("   - HighQuality: Premium codecs (opus, G722), all processing");
    println!("   - LowBandwidth: Compressed codecs, minimal processing");
    println!("   - Compatibility: Maximum codec support for interop");
    
    // Show how it flows to session-core
    println!("\n5. Flow to session-core:");
    println!("   client MediaConfig → SessionMediaConfig → SessionManagerBuilder");
    println!("   ↓");
    println!("   SessionCoordinator uses preferences for:");
    println!("   - SDP offer generation (outgoing calls)");
    println!("   - SDP answer generation (incoming calls)");
    println!("   - Codec negotiation");
    println!("   - Media session configuration");
    
    // Show example SDP output
    println!("\n6. Example SDP with opus preference:");
    println!("   m=audio 10000 RTP/AVP 111 0 8 101");
    println!("   a=rtpmap:111 opus/48000/2");
    println!("   a=rtpmap:0 PCMU/8000");
    println!("   a=rtpmap:8 PCMA/8000");
    println!("   a=rtpmap:101 telephone-event/8000");
    println!("   a=ptime:20");
    
    println!("\n=== Benefits ===");
    println!("• Configure once, use everywhere");
    println!("• Automatic codec negotiation");
    println!("• Consistent media handling");
    println!("• No manual SDP manipulation");
    println!("• Clean API separation");
} 