mod support;

use support::perf::{profile_active_calls, ACTIVE_CALL_COUNTS};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "performance profile: run with --ignored --nocapture"]
async fn profile_active_calls_100_500_1000() {
    eprintln!("orchestration-core active-call profile; RSS is process-level resident memory");

    for active_calls in ACTIVE_CALL_COUNTS {
        let profile = profile_active_calls(active_calls).await.unwrap();
        eprintln!("{}", profile.report_line());
        assert_eq!(profile.connected_calls, active_calls);
        assert_eq!(profile.queued_calls_remaining, 0);
    }
}
