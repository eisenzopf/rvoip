use bytes::{Bytes, BytesMut};
use rvoip_rtp_core::{
    packet::RtpPacket,
    srtp::{
        SrtpContext, SrtpCryptoSuite, SrtpCryptoKey, ProtectedRtpPacket,
        SrtpAuthenticationAlgorithm, SrtpEncryptionAlgorithm,
        SRTP_AES128_CM_SHA1_80, SRTP_NULL_NULL,
    },
    Result,
};

fn main() -> Result<()> {
    println!("SRTP Context API Example");
    println!("========================");
    
    // Create test packets
    let packet1 = create_test_packet(96, 1000, 12345, 0xABCD0001, 
                                     "This is a secure RTP packet");
    
    println!("Testing with AES_CM_SHA1_80 (encryption + authentication)");
    test_srtp_context(SRTP_AES128_CM_SHA1_80, &packet1)?;
    
    println!("\nTesting with NULL_NULL (no encryption or authentication)");
    test_srtp_context(SRTP_NULL_NULL, &packet1)?;
    
    // Create a custom suite with authentication but no encryption
    let auth_only = SrtpCryptoSuite {
        encryption: SrtpEncryptionAlgorithm::Null,
        authentication: SrtpAuthenticationAlgorithm::HmacSha1_80,
        key_length: 16,
        tag_length: 10,
    };
    
    println!("\nTesting with NULL_SHA1_80 (authentication only)");
    test_srtp_context(auth_only, &packet1)?;
    
    Ok(())
}

fn test_srtp_context(suite: SrtpCryptoSuite, packet: &RtpPacket) -> Result<()> {
    // Create crypto key
    let key_data = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
    let salt_data = vec![16, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1];
    let key = SrtpCryptoKey::new(key_data, salt_data);
    
    // Create SRTP context
    let mut context = SrtpContext::new(suite.clone(), key)?;
    
    // Protect the packet - this returns a ProtectedRtpPacket with auth tag
    let protected = context.protect(packet)?;
    
    println!("Original packet: PT={}, SEQ={}, TS={}, SSRC={:#X}, Payload[{}]",
        packet.header.payload_type,
        packet.header.sequence_number,
        packet.header.timestamp,
        packet.header.ssrc,
        packet.payload.len());
    
    println!("Protected packet: PT={}, SEQ={}, Payload[{}], Auth tag: {}",
        protected.packet.header.payload_type,
        protected.packet.header.sequence_number,
        protected.packet.payload.len(),
        if let Some(tag) = &protected.auth_tag {
            format!("{} bytes", tag.len())
        } else {
            "None".to_string()
        });
    
    // Serialize the protected packet with its auth tag
    let serialized = protected.serialize()?;
    println!("Serialized size: {} bytes", serialized.len());
    
    // Unprotect the serialized data (includes auth tag)
    let unprotected = context.unprotect(&serialized)?;
    
    println!("Unprotected packet: PT={}, SEQ={}, TS={}, SSRC={:#X}, Payload[{}]",
        unprotected.header.payload_type,
        unprotected.header.sequence_number,
        unprotected.header.timestamp,
        unprotected.header.ssrc,
        unprotected.payload.len());
    
    // Verify the unprotected packet matches the original
    assert_eq!(unprotected.header.payload_type, packet.header.payload_type);
    assert_eq!(unprotected.header.sequence_number, packet.header.sequence_number);
    assert_eq!(unprotected.header.timestamp, packet.header.timestamp);
    assert_eq!(unprotected.header.ssrc, packet.header.ssrc);
    assert_eq!(unprotected.payload, packet.payload);
    
    println!("Verification: Success - Packet was protected and unprotected correctly");
    
    // Test tamper resistance
    if suite.authentication != SrtpAuthenticationAlgorithm::Null {
        println!("\nTesting tamper resistance:");
        
        // Tamper with the payload
        let mut tampered = serialized.to_vec();
        let header_size = 12; // RTP header size
        if tampered.len() > header_size + 5 {
            tampered[header_size + 5] ^= 0x42; // Flip some bits
            
            // Try to unprotect the tampered data
            match context.unprotect(&tampered) {
                Ok(_) => println!("WARNING: Tampered packet was accepted!"),
                Err(e) => println!("Success: Tampered packet was rejected: {}", e),
            }
        }
    }
    
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