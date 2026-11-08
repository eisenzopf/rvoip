//! RFC 4475 element behaviour of an rvoip endpoint, end to end: each
//! bit-exact torture message (the sip-core fixtures) is sent over UDP to a
//! running coordinator, and the responses it puts on the wire are read and
//! checked byte for byte.
//!
//! The test sends from 127.0.0.1:5060 and also listens for TCP there. Most
//! fixtures' top Via has no port, so the response goes to the source address
//! on the default port 5060 (RFC 3261 §18.2.2), where the test receives it.
//!
//! Every response must itself parse as valid SIP in Strict mode.
//!
//! Endpoints listen on `PORT` and the ports after it, one per case.

mod support;

use std::path::PathBuf;
use std::time::Duration;

use rvoip_sip::api::unified::UnifiedCoordinator;
use rvoip_sip_core::framing::{inspect_sip_frame_with_policy, SipFrameStatus, SipFramingPolicy};
use rvoip_sip_core::{parse_message_with_mode, ParseMode};
use support::traces::receiver_config;
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::mpsc;

const PORT: u16 = 15940;

/// What the endpoint does with a message.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Expect {
    /// Final response with this status.
    Final(u16),
    /// Processed: a provisional or success response, and no rejection.
    Processed,
    /// Nothing is sent: an unanswerable message or a stray response.
    Silent,
}
use Expect::{Final, Processed, Silent};

const CASES: &[(&str, &str, Expect)] = &[
    ("wsinv", "§3.1.1.1", Processed),
    // Endpoints answer an unknown method with 501 and an Allow header.
    ("intmeth", "§3.1.1.2", Final(501)),
    ("esc01", "§3.1.1.3", Processed),
    ("esc02", "§3.1.1.5", Final(501)),
    ("lwsdisp", "§3.1.1.6", Final(200)),
    ("longreq", "§3.1.1.7", Processed),
    ("semiuri", "§3.1.1.9", Final(200)),
    ("transports", "§3.1.1.10", Final(200)),
    // Unparseable, but Via/From/To/Call-ID/CSeq parse: 400.
    ("clerr", "§3.1.2.2", Final(400)),
    ("ncl", "§3.1.2.3", Final(400)),
    ("lwsruri", "§3.1.2.8", Final(400)),
    ("mcl01", "§3.3.9", Final(400)),
    // Unparseable with a malformed Via, CSeq or To: no valid response exists.
    ("badinv01", "§3.1.2.1", Silent),
    ("scalar02", "§3.1.2.4", Silent),
    ("quotbal", "§3.1.2.6", Silent),
    ("test", "Appendix A", Silent),
    // Responses that must be discarded.
    ("scalarlg", "§3.1.2.5", Silent),
    ("bigcode", "§3.1.2.19", Silent),
    ("bcast", "§3.3.10", Silent),
    ("unreason", "§3.1.1.12", Silent),
    ("noreason", "§3.1.1.13", Silent),
    // Accepted liberally, then processed.
    ("trws", "§3.1.2.10", Final(200)),
    ("badaspec", "§3.1.2.14", Final(200)),
    ("baddn", "§3.1.2.15", Final(200)),
    ("badvers", "§3.1.2.16", Final(505)),
    ("mismatch01", "§3.1.2.17", Final(400)),
    ("mismatch02", "§3.1.2.18", Final(501)),
    ("badbranch", "§3.2.1", Final(400)),
    // No Call-ID, From or To: no valid response exists (every response
    // copies them, RFC 3261 §8.2.6.2), so the request is discarded.
    ("insuf", "§3.3.1", Silent),
    ("unkscm", "§3.3.2", Final(416)),
    ("novelsc", "§3.3.3", Final(416)),
    // A registrar rejects a non-SIP address-of-record.
    ("unksm2", "§3.3.4", Final(400)),
    ("bext01", "§3.3.5", Final(420)),
    ("invut", "§3.3.6", Final(415)),
    ("multi01", "§3.3.8", Final(400)),
    // An endpoint processes it as if Max-Forwards were still positive.
    ("zeromf", "§3.3.11", Final(200)),
    ("sdp01", "§3.3.15", Final(406)),
    // RFC 2543 INVITE (no branch): matched with the RFC 2543 rules.
    ("inv2543", "§3.4.1", Processed),
];

fn fixture(id: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../sip-core/tests/rfc_compliance/torture/rfc4475")
        .join(format!("{id}.dat"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// A value every response to the message carries: its Call-ID, or for a
/// message without one (insuf) its top Via branch.
fn correlation(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let field = |names: &[&str]| {
        text.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            names
                .contains(&name.trim().to_ascii_lowercase().as_str())
                .then(|| value.trim().to_string())
        })
    };
    field(&["call-id", "i"])
        .or_else(|| {
            field(&["via", "v"]).and_then(|via| {
                via.split(';')
                    .find_map(|param| param.trim().strip_prefix("branch=").map(str::to_string))
            })
        })
        .unwrap_or_default()
}

/// Reads SIP messages framed on one TCP connection into `wire`.
async fn read_stream(mut stream: tokio::net::TcpStream, wire: mpsc::UnboundedSender<Vec<u8>>) {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        while let Ok(SipFrameStatus::Complete(frame)) =
            inspect_sip_frame_with_policy(&buffer, SipFramingPolicy::Stream)
        {
            let message: Vec<u8> = buffer.drain(..frame.total_bytes).collect();
            if wire.send(message).is_err() {
                return;
            }
        }
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(n) => buffer.extend_from_slice(&chunk[..n]),
        }
    }
}

/// Responses carrying `correlation`, in order, until `window` passes or a
/// final response arrives. Other messages (retransmissions for earlier
/// cases) are skipped.
async fn responses_for(
    wire: &mut mpsc::UnboundedReceiver<Vec<u8>>,
    correlation: &str,
    window: Duration,
) -> Vec<(u16, Vec<u8>)> {
    let deadline = tokio::time::Instant::now() + window;
    let mut found = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return found;
        }
        let Ok(Some(message)) = tokio::time::timeout(remaining, wire.recv()).await else {
            return found;
        };
        let text = String::from_utf8_lossy(&message);
        if correlation.is_empty() || !text.starts_with("SIP/2.0 ") || !text.contains(correlation) {
            continue;
        }
        let status = text
            .get(8..11)
            .and_then(|code| code.parse().ok())
            .unwrap_or(0);
        found.push((status, message));
        if status >= 200 {
            return found;
        }
    }
}

/// The test's side of the wire: fixtures go out from 127.0.0.1:5060, and
/// every message arriving there over UDP or TCP is collected.
struct Wire {
    socket: std::sync::Arc<UdpSocket>,
    received: mpsc::UnboundedReceiver<Vec<u8>>,
    tasks: [tokio::task::JoinHandle<()>; 2],
}

impl Wire {
    async fn bind() -> Self {
        let socket = UdpSocket::bind("127.0.0.1:5060")
            .await
            .expect("bind UDP 127.0.0.1:5060, where responses to the fixtures go");
        let listener = TcpListener::bind("127.0.0.1:5060")
            .await
            .expect("bind TCP 127.0.0.1:5060, where responses to the fixtures go");
        let socket = std::sync::Arc::new(socket);
        let (wire_tx, received) = mpsc::unbounded_channel();
        let udp = {
            let socket = socket.clone();
            let wire = wire_tx.clone();
            tokio::spawn(async move {
                let mut datagram = vec![0u8; 65535];
                while let Ok((n, _)) = socket.recv_from(&mut datagram).await {
                    if wire.send(datagram[..n].to_vec()).is_err() {
                        return;
                    }
                }
            })
        };
        let tcp = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(read_stream(stream, wire_tx.clone()));
            }
        });
        Self {
            socket,
            received,
            tasks: [udp, tcp],
        }
    }

    /// Sends fixture `id` to the endpoint on `port` and returns its
    /// responses carrying `correlation`, each checked to be valid SIP.
    async fn exchange(
        &mut self,
        id: &str,
        port: u16,
        correlation: &str,
        window: Duration,
        failures: &mut Vec<String>,
    ) -> Vec<u16> {
        tracing::info!(case = id, "sending RFC 4475 message");
        self.socket
            .send_to(&fixture(id), ("127.0.0.1", port))
            .await
            .expect("send fixture");
        let responses = responses_for(&mut self.received, correlation, window).await;
        for (status, response) in &responses {
            if let Err(error) = parse_message_with_mode(response, ParseMode::Strict) {
                failures.push(format!(
                    "{id}: response {status} is not valid SIP ({error}):\n{}",
                    String::from_utf8_lossy(response).escape_debug()
                ));
            }
        }
        responses.into_iter().map(|(status, _)| status).collect()
    }
}

impl Drop for Wire {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

const FINAL_WINDOW: Duration = Duration::from_secs(5);
const QUIET_WINDOW: Duration = Duration::from_millis(1500);

/// Each message goes to a fresh endpoint, so it is judged on its own: several
/// RFC 4475 messages share a branch and sent-by, which would otherwise make
/// one a retransmission of another (checked separately below).
async fn each_message_gets_the_rfc_response(wire: &mut Wire, failures: &mut Vec<String>) {
    for (index, (id, section, expect)) in CASES.iter().enumerate() {
        let port = PORT + 1 + index as u16;
        let coordinator = UnifiedCoordinator::new(receiver_config(id, port))
            .await
            .expect("coordinator");
        let window = match expect {
            Silent | Processed => QUIET_WINDOW,
            Final(_) => FINAL_WINDOW,
        };
        let statuses = wire
            .exchange(id, port, &correlation(&fixture(id)), window, failures)
            .await;
        coordinator.shutdown();

        let final_status = statuses.iter().copied().find(|s| *s >= 200);
        let ok = match expect {
            Final(status) => final_status == Some(*status),
            Silent => statuses.is_empty(),
            // An INVITE that is processed: a provisional or success response
            // and no rejection.
            Processed => {
                !statuses.is_empty()
                    && !matches!(
                        final_status,
                        Some(400 | 405 | 406 | 415 | 416 | 420 | 501 | 505)
                    )
            }
        };
        if !ok {
            failures.push(format!(
                "{id} (RFC 4475 {section}): expected {expect:?}, sent {statuses:?}"
            ));
        }
    }
}

/// RFC 3261 §17.2.3 on one endpoint: a request matches a server transaction
/// only when branch, sent-by and method all match.
async fn transactions_match_on_branch_sent_by_and_method(
    wire: &mut Wire,
    failures: &mut Vec<String>,
) {
    let coordinator = UnifiedCoordinator::new(receiver_config("torture-matching", PORT))
        .await
        .expect("coordinator");
    // One branch (z9hG4bKkdjuw), four sent-by values: four transactions.
    for id in ["lwsdisp", "semiuri", "transports", "baddn"] {
        let statuses = wire
            .exchange(id, PORT, &correlation(&fixture(id)), FINAL_WINDOW, failures)
            .await;
        if statuses != [200] {
            failures.push(format!(
                "{id} after a request with its branch and another sent-by: expected [200], sent {statuses:?}"
            ));
        }
    }
    // novelsc has unkscm's branch, sent-by and method: it is a
    // retransmission, answered with unkscm's response again.
    let unkscm = correlation(&fixture("unkscm"));
    let statuses = wire
        .exchange("unkscm", PORT, &unkscm, FINAL_WINDOW, failures)
        .await;
    if statuses != [416] {
        failures.push(format!("unkscm: expected [416], sent {statuses:?}"));
    }
    let statuses = wire
        .exchange("novelsc", PORT, &unkscm, FINAL_WINDOW, failures)
        .await;
    if statuses != [416] {
        failures.push(format!(
            "novelsc after unkscm (same branch, sent-by, method): expected unkscm's 416 again, sent {statuses:?}"
        ));
    }
    coordinator.shutdown();
}

// One test: both parts receive on 127.0.0.1:5060.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rfc4475_messages_get_the_rfc_response() {
    let _ = tracing_subscriber::fmt::try_init();
    let mut wire = Wire::bind().await;
    let mut failures = Vec::new();
    each_message_gets_the_rfc_response(&mut wire, &mut failures).await;
    transactions_match_on_branch_sent_by_and_method(&mut wire, &mut failures).await;
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
