use rvoip_sip::Config;

#[test]
fn incoming_call_channel_capacity_defaults_to_1000() {
    assert_eq!(Config::default().incoming_call_channel_capacity, 1000);
    assert_eq!(Config::default().state_event_channel_capacity, 1000);
    assert_eq!(Config::default().sip_transport_channel_capacity, 10_000);
    assert_eq!(Config::default().transaction_event_channel_capacity, 10_000);
    assert!(Config::default().session_event_dispatcher_workers >= 1);
    assert_eq!(
        Config::default().session_event_dispatcher_channel_capacity,
        10_000
    );
}

#[test]
fn incoming_call_channel_capacity_is_configurable() {
    let config = Config::local("capacity-test", 5060)
        .with_incoming_call_channel_capacity(4096)
        .with_state_event_channel_capacity(2048)
        .with_sip_transport_channel_capacity(8192)
        .with_transaction_event_channel_capacity(12_288)
        .with_session_event_dispatcher_workers(4)
        .with_session_event_dispatcher_channel_capacity(16_384);

    assert_eq!(config.incoming_call_channel_capacity, 4096);
    assert_eq!(config.state_event_channel_capacity, 2048);
    assert_eq!(config.sip_transport_channel_capacity, 8192);
    assert_eq!(config.transaction_event_channel_capacity, 12_288);
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
    assert_eq!(config.session_event_dispatcher_channel_capacity, 5120);
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
