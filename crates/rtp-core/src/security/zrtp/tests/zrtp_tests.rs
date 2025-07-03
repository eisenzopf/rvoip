use crate::security::zrtp::{Zrtp, ZrtpConfig, ZrtpRole, ZrtpCipher, ZrtpHash, ZrtpAuthTag, ZrtpKeyAgreement, ZrtpSasType};
use crate::security::SecurityKeyExchange;
use crate::srtp::SRTP_AES128_CM_SHA1_80;
use crate::security::zrtp::packet::{ZrtpPacket, ZrtpMessageType, ZrtpVersion};

#[test]
fn test_zrtp_packet_formats() {
    // Create Hello packet
    let mut hello = ZrtpPacket::new(ZrtpMessageType::Hello);
    hello.set_version(ZrtpVersion::V12);
    hello.set_client_id("RVOIP ZRTP Test");
    
    let mut zid = [0u8; 12];
    for i in 0..12 {
        zid[i] = i as u8;
    }
    hello.set_zid(&zid);
    
    hello.add_cipher(ZrtpCipher::Aes1);
    hello.add_hash(ZrtpHash::S256);
    hello.add_auth_tag(ZrtpAuthTag::HS80);
    hello.add_key_agreement(ZrtpKeyAgreement::EC25);
    hello.add_sas_type(ZrtpSasType::B32);
    
    // Serialize to bytes
    let hello_bytes = hello.to_bytes();
    
    // Should be non-empty
    assert!(!hello_bytes.is_empty());
    
    // Parse back from bytes
    let parsed_hello = ZrtpPacket::parse(&hello_bytes).expect("Failed to parse Hello packet");
    
    // Check fields
    assert_eq!(parsed_hello.message_type(), ZrtpMessageType::Hello);
    assert_eq!(parsed_hello.zid().unwrap(), zid);
    assert!(!parsed_hello.ciphers().is_empty());
    assert!(!parsed_hello.hashes().is_empty());
    assert!(!parsed_hello.auth_tags().is_empty());
    assert!(!parsed_hello.key_agreements().is_empty());
    assert!(!parsed_hello.sas_types().is_empty());
    
    // Create Commit packet
    let mut commit = ZrtpPacket::new(ZrtpMessageType::Commit);
    commit.set_zid(&zid);
    commit.set_cipher(ZrtpCipher::Aes1);
    commit.set_hash(ZrtpHash::S256);
    commit.set_auth_tag(ZrtpAuthTag::HS80);
    commit.set_key_agreement(ZrtpKeyAgreement::EC25);
    commit.set_sas_type(ZrtpSasType::B32);
    
    // Serialize to bytes
    let commit_bytes = commit.to_bytes();
    
    // Parse back from bytes
    let parsed_commit = ZrtpPacket::parse(&commit_bytes).expect("Failed to parse Commit packet");
    
    // Check fields
    assert_eq!(parsed_commit.message_type(), ZrtpMessageType::Commit);
    assert_eq!(parsed_commit.zid().unwrap(), zid);
    assert_eq!(parsed_commit.cipher().unwrap(), ZrtpCipher::Aes1);
    assert_eq!(parsed_commit.hash().unwrap(), ZrtpHash::S256);
    assert_eq!(parsed_commit.auth_tag().unwrap(), ZrtpAuthTag::HS80);
    assert_eq!(parsed_commit.key_agreement().unwrap(), ZrtpKeyAgreement::EC25);
    assert_eq!(parsed_commit.sas_type().unwrap(), ZrtpSasType::B32);
}

#[test]
fn test_zrtp_hash_functions() {
    use crate::security::zrtp::hash::ZrtpHash;
    
    // Test SHA-256
    let data = b"test data for hashing";
    let hash = ZrtpHash::sha256(data);
    
    // Hash should be correct length
    assert_eq!(hash.len(), 32);
    
    // Hash should be deterministic
    let hash2 = ZrtpHash::sha256(data);
    assert_eq!(hash, hash2);
    
    // Different data should produce different hash
    let data2 = b"different test data";
    let hash3 = ZrtpHash::sha256(data2);
    assert_ne!(hash, hash3);
    
    // Test SHA-384
    let hash4 = ZrtpHash::sha384(data);
    
    // Hash should be correct length
    assert_eq!(hash4.len(), 48);
}

#[test]
fn test_zrtp_basic_init() {
    // Create config for initiator
    let initiator_config = ZrtpConfig {
        ciphers: vec![ZrtpCipher::Aes1],
        hashes: vec![ZrtpHash::S256],
        auth_tags: vec![ZrtpAuthTag::HS80],
        key_agreements: vec![ZrtpKeyAgreement::EC25],
        sas_types: vec![ZrtpSasType::B32],
        client_id: "RVOIP ZRTP Initiator".to_string(),
        srtp_profile: SRTP_AES128_CM_SHA1_80,
    };
    
    let mut initiator = Zrtp::new(initiator_config, ZrtpRole::Initiator);
    
    // Initialize key exchange
    let result = initiator.init();
    assert!(result.is_ok(), "Failed to initialize initiator: {:?}", result);
    
    // Create Hello message
    let hello_result = initiator.create_hello();
    assert!(hello_result.is_ok(), "Failed to create Hello message: {:?}", hello_result);
    
    // Verify Hello message is valid
    let hello = hello_result.unwrap();
    assert_eq!(hello.message_type(), ZrtpMessageType::Hello);
}

#[test]
fn test_zrtp_config() {
    // Create config with all supported algorithms
    let config = ZrtpConfig {
        ciphers: vec![ZrtpCipher::Aes1, ZrtpCipher::Aes3, ZrtpCipher::TwoF],
        hashes: vec![ZrtpHash::S256, ZrtpHash::S384],
        auth_tags: vec![ZrtpAuthTag::HS80, ZrtpAuthTag::HS32],
        key_agreements: vec![ZrtpKeyAgreement::EC25, ZrtpKeyAgreement::DH3k, ZrtpKeyAgreement::DH4k, ZrtpKeyAgreement::EC38],
        sas_types: vec![ZrtpSasType::B32, ZrtpSasType::B32E],
        client_id: "RVOIP ZRTP Test".to_string(),
        srtp_profile: SRTP_AES128_CM_SHA1_80,
    };
    
    // Create ZRTP instance with this config
    let zrtp = Zrtp::new(config, ZrtpRole::Initiator);
    
    // Check that the config was properly stored
    assert_eq!(zrtp.role, ZrtpRole::Initiator);
    assert!(!zrtp.is_complete());
}

#[test]
fn test_zrtp_sas_generation() {
    // Create ZRTP instances with completed exchange for SAS testing
    let config = ZrtpConfig {
        ciphers: vec![ZrtpCipher::Aes1],
        hashes: vec![ZrtpHash::S256],
        auth_tags: vec![ZrtpAuthTag::HS80],
        key_agreements: vec![ZrtpKeyAgreement::EC25],
        sas_types: vec![ZrtpSasType::B32],
        client_id: "RVOIP ZRTP SAS Test".to_string(),
        srtp_profile: SRTP_AES128_CM_SHA1_80,
    };
    
    let mut zrtp = Zrtp::new(config, ZrtpRole::Initiator);
    
    // Manually set up state for testing SAS generation
    // In real usage, this would happen after DH exchange
    zrtp.shared_secret = Some(vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
    zrtp.hello_hash = Some([0x11; 32]);
    zrtp.peer_hello_hash = Some([0x22; 32]);
    zrtp.selected_sas_type = Some(ZrtpSasType::B32);
    zrtp.state = crate::security::zrtp::ZrtpState::Completed;
    
    // Test SAS generation
    let sas_result = zrtp.generate_sas();
    assert!(sas_result.is_ok(), "SAS generation failed: {:?}", sas_result);
    
    let sas = sas_result.unwrap();
    assert_eq!(sas.len(), 4, "SAS should be 4 characters for B32 type");
    
    // Test SAS display
    let display_result = zrtp.get_sas_display();
    assert!(display_result.is_ok(), "SAS display generation failed: {:?}", display_result);
    
    let display = display_result.unwrap();
    assert!(display.contains(&sas), "SAS display should contain the generated SAS");
    assert!(display.contains("SAS:"), "SAS display should be formatted properly");
}

#[test]
fn test_zrtp_sas_verification() {
    // Create ZRTP instances for SAS verification testing
    let config = ZrtpConfig {
        ciphers: vec![ZrtpCipher::Aes1],
        hashes: vec![ZrtpHash::S256],
        auth_tags: vec![ZrtpAuthTag::HS80],
        key_agreements: vec![ZrtpKeyAgreement::EC25],
        sas_types: vec![ZrtpSasType::B32],
        client_id: "RVOIP ZRTP Verification Test".to_string(),
        srtp_profile: SRTP_AES128_CM_SHA1_80,
    };
    
    let mut zrtp = Zrtp::new(config, ZrtpRole::Initiator);
    
    // Set up completed state
    zrtp.shared_secret = Some(vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
    zrtp.hello_hash = Some([0x11; 32]);
    zrtp.peer_hello_hash = Some([0x22; 32]);
    zrtp.selected_sas_type = Some(ZrtpSasType::B32);
    zrtp.state = crate::security::zrtp::ZrtpState::Completed;
    
    // Generate SAS
    let sas = zrtp.generate_sas().unwrap();
    
    // Test correct verification
    let verify_correct = zrtp.verify_sas(&sas);
    assert!(verify_correct.is_ok(), "SAS verification should work");
    assert!(verify_correct.unwrap(), "Correct SAS should verify");
    
    // Test case-insensitive verification
    let verify_case = zrtp.verify_sas(&sas.to_lowercase());
    assert!(verify_case.is_ok(), "Case-insensitive SAS verification should work");
    assert!(verify_case.unwrap(), "Case-insensitive SAS should verify");
    
    // Test incorrect verification
    let verify_incorrect = zrtp.verify_sas("WXYZ");
    assert!(verify_incorrect.is_ok(), "SAS verification should not error");
    assert!(!verify_incorrect.unwrap(), "Incorrect SAS should not verify");
}

#[test]
fn test_zrtp_sas_different_types() {
    // Test different SAS types
    let base_config = ZrtpConfig {
        ciphers: vec![ZrtpCipher::Aes1],
        hashes: vec![ZrtpHash::S256],
        auth_tags: vec![ZrtpAuthTag::HS80],
        key_agreements: vec![ZrtpKeyAgreement::EC25],
        sas_types: vec![ZrtpSasType::B32],
        client_id: "RVOIP ZRTP SAS Type Test".to_string(),
        srtp_profile: SRTP_AES128_CM_SHA1_80,
    };
    
    // Test B32 type
    let mut zrtp_b32 = Zrtp::new(base_config.clone(), ZrtpRole::Initiator);
    zrtp_b32.shared_secret = Some(vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
    zrtp_b32.hello_hash = Some([0x11; 32]);
    zrtp_b32.peer_hello_hash = Some([0x22; 32]);
    zrtp_b32.selected_sas_type = Some(ZrtpSasType::B32);
    zrtp_b32.state = crate::security::zrtp::ZrtpState::Completed;
    
    let sas_b32 = zrtp_b32.generate_sas().unwrap();
    assert_eq!(sas_b32.len(), 4, "B32 SAS should be 4 characters");
    assert!(sas_b32.chars().all(|c| c.is_ascii_alphanumeric()), "B32 SAS should be alphanumeric");
    
    // Test B32E type
    let mut zrtp_b32e = Zrtp::new(base_config, ZrtpRole::Initiator);
    zrtp_b32e.shared_secret = Some(vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
    zrtp_b32e.hello_hash = Some([0x11; 32]);
    zrtp_b32e.peer_hello_hash = Some([0x22; 32]);
    zrtp_b32e.selected_sas_type = Some(ZrtpSasType::B32E);
    zrtp_b32e.state = crate::security::zrtp::ZrtpState::Completed;
    
    let sas_b32e = zrtp_b32e.generate_sas().unwrap();
    assert_eq!(sas_b32e.len(), 4, "B32E SAS should be 4 characters");
    assert!(sas_b32e.chars().all(|c| c.is_ascii_digit()), "B32E SAS should be numeric");
}

#[test]
fn test_zrtp_sas_deterministic() {
    // Test that SAS generation is deterministic
    let config = ZrtpConfig {
        ciphers: vec![ZrtpCipher::Aes1],
        hashes: vec![ZrtpHash::S256],
        auth_tags: vec![ZrtpAuthTag::HS80],
        key_agreements: vec![ZrtpKeyAgreement::EC25],
        sas_types: vec![ZrtpSasType::B32],
        client_id: "RVOIP ZRTP Deterministic Test".to_string(),
        srtp_profile: SRTP_AES128_CM_SHA1_80,
    };
    
    let mut zrtp1 = Zrtp::new(config.clone(), ZrtpRole::Initiator);
    let mut zrtp2 = Zrtp::new(config, ZrtpRole::Responder);
    
    // Set up identical state on both
    let shared_secret = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
    let hello_hash_i = [0x11; 32];  // Initiator hello hash
    let hello_hash_r = [0x22; 32];  // Responder hello hash
    
    zrtp1.shared_secret = Some(shared_secret.clone());
    zrtp1.hello_hash = Some(hello_hash_i);      // Initiator's own hello
    zrtp1.peer_hello_hash = Some(hello_hash_r); // Responder's hello (peer)
    zrtp1.selected_sas_type = Some(ZrtpSasType::B32);
    zrtp1.state = crate::security::zrtp::ZrtpState::Completed;
    
    zrtp2.shared_secret = Some(shared_secret);
    zrtp2.hello_hash = Some(hello_hash_r);      // Responder's own hello
    zrtp2.peer_hello_hash = Some(hello_hash_i); // Initiator's hello (peer)
    zrtp2.selected_sas_type = Some(ZrtpSasType::B32);
    zrtp2.state = crate::security::zrtp::ZrtpState::Completed;
    
    // Both should generate the same SAS
    let sas1 = zrtp1.generate_sas().unwrap();
    let sas2 = zrtp2.generate_sas().unwrap();
    
    assert_eq!(sas1, sas2, "Both endpoints should generate the same SAS");
} 