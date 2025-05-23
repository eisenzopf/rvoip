//! SRTP Encryption/Decryption Debug Example
//!
//! This example demonstrates that SRTP encryption and decryption work correctly
//! by manually calling the protect() and unprotect() methods.

use rvoip_rtp_core::{
    srtp::{SrtpContext, SrtpCryptoSuite, SRTP_AES128_CM_SHA1_80},
    srtp::crypto::SrtpCryptoKey,
    packet::{RtpHeader, RtpPacket},
};

use bytes::Bytes;
use tracing::{info, error, debug};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    info!("=== SRTP Debug Example ===");
    info!("Testing SRTP encryption/decryption manually");

    // 1. Create the same SRTP keys as our example
    let key_data = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 
                     0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10];
    
    let salt_data = vec![0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 
                     0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E];

    let crypto_key = SrtpCryptoKey::new(key_data, salt_data);
    
    // 2. Create SRTP contexts (client and server should use same keys for pre-shared key mode)
    let mut client_context = SrtpContext::new(SRTP_AES128_CM_SHA1_80, crypto_key.clone())
        .map_err(|e| format!("Failed to create client SRTP context: {}", e))?;
        
    let mut server_context = SrtpContext::new(SRTP_AES128_CM_SHA1_80, crypto_key)
        .map_err(|e| format!("Failed to create server SRTP context: {}", e))?;

    info!("✅ Created SRTP contexts successfully");

    // 3. Test encryption/decryption with same frames as our example
    for i in 0..5 {
        let test_data = format!("Secure test frame {}", i);
        info!("\n--- Testing frame {} ---", i);
        info!("Original message: '{}'", test_data);
        
        // Create RTP packet with exact same parameters as our client
        let rtp_header = RtpHeader::new(
            0, // payload_type (PCMU)
            i as u16, // sequence_number (incremental like our fix)
            i * 160, // timestamp (different per frame)
            0x1234ABCD, // EXACT SSRC from our example
        );
        
        let payload = Bytes::from(test_data.clone().into_bytes());
        let rtp_packet = RtpPacket::new(rtp_header, payload);
        
        info!("RTP packet: SSRC={:08x}, seq={}, ts={}", 
              rtp_packet.header.ssrc, rtp_packet.header.sequence_number, rtp_packet.header.timestamp);
        
        // Client encrypts (like our client)
        let protected_packet = client_context.protect(&rtp_packet)
            .map_err(|e| format!("Client encryption failed: {}", e))?;
            
        let protected_bytes = protected_packet.serialize()
            .map_err(|e| format!("Failed to serialize protected packet: {}", e))?;
            
        info!("🔒 Client encrypted: {} -> {} bytes", 
              rtp_packet.serialize()?.len(), protected_bytes.len());
        
        // Server decrypts (like our server)
        let decrypted_packet = server_context.unprotect(&protected_bytes)
            .map_err(|e| format!("Server decryption failed: {}", e))?;
            
        let decrypted_message = String::from_utf8_lossy(&decrypted_packet.payload);
        
        info!("🔓 Server decrypted: '{}'", decrypted_message);
        
        // Verify the message matches
        if decrypted_message == test_data {
            info!("✅ SRTP Encryption/Decryption SUCCESS - Messages match!");
        } else {
            error!("❌ SRTP Encryption/Decryption FAILED");
            error!("   Expected: '{}'", test_data);
            error!("   Got:      '{}'", decrypted_message);
            return Err("SRTP test failed".into());
        }
        
        // Show encrypted data preview (should be different from plaintext)
        let preview: String = protected_bytes.iter().skip(12).take(8) // Skip RTP header
            .map(|b| format!("{:02x}", b))
            .collect::<Vec<String>>()
            .join(" ");
        info!("🔐 Encrypted payload preview: {}", preview);
    }
    
    info!("\n🎉 ALL SRTP TESTS PASSED!");
    info!("The SRTP implementation works correctly.");
    info!("The issue is that the transport layer is NOT using SRTP encryption.");
    
    Ok(())
} 