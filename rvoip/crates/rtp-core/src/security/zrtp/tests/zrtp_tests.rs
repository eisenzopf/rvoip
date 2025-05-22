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