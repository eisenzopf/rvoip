use crate::packet::{RtpHeader, RtpPacket};
use crate::security::{
    sdes::{Sdes, SdesConfig, SdesRole},
    SecurityKeyExchange,
};
use crate::srtp::{SrtpContext, SRTP_AES128_CM_SHA1_80};
use bytes::Bytes;

#[test]
fn test_srtp_with_sdes_key_exchange() {
    // 1. Set up SDES key exchange

    // Configure offerer
    let offerer_config = SdesConfig {
        crypto_suites: vec![SRTP_AES128_CM_SHA1_80],
        offer_count: 1,
    };

    let mut offerer = Sdes::new(offerer_config, SdesRole::Offerer);

    // Configure answerer
    let answerer_config = SdesConfig {
        crypto_suites: vec![SRTP_AES128_CM_SHA1_80],
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
    let offer = offer_result.unwrap();

    // Answerer processes offer and creates answer
    let answer_result = answerer
        .process_message(&offer)
        .expect("Failed to process offer");
    let answer = answer_result.unwrap();

    // Offerer processes answer
    offerer
        .process_message(&answer)
        .expect("Failed to process answer");

    // Verify both sides have completed the exchange
    assert!(offerer.is_complete());
    assert!(answerer.is_complete());

    // 2. Use the negotiated keys with SRTP

    // Create directional SRTP contexts. Each endpoint protects with its own
    // advertised key and unprotects with the peer's advertised key.
    let offerer_keys = offerer.get_directional_keys().unwrap();
    let answerer_keys = answerer.get_directional_keys().unwrap();
    let suite = offerer.get_srtp_suite().unwrap();
    let mut offerer_srtp =
        SrtpContext::new_directional(suite.clone(), offerer_keys.local_tx, offerer_keys.remote_rx)
            .expect("Failed to create offerer SRTP context");

    let mut answerer_srtp =
        SrtpContext::new_directional(suite, answerer_keys.local_tx, answerer_keys.remote_rx)
            .expect("Failed to create answerer SRTP context");

    // 3. Test SRTP encryption and decryption

    // Create a test RTP packet
    let header = RtpHeader::new(96, 1000, 12345, 0xabcdef01);
    let payload = Bytes::from_static(b"Hello secure RTP world!");
    let packet = RtpPacket::new(header, payload);

    // Encrypt with offerer's context
    let protected = offerer_srtp
        .protect(&packet)
        .expect("Failed to protect RTP packet");

    // Serialize the protected packet
    let protected_bytes = protected
        .serialize()
        .expect("Failed to serialize protected packet");

    // Decrypt with answerer's context
    let decrypted = answerer_srtp
        .unprotect(&protected_bytes)
        .expect("Failed to unprotect RTP packet");

    // Verify decrypted packet matches original
    assert_eq!(decrypted.header.payload_type, packet.header.payload_type);
    assert_eq!(
        decrypted.header.sequence_number,
        packet.header.sequence_number
    );
    assert_eq!(decrypted.header.timestamp, packet.header.timestamp);
    assert_eq!(decrypted.header.ssrc, packet.header.ssrc);
    assert_eq!(decrypted.payload, packet.payload);

    // The answerer's independent transmit key works in the reverse direction.
    let reverse = RtpPacket::new(
        RtpHeader::new(96, 1001, 12505, 0xabcdef02),
        Bytes::from_static(b"Hello back from the answerer!"),
    );
    let protected = answerer_srtp
        .protect(&reverse)
        .expect("Failed to protect reverse RTP packet")
        .serialize()
        .expect("Failed to serialize reverse RTP packet");
    let decrypted = offerer_srtp
        .unprotect(&protected)
        .expect("Failed to unprotect reverse RTP packet");
    assert_eq!(decrypted.payload, reverse.payload);

    // SRTCP uses the same directional material in both directions.
    let mut offerer_report = vec![0x80, 200, 0, 6];
    offerer_report.extend_from_slice(&0xabcdef01_u32.to_be_bytes());
    offerer_report.extend_from_slice(&[0x11; 20]);
    let protected = offerer_srtp
        .protect_rtcp(&offerer_report)
        .expect("Failed to protect offerer RTCP");
    assert_eq!(
        answerer_srtp
            .unprotect_rtcp(&protected)
            .expect("Failed to unprotect offerer RTCP")
            .as_ref(),
        offerer_report
    );

    let mut answerer_report = vec![0x80, 200, 0, 6];
    answerer_report.extend_from_slice(&0xabcdef02_u32.to_be_bytes());
    answerer_report.extend_from_slice(&[0x22; 20]);
    let protected = answerer_srtp
        .protect_rtcp(&answerer_report)
        .expect("Failed to protect answerer RTCP");
    assert_eq!(
        offerer_srtp
            .unprotect_rtcp(&protected)
            .expect("Failed to unprotect answerer RTCP")
            .as_ref(),
        answerer_report
    );
}

#[test]
fn mikey_fails_closed_before_srtp_setup() {
    use crate::security::mikey::{Mikey, MikeyConfig, MikeyKeyExchangeMethod, MikeyRole};

    let config = MikeyConfig {
        method: MikeyKeyExchangeMethod::Psk,
        psk: Some(vec![0x31; 16]),
        srtp_profile: SRTP_AES128_CM_SHA1_80,
        ..Default::default()
    };

    assert!(matches!(
        Mikey::try_new(config.clone(), MikeyRole::Initiator),
        Err(crate::Error::UnsupportedFeature(_))
    ));

    let mut compatibility_instance = Mikey::new(config, MikeyRole::Initiator);
    assert!(matches!(
        compatibility_instance.init(),
        Err(crate::Error::UnsupportedFeature(_))
    ));
    assert!(compatibility_instance.get_srtp_key().is_none());
    assert!(compatibility_instance.get_srtp_suite().is_none());
}

#[test]
fn test_zrtp_fails_closed_before_srtp_setup() {
    use crate::security::zrtp::{Zrtp, ZrtpConfig, ZrtpRole};

    assert!(matches!(
        Zrtp::try_new(ZrtpConfig::default(), ZrtpRole::Initiator),
        Err(crate::Error::UnsupportedFeature(_))
    ));

    let mut compatibility_instance = Zrtp::new(ZrtpConfig::default(), ZrtpRole::Initiator);
    assert!(matches!(
        compatibility_instance.init(),
        Err(crate::Error::UnsupportedFeature(_))
    ));
    assert!(compatibility_instance.get_srtp_key().is_none());
    assert!(compatibility_instance.get_srtp_suite().is_none());
}
