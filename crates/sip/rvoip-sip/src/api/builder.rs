//! Simplified Session Builder
//!
//! Just builds the UnifiedCoordinator with configuration.
//! No complex setup - the state table handles everything.

use crate::api::performance::PerformanceConfig;
use crate::api::unified::{
    Config, MediaMode, MediaSessionControllerConfig, RtpSessionBufferConfig,
    RtpTransportBufferConfig, UnifiedCoordinator,
};
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

    /// Set app-facing event buffer capacity.
    pub fn with_app_event_channel_capacity(mut self, capacity: usize) -> Self {
        self.config = self.config.with_app_event_channel_capacity(capacity);
        self
    }

    /// Set a server-side active-call capacity profile.
    pub fn with_server_capacity(mut self, capacity: usize) -> Self {
        self.config = self.config.with_server_capacity(capacity);
        self
    }

    /// Set the bound for active and retained SIP lifecycle records.
    pub fn with_server_retained_lifecycle_capacity(mut self, capacity: usize) -> Self {
        self.config = self
            .config
            .with_server_retained_lifecycle_capacity(capacity);
        self
    }

    /// Set the server-side inbound call admission limit.
    pub fn with_server_call_admission_limit(mut self, limit: usize) -> Self {
        self.config = self.config.with_server_call_admission_limit(limit);
        self
    }

    /// Set the soft threshold where server-side admission starts pacing.
    pub fn with_server_call_admission_soft_limit(mut self, limit: usize) -> Self {
        self.config = self.config.with_server_call_admission_soft_limit(limit);
        self
    }

    /// Set the delay in milliseconds while above the soft admission threshold.
    pub fn with_server_call_admission_pacing_delay_ms(mut self, delay_ms: u64) -> Self {
        self.config = self
            .config
            .with_server_call_admission_pacing_delay_ms(delay_ms);
        self
    }

    /// Set the `Retry-After` value used for server overload rejections.
    pub fn with_server_overload_retry_after_secs(mut self, seconds: u32) -> Self {
        self.config = self.config.with_server_overload_retry_after_secs(seconds);
        self
    }

    /// Apply the high-CPS UDP auto-answer profile.
    pub fn with_high_cps_udp_auto_answer(mut self, capacity: usize) -> Self {
        self.config = self.config.with_high_cps_udp_auto_answer(capacity);
        self
    }

    /// Apply a YAML-backed performance recipe.
    pub fn with_performance_config(mut self, performance: PerformanceConfig) -> Result<Self> {
        self.config = self.config.try_with_performance_config(performance)?;
        Ok(self)
    }

    /// Apply the PBX media server performance recipe.
    pub fn with_pbx_media_server_performance(mut self, capacity: usize) -> Self {
        self.config = self.config.with_pbx_media_server_performance(capacity);
        self
    }

    /// Apply the signaling-only high-performance server recipe.
    pub fn with_signaling_only_server_high_performance(
        mut self,
        capacity: usize,
        sdp_rtp_port: u16,
    ) -> Self {
        self.config = self
            .config
            .with_signaling_only_server_high_performance(capacity, sdp_rtp_port);
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

    /// Set RTP session queue sizing for SIP media calls.
    pub fn with_rtp_session_buffer_config(mut self, config: RtpSessionBufferConfig) -> Self {
        self.config = self.config.with_rtp_session_buffer_config(config);
        self
    }

    /// Set RTP transport event and receive buffer sizing for SIP media calls.
    pub fn with_rtp_transport_buffer_config(mut self, config: RtpTransportBufferConfig) -> Self {
        self.config = self.config.with_rtp_transport_buffer_config(config);
        self
    }

    /// Set media-core controller pool and capacity tuning for SIP media calls.
    pub fn with_media_session_controller_config(
        mut self,
        config: MediaSessionControllerConfig,
    ) -> Self {
        self.config = self.config.with_media_session_controller_config(config);
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

    /// Set the SIP transport-manager forwarding worker count.
    pub fn with_sip_transport_dispatch_workers(mut self, workers: usize) -> Self {
        self.config.sip_transport_dispatch_workers = Some(workers);
        self
    }

    /// Set the SIP transport-manager forwarding queue capacity.
    pub fn with_sip_transport_dispatch_queue_capacity(mut self, capacity: usize) -> Self {
        self.config.sip_transport_dispatch_queue_capacity = Some(capacity);
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

    /// Set the UDP parse worker dispatch strategy.
    pub fn with_sip_udp_parse_dispatch(
        mut self,
        dispatch: rvoip_sip_transport::UdpParseDispatch,
    ) -> Self {
        self.config.sip_udp_parse_dispatch = Some(dispatch);
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

    /// Set the RSS growth threshold used by perf soak release gates.
    #[cfg(feature = "perf-tests")]
    pub fn with_perf_max_rss_growth_mb_per_hr(mut self, limit: f64) -> Self {
        self.config = self.config.with_perf_max_rss_growth_mb_per_hr(limit);
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

    /// Set the transaction-manager ingress dispatch worker count.
    pub fn with_sip_transaction_dispatch_workers(mut self, workers: usize) -> Self {
        self.config.sip_transaction_dispatch_workers = Some(workers);
        self
    }

    /// Set the transaction-manager ingress dispatch queue capacity.
    pub fn with_sip_transaction_dispatch_queue_capacity(mut self, capacity: usize) -> Self {
        self.config.sip_transaction_dispatch_queue_capacity = Some(capacity);
        self
    }

    /// Set the per-transaction command channel capacity.
    pub fn with_sip_transaction_command_channel_capacity(mut self, capacity: usize) -> Self {
        self.config = self
            .config
            .with_sip_transaction_command_channel_capacity(capacity);
        self
    }

    /// Set the transaction-manager ACK/BYE priority burst limit.
    pub fn with_sip_transaction_dispatch_priority_burst_max(mut self, max_burst: usize) -> Self {
        self.config.sip_transaction_dispatch_priority_burst_max = Some(max_burst);
        self
    }

    /// Set the cached INVITE `2xx` retransmission maintenance budget.
    pub fn with_sip_invite_2xx_retransmit_max_due_per_tick(
        mut self,
        max_due_per_tick: usize,
    ) -> Self {
        self.config.sip_invite_2xx_retransmit_max_due_per_tick = Some(max_due_per_tick);
        self
    }

    /// Set the rvoip-sip-dialog transaction-event dispatch worker count.
    pub fn with_sip_dialog_dispatch_workers(mut self, workers: usize) -> Self {
        self.config.sip_dialog_dispatch_workers = Some(workers);
        self
    }

    /// Set the rvoip-sip-dialog transaction-event dispatch queue capacity.
    pub fn with_sip_dialog_dispatch_queue_capacity(mut self, capacity: usize) -> Self {
        self.config.sip_dialog_dispatch_queue_capacity = Some(capacity);
        self
    }

    /// Enable or disable high-cardinality dialog timing diagnostics.
    pub fn with_sip_dialog_timing_diagnostics(mut self, enabled: bool) -> Self {
        self.config = self.config.with_sip_dialog_timing_diagnostics(enabled);
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
            .with_server_retained_lifecycle_capacity(1024)
            .with_server_call_admission_limit(512)
            .with_server_call_admission_soft_limit(480)
            .with_server_call_admission_pacing_delay_ms(2)
            .with_server_overload_retry_after_secs(2)
            .with_sip_transport_dispatch_workers(2)
            .with_sip_transport_dispatch_queue_capacity(4096)
            .with_sip_udp_socket_buffers(Some(65_536), Some(32_768))
            .with_sip_udp_parse_workers(4)
            .with_sip_udp_parse_queue_capacity(8192)
            .with_sip_udp_parse_dispatch(rvoip_sip_transport::UdpParseDispatch::RoundRobin)
            .with_sip_transaction_dispatch_workers(4)
            .with_sip_transaction_dispatch_queue_capacity(8192)
            .with_sip_transaction_dispatch_priority_burst_max(32)
            .with_sip_invite_2xx_retransmit_max_due_per_tick(512)
            .with_sip_dialog_dispatch_workers(4)
            .with_sip_dialog_dispatch_queue_capacity(8192)
            .with_app_event_channel_capacity(2048)
            .with_sip_dialog_timing_diagnostics(true);

        assert_eq!(builder.config.incoming_call_channel_capacity, 256);
        assert_eq!(builder.config.state_event_channel_capacity, 256);
        assert_eq!(builder.config.sip_transport_channel_capacity, 2560);
        assert_eq!(builder.config.sip_transport_dispatch_workers, Some(2));
        assert_eq!(
            builder.config.sip_transport_dispatch_queue_capacity,
            Some(4096)
        );
        assert_eq!(builder.config.server_call_capacity, Some(128));
        assert_eq!(
            builder.config.server_retained_lifecycle_capacity,
            Some(1024)
        );
        assert_eq!(builder.config.server_call_admission_limit, Some(512));
        assert_eq!(builder.config.server_call_admission_soft_limit, Some(480));
        assert_eq!(
            builder.config.server_call_admission_pacing_delay_ms,
            Some(2)
        );
        assert_eq!(builder.config.server_overload_retry_after_secs, Some(2));
        assert_eq!(builder.config.sip_udp_recv_buffer_size, Some(65_536));
        assert_eq!(builder.config.sip_udp_send_buffer_size, Some(32_768));
        assert_eq!(builder.config.sip_udp_parse_workers, Some(4));
        assert_eq!(builder.config.sip_udp_parse_queue_capacity, Some(8192));
        assert_eq!(
            builder.config.sip_udp_parse_dispatch,
            Some(rvoip_sip_transport::UdpParseDispatch::RoundRobin)
        );
        assert_eq!(builder.config.transaction_event_channel_capacity, 2560);
        assert_eq!(builder.config.sip_transaction_dispatch_workers, Some(4));
        assert_eq!(
            builder.config.sip_transaction_dispatch_queue_capacity,
            Some(8192)
        );
        assert_eq!(
            builder.config.sip_transaction_dispatch_priority_burst_max,
            Some(32)
        );
        assert_eq!(
            builder.config.sip_invite_2xx_retransmit_max_due_per_tick,
            Some(512)
        );
        assert_eq!(builder.config.sip_dialog_dispatch_workers, Some(4));
        assert_eq!(
            builder.config.sip_dialog_dispatch_queue_capacity,
            Some(8192)
        );
        assert!(builder.config.sip_dialog_timing_diagnostics);
        assert_eq!(
            builder.config.session_event_dispatcher_channel_capacity,
            2048
        );
        assert_eq!(builder.config.global_event_channel_capacity, 2048);
    }

    #[test]
    fn test_builder_rtp_media_buffer_tuning() {
        let session_buffers = RtpSessionBufferConfig {
            sender_channel_capacity: 8,
            receiver_channel_capacity: 4,
            event_channel_capacity: 12,
        };
        let transport_buffers = RtpTransportBufferConfig {
            event_channel_capacity: 10,
            recv_buffer_size: 2048,
            rtcp_recv_buffer_size: 1024,
        };
        let media_config = MediaSessionControllerConfig {
            rtp_buffer_size: 960,
            rtp_buffer_initial_count: 4,
            rtp_buffer_max_count: 16,
            ..Default::default()
        };

        let builder = SessionBuilder::new()
            .with_media_session_controller_config(media_config)
            .with_rtp_session_buffer_config(session_buffers)
            .with_rtp_transport_buffer_config(transport_buffers);

        assert_eq!(builder.config.rtp_session_buffer_config, session_buffers);
        assert_eq!(
            builder.config.rtp_transport_buffer_config,
            transport_buffers
        );
        assert_eq!(
            builder
                .config
                .media_session_controller_config
                .rtp_buffer_size,
            960
        );
        assert_eq!(
            builder
                .config
                .media_session_controller_config
                .rtp_buffer_initial_count,
            4
        );
        assert_eq!(
            builder
                .config
                .media_session_controller_config
                .rtp_buffer_max_count,
            16
        );
    }
}
