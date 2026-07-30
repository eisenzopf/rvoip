use bytes::Bytes;
use rvoip_rtp_core::api::client::security::srtp::{SdesClient, SdesClientConfig};
use rvoip_rtp_core::api::client::security::{
    ClientSecurityConfig, ClientSecurityContext, DefaultClientSecurityContext,
};
use rvoip_rtp_core::api::common::{SecurityError, SrtpProfile};
use rvoip_rtp_core::api::server::security::srtp::{
    SdesServer, SdesServerConfig, SdesServerSession,
};
use rvoip_rtp_core::api::server::security::{
    DefaultServerSecurityContext, ServerSecurityConfig, SocketHandle,
};
use rvoip_rtp_core::dtls::message::extension::{SrtpProtectionProfile, UseSrtpExtension};
use rvoip_rtp_core::dtls::{create_connection, DtlsConfig};
use rvoip_rtp_core::security::sdes::{Sdes, SdesConfig, SdesRole};
use rvoip_rtp_core::security::SecurityKeyExchange;
use rvoip_rtp_core::srtp::{
    SrtpContext, SrtpCryptoKey, SrtpEncryptionAlgorithm, SRTP_AEAD_AES_128_GCM,
    SRTP_AEAD_AES_256_GCM, SRTP_AES128_CM_SHA1_32, SRTP_AES128_CM_SHA1_80, SRTP_AES256_CM_SHA1_80,
};
use rvoip_rtp_core::transport::{
    RtpTransport, RtpTransportConfig, SecurityRtpTransport, UdpRtpTransport,
};
use rvoip_rtp_core::Error;
use rvoip_rtp_core::RtpPacket;
use std::sync::Arc;
use tokio::net::UdpSocket;

#[test]
fn retained_gcm_identities_do_not_change_the_null_algorithm_ordinal() {
    assert_eq!(SrtpEncryptionAlgorithm::Null as u8, 2);
    assert!((SrtpEncryptionAlgorithm::AeadAes128Gcm as u8) > 2);
    assert!((SrtpEncryptionAlgorithm::AeadAes256Gcm as u8) > 2);
}

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

    let client = DefaultClientSecurityContext::new(ClientSecurityConfig::default())
        .await
        .unwrap();
    let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    client
        .set_socket(SocketHandle {
            remote_addr: Some(socket.local_addr().unwrap()),
            socket,
        })
        .await
        .unwrap();
    let error = client.get_security_info().await.unwrap_err();
    assert!(matches!(error, SecurityError::UnsupportedFeature(_)));
}

#[tokio::test]
async fn enabled_srtp_transport_never_sends_plaintext_without_a_context() {
    let sink = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let inner = UdpRtpTransport::new(RtpTransportConfig {
        local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
        ..RtpTransportConfig::default()
    })
    .await
    .unwrap();
    let transport = SecurityRtpTransport::new(Arc::new(inner), true)
        .await
        .unwrap();
    let packet = RtpPacket::new_with_payload(0, 1, 160, 0x0102_0304, Bytes::from_static(b"secret"));

    let error = transport
        .send_rtp(&packet, sink.local_addr().unwrap())
        .await
        .unwrap_err();
    assert!(matches!(error, Error::InvalidState(_)));

    let mut buffer = [0u8; 128];
    assert!(tokio::time::timeout(
        std::time::Duration::from_millis(50),
        sink.recv_from(&mut buffer)
    )
    .await
    .is_err());
    transport.close().await.unwrap();
}

#[tokio::test]
async fn enabled_srtp_transport_never_receives_plaintext_without_a_context() {
    let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let inner = UdpRtpTransport::new(RtpTransportConfig {
        local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
        ..RtpTransportConfig::default()
    })
    .await
    .unwrap();
    let transport = SecurityRtpTransport::new(Arc::new(inner), true)
        .await
        .unwrap();
    let mut events = transport.subscribe();
    let packet =
        RtpPacket::new_with_payload(0, 2, 320, 0x0102_0304, Bytes::from_static(b"plaintext"));

    sender
        .send_to(
            &packet.serialize().unwrap(),
            transport.local_rtp_addr().unwrap(),
        )
        .await
        .unwrap();

    let mut buffer = [0u8; 128];
    let error = transport.receive_packet(&mut buffer).await.unwrap_err();
    assert!(matches!(error, Error::InvalidState(_)));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), events.recv())
            .await
            .is_err()
    );
    transport.close().await.unwrap();
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

#[tokio::test]
async fn public_sdes_wrappers_reject_gcm_before_offer_or_negotiation() {
    for profiles in [
        vec![SrtpProfile::AesGcm128],
        vec![SrtpProfile::AesCm128HmacSha1_80, SrtpProfile::AesGcm256],
    ] {
        let client_config = SdesClientConfig {
            supported_profiles: profiles.clone(),
            ..SdesClientConfig::default()
        };
        let Err(error) = SdesClient::new(client_config) else {
            panic!("SDES client construction must reject the complete profile list");
        };
        assert!(matches!(error, SecurityError::UnsupportedFeature(_)));

        let server_config = SdesServerConfig {
            supported_profiles: profiles,
            ..SdesServerConfig::default()
        };
        let Err(error) = SdesServer::new(server_config.clone()) else {
            panic!("SDES server construction must reject the complete profile list");
        };
        assert!(matches!(error, SecurityError::UnsupportedFeature(_)));

        let Err(error) = SdesServerSession::new("invalid".to_string(), server_config) else {
            panic!("SDES session construction must reject the complete profile list");
        };
        assert!(matches!(error, SecurityError::UnsupportedFeature(_)));
    }
}

#[tokio::test]
async fn public_sdes_wrappers_generate_only_real_implemented_offers() {
    let client = SdesClient::new(SdesClientConfig::default()).unwrap();
    let client_offer = client.generate_offer().await.unwrap();
    assert!(!client_offer.is_empty());

    let server = SdesServer::new(SdesServerConfig::default()).unwrap();
    let session = server.create_session("valid".to_string()).await.unwrap();
    let server_offer = session.generate_offer().await.unwrap();
    assert!(!server_offer.is_empty());

    for line in client_offer.into_iter().chain(server_offer) {
        assert!(line.starts_with("a=crypto:"));
        assert!(!line.contains("GCM"));
        assert!(!line.contains("placeholder"));
        let attribute = rvoip_rtp_core::security::sdes::SdesCryptoAttribute::parse(
            line.strip_prefix("a=crypto:").unwrap(),
        )
        .unwrap();
        assert_eq!(attribute.key_method, "inline");
        assert!(!attribute.key_info.is_empty());
    }
}
