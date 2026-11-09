//! End-to-end call between two endpoints bound to the IPv6 loopback only.
//!
//! Two in-process `StreamPeer`s are configured with `Config::on("…", ::1, …)`
//! — every SIP and media address is IPv6, nothing listens on IPv4. Alice
//! calls Bob, Bob answers, both stream PCMU, Alice pings Bob with OPTIONS and
//! then hangs up. The test asserts what an IPv6 peer on the wire would check:
//!
//! - every SIP message seen by either transport (INVITE, 100/180/200, ACK,
//!   OPTIONS/200, BYE/200) parses in `ParseMode::Strict`;
//! - IPv6 hosts in Via sent-by, Contact and the Request-URI are written as
//!   an `IPv6reference` (`[::1]`, RFC 3261 §25.1, RFC 5118 §4.1);
//! - both SDP bodies use `o=… IN IP6 ::1` / `c=IN IP6 ::1` (RFC 8866 §5.2,
//!   §5.7);
//! - RTP flows both ways over the IPv6 RTP sockets.
//!
//! Ports 16460–16479 are reserved for this file.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rvoip_media_core::types::AudioFrame;
use rvoip_sip::api::unified::Config;
use rvoip_sip::{Event, SessionHandle, SipTrace, SipTraceConfig, SipTraceDirection, StreamPeer};
use rvoip_sip_core::{parse_message_with_mode, Message, ParseMode};

const ALICE_SIP_PORT: u16 = 16460;
const BOB_SIP_PORT: u16 = 16461;
const ALICE_MEDIA: (u16, u16) = (16462, 16469);
const BOB_MEDIA: (u16, u16) = (16470, 16479);

const SAMPLE_RATE: u32 = 8_000;
const FRAME_SAMPLES: usize = 160;

fn ipv6_config(name: &str, sip_port: u16, media: (u16, u16)) -> Config {
    let mut config =
        Config::on(name, "::1".parse().unwrap(), sip_port).with_media_ports(media.0, media.1);
    config.sip_trace = SipTraceConfig {
        enabled: true,
        redact_sensitive_headers: false,
        include_body: true,
        ..SipTraceConfig::default()
    };
    // The trace is the packet-capture oracle here, so it must be verbatim.
    config.trace_passthrough_for_development()
}

/// Collect every SIP trace an endpoint emits until the task is aborted.
fn collect_traces(
    mut events: rvoip_sip::EventReceiver,
) -> (Arc<Mutex<Vec<SipTrace>>>, tokio::task::JoinHandle<()>) {
    let traces = Arc::new(Mutex::new(Vec::new()));
    let sink = traces.clone();
    let task = tokio::spawn(async move {
        while let Some(event) = events.next().await {
            if let Event::SipTrace(trace) = event {
                sink.lock().unwrap().push(trace);
            }
        }
    });
    (traces, task)
}

/// The message as sent: the trace normalizes line endings to LF, and every
/// SIP line and SDP line rvoip writes ends in CRLF (RFC 3261 §7, RFC 8866
/// §5), so restoring CRLF gives back the exact serialized bytes.
fn wire_text(trace: &SipTrace) -> String {
    trace
        .raw_message
        .replace("\r\n", "\n")
        .replace('\n', "\r\n")
}

fn inbound(traces: &Arc<Mutex<Vec<SipTrace>>>) -> Vec<SipTrace> {
    traces
        .lock()
        .unwrap()
        .iter()
        .filter(|trace| trace.direction == SipTraceDirection::Inbound)
        .cloned()
        .collect()
}

fn has_inbound(traces: &Arc<Mutex<Vec<SipTrace>>>, prefix: &str, cseq_method: &str) -> bool {
    inbound(traces).iter().any(|trace| {
        trace.start_line.starts_with(prefix)
            && trace
                .raw_message
                .lines()
                .any(|line| line.starts_with("CSeq:") && line.trim_end().ends_with(cseq_method))
    })
}

async fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !done() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}

/// Send a tone on `handle` and count the frames received from the peer.
async fn exchange_audio(handle: SessionHandle, frames: usize, received: Arc<AtomicUsize>) {
    let audio = handle.audio().await.expect("audio stream opens");
    let (sender, mut receiver) = audio.split();
    let counter = received.clone();
    let drain = tokio::spawn(async move {
        while receiver.recv().await.is_some() {
            counter.fetch_add(1, Ordering::Relaxed);
        }
    });
    for i in 0..frames {
        let samples = (0..FRAME_SAMPLES)
            .map(|j| {
                let t = (i * FRAME_SAMPLES + j) as f32 / SAMPLE_RATE as f32;
                (0.3 * (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 32767.0) as i16
            })
            .collect();
        let frame = AudioFrame::new(samples, SAMPLE_RATE, 1, (i * FRAME_SAMPLES) as u32);
        if sender.send(frame).await.is_err() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    drain.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ipv6_only_call_is_rfc_compliant_on_the_wire() {
    let _ = tracing_subscriber::fmt::try_init();

    let mut bob = StreamPeer::with_config(ipv6_config("bob", BOB_SIP_PORT, BOB_MEDIA))
        .await
        .expect("bob binds [::1]");
    let mut alice = StreamPeer::with_config(ipv6_config("alice", ALICE_SIP_PORT, ALICE_MEDIA))
        .await
        .expect("alice binds [::1]");
    let (alice_traces, alice_task) =
        collect_traces(alice.coordinator().events().await.expect("alice events"));
    let (bob_traces, bob_task) =
        collect_traces(bob.coordinator().events().await.expect("bob events"));

    let bob_answer = tokio::spawn(async move {
        let incoming = tokio::time::timeout(Duration::from_secs(10), bob.wait_for_incoming())
            .await
            .expect("bob timed out waiting for the INVITE")
            .expect("bob received the INVITE");
        let handle = incoming.accept().await.expect("bob accepts");
        (bob, handle)
    });
    let target = format!("sip:bob@[::1]:{BOB_SIP_PORT}");
    let call_id = alice
        .invite(target.clone())
        .send()
        .await
        .expect("INVITE sent");
    let alice_handle =
        tokio::time::timeout(Duration::from_secs(10), alice.wait_for_answered(&call_id))
            .await
            .expect("alice timed out waiting for the answer")
            .expect("call answered");
    let (bob, bob_handle) = bob_answer.await.unwrap();

    // RTP both ways over the IPv6 media sockets.
    let alice_received = Arc::new(AtomicUsize::new(0));
    let bob_received = Arc::new(AtomicUsize::new(0));
    let alice_audio = tokio::spawn(exchange_audio(
        alice_handle.clone(),
        100,
        alice_received.clone(),
    ));
    let bob_audio = tokio::spawn(exchange_audio(
        bob_handle.clone(),
        100,
        bob_received.clone(),
    ));
    alice_audio.await.unwrap();
    bob_audio.await.unwrap();
    assert!(
        bob_received.load(Ordering::Relaxed) >= 25,
        "bob received only {} audio frames from alice over IPv6",
        bob_received.load(Ordering::Relaxed)
    );
    assert!(
        alice_received.load(Ordering::Relaxed) >= 25,
        "alice received only {} audio frames from bob over IPv6",
        alice_received.load(Ordering::Relaxed)
    );

    // An out-of-dialog OPTIONS over [::1].
    let options = alice
        .coordinator()
        .options(target.clone())
        .with_timeout(Duration::from_secs(5))
        .send()
        .await
        .expect("OPTIONS over IPv6 is answered");
    assert_eq!(options.status_code, 200, "OPTIONS answered {options:?}");

    alice_handle.hangup().await.expect("alice hangs up");
    wait_until("BYE at bob", || has_inbound(&bob_traces, "BYE ", "BYE")).await;
    wait_until("200 OK to BYE at alice", || {
        has_inbound(&alice_traces, "SIP/2.0 200", "BYE")
    })
    .await;
    // Bob saw the ACK for his 200 OK to the INVITE.
    wait_until("ACK at bob", || has_inbound(&bob_traces, "ACK ", "ACK")).await;

    alice_task.abort();
    bob_task.abort();

    // Every message each endpoint sent and received.
    for (side, traces) in [("alice", &alice_traces), ("bob", &bob_traces)] {
        let traces = traces.lock().unwrap().clone();
        assert!(
            traces
                .iter()
                .any(|trace| trace.direction == SipTraceDirection::Outbound),
            "{side} sent no SIP traffic"
        );
        assert!(
            traces
                .iter()
                .any(|trace| trace.direction == SipTraceDirection::Inbound),
            "{side} received no SIP traffic"
        );
        for trace in &traces {
            assert!(
                !trace.truncated,
                "{side}: truncated trace {}",
                trace.start_line
            );
            let wire = wire_text(trace);
            let message = parse_message_with_mode(wire.as_bytes(), ParseMode::Strict)
                .unwrap_or_else(|error| {
                    panic!("{side}: message is not Strict-valid ({error:?}):\n{wire}")
                });
            assert!(
                trace.remote_addr.starts_with("[::1]:") && trace.local_addr.starts_with("[::1]:"),
                "{side}: {} went {} -> {}",
                trace.start_line,
                trace.local_addr,
                trace.remote_addr
            );
            // RFC 3261 §25.1 / RFC 5118 §4.1: never a bare IPv6 host, and
            // no IPv4 address anywhere on an IPv6-only path.
            assert!(
                !wire.contains("127.0.0.1"),
                "{side}: IPv4 on the wire:\n{wire}"
            );
            for line in wire
                .lines()
                .filter(|line| line.starts_with("Via:") || line.starts_with("Contact:"))
            {
                assert!(line.contains("[::1]"), "{side}: unbracketed host: {line}");
            }
            if let Message::Request(request) = &message {
                assert_eq!(
                    request.uri().host,
                    rvoip_sip_core::Host::Address("::1".parse().unwrap()),
                    "{side}: Request-URI {}",
                    request.uri()
                );
            }
        }
    }

    // The offer (INVITE at bob) and the answer (200 OK at alice) carry
    // IPv6 SDP.
    for (side, traces, prefix, method) in [
        ("bob", &bob_traces, "INVITE ", "INVITE"),
        ("alice", &alice_traces, "SIP/2.0 200", "INVITE"),
    ] {
        let sdp_trace = inbound(traces)
            .into_iter()
            .find(|trace| {
                trace.start_line.starts_with(prefix)
                    && trace
                        .raw_message
                        .lines()
                        .any(|line| line.starts_with("CSeq:") && line.trim_end().ends_with(method))
                    && wire_text(trace).contains("\r\n\r\nv=0")
            })
            .unwrap_or_else(|| panic!("{side}: no SDP-bearing {prefix}"));
        let sdp = &wire_text(&sdp_trace);
        assert!(sdp.contains("\r\nc=IN IP6 ::1\r\n"), "{side}: {sdp}");
        assert!(
            sdp.lines()
                .any(|line| line.starts_with("o=") && line.ends_with(" IN IP6 ::1")),
            "{side}: {sdp}"
        );
        assert!(!sdp.contains("IP4"), "{side}: {sdp}");
    }

    drop(bob_handle);
    let _ = alice.shutdown().await;
    let _ = bob.shutdown().await;
}
