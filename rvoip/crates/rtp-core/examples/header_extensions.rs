//! RTP Header Extensions Example
//!
//! This example demonstrates how to use RTP header extensions as defined in RFC 8285.
//! It shows both one-byte and two-byte header formats and demonstrates how to
//! add, parse, and manipulate header extensions.

use bytes::Bytes;
use tracing::{info, debug};

use rvoip_rtp_core::{
    RtpHeader, RtpPacket, 
    packet::extension::{
        ExtensionFormat, RtpHeaderExtensions,
        ids::{AUDIO_LEVEL, VIDEO_ORIENTATION, TRANSPORT_CC}
    }
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    
    info!("RTP Header Extensions Example");
    info!("=============================");
    
    // Create a packet with one-byte header extensions
    info!("Creating packet with one-byte header extensions");
    let packet_with_one_byte_ext = create_packet_with_one_byte_extensions()?;
    
    // Create a packet with two-byte header extensions
    info!("Creating packet with two-byte header extensions");
    let packet_with_two_byte_ext = create_packet_with_two_byte_extensions()?;
    
    // Demonstrate parsing extensions from an existing packet
    info!("Parsing header extensions from serialized packets");
    parse_and_display_extensions(&packet_with_one_byte_ext, "one-byte")?;
    parse_and_display_extensions(&packet_with_two_byte_ext, "two-byte")?;
    
    // Demonstrate using the convenience methods on RtpHeader
    info!("Using convenience methods for header extensions");
    demonstrate_header_convenience_methods()?;
    
    // Demonstrate a real-world example: audio levels in VoIP
    info!("Real-world example: Audio level indication");
    demonstrate_audio_level_extension()?;
    
    // Demonstrate another real-world example: video orientation
    info!("Real-world example: Video orientation");
    demonstrate_video_orientation_extension()?;
    
    info!("Header extensions example completed successfully");
    
    Ok(())
}

/// Create an RTP packet with one-byte header extensions
fn create_packet_with_one_byte_extensions() -> Result<RtpPacket, Box<dyn std::error::Error>> {
    // Create a basic RTP header
    let mut header = RtpHeader::new(96, 1000, 48000, 0x12345678);
    
    // Create one-byte header extensions
    let mut extensions = RtpHeaderExtensions::new_one_byte();
    
    // Add some extensions
    // Audio level extension (RFC 6464) - ID 1
    // Format: [0] = voice activity flag + level (7 bits)
    let vad_flag = true;                // Voice activity detected
    let level = 40;                     // 40 dB below full scale
    let audio_level_data = vec![(if vad_flag { 0x80 } else { 0x00 }) | (level & 0x7F)];
    extensions.add_extension(AUDIO_LEVEL, audio_level_data)?;
    
    // Transport-wide congestion control (TWCC) - ID 4
    // Format: [0-1] = transport sequence number
    let twcc_seq = 0x1234;
    let twcc_data = vec![(twcc_seq >> 8) as u8, twcc_seq as u8];
    extensions.add_extension(TRANSPORT_CC, twcc_data)?;
    
    // Set the extensions in the header
    header.extensions = Some(extensions);
    header.extension = true;
    
    // Create payload and packet
    let payload = Bytes::from_static(b"Packet with one-byte extensions");
    let packet = RtpPacket::new(header, payload);
    
    info!("Created packet with one-byte extensions");
    info!("  - Audio level: VAD={}, level={}dB", vad_flag, level);
    info!("  - TWCC sequence number: {}", twcc_seq);
    
    Ok(packet)
}

/// Create an RTP packet with two-byte header extensions
fn create_packet_with_two_byte_extensions() -> Result<RtpPacket, Box<dyn std::error::Error>> {
    // Create a basic RTP header
    let mut header = RtpHeader::new(96, 2000, 90000, 0x87654321);
    
    // Create two-byte header extensions
    let mut extensions = RtpHeaderExtensions::new_two_byte();
    
    // Add some extensions
    // Video orientation (RFC 7742) - ID 3
    // Format: [0] = Camera orientation bits
    // Bits: C F R R (Camera flipped, Front-facing, Rotation 00=0, 01=90, 10=180, 11=270)
    let is_front_camera = true;
    let is_flipped = false;
    let rotation = 1; // 90 degrees
    let voi_data = vec![(if is_front_camera { 0x40 } else { 0x00 }) | 
                   (if is_flipped { 0x80 } else { 0x00 }) | 
                   ((rotation & 0x03) << 4)];
    extensions.add_extension(VIDEO_ORIENTATION, voi_data)?;
    
    // Longer extension (demonstrating two-byte format's ability to handle longer data)
    let long_data: Vec<u8> = (0..32).collect(); // 32 bytes is > 16, so needs two-byte format
    extensions.add_extension(10, long_data)?;
    
    // Set the extensions in the header
    header.extensions = Some(extensions);
    header.extension = true;
    
    // Create payload and packet
    let payload = Bytes::from_static(b"Packet with two-byte extensions");
    let packet = RtpPacket::new(header, payload);
    
    info!("Created packet with two-byte extensions");
    info!("  - Video orientation: front={}, flipped={}, rotation={}°", 
           is_front_camera, is_flipped, rotation * 90);
    info!("  - Custom extension (ID 10) with {} bytes", 32);
    
    Ok(packet)
}

/// Parse extensions from an existing packet and display them
fn parse_and_display_extensions(packet: &RtpPacket, format_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Serialize the packet
    let serialized = packet.serialize()?;
    
    // Parse it back
    let parsed = RtpPacket::parse(&serialized)?;
    
    info!("Parsed {} extensions from packet:", format_name);
    
    // Display extensions if present
    if let Some(extensions) = &parsed.header.extensions {
        info!("  - Extension format: {:?}", extensions.format);
        info!("  - Extension count: {}", extensions.len());
        
        for element in &extensions.elements {
            info!("  - Extension ID {}: {} bytes", element.id, element.data.len());
            
            // Interpret some common extensions
            match element.id {
                AUDIO_LEVEL => {
                    if element.data.len() >= 1 {
                        let vad = (element.data[0] & 0x80) != 0;
                        let level = element.data[0] & 0x7F;
                        info!("    Audio level: VAD={}, level={}dB", vad, level);
                    }
                },
                VIDEO_ORIENTATION => {
                    if element.data.len() >= 1 {
                        let flipped = (element.data[0] & 0x80) != 0;
                        let front_facing = (element.data[0] & 0x40) != 0;
                        let rotation = ((element.data[0] >> 4) & 0x03) * 90;
                        info!("    Video orientation: front={}, flipped={}, rotation={}°", 
                               front_facing, flipped, rotation);
                    }
                },
                TRANSPORT_CC => {
                    if element.data.len() >= 2 {
                        let seq = ((element.data[0] as u16) << 8) | (element.data[1] as u16);
                        info!("    TWCC sequence number: {}", seq);
                    }
                },
                _ => {
                    // For other extensions, just print the first few bytes as hex
                    let hex_data: Vec<String> = element.data.iter()
                        .take(8)
                        .map(|b| format!("{:02x}", b))
                        .collect();
                    
                    let more = if element.data.len() > 8 { "..." } else { "" };
                    info!("    Data: {} {}", hex_data.join(" "), more);
                }
            }
        }
    } else {
        info!("  No extensions found");
    }
    
    Ok(())
}

/// Demonstrate using the convenience methods on RtpHeader for extensions
fn demonstrate_header_convenience_methods() -> Result<(), Box<dyn std::error::Error>> {
    // Create a basic RTP header
    let mut header = RtpHeader::new(96, 3000, 48000, 0xdeadbeef);
    
    // Add extensions directly to the header
    header.add_extension(AUDIO_LEVEL, vec![0x40])?; // VAD=false, level=64
    header.add_extension(5, vec![1, 2, 3, 4])?;     // Custom extension
    
    info!("Added extensions directly to header:");
    
    // Get extensions
    if let Some(audio_level) = header.get_extension(AUDIO_LEVEL) {
        let vad = (audio_level[0] & 0x80) != 0;
        let level = audio_level[0] & 0x7F;
        info!("  - Audio level: VAD={}, level={}dB", vad, level);
    }
    
    if let Some(custom) = header.get_extension(5) {
        info!("  - Custom extension data: {:?}", custom);
    }
    
    // Remove an extension
    if let Some(removed_data) = header.remove_extension(AUDIO_LEVEL) {
        info!("  - Removed audio level extension: {:?}", removed_data);
    }
    
    // Check what's left
    info!("  - Extensions after removal: {}", 
           if let Some(exts) = &header.extensions { exts.len() } else { 0 });
    
    // Change format
    header.set_extension_format(ExtensionFormat::TwoByte)?;
    info!("  - Changed format to: {:?}", header.extension_format().unwrap());
    
    // Clear extensions
    header.clear_extensions();
    info!("  - Extensions after clearing: {}", 
           if let Some(exts) = &header.extensions { exts.len() } else { 0 });
    
    Ok(())
}

/// Real-world example: Audio level indication for VoIP applications
fn demonstrate_audio_level_extension() -> Result<(), Box<dyn std::error::Error>> {
    info!("VoIP application with audio level indicators:");
    
    // Simulate a multi-party VoIP call with 3 participants
    let participants = [
        (0xaaaaaaaa, "Alice", true, 25),   // Alice is speaking at moderate volume
        (0xbbbbbbbb, "Bob", false, 0),     // Bob is silent
        (0xcccccccc, "Carol", true, 10),   // Carol is speaking loudly
    ];
    
    for (ssrc, name, is_speaking, level) in participants {
        // Create a packet for each participant
        let mut header = RtpHeader::new(96, 1000, 48000, ssrc);
        
        // Add audio level extension
        let audio_data = vec![(if is_speaking { 0x80 } else { 0x00 }) | (level & 0x7F)];
        header.add_extension(AUDIO_LEVEL, audio_data)?;
        
        // Create a sample packet and serialize it
        let packet = RtpPacket::new(header, Bytes::from_static(b"audio frame"));
        let serialized = packet.serialize()?;
        
        // In a real application, this packet would be sent over the network
        // For this example, we'll just parse it back to simulate receiving it
        
        // Simulate receiving the packet
        let received = RtpPacket::parse(&serialized)?;
        
        // Process the audio level
        if let Some(audio_level) = received.header.get_extension(AUDIO_LEVEL) {
            if audio_level.len() >= 1 {
                let vad = (audio_level[0] & 0x80) != 0;
                let db_level = audio_level[0] & 0x7F;
                
                info!("  Participant {}: SSRC=0x{:08x}, speaking={}, level={}dB", 
                       name, ssrc, vad, db_level);
                
                // In a real app, we might use this to highlight active speakers
                if vad && db_level < 20 {
                    info!("    → {} is the current active speaker", name);
                }
            }
        }
    }
    
    Ok(())
}

/// Real-world example: Video orientation for mobile video
fn demonstrate_video_orientation_extension() -> Result<(), Box<dyn std::error::Error>> {
    info!("Mobile video application with orientation tracking:");
    
    // Simulate device orientation changes
    let orientations = [
        (0u16, false, "Portrait"),              // 0 degrees, not flipped
        (90u16, false, "Landscape left"),       // 90 degrees, not flipped
        (180u16, false, "Portrait upside down"), // 180 degrees, not flipped
        (270u16, false, "Landscape right"),     // 270 degrees, not flipped
    ];
    
    let ssrc = 0xdeadbeef;
    let mut seq = 1000;
    
    for (rotation_degrees, flipped, name) in orientations {
        // Convert degrees to the 2-bit representation (0=0°, 1=90°, 2=180°, 3=270°)
        let rotation_bits = ((rotation_degrees / 90) & 0x03) as u8;
        
        // Create a packet
        let mut header = RtpHeader::new(96, seq, 90000, ssrc);
        seq += 1;
        
        // Add video orientation extension
        // C=0 (not mirrored), F=0/1 (front/back camera), R=rotation
        let front_camera = true;
        let voi_data = vec![(if flipped { 0x80 } else { 0x00 }) | 
                       (if front_camera { 0x40 } else { 0x00 }) | 
                       (rotation_bits << 4)];
        
        header.add_extension(VIDEO_ORIENTATION, voi_data)?;
        
        // Create a sample packet and serialize it
        let packet = RtpPacket::new(header, Bytes::from_static(b"video frame"));
        let serialized = packet.serialize()?;
        
        // Simulate receiving the packet
        let received = RtpPacket::parse(&serialized)?;
        
        // Process the video orientation
        if let Some(voi) = received.header.get_extension(VIDEO_ORIENTATION) {
            if voi.len() >= 1 {
                let flipped = (voi[0] & 0x80) != 0;
                let front_facing = (voi[0] & 0x40) != 0;
                let rotation_bits = (voi[0] >> 4) & 0x03;
                let rotation = (rotation_bits as u16) * 90;
                
                info!("  Orientation change: {}", name);
                info!("    Front camera: {}", front_facing);
                info!("    Flipped: {}", flipped);
                info!("    Rotation: {}°", rotation);
                
                // In a real app, we would adjust the video rendering
                info!("    → Adjusting video rendering for {}° rotation", rotation);
            }
        }
    }
    
    Ok(())
} 