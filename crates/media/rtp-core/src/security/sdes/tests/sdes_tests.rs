use crate::security::sdes::{Sdes, SdesConfig, SdesCryptoAttribute, SdesRole, SdesState};
use crate::security::SecurityKeyExchange;
use crate::srtp::{SRTP_AES128_CM_SHA1_32, SRTP_AES128_CM_SHA1_80};
use crate::Error;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

#[test]
fn test_sdes_crypto_attribute_parsing() {
    // Test parsing a valid crypto attribute
    let attr_str = "1 AES_CM_128_HMAC_SHA1_80 inline:PS1uQCVeeCFCanVmcjkpPywjNWhcYD0mXXtxaVBR";
    let attr =
        SdesCryptoAttribute::parse(attr_str).expect("Failed to parse valid crypto attribute");

    assert_eq!(attr.tag, 1);
    assert_eq!(attr.crypto_suite, "AES_CM_128_HMAC_SHA1_80");
    assert_eq!(attr.key_method, "inline");
    assert_eq!(attr.key_info, "PS1uQCVeeCFCanVmcjkpPywjNWhcYD0mXXtxaVBR");
    assert!(attr.session_params.is_empty());

    // Test parsing with session parameters
    let attr_str = "2 AES_CM_128_HMAC_SHA1_32 inline:PS1uQCVeeCFCanVmcjkpPywjNWhcYD0mXXtxaVBR KDR=1;UNENCRYPTED_SRTP";
    let attr =
        SdesCryptoAttribute::parse(attr_str).expect("Failed to parse crypto attribute with params");

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
    let offer_result = offerer
        .process_message(b"")
        .expect("Failed to create offer");
    assert!(offer_result.is_some());
    let offer = offer_result.unwrap();

    // Convert offer bytes to string for inspection
    let offer_str = std::str::from_utf8(&offer).expect("Offer is not valid UTF-8");
    println!("SDP Offer: {}", offer_str);

    // Offer should contain crypto lines
    assert!(offer_str.contains("a=crypto:"));
    assert!(offer_str.contains("AES_CM_128_HMAC_SHA1_80"));

    // Answerer processes offer and creates answer
    let answer_result = answerer
        .process_message(&offer)
        .expect("Failed to process offer");
    assert!(answer_result.is_some());
    let answer = answer_result.unwrap();

    // Convert answer bytes to string for inspection
    let answer_str = std::str::from_utf8(&answer).expect("Answer is not valid UTF-8");
    println!("SDP Answer: {}", answer_str);

    // Answer should contain exactly one crypto line
    assert!(answer_str.contains("a=crypto:"));

    // Offerer processes answer
    offerer
        .process_message(&answer)
        .expect("Failed to process answer");

    // Verify both sides have completed the exchange
    assert!(offerer.is_complete());
    assert!(answerer.is_complete());

    let offerer_keys = offerer
        .get_directional_keys()
        .expect("offerer directional keys");
    let answerer_keys = answerer
        .get_directional_keys()
        .expect("answerer directional keys");

    // Every endpoint advertises a fresh transmit key, and each peer installs
    // that key only on its receive side.
    assert_ne!(offerer_keys.local_tx.key(), answerer_keys.local_tx.key());
    assert_ne!(offerer_keys.local_tx.salt(), answerer_keys.local_tx.salt());
    assert_eq!(offerer_keys.local_tx.key(), answerer_keys.remote_rx.key());
    assert_eq!(offerer_keys.local_tx.salt(), answerer_keys.remote_rx.salt());
    assert_eq!(answerer_keys.local_tx.key(), offerer_keys.remote_rx.key());
    assert_eq!(answerer_keys.local_tx.salt(), offerer_keys.remote_rx.salt());

    // The compatibility accessor remains the local transmit key.
    assert_eq!(
        offerer.get_srtp_key().unwrap().key(),
        offerer_keys.local_tx.key()
    );
    assert_eq!(
        answerer.get_srtp_key().unwrap().key(),
        answerer_keys.local_tx.key()
    );
}

#[test]
fn test_sdes_answer_selects_the_matching_offered_transmit_key() {
    let mut offerer = Sdes::new(SdesConfig::default(), SdesRole::Offerer);
    let offer = offerer
        .process_message(b"")
        .expect("create offer")
        .expect("offer body");
    let offer = std::str::from_utf8(&offer).expect("UTF-8 offer");
    let tag_two = offer
        .lines()
        .find(|line| line.starts_with("a=crypto:2 "))
        .expect("second offered crypto attribute");
    let offered = SdesCryptoAttribute::parse(tag_two.trim_start_matches("a=crypto:"))
        .expect("parse second offered attribute");
    let offered_key = BASE64
        .decode(&offered.key_info)
        .expect("decode offered key");

    let remote_key = vec![0x5a; 30];
    let answer = format!(
        "a=crypto:2 AES_CM_128_HMAC_SHA1_32 inline:{}",
        BASE64.encode(&remote_key)
    );
    offerer
        .process_message(answer.as_bytes())
        .expect("process tag-two answer");

    let directional = offerer.get_directional_keys().expect("directional keys");
    assert_eq!(directional.local_tx.key(), &offered_key[..16]);
    assert_eq!(directional.local_tx.salt(), &offered_key[16..30]);
    assert_eq!(directional.remote_rx.key(), &remote_key[..16]);
    assert_eq!(directional.remote_rx.salt(), &remote_key[16..30]);
    assert_eq!(offerer.get_srtp_suite(), Some(SRTP_AES128_CM_SHA1_32));
}

#[test]
fn test_sdes_answer_cannot_change_the_suite_for_an_offered_tag() {
    let mut offerer = Sdes::new(SdesConfig::default(), SdesRole::Offerer);
    offerer
        .process_message(b"")
        .expect("create offer")
        .expect("offer body");

    let answer = format!(
        "a=crypto:2 AES_CM_128_HMAC_SHA1_80 inline:{}",
        BASE64.encode(vec![0x7c; 30])
    );
    let error = offerer
        .process_message(answer.as_bytes())
        .expect_err("answer must preserve the suite associated with its tag");
    assert!(error.to_string().contains("changed suite"));
    assert!(!offerer.is_complete());
    assert!(offerer.get_directional_keys().is_err());
}

#[test]
fn test_sdes_directional_keys_fail_closed_without_remote_material() {
    let mut offerer = Sdes::new(SdesConfig::default(), SdesRole::Offerer);
    offerer
        .process_message(b"")
        .expect("create offer")
        .expect("offer body");

    // The compatibility accessor may expose the local offer key, but callers
    // cannot obtain a usable directional pair until a peer key is validated.
    assert!(offerer.get_srtp_key().is_some());
    assert!(offerer.get_remote_srtp_key().is_none());
    let error = offerer
        .get_directional_keys()
        .expect_err("an incomplete exchange must not yield usable key material");
    assert!(error.to_string().contains("remote receive key"));
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
    let offer_result = offerer
        .process_message(b"")
        .expect("Failed to create offer");
    assert!(offer_result.is_some());
    let offer = offer_result.unwrap();

    // Convert offer to string
    let offer_str = std::str::from_utf8(&offer).expect("Offer is not valid UTF-8");

    // Offer should contain both crypto suites
    assert!(offer_str.contains("AES_CM_128_HMAC_SHA1_80"));
    assert!(offer_str.contains("AES_CM_128_HMAC_SHA1_32"));

    // Answerer processes offer and creates answer
    let answer_result = answerer
        .process_message(&offer)
        .expect("Failed to process offer");
    assert!(answer_result.is_some());
    let answer = answer_result.unwrap();

    // Convert answer to string
    let answer_str = std::str::from_utf8(&answer).expect("Answer is not valid UTF-8");

    // Answer should select the first offered crypto suite (tag=1)
    assert!(answer_str.contains("a=crypto:1"));

    // Offerer processes answer
    offerer
        .process_message(&answer)
        .expect("Failed to process answer");

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

#[test]
fn unsupported_sdes_parameters_fail_without_answerer_state_or_key_mutation() {
    let base = "a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:PS1uQCVeeCFCanVmcjkpPywjNWhcYD0mXXtxaVBR";
    let offers = [
        format!("{base} KDR=1"),
        format!("{base} UNENCRYPTED_SRTP"),
        format!("{base} UNENCRYPTED_SRTCP"),
        format!("{base} UNAUTHENTICATED_SRTP"),
        format!("{base} FEC_ORDER=SRTP_FEC"),
        format!("{base}|2^20"),
        format!("{base}|1:4"),
    ];

    for offer in offers {
        let mut answerer = Sdes::new(SdesConfig::default(), SdesRole::Answerer);
        let result = answerer.process_message(offer.as_bytes());
        assert!(
            matches!(result, Err(Error::UnsupportedFeature(_))),
            "unexpected result for {offer:?}: {result:?}"
        );
        assert_eq!(answerer.state, SdesState::Initial);
        assert!(answerer.local_attrs.is_empty());
        assert!(answerer.remote_attrs.is_empty());
        assert!(answerer.selected_attr.is_none());
        assert!(answerer.local_srtp_key.is_none());
        assert!(answerer.remote_srtp_key.is_none());
        assert!(answerer.srtp_suite.is_none());
    }
}

#[test]
fn unsupported_sdes_parameters_fail_without_offerer_state_or_key_mutation() {
    let suffixes = [
        " KDR=1",
        " UNENCRYPTED_SRTP",
        " UNENCRYPTED_SRTCP",
        " UNAUTHENTICATED_SRTP",
        " FEC_ORDER=FEC_SRTP",
        "|2^20",
        "|1:4",
    ];

    for suffix in suffixes {
        let mut offerer = Sdes::new(SdesConfig::default(), SdesRole::Offerer);
        let offer = offerer.process_message(b"").unwrap().unwrap();
        let first_line = std::str::from_utf8(&offer).unwrap().lines().next().unwrap();
        let invalid_answer = format!("{first_line}{suffix}");

        let before_state = offerer.state.clone();
        let before_key = offerer.get_srtp_key().unwrap();
        let before_suite = offerer.get_srtp_suite();
        let before_local_attrs = offerer.local_attrs.clone();
        let result = offerer.process_message(invalid_answer.as_bytes());
        assert!(
            matches!(result, Err(Error::UnsupportedFeature(_))),
            "unexpected result for {invalid_answer:?}: {result:?}"
        );
        assert_eq!(offerer.state, before_state);
        assert_eq!(offerer.state, SdesState::OfferSent);
        assert_eq!(offerer.local_attrs, before_local_attrs);
        assert!(offerer.remote_attrs.is_empty());
        assert!(offerer.selected_attr.is_none());
        assert!(offerer.remote_srtp_key.is_none());
        assert_eq!(offerer.get_srtp_key().unwrap().key(), before_key.key());
        assert_eq!(offerer.get_srtp_key().unwrap().salt(), before_key.salt());
        assert_eq!(offerer.get_srtp_suite(), before_suite);
    }
}
