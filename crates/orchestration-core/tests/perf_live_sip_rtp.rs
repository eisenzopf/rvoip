mod support;

use support::perf::{
    live_sip_rtp_counts_from_env, live_sip_rtp_hold_duration_from_env, profile_live_sip_rtp,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "live SIP/RTP profile: run with --ignored --nocapture; optional env RVOIP_LIVE_SIP_RTP_COUNTS=5,10 RVOIP_LIVE_SIP_RTP_HOLD_SECS=5"]
async fn profile_live_sip_rtp_calls_for_at_least_5_seconds() {
    let counts = live_sip_rtp_counts_from_env();
    let hold_duration = live_sip_rtp_hold_duration_from_env();

    eprintln!(
        "orchestration-core live SIP/RTP profile; counts={counts:?}; hold_secs={:.2}; RSS is process-level resident memory",
        hold_duration.as_secs_f64()
    );

    for active_calls in counts {
        let profile = profile_live_sip_rtp(active_calls, hold_duration)
            .await
            .unwrap();
        eprintln!("{}", profile.report_line());
        assert_eq!(profile.connected_calls, active_calls);
        assert_eq!(profile.active_bridges, active_calls);
        assert!(profile.media_wall_time >= hold_duration);
        assert!(
            profile.caller_received_frames + profile.agent_received_frames > 0,
            "expected real RTP audio frames to be received"
        );
    }
}
