use crate::security::mikey::{Mikey, MikeyConfig, MikeyRole, MikeyKeyExchangeMethod};
use crate::security::SecurityKeyExchange;
use crate::srtp::{SRTP_AES128_CM_SHA1_80, SRTP_AES128_CM_SHA1_32};

#[test]
fn test_mikey_init() {
    // Create pre-shared key
    let psk = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
    
    // Configure initiator
    let initiator_config = MikeyConfig {
        method: MikeyKeyExchangeMethod::Psk,
        psk: Some(psk.clone()),
        srtp_profile: SRTP_AES128_CM_SHA1_80,
        ..Default::default()
    };
    
    let mut initiator = Mikey::new(initiator_config, MikeyRole::Initiator);
    
    // Initialize initiator
    let result = initiator.init();
    assert!(result.is_ok(), "Failed to initialize initiator: {:?}", result);
}

#[test]
fn test_mikey_status_check() {
    // Create pre-shared key
    let psk = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
    
    // Configure initiator
    let initiator_config = MikeyConfig {
        method: MikeyKeyExchangeMethod::Psk,
        psk: Some(psk.clone()),
        srtp_profile: SRTP_AES128_CM_SHA1_80,
        ..Default::default()
    };
    
    let mut initiator = Mikey::new(initiator_config, MikeyRole::Initiator);
    
    // Check initial state
    assert!(!initiator.is_complete());
    assert!(initiator.get_srtp_key().is_none());
    assert!(initiator.get_srtp_suite().is_none());
    
    // Initialize
    initiator.init().expect("Failed to initialize");
    
    // Status should still be incomplete
    assert!(!initiator.is_complete());
}

#[test]
fn test_mikey_crypto_suites() {
    // Create pre-shared key
    let psk = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
    
    // Configure initiator with a specific SRTP profile
    let initiator_config = MikeyConfig {
        method: MikeyKeyExchangeMethod::Psk,
        psk: Some(psk.clone()),
        srtp_profile: SRTP_AES128_CM_SHA1_80,
        ..Default::default()
    };
    
    let mut initiator = Mikey::new(initiator_config, MikeyRole::Initiator);
    initiator.init().expect("Failed to initialize");
    
    // Create another initiator with a different SRTP profile
    let initiator_config2 = MikeyConfig {
        method: MikeyKeyExchangeMethod::Psk,
        psk: Some(psk.clone()),
        srtp_profile: SRTP_AES128_CM_SHA1_32,
        ..Default::default()
    };
    
    let mut initiator2 = Mikey::new(initiator_config2, MikeyRole::Initiator);
    initiator2.init().expect("Failed to initialize");
    
    // If the implementation was complete, we could test the derived keys,
    // but for now we'll just verify the basic APIs work.
    assert!(!initiator.is_complete());
    assert!(!initiator2.is_complete());
} 