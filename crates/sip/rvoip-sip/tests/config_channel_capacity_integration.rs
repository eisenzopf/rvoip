use std::time::Duration;

use rvoip_sip::{
    cleanup_diag, Config, MediaSessionControllerConfig, RtpSessionBufferConfig,
    RtpTransportBufferConfig, SessionError, UnifiedCoordinator,
};
use serial_test::serial;

fn config_error_detail(error: &SessionError) -> &str {
    let SessionError::ConfigError(detail) = error else {
        panic!("expected typed ConfigError, got {error:?}");
    };
    let rendered = error.to_string();
    assert!(
        !rendered.contains(detail),
        "ConfigError Display must remain redacted"
    );
    assert!(rendered.contains("redacted"));
    detail
}

#[test]
fn incoming_call_channel_capacity_defaults_to_1000() {
    assert_eq!(Config::default().incoming_call_channel_capacity, 1000);
    assert_eq!(Config::default().state_event_channel_capacity, 1000);
    assert_eq!(Config::default().sip_transport_channel_capacity, 10_000);
    assert_eq!(Config::default().sip_transport_dispatch_workers, None);
    assert_eq!(
        Config::default().sip_transport_dispatch_queue_capacity,
        None
    );
    assert_eq!(Config::default().sip_udp_recv_buffer_size, None);
    assert_eq!(Config::default().sip_udp_send_buffer_size, None);
    assert_eq!(Config::default().sip_udp_parse_workers, None);
    assert_eq!(Config::default().sip_udp_parse_queue_capacity, None);
    assert_eq!(Config::default().sip_udp_parse_dispatch, None);
    assert_eq!(Config::default().media_port_capacity, None);
    assert_eq!(Config::default().media_session_capacity, None);
    assert_eq!(
        Config::default().rtp_session_buffer_config,
        RtpSessionBufferConfig::default()
    );
    assert_eq!(
        Config::default().rtp_transport_buffer_config,
        RtpTransportBufferConfig::default()
    );
    assert_eq!(
        Config::default()
            .media_session_controller_config
            .rtp_buffer_size,
        480
    );
    assert_eq!(
        Config::default()
            .media_session_controller_config
            .rtp_buffer_initial_count,
        32
    );
    assert_eq!(
        Config::default()
            .media_session_controller_config
            .rtp_buffer_max_count,
        128
    );
    assert_eq!(Config::default().transaction_event_channel_capacity, 10_000);
    assert_eq!(Config::default().sip_transaction_dispatch_workers, None);
    assert_eq!(
        Config::default().sip_transaction_dispatch_queue_capacity,
        None
    );
    assert_eq!(
        Config::default().sip_transaction_dispatch_priority_burst_max,
        None
    );
    assert_eq!(
        Config::default().sip_invite_2xx_retransmit_max_due_per_tick,
        None
    );
    assert_eq!(Config::default().sip_dialog_dispatch_workers, None);
    assert_eq!(Config::default().sip_dialog_dispatch_queue_capacity, None);
    assert_eq!(
        Config::default().global_event_channel_capacity,
        Config::DEFAULT_APP_EVENT_CHANNEL_CAPACITY
    );
    assert!(Config::default().session_event_dispatcher_workers >= 1);
    assert_eq!(
        Config::default().session_event_dispatcher_channel_capacity,
        Config::DEFAULT_APP_EVENT_CHANNEL_CAPACITY
    );
    assert_eq!(Config::default().server_call_capacity, None);
    assert_eq!(Config::default().server_call_admission_limit, None);
    assert_eq!(Config::default().server_call_admission_soft_limit, None);
    assert_eq!(
        Config::default().server_call_admission_pacing_delay_ms,
        None
    );
    assert_eq!(Config::default().server_overload_retry_after_secs, Some(1));
    assert!(!Config::default().sip_udp_diagnostics);
    assert!(!Config::default().sip_transaction_timing_diagnostics);
    assert!(!Config::default().sip_dialog_timing_diagnostics);
    assert!(!Config::default().media_setup_diagnostics);
    assert!(!Config::default().cleanup_diagnostics);
    assert!(!Config::default().cleanup_diagnostic_events);
    assert!(!Config::default().srtp_diagnostics);
    assert!(!Config::default().rtp_diagnostics);
    assert!(!Config::default().media_sdp_diagnostics);
    assert!(Config::default().auto_100_trying);
    assert!(!Config::default().fast_auto_accept_incoming_calls);
}

#[test]
fn incoming_call_channel_capacity_is_configurable() {
    let config = Config::local("capacity-test", 5060)
        .with_incoming_call_channel_capacity(4096)
        .with_state_event_channel_capacity(2048)
        .with_sip_transport_channel_capacity(8192)
        .with_sip_transport_dispatch_workers(4)
        .with_sip_transport_dispatch_queue_capacity(65_536)
        .with_sip_udp_socket_buffers(Some(1_048_576), Some(524_288))
        .with_sip_udp_parse_workers(4)
        .with_sip_udp_parse_queue_capacity(32_768)
        .with_sip_udp_parse_dispatch(rvoip_sip_transport::UdpParseDispatch::RoundRobin)
        .with_transaction_event_channel_capacity(12_288)
        .with_sip_transaction_dispatch_workers(4)
        .with_sip_transaction_dispatch_queue_capacity(65_536)
        .with_sip_transaction_dispatch_priority_burst_max(32)
        .with_sip_invite_2xx_retransmit_max_due_per_tick(512)
        .with_sip_transaction_timing_diagnostics(true)
        .with_sip_dialog_dispatch_workers(4)
        .with_sip_dialog_dispatch_queue_capacity(65_536)
        .with_sip_dialog_timing_diagnostics(true)
        .with_global_event_channel_capacity(20_000)
        .with_session_event_dispatcher_workers(4)
        .with_session_event_dispatcher_channel_capacity(16_384)
        .with_server_call_admission_limit(8_192)
        .with_server_call_admission_soft_limit(7_500)
        .with_server_call_admission_pacing_delay_ms(2)
        .with_server_overload_retry_after_secs(2);

    assert_eq!(config.incoming_call_channel_capacity, 4096);
    assert_eq!(config.state_event_channel_capacity, 2048);
    assert_eq!(config.sip_transport_channel_capacity, 8192);
    assert_eq!(config.sip_transport_dispatch_workers, Some(4));
    assert_eq!(config.sip_transport_dispatch_queue_capacity, Some(65_536));
    assert_eq!(config.sip_udp_recv_buffer_size, Some(1_048_576));
    assert_eq!(config.sip_udp_send_buffer_size, Some(524_288));
    assert_eq!(config.sip_udp_parse_workers, Some(4));
    assert_eq!(config.sip_udp_parse_queue_capacity, Some(32_768));
    assert_eq!(
        config.sip_udp_parse_dispatch,
        Some(rvoip_sip_transport::UdpParseDispatch::RoundRobin)
    );
    assert_eq!(config.transaction_event_channel_capacity, 12_288);
    assert_eq!(config.sip_transaction_dispatch_workers, Some(4));
    assert_eq!(config.sip_transaction_dispatch_queue_capacity, Some(65_536));
    assert_eq!(config.sip_transaction_dispatch_priority_burst_max, Some(32));
    assert_eq!(config.sip_invite_2xx_retransmit_max_due_per_tick, Some(512));
    assert!(config.sip_transaction_timing_diagnostics);
    assert_eq!(config.sip_dialog_dispatch_workers, Some(4));
    assert_eq!(config.sip_dialog_dispatch_queue_capacity, Some(65_536));
    assert!(config.sip_dialog_timing_diagnostics);
    assert_eq!(config.global_event_channel_capacity, 20_000);
    assert_eq!(config.session_event_dispatcher_workers, 4);
    assert_eq!(config.session_event_dispatcher_channel_capacity, 16_384);
    assert_eq!(config.server_call_admission_limit, Some(8_192));
    assert_eq!(config.server_call_admission_soft_limit, Some(7_500));
    assert_eq!(config.server_call_admission_pacing_delay_ms, Some(2));
    assert_eq!(config.server_overload_retry_after_secs, Some(2));
}

#[test]
fn app_event_channel_capacity_sets_app_facing_queues() {
    let config = Config::local("capacity-test", 5060).with_app_event_channel_capacity(512);

    assert_eq!(config.global_event_channel_capacity, 512);
    assert_eq!(config.session_event_dispatcher_channel_capacity, 512);
    assert_eq!(config.sip_transport_channel_capacity, 10_000);
    assert_eq!(config.transaction_event_channel_capacity, 10_000);
}

#[test]
fn channel_capacity_sets_related_signaling_queues() {
    let config = Config::local("capacity-test", 5060).with_channel_capacity(512);

    assert_eq!(config.incoming_call_channel_capacity, 512);
    assert_eq!(config.state_event_channel_capacity, 512);
    assert_eq!(config.sip_transport_channel_capacity, 5120);
    assert_eq!(config.transaction_event_channel_capacity, 5120);
    assert_eq!(config.global_event_channel_capacity, 5120);
    assert_eq!(config.session_event_dispatcher_channel_capacity, 5120);
    assert_eq!(config.server_call_capacity, None);
    assert_eq!(config.server_call_admission_limit, None);
    assert_eq!(config.server_call_admission_soft_limit, None);
}

#[test]
fn high_cps_udp_auto_answer_profile_sets_fast_config_without_server_capacity() {
    let config = Config::local("capacity-test", 5060).with_high_cps_udp_auto_answer(20_000);

    assert!(!config.auto_180_ringing);
    assert!(!config.auto_100_trying);
    assert!(!config.fast_auto_accept_incoming_calls);
    assert_eq!(config.incoming_call_channel_capacity, 20_000);
    assert_eq!(config.state_event_channel_capacity, 20_000);
    assert_eq!(config.sip_transport_channel_capacity, 200_000);
    assert_eq!(config.transaction_event_channel_capacity, 200_000);
    assert_eq!(config.global_event_channel_capacity, 200_000);
    assert_eq!(config.session_event_dispatcher_channel_capacity, 200_000);
    assert_eq!(config.sip_transport_dispatch_workers, None);
    assert_eq!(config.sip_transport_dispatch_queue_capacity, None);
    assert_eq!(config.sip_udp_parse_workers, Some(1));
    assert_eq!(config.sip_udp_parse_queue_capacity, Some(20_000));
    assert_eq!(config.sip_transaction_dispatch_workers, None);
    assert_eq!(config.sip_transaction_dispatch_queue_capacity, None);
    assert_eq!(config.sip_dialog_dispatch_workers, None);
    assert_eq!(config.sip_dialog_dispatch_queue_capacity, None);
    assert_eq!(config.server_call_capacity, None);
}

#[test]
fn server_capacity_sets_hot_index_capacity_only() {
    let config = Config::local("capacity-test", 5060).with_server_capacity(2048);

    assert_eq!(config.server_call_capacity, Some(2048));
    assert_eq!(config.incoming_call_channel_capacity, 1000);
    assert_eq!(config.state_event_channel_capacity, 1000);
    assert_eq!(config.sip_transport_channel_capacity, 10_000);
    assert_eq!(config.transaction_event_channel_capacity, 10_000);
    assert_eq!(
        config.global_event_channel_capacity,
        Config::DEFAULT_APP_EVENT_CHANNEL_CAPACITY
    );
    assert_eq!(
        config.session_event_dispatcher_channel_capacity,
        Config::DEFAULT_APP_EVENT_CHANNEL_CAPACITY
    );
}

#[test]
fn server_capacity_composes_with_channel_capacity() {
    let config = Config::local("capacity-test", 5060)
        .with_channel_capacity(4096)
        .with_server_capacity(1024);

    assert_eq!(config.server_call_capacity, Some(1024));
    assert_eq!(config.incoming_call_channel_capacity, 4096);
    assert_eq!(config.state_event_channel_capacity, 4096);
    assert_eq!(config.sip_transport_channel_capacity, 40_960);
    assert_eq!(config.transaction_event_channel_capacity, 40_960);
    assert_eq!(config.global_event_channel_capacity, 40_960);
    assert_eq!(config.session_event_dispatcher_channel_capacity, 40_960);
}

#[test]
fn media_session_capacity_is_independent_from_server_capacity() {
    let config = Config::local("capacity-test", 5060).with_media_session_capacity(4096);

    assert_eq!(config.media_session_capacity, Some(4096));
    assert_eq!(config.server_call_capacity, None);
    assert_eq!(config.transaction_event_channel_capacity, 10_000);
}

#[test]
fn rtp_media_buffer_tuning_is_configurable() {
    let session_buffers = RtpSessionBufferConfig {
        sender_channel_capacity: 8,
        receiver_channel_capacity: 4,
        event_channel_capacity: 16,
    };
    let transport_buffers = RtpTransportBufferConfig {
        event_channel_capacity: 12,
        recv_buffer_size: 2048,
        rtcp_recv_buffer_size: 1024,
    };
    let mut media_config = MediaSessionControllerConfig::default();
    media_config.audio_frame_pool.initial_size = 8;
    media_config.audio_frame_pool.max_size = 32;
    media_config.rtp_buffer_size = 960;
    media_config.rtp_buffer_initial_count = 8;
    media_config.rtp_buffer_max_count = 32;

    let config = Config::local("capacity-test", 5060)
        .with_media_session_controller_config(media_config)
        .with_rtp_session_buffer_config(session_buffers)
        .with_rtp_transport_buffer_config(transport_buffers);

    assert_eq!(config.rtp_session_buffer_config, session_buffers);
    assert_eq!(config.rtp_transport_buffer_config, transport_buffers);
    assert_eq!(
        config
            .media_session_controller_config
            .audio_frame_pool
            .initial_size,
        8
    );
    assert_eq!(
        config
            .media_session_controller_config
            .audio_frame_pool
            .max_size,
        32
    );
    assert_eq!(config.media_session_controller_config.rtp_buffer_size, 960);
    assert_eq!(
        config
            .media_session_controller_config
            .rtp_buffer_initial_count,
        8
    );
    assert_eq!(
        config.media_session_controller_config.rtp_buffer_max_count,
        32
    );
    config.validate().expect("valid RTP/media buffer tuning");
}

#[test]
fn media_port_capacity_sets_requested_range() {
    let config = Config::local("capacity-test", 5060).with_media_port_capacity(16_384, 49_152);

    assert_eq!(config.media_port_start, 16_384);
    assert_eq!(config.media_port_end, 65_535);
    assert_eq!(config.media_port_capacity, Some(49_152));
    config.validate().expect("valid RTP media port capacity");
}

#[test]
fn media_port_capacity_overflow_is_rejected() {
    let config = Config::local("capacity-test", 5060).with_media_port_capacity(60_000, 20_000);

    let err = config
        .validate()
        .expect_err("overflowing RTP media port capacity must fail");
    assert!(
        config_error_detail(&err).contains("below requested media_port_capacity"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn zero_incoming_call_channel_capacity_is_rejected() {
    let mut config = Config::local("capacity-test", 5060);
    config.incoming_call_channel_capacity = 0;

    let err = config.validate().expect_err("zero capacity must fail");
    assert!(
        config_error_detail(&err).contains("incoming_call_channel_capacity must be at least 1"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn zero_media_session_capacity_is_rejected_when_set() {
    let mut config = Config::local("capacity-test", 5060);
    config.media_session_capacity = Some(0);

    let err = config.validate().expect_err("zero capacity must fail");
    assert!(
        config_error_detail(&err).contains("media_session_capacity must be at least 1"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn zero_media_port_capacity_is_rejected_when_set() {
    let mut config = Config::local("capacity-test", 5060);
    config.media_port_capacity = Some(0);

    let err = config.validate().expect_err("zero capacity must fail");
    assert!(
        config_error_detail(&err).contains("media_port_capacity must be at least 1"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn invalid_rtp_session_buffer_config_is_rejected() {
    let mut config = Config::local("capacity-test", 5060);
    config.rtp_session_buffer_config.sender_channel_capacity = 0;

    let err = config
        .validate()
        .expect_err("zero RTP sender capacity must fail");
    assert!(
        config_error_detail(&err).contains("rtp_session_buffer_config.sender_channel_capacity"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn invalid_rtp_transport_buffer_config_is_rejected() {
    let mut config = Config::local("capacity-test", 5060);
    config.rtp_transport_buffer_config.rtcp_recv_buffer_size = 0;

    let err = config
        .validate()
        .expect_err("zero RTCP receive buffer must fail");
    assert!(
        config_error_detail(&err).contains("rtp_transport_buffer_config.rtcp_recv_buffer_size"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn invalid_media_session_controller_buffer_config_is_rejected() {
    let mut config = Config::local("capacity-test", 5060);
    config.media_session_controller_config.rtp_buffer_size = 0;

    let err = config
        .validate()
        .expect_err("zero media RTP buffer size must fail");
    assert!(
        config_error_detail(&err).contains("media_session_controller_config.rtp_buffer_size"),
        "unexpected validation error: {err}"
    );

    let mut config = Config::local("capacity-test", 5060);
    config
        .media_session_controller_config
        .audio_frame_pool
        .samples_per_frame = 0;

    let err = config
        .validate()
        .expect_err("zero audio frame size must fail");
    assert!(
        config_error_detail(&err)
            .contains("media_session_controller_config.audio_frame_pool.samples_per_frame"),
        "unexpected validation error: {err}"
    );

    let mut config = Config::local("capacity-test", 5060);
    config
        .media_session_controller_config
        .rtp_buffer_initial_count = 9;
    config.media_session_controller_config.rtp_buffer_max_count = 8;

    let err = config
        .validate()
        .expect_err("initial media RTP pool count above max must fail");
    assert!(
        config_error_detail(&err)
            .contains("rtp_buffer_initial_count must be <= rtp_buffer_max_count"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn zero_state_event_channel_capacity_is_rejected() {
    let mut config = Config::local("capacity-test", 5060);
    config.state_event_channel_capacity = 0;

    let err = config.validate().expect_err("zero capacity must fail");
    assert!(
        config_error_detail(&err).contains("state_event_channel_capacity must be at least 1"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn zero_sip_transport_channel_capacity_is_rejected() {
    let mut config = Config::local("capacity-test", 5060);
    config.sip_transport_channel_capacity = 0;

    let err = config.validate().expect_err("zero capacity must fail");
    assert!(
        config_error_detail(&err).contains("sip_transport_channel_capacity must be at least 1"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn zero_sip_transport_dispatch_config_is_rejected_when_set() {
    let mut config = Config::local("capacity-test", 5060);
    config.sip_transport_dispatch_workers = Some(0);

    let err = config.validate().expect_err("zero workers must fail");
    assert!(
        config_error_detail(&err)
            .contains("sip_transport_dispatch_workers must be at least 1 when set"),
        "unexpected validation error: {err}"
    );

    let mut config = Config::local("capacity-test", 5060);
    config.sip_transport_dispatch_queue_capacity = Some(0);

    let err = config.validate().expect_err("zero capacity must fail");
    assert!(
        config_error_detail(&err)
            .contains("sip_transport_dispatch_queue_capacity must be at least 1 when set"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn zero_transaction_event_channel_capacity_is_rejected() {
    let mut config = Config::local("capacity-test", 5060);
    config.transaction_event_channel_capacity = 0;

    let err = config.validate().expect_err("zero capacity must fail");
    assert!(
        config_error_detail(&err).contains("transaction_event_channel_capacity must be at least 1"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn zero_transaction_dispatch_config_is_rejected_when_set() {
    let mut config = Config::local("capacity-test", 5060);
    config.sip_transaction_dispatch_workers = Some(0);

    let err = config.validate().expect_err("zero workers must fail");
    assert!(
        config_error_detail(&err)
            .contains("sip_transaction_dispatch_workers must be at least 1 when set"),
        "unexpected validation error: {err}"
    );

    let mut config = Config::local("capacity-test", 5060);
    config.sip_transaction_dispatch_queue_capacity = Some(0);

    let err = config.validate().expect_err("zero capacity must fail");
    assert!(
        config_error_detail(&err)
            .contains("sip_transaction_dispatch_queue_capacity must be at least 1 when set"),
        "unexpected validation error: {err}"
    );

    let mut config = Config::local("capacity-test", 5060);
    config.sip_transaction_dispatch_priority_burst_max = Some(0);

    let err = config.validate().expect_err("zero burst max must fail");
    assert!(
        config_error_detail(&err)
            .contains("sip_transaction_dispatch_priority_burst_max must be at least 1 when set"),
        "unexpected validation error: {err}"
    );

    let mut config = Config::local("capacity-test", 5060);
    config.sip_invite_2xx_retransmit_max_due_per_tick = Some(0);

    let err = config
        .validate()
        .expect_err("zero retransmit budget must fail");
    assert!(
        config_error_detail(&err)
            .contains("sip_invite_2xx_retransmit_max_due_per_tick must be at least 1 when set"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn zero_dialog_dispatch_config_is_rejected_when_set() {
    let mut config = Config::local("capacity-test", 5060);
    config.sip_dialog_dispatch_workers = Some(0);

    let err = config.validate().expect_err("zero workers must fail");
    assert!(
        config_error_detail(&err)
            .contains("sip_dialog_dispatch_workers must be at least 1 when set"),
        "unexpected validation error: {err}"
    );

    let mut config = Config::local("capacity-test", 5060);
    config.sip_dialog_dispatch_queue_capacity = Some(0);

    let err = config.validate().expect_err("zero capacity must fail");
    assert!(
        config_error_detail(&err)
            .contains("sip_dialog_dispatch_queue_capacity must be at least 1 when set"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn zero_global_event_channel_capacity_is_rejected() {
    let mut config = Config::local("capacity-test", 5060);
    config.global_event_channel_capacity = 0;

    let err = config.validate().expect_err("zero capacity must fail");
    assert!(
        config_error_detail(&err).contains("global_event_channel_capacity must be at least 1"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn zero_sip_udp_socket_buffers_are_rejected_when_set() {
    let mut config = Config::local("capacity-test", 5060);
    config.sip_udp_recv_buffer_size = Some(0);

    let err = config
        .validate()
        .expect_err("zero receive buffer must fail");
    assert!(
        config_error_detail(&err).contains("sip_udp_recv_buffer_size must be at least 1 when set"),
        "unexpected validation error: {err}"
    );

    let mut config = Config::local("capacity-test", 5060);
    config.sip_udp_send_buffer_size = Some(0);

    let err = config.validate().expect_err("zero send buffer must fail");
    assert!(
        config_error_detail(&err).contains("sip_udp_send_buffer_size must be at least 1 when set"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn zero_session_event_dispatcher_workers_is_rejected() {
    let mut config = Config::local("capacity-test", 5060);
    config.session_event_dispatcher_workers = 0;

    let err = config.validate().expect_err("zero capacity must fail");
    assert!(
        config_error_detail(&err).contains("session_event_dispatcher_workers must be at least 1"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn zero_session_event_dispatcher_channel_capacity_is_rejected() {
    let mut config = Config::local("capacity-test", 5060);
    config.session_event_dispatcher_channel_capacity = 0;

    let err = config.validate().expect_err("zero capacity must fail");
    assert!(
        config_error_detail(&err)
            .contains("session_event_dispatcher_channel_capacity must be at least 1"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn zero_server_call_capacity_is_rejected_when_set() {
    let mut config = Config::local("capacity-test", 5060);
    config.server_call_capacity = Some(0);

    let err = config.validate().expect_err("zero capacity must fail");
    assert!(
        config_error_detail(&err).contains("server_call_capacity must be at least 1 when set"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn zero_server_call_admission_limit_is_rejected_when_set() {
    let mut config = Config::local("capacity-test", 5060);
    config.server_call_admission_limit = Some(0);

    let err = config.validate().expect_err("zero limit must fail");
    assert!(
        config_error_detail(&err)
            .contains("server_call_admission_limit must be at least 1 when set"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn zero_server_overload_retry_after_is_rejected_when_set() {
    let mut config = Config::local("capacity-test", 5060);
    config.server_overload_retry_after_secs = Some(0);

    let err = config.validate().expect_err("zero retry-after must fail");
    assert!(
        config_error_detail(&err)
            .contains("server_overload_retry_after_secs must be at least 1 when set"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn invalid_server_call_admission_soft_limit_is_rejected() {
    let mut config = Config::local("capacity-test", 5060);
    config.server_call_admission_limit = Some(10);
    config.server_call_admission_soft_limit = Some(11);

    let err = config
        .validate()
        .expect_err("soft limit above hard must fail");
    assert!(
        config_error_detail(&err).contains("server_call_admission_soft_limit"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn zero_server_call_admission_pacing_delay_is_rejected_when_set() {
    let mut config = Config::local("capacity-test", 5060);
    config.server_call_admission_pacing_delay_ms = Some(0);

    let err = config.validate().expect_err("zero pacing delay must fail");
    assert!(
        config_error_detail(&err)
            .contains("server_call_admission_pacing_delay_ms must be at least 1 when set"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn unsupported_beta_media_codec_advertisement_is_rejected() {
    let mut config = Config::local("codec-test", 5060);
    config.offered_codecs = vec![0, 8, 111, 101];

    let err = config
        .validate()
        .expect_err("unsupported Opus advertisement must fail beta validation");
    assert!(
        config_error_detail(&err).contains("payload type 111 is not beta-supported for full media"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn comfort_noise_payload_requires_comfort_noise_flag() {
    let mut config = Config::local("codec-test", 5060);
    config.offered_codecs = vec![0, 8, 13, 101];

    let err = config
        .validate()
        .expect_err("CN without comfort_noise_enabled must fail");
    assert!(
        config_error_detail(&err).contains("payload type 13 requires comfort_noise_enabled=true"),
        "unexpected validation error: {err}"
    );

    config.comfort_noise_enabled = true;
    config.validate().expect("CN should pass when enabled");
}

#[test]
fn beta_media_codec_set_requires_real_audio_codec() {
    let mut config = Config::local("codec-test", 5060);
    config.offered_codecs = vec![101];

    let err = config
        .validate()
        .expect_err("DTMF-only codec set must fail");
    assert!(
        config_error_detail(&err).contains("offered_codecs must include PCMU (0) or PCMA (8)"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn duplicate_payload_types_are_rejected() {
    let mut config = Config::local("codec-test", 5060);
    config.offered_codecs = vec![0, 8, 8, 101];

    let err = config
        .validate()
        .expect_err("duplicate codec payload types must fail");
    assert!(
        config_error_detail(&err).contains("offered_codecs contains duplicate payload type 8"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn mandatory_srtp_requires_srtp_offer() {
    let mut config = Config::local("srtp-test", 5060);
    config.srtp_required = true;
    config.offer_srtp = false;

    let err = config
        .validate()
        .expect_err("mandatory SRTP without offer_srtp must fail");
    assert!(
        config_error_detail(&err).contains("srtp_required=true requires offer_srtp=true"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn srtp_offer_requires_at_least_one_suite() {
    let mut config = Config::local("srtp-test", 5060);
    config.offer_srtp = true;
    config.srtp_offered_suites.clear();

    let err = config
        .validate()
        .expect_err("SRTP offer without suites must fail");
    assert!(
        config_error_detail(&err)
            .contains("offer_srtp=true requires at least one srtp_offered_suites entry"),
        "unexpected validation error: {err}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn diagnostic_flags_are_independent_at_runtime() {
    let sip_only =
        UnifiedCoordinator::new(Config::local("diag-sip", 0).with_sip_udp_diagnostics(true))
            .await
            .expect("sip diagnostics coordinator");
    assert!(rvoip_sip_transport::diagnostics::enabled());
    assert!(rvoip_sip_dialog::diagnostics::enabled());
    assert!(!rvoip_sip_dialog::diagnostics::transaction_timing_enabled());
    assert!(!rvoip_sip_dialog::diagnostics::dialog_timing_enabled());
    assert!(!rvoip_media_core::diagnostics::enabled());
    assert!(!cleanup_diag::enabled());
    sip_only
        .shutdown_gracefully(Some(Duration::ZERO))
        .await
        .expect("shutdown sip diagnostics coordinator");

    let transaction_timing = UnifiedCoordinator::new(
        Config::local("diag-tx-timing", 0)
            .with_sip_udp_diagnostics(true)
            .with_sip_transaction_timing_diagnostics(true),
    )
    .await
    .expect("transaction timing diagnostics coordinator");
    assert!(rvoip_sip_dialog::diagnostics::enabled());
    assert!(rvoip_sip_dialog::diagnostics::transaction_timing_enabled());
    assert!(!rvoip_sip_dialog::diagnostics::dialog_timing_enabled());
    transaction_timing
        .shutdown_gracefully(Some(Duration::ZERO))
        .await
        .expect("shutdown transaction timing diagnostics coordinator");

    let dialog_timing = UnifiedCoordinator::new(
        Config::local("diag-dialog-timing", 0)
            .with_sip_udp_diagnostics(true)
            .with_sip_dialog_timing_diagnostics(true),
    )
    .await
    .expect("dialog timing diagnostics coordinator");
    assert!(rvoip_sip_dialog::diagnostics::enabled());
    assert!(!rvoip_sip_dialog::diagnostics::transaction_timing_enabled());
    assert!(rvoip_sip_dialog::diagnostics::dialog_timing_enabled());
    dialog_timing
        .shutdown_gracefully(Some(Duration::ZERO))
        .await
        .expect("shutdown dialog timing diagnostics coordinator");

    let media_only =
        UnifiedCoordinator::new(Config::local("diag-media", 0).with_media_setup_diagnostics(true))
            .await
            .expect("media diagnostics coordinator");
    assert!(!rvoip_sip_transport::diagnostics::enabled());
    assert!(!rvoip_sip_dialog::diagnostics::enabled());
    assert!(!rvoip_sip_dialog::diagnostics::transaction_timing_enabled());
    if cfg!(feature = "perf-media-diagnostics") {
        assert!(rvoip_media_core::diagnostics::enabled());
    } else {
        assert!(!rvoip_media_core::diagnostics::enabled());
    }
    assert!(!cleanup_diag::enabled());
    media_only
        .shutdown_gracefully(Some(Duration::ZERO))
        .await
        .expect("shutdown media diagnostics coordinator");

    let cleanup_only =
        UnifiedCoordinator::new(Config::local("diag-cleanup", 0).with_cleanup_diagnostics(true))
            .await
            .expect("cleanup diagnostics coordinator");
    assert!(!rvoip_sip_transport::diagnostics::enabled());
    assert!(!rvoip_sip_dialog::diagnostics::enabled());
    assert!(!rvoip_sip_dialog::diagnostics::transaction_timing_enabled());
    assert!(!rvoip_media_core::diagnostics::enabled());
    assert!(cleanup_diag::enabled());
    cleanup_only
        .shutdown_gracefully(Some(Duration::ZERO))
        .await
        .expect("shutdown cleanup diagnostics coordinator");

    let defaults = UnifiedCoordinator::new(Config::local("diag-default", 0))
        .await
        .expect("default diagnostics coordinator");
    assert!(!rvoip_sip_transport::diagnostics::enabled());
    assert!(!rvoip_sip_dialog::diagnostics::enabled());
    assert!(!rvoip_sip_dialog::diagnostics::transaction_timing_enabled());
    assert!(!rvoip_media_core::diagnostics::enabled());
    assert!(!cleanup_diag::enabled());
    defaults
        .shutdown_gracefully(Some(Duration::ZERO))
        .await
        .expect("shutdown default diagnostics coordinator");
}
