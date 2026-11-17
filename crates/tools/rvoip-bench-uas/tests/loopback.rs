//! The DUT binary on loopback, driven by rvoip's own UAC: 200 OK and BYE 200
//! over UDP and TLS, RTP and SRTP echo of the caller's tone, `max-cps`
//! admission 503s, and the finish-file drain with `ready.json`,
//! `stats.jsonl`, `summary.json` and `/metrics`.

mod support;

use std::time::Duration;

use support::{call, goertzel, stream_tone, uac, Dut, RawOutcome, RawUac, TONE_HZ};

const DRAIN: Duration = Duration::from_secs(30);

/// Place `calls` calls one after another, each answered and hung up.
async fn signaling_calls(dut: &Dut, calls: usize) {
    let mut peer = uac(dut, "none").await;
    for _ in 0..calls {
        let handle = call(&mut peer, dut).await;
        let reason = handle
            .hangup_and_wait(Some(Duration::from_secs(10)))
            .await
            .expect("BYE completes");
        assert!(!reason.is_empty());
    }
}

fn assert_ready(dut: &Dut, transport: &str, media: &str) {
    let ready = &dut.ready;
    assert_eq!(ready["transport"], transport);
    assert_eq!(ready["media"], media);
    assert_eq!(ready["listen"], format!("127.0.0.1:{}", dut.sip_port));
    assert!(ready["pid"].as_u64().unwrap() > 0);
    assert!(ready["started_at"].as_f64().unwrap() > 1.6e9);
    assert_eq!(ready["config_sha256"].as_str().unwrap().len(), 64);
    assert!(ready["version"].is_string() && ready["git_sha"].is_string());
    assert!(ready["metrics_listen"]
        .as_str()
        .unwrap()
        .starts_with("127.0.0.1:"));
}

/// Drain and check the run files: exit 0, true zero, clean shutdown.
fn drain_cleanly(mut dut: Dut) -> serde_json::Value {
    dut.finish();
    let code = dut.wait_exit(DRAIN);
    assert_eq!(code, 0, "drain must succeed: {}", dut.log());
    let summary = dut.summary();
    assert_eq!(summary["true_zero"], true, "{summary:#}");
    assert_eq!(summary["clean_shutdown"], true, "{summary:#}");
    assert_eq!(summary["drain_timed_out"], false);
    assert!(summary["drain_seconds"].as_f64().unwrap() >= 0.0);
    assert!(summary["ended_at"].as_f64() >= summary["started_at"].as_f64());
    let lines = dut.stats_lines();
    assert!(lines.len() >= 2, "stats.jsonl has a line per interval");
    for line in &lines {
        assert!(line["t"].as_f64().unwrap() > 1.6e9);
        assert!(line.get("active_calls").is_some());
    }
    let last = lines.last().unwrap();
    assert_eq!(last["draining"], true);
    assert_eq!(last["active_calls"], 0);
    summary["counters"].clone()
}

async fn signaling_only(transport: &str) {
    let dut = Dut::start(&format!("{transport}-none"), transport, "none", "");
    assert_ready(&dut, transport, "none");
    signaling_calls(&dut, 3).await;
    // Live gauges are back at zero once the calls are over.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(dut.metric("active_calls"), 0.0);
    assert_eq!(dut.metric("calls_established_total"), 3.0);
    if transport == "tls" {
        // The UAC keeps its TLS connection open.
        assert!(dut.metric("connections") >= 1.0);
    }
    let counters = drain_cleanly(dut);
    assert_eq!(counters["invites_received_total"], 3, "{counters:#}");
    assert_eq!(counters["responses_sent_total"]["200"], 3);
    assert_eq!(counters["responses_sent_total"]["503"], 0);
    assert_eq!(counters["calls_established_total"], 3);
    assert_eq!(counters["byes_received_total"], 3);
    assert_eq!(counters["byes_answered_total"]["200"], 3);
    assert_eq!(counters["open_dialogs"], 0);
    assert_eq!(counters["rtp_packets_received_total"], 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn signaling_only_answers_an_sdes_offer_with_rtp_savp() {
    // The TLS rows offer SDES-SRTP and send no media: the answer keeps the
    // offer's RTP/SAVP with an answer key (RFC 3264 §6) on port 9.
    let dut = Dut::start("tls-none-sdes", "tls", "none", "");
    assert_ready(&dut, "tls", "none");
    let mut peer = uac(&dut, "none-srtp").await;
    let handle = call(&mut peer, &dut).await;
    let lifecycle = handle.lifecycle().await.expect("lifecycle");
    let sdp = lifecycle.answered.and_then(|a| a.sdp).expect("SDP answer");
    let audio = sdp
        .lines()
        .find(|l| l.starts_with("m=audio "))
        .expect("m=audio");
    assert!(audio.starts_with("m=audio 9 RTP/SAVP "), "{sdp}");
    assert!(sdp.lines().any(|l| l.starts_with("a=crypto:")), "{sdp}");
    handle
        .hangup_and_wait(Some(Duration::from_secs(10)))
        .await
        .expect("BYE completes");
    let counters = drain_cleanly(dut);
    assert_eq!(counters["byes_answered_total"]["200"], 1);
    assert_eq!(counters["rtp_packets_received_total"], 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn udp_signaling_only_calls_answer_and_drain_to_true_zero() {
    signaling_only("udp").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tls_signaling_only_calls_answer_and_drain_to_true_zero() {
    signaling_only("tls").await;
}

async fn echo(transport: &str, media: &str) {
    let dut = Dut::start(&format!("{transport}-{media}"), transport, media, "");
    assert_ready(&dut, transport, media);
    let mut peer = uac(&dut, media).await;
    let handle = call(&mut peer, &dut).await;
    if media == "srtp" {
        handle
            .wait_for_media_security(Some(Duration::from_secs(5)))
            .await
            .expect("SDES-SRTP negotiated");
    }
    const FRAMES: usize = 100;
    let samples = stream_tone(&handle, FRAMES).await;
    assert!(
        samples.len() >= FRAMES / 2 * 160,
        "the echo returned {} samples of {} sent",
        samples.len(),
        FRAMES * 160
    );
    let tone = goertzel(&samples, TONE_HZ);
    let off_tone = goertzel(&samples, 1_000.0).max(1.0);
    assert!(
        tone / off_tone >= 5.0,
        "echoed audio carries the caller's 440 Hz tone ({tone:.0} vs {off_tone:.0})"
    );
    let received = dut.metric("rtp_packets_received_total");
    let sent = dut.metric("rtp_packets_sent_total");
    assert!(
        received >= (FRAMES / 2) as f64,
        "DUT received {received} RTP"
    );
    assert!(sent >= received * 0.9, "DUT echoed {sent} of {received}");
    // Executor v2: the echo runs inline on the RTP socket's media lane.
    let inline = dut.metric("rtp_packets_sent_inline_total");
    assert!(inline >= sent * 0.9, "{inline} of {sent} echoed inline");
    assert_eq!(dut.metric("srtp_unprotect_failures_total"), 0.0);
    assert!(dut.metric("media_ports") >= 1.0);
    handle
        .hangup_and_wait(Some(Duration::from_secs(10)))
        .await
        .expect("BYE completes");
    let counters = drain_cleanly(dut);
    assert_eq!(counters["byes_answered_total"]["200"], 1);
    assert_eq!(counters["media_ports"], 0);
    assert_eq!(counters["echo_bind_failures_total"], 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn udp_rtp_is_echoed_back_to_the_caller() {
    echo("udp", "rtp").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tls_srtp_is_decrypted_echoed_and_reencrypted() {
    echo("tls", "srtp").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn max_cps_admission_sheds_with_503_and_retry_after() {
    let dut = Dut::start(
        "max-cps",
        "udp",
        "none",
        r#"{"admission": {"mode": "max-cps", "max_cps": 2, "retry_after_secs": 4}}"#,
    );
    let raw = RawUac::new(dut.sip_addr()).await;
    let mut answered = 0;
    let mut shed = 0;
    for index in 0..12 {
        match raw.call(&format!("maxcps-{index}")).await {
            RawOutcome::Answered { bye_status } => {
                assert_eq!(bye_status, 200, "BYE answered 200");
                answered += 1;
            }
            RawOutcome::Rejected {
                status,
                retry_after,
            } => {
                assert_eq!(status, 503);
                assert_eq!(retry_after, Some(4), "503 carries Retry-After");
                shed += 1;
            }
        }
    }
    // A one-second burst of 2 (plus GCRA slack and refill while the calls
    // ran) is admitted; the rest is shed.
    assert!((2..=6).contains(&answered), "answered {answered}");
    assert!(shed >= 6, "shed {shed}");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(dut.metric("admission_rejected_total"), f64::from(shed));
    assert_eq!(
        dut.metric("responses_sent_total{code=\"503\"}"),
        f64::from(shed)
    );
    let counters = drain_cleanly(dut);
    assert_eq!(counters["invites_received_total"], answered + shed);
    assert_eq!(counters["responses_sent_total"]["200"], answered);
    assert_eq!(counters["byes_received_total"], answered);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn drain_refuses_new_calls_and_waits_for_the_live_one() {
    let mut dut = Dut::start("drain", "udp", "none", "");
    let mut peer = uac(&dut, "none").await;
    let handle = call(&mut peer, &dut).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(dut.metric("active_calls"), 1.0);

    dut.finish();
    tokio::time::sleep(Duration::from_millis(500)).await;
    // Still draining: the live call holds it open, new calls get 503.
    assert!(dut.child.as_mut().unwrap().try_wait().unwrap().is_none());
    let raw = RawUac::new(dut.sip_addr()).await;
    match raw.call("during-drain").await {
        RawOutcome::Rejected {
            status,
            retry_after,
        } => {
            assert_eq!(status, 503);
            assert!(retry_after.is_some());
        }
        other => panic!("a call during the drain must be refused, got {other:?}"),
    }

    handle
        .hangup_and_wait(Some(Duration::from_secs(10)))
        .await
        .expect("BYE completes");
    let code = dut.wait_exit(DRAIN);
    assert_eq!(code, 0, "{}", dut.log());
    let summary = dut.summary();
    assert_eq!(summary["true_zero"], true, "{summary:#}");
    assert_eq!(summary["clean_shutdown"], true);
    assert!(summary["drain_seconds"].as_f64().unwrap() >= 0.4);
    assert_eq!(summary["counters"]["admission_rejected_total"], 1);
    assert_eq!(summary["counters"]["calls_established_total"], 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_drain_that_times_out_exits_3_without_true_zero() {
    let mut dut = Dut::start(
        "drain-timeout",
        "udp",
        "none",
        r#"{"drain_timeout_secs": 1}"#,
    );
    let mut peer = uac(&dut, "none").await;
    let handle = call(&mut peer, &dut).await;
    dut.finish();
    let code = dut.wait_exit(Duration::from_secs(20));
    assert_eq!(code, 3, "{}", dut.log());
    let summary = dut.summary();
    assert_eq!(summary["true_zero"], false);
    assert_eq!(summary["clean_shutdown"], false);
    assert_eq!(summary["drain_timed_out"], true);
    assert_eq!(summary["counters"]["active_calls"], 1);
    assert!(summary["drain_seconds"].as_f64().unwrap() >= 1.0);
    let _ = handle.hangup().await;
}

#[test]
fn a_bad_config_exits_2_before_binding() {
    let dir = support::temp_dir("bad-config");
    let path = dir.join("dut.json");
    std::fs::write(&path, r#"{"run_dir": "/tmp/x", "transport": "sctp"}"#).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rvoip-bench-uas"))
        .arg("--config")
        .arg(&path)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("sctp"));
    let missing = std::process::Command::new(env!("CARGO_BIN_EXE_rvoip-bench-uas"))
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(2));
}
