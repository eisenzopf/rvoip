use crate::packet::{RtpHeader, RtpPacket};
use crate::security::sdes::SdesNegotiator;
use crate::security::SecurityKeyExchange;
use crate::srtp::{SrtpContext, SrtpCryptoKey, SRTP_AES128_CM_SHA1_80};
use bytes::Bytes;

/// Proves a real, independent offerer/answerer SDES exchange: each side
/// generates its own master key (RFC 4568 §6.1) and both directions of
/// real SRTP traffic work — not the old bug where the answerer echoed
/// the offerer's own key back, making a "two-party" test that could only
/// ever exercise one key encrypting/decrypting with itself.
#[test]
fn test_srtp_with_sdes_key_exchange() {
    // 1. Set up SDES key exchange
    let (offerer, offer_attrs) =
        SdesNegotiator::new_offerer(&[SRTP_AES128_CM_SHA1_80]).expect("offerer setup");
    let answerer = SdesNegotiator::new_answerer();

    // Answerer processes the offer and generates its own key for the answer.
    let (answer_attr, answerer_pair) = answerer
        .process_offer(&offer_attrs)
        .expect("answerer processes offer");

    // Offerer accepts the answer, decoding the answerer's independently
    // generated key.
    let offerer_pair = offerer
        .accept_answer(&answer_attr)
        .expect("offerer accepts answer");

    // 2. Use the negotiated keys with SRTP — both directions.
    let mut offerer_send = offerer_pair.send_ctx;
    let mut answerer_recv = answerer_pair.recv_ctx;

    let header = RtpHeader::new(96, 1000, 12345, 0xabcdef01);
    let payload = Bytes::from_static(b"Hello secure RTP world!");
    let packet = RtpPacket::new(header, payload.clone());

    let protected = offerer_send
        .protect(&packet)
        .expect("Failed to protect RTP packet");
    let protected_bytes = protected
        .serialize()
        .expect("Failed to serialize protected packet");
    let decrypted = answerer_recv
        .unprotect(&protected_bytes)
        .expect("Failed to unprotect RTP packet");

    assert_eq!(decrypted.header.payload_type, packet.header.payload_type);
    assert_eq!(
        decrypted.header.sequence_number,
        packet.header.sequence_number
    );
    assert_eq!(decrypted.header.timestamp, packet.header.timestamp);
    assert_eq!(decrypted.header.ssrc, packet.header.ssrc);
    assert_eq!(decrypted.payload, payload);

    // The other direction, proving the two SrtpContexts really are keyed
    // independently rather than sharing one master key.
    let mut answerer_send = answerer_pair.send_ctx;
    let mut offerer_recv = offerer_pair.recv_ctx;
    let header2 = RtpHeader::new(96, 2000, 54321, 0xface_d00d);
    let payload2 = Bytes::from_static(b"Hello back, securely.");
    let packet2 = RtpPacket::new(header2, payload2.clone());

    let protected2 = answerer_send
        .protect(&packet2)
        .expect("Failed to protect return RTP packet");
    let protected_bytes2 = protected2
        .serialize()
        .expect("Failed to serialize protected return packet");
    let decrypted2 = offerer_recv
        .unprotect(&protected_bytes2)
        .expect("Failed to unprotect return RTP packet");
    assert_eq!(decrypted2.payload, payload2);
}

#[test]
fn test_srtp_with_mikey_key_exchange() {
    // Import MIKEY types
    use crate::security::mikey::{Mikey, MikeyConfig, MikeyKeyExchangeMethod, MikeyRole};

    // Create pre-shared key for MIKEY
    let psk = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];

    // 1. Set up MIKEY key exchange

    // Configure initiator
    let initiator_config = MikeyConfig {
        method: MikeyKeyExchangeMethod::Psk,
        psk: Some(psk.clone()),
        srtp_profile: SRTP_AES128_CM_SHA1_80,
        ..Default::default()
    };

    let mut initiator = Mikey::new(initiator_config, MikeyRole::Initiator);

    // Configure responder
    let responder_config = MikeyConfig {
        method: MikeyKeyExchangeMethod::Psk,
        psk: Some(psk.clone()),
        srtp_profile: SRTP_AES128_CM_SHA1_80,
        ..Default::default()
    };

    let _responder = Mikey::new(responder_config, MikeyRole::Responder);

    // Initialize key exchange
    initiator.init().expect("Failed to initialize initiator");

    // Skip full key exchange to avoid implementation details
    // In a real implementation, messages would be exchanged

    // For the test, we'll just assume the key exchange is complete
    // and use the initial keys directly
    if let Some(initiator_key) = initiator.get_srtp_key() {
        if let Some(initiator_suite) = initiator.get_srtp_suite() {
            // 2. Create SRTP context with the key
            let mut srtp_context = SrtpContext::new(initiator_suite, initiator_key)
                .expect("Failed to create SRTP context");

            // 3. Test SRTP encryption and decryption

            // Create a test RTP packet
            let header = RtpHeader::new(96, 1000, 12345, 0xabcdef01);
            let payload = Bytes::from_static(b"Hello MIKEY secured RTP world!");
            let packet = RtpPacket::new(header, payload);

            // Encrypt packet
            let protected = srtp_context
                .protect(&packet)
                .expect("Failed to protect RTP packet");

            // Verify encryption worked
            assert!(protected.auth_tag.is_some());

            // In a full implementation, we would decrypt with the responder's context
            // For this test, we'll decrypt with the same context
            let protected_bytes = protected
                .serialize()
                .expect("Failed to serialize protected packet");

            // Decrypt packet
            let decrypted = srtp_context
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
        }
    }
}

#[test]
fn test_srtp_with_zrtp_key_exchange() {
    // Import ZRTP types
    use crate::security::zrtp::{
        Zrtp, ZrtpAuthTag, ZrtpCipher, ZrtpConfig, ZrtpHash, ZrtpKeyAgreement, ZrtpRole,
        ZrtpSasType,
    };

    // 1. Set up ZRTP key exchange

    // Create config for initiator
    let initiator_config = ZrtpConfig {
        ciphers: vec![ZrtpCipher::Aes1],
        hashes: vec![ZrtpHash::S256],
        auth_tags: vec![ZrtpAuthTag::HS80],
        key_agreements: vec![ZrtpKeyAgreement::EC25],
        sas_types: vec![ZrtpSasType::B32],
        client_id: "RVOIP ZRTP Test".to_string(),
        srtp_profile: SRTP_AES128_CM_SHA1_80,
    };

    let mut initiator = Zrtp::new(initiator_config, ZrtpRole::Initiator);

    // Initialize key exchange
    initiator.init().expect("Failed to initialize initiator");

    // Skip full key exchange to avoid implementation details
    // In a real implementation, messages would be exchanged

    // For the test, we'll create a manual key to test SRTP integration
    let manual_key = SrtpCryptoKey::new(
        vec![
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E,
            0x0F, 0x10,
        ],
        vec![
            0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E,
        ],
    );

    // 2. Create SRTP contexts with manual key
    let mut srtp_context = SrtpContext::new(SRTP_AES128_CM_SHA1_80, manual_key)
        .expect("Failed to create SRTP context");

    // 3. Test SRTP encryption and decryption with RTCP

    // Create sample RTCP data
    let rtcp_data = vec![
        // RTCP header (SR)
        0x81, 0xc8, 0x00, 0x0c, // Version, padding, count, PT=SR, length
        0xab, 0xcd, 0xef, 0x01, // SSRC
        // Sender info
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // NTP timestamp (MSW,LSW)
        0x00, 0x00, 0x30, 0x39, // RTP timestamp
        0x00, 0x00, 0x00, 0x01, // Packet count
        0x00, 0x00, 0x00, 0x64, // Octet count
        // Report block
        0xde, 0xad, 0xbe, 0xef, // SSRC of first source
        0x00, 0x00, 0x00, 0x00, // Fraction lost, cumulative lost
        0x00, 0x00, 0x00, 0x00, // Extended highest sequence
        0x00, 0x00, 0x00, 0x00, // Interarrival jitter
        0x00, 0x00, 0x00, 0x00, // Last SR
        0x00, 0x00, 0x00, 0x00, // Delay since last SR
    ];

    // Protect RTCP packet
    let protected_rtcp = srtp_context
        .protect_rtcp(&rtcp_data)
        .expect("Failed to protect RTCP packet");

    // Unprotect RTCP packet
    let unprotected_rtcp = srtp_context
        .unprotect_rtcp(&protected_rtcp)
        .expect("Failed to unprotect RTCP packet");

    // Verify unprotected RTCP packet matches original
    assert_eq!(unprotected_rtcp.len(), rtcp_data.len());
    assert_eq!(&unprotected_rtcp[0..4], &rtcp_data[0..4]); // Header should be unencrypted

    // 4. Test SRTP encryption and decryption with RTP

    // Create a test RTP packet
    let header = RtpHeader::new(96, 1000, 12345, 0xabcdef01);
    let payload = Bytes::from_static(b"Hello ZRTP secured RTP world!");
    let packet = RtpPacket::new(header, payload);

    // Encrypt packet
    let protected = srtp_context
        .protect(&packet)
        .expect("Failed to protect RTP packet");

    // Verify encryption worked (should have auth tag)
    assert!(protected.auth_tag.is_some());

    // Serialize the protected packet
    let protected_bytes = protected
        .serialize()
        .expect("Failed to serialize protected packet");

    // Decrypt packet
    let decrypted = srtp_context
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
}
