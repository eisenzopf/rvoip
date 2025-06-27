//! Simplified RTP SRTP Example - Demonstrates Security Without Transport Issues
//!
//! This example shows SRTP functionality in a straightforward way, focusing on
//! the security features rather than complex network transport.

use rvoip_rtp_core::{
    api::{
        common::frame::{MediaFrame, MediaFrameType},
        common::config::SrtpProfile,
    },
    srtp::{SrtpCryptoKey, SrtpCryptoSuite, SRTP_AES128_CM_SHA1_80, SrtpContext},
    packet::{RtpPacket, RtpHeader},
};

use std::time::Duration;
use tracing::{info, debug, warn, error};
use std::sync::Arc;
use bytes::Bytes;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    
    info!("🔐 Simplified SRTP Security Example");
    info!("===================================");
    info!("Demonstrating SRTP encryption/decryption without transport complexity");
    info!("");
    
    // Step 1: Create SRTP keys and context
    info!("Step 1: Setting up SRTP security context...");
    
    // Example key (16 bytes for AES-128) and salt (14 bytes)
    let key_data = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 
                       0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10];
    
    let salt_data = vec![0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 
                        0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E];
    
    // Create SRTP crypto key
    let crypto_key = SrtpCryptoKey::new(key_data.clone(), salt_data.clone());
    
    // Key information for SDP in base64 format
    let mut combined = Vec::with_capacity(key_data.len() + salt_data.len());
    combined.extend_from_slice(&key_data);
    combined.extend_from_slice(&salt_data);
    let base64_key = base64::encode(&combined);
    
    info!("✅ SRTP crypto suite: AES_CM_128_HMAC_SHA1_80");
    info!("✅ SRTP key+salt (base64): {}", base64_key);
    info!("✅ SDP crypto line: 1 AES_CM_128_HMAC_SHA1_80 inline:{}", base64_key);
    info!("");
    
    // Step 2: Create SRTP contexts for sender and receiver
    info!("Step 2: Creating SRTP encryption contexts...");
    
    let mut sender_context = SrtpContext::new(SRTP_AES128_CM_SHA1_80, crypto_key.clone())
        .map_err(|e| format!("Failed to create sender SRTP context: {}", e))?;
        
    let mut receiver_context = SrtpContext::new(SRTP_AES128_CM_SHA1_80, crypto_key.clone())
        .map_err(|e| format!("Failed to create receiver SRTP context: {}", e))?;
        
    info!("✅ Sender SRTP context created");
    info!("✅ Receiver SRTP context created");
    info!("");
    
    // Step 3: Create test RTP packets and demonstrate encryption/decryption
    info!("Step 3: Testing SRTP encryption and decryption...");
    info!("");
    
    for i in 0..3 {
        let test_message = format!("🎵 Secure audio frame #{}", i);
        
        // Create RTP packet
        let rtp_header = RtpHeader::new(
            0, // payload_type (PCMU)
            i as u16, // sequence_number
            i * 160, // timestamp (20ms at 8kHz)
            0x1234ABCD, // ssrc
        );
        
        let payload = Bytes::from(test_message.clone().into_bytes());
        let mut rtp_packet = RtpPacket::new(rtp_header, payload);
        
        info!("🔤 Original message {}: '{}'", i, test_message);
        info!("📦 RTP packet size: {} bytes", rtp_packet.serialize()?.len());
        
        // Encrypt with SRTP
        let protected_packet = sender_context.protect(&rtp_packet)
            .map_err(|e| format!("Failed to encrypt RTP packet: {}", e))?;
            
        // Serialize the protected packet for transmission
        let protected_bytes = protected_packet.serialize()
            .map_err(|e| format!("Failed to serialize protected packet: {}", e))?;
            
        info!("🔒 Encrypted packet size: {} bytes", protected_bytes.len());
        
        // Show some encrypted bytes for demonstration
        let preview: String = protected_bytes.iter().take(16)
            .map(|b| format!("{:02x}", b))
            .collect::<Vec<String>>()
            .join(" ");
        info!("🔐 Encrypted preview: {} ...", preview);
        
        // Decrypt with SRTP
        let decrypted_packet = receiver_context.unprotect(&protected_bytes)
            .map_err(|e| format!("Failed to decrypt SRTP packet: {}", e))?;
            
        // Extract the payload
        let decrypted_message = String::from_utf8_lossy(&decrypted_packet.payload);
        
        info!("🔓 Decrypted message: '{}'", decrypted_message);
        
        // Verify the message matches
        if decrypted_message == test_message {
            info!("✅ Encryption/Decryption SUCCESS - Messages match!");
        } else {
            error!("❌ Encryption/Decryption FAILED - Messages don't match!");
            return Err("SRTP test failed".into());
        }
        
        info!("");
        
        // Wait a bit between packets
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    
    // Step 4: Demonstrate MediaFrame security
    info!("Step 4: Testing MediaFrame-level security...");
    info!("");
    
    let media_frame = MediaFrame {
        frame_type: MediaFrameType::Audio,
        data: "🎤 Secure voice data".as_bytes().to_vec(),
        timestamp: 3200, // 400ms at 8kHz
        sequence: 3,
        marker: true,
        payload_type: 0, // PCMU
        ssrc: 0x1234ABCD,
        csrcs: Vec::new(),
    };
    
    info!("📢 Original MediaFrame: {} bytes", media_frame.data.len());
    info!("🔤 Frame data: '{}'", String::from_utf8_lossy(&media_frame.data));
    
    // Convert MediaFrame to RTP packet for encryption
    let frame_header = RtpHeader::new(
        media_frame.payload_type,
        media_frame.sequence,
        media_frame.timestamp,
        media_frame.ssrc,
    );
    
    let frame_payload = Bytes::from(media_frame.data.clone());
    let frame_packet = RtpPacket::new(frame_header, frame_payload);
    
    // Encrypt the MediaFrame
    let protected_frame = sender_context.protect(&frame_packet)
        .map_err(|e| format!("Failed to encrypt MediaFrame: {}", e))?;
        
    let protected_frame_bytes = protected_frame.serialize()
        .map_err(|e| format!("Failed to serialize protected MediaFrame: {}", e))?;
        
    info!("🔒 Encrypted MediaFrame: {} bytes", protected_frame_bytes.len());
    
    // Decrypt the MediaFrame
    let decrypted_frame_packet = receiver_context.unprotect(&protected_frame_bytes)
        .map_err(|e| format!("Failed to decrypt MediaFrame: {}", e))?;
        
    let decrypted_frame_message = String::from_utf8_lossy(&decrypted_frame_packet.payload);
    
    info!("🔓 Decrypted MediaFrame: '{}'", decrypted_frame_message);
    
    if decrypted_frame_packet.payload.to_vec() == media_frame.data {
        info!("✅ MediaFrame encryption/decryption SUCCESS!");
    } else {
        error!("❌ MediaFrame encryption/decryption FAILED!");
        return Err("MediaFrame SRTP test failed".into());
    }
    
    info!("");
    
    // Step 5: Security summary
    info!("🎉 SRTP Security Example Completed Successfully!");
    info!("============================================");
    info!("✅ SRTP contexts created and initialized");
    info!("✅ RTP packet encryption/decryption working");
    info!("✅ MediaFrame security integration working");
    info!("✅ AES-128 encryption with HMAC-SHA1 authentication");
    info!("✅ Ready for SIP/SDP integration with crypto lines");
    info!("");
    info!("🔧 Next steps:");
    info!("   • Integrate with SDP crypto attribute parsing");
    info!("   • Add key exchange protocols (SDES, MIKEY, ZRTP)"); 
    info!("   • Enable full transport layer with DTLS-SRTP");
    
    Ok(())
} 