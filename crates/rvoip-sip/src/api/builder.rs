//! Simplified Session Builder
//!
//! Just builds the UnifiedCoordinator with configuration.
//! No complex setup - the state table handles everything.

use crate::api::unified::{Config, MediaMode, UnifiedCoordinator};
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

    /// Enable or disable automatic `180 Ringing` on inbound INVITEs.
    pub fn with_auto_180_ringing(mut self, enabled: bool) -> Self {
        self.config = self.config.with_auto_180_ringing(enabled);
        self
    }

    /// Enable or disable automatic `100 Trying` timer tasks on inbound INVITEs.
    pub fn with_auto_100_trying(mut self, enabled: bool) -> Self {
        self.config = self.config.with_auto_100_trying(enabled);
        self
    }

    /// Enable or disable immediate session-path accept for inbound INVITEs.
    pub fn with_fast_auto_accept_incoming_calls(mut self, enabled: bool) -> Self {
        self.config = self.config.with_fast_auto_accept_incoming_calls(enabled);
        self
    }

    /// Enable or disable real media-core RTP allocation.
    pub fn with_media_enabled(mut self, enabled: bool) -> Self {
        self.config = self.config.with_media_enabled(enabled);
        self
    }

    /// Skip media-core RTP allocation while still generating SDP.
    pub fn with_signaling_only_media(mut self, sdp_rtp_port: u16) -> Self {
        self.config = self
            .config
            .with_media_mode(MediaMode::SignalingOnly { sdp_rtp_port });
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

    /// Apply the high-CPS UDP auto-answer profile.
    pub fn with_high_cps_udp_auto_answer(mut self, capacity: usize) -> Self {
        self.config = self.config.with_high_cps_udp_auto_answer(capacity);
        self
    }

    /// Set the RTP media port range by start port and requested capacity.
    pub fn with_media_port_capacity(mut self, start: u16, capacity: usize) -> Self {
        self.config = self.config.with_media_port_capacity(start, capacity);
        self
    }

    /// Set the media-core session and RTP allocator capacity hint.
    pub fn with_media_session_capacity(mut self, capacity: usize) -> Self {
        self.config = self.config.with_media_session_capacity(capacity);
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

    /// Set the UDP parse worker count.
    pub fn with_sip_udp_parse_workers(mut self, workers: usize) -> Self {
        self.config.sip_udp_parse_workers = Some(workers);
        self
    }

    /// Set the per-worker UDP parse queue capacity.
    pub fn with_sip_udp_parse_queue_capacity(mut self, capacity: usize) -> Self {
        self.config.sip_udp_parse_queue_capacity = Some(capacity);
        self
    }

    /// Enable or disable SIP UDP transport and duplicate-recovery diagnostics.
    pub fn with_sip_udp_diagnostics(mut self, enabled: bool) -> Self {
        self.config = self.config.with_sip_udp_diagnostics(enabled);
        self
    }

    /// Enable or disable high-cardinality transaction timing diagnostics.
    pub fn with_sip_transaction_timing_diagnostics(mut self, enabled: bool) -> Self {
        self.config = self.config.with_sip_transaction_timing_diagnostics(enabled);
        self
    }

    /// Enable or disable media setup/teardown timing diagnostics.
    pub fn with_media_setup_diagnostics(mut self, enabled: bool) -> Self {
        self.config = self.config.with_media_setup_diagnostics(enabled);
        self
    }

    /// Enable or disable cleanup-stage timing diagnostics.
    pub fn with_cleanup_diagnostics(mut self, enabled: bool) -> Self {
        self.config = self.config.with_cleanup_diagnostics(enabled);
        self
    }

    /// Enable or disable per-operation cleanup diagnostic event logs.
    pub fn with_cleanup_diagnostic_events(mut self, enabled: bool) -> Self {
        self.config = self.config.with_cleanup_diagnostic_events(enabled);
        self
    }

    /// Enable or disable SRTP negotiation diagnostic log lines.
    pub fn with_srtp_diagnostics(mut self, enabled: bool) -> Self {
        self.config = self.config.with_srtp_diagnostics(enabled);
        self
    }

    /// Enable or disable RTP packet diagnostic log lines.
    pub fn with_rtp_diagnostics(mut self, enabled: bool) -> Self {
        self.config = self.config.with_rtp_diagnostics(enabled);
        self
    }

    /// Enable or disable SDP media diagnostic log lines.
    pub fn with_media_sdp_diagnostics(mut self, enabled: bool) -> Self {
        self.config = self.config.with_media_sdp_diagnostics(enabled);
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
