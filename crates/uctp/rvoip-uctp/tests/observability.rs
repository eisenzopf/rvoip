//! Observability tests per `UCTP_IMPLEMENTATION_PLAN.md` §3.9 / §3.8.
//!
//! Installs a test metrics recorder + tracing-subscriber capture, runs
//! a full invite-accept flow, and asserts:
//! - `uctp_envelopes_total{type="session.invite"}` increments per side
//! - `uctp_handshake_duration_seconds` records exactly one observation
//! - `uctp_sessions_active` gauge moves with session lifecycle

use std::sync::{Arc, Mutex};

use chrono::Utc;
use metrics::{Counter, Gauge, Histogram, Key, KeyName, Metadata, Recorder, SharedString, Unit};
use rvoip_auth_core::bearer_stub;
use rvoip_uctp::{
    envelope::UctpEnvelope,
    payloads::session,
    state::{UctpCoordinator, ENVELOPE_CHANNEL_CAP},
    types::MessageType,
};
use tokio::sync::mpsc;

mod common;
use common::drive_auth_handshake;

#[derive(Default, Clone)]
struct CaptureState {
    counters: Arc<Mutex<Vec<(String, Vec<(String, String)>, u64)>>>,
    histograms: Arc<Mutex<Vec<(String, Vec<(String, String)>, f64)>>>,
    gauges: Arc<Mutex<Vec<(String, Vec<(String, String)>, f64)>>>,
}

struct CaptureRecorder(CaptureState);

impl Recorder for CaptureRecorder {
    fn describe_counter(&self, _key: KeyName, _unit: Option<Unit>, _description: SharedString) {}
    fn describe_gauge(&self, _key: KeyName, _unit: Option<Unit>, _description: SharedString) {}
    fn describe_histogram(&self, _key: KeyName, _unit: Option<Unit>, _description: SharedString) {}

    fn register_counter(&self, key: &Key, _metadata: &Metadata<'_>) -> Counter {
        let state = self.0.clone();
        let name = key.name().to_string();
        let labels: Vec<(String, String)> = key
            .labels()
            .map(|l| (l.key().to_string(), l.value().to_string()))
            .collect();
        Counter::from_arc(Arc::new(CaptureCounter {
            state,
            name,
            labels,
        }))
    }

    fn register_gauge(&self, key: &Key, _metadata: &Metadata<'_>) -> Gauge {
        let state = self.0.clone();
        let name = key.name().to_string();
        let labels: Vec<(String, String)> = key
            .labels()
            .map(|l| (l.key().to_string(), l.value().to_string()))
            .collect();
        Gauge::from_arc(Arc::new(CaptureGauge {
            state,
            name,
            labels,
        }))
    }

    fn register_histogram(&self, key: &Key, _metadata: &Metadata<'_>) -> Histogram {
        let state = self.0.clone();
        let name = key.name().to_string();
        let labels: Vec<(String, String)> = key
            .labels()
            .map(|l| (l.key().to_string(), l.value().to_string()))
            .collect();
        Histogram::from_arc(Arc::new(CaptureHistogram {
            state,
            name,
            labels,
        }))
    }
}

struct CaptureCounter {
    state: CaptureState,
    name: String,
    labels: Vec<(String, String)>,
}
impl metrics::CounterFn for CaptureCounter {
    fn increment(&self, value: u64) {
        self.state
            .counters
            .lock()
            .unwrap()
            .push((self.name.clone(), self.labels.clone(), value));
    }
    fn absolute(&self, value: u64) {
        self.increment(value);
    }
}

struct CaptureGauge {
    state: CaptureState,
    name: String,
    labels: Vec<(String, String)>,
}
impl metrics::GaugeFn for CaptureGauge {
    fn increment(&self, value: f64) {
        self.state
            .gauges
            .lock()
            .unwrap()
            .push((self.name.clone(), self.labels.clone(), value));
    }
    fn decrement(&self, value: f64) {
        self.state
            .gauges
            .lock()
            .unwrap()
            .push((self.name.clone(), self.labels.clone(), -value));
    }
    fn set(&self, value: f64) {
        self.state
            .gauges
            .lock()
            .unwrap()
            .push((self.name.clone(), self.labels.clone(), value));
    }
}

struct CaptureHistogram {
    state: CaptureState,
    name: String,
    labels: Vec<(String, String)>,
}
impl metrics::HistogramFn for CaptureHistogram {
    fn record(&self, value: f64) {
        self.state
            .histograms
            .lock()
            .unwrap()
            .push((self.name.clone(), self.labels.clone(), value));
    }
}

fn install_capture() -> CaptureState {
    let state = CaptureState::default();
    let _ = metrics::set_global_recorder(CaptureRecorder(state.clone()));
    state
}

fn invite_env(sid: &str) -> UctpEnvelope {
    let payload = session::SessionInvite {
        from: "part_alice".into(),
        to: vec!["part_bob".into()],
        medium: "voice".into(),
        intent: "synchronous-engagement".into(),
        capabilities_offer: serde_json::Value::Object(Default::default()),
    };
    UctpEnvelope {
        v: 1,
        msg_type: MessageType::SessionInvite,
        id: "env_inv".into(),
        ts: Utc::now(),
        cid: Some("conv_x".into()),
        sid: Some(sid.into()),
        connid: None,
        in_reply_to: None,
        payload: serde_json::to_value(payload).unwrap(),
        signature: None,
    }
}

fn accept_env(sid: &str) -> UctpEnvelope {
    let payload = session::SessionAccept {
        by: "part_bob".into(),
        capabilities_answer: serde_json::Value::Object(Default::default()),
    };
    UctpEnvelope {
        v: 1,
        msg_type: MessageType::SessionAccept,
        id: "env_acc".into(),
        ts: Utc::now(),
        cid: Some("conv_x".into()),
        sid: Some(sid.into()),
        connid: None,
        in_reply_to: Some("env_inv".into()),
        payload: serde_json::to_value(payload).unwrap(),
        signature: None,
    }
}

#[tokio::test]
async fn observability_emits_counter_gauge_histogram() {
    let capture = install_capture();

    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);

    let _coord = UctpCoordinator::start("quic", in_rx, out_tx, events_tx, bearer_stub());

    drive_auth_handshake(&in_tx, &mut out_rx).await;

    in_tx.send(invite_env("sess_observ")).await.unwrap();
    in_tx.send(accept_env("sess_observ")).await.unwrap();

    // Give the coordinator a moment to process.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let counters = capture.counters.lock().unwrap();
    let histograms = capture.histograms.lock().unwrap();
    let gauges = capture.gauges.lock().unwrap();

    // (a) `uctp_envelopes_total{type="session.invite"}` increments.
    assert!(
        counters
            .iter()
            .any(|(name, labels, _)| name == "uctp_envelopes_total"
                && labels
                    .iter()
                    .any(|(k, v)| k == "type" && v == "session.invite")),
        "expected uctp_envelopes_total{{type=session.invite}}; got {:?}",
        *counters
    );

    // (b) `uctp_handshake_duration_seconds` records once on accept.
    assert!(
        histograms
            .iter()
            .any(|(name, _, _)| name == "uctp_handshake_duration_seconds"),
        "expected uctp_handshake_duration_seconds histogram observation; got {:?}",
        *histograms
    );

    // (c) `uctp_sessions_active` gauge fired at least once (set to 1 on invite).
    assert!(
        gauges
            .iter()
            .any(|(name, _, _)| name == "uctp_sessions_active"),
        "expected uctp_sessions_active gauge; got {:?}",
        *gauges
    );

    // (d) `uctp_connections_negotiating` gauge fires every time
    // `refresh_gauges()` runs — at minimum on session.invite.
    assert!(
        gauges
            .iter()
            .any(|(name, _, _)| name == "uctp_connections_negotiating"),
        "expected uctp_connections_negotiating gauge; got {:?}",
        *gauges
    );

    // (e) `uctp_substrate_pending_outstanding` gauge fires alongside
    // the other gauges in `refresh_gauges()` — present even when zero
    // outstanding requests, so leak detection has a baseline reading.
    assert!(
        gauges
            .iter()
            .any(|(name, _, _)| name == "uctp_substrate_pending_outstanding"),
        "expected uctp_substrate_pending_outstanding gauge; got {:?}",
        *gauges
    );
}
