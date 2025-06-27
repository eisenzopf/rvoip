use crate::security::sdes::{Sdes, SdesConfig, SdesRole, SdesCryptoAttribute};
use crate::security::SecurityKeyExchange;
use crate::srtp::{SRTP_AES128_CM_SHA1_80, SRTP_AES128_CM_SHA1_32};

#[test]
fn test_sdes_crypto_attribute_parsing() {
    // Test parsing a valid crypto attribute
    let attr_str = "1 AES_CM_128_HMAC_SHA1_80 inline:PS1uQCVeeCFCanVmcjkpPywjNWhcYD0mXXtxaVBR";
    let attr = SdesCryptoAttribute::parse(attr_str).expect("Failed to parse valid crypto attribute");
    
    assert_eq!(attr.tag, 1);
    assert_eq!(attr.crypto_suite, "AES_CM_128_HMAC_SHA1_80");
    assert_eq!(attr.key_method, "inline");
    assert_eq!(attr.key_info, "PS1uQCVeeCFCanVmcjkpPywjNWhcYD0mXXtxaVBR");
    assert!(attr.session_params.is_empty());
    
    // Test parsing with session parameters
    let attr_str = "2 AES_CM_128_HMAC_SHA1_32 inline:PS1uQCVeeCFCanVmcjkpPywjNWhcYD0mXXtxaVBR KDR=1;UNENCRYPTED_SRTP";
    let attr = SdesCryptoAttribute::parse(attr_str).expect("Failed to parse crypto attribute with params");
    
    assert_eq!(attr.tag, 2);
    assert_eq!(attr.crypto_suite, "AES_CM_128_HMAC_SHA1_32");
    assert_eq!(attr.key_method, "inline");
    assert_eq!(attr.key_info, "PS1uQCVeeCFCanVmcjkpPywjNWhcYD0mXXtxaVBR");
    assert_eq!(attr.session_params.len(), 2);
    assert_eq!(attr.session_params[0], "KDR=1");
    assert_eq!(attr.session_params[1], "UNENCRYPTED_SRTP");
    
    // Test string conversion
    let str_repr = attr.to_string();
    assert_eq!(str_repr, "2 AES_CM_128_HMAC_SHA1_32 inline:PS1uQCVeeCFCanVmcjkpPywjNWhcYD0mXXtxaVBR KDR=1;UNENCRYPTED_SRTP");
    
    // Test invalid attribute (missing key info)
    let invalid_attr = "1 AES_CM_128_HMAC_SHA1_80 inline";
    let result = SdesCryptoAttribute::parse(invalid_attr);
    assert!(result.is_err());
}

#[test]
fn test_sdes_offer_answer_exchange() {
    // Configure offerer
    let offerer_config = SdesConfig {
        crypto_suites: vec![SRTP_AES128_CM_SHA1_80, SRTP_AES128_CM_SHA1_32],
        offer_count: 2,
    };
    
    let mut offerer = Sdes::new(offerer_config, SdesRole::Offerer);
    
    // Configure answerer
    let answerer_config = SdesConfig {
        crypto_suites: vec![SRTP_AES128_CM_SHA1_80, SRTP_AES128_CM_SHA1_32],
        offer_count: 1,
    };
    
    let mut answerer = Sdes::new(answerer_config, SdesRole::Answerer);
    
    // Initialize key exchange
    offerer.init().expect("Failed to initialize offerer");
    answerer.init().expect("Failed to initialize answerer");
    
    // Offerer creates offer
    let offer_result = offerer.process_message(b"").expect("Failed to create offer");
    assert!(offer_result.is_some());
    let offer = offer_result.unwrap();
    
    // Convert offer bytes to string for inspection
    let offer_str = std::str::from_utf8(&offer).expect("Offer is not valid UTF-8");
    println!("SDP Offer: {}", offer_str);
    
    // Offer should contain crypto lines
    assert!(offer_str.contains("a=crypto:"));
    assert!(offer_str.contains("AES_CM_128_HMAC_SHA1_80"));
    
    // Answerer processes offer and creates answer
    let answer_result = answerer.process_message(&offer).expect("Failed to process offer");
    assert!(answer_result.is_some());
    let answer = answer_result.unwrap();
    
    // Convert answer bytes to string for inspection
    let answer_str = std::str::from_utf8(&answer).expect("Answer is not valid UTF-8");
    println!("SDP Answer: {}", answer_str);
    
    // Answer should contain exactly one crypto line
    assert!(answer_str.contains("a=crypto:"));
    
    // Offerer processes answer
    offerer.process_message(&answer).expect("Failed to process answer");
    
    // Verify both sides have completed the exchange
    assert!(offerer.is_complete());
    assert!(answerer.is_complete());
    
    // Verify both sides have SRTP keys
    assert!(offerer.get_srtp_key().is_some());
    assert!(answerer.get_srtp_key().is_some());
    
    // The keys should match
    assert_eq!(
        offerer.get_srtp_key().unwrap().key(),
        answerer.get_srtp_key().unwrap().key()
    );
    
    assert_eq!(
        offerer.get_srtp_key().unwrap().salt(),
        answerer.get_srtp_key().unwrap().salt()
    );
}

#[test]
fn test_sdes_multiple_crypto_suites() {
    // Configure offerer with multiple crypto suites
    let offerer_config = SdesConfig {
        crypto_suites: vec![SRTP_AES128_CM_SHA1_80, SRTP_AES128_CM_SHA1_32],
        offer_count: 2,
    };
    
    let mut offerer = Sdes::new(offerer_config, SdesRole::Offerer);
    
    // Configure answerer with preference for the second crypto suite
    let answerer_config = SdesConfig {
        crypto_suites: vec![SRTP_AES128_CM_SHA1_32, SRTP_AES128_CM_SHA1_80],
        offer_count: 1,
    };
    
    let mut answerer = Sdes::new(answerer_config, SdesRole::Answerer);
    
    // Initialize key exchange
    offerer.init().expect("Failed to initialize offerer");
    answerer.init().expect("Failed to initialize answerer");
    
    // Offerer creates offer
    let offer_result = offerer.process_message(b"").expect("Failed to create offer");
    assert!(offer_result.is_some());
    let offer = offer_result.unwrap();
    
    // Convert offer to string
    let offer_str = std::str::from_utf8(&offer).expect("Offer is not valid UTF-8");
    
    // Offer should contain both crypto suites
    assert!(offer_str.contains("AES_CM_128_HMAC_SHA1_80"));
    assert!(offer_str.contains("AES_CM_128_HMAC_SHA1_32"));
    
    // Answerer processes offer and creates answer
    let answer_result = answerer.process_message(&offer).expect("Failed to process offer");
    assert!(answer_result.is_some());
    let answer = answer_result.unwrap();
    
    // Convert answer to string
    let answer_str = std::str::from_utf8(&answer).expect("Answer is not valid UTF-8");
    
    // Answer should select the first offered crypto suite (tag=1)
    assert!(answer_str.contains("a=crypto:1"));
    
    // Offerer processes answer
    offerer.process_message(&answer).expect("Failed to process answer");
    
    // Verify both sides have completed the exchange
    assert!(offerer.is_complete());
    assert!(answerer.is_complete());
    
    // Verify crypto suites match
    assert_eq!(
        offerer.get_srtp_suite().unwrap().tag_length,
        answerer.get_srtp_suite().unwrap().tag_length
    );
}

#[test]
fn test_sdes_error_handling() {
    // Test with empty offer
    let answerer_config = SdesConfig::default();
    let mut answerer = Sdes::new(answerer_config, SdesRole::Answerer);
    
    let result = answerer.process_message(b"");
    assert!(result.is_err());
    
    // Test with invalid crypto attribute
    let invalid_offer = b"a=crypto:1 INVALID_SUITE inline:PS1uQCVeeCFCanVmcjkpPywjNWhcYD0mXXtxaVBR";
    let result = answerer.process_message(invalid_offer);
    assert!(result.is_err());
} 