//! Loopback throughput sanity check (ignored by default; run in release):
//!
//! ```text
//! BENCH_TRANSPORT=tls BENCH_CPS=2000 BENCH_SECONDS=15 BENCH_CONNECTIONS=8 \
//!   cargo test --release -p rvoip-bench-uas --test throughput -- --ignored --nocapture
//! ```
//!
//! A minimal signaling-only caller: `BENCH_CONNECTIONS` flows (one TLS
//! connection or one UDP socket each) send INVITE at an even share of
//! `BENCH_CPS`, ACK every 200 OK and BYE at once (zero hold). It is not the
//! benchmark's load generator; it only checks that the DUT answers a few
//! thousand calls per second on loopback without a failure and drains to
//! true zero afterwards.

mod support;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_rustls::rustls;

use support::Dut;

fn env<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

#[derive(Default)]
struct Totals {
    sent: AtomicU64,
    answered: AtomicU64,
    retransmitted_200: AtomicU64,
    rejected: AtomicU64,
    completed: AtomicU64,
    bye_failed: AtomicU64,
}

/// A TLS client that does not verify the server (the DUT's self-signed
/// test certificate); signatures are still checked.
#[derive(Debug)]
struct AcceptAnyServer(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for AcceptAnyServer {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

fn header<'a>(message: &'a str, name: &str) -> Option<&'a str> {
    message.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then_some(value.trim())
    })
}

/// Split complete SIP messages off the front of `buffer`.
fn take_messages(buffer: &mut Vec<u8>) -> Vec<String> {
    let mut messages = Vec::new();
    loop {
        let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") else {
            break;
        };
        let head = String::from_utf8_lossy(&buffer[..end]).into_owned();
        let body = header(&head, "Content-Length")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        let total = end + 4 + body;
        if buffer.len() < total {
            break;
        }
        messages.push(String::from_utf8_lossy(&buffer[..total]).into_owned());
        buffer.drain(..total);
    }
    messages
}

/// Turn one response into the requests it calls for (ACK and BYE for a
/// 200 OK to INVITE), updating the totals.
fn on_response(
    message: &str,
    transport: &str,
    local: SocketAddr,
    target: SocketAddr,
    open: &mut HashMap<String, bool>,
    totals: &Totals,
) -> Option<String> {
    let status: u16 = message.split_whitespace().nth(1)?.parse().ok()?;
    if status < 200 {
        return None;
    }
    let call_id = header(message, "Call-ID")?.to_string();
    let cseq = header(message, "CSeq")?;
    if cseq.ends_with("BYE") {
        open.remove(&call_id);
        if status == 200 {
            totals.completed.fetch_add(1, Ordering::Relaxed);
        } else {
            totals.bye_failed.fetch_add(1, Ordering::Relaxed);
        }
        return None;
    }
    let to = header(message, "To")?;
    let branch = format!("z9hG4bK-{call_id}");
    if status >= 300 {
        totals.rejected.fetch_add(1, Ordering::Relaxed);
        open.remove(&call_id);
        return Some(format!(
            "ACK sip:dut@{target} SIP/2.0\r\nVia: SIP/2.0/{transport} {local};branch={branch}\r\n\
             Max-Forwards: 70\r\nFrom: <sip:drv@{local}>;tag=t-{call_id}\r\nTo: {to}\r\n\
             Call-ID: {call_id}\r\nCSeq: 1 ACK\r\nContent-Length: 0\r\n\r\n"
        ));
    }
    // A retransmitted 200 OK (the ACK crossed it) only needs its ACK again;
    // the BYE went with the first one.
    let first = match open.get_mut(&call_id) {
        Some(answered) if !*answered => {
            *answered = true;
            true
        }
        _ => false,
    };
    if first {
        totals.answered.fetch_add(1, Ordering::Relaxed);
    } else {
        totals.retransmitted_200.fetch_add(1, Ordering::Relaxed);
    }
    let contact = header(message, "Contact")
        .map(|contact| {
            contact
                .trim_start_matches('<')
                .split('>')
                .next()
                .unwrap_or(contact)
                .to_string()
        })
        .unwrap_or_else(|| format!("sip:dut@{target}"));
    let mut out = String::new();
    let requests: &[(&str, u32, &str)] = if first {
        &[("ACK", 1, "a"), ("BYE", 2, "b")]
    } else {
        &[("ACK", 1, "a")]
    };
    for &(method, cseq, suffix) in requests {
        out.push_str(&format!(
            "{method} {contact} SIP/2.0\r\nVia: SIP/2.0/{transport} {local};branch={branch}-{suffix}\r\n\
             Max-Forwards: 70\r\nFrom: <sip:drv@{local}>;tag=t-{call_id}\r\nTo: {to}\r\n\
             Call-ID: {call_id}\r\nCSeq: {cseq} {method}\r\nContent-Length: 0\r\n\r\n"
        ));
    }
    Some(out)
}

fn invite(transport: &str, local: SocketAddr, target: SocketAddr, call_id: &str) -> String {
    let scheme_params = if transport == "TLS" {
        ";transport=tls"
    } else {
        ""
    };
    let sdp = "v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\n\
               m=audio 9 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
    format!(
        "INVITE sip:dut@{target}{scheme_params} SIP/2.0\r\n\
         Via: SIP/2.0/{transport} {local};branch=z9hG4bK-{call_id}\r\n\
         Max-Forwards: 70\r\nFrom: <sip:drv@{local}>;tag=t-{call_id}\r\n\
         To: <sip:dut@{target}>\r\nContact: <sip:drv@{local}{scheme_params}>\r\n\
         Call-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContent-Type: application/sdp\r\n\
         Content-Length: {}\r\n\r\n{sdp}",
        sdp.len()
    )
}

/// One TLS flow: paced INVITEs and the responses, until `deadline` plus a
/// settling grace for the last calls.
async fn tls_flow<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    flow: usize,
    local: SocketAddr,
    target: SocketAddr,
    interval: Duration,
    deadline: Instant,
    totals: Arc<Totals>,
) -> usize {
    let (mut reader, mut writer) = tokio::io::split(stream);
    let mut open: HashMap<String, bool> = HashMap::new();
    let mut buffer = Vec::with_capacity(64 * 1024);
    let mut chunk = vec![0u8; 64 * 1024];
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
    let mut index = 0u64;
    let grace = deadline + Duration::from_secs(5);
    loop {
        let sending = Instant::now() < deadline;
        if !sending && (open.is_empty() || Instant::now() > grace) {
            return open.len();
        }
        tokio::select! {
            _ = ticker.tick(), if sending => {
                let call_id = format!("tp-{flow}-{index}");
                index += 1;
                open.insert(call_id.clone(), false);
                totals.sent.fetch_add(1, Ordering::Relaxed);
                writer.write_all(invite("TLS", local, target, &call_id).as_bytes()).await.unwrap();
            }
            read = reader.read(&mut chunk) => {
                let read = read.expect("TLS read");
                assert!(read > 0, "DUT closed the TLS connection");
                buffer.extend_from_slice(&chunk[..read]);
                let mut out = String::new();
                for message in take_messages(&mut buffer) {
                    if let Some(requests) = on_response(&message, "TLS", local, target, &mut open, &totals) {
                        out.push_str(&requests);
                    }
                }
                if !out.is_empty() {
                    writer.write_all(out.as_bytes()).await.unwrap();
                }
            }
            () = tokio::time::sleep(Duration::from_millis(50)), if !sending => {}
        }
    }
}

async fn udp_flow(
    flow: usize,
    target: SocketAddr,
    interval: Duration,
    deadline: Instant,
    totals: Arc<Totals>,
) -> usize {
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let local = socket.local_addr().unwrap();
    let mut open: HashMap<String, bool> = HashMap::new();
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
    let mut buffer = vec![0u8; 65_536];
    let mut index = 0u64;
    let grace = deadline + Duration::from_secs(5);
    loop {
        let sending = Instant::now() < deadline;
        if !sending && (open.is_empty() || Instant::now() > grace) {
            return open.len();
        }
        tokio::select! {
            _ = ticker.tick(), if sending => {
                let call_id = format!("tp-{flow}-{index}");
                index += 1;
                open.insert(call_id.clone(), false);
                totals.sent.fetch_add(1, Ordering::Relaxed);
                socket.send_to(invite("UDP", local, target, &call_id).as_bytes(), target).await.unwrap();
            }
            received = socket.recv_from(&mut buffer) => {
                let (count, _) = received.unwrap();
                let message = String::from_utf8_lossy(&buffer[..count]).into_owned();
                if let Some(requests) = on_response(&message, "UDP", local, target, &mut open, &totals) {
                    let mut rest = requests.as_bytes().to_vec();
                    for message in take_messages(&mut rest) {
                        socket.send_to(message.as_bytes(), target).await.unwrap();
                    }
                }
            }
            () = tokio::time::sleep(Duration::from_millis(50)), if !sending => {}
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "throughput sanity check; run in release with --ignored"]
async fn loopback_throughput() {
    let transport: String = env("BENCH_TRANSPORT", "tls".to_string());
    let cps: f64 = env("BENCH_CPS", 2_000.0);
    let seconds: u64 = env("BENCH_SECONDS", 15);
    let connections: usize = env("BENCH_CONNECTIONS", 8);
    let dut = Dut::start(
        "throughput",
        &transport,
        "none",
        r#"{"worker_threads": null, "tuning": {}, "stats_interval_ms": 1000}"#,
    );
    let target = dut.sip_addr();
    let totals = Arc::new(Totals::default());
    let interval = Duration::from_secs_f64(connections as f64 / cps);
    let started = Instant::now();
    let deadline = started + Duration::from_secs(seconds);
    let mut flows = Vec::new();
    let tls_config = (transport == "tls").then(|| {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        Arc::new(
            rustls::ClientConfig::builder_with_provider(Arc::clone(&provider))
                .with_safe_default_protocol_versions()
                .unwrap()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(AcceptAnyServer(provider)))
                .with_no_client_auth(),
        )
    });
    for flow in 0..connections {
        let totals = Arc::clone(&totals);
        let tls_config = tls_config.clone();
        flows.push(tokio::spawn(async move {
            match tls_config {
                Some(config) => {
                    let tcp = tokio::net::TcpStream::connect(target).await.unwrap();
                    tcp.set_nodelay(true).unwrap();
                    let local = tcp.local_addr().unwrap();
                    let stream = tokio_rustls::TlsConnector::from(config)
                        .connect(
                            rustls::pki_types::ServerName::try_from("localhost").unwrap(),
                            tcp,
                        )
                        .await
                        .expect("TLS handshake");
                    tls_flow(stream, flow, local, target, interval, deadline, totals).await
                }
                None => udp_flow(flow, target, interval, deadline, totals).await,
            }
        }));
    }
    let mut unfinished = 0;
    for flow in flows {
        unfinished += flow.await.unwrap();
    }
    let elapsed = Duration::from_secs(seconds).as_secs_f64();
    let sent = totals.sent.load(Ordering::Relaxed);
    let completed = totals.completed.load(Ordering::Relaxed);
    println!(
        "transport={transport} offered={cps} cps connections={connections} sent={sent} \
         answered={} retransmitted_200={} rejected={} completed={completed} bye_failed={} unfinished={unfinished} \
         completed_rate={:.0} cps",
        totals.answered.load(Ordering::Relaxed),
        totals.retransmitted_200.load(Ordering::Relaxed),
        totals.rejected.load(Ordering::Relaxed),
        totals.bye_failed.load(Ordering::Relaxed),
        completed as f64 / elapsed,
    );
    println!(
        "dut cpu_seconds={} rss_bytes={}",
        dut.metric("process_cpu_seconds_total"),
        dut.metric("process_resident_memory_bytes")
    );
    assert_eq!(totals.rejected.load(Ordering::Relaxed), 0);
    assert_eq!(totals.bye_failed.load(Ordering::Relaxed), 0);
    assert_eq!(unfinished, 0, "every call completed");
    assert_eq!(completed, sent);

    let mut dut = dut;
    dut.finish();
    assert_eq!(dut.wait_exit(Duration::from_secs(60)), 0, "{}", dut.log());
    let summary = dut.summary();
    assert_eq!(summary["true_zero"], true, "{summary:#}");
    println!(
        "dut summary: invites={} established={} byes={} drain_seconds={}",
        summary["counters"]["invites_received_total"],
        summary["counters"]["calls_established_total"],
        summary["counters"]["byes_received_total"],
        summary["drain_seconds"]
    );
}
