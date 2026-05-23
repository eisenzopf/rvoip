//! Simplified Session Builder
//!
//! Just builds the UnifiedCoordinator with configuration.
//! No complex setup - the state table handles everything.

use crate::api::unified::{Config, UnifiedCoordinator};
use crate::errors::Result;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

/// Builder for creating a UnifiedCoordinator
pub struct SessionBuilder {
    config: Config,
}

impl SessionBuilder {
    /// Create a new builder with default configuration
    pub fn new() -> Self {
        Self {
            config: Config::default(),
        }
    }

    /// Set the SIP port
    pub fn with_sip_port(mut self, port: u16) -> Self {
        self.config.sip_port = port;
        self.config.bind_addr.set_port(port);
        self
    }

    /// Set the media port range
    pub fn with_media_ports(mut self, start: u16, end: u16) -> Self {
        self.config = self.config.with_media_ports(start, end);
        self
    }

    /// Set the legacy incoming-call compatibility channel capacity.
    pub fn with_incoming_call_channel_capacity(mut self, capacity: usize) -> Self {
        self.config.incoming_call_channel_capacity = capacity;
        self
    }

    /// Set SIP signaling channel capacities from one expected-concurrency knob.
    ///
    /// Per-call queues use `capacity`; lower-level transport and transaction
    /// queues use `capacity * 10` because each call generates several SIP
    /// messages and transaction events.
    pub fn with_channel_capacity(mut self, capacity: usize) -> Self {
        self.config = self.config.with_channel_capacity(capacity);
        self
    }

    /// Set a server-side active-call capacity profile.
    pub fn with_server_capacity(mut self, capacity: usize) -> Self {
        self.config = self.config.with_server_capacity(capacity);
        self
    }

    /// Set the internal state-machine event channel capacity.
    pub fn with_state_event_channel_capacity(mut self, capacity: usize) -> Self {
        self.config.state_event_channel_capacity = capacity;
        self
    }

    /// Set the SIP transport event channel capacity.
    pub fn with_sip_transport_channel_capacity(mut self, capacity: usize) -> Self {
        self.config.sip_transport_channel_capacity = capacity;
        self
    }

    /// Set SIP UDP socket receive/send buffer sizes in bytes.
    pub fn with_sip_udp_socket_buffers(
        mut self,
        recv_buffer_size: Option<usize>,
        send_buffer_size: Option<usize>,
    ) -> Self {
        self.config = self
            .config
            .with_sip_udp_socket_buffers(recv_buffer_size, send_buffer_size);
        self
    }

    /// Set the SIP UDP receive socket buffer size (`SO_RCVBUF`) in bytes.
    pub fn with_sip_udp_recv_buffer_size(mut self, size: usize) -> Self {
        self.config.sip_udp_recv_buffer_size = Some(size);
        self
    }

    /// Set the SIP UDP send socket buffer size (`SO_SNDBUF`) in bytes.
    pub fn with_sip_udp_send_buffer_size(mut self, size: usize) -> Self {
        self.config.sip_udp_send_buffer_size = Some(size);
        self
    }

    /// Set the transaction-manager event channel capacity.
    pub fn with_transaction_event_channel_capacity(mut self, capacity: usize) -> Self {
        self.config.transaction_event_channel_capacity = capacity;
        self
    }

    /// Set the app-session event dispatcher worker count.
    pub fn with_session_event_dispatcher_workers(mut self, workers: usize) -> Self {
        self.config.session_event_dispatcher_workers = workers;
        self
    }

    /// Set the app-session event dispatcher per-worker queue capacity.
    pub fn with_session_event_dispatcher_channel_capacity(mut self, capacity: usize) -> Self {
        self.config.session_event_dispatcher_channel_capacity = capacity;
        self
    }

    /// Set the local IP address
    pub fn with_local_ip(mut self, ip: IpAddr) -> Self {
        self.config.local_ip = ip;
        self.config.bind_addr.set_ip(ip);
        self
    }

    /// Set the bind address
    pub fn with_bind_addr(mut self, addr: SocketAddr) -> Self {
        self.config.bind_addr = addr;
        self.config.local_ip = addr.ip();
        self.config.sip_port = addr.port();
        self
    }

    /// Build the UnifiedCoordinator
    pub async fn build(self) -> Result<Arc<UnifiedCoordinator>> {
        UnifiedCoordinator::new(self.config).await
    }
}

impl Default for SessionBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_configuration() {
        let builder = SessionBuilder::new()
            .with_sip_port(5061)
            .with_media_ports(20000, 30000)
            .with_local_ip("192.168.1.100".parse().unwrap());

        assert_eq!(builder.config.sip_port, 5061);
        assert_eq!(builder.config.media_port_start, 20000);
        assert_eq!(builder.config.media_port_end, 30000);
        assert_eq!(builder.config.local_ip.to_string(), "192.168.1.100");
    }

    #[test]
    fn test_builder_channel_capacity_profile() {
        let builder = SessionBuilder::new()
            .with_channel_capacity(256)
            .with_server_capacity(128)
            .with_sip_udp_socket_buffers(Some(65_536), Some(32_768));

        assert_eq!(builder.config.incoming_call_channel_capacity, 256);
        assert_eq!(builder.config.state_event_channel_capacity, 256);
        assert_eq!(builder.config.sip_transport_channel_capacity, 2560);
        assert_eq!(builder.config.server_call_capacity, Some(128));
        assert_eq!(builder.config.sip_udp_recv_buffer_size, Some(65_536));
        assert_eq!(builder.config.sip_udp_send_buffer_size, Some(32_768));
        assert_eq!(builder.config.transaction_event_channel_capacity, 2560);
        assert_eq!(
            builder.config.session_event_dispatcher_channel_capacity,
            2560
        );
    }
}
