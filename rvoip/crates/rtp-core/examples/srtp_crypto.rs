use bytes::{Bytes, BytesMut, BufMut};
use rvoip_rtp_core::{
    packet::RtpPacket,
    srtp::{
        SrtpCryptoSuite, SrtpCryptoKey, ProtectedRtpPacket,
        crypto::SrtpCrypto,
        auth::SrtpAuthenticator,
        SrtpAuthenticationAlgorithm,
        SrtpEncryptionAlgorithm,
        SRTP_AES128_CM_SHA1_80,
        SRTP_AES128_CM_SHA1_32,
        SRTP_NULL_NULL,
    },
    Result,
};

fn main() -> Result<()> {
    println!("SRTP Crypto Test Example");
    println!("========================");
    
    // Test standard RFC 3711 cipher suites
    test_crypto_suite("NULL_NULL", SRTP_NULL_NULL)?;
    test_crypto_suite("AES128_CM_SHA1_80", SRTP_AES128_CM_SHA1_80)?;
    test_crypto_suite("AES128_CM_SHA1_32", SRTP_AES128_CM_SHA1_32)?;
    
    // Test specific algorithm combinations
    println!("\nTesting custom algorithm combinations:");
    
    // NULL encryption with SHA1-80 authentication
    let null_sha1_80 = SrtpCryptoSuite {
        encryption: SrtpEncryptionAlgorithm::Null,
        authentication: SrtpAuthenticationAlgorithm::HmacSha1_80,
        key_length: 16, // Key still needed for auth
        tag_length: 10,
    };
    test_crypto_suite("NULL_SHA1_80", null_sha1_80)?;
    
    // AES-CM with NULL authentication
    let aes_cm_null = SrtpCryptoSuite {
        encryption: SrtpEncryptionAlgorithm::AesCm,
        authentication: SrtpAuthenticationAlgorithm::Null,
        key_length: 16,
        tag_length: 0,
    };
    test_crypto_suite("AES128_CM_NULL", aes_cm_null)?;
    
    // Test tamper resistance
    test_tamper_resistance()?;
    
    println!("\nAll SRTP cipher tests completed successfully!");
    
    Ok(())
}

fn test_crypto_suite(name: &str, suite: SrtpCryptoSuite) -> Result<()> {
    println!("\nTesting SRTP crypto suite: {}", name);
    println!("  Encryption: {:?}", suite.encryption);
    println!("  Authentication: {:?}", suite.authentication);
    println!("  Key length: {} bytes", suite.key_length);
    println!("  Tag length: {} bytes", suite.tag_length);
    
    // Create a test master key and salt
    let master_key = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 
                         0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10];
    let master_salt = vec![0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 
                           0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D];
    
    let srtp_key = SrtpCryptoKey::new(master_key, master_salt);
    
    // Create SRTP crypto context
    let crypto = SrtpCrypto::new(suite.clone(), srtp_key)?;
    
    // Create test RTP packets with different payload types and content
    let packets = vec![
        create_test_packet(96, 1000, 12345, 0xABCD0001, "This is packet #1 with payload type 96"),
        create_test_packet(97, 1001, 12346, 0xABCD0001, "This is packet #2 with payload type 97"),
        create_test_packet(98, 1002, 12347, 0xABCD0001, "This is packet #3 with payload type 98 with some longer content"),
    ];
    
    // For each packet, encrypt it, print details, and decrypt it back
    for (i, packet) in packets.iter().enumerate() {
        println!("\nProcessing packet #{}", i+1);
        
        // Print original packet details
        println!("Original: PT={}, SEQ={}, TS={}, SSRC={:#X}, Payload[{}]: {:?}",
            packet.header.payload_type,
            packet.header.sequence_number,
            packet.header.timestamp,
            packet.header.ssrc,
            packet.payload.len(),
            payload_as_text(&packet.payload));
            
        // Encrypt the packet - now returns tuple of (packet, auth_tag)
        let (encrypted, auth_tag) = crypto.encrypt_rtp(&packet)?;
        
        // Print encrypted packet details
        println!("Encrypted: PT={}, SEQ={}, TS={}, SSRC={:#X}, Payload[{}]: {:?}",
            encrypted.header.payload_type,
            encrypted.header.sequence_number,
            encrypted.header.timestamp,
            encrypted.header.ssrc,
            encrypted.payload.len(),
            format!("{:02X?}", &encrypted.payload[..min(16, encrypted.payload.len())]));
            
        // Serialize the encrypted packet
        let serialized = encrypted.serialize()?;
        
        // Add auth tag if provided
        let mut packet_with_auth = BytesMut::with_capacity(serialized.len() + suite.tag_length);
        packet_with_auth.extend_from_slice(&serialized);
        
        if let Some(tag) = auth_tag {
            packet_with_auth.extend_from_slice(&tag);
            println!("Added authentication tag of {} bytes", tag.len());
        }
        
        // Decrypt the packet
        let decrypted = crypto.decrypt_rtp(&packet_with_auth)?;
        
        // Print decrypted packet details
        println!("Decrypted: PT={}, SEQ={}, TS={}, SSRC={:#X}, Payload[{}]: {:?}",
            decrypted.header.payload_type,
            decrypted.header.sequence_number,
            decrypted.header.timestamp,
            decrypted.header.ssrc,
            decrypted.payload.len(),
            payload_as_text(&decrypted.payload));
            
        // Verify the decrypted packet matches the original
        assert_eq!(decrypted.header.payload_type, packet.header.payload_type);
        assert_eq!(decrypted.header.sequence_number, packet.header.sequence_number);
        assert_eq!(decrypted.header.timestamp, packet.header.timestamp);
        assert_eq!(decrypted.header.ssrc, packet.header.ssrc);
        assert_eq!(decrypted.payload, packet.payload);
        
        println!("Verification: Success - Decrypted packet matches original");
    }
    
    println!("\n{} test completed successfully", name);
    Ok(())
}

// Test tamper resistance of SRTP with authentication
fn test_tamper_resistance() -> Result<()> {
    println!("\nTesting SRTP tamper resistance");
    
    // Use AES128_CM_SHA1_80 which has strong authentication
    let suite = SRTP_AES128_CM_SHA1_80;
    
    // Create a test master key and salt
    let master_key = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 
                         0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10];
    let master_salt = vec![0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 
                           0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D];
    
    let srtp_key = SrtpCryptoKey::new(master_key, master_salt);
    
    // Create SRTP crypto context
    let crypto = SrtpCrypto::new(suite.clone(), srtp_key.clone())?;
    
    // Create a test packet
    let packet = create_test_packet(96, 1000, 12345, 0xABCD0001, 
                                  "This packet will be tampered with");
    
    // Encrypt the packet and get authentication tag
    let (encrypted, auth_tag) = crypto.encrypt_rtp(&packet)?;
    let serialized = encrypted.serialize()?;
    
    // Build packet with authentication tag
    let mut packet_with_auth = BytesMut::with_capacity(serialized.len() + suite.tag_length);
    packet_with_auth.extend_from_slice(&serialized);
    
    if let Some(tag) = auth_tag {
        packet_with_auth.extend_from_slice(&tag);
        println!("Added authentication tag of {} bytes", tag.len());
    } else {
        println!("ERROR: No authentication tag returned, cannot test tamper resistance");
        return Ok(());
    }
    
    // Try decrypting untampered packet - should succeed
    println!("Attempting to decrypt untampered packet...");
    match crypto.decrypt_rtp(&packet_with_auth) {
        Ok(_) => println!("Success: Untampered packet decrypted correctly"),
        Err(e) => println!("ERROR: Failed to decrypt untampered packet: {}", e),
    }
    
    // Now tamper with the payload
    println!("\nAttempting to decrypt packet with tampered payload...");
    let mut tampered = packet_with_auth.to_vec();
    
    // Modify a byte in the middle of the packet (payload)
    let header_size = 12; // Standard RTP header size
    if tampered.len() > header_size + 5 {
        // Flip some bits in the payload
        tampered[header_size + 5] ^= 0x42;
        
        // Try to decrypt the tampered packet - should fail due to auth tag mismatch
        let result = crypto.decrypt_rtp(&tampered);
        match result {
            Ok(_) => println!("ERROR: Tampered packet was accepted!"),
            Err(e) => println!("Success: Tampered packet correctly rejected: {}", e),
        }
    }
    
    // Now tamper with the authentication tag itself
    println!("\nAttempting to decrypt packet with tampered authentication tag...");
    let mut tampered = packet_with_auth.to_vec();
    
    // Modify the last byte of the auth tag
    if tampered.len() > 0 {
        let last_idx = tampered.len() - 1;
        tampered[last_idx] ^= 0xFF; // Flip all bits in last byte
        
        // Try to decrypt the tampered packet - should fail
        let result = crypto.decrypt_rtp(&tampered);
        match result {
            Ok(_) => println!("ERROR: Packet with tampered auth tag was accepted!"),
            Err(e) => println!("Success: Packet with tampered auth tag correctly rejected: {}", e),
        }
    }
    
    println!("\nTamper resistance test completed successfully");
    Ok(())
}

// Helper to create a test RTP packet
fn create_test_packet(
    payload_type: u8,
    sequence: u16,
    timestamp: u32,
    ssrc: u32,
    payload_text: &str
) -> RtpPacket {
    let header = rvoip_rtp_core::packet::RtpHeader::new(payload_type, sequence, timestamp, ssrc);
    let payload = Bytes::from(payload_text.as_bytes().to_vec());
    RtpPacket::new(header, payload)
}

// Helper to convert binary payload to text for display
fn payload_as_text(payload: &Bytes) -> String {
    String::from_utf8_lossy(payload).to_string()
}

// Helper to take minimum of two values
fn min(a: usize, b: usize) -> usize {
    if a < b { a } else { b }
} 