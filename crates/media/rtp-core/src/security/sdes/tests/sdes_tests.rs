use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

use crate::security::sdes::{SdesCryptoAttribute, SdesNegotiator};
use crate::srtp::{
    SrtpCryptoSuite, SRTP_AES128_CM_SHA1_32, SRTP_AES128_CM_SHA1_80, SRTP_AES256_CM_SHA1_32,
    SRTP_AES256_CM_SHA1_80,
};

fn all_suites() -> Vec<SrtpCryptoSuite> {
    vec![
        SRTP_AES128_CM_SHA1_80,
        SRTP_AES128_CM_SHA1_32,
        SRTP_AES256_CM_SHA1_80,
        SRTP_AES256_CM_SHA1_32,
    ]
}

/// For every suite this module supports, a full offer/answer exchange
/// must produce two *independently* generated keys (not the same key
/// reused for both directions — the bug this module replaces), and both
/// directions of real SRTP traffic must work.
#[test]
fn full_exchange_produces_independent_directional_keys_for_every_suite() {
    use crate::packet::RtpPacket;
    use bytes::Bytes;

    for suite in all_suites() {
        let (offerer, offer_attrs) = SdesNegotiator::new_offerer(&[suite.clone()]).unwrap();
        let answerer = SdesNegotiator::new_answerer();

        let (answer_attr, answerer_pair) = answerer.process_offer(&offer_attrs).unwrap();
        assert_eq!(answer_attr.tag, offer_attrs[0].tag);
        assert_eq!(answer_attr.suite, suite);

        let offerer_pair = offerer.accept_answer(&answer_attr).unwrap();

        let mut offerer_send = offerer_pair.send_ctx;
        let mut answerer_send = answerer_pair.send_ctx;

        // The whole point: the two sides must NOT have derived the same
        // key from a single shared secret. If they had, encrypting the
        // identical plaintext under identical header fields would
        // produce identical ciphertext.
        let probe = RtpPacket::new_with_payload(0, 1, 1, 1, Bytes::from_static(b"probe"));
        let offerer_probe_wire = offerer_send.protect(&probe).unwrap().serialize().unwrap();
        let answerer_probe_wire = answerer_send.protect(&probe).unwrap().serialize().unwrap();
        assert_ne!(
            offerer_probe_wire, answerer_probe_wire,
            "offerer and answerer must use independently generated keys for suite {suite:?}"
        );

        // Re-derive fresh contexts for the real directional test below —
        // the probe above already advanced each context's internal
        // sequence/ROC bookkeeping by one packet, which is fine, but
        // starting the real assertions from a clean pair keeps this test
        // easy to read.
        let (offerer, offer_attrs) = SdesNegotiator::new_offerer(&[suite.clone()]).unwrap();
        let answerer = SdesNegotiator::new_answerer();
        let (answer_attr, answerer_pair) = answerer.process_offer(&offer_attrs).unwrap();
        let offerer_pair = offerer.accept_answer(&answer_attr).unwrap();

        // offerer -> answerer: offerer encrypts with its own key,
        // answerer decrypts with what it stored as the peer's key.
        let mut offerer_send = offerer_pair.send_ctx;
        let mut answerer_recv = answerer_pair.recv_ctx;
        let packet = RtpPacket::new_with_payload(
            96,
            1000,
            12345,
            0xdead_beef,
            Bytes::from_static(b"hello from offerer"),
        );
        let protected = offerer_send.protect(&packet).unwrap();
        let wire = protected.serialize().unwrap();
        let decrypted = answerer_recv.unprotect(&wire).unwrap();
        assert_eq!(decrypted.payload, packet.payload, "suite {suite:?}");

        // answerer -> offerer: the other direction.
        let mut answerer_send = answerer_pair.send_ctx;
        let mut offerer_recv = offerer_pair.recv_ctx;
        let packet2 = RtpPacket::new_with_payload(
            96,
            2000,
            54321,
            0xface_d00d,
            Bytes::from_static(b"hello from answerer"),
        );
        let protected2 = answerer_send.protect(&packet2).unwrap();
        let wire2 = protected2.serialize().unwrap();
        let decrypted2 = offerer_recv.unprotect(&wire2).unwrap();
        assert_eq!(decrypted2.payload, packet2.payload, "suite {suite:?}");
    }
}

/// RFC 4568 §6.1: master key + salt must be exactly `key_length + 14`
/// bytes for every suite — not one byte short, not one byte over.
#[test]
fn key_length_is_validated_exactly_for_every_suite() {
    for suite in all_suites() {
        let expected_len = suite.key_length + 14;

        // Exactly right: accepted.
        let exact = BASE64.encode(vec![0u8; expected_len]);
        let attr = SdesCryptoAttribute::new(1, suite.clone(), exact);
        let answerer = SdesNegotiator::new_answerer();
        assert!(
            answerer.process_offer(&[attr]).is_ok(),
            "exact length must be accepted for suite {suite:?}"
        );

        // One byte short: rejected.
        let short = BASE64.encode(vec![0u8; expected_len - 1]);
        let attr = SdesCryptoAttribute::new(1, suite.clone(), short);
        let answerer = SdesNegotiator::new_answerer();
        assert!(
            answerer.process_offer(&[attr]).is_err(),
            "one byte short must be rejected for suite {suite:?}"
        );

        // One byte over: rejected.
        let long = BASE64.encode(vec![0u8; expected_len + 1]);
        let attr = SdesCryptoAttribute::new(1, suite.clone(), long);
        let answerer = SdesNegotiator::new_answerer();
        assert!(
            answerer.process_offer(&[attr]).is_err(),
            "one byte over must be rejected for suite {suite:?}"
        );
    }
}

#[test]
fn offerer_emits_one_attribute_per_suite_with_sequential_tags() {
    let suites = all_suites();
    let (_, attrs) = SdesNegotiator::new_offerer(&suites).unwrap();
    assert_eq!(attrs.len(), 4);
    for (i, attr) in attrs.iter().enumerate() {
        assert_eq!(attr.tag, (i + 1) as u32);
        assert_eq!(attr.suite, suites[i]);
    }
}

#[test]
fn answerer_honors_offerer_preference_order() {
    let (_, offer_attrs) =
        SdesNegotiator::new_offerer(&[SRTP_AES128_CM_SHA1_80, SRTP_AES256_CM_SHA1_80]).unwrap();
    let answerer = SdesNegotiator::new_answerer();
    let (chosen, pair) = answerer.process_offer(&offer_attrs).unwrap();
    assert_eq!(chosen.tag, 1, "answerer should honor offerer order");
    assert_eq!(chosen.suite, SRTP_AES128_CM_SHA1_80);
    assert_eq!(pair.suite, SRTP_AES128_CM_SHA1_80);
}

#[test]
fn accept_answer_rejects_unknown_tag() {
    let (offerer, _) = SdesNegotiator::new_offerer(&[SRTP_AES128_CM_SHA1_80]).unwrap();
    let bogus = SdesCryptoAttribute::new(99, SRTP_AES128_CM_SHA1_80, BASE64.encode(vec![0u8; 30]));
    let result = offerer.accept_answer(&bogus);
    assert!(matches!(&result, Err(e) if format!("{e:?}").contains("was not offered")));
}

#[test]
fn accept_answer_rejects_suite_mismatch_for_known_tag() {
    let (offerer, _) = SdesNegotiator::new_offerer(&[SRTP_AES128_CM_SHA1_80]).unwrap();
    // Tag 1 was offered as _80, answerer claims _32.
    let mismatch =
        SdesCryptoAttribute::new(1, SRTP_AES128_CM_SHA1_32, BASE64.encode(vec![0u8; 30]));
    let result = offerer.accept_answer(&mismatch);
    assert!(matches!(&result, Err(e) if format!("{e:?}").contains("does not match")));
}

#[test]
fn process_offer_errors_when_no_attributes_are_available() {
    let answerer = SdesNegotiator::new_answerer();
    let result = answerer.process_offer(&[]);
    assert!(matches!(&result, Err(e) if format!("{e:?}").contains("no offered")));
}

#[test]
fn new_offerer_rejects_empty_suite_list() {
    assert!(SdesNegotiator::new_offerer(&[]).is_err());
}

#[test]
fn rejects_key_lifetime_parameter() {
    let mut attr =
        SdesCryptoAttribute::new(1, SRTP_AES128_CM_SHA1_80, BASE64.encode(vec![0u8; 30]));
    attr.key_lifetime = Some("2^20".to_string());
    let answerer = SdesNegotiator::new_answerer();
    let result = answerer.process_offer(&[attr]);
    assert!(matches!(&result, Err(e) if format!("{e:?}").contains("lifetime")));
}

#[test]
fn rejects_mki_parameter() {
    let mut attr =
        SdesCryptoAttribute::new(1, SRTP_AES128_CM_SHA1_80, BASE64.encode(vec![0u8; 30]));
    attr.key_mki = Some((1, 4));
    let answerer = SdesNegotiator::new_answerer();
    let result = answerer.process_offer(&[attr]);
    assert!(matches!(&result, Err(e) if format!("{e:?}").contains("MKI")));
}

#[test]
fn rejects_session_parameters() {
    let mut attr =
        SdesCryptoAttribute::new(1, SRTP_AES128_CM_SHA1_80, BASE64.encode(vec![0u8; 30]));
    attr.session_params = vec!["UNENCRYPTED_SRTP".to_string()];
    let answerer = SdesNegotiator::new_answerer();
    let result = answerer.process_offer(&[attr]);
    assert!(matches!(&result, Err(e) if format!("{e:?}").contains("session parameters")));
}

#[test]
fn accept_answer_rejects_unsupported_extensions_too() {
    let (offerer, _) = SdesNegotiator::new_offerer(&[SRTP_AES128_CM_SHA1_80]).unwrap();
    let mut attr =
        SdesCryptoAttribute::new(1, SRTP_AES128_CM_SHA1_80, BASE64.encode(vec![0u8; 30]));
    attr.key_mki = Some((1, 4));
    let result = offerer.accept_answer(&attr);
    assert!(matches!(&result, Err(e) if format!("{e:?}").contains("MKI")));
}
