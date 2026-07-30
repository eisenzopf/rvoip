use bytes::Bytes;
use rvoip_rtp_core::api::client::security::dtls::handshake as client_dtls_handshake;
use rvoip_rtp_core::api::client::security::dtls::transport as client_dtls_transport;
use rvoip_rtp_core::api::client::security::srtp::{
    SdesClient, SdesClientConfig, SrtpClientSecurityContext,
};
use rvoip_rtp_core::api::client::security::{
    ClientSecurityConfig, ClientSecurityContext as OutboundClientSecurityContext,
    DefaultClientSecurityContext,
};
use rvoip_rtp_core::api::common::{
    SecurityConfig, SecurityError, SecurityMode, SecurityProfile, SrtpProfile,
};
use rvoip_rtp_core::api::server::security::client::context::DefaultClientSecurityContext as ServerManagedClientSecurityContext;
use rvoip_rtp_core::api::server::security::core::connection as server_dtls_connection;
use rvoip_rtp_core::api::server::security::dtls::transport as server_dtls_transport;
use rvoip_rtp_core::api::server::security::srtp::{
    SdesServer, SdesServerConfig, SdesServerSession, SrtpServerClientContext,
    SrtpServerSecurityContext,
};
use rvoip_rtp_core::api::server::security::{
    ClientSecurityContext as InboundClientSecurityContext, DefaultServerSecurityContext,
    ServerSecurityConfig, ServerSecurityContext, SocketHandle,
};
use rvoip_rtp_core::dtls::message::extension::{SrtpProtectionProfile, UseSrtpExtension};
use rvoip_rtp_core::dtls::{create_connection, DtlsConfig};
use rvoip_rtp_core::security::sdes::{Sdes, SdesConfig, SdesRole};
use rvoip_rtp_core::security::SecurityKeyExchange;
use rvoip_rtp_core::srtp::crypto::SrtpCrypto;
use rvoip_rtp_core::srtp::{
    SrtpAuthenticationAlgorithm, SrtpContext, SrtpCryptoKey, SrtpCryptoSuite,
    SrtpEncryptionAlgorithm, SRTP_AEAD_AES_128_GCM, SRTP_AEAD_AES_256_GCM, SRTP_AES128_CM_SHA1_32,
    SRTP_AES128_CM_SHA1_80, SRTP_AES256_CM_SHA1_32, SRTP_AES256_CM_SHA1_80, SRTP_NULL_NULL,
    SRTP_NULL_SHA1_80,
};
use rvoip_rtp_core::transport::{
    RtpTransport, RtpTransportConfig, SecurityRtpTransport, UdpRtpTransport,
};
use rvoip_rtp_core::Error;
use rvoip_rtp_core::RtpPacket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::net::UdpSocket;

fn dtls_client_config(profiles: Vec<SrtpProfile>) -> ClientSecurityConfig {
    ClientSecurityConfig {
        security_mode: SecurityMode::DtlsSrtp,
        validate_fingerprint: true,
        srtp_profiles: profiles,
        ..ClientSecurityConfig::default()
    }
}

fn dtls_server_config(profiles: Vec<SrtpProfile>) -> ServerSecurityConfig {
    ServerSecurityConfig {
        security_mode: SecurityMode::DtlsSrtp,
        srtp_profiles: profiles,
        ..ServerSecurityConfig::default()
    }
}

#[test]
fn all_public_security_defaults_are_explicitly_unsecured() {
    assert_eq!(SecurityMode::default(), SecurityMode::None);
    let common = SecurityConfig::default();
    assert_eq!(common.profile, SecurityProfile::Unsecured);
    assert_eq!(common.mode, SecurityMode::None);
    assert!(!common.required);
    assert!(common.srtp_profiles.is_empty());
    assert!(common.srtp_key.is_none());

    let client = ClientSecurityConfig::default();
    assert_eq!(client.security_mode, SecurityMode::None);
    assert!(!client.validate_fingerprint);
    assert!(client.srtp_profiles.is_empty());
    assert!(client.srtp_key.is_none());

    let server = ServerSecurityConfig::default();
    assert_eq!(server.security_mode, SecurityMode::None);
    assert!(server.srtp_profiles.is_empty());
    assert!(server.srtp_key.is_none());
}

#[tokio::test]
async fn standalone_psk_contexts_never_claim_an_active_secure_transport() {
    let client_config = ClientSecurityConfig {
        security_mode: SecurityMode::Srtp,
        srtp_profiles: vec![SrtpProfile::AesCm128HmacSha1_80],
        srtp_key: Some(vec![0x21; 30]),
        ..ClientSecurityConfig::default()
    };
    let client = SrtpClientSecurityContext::new(client_config).await.unwrap();
    assert!(!OutboundClientSecurityContext::is_secure(client.as_ref()));

    let server_config = ServerSecurityConfig {
        security_mode: SecurityMode::Srtp,
        srtp_profiles: vec![SrtpProfile::AesCm128HmacSha1_80],
        srtp_key: Some(vec![0x32; 30]),
        ..ServerSecurityConfig::default()
    };
    let server = SrtpServerSecurityContext::new(server_config.clone())
        .await
        .unwrap();
    assert!(!ServerSecurityContext::is_secure(server.as_ref()));

    let server_client =
        SrtpServerClientContext::new("127.0.0.1:5004".parse().unwrap(), server_config)
            .await
            .unwrap();
    assert!(!InboundClientSecurityContext::is_secure(&server_client));
}

#[test]
fn server_managed_dtls_context_rejects_direct_srtp_configuration() {
    let config = ServerSecurityConfig {
        security_mode: SecurityMode::Srtp,
        srtp_profiles: vec![SrtpProfile::AesCm128HmacSha1_80],
        srtp_key: Some(vec![0x43; 30]),
        ..ServerSecurityConfig::default()
    };

    assert!(matches!(
        ServerManagedClientSecurityContext::new(
            "127.0.0.1:5004".parse().unwrap(),
            None,
            None,
            config,
            None,
        ),
        Err(SecurityError::UnsupportedFeature(_))
    ));
}

#[tokio::test]
async fn field_literal_server_context_cannot_claim_or_advertise_unwired_srtp() {
    let forged_srtp = SrtpContext::new(
        SRTP_AES128_CM_SHA1_80,
        SrtpCryptoKey::new(vec![0x43; 16], vec![0x54; 14]),
    )
    .unwrap();
    let context = ServerManagedClientSecurityContext {
        address: "127.0.0.1:5004".parse().unwrap(),
        connection: Arc::new(tokio::sync::Mutex::new(None)),
        srtp_context: Arc::new(tokio::sync::Mutex::new(Some(forged_srtp))),
        handshake_completed: Arc::new(tokio::sync::Mutex::new(true)),
        socket: Arc::new(tokio::sync::Mutex::new(None)),
        config: ServerSecurityConfig {
            security_mode: SecurityMode::DtlsSrtp,
            srtp_profiles: vec![SrtpProfile::AesCm128HmacSha1_80],
            ..ServerSecurityConfig::default()
        },
        transport: Arc::new(tokio::sync::Mutex::new(None)),
        waiting_for_first_packet: Arc::new(tokio::sync::Mutex::new(false)),
        initial_packet: Arc::new(tokio::sync::Mutex::new(None)),
    };

    assert!(!InboundClientSecurityContext::is_secure(&context));
    assert!(
        !InboundClientSecurityContext::is_handshake_complete(&context)
            .await
            .unwrap()
    );
    let info = InboundClientSecurityContext::get_security_info(&context);
    assert_eq!(info.mode, SecurityMode::None);
    assert!(info.crypto_suites.is_empty());
    assert!(info.srtp_profile.is_none());
}

#[tokio::test]
async fn public_dtls_transport_helpers_fail_before_raw_socket_activity() {
    let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let handle = SocketHandle {
        socket: socket.clone(),
        remote_addr: None,
    };
    let connection = Arc::new(tokio::sync::Mutex::new(None));

    assert!(matches!(
        client_dtls_transport::setup_transport(&handle, &connection).await,
        Err(SecurityError::UnsupportedFeature(_))
    ));
    assert!(matches!(
        client_dtls_transport::create_udp_transport(socket, 1_500).await,
        Err(SecurityError::UnsupportedFeature(_))
    ));
    assert!(matches!(
        server_dtls_transport::create_udp_transport(&handle, 1_500).await,
        Err(SecurityError::UnsupportedFeature(_))
    ));
    assert!(matches!(
        server_dtls_transport::start_packet_handler(&handle, |_, _| Ok(())).await,
        Err(SecurityError::UnsupportedFeature(_))
    ));
    assert!(matches!(
        server_dtls_transport::capture_initial_packet(&handle, 0).await,
        Err(SecurityError::UnsupportedFeature(_))
    ));
    assert!(matches!(
        server_dtls_connection::create_dtls_transport(&handle).await,
        Err(SecurityError::UnsupportedFeature(_))
    ));

    let running = Arc::new(AtomicBool::new(false));
    let remote_addr = Arc::new(tokio::sync::Mutex::new(Some(
        "127.0.0.1:5004".parse().unwrap(),
    )));
    let socket = Arc::new(tokio::sync::Mutex::new(Some(handle)));
    let completed = Arc::new(tokio::sync::Mutex::new(false));
    assert!(matches!(
        client_dtls_handshake::start_handshake_monitor(
            &running,
            &remote_addr,
            &socket,
            &connection,
            &completed,
        )
        .await,
        Err(SecurityError::UnsupportedFeature(_))
    ));
    assert!(!running.load(Ordering::SeqCst));
}

#[test]
fn active_security_examples_never_simulate_unavailable_protocol_success() {
    let examples = [
        include_str!("../examples/api_mikey_srtp.rs"),
        include_str!("../examples/api_mikey_pke.rs"),
        include_str!("../examples/api_zrtp_p2p.rs"),
        include_str!("../examples/api_advanced_security.rs"),
        include_str!("../examples/api_complete_security_showcase.rs"),
        include_str!("../examples/api_unified_security.rs"),
        include_str!("../examples/dtls_test.rs"),
        include_str!("../examples/direct_dtls_media_streaming.rs"),
        include_str!("../examples/TODO.md"),
        include_str!("../examples/examples_output_log.txt"),
    ];

    for example in examples {
        assert!(example.contains("UnsupportedFeature"));
        for unsafe_claim in [
            "simulate a successful key exchange",
            "Fallback SRTP",
            "SECURE COMMUNICATION ESTABLISHED",
            "Ready for production",
            "DTLS-SRTP support (existing)",
            "MIKEY context: Ready",
            "Phase 3 advanced security features are production-ready",
            "Direct DTLS Media Streaming example completed successfully",
            "DTLS test completed successfully",
        ] {
            assert!(
                !example.contains(unsafe_claim),
                "security example contains stale claim: {unsafe_claim}"
            );
        }
    }
}

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

    let client = dtls_client_config(vec![SrtpProfile::AesGcm128]);
    let Err(error) = DefaultClientSecurityContext::new(client).await else {
        panic!("GCM-only client security context must fail closed");
    };
    assert!(matches!(error, SecurityError::UnsupportedFeature(_)));

    let Err(error) = DefaultClientSecurityContext::new(dtls_client_config(vec![
        SrtpProfile::AesCm128HmacSha1_80,
    ]))
    .await
    else {
        panic!("the high-level DTLS client context must fail during construction");
    };
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
async fn udp_raw_bytes_cannot_bypass_an_installed_srtp_context() {
    let sink = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let transport = UdpRtpTransport::new(RtpTransportConfig {
        local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
        ..RtpTransportConfig::default()
    })
    .await
    .unwrap();
    let key = SrtpCryptoKey::new(vec![0x31; 16], vec![0x42; 14]);
    transport
        .set_srtp_contexts(
            SrtpContext::new(SRTP_AES128_CM_SHA1_80, key.clone()).unwrap(),
            SrtpContext::new(SRTP_AES128_CM_SHA1_80, key).unwrap(),
        )
        .await
        .unwrap();
    let packet =
        RtpPacket::new_with_payload(0, 3, 480, 0x0102_0304, Bytes::from_static(b"never raw"));
    let plaintext = packet.serialize().unwrap();

    assert!(matches!(
        transport
            .send_rtp_bytes(&plaintext, sink.local_addr().unwrap())
            .await,
        Err(Error::InvalidState(_))
    ));
    let mut wire = [0_u8; 128];
    assert!(tokio::time::timeout(
        std::time::Duration::from_millis(50),
        sink.recv_from(&mut wire)
    )
    .await
    .is_err());

    transport
        .send_rtp(&packet, sink.local_addr().unwrap())
        .await
        .unwrap();
    let (wire_len, _) =
        tokio::time::timeout(std::time::Duration::from_secs(1), sink.recv_from(&mut wire))
            .await
            .unwrap()
            .unwrap();
    assert_ne!(&wire[..wire_len], plaintext.as_ref());
    transport.close().await.unwrap();
}

#[tokio::test]
async fn disabled_contexts_cannot_turn_secure_transports_into_plain_rtp() {
    let sink = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let direct = UdpRtpTransport::new(RtpTransportConfig {
        local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
        ..RtpTransportConfig::default()
    })
    .await
    .unwrap();
    let key = SrtpCryptoKey::new(vec![0x51; 16], vec![0x62; 14]);
    let mut direct_send = SrtpContext::new(SRTP_AES128_CM_SHA1_80, key.clone()).unwrap();
    let mut direct_recv = SrtpContext::new(SRTP_AES128_CM_SHA1_80, key.clone()).unwrap();
    direct_send.set_enabled(false);
    direct_recv.set_enabled(false);
    assert!(matches!(
        direct.set_srtp_contexts(direct_send, direct_recv).await,
        Err(Error::InvalidState(_))
    ));
    assert!(!direct.srtp_enabled().await);

    let packet = RtpPacket::new_with_payload(
        0,
        4,
        640,
        0x0102_0304,
        Bytes::from_static(b"disabled is not plain"),
    );
    assert!(matches!(
        direct.send_rtp(&packet, sink.local_addr().unwrap()).await,
        Err(Error::InvalidState(_))
    ));

    let inner = Arc::new(
        UdpRtpTransport::new(RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            ..RtpTransportConfig::default()
        })
        .await
        .unwrap(),
    );
    let wrapped = SecurityRtpTransport::new(inner, true).await.unwrap();
    let mut disabled = SrtpContext::new(SRTP_AES128_CM_SHA1_80, key).unwrap();
    disabled.set_enabled(false);
    assert!(matches!(
        wrapped.set_srtp_context(disabled).await,
        Err(Error::InvalidState(_))
    ));
    assert!(!wrapped.is_srtp_ready().await);
    assert!(matches!(
        wrapped.send_rtp(&packet, sink.local_addr().unwrap()).await,
        Err(Error::InvalidState(_))
    ));
    assert!(matches!(
        wrapped
            .inner_transport()
            .send_rtp_bytes(&packet.serialize().unwrap(), sink.local_addr().unwrap())
            .await,
        Err(Error::InvalidState(_))
    ));

    let mut wire = [0_u8; 128];
    assert!(tokio::time::timeout(
        std::time::Duration::from_millis(50),
        sink.recv_from(&mut wire)
    )
    .await
    .is_err());
    direct.close().await.unwrap();
    wrapped.close().await.unwrap();
}

#[tokio::test]
async fn null_and_unreviewed_suites_cannot_become_ready_secure_transports() {
    let key = SrtpCryptoKey::new(vec![0x63; 16], vec![0x74; 14]);
    let aes_without_authentication = SrtpCryptoSuite {
        encryption: SrtpEncryptionAlgorithm::AesCm,
        authentication: SrtpAuthenticationAlgorithm::Null,
        key_length: 16,
        tag_length: 0,
    };
    for suite in [
        SRTP_NULL_NULL,
        SRTP_NULL_SHA1_80,
        aes_without_authentication,
    ] {
        assert!(matches!(
            SrtpContext::new(suite.clone(), key.clone()),
            Err(Error::UnsupportedFeature(_))
        ));
        assert!(matches!(
            SrtpCrypto::new(suite, key.clone()),
            Err(Error::UnsupportedFeature(_))
        ));
    }

    // The exact reviewed AES-CM/HMAC identities remain constructible.
    for suite in [
        SRTP_AES128_CM_SHA1_80,
        SRTP_AES128_CM_SHA1_32,
        SRTP_AES256_CM_SHA1_80,
        SRTP_AES256_CM_SHA1_32,
    ] {
        let key = SrtpCryptoKey::new(vec![0x63; suite.key_length], vec![0x74; 14]);
        assert!(SrtpContext::new(suite, key).is_ok());
    }

    let sink = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let inner = Arc::new(
        UdpRtpTransport::new(RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            ..RtpTransportConfig::default()
        })
        .await
        .unwrap(),
    );
    let transport = SecurityRtpTransport::new(inner, true).await.unwrap();
    let mut events = transport.subscribe();
    assert!(!transport.is_srtp_ready().await);

    let packet = RtpPacket::new_with_payload(
        0,
        5,
        800,
        0x0102_0304,
        Bytes::from_static(b"null suite must not pass"),
    );
    assert!(matches!(
        transport
            .send_rtp(&packet, sink.local_addr().unwrap())
            .await,
        Err(Error::InvalidState(_))
    ));
    let mut wire = [0_u8; 128];
    assert!(tokio::time::timeout(
        std::time::Duration::from_millis(50),
        sink.recv_from(&mut wire)
    )
    .await
    .is_err());

    let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    sender
        .send_to(
            &packet.serialize().unwrap(),
            transport.local_rtp_addr().unwrap(),
        )
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(75), events.recv())
            .await
            .is_err()
    );
    transport.close().await.unwrap();
}

#[tokio::test]
async fn plaintext_wrapper_rejects_late_srtp_context_installation() {
    let sink = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let inner = Arc::new(
        UdpRtpTransport::new(RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            ..RtpTransportConfig::default()
        })
        .await
        .unwrap(),
    );
    let wrapped = SecurityRtpTransport::new(inner, false).await.unwrap();
    let context = SrtpContext::new(
        SRTP_AES128_CM_SHA1_80,
        SrtpCryptoKey::new(vec![0x19; 16], vec![0x2a; 14]),
    )
    .unwrap();

    assert!(matches!(
        wrapped.set_srtp_context(context).await,
        Err(Error::InvalidState(_))
    ));
    assert!(!wrapped.is_srtp_ready().await);

    let mut wire = [0_u8; 128];
    assert!(tokio::time::timeout(
        std::time::Duration::from_millis(50),
        sink.recv_from(&mut wire)
    )
    .await
    .is_err());
    wrapped.close().await.unwrap();
}

#[test]
fn low_level_srtcp_crypto_entry_points_are_unavailable() {
    let crypto = SrtpCrypto::new(
        SRTP_AES128_CM_SHA1_80,
        SrtpCryptoKey::new(vec![0x71; 16], vec![0x82; 14]),
    )
    .unwrap();
    let rtcp = [0x80, 200, 0, 1, 0, 0, 0, 1];

    assert!(matches!(
        crypto.encrypt_rtcp(&rtcp),
        Err(Error::UnsupportedFeature(_))
    ));
    assert!(matches!(
        crypto.decrypt_rtcp(&rtcp),
        Err(Error::UnsupportedFeature(_))
    ));
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
    assert!(matches!(error, Error::UnsupportedFeature(_)));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), events.recv())
            .await
            .is_err()
    );
    transport.close().await.unwrap();
}

#[tokio::test]
async fn installed_wrapper_context_still_rejects_racy_direct_receive_and_plaintext() {
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
    transport
        .set_srtp_context(
            SrtpContext::new(
                SRTP_AES128_CM_SHA1_80,
                SrtpCryptoKey::new(vec![0x73; 16], vec![0x84; 14]),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let mut events = transport.subscribe();
    let packet = RtpPacket::new_with_payload(
        0,
        17,
        2_720,
        0x5566_7788,
        Bytes::from_static(b"unauthenticated"),
    );

    sender
        .send_to(
            &packet.serialize().unwrap(),
            transport.local_rtp_addr().unwrap(),
        )
        .await
        .unwrap();
    let mut buffer = [0_u8; 128];
    assert!(matches!(
        transport.receive_packet(&mut buffer).await,
        Err(Error::UnsupportedFeature(_))
    ));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(75), events.recv())
            .await
            .is_err()
    );
    transport.close().await.unwrap();
}

#[tokio::test]
async fn enabled_srtp_transport_rejects_rtcp_until_authenticated_srtcp_exists() {
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
    let context = SrtpContext::new(
        SRTP_AES128_CM_SHA1_80,
        SrtpCryptoKey::new(vec![0x31; 16], vec![0x42; 14]),
    )
    .unwrap();
    transport.set_srtp_context(context).await.unwrap();
    let report = rvoip_rtp_core::RtcpPacket::ReceiverReport(
        rvoip_rtp_core::RtcpReceiverReport::new(0x1122_3344),
    );

    assert!(matches!(
        transport
            .send_rtcp(&report, sink.local_addr().unwrap())
            .await,
        Err(Error::UnsupportedFeature(_))
    ));
    assert!(matches!(
        transport
            .send_rtcp_bytes(&report.serialize().unwrap(), sink.local_addr().unwrap())
            .await,
        Err(Error::UnsupportedFeature(_))
    ));

    let mut wire = [0_u8; 128];
    assert!(tokio::time::timeout(
        std::time::Duration::from_millis(50),
        sink.recv_from(&mut wire)
    )
    .await
    .is_err());
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

    let server_config = dtls_server_config(vec![SrtpProfile::AesCm128HmacSha1_80]);
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

    let Err(error) = DefaultClientSecurityContext::new(dtls_client_config(vec![
        SrtpProfile::AesCm128HmacSha1_80,
    ]))
    .await
    else {
        panic!("high-level client DTLS context must fail closed during construction");
    };
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

    assert!(matches!(
        SrtpProtectionProfile::Unknown(0xbeef).ensure_supported(),
        Err(Error::UnsupportedFeature(_))
    ));
    assert!(matches!(
        rvoip_rtp_core::api::server::security::util::string_to_security_mode("not-a-security-mode"),
        Err(SecurityError::UnsupportedFeature(_))
    ));
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
