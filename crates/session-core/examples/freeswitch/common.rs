#![allow(dead_code)]

use rvoip_session_core::{Config, Registration};
use std::env;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

pub fn env_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

pub fn env_u16(key: &str, default: u16) -> u16 {
    env::var(key)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

pub fn env_duration_secs(key: &str, default: u64) -> Duration {
    Duration::from_secs(
        env::var(key)
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(default),
    )
}

pub fn bind_addr(default_port: u16) -> SocketAddr {
    let ip: IpAddr = env_or("RVOIP_LOCAL_IP", "127.0.0.1")
        .parse()
        .expect("RVOIP_LOCAL_IP must be an IP address");
    let port = env_u16("RVOIP_SIP_PORT", default_port);
    SocketAddr::new(ip, port)
}

pub fn freeswitch_addr() -> String {
    env_or("FREESWITCH_ADDR", "127.0.0.1:5060")
}

pub fn config(user: &str, default_port: u16) -> Config {
    Config::freeswitch_internal(user, bind_addr(default_port))
}

pub fn registration(user: &str, password: &str) -> Registration {
    let registrar = format!("sip:{}", freeswitch_addr());
    Registration::new(registrar, user, password)
}

pub fn call_uri(user: &str) -> String {
    format!("sip:{}@{}", user, freeswitch_addr())
}
