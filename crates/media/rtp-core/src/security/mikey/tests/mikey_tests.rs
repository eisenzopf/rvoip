use crate::security::mikey::{Mikey, MikeyConfig, MikeyKeyExchangeMethod, MikeyRole, MikeyState};
use crate::security::SecurityKeyExchange;
use crate::srtp::{SRTP_AES128_CM_SHA1_32, SRTP_AES128_CM_SHA1_80};

#[test]
fn checked_construction_rejects_incomplete_mikey_modes() {
    for method in [
        MikeyKeyExchangeMethod::Psk,
        MikeyKeyExchangeMethod::Pk,
        MikeyKeyExchangeMethod::Dh,
    ] {
        let config = MikeyConfig {
            method,
            ..Default::default()
        };
        assert!(matches!(
            Mikey::try_new(config, MikeyRole::Initiator),
            Err(crate::Error::UnsupportedFeature(_))
        ));
    }
}

#[test]
fn mikey_psk_operations_fail_before_mutating_state_or_keys() {
    let initiator_config = MikeyConfig {
        method: MikeyKeyExchangeMethod::Psk,
        psk: Some(vec![0x42; 16]),
        srtp_profile: SRTP_AES128_CM_SHA1_80,
        ..Default::default()
    };

    let mut initiator = Mikey::new(initiator_config, MikeyRole::Initiator);
    assert_eq!(initiator.state, MikeyState::Initial);
    assert!(!initiator.is_complete());
    assert!(initiator.get_srtp_key().is_none());
    assert!(initiator.get_srtp_suite().is_none());

    for result in [
        initiator.init(),
        initiator.init(),
        initiator
            .process_message(b"untrusted MIKEY message")
            .map(|_| ()),
    ] {
        assert!(matches!(result, Err(crate::Error::UnsupportedFeature(_))));
        assert_eq!(initiator.state, MikeyState::Initial);
        assert!(!initiator.is_complete());
        assert!(initiator.get_srtp_key().is_none());
        assert!(initiator.get_srtp_suite().is_none());
        assert!(initiator.rand_i.is_none());
        assert!(initiator.rand_r.is_none());
        assert!(initiator.generated_tek.is_none());
        assert!(initiator.generated_salt.is_none());
    }

    // Profile selection cannot make the unavailable PSK exchange usable.
    let config = MikeyConfig {
        method: MikeyKeyExchangeMethod::Psk,
        psk: Some(vec![0x24; 16]),
        srtp_profile: SRTP_AES128_CM_SHA1_32,
        ..Default::default()
    };
    assert!(matches!(
        Mikey::try_new(config, MikeyRole::Responder),
        Err(crate::Error::UnsupportedFeature(_))
    ));
}

#[test]
fn test_mikey_pke_certificate_generation() {
    use crate::security::mikey::crypto::{generate_key_pair_and_certificate, CertificateConfig};

    // Create certificate configuration
    let config = CertificateConfig::enterprise_server("test-server.example.com");

    // Generate certificate and key pair
    let result = generate_key_pair_and_certificate(config);
    assert!(
        result.is_ok(),
        "Failed to generate certificate: {:?}",
        result
    );

    let key_pair = result.unwrap();

    // Verify we have all required components
    assert!(
        !key_pair.certificate.is_empty(),
        "Certificate should not be empty"
    );
    assert!(
        !key_pair.private_key.is_empty(),
        "Private key should not be empty"
    );
    assert!(
        !key_pair.public_key.is_empty(),
        "Public key should not be empty"
    );

    // Verify certificate is parseable
    use crate::security::mikey::crypto::extract_certificate_info;
    let cert_info = extract_certificate_info(&key_pair.certificate);
    assert!(
        cert_info.is_ok(),
        "Certificate should be parseable: {:?}",
        cert_info
    );

    let info = cert_info.unwrap();
    assert_eq!(info.subject_cn, "test-server.example.com");
}

#[test]
fn test_mikey_pke_ca_generation() {
    use crate::security::mikey::crypto::{generate_ca_certificate, CertificateConfig};

    // Create CA configuration
    let config = CertificateConfig::high_security("Test Root CA");

    // Generate CA certificate
    let result = generate_ca_certificate(config);
    assert!(
        result.is_ok(),
        "Failed to generate CA certificate: {:?}",
        result
    );

    let ca_key_pair = result.unwrap();

    // Verify CA certificate components
    assert!(!ca_key_pair.certificate.is_empty());
    assert!(!ca_key_pair.private_key.is_empty());
    assert!(!ca_key_pair.public_key.is_empty());
}

#[test]
fn test_mikey_pke_certificate_signing_fails_closed() {
    use crate::security::mikey::crypto::{
        generate_ca_certificate, sign_certificate_with_ca, CertificateConfig,
    };

    // Generate CA
    let ca_config = CertificateConfig::enterprise_server("Test CA");
    let ca_key_pair = generate_ca_certificate(ca_config).unwrap();

    let subject_config = CertificateConfig::enterprise_client("test-user@example.com");
    let result = sign_certificate_with_ca(&ca_key_pair, subject_config);
    assert!(matches!(result, Err(crate::Error::UnsupportedFeature(_))));
}

#[test]
fn test_mikey_pke_init() {
    use crate::security::mikey::crypto::{generate_key_pair_and_certificate, CertificateConfig};

    // Generate certificates for both endpoints
    let server_config = CertificateConfig::enterprise_server("server.example.com");
    let server_keys = generate_key_pair_and_certificate(server_config).unwrap();

    let client_config = CertificateConfig::enterprise_client("client@example.com");
    let client_keys = generate_key_pair_and_certificate(client_config).unwrap();

    // Configure MIKEY-PKE initiator
    let initiator_config = MikeyConfig {
        method: MikeyKeyExchangeMethod::Pk,
        certificate: Some(server_keys.certificate.clone()),
        private_key: Some(server_keys.private_key.clone()),
        peer_certificate: Some(client_keys.certificate.clone()),
        srtp_profile: SRTP_AES128_CM_SHA1_80,
        ..Default::default()
    };

    let mut initiator = Mikey::new(initiator_config, MikeyRole::Initiator);

    // Initialize PKE mode
    let result = initiator.init();
    assert!(matches!(result, Err(crate::Error::UnsupportedFeature(_))));
    assert!(initiator.get_srtp_key().is_none());
    assert!(initiator.get_srtp_suite().is_none());
}

#[test]
fn test_mikey_pke_and_psk_modes_are_unavailable() {
    use crate::security::mikey::crypto::{generate_key_pair_and_certificate, CertificateConfig};

    // Test PSK mode
    let psk = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
    let psk_config = MikeyConfig {
        method: MikeyKeyExchangeMethod::Psk,
        psk: Some(psk),
        srtp_profile: SRTP_AES128_CM_SHA1_80,
        ..Default::default()
    };

    let mut psk_mikey = Mikey::new(psk_config, MikeyRole::Initiator);
    let psk_result = psk_mikey.init();
    assert!(matches!(
        psk_result,
        Err(crate::Error::UnsupportedFeature(_))
    ));
    assert_eq!(psk_mikey.state, MikeyState::Initial);
    assert!(psk_mikey.get_srtp_key().is_none());

    // Test PKE mode
    let server_config = CertificateConfig::enterprise_server("test.example.com");
    let server_keys = generate_key_pair_and_certificate(server_config).unwrap();

    let client_config = CertificateConfig::enterprise_client("client@example.com");
    let client_keys = generate_key_pair_and_certificate(client_config).unwrap();

    let pke_config = MikeyConfig {
        method: MikeyKeyExchangeMethod::Pk,
        certificate: Some(server_keys.certificate),
        private_key: Some(server_keys.private_key),
        peer_certificate: Some(client_keys.certificate),
        srtp_profile: SRTP_AES128_CM_SHA1_80,
        ..Default::default()
    };

    let mut pke_mikey = Mikey::new(pke_config, MikeyRole::Initiator);
    let pke_result = pke_mikey.init();
    assert!(matches!(
        pke_result,
        Err(crate::Error::UnsupportedFeature(_))
    ));
    assert!(pke_mikey.get_srtp_key().is_none());
}

#[test]
fn test_mikey_pke_unified_security_integration() {
    use crate::api::common::config::SecurityConfig;
    use crate::api::common::unified_security::SecurityContextFactory;
    use crate::security::mikey::crypto::{generate_key_pair_and_certificate, CertificateConfig};

    // Generate certificates
    let server_config = CertificateConfig::enterprise_server("unified-test.example.com");
    let server_keys = generate_key_pair_and_certificate(server_config).unwrap();

    let client_config = CertificateConfig::enterprise_client("unified-client@example.com");
    let client_keys = generate_key_pair_and_certificate(client_config).unwrap();

    // Create security config with certificate data
    let security_config = SecurityConfig::mikey_pke_with_certificates(
        server_keys.certificate,
        server_keys.private_key,
        client_keys.certificate,
    );

    // Create unified security context
    let result = SecurityContextFactory::create_context(security_config);
    assert!(matches!(
        result,
        Err(crate::api::common::error::SecurityError::UnsupportedFeature(_))
    ));
}

#[test]
fn test_mikey_certificate_validation_fails_closed() {
    use crate::security::mikey::crypto::{
        generate_ca_certificate, validate_certificate_chain, CertificateConfig,
    };

    // Generate CA
    let ca_config = CertificateConfig::enterprise_server("Validation Test CA");
    let ca_keys = generate_ca_certificate(ca_config).unwrap();

    // Even parseable, currently valid certificates cannot be elevated to a
    // trusted chain without issuer/signature verification.
    let validation_result = validate_certificate_chain(&ca_keys.certificate, &ca_keys.certificate);
    assert!(matches!(
        validation_result,
        Err(crate::Error::UnsupportedFeature(_))
    ));

    // Malformed inputs take the same fail-closed capability path and cannot
    // accidentally be reported as an ordinary validation result.
    assert!(matches!(
        validate_certificate_chain(b"not-a-certificate", b"not-a-ca"),
        Err(crate::Error::UnsupportedFeature(_))
    ));
}
