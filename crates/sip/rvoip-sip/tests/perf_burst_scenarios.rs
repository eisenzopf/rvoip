#[path = "perf/support/burst.rs"]
mod burst;

use burst::BurstScenarioBook;

#[test]
fn bundled_burst_scenarios_parse_and_validate() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("config")
        .join("perf-burst-scenarios.yaml");
    let book = BurstScenarioBook::from_path(path);

    let names = book
        .scenarios
        .iter()
        .map(|scenario| scenario.name.as_str())
        .collect::<Vec<_>>();
    assert!(names.contains(&"carrier-smoke"));
    assert!(names.contains(&"access-edge-microburst"));
    assert!(names.contains(&"contact-center-flash"));
    assert!(names.contains(&"shift-change-long-hold"));
    assert!(names.contains(&"overload-recovery"));
    assert!(names.contains(&"high-density-media-burst"));
    assert!(names.contains(&"buffer-ab-legacy"));
    assert!(book.scenarios.iter().all(|scenario| {
        scenario.acceptance.min_rss_gate_window_secs >= burst::MIN_RSS_GATE_WINDOW_SECS
    }));

    let smoke = book.scenario("carrier-smoke");
    assert_eq!(smoke.total_offered_calls(), 39);
    assert_eq!(smoke.retained_lifecycle_capacity(), 139);
    assert_eq!(smoke.duration_secs(), 12);
    assert_eq!(smoke.hold_duration(0), smoke.hold_duration(0));

    let overload = book.scenario("overload-recovery");
    assert_eq!(overload.total_offered_calls(), 10_600);
    assert_eq!(overload.retained_lifecycle_capacity(), 10_850);
    assert_eq!(overload.acceptance.min_recovery_asr, Some(0.999));
    assert_eq!(overload.acceptance.max_recovery_secs, Some(5));
    assert!(overload.acceptance.allow_overload_rejections);
    assert!(overload
        .hold_distribution
        .iter()
        .all(|bucket| bucket.max_secs <= 5));

    let legacy = book.scenario("buffer-ab-legacy");
    assert_eq!(legacy.total_offered_calls(), 1400);
    assert_eq!(legacy.retained_lifecycle_capacity(), 2200);
    assert_eq!(legacy.duration_secs(), 80);
    assert_eq!(legacy.hold_duration(42), legacy.hold_duration(42));
}
