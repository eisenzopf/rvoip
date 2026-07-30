use bytes::Bytes;
use rvoip_rtp_core::api::client::security::{
    ClientSecurityConfig, ClientSecurityContext, DefaultClientSecurityContext,
};
use rvoip_rtp_core::api::common::{SecurityError, SrtpProfile};
use rvoip_rtp_core::api::server::security::{
    DefaultServerSecurityContext, ServerSecurityConfig, SocketHandle,
};
use rvoip_rtp_core::dtls::message::extension::{SrtpProtectionProfile, UseSrtpExtension};
use rvoip_rtp_core::dtls::{create_connection, DtlsConfig};
use rvoip_rtp_core::security::sdes::{Sdes, SdesConfig, SdesRole};
use rvoip_rtp_core::security::SecurityKeyExchange;
use rvoip_rtp_core::srtp::{
    SrtpContext, SrtpCryptoKey, SRTP_AEAD_AES_128_GCM, SRTP_AEAD_AES_256_GCM,
    SRTP_AES128_CM_SHA1_32, SRTP_AES128_CM_SHA1_80, SRTP_AES256_CM_SHA1_80,
};
use rvoip_rtp_core::Error;
use std::sync::Arc;
use tokio::net::UdpSocket;

#[test]
fn aes_gcm_profiles_retain_identity_but_cannot_be_constructed_or_advertised() {
    for (suite, profile) in [
        (SRTP_AEAD_AES_128_GCM, SrtpProtectionProfile::AeadAes128Gcm),
        (SRTP_AEAD_AES_256_GCM, SrtpProtectionProfile::AeadAes256Gcm),
    ] {
        let key = SrtpCryptoKey::new(vec![0; suite.key_length], vec![0; 14]);
        assert!(matches!(
            SrtpContext::new(suite.clone(), key),
            Err(Error::UnsupportedFeature(_))
        ));

        assert!(matches!(
            UseSrtpExtension::new(vec![profile], Bytes::new()).serialize(),
            Err(Error::UnsupportedFeature(_))
        ));

        let mut sdes = Sdes::new(
            SdesConfig {
                crypto_suites: vec![suite],
                offer_count: 1,
            },
            SdesRole::Offerer,
        );
        assert!(matches!(
            sdes.process_message(b""),
            Err(Error::UnsupportedFeature(_))
        ));
    }
}

#[tokio::test]
async fn public_dtls_and_security_construction_fail_with_typed_errors() {
    let Err(error) = create_connection(DtlsConfig::default()).await else {
        panic!("the incomplete DTLS implementation must not construct");
    };
    assert!(matches!(error, Error::UnsupportedFeature(_)));

    let mut dtls = DtlsConfig::default();
    dtls.srtp_profiles = vec![SRTP_AEAD_AES_128_GCM];
    let Err(error) = create_connection(dtls).await else {
        panic!("DTLS must reject an unimplemented protection profile");
    };
    assert!(matches!(error, Error::UnsupportedFeature(_)));

    let mut dtls = DtlsConfig::default();
    dtls.srtp_profiles = vec![SRTP_AES256_CM_SHA1_80];
    let Err(error) = create_connection(dtls).await else {
        panic!("DTLS must reject suites its handshake cannot negotiate");
    };
    assert!(matches!(error, Error::UnsupportedFeature(_)));

    let client = ClientSecurityConfig {
        srtp_profiles: vec![SrtpProfile::AesGcm128],
        ..ClientSecurityConfig::default()
    };
    let Err(error) = DefaultClientSecurityContext::new(client).await else {
        panic!("GCM-only client security context must fail closed");
    };
    assert!(matches!(error, SecurityError::UnsupportedFeature(_)));
}

#[tokio::test]
async fn production_client_and_server_dtls_builders_return_unsupported() {
    let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let remote_addr = socket.local_addr().unwrap();

    let Err(error) = rvoip_rtp_core::api::client::security::dtls::connection::create_connection(
        &socket,
        remote_addr,
    )
    .await
    else {
        panic!("client DTLS builder must fail closed");
    };
    assert!(matches!(error, SecurityError::UnsupportedFeature(_)));

    let server_config = ServerSecurityConfig::default();
    let Err(error) =
        rvoip_rtp_core::api::server::security::core::create_server_connection(&server_config).await
    else {
        panic!("server DTLS builder must fail closed");
    };
    assert!(matches!(error, SecurityError::UnsupportedFeature(_)));

    let Err(error) = DefaultServerSecurityContext::new(server_config).await else {
        panic!("high-level server DTLS context must fail closed");
    };
    assert!(matches!(error, SecurityError::UnsupportedFeature(_)));

    let client = DefaultClientSecurityContext::new(ClientSecurityConfig::default())
        .await
        .unwrap();
    client
        .set_socket(SocketHandle {
            socket,
            remote_addr: Some(remote_addr),
        })
        .await
        .unwrap();
    let error = client.initialize().await.unwrap_err();
    assert!(matches!(error, SecurityError::UnsupportedFeature(_)));
}

#[test]
fn public_profile_conversions_reject_gcm_and_unknown_ids() {
    use rvoip_rtp_core::api::client::security::srtp::keys as client_keys;
    use rvoip_rtp_core::api::server::security::srtp::keys as server_keys;

    assert_eq!(
        server_keys::convert_profile(SrtpProfile::AesCm128HmacSha1_80).unwrap(),
        SRTP_AES128_CM_SHA1_80
    );
    assert_eq!(
        server_keys::profile_id_to_suite(0x0002).unwrap(),
        SRTP_AES128_CM_SHA1_32
    );

    for profile in [SrtpProfile::AesGcm128, SrtpProfile::AesGcm256] {
        assert!(matches!(
            profile.advertised_name(),
            Err(Error::UnsupportedFeature(_))
        ));
        assert!(matches!(
            server_keys::convert_profile(profile),
            Err(SecurityError::UnsupportedFeature(_))
        ));
        assert!(matches!(
            server_keys::convert_profiles(&[SrtpProfile::AesCm128HmacSha1_80, profile]),
            Err(SecurityError::UnsupportedFeature(_))
        ));
        assert!(matches!(
            server_keys::profile_to_string(profile),
            Err(SecurityError::UnsupportedFeature(_))
        ));
        assert!(matches!(
            client_keys::profile_to_suite(profile),
            Err(SecurityError::UnsupportedFeature(_))
        ));
    }

    for profile_id in [0x0007, 0x0008, 0xffff] {
        assert!(matches!(
            server_keys::profile_id_to_suite(profile_id),
            Err(SecurityError::UnsupportedFeature(_))
        ));
    }
}

#[test]
fn sdes_rejects_empty_and_unimplemented_configurations() {
    let invalid_configs = [
        (
            SdesConfig {
                crypto_suites: Vec::new(),
                offer_count: 1,
            },
            false,
        ),
        (
            SdesConfig {
                crypto_suites: vec![SRTP_AES128_CM_SHA1_80],
                offer_count: 0,
            },
            false,
        ),
        (
            SdesConfig {
                crypto_suites: vec![SRTP_AEAD_AES_128_GCM],
                offer_count: 1,
            },
            true,
        ),
        (
            SdesConfig {
                crypto_suites: vec![SRTP_AES256_CM_SHA1_80],
                offer_count: 1,
            },
            true,
        ),
    ];

    for (config, unsupported) in invalid_configs {
        let mut sdes = Sdes::new(config, SdesRole::Offerer);
        let error = sdes.process_message(b"").unwrap_err();
        if unsupported {
            assert!(matches!(error, Error::UnsupportedFeature(_)));
        } else {
            assert!(matches!(error, Error::InvalidParameter(_)));
        }
        assert!(!sdes.is_complete());
    }
}

#[test]
fn sdes_answerer_requires_a_configured_suite_intersection() {
    let mut offerer = Sdes::new(
        SdesConfig {
            crypto_suites: vec![SRTP_AES128_CM_SHA1_32],
            offer_count: 1,
        },
        SdesRole::Offerer,
    );
    let offer = offerer.process_message(b"").unwrap().unwrap();

    let mut answerer = Sdes::new(
        SdesConfig {
            crypto_suites: vec![SRTP_AES128_CM_SHA1_80],
            offer_count: 1,
        },
        SdesRole::Answerer,
    );
    assert!(matches!(
        answerer.process_message(&offer),
        Err(Error::NegotiationFailed(_))
    ));
    assert!(!answerer.is_complete());

    let gcm_offer = b"a=crypto:1 AEAD_AES_128_GCM inline:PS1uQCVeeCFCanVmcjkpPywjNWhcYD0mXXtxaVBR";
    let mut answerer = Sdes::new(SdesConfig::default(), SdesRole::Answerer);
    assert!(matches!(
        answerer.process_message(gcm_offer),
        Err(Error::NegotiationFailed(_))
    ));
    assert!(!answerer.is_complete());
}
