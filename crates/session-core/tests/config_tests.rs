//! Configuration Tests
//!
//! Tests Config constructors, defaults, and field values.

use rvoip_session_core::Config;
use std::net::IpAddr;

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
    assert_eq!(c.media_port_start, 16000);
    assert_eq!(c.media_port_end, 17000);
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
