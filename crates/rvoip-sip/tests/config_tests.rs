//! Configuration Tests
//!
//! Tests Config constructors, defaults, and field values.

use rvoip_sip::{Config, MediaMode, PerformanceConfig, SipContactMode, SipTlsMode};
use rvoip_sip_transport::UdpParseDispatch;
use std::net::{IpAddr, SocketAddr};

// ── Config::local ───────────────────────────────────────────────────────────

#[test]
fn test_config_local_ip() {
    let c = Config::local("alice", 5060);
    let localhost: IpAddr = "127.0.0.1".parse().unwrap();
    assert_eq!(c.local_ip, localhost);
}

#[test]
fn test_config_local_port() {
    let c = Config::local("alice", 5060);
    assert_eq!(c.sip_port, 5060);
    assert_eq!(c.bind_addr.port(), 5060);
}

#[test]
fn test_config_local_uri() {
    let c = Config::local("alice", 5060);
    assert_eq!(c.local_uri, "sip:alice@127.0.0.1:5060");
}

#[test]
fn test_config_local_media_ports() {
    let c = Config::local("alice", 5060);
    assert!(c.media_port_start < c.media_port_end);
    assert_eq!(c.media_port_start, Config::DEFAULT_MEDIA_PORT_START);
    assert_eq!(c.media_port_end, Config::DEFAULT_MEDIA_PORT_END);
}

#[test]
fn test_config_local_no_state_table_path() {
    let c = Config::local("alice", 5060);
    assert!(c.state_table_path.is_none());
}

// ── Config::on ──────────────────────────────────────────────────────────────

#[test]
fn test_config_on_custom_ip() {
    let ip: IpAddr = "192.168.1.50".parse().unwrap();
    let c = Config::on("bob", ip, 5080);
    assert_eq!(c.local_ip, ip);
    assert_eq!(c.sip_port, 5080);
    assert_eq!(c.bind_addr.ip(), ip);
    assert_eq!(c.bind_addr.port(), 5080);
}

#[test]
fn test_config_on_uri_includes_ip() {
    let ip: IpAddr = "10.0.0.1".parse().unwrap();
    let c = Config::on("charlie", ip, 5090);
    assert_eq!(c.local_uri, "sip:charlie@10.0.0.1:5090");
}

#[test]
fn test_config_on_ipv6() {
    let ip: IpAddr = "::1".parse().unwrap();
    let c = Config::on("ipv6user", ip, 5060);
    assert_eq!(c.local_ip, ip);
    assert!(c.local_uri.contains("::1"));
}

// ── Config::default ─────────────────────────────────────────────────────────

#[test]
fn test_config_default() {
    let c = Config::default();
    assert_eq!(c.sip_port, 5060);
    assert_eq!(c.local_uri, "sip:user@127.0.0.1:5060");
    assert!(c.auto_180_ringing);
    assert!(c.auto_100_trying);
    assert!(!c.fast_auto_accept_incoming_calls);
    assert!(!c.cleanup_diagnostics);
    assert!(!c.cleanup_diagnostic_events);
    assert!(!c.srtp_diagnostics);
    assert!(!c.rtp_diagnostics);
    assert!(!c.media_sdp_diagnostics);
    assert_eq!(c.media_mode, MediaMode::Enabled);
}

#[test]
fn test_config_media_port_validation() {
    let mut c = Config::local("alice", 5060).with_media_ports(40_000, 39_999);
    assert!(c.validate().is_err());

    c = Config::local("alice", 5060).with_media_ports(1_023, 2_000);
    assert!(c.validate().is_err());

    c = Config::local("alice", 5060).with_media_ports(40_000, 40_000);
    assert!(c.validate().is_ok());
}

#[test]
fn test_config_graduated_perf_knobs_are_configurable() {
    let c = Config::local("alice", 5060)
        .with_auto_180_ringing(false)
        .with_auto_100_trying(false)
        .with_fast_auto_accept_incoming_calls(true)
        .with_sip_udp_parse_workers(4)
        .with_sip_udp_parse_queue_capacity(8192)
        .with_global_event_channel_capacity(16_384)
        .with_media_session_capacity(4096)
        .with_media_port_capacity(16_384, 1024)
        .with_sip_udp_diagnostics(true)
        .with_media_setup_diagnostics(true)
        .with_cleanup_diagnostics(true)
        .with_cleanup_diagnostic_events(true)
        .with_srtp_diagnostics(true)
        .with_rtp_diagnostics(true)
        .with_media_sdp_diagnostics(true)
        .with_signaling_only_media(9);

    assert!(!c.auto_180_ringing);
    assert!(!c.auto_100_trying);
    assert!(c.fast_auto_accept_incoming_calls);
    assert_eq!(c.sip_udp_parse_workers, Some(4));
    assert_eq!(c.sip_udp_parse_queue_capacity, Some(8192));
    assert_eq!(c.global_event_channel_capacity, 16_384);
    assert_eq!(c.media_session_capacity, Some(4096));
    assert_eq!(c.media_port_start, 16_384);
    assert_eq!(c.media_port_end, 17_407);
    assert_eq!(c.media_port_capacity, Some(1024));
    assert!(c.sip_udp_diagnostics);
    assert!(c.media_setup_diagnostics);
    assert!(c.cleanup_diagnostics);
    assert!(c.cleanup_diagnostic_events);
    assert!(c.srtp_diagnostics);
    assert!(c.rtp_diagnostics);
    assert!(c.media_sdp_diagnostics);
    assert_eq!(c.media_mode, MediaMode::SignalingOnly { sdp_rtp_port: 9 });
}

#[test]
fn test_config_pbx_media_server_performance_recipe() {
    let c = Config::local("alice", 5060).with_pbx_media_server_performance(2_000);

    assert!(!c.auto_180_ringing);
    assert!(!c.auto_100_trying);
    assert!(c.fast_auto_accept_incoming_calls);
    assert_eq!(c.sip_udp_recv_buffer_size, Some(8_388_608));
    assert_eq!(c.sip_udp_send_buffer_size, Some(8_388_608));
    assert_eq!(c.sip_udp_parse_workers, Some(4));
    assert_eq!(c.sip_udp_parse_queue_capacity, Some(2_000));
    assert_eq!(c.sip_udp_parse_dispatch, Some(UdpParseDispatch::RoundRobin));
    assert_eq!(c.sip_transaction_dispatch_workers, Some(2));
    assert_eq!(c.sip_dialog_dispatch_workers, Some(4));
    assert_eq!(c.session_event_dispatcher_workers, 4);
    assert_eq!(c.sip_transaction_command_channel_capacity, Some(128));
    assert_eq!(c.server_call_capacity, Some(2_000));
    assert_eq!(c.server_call_admission_limit, Some(2_000));
    assert_eq!(c.server_call_admission_soft_limit, Some(1_800));
    assert_eq!(c.server_call_admission_pacing_delay_ms, Some(1));
    assert_eq!(c.server_overload_retry_after_secs, Some(1));
    assert_eq!(c.media_port_start, 16_384);
    assert_eq!(c.media_port_end, 65_535);
    assert_eq!(c.media_port_capacity, Some(49_152));
    assert_eq!(c.media_session_capacity, Some(2_000));
    assert_eq!(c.media_mode, MediaMode::Enabled);
}

#[test]
fn test_config_signaling_only_server_high_performance_recipe() {
    let c = Config::local("alice", 5060).with_signaling_only_server_high_performance(2_000, 9);

    assert!(c.fast_auto_accept_incoming_calls);
    assert_eq!(c.sip_udp_parse_workers, Some(4));
    assert_eq!(c.sip_udp_parse_dispatch, Some(UdpParseDispatch::RoundRobin));
    assert_eq!(c.sip_transaction_dispatch_workers, Some(2));
    assert_eq!(c.sip_dialog_dispatch_workers, Some(4));
    assert_eq!(c.sip_transaction_command_channel_capacity, Some(128));
    assert_eq!(c.server_call_capacity, Some(2_000));
    assert_eq!(c.server_call_admission_limit, Some(2_000));
    assert_eq!(c.server_call_admission_soft_limit, Some(1_800));
    assert_eq!(c.server_call_admission_pacing_delay_ms, Some(1));
    assert_eq!(c.server_overload_retry_after_secs, Some(1));
    assert_eq!(c.media_mode, MediaMode::SignalingOnly { sdp_rtp_port: 9 });
}

#[test]
fn test_config_profile_defaults_can_be_overridden_after_profile() {
    let c = Config::local("alice", 5060)
        .with_pbx_media_server_performance(2_000)
        .with_sip_transaction_command_channel_capacity(256)
        .with_sip_udp_parse_workers(2)
        .with_sip_udp_parse_dispatch(UdpParseDispatch::SourceHash);

    assert_eq!(c.sip_transaction_command_channel_capacity, Some(256));
    assert_eq!(c.sip_udp_parse_workers, Some(2));
    assert_eq!(c.sip_udp_parse_dispatch, Some(UdpParseDispatch::SourceHash));
}

#[test]
fn test_config_profile_rejects_invalid_capacity() {
    let err = Config::local("alice", 5060)
        .try_with_performance_config(PerformanceConfig::pbx_media_server(0))
        .unwrap_err();
    assert!(err.to_string().contains("capacity"));
}

#[test]
fn test_config_can_load_custom_performance_recipe_yaml() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recipes.yaml");
    std::fs::write(
        &path,
        r#"
version: 1
performanceProfiles:
  custom-pbx:
    requiresCapacity: true
    config:
      channelCapacity: "$capacity"
      auto180Ringing: false
      auto100Trying: false
      fastAutoAcceptIncomingCalls: true
      sipUdpParseWorkers: 2
      sipUdpParseDispatch: round-robin
      sipTransactionCommandChannelCapacity: 64
      serverCallAdmissionLimit: "$capacity"
      serverCallAdmissionSoftLimit: "$capacity90Percent"
      serverCallAdmissionPacingDelayMs: 3
      serverOverloadRetryAfterSecs: 2
      mediaMode: enabled
      mediaSessionCapacity: "$capacity"
"#,
    )
    .unwrap();

    let c = Config::local("alice", 5060)
        .try_with_performance_config(
            PerformanceConfig::profile("custom-pbx")
                .with_capacity(321)
                .with_recipe_path(&path),
        )
        .unwrap();

    assert_eq!(c.incoming_call_channel_capacity, 321);
    assert!(!c.auto_180_ringing);
    assert!(!c.auto_100_trying);
    assert!(c.fast_auto_accept_incoming_calls);
    assert_eq!(c.sip_udp_parse_workers, Some(2));
    assert_eq!(c.sip_udp_parse_dispatch, Some(UdpParseDispatch::RoundRobin));
    assert_eq!(c.sip_transaction_command_channel_capacity, Some(64));
    assert_eq!(c.server_call_admission_limit, Some(321));
    assert_eq!(c.server_call_admission_soft_limit, Some(289));
    assert_eq!(c.server_call_admission_pacing_delay_ms, Some(3));
    assert_eq!(c.server_overload_retry_after_secs, Some(2));
    assert_eq!(c.media_session_capacity, Some(321));
    assert_eq!(c.media_mode, MediaMode::Enabled);
}

// ── Different names ─────────────────────────────────────────────────────────

#[test]
fn test_config_name_in_uri() {
    let c1 = Config::local("alice", 5060);
    let c2 = Config::local("bob", 5060);
    assert!(c1.local_uri.contains("alice"));
    assert!(c2.local_uri.contains("bob"));
    assert_ne!(c1.local_uri, c2.local_uri);
}

#[test]
fn test_config_lan_pbx_profile_sets_advertised_addresses() {
    let bind: SocketAddr = "0.0.0.0:5060".parse().unwrap();
    let advertised: SocketAddr = "192.0.2.10:5060".parse().unwrap();
    let c = Config::lan_pbx("alice", bind, advertised);
    assert_eq!(c.bind_addr, bind);
    assert_eq!(c.sip_advertised_addr, Some(advertised));
    assert_eq!(c.media_public_addr.unwrap().ip(), advertised.ip());
    assert_eq!(
        c.media_public_addr.unwrap().port(),
        0,
        "LAN PBX media public address must not reuse the SIP port; SDP should advertise the allocated RTP port"
    );
}

#[test]
fn test_config_asterisk_registered_flow_profile() {
    let c = Config::asterisk_tls_registered_flow(
        "alice",
        "127.0.0.1:5060".parse().unwrap(),
        "urn:uuid:00000000-0000-0000-0000-000000000001",
    );
    assert_eq!(c.sip_tls_mode, SipTlsMode::ClientOnly);
    assert_eq!(c.sip_contact_mode, SipContactMode::RegisteredFlowSymmetric);
    assert!(c.offer_srtp);
    assert!(c.srtp_required);
}

#[test]
fn test_config_carrier_sbc_profile() {
    let c = Config::carrier_sbc(
        "trunk",
        "0.0.0.0:5060".parse().unwrap(),
        "198.51.100.20:5061".parse().unwrap(),
        "sips:sbc.example.com:5061;lr",
        "urn:uuid:00000000-0000-0000-0000-000000000002",
    );
    assert_eq!(c.sip_tls_mode, SipTlsMode::ClientOnly);
    assert_eq!(c.sip_contact_mode, SipContactMode::RegisteredFlowRfc5626);
    assert_eq!(
        c.outbound_proxy_uri.as_deref(),
        Some("sips:sbc.example.com:5061;lr")
    );
    assert!(c.offer_srtp);
    assert!(c.srtp_required);
}
