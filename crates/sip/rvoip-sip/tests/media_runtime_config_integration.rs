//! `Config::media_runtime` lays out the process-wide media executor.
//!
//! The executor is global and configured once, so this binary installs one
//! layout and checks every coordinator against it.

use std::time::Duration;

use rvoip_sip::{Config, MediaLanes, MediaRuntimeConfig, SessionError, UnifiedCoordinator};

fn two_lanes() -> MediaRuntimeConfig {
    MediaRuntimeConfig::new(MediaLanes::Shared {
        worker_threads: Some(4),
        lanes: 2,
    })
}

fn config_error_detail(error: &SessionError) -> &str {
    let SessionError::ConfigError(detail) = error else {
        panic!("expected typed ConfigError, got {error:?}");
    };
    detail
}

#[test]
fn media_runtime_defaults_to_the_built_in_single_lane() {
    assert_eq!(Config::default().media_runtime, None);
    assert_eq!(Config::local("lanes", 5060).media_runtime, None);
    assert_eq!(
        Config::local("lanes", 5060)
            .with_media_runtime(two_lanes())
            .media_runtime,
        Some(two_lanes())
    );
}

#[test]
fn invalid_media_runtime_layout_is_rejected_by_validation() {
    let config = Config::local("lanes", 5060).with_media_runtime(MediaRuntimeConfig::new(
        MediaLanes::Shared {
            worker_threads: Some(2),
            lanes: 4,
        },
    ));
    let error = config.validate().expect_err("more lanes than workers");
    assert!(
        config_error_detail(&error).contains("media_runtime"),
        "unexpected validation error: {error}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn coordinator_installs_the_layout_and_rejects_a_conflicting_one() {
    let first =
        UnifiedCoordinator::new(Config::local("lanes-first", 0).with_media_runtime(two_lanes()))
            .await
            .expect("first coordinator installs the layout");

    // Another coordinator with the same layout, or with none, shares it.
    let same =
        UnifiedCoordinator::new(Config::local("lanes-same", 0).with_media_runtime(two_lanes()))
            .await
            .expect("an equal layout is accepted");
    let unset = UnifiedCoordinator::new(Config::local("lanes-unset", 0))
        .await
        .expect("a coordinator without a layout uses the installed one");

    let error = match UnifiedCoordinator::new(
        Config::local("lanes-conflict", 0).with_media_runtime(MediaRuntimeConfig::default()),
    )
    .await
    {
        Ok(_) => panic!("a conflicting layout must not start"),
        Err(error) => error,
    };
    assert!(
        config_error_detail(&error).contains("already configured"),
        "unexpected error: {error}"
    );

    // The installed layout is what the executor runs once media starts.
    let handle = rvoip_rtp_core::task_runtime::spawn_media_task(async {});
    handle.await.expect("media task");
    let snapshot = rvoip_rtp_core::task_runtime::media_runtime_snapshot()
        .expect("a media task started the executor");
    assert_eq!(snapshot.config, two_lanes());
    assert_eq!(snapshot.lanes.len(), 2);

    for coordinator in [first, same, unset] {
        coordinator
            .shutdown_gracefully(Some(Duration::ZERO))
            .await
            .expect("shutdown");
    }
}
