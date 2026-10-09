//! Deployment profile contracts.
//!
//! One test per `Config` deployment profile asserts every field the profile
//! is responsible for, plus the fields it deliberately leaves at the
//! `Config::on` default, so any change to a profile has to be made here too.
//! Each profile must also pass `Config::validate` as constructed.

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use rvoip_sip::{
    Config, PlayoutConfig, SessionError, SipContactMode, SipIcePolicy, SipTlsMode, SrtpKeyingMode,
};
use rvoip_sip_transport::transport::tls::TlsClientAuthMode;

const INSTANCE: &str = "urn:uuid:00000000-0000-0000-0000-0000000000aa";

fn addr(value: &str) -> SocketAddr {
    value.parse().unwrap()
}

fn media_at(ip: &str) -> Option<SocketAddr> {
    Some(SocketAddr::new(ip.parse().unwrap(), 0))
}

/// `PlayoutConfig` has no `PartialEq`; its `Debug` lists every field.
fn assert_default_playout(config: &Config) {
    assert_eq!(
        format!("{:?}", config.playout),
        format!("{:?}", Some(PlayoutConfig::default()))
    );
}

/// The fields every profile leaves alone unless its own test says otherwise.
fn assert_untouched_media_security(config: &Config) {
    assert_eq!(config.srtp_keying, SrtpKeyingMode::Sdes);
    assert!(config.offer_rtcp_mux);
    assert!(config.strict_codec_matching);
    assert_eq!(config.offered_codecs, vec![0, 8, 101]);
}

fn assert_no_session_timers(config: &Config) {
    assert_eq!(config.session_timer_secs, None);
    assert_eq!(config.session_timer_min_se, 90);
}

fn assert_profile_session_timers(config: &Config) {
    assert_eq!(config.session_timer_secs, Some(1800));
    assert_eq!(config.session_timer_min_se, 90);
    assert_eq!(Config::PROFILE_SESSION_TIMER_SECS, 1800);
    assert_eq!(Config::PROFILE_SESSION_TIMER_MIN_SE, 90);
}

fn assert_no_keepalive_pings(config: &Config) {
    assert!(config.options_keepalive_targets.is_empty());
    assert_eq!(config.options_keepalive_interval_secs, 60);
}

#[test]
fn local_lab_is_exactly_local() {
    let lab = Config::local_lab("alice", 5070);
    let local = Config::local("alice", 5070);
    assert_eq!(format!("{lab:?}"), format!("{local:?}"));
    assert_eq!(lab.local_uri, local.local_uri);
    assert_eq!(lab.bind_addr, addr("127.0.0.1:5070"));
    assert!(lab.playout.is_none());
    assert_eq!(lab.ice, SipIcePolicy::Disabled);
    assert_no_session_timers(&lab);
    assert_no_keepalive_pings(&lab);
    lab.validate().unwrap();
}

#[test]
fn lan_pbx_profile_fields() {
    let c = Config::lan_pbx("alice", addr("0.0.0.0:5060"), addr("192.168.1.50:5060"));
    assert_eq!(c.bind_addr, addr("0.0.0.0:5060"));
    assert_eq!(c.sip_advertised_addr, Some(addr("192.168.1.50:5060")));
    assert_eq!(c.media_public_addr, media_at("192.168.1.50"));
    // Deliberately off on a LAN.
    assert!(c.playout.is_none());
    assert_eq!(c.ice, SipIcePolicy::Disabled);
    assert_eq!(c.sip_tls_mode, SipTlsMode::Disabled);
    assert!(!c.offer_srtp && !c.srtp_required);
    assert!(!c.rtcp_mux_required);
    assert!(c.outbound_proxy_uri.is_none());
    assert!(c.stun_server.is_none());
    assert_no_session_timers(&c);
    assert_no_keepalive_pings(&c);
    assert_untouched_media_security(&c);
    c.validate().unwrap();
}

#[test]
#[allow(deprecated)]
fn freeswitch_internal_is_a_deprecated_lan_alias() {
    let c = Config::freeswitch_internal("alice", addr("192.168.1.50:5060"));
    let on = Config::on("alice", "192.168.1.50".parse().unwrap(), 5060);
    assert_eq!(format!("{c:?}"), format!("{on:?}"));
    assert!(c.strict_codec_matching);
    assert_eq!(c.bind_addr, addr("192.168.1.50:5060"));
    c.validate().unwrap();
}

#[test]
fn asterisk_tls_registered_flow_profile_fields() {
    let c = Config::asterisk_tls_registered_flow("alice", addr("0.0.0.0:5061"), INSTANCE);
    assert_eq!(c.bind_addr, addr("0.0.0.0:5061"));
    assert_eq!(c.sip_tls_mode, SipTlsMode::ClientOnly);
    assert_eq!(c.sip_contact_mode, SipContactMode::RegisteredFlowSymmetric);
    assert_eq!(c.sip_instance.as_deref(), Some(INSTANCE));
    assert!(c.offer_srtp && c.srtp_required);
    assert!(c.playout.is_none());
    assert_no_session_timers(&c);
    assert_no_keepalive_pings(&c);
    assert_untouched_media_security(&c);
    c.validate().unwrap();
}

#[test]
fn freeswitch_tls_srtp_profile_fields() {
    let c = Config::freeswitch_tls_srtp_reachable_contact(
        "alice",
        addr("0.0.0.0:5060"),
        addr("0.0.0.0:5061"),
        "cert.pem",
        "key.pem",
    );
    assert_eq!(c.bind_addr, addr("0.0.0.0:5060"));
    assert_eq!(c.sip_tls_mode, SipTlsMode::ClientAndServer);
    assert_eq!(c.sip_contact_mode, SipContactMode::ReachableContact);
    assert_eq!(c.tls_bind_addr, Some(addr("0.0.0.0:5061")));
    assert_eq!(c.tls_cert_path, Some(PathBuf::from("cert.pem")));
    assert_eq!(c.tls_key_path, Some(PathBuf::from("key.pem")));
    assert!(c.offer_srtp && c.srtp_required);
    assert_eq!(c.srtp_offered_suites.len(), 4);
    assert!(c.strict_codec_matching);
    assert_no_session_timers(&c);
    c.validate().unwrap();
}

#[test]
fn carrier_sbc_profile_fields() {
    let c = Config::carrier_sbc(
        "trunk",
        addr("0.0.0.0:5061"),
        addr("198.51.100.20:5061"),
        "sips:sbc.example.com:5061;lr",
        INSTANCE,
    );
    assert_eq!(c.bind_addr, addr("0.0.0.0:5061"));
    assert_eq!(c.sip_tls_mode, SipTlsMode::ClientOnly);
    assert_eq!(c.sip_contact_mode, SipContactMode::RegisteredFlowRfc5626);
    assert!(c.sip_outbound_enabled);
    assert_eq!(c.sip_instance.as_deref(), Some(INSTANCE));
    assert_eq!(c.outbound_keepalive_interval_secs, 25);
    assert_eq!(c.sip_advertised_addr, Some(addr("198.51.100.20:5061")));
    assert_eq!(c.tls_advertised_addr, Some(addr("198.51.100.20:5061")));
    assert_eq!(c.media_public_addr, media_at("198.51.100.20"));
    assert_eq!(
        c.outbound_proxy_uri.as_deref(),
        Some("sips:sbc.example.com:5061;lr")
    );
    assert!(c.offer_srtp && c.srtp_required);
    assert_default_playout(&c);
    assert_profile_session_timers(&c);
    assert_eq!(c.ice, SipIcePolicy::Disabled);
    assert!(!c.rtcp_mux_required);
    assert_no_keepalive_pings(&c);
    assert_untouched_media_security(&c);
    c.validate().unwrap();
}

#[test]
fn carrier_trunk_udp_profile_fields() {
    let c = Config::carrier_trunk_udp(
        "pbx",
        addr("10.0.0.5:5060"),
        addr("203.0.113.10:5060"),
        "sip:sbc.carrier.example:5060;lr",
    );
    assert_eq!(c.bind_addr, addr("10.0.0.5:5060"));
    assert_eq!(c.local_uri, "sip:pbx@10.0.0.5:5060");
    assert_eq!(c.sip_advertised_addr, Some(addr("203.0.113.10:5060")));
    assert_eq!(c.media_public_addr, media_at("203.0.113.10"));
    assert_eq!(
        c.outbound_proxy_uri.as_deref(),
        Some("sip:sbc.carrier.example:5060;lr")
    );
    assert_default_playout(&c);
    assert_profile_session_timers(&c);
    // IP-authenticated plain UDP trunk: no TLS, no SRTP, no registration.
    assert_eq!(c.sip_tls_mode, SipTlsMode::Disabled);
    assert_eq!(c.sip_contact_mode, SipContactMode::ReachableContact);
    assert!(!c.sip_outbound_enabled);
    assert!(c.sip_instance.is_none());
    assert!(c.credentials.is_none() && c.auth.is_none());
    assert!(!c.offer_srtp && !c.srtp_required);
    assert_eq!(c.ice, SipIcePolicy::Disabled);
    assert!(c.stun_server.is_none());
    assert!(!c.rtcp_mux_required);
    assert_no_keepalive_pings(&c);
    assert_untouched_media_security(&c);
    c.validate().unwrap();
}

#[test]
fn public_server_profile_fields() {
    let c = Config::public_server("ivr", addr("10.0.0.5:5060"), addr("203.0.113.10:5060"));
    assert_eq!(c.bind_addr, addr("10.0.0.5:5060"));
    assert_eq!(c.sip_advertised_addr, Some(addr("203.0.113.10:5060")));
    assert_eq!(c.media_public_addr, media_at("203.0.113.10"));
    assert_eq!(c.ice, SipIcePolicy::Lite);
    assert_default_playout(&c);
    assert_profile_session_timers(&c);
    assert_eq!(c.sip_tls_mode, SipTlsMode::Disabled);
    assert!(!c.offer_srtp && !c.srtp_required);
    assert!(c.outbound_proxy_uri.is_none());
    assert!(c.stun_server.is_none());
    assert!(!c.rtcp_mux_required);
    assert_no_keepalive_pings(&c);
    assert_untouched_media_security(&c);
    c.validate().unwrap();
}

#[test]
fn public_server_tls_listener_advertises_the_public_ip() {
    let c = Config::public_server("ivr", addr("10.0.0.5:5060"), addr("203.0.113.10:5060"))
        .tls_reachable_contact(addr("0.0.0.0:5061"), "cert.pem", "key.pem");
    assert_eq!(c.sip_tls_mode, SipTlsMode::ClientAndServer);
    assert_eq!(c.tls_bind_addr, Some(addr("0.0.0.0:5061")));
    assert_eq!(c.tls_advertised_addr, Some(addr("203.0.113.10:5061")));
    c.validate().unwrap();

    // An explicit advertised TLS address is never overwritten.
    let mut explicit =
        Config::public_server("ivr", addr("10.0.0.5:5060"), addr("203.0.113.10:5060"));
    explicit.tls_advertised_addr = Some(addr("203.0.113.11:443"));
    let explicit = explicit.tls_reachable_contact(addr("0.0.0.0:5061"), "cert.pem", "key.pem");
    assert_eq!(explicit.tls_advertised_addr, Some(addr("203.0.113.11:443")));

    // Without a public address the concrete bind address is advertised, as before.
    let lan = Config::local("alice", 5060).tls_reachable_contact(
        addr("127.0.0.1:5061"),
        "cert.pem",
        "key.pem",
    );
    assert_eq!(lan.tls_advertised_addr, Some(addr("127.0.0.1:5061")));
}

#[test]
fn behind_nat_profile_fields() {
    let c = Config::behind_nat(
        "alice",
        addr("0.0.0.0:5061"),
        "stun.example.com:3478",
        INSTANCE,
    );
    assert_eq!(c.bind_addr, addr("0.0.0.0:5061"));
    assert_eq!(c.sip_tls_mode, SipTlsMode::ClientOnly);
    assert_eq!(c.sip_contact_mode, SipContactMode::RegisteredFlowRfc5626);
    assert!(c.sip_outbound_enabled);
    assert_eq!(c.sip_instance.as_deref(), Some(INSTANCE));
    assert_eq!(c.outbound_keepalive_interval_secs, 25);
    assert_eq!(c.stun_server.as_deref(), Some("stun.example.com:3478"));
    assert_eq!(c.ice, SipIcePolicy::Full);
    assert!(c.rtcp_mux_required);
    assert!(c.offer_rtcp_mux);
    assert_default_playout(&c);
    // STUN discovers the mapping; a static public address would override it.
    assert!(c.media_public_addr.is_none());
    assert!(c.sip_advertised_addr.is_none());
    assert!(!c.offer_srtp && !c.srtp_required);
    assert_no_session_timers(&c);
    assert_no_keepalive_pings(&c);
    c.validate().unwrap();
}

#[test]
fn proxy_rtpengine_profile_fields() {
    let c = Config::proxy_rtpengine(
        "alice",
        addr("0.0.0.0:5060"),
        addr("192.168.1.50:5060"),
        "sip:proxy.example.com;lr",
    );
    assert_eq!(c.bind_addr, addr("0.0.0.0:5060"));
    assert_eq!(c.sip_advertised_addr, Some(addr("192.168.1.50:5060")));
    assert_eq!(c.media_public_addr, media_at("192.168.1.50"));
    assert_eq!(
        c.outbound_proxy_uri.as_deref(),
        Some("sip:proxy.example.com;lr")
    );
    assert_default_playout(&c);
    assert_profile_session_timers(&c);
    assert_eq!(c.sip_tls_mode, SipTlsMode::Disabled);
    assert!(!c.offer_srtp && !c.srtp_required);
    assert_eq!(c.ice, SipIcePolicy::Disabled);
    assert!(!c.rtcp_mux_required);
    assert_no_keepalive_pings(&c);
    assert_untouched_media_security(&c);
    c.validate().unwrap();
}

#[test]
fn tls_direct_routing_profile_fields() {
    let public_ip: IpAddr = "203.0.113.10".parse().unwrap();
    let c = Config::tls_direct_routing(
        "sbc",
        "sbc1.example.com",
        addr("10.0.0.5:5061"),
        public_ip,
        "sbc.crt",
        "sbc.key",
        "roots.pem",
        "sip:sip.pstnhub.microsoft.com:5061;transport=tls",
    );
    assert_eq!(c.bind_addr, addr("10.0.0.5:0"));
    assert_eq!(c.local_uri, "sip:sbc@sbc1.example.com");
    assert_eq!(
        c.contact_uri.as_deref(),
        Some("sip:sbc1.example.com:5061;transport=tls")
    );
    assert_eq!(c.sip_tls_mode, SipTlsMode::ClientAndServer);
    assert_eq!(c.sip_contact_mode, SipContactMode::ReachableContact);
    assert_eq!(c.tls_bind_addr, Some(addr("10.0.0.5:5061")));
    assert_eq!(c.tls_advertised_addr, Some(addr("203.0.113.10:5061")));
    assert_eq!(c.tls_cert_path, Some(PathBuf::from("sbc.crt")));
    assert_eq!(c.tls_key_path, Some(PathBuf::from("sbc.key")));
    assert_eq!(c.tls_client_cert_path, Some(PathBuf::from("sbc.crt")));
    assert_eq!(c.tls_client_key_path, Some(PathBuf::from("sbc.key")));
    assert_eq!(c.tls_server_client_auth.mode, TlsClientAuthMode::Required);
    assert_eq!(
        c.tls_server_client_auth.client_ca_path,
        Some(PathBuf::from("roots.pem"))
    );
    assert!(c.offer_srtp && c.srtp_required);
    assert_eq!(c.srtp_keying, SrtpKeyingMode::Sdes);
    assert_eq!(
        c.srtp_offered_suites.first(),
        Some(&rvoip_sip_core::types::sdp::CryptoSuite::AesCm128HmacSha1_80)
    );
    assert_eq!(c.media_public_addr, Some(SocketAddr::new(public_ip, 0)));
    assert_eq!(c.ice, SipIcePolicy::Lite);
    assert_default_playout(&c);
    assert_profile_session_timers(&c);
    assert_eq!(
        c.options_keepalive_targets,
        vec!["sip:sip.pstnhub.microsoft.com:5061;transport=tls".to_string()]
    );
    assert_eq!(c.options_keepalive_interval_secs, 60);
    // No registration: the peer trusts the certificate.
    assert!(!c.sip_outbound_enabled);
    assert!(c.sip_instance.is_none());
    assert!(c.credentials.is_none() && c.auth.is_none());
    assert!(c.outbound_proxy_uri.is_none());
    assert!(c.sip_advertised_addr.is_none());
    c.validate().unwrap();
}

#[test]
fn every_profile_field_stays_overridable() {
    let mut c = Config::carrier_trunk_udp(
        "pbx",
        addr("10.0.0.5:5060"),
        addr("203.0.113.10:5060"),
        "sip:sbc.carrier.example;lr",
    );
    c.session_timer_secs = None;
    c.playout = None;
    c.offer_srtp = true;
    c.options_keepalive_targets
        .push("sip:sbc.carrier.example".to_string());
    c.validate().unwrap();
    assert!(c.session_timer_secs.is_none() && c.playout.is_none());
}

#[test]
fn validate_rejects_session_interval_below_min_se() {
    let mut c = Config::local("alice", 5060);
    c.session_timer_secs = Some(60);
    let error = c.validate().expect_err("60 < Min-SE 90");
    assert!(
        matches!(&error, SessionError::ConfigError(message) if message.contains("session_timer_min_se")),
        "{error:?}"
    );
    c.session_timer_min_se = 60;
    c.validate().unwrap();
}

#[test]
fn validate_rejects_unparseable_keepalive_targets() {
    let mut c = Config::local("alice", 5060);
    c.options_keepalive_targets.push("not a uri".to_string());
    let error = c.validate().expect_err("invalid target");
    assert!(
        matches!(&error, SessionError::ConfigError(message) if message.contains("options_keepalive_targets")),
        "{error:?}"
    );
}
