use std::time::Duration;

use rvoip_sip::{cleanup_diag, Config, UnifiedCoordinator};
use serial_test::serial;

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
    assert_eq!(Config::default().transaction_event_channel_capacity, 10_000);
    assert_eq!(Config::default().sip_transaction_dispatch_workers, None);
    assert_eq!(
        Config::default().sip_transaction_dispatch_queue_capacity,
        None
    );
    assert_eq!(Config::default().sip_dialog_dispatch_workers, None);
    assert_eq!(Config::default().sip_dialog_dispatch_queue_capacity, None);
    assert_eq!(Config::default().global_event_channel_capacity, 10_000);
    assert!(Config::default().session_event_dispatcher_workers >= 1);
    assert_eq!(
        Config::default().session_event_dispatcher_channel_capacity,
        10_000
    );
    assert_eq!(Config::default().server_call_capacity, None);
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
        .with_sip_transaction_timing_diagnostics(true)
        .with_sip_dialog_dispatch_workers(4)
        .with_sip_dialog_dispatch_queue_capacity(65_536)
        .with_sip_dialog_timing_diagnostics(true)
        .with_global_event_channel_capacity(20_000)
        .with_session_event_dispatcher_workers(4)
        .with_session_event_dispatcher_channel_capacity(16_384);

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
    assert!(config.sip_transaction_timing_diagnostics);
    assert_eq!(config.sip_dialog_dispatch_workers, Some(4));
    assert_eq!(config.sip_dialog_dispatch_queue_capacity, Some(65_536));
    assert!(config.sip_dialog_timing_diagnostics);
    assert_eq!(config.global_event_channel_capacity, 20_000);
    assert_eq!(config.session_event_dispatcher_workers, 4);
    assert_eq!(config.session_event_dispatcher_channel_capacity, 16_384);
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
    assert_eq!(config.global_event_channel_capacity, 10_000);
    assert_eq!(config.session_event_dispatcher_channel_capacity, 10_000);
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
        err.to_string()
            .contains("below requested media_port_capacity"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn zero_incoming_call_channel_capacity_is_rejected() {
    let mut config = Config::local("capacity-test", 5060);
    config.incoming_call_channel_capacity = 0;

    let err = config.validate().expect_err("zero capacity must fail");
    assert!(
        err.to_string()
            .contains("incoming_call_channel_capacity must be at least 1"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn zero_media_session_capacity_is_rejected_when_set() {
    let mut config = Config::local("capacity-test", 5060);
    config.media_session_capacity = Some(0);

    let err = config.validate().expect_err("zero capacity must fail");
    assert!(
        err.to_string()
            .contains("media_session_capacity must be at least 1"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn zero_media_port_capacity_is_rejected_when_set() {
    let mut config = Config::local("capacity-test", 5060);
    config.media_port_capacity = Some(0);

    let err = config.validate().expect_err("zero capacity must fail");
    assert!(
        err.to_string()
            .contains("media_port_capacity must be at least 1"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn zero_state_event_channel_capacity_is_rejected() {
    let mut config = Config::local("capacity-test", 5060);
    config.state_event_channel_capacity = 0;

    let err = config.validate().expect_err("zero capacity must fail");
    assert!(
        err.to_string()
            .contains("state_event_channel_capacity must be at least 1"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn zero_sip_transport_channel_capacity_is_rejected() {
    let mut config = Config::local("capacity-test", 5060);
    config.sip_transport_channel_capacity = 0;

    let err = config.validate().expect_err("zero capacity must fail");
    assert!(
        err.to_string()
            .contains("sip_transport_channel_capacity must be at least 1"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn zero_sip_transport_dispatch_config_is_rejected_when_set() {
    let mut config = Config::local("capacity-test", 5060);
    config.sip_transport_dispatch_workers = Some(0);

    let err = config.validate().expect_err("zero workers must fail");
    assert!(
        err.to_string()
            .contains("sip_transport_dispatch_workers must be at least 1 when set"),
        "unexpected validation error: {err}"
    );

    let mut config = Config::local("capacity-test", 5060);
    config.sip_transport_dispatch_queue_capacity = Some(0);

    let err = config.validate().expect_err("zero capacity must fail");
    assert!(
        err.to_string()
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
        err.to_string()
            .contains("transaction_event_channel_capacity must be at least 1"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn zero_transaction_dispatch_config_is_rejected_when_set() {
    let mut config = Config::local("capacity-test", 5060);
    config.sip_transaction_dispatch_workers = Some(0);

    let err = config.validate().expect_err("zero workers must fail");
    assert!(
        err.to_string()
            .contains("sip_transaction_dispatch_workers must be at least 1 when set"),
        "unexpected validation error: {err}"
    );

    let mut config = Config::local("capacity-test", 5060);
    config.sip_transaction_dispatch_queue_capacity = Some(0);

    let err = config.validate().expect_err("zero capacity must fail");
    assert!(
        err.to_string()
            .contains("sip_transaction_dispatch_queue_capacity must be at least 1 when set"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn zero_dialog_dispatch_config_is_rejected_when_set() {
    let mut config = Config::local("capacity-test", 5060);
    config.sip_dialog_dispatch_workers = Some(0);

    let err = config.validate().expect_err("zero workers must fail");
    assert!(
        err.to_string()
            .contains("sip_dialog_dispatch_workers must be at least 1 when set"),
        "unexpected validation error: {err}"
    );

    let mut config = Config::local("capacity-test", 5060);
    config.sip_dialog_dispatch_queue_capacity = Some(0);

    let err = config.validate().expect_err("zero capacity must fail");
    assert!(
        err.to_string()
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
        err.to_string()
            .contains("global_event_channel_capacity must be at least 1"),
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
        err.to_string()
            .contains("sip_udp_recv_buffer_size must be at least 1 when set"),
        "unexpected validation error: {err}"
    );

    let mut config = Config::local("capacity-test", 5060);
    config.sip_udp_send_buffer_size = Some(0);

    let err = config.validate().expect_err("zero send buffer must fail");
    assert!(
        err.to_string()
            .contains("sip_udp_send_buffer_size must be at least 1 when set"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn zero_session_event_dispatcher_workers_is_rejected() {
    let mut config = Config::local("capacity-test", 5060);
    config.session_event_dispatcher_workers = 0;

    let err = config.validate().expect_err("zero capacity must fail");
    assert!(
        err.to_string()
            .contains("session_event_dispatcher_workers must be at least 1"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn zero_session_event_dispatcher_channel_capacity_is_rejected() {
    let mut config = Config::local("capacity-test", 5060);
    config.session_event_dispatcher_channel_capacity = 0;

    let err = config.validate().expect_err("zero capacity must fail");
    assert!(
        err.to_string()
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
        err.to_string()
            .contains("server_call_capacity must be at least 1 when set"),
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
    assert!(rvoip_media_core::diagnostics::enabled());
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
