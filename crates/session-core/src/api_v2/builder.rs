//! Simplified Session Builder (v2)

use crate::errors_v2::Result;
use std::net::{IpAddr, SocketAddr};

/// Configuration for the session builder
#[derive(Debug, Clone)]
pub struct Config {
    pub local_ip: IpAddr,
    pub sip_port: u16,
    pub media_port_start: u16,
    pub media_port_end: u16,
    pub bind_addr: SocketAddr,
    pub state_table_path: Option<String>,
    pub local_uri: String,
}

impl Default for Config {
    fn default() -> Self {
        let ip = "127.0.0.1".parse::<IpAddr>().unwrap_or_else(|_| IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
        let port = 5060;
        Self {
            local_ip: ip,
            sip_port: port,
            media_port_start: 16000,
            media_port_end: 17000,
            bind_addr: SocketAddr::new(ip, port),
            state_table_path: None,
            local_uri: format!("sip:user@{}:{}", ip, port),
        }
    }
}

/// Builder for creating sessions
pub struct SessionBuilder {
    config: Config,
}

impl SessionBuilder {
    pub fn new() -> Self {
        Self { config: Config::default() }
    }

    pub fn with_sip_port(mut self, port: u16) -> Self {
        self.config.sip_port = port;
        self.config.bind_addr.set_port(port);
        self
    }

    pub fn with_media_ports(mut self, start: u16, end: u16) -> Self {
        self.config.media_port_start = start;
        self.config.media_port_end = end;
        self
    }

    pub fn with_local_ip(mut self, ip: IpAddr) -> Self {
        self.config.local_ip = ip;
        self.config.bind_addr.set_ip(ip);
        self
    }

    pub fn with_bind_addr(mut self, addr: SocketAddr) -> Self {
        self.config.bind_addr = addr;
        self.config.local_ip = addr.ip();
        self.config.sip_port = addr.port();
        self
    }
}

impl Default for SessionBuilder {
    fn default() -> Self {
        Self::new()
    }
}
