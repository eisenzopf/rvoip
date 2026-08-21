use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use rvoip_rtp_core::quality::monitor::{
    fraction_lost_percent, jitter_timestamp_units_to_ms, round_trip_ms, signed_24_bit,
    ReceiverReportDelta, ReceiverReportDeltaTracker, RtcpRelayObserver, RtpQualityTracker,
};

fn rtp(sequence: u16, timestamp: u32) -> [u8; 12] {
    rtp_for_ssrc(42, sequence, timestamp)
}

fn rtp_for_ssrc(ssrc: u32, sequence: u16, timestamp: u32) -> [u8; 12] {
    let mut packet = [0_u8; 12];
    packet[0] = 0x80;
    packet[1] = 8;
    packet[2..4].copy_from_slice(&sequence.to_be_bytes());
    packet[4..8].copy_from_slice(&timestamp.to_be_bytes());
    packet[8..12].copy_from_slice(&ssrc.to_be_bytes());
    packet
}

#[test]
fn bounds_ssrc_state_and_reseeds_an_evicted_stream() {
    let mut tracker = RtpQualityTracker::with_stream_limit(HashMap::from([(8, 8_000)]), 2);
    tracker.observe(&rtp_for_ssrc(1, 10, 0)).unwrap();
    tracker.observe(&rtp_for_ssrc(2, 10, 0)).unwrap();
    tracker.observe(&rtp_for_ssrc(3, 10, 0)).unwrap();

    let reintroduced = tracker.observe(&rtp_for_ssrc(1, 30_000, 160)).unwrap();
    assert_eq!(reintroduced.inferred_lost, 0);
}

fn sender_report(compact_ntp: u32) -> Vec<u8> {
    let mut packet = vec![0x80, 200, 0, 6, 0, 0, 0, 1];
    packet.extend_from_slice(&(compact_ntp >> 16).to_be_bytes());
    packet.extend_from_slice(&(compact_ntp << 16).to_be_bytes());
    packet.extend_from_slice(&[0; 12]);
    packet
}

fn receiver_report(compact_ntp: u32, delay_since_last_sr: u32) -> Vec<u8> {
    let mut packet = vec![0x81, 201, 0, 7, 0, 0, 0, 2];
    packet.extend_from_slice(&1_u32.to_be_bytes());
    packet.extend_from_slice(&[0; 12]);
    packet.extend_from_slice(&compact_ntp.to_be_bytes());
    packet.extend_from_slice(&delay_since_last_sr.to_be_bytes());
    packet
}

#[test]
fn converts_rtcp_fixed_point_units_without_magic_at_call_sites() {
    assert_eq!(fraction_lost_percent(128), 50.0);
    assert_eq!(jitter_timestamp_units_to_ms(400, 8_000), Some(50.0));
    assert_eq!(jitter_timestamp_units_to_ms(400, 0), None);
    assert_eq!(round_trip_ms(Duration::from_secs(2), 65_536), 1_000.0);
}

#[test]
fn sign_extends_the_rtcp_signed_24_bit_loss_field() {
    assert_eq!(signed_24_bit(0x007f_ffff), 8_388_607);
    assert_eq!(signed_24_bit(0x00ff_ffff), -1);
}

#[test]
fn reports_window_deltas_and_rebaselines_after_a_restart() {
    let mut tracker = ReceiverReportDeltaTracker::default();
    assert_eq!(tracker.observe(10, 100), None);
    assert_eq!(
        tracker.observe(13, 120),
        Some(ReceiverReportDelta {
            packets_lost: 3,
            packets_expected: 20
        })
    );
    assert_eq!(tracker.observe(1, 5), None);
    assert_eq!(
        tracker.observe(2, 15),
        Some(ReceiverReportDelta {
            packets_lost: 1,
            packets_expected: 10
        })
    );
}

#[test]
fn observes_rtp_loss_and_jitter_without_changing_the_packet() {
    let mut tracker = RtpQualityTracker::new(HashMap::from([(8, 8_000)]));
    let first = rtp(10, 0);
    let gap = rtp(13, 160);
    assert_eq!(tracker.observe(&first).unwrap().inferred_lost, 0);
    let observation = tracker.observe(&gap).unwrap();
    assert_eq!(observation.inferred_lost, 2);
    assert!(observation.jitter_seconds.unwrap().is_finite());
    assert_eq!(gap, rtp(13, 160));
}

#[test]
fn correlates_relayed_sr_and_rr_then_consumes_the_match() {
    let compact_ntp = 0x1234_5678;
    let started = Instant::now();
    let mut observer = RtcpRelayObserver::default();
    assert!(observer
        .observe_at(&sender_report(compact_ntp), started)
        .is_empty());

    let reports = observer.observe_at(
        &receiver_report(compact_ntp, 65_536),
        started + Duration::from_secs(2),
    );
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].rtt_seconds, Some(1.0));

    let repeated = observer.observe_at(
        &receiver_report(compact_ntp, 65_536),
        started + Duration::from_secs(3),
    );
    assert_eq!(repeated[0].rtt_seconds, None);
}

#[test]
fn expires_unmatched_sender_reports_at_the_configured_bound() {
    let compact_ntp = 0x2345_6789;
    let started = Instant::now();
    let mut observer = RtcpRelayObserver::new(Duration::from_secs(5));
    observer.observe_at(&sender_report(compact_ntp), started);
    let reports = observer.observe_at(
        &receiver_report(compact_ntp, 0),
        started + Duration::from_secs(6),
    );
    assert_eq!(reports[0].rtt_seconds, None);
}

#[test]
fn bounds_unmatched_sender_reports_without_matching_evicted_entries() {
    let started = Instant::now();
    let mut observer = RtcpRelayObserver::default();
    for compact_ntp in 0..=64 {
        observer.observe_at(&sender_report(compact_ntp), started);
    }

    let oldest = observer.observe_at(&receiver_report(0, 0), started + Duration::from_secs(1));
    assert_eq!(oldest[0].rtt_seconds, None);
    let newest = observer.observe_at(&receiver_report(64, 0), started + Duration::from_secs(1));
    assert_eq!(newest[0].rtt_seconds, Some(1.0));
}
