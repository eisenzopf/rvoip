//! Shared harness: run the DUT binary on loopback, place calls at it with
//! rvoip's own UAC (and a raw UDP UAC where the test needs the wire).

#![allow(dead_code)]

use std::net::{SocketAddr, TcpListener, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use rvoip_media_core::types::AudioFrame;
use rvoip_sip::api::unified::{Config, SipTlsMode};
use rvoip_sip::{SessionHandle, StreamPeer};
use serde_json::Value;

pub const SAMPLE_RATE: u32 = 8_000;
pub const FRAME_SAMPLES: usize = 160;
pub const TONE_HZ: f32 = 440.0;

static NEXT_DIR: AtomicU32 = AtomicU32::new(0);

/// A fresh directory under the system temp dir.
pub fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rvoip-bench-uas-{label}-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A loopback port free for both UDP and TCP right now.
pub fn free_port() -> u16 {
    for _ in 0..200 {
        let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = udp.local_addr().unwrap().port();
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!("no free loopback port");
}

/// `count` consecutive loopback UDP ports (even start) free right now, below
/// the kernel's ephemeral range so no later ephemeral bind takes them.
pub fn free_port_block(count: u16) -> (u16, u16) {
    static NEXT_BLOCK: AtomicU32 = AtomicU32::new(0);
    const LOW: u32 = 20_000;
    const SPAN: u32 = 12_000;
    let offset = (std::process::id() % 97) * 128;
    for _ in 0..400 {
        let index = NEXT_BLOCK.fetch_add(1, Ordering::Relaxed);
        let start =
            (LOW + (offset + index * (u32::from(count) + 2)) % (SPAN - u32::from(count))) & !1;
        let start = u16::try_from(start).unwrap();
        let held: Vec<_> = (start..start + count)
            .map_while(|port| UdpSocket::bind(("127.0.0.1", port)).ok())
            .collect();
        if held.len() == usize::from(count) {
            return (start, start + count - 1);
        }
    }
    panic!("no free block of {count} UDP ports");
}

/// A self-signed certificate and key for `localhost` / `127.0.0.1`.
pub fn self_signed(dir: &Path) -> (PathBuf, PathBuf) {
    let cert =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string(), "127.0.0.1".to_string()])
            .unwrap();
    let cert_path = dir.join("dut.crt");
    let key_path = dir.join("dut.key");
    std::fs::write(&cert_path, cert.cert.pem()).unwrap();
    std::fs::write(&key_path, cert.signing_key.serialize_pem()).unwrap();
    (cert_path, key_path)
}

/// A running DUT process.
pub struct Dut {
    pub child: Option<Child>,
    pub run_dir: PathBuf,
    pub sip_port: u16,
    pub tls: bool,
    pub tls_files: Option<(PathBuf, PathBuf)>,
    pub ready: Value,
}

impl Drop for Dut {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Dut {
    /// Start the DUT with `transport`/`media`; `extra` is a JSON object (or
    /// empty) whose fields override the defaults. Waits for `ready.json`.
    pub fn start(label: &str, transport: &str, media: &str, extra: &str) -> Self {
        let dir = temp_dir(label);
        let run_dir = dir.join("run");
        let sip_port = free_port();
        let (rtp_start, rtp_end) = free_port_block(64);
        let tls = transport == "tls";
        let tls_files = tls.then(|| self_signed(&dir));
        let mut config = serde_json::json!({
            "run_dir": run_dir,
            "transport": transport,
            "bind": format!("127.0.0.1:{sip_port}"),
            "advertise_ip": "127.0.0.1",
            "media": media,
            "rtp_port_range": [rtp_start, rtp_end],
            "metrics_listen": "127.0.0.1:0",
            "stats_interval_ms": 200,
            "drain_timeout_secs": 20,
            "worker_threads": 2,
            "tuning": {"media_lanes": 2, "queue_capacity": 2000},
        });
        if let Some((cert, key)) = &tls_files {
            config["tls_cert"] = serde_json::json!(cert);
            config["tls_key"] = serde_json::json!(key);
        }
        if !extra.is_empty() {
            let extra: Value = serde_json::from_str(extra).expect("extra is a JSON object");
            for (key, value) in extra.as_object().expect("extra is a JSON object") {
                config[key] = value.clone();
            }
        }
        let config = config.to_string();
        let config_path = dir.join("dut.json");
        std::fs::write(&config_path, config).unwrap();
        let log = std::fs::File::create(dir.join("dut.log")).unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_rvoip-bench-uas"))
            .arg("--config")
            .arg(&config_path)
            .env(
                "RUST_LOG",
                std::env::var("BENCH_UAS_TEST_LOG").unwrap_or_else(|_| "warn".into()),
            )
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .expect("spawn rvoip-bench-uas");
        let mut dut = Self {
            child: Some(child),
            run_dir,
            sip_port,
            tls,
            tls_files,
            ready: Value::Null,
        };
        let ready_path = dut.run_dir.join("ready.json");
        let deadline = Instant::now() + Duration::from_secs(30);
        while !ready_path.exists() {
            if let Some(status) = dut.child.as_mut().unwrap().try_wait().unwrap() {
                panic!("DUT exited before ready ({status}): {}", dut.log());
            }
            assert!(Instant::now() < deadline, "DUT not ready: {}", dut.log());
            std::thread::sleep(Duration::from_millis(20));
        }
        dut.ready = serde_json::from_slice(&std::fs::read(&ready_path).unwrap()).unwrap();
        dut
    }

    pub fn log(&self) -> String {
        std::fs::read_to_string(self.run_dir.parent().unwrap().join("dut.log")).unwrap_or_default()
    }

    pub fn sip_addr(&self) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], self.sip_port))
    }

    /// The SIP target for rvoip's UAC.
    pub fn target(&self) -> String {
        if self.tls {
            format!("sips:rvoip-bench-uas@127.0.0.1:{}", self.sip_port)
        } else {
            format!("sip:rvoip-bench-uas@127.0.0.1:{}", self.sip_port)
        }
    }

    /// `GET /metrics`.
    pub fn metrics(&self) -> String {
        use std::io::{Read, Write};
        let address: SocketAddr = self.ready["metrics_listen"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        let mut stream = std::net::TcpStream::connect(address).unwrap();
        stream
            .write_all(b"GET /metrics HTTP/1.1\r\nHost: dut\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        response
    }

    /// One metric sample from `/metrics` (`name` without the prefix, with
    /// labels if any).
    pub fn metric(&self, name: &str) -> f64 {
        let text = self.metrics();
        let wanted = format!("rvoip_bench_uas_{name} ");
        text.lines()
            .find_map(|line| line.strip_prefix(&wanted))
            .unwrap_or_else(|| panic!("metric {name} missing:\n{text}"))
            .parse()
            .unwrap()
    }

    /// Create the finish file.
    pub fn finish(&self) {
        std::fs::write(self.run_dir.join("finish"), b"").unwrap();
    }

    /// Wait for the process to exit; returns its exit code.
    pub fn wait_exit(&mut self, timeout: Duration) -> i32 {
        let deadline = Instant::now() + timeout;
        let child = self.child.as_mut().expect("still running");
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                self.child = None;
                return status.code().unwrap_or(-1);
            }
            assert!(
                Instant::now() < deadline,
                "DUT did not exit: {}",
                self.log()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn summary(&self) -> Value {
        serde_json::from_slice(&std::fs::read(self.run_dir.join("summary.json")).unwrap()).unwrap()
    }

    pub fn stats_lines(&self) -> Vec<Value> {
        std::fs::read_to_string(self.run_dir.join("stats.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

/// An rvoip UAC for the DUT's transport and media.
pub async fn uac(dut: &Dut, media: &str) -> StreamPeer {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init();
    let port = free_port();
    let mut config = Config::local("bench-caller", port);
    let (start, end) = free_port_block(32);
    config = config.with_media_ports(start, end);
    match media {
        "none" => config = config.with_signaling_only_media(9),
        // Signaling only, offering SDES-SRTP (as the benchmark caller does on TLS rows).
        "none-srtp" => {
            config = config.with_signaling_only_media(9);
            config.offer_srtp = true;
            config.srtp_required = true;
        }
        "srtp" => {
            config.offer_srtp = true;
            config.srtp_required = true;
        }
        _ => {}
    }
    if let Some((cert, key)) = &dut.tls_files {
        // As rvoip's own TLS tests: a TLS listener of its own (sharing the
        // DUT's self-signed certificate) and a `sips:` Contact, trusting the
        // DUT's certificate without verification.
        let tls_port = free_port();
        config.sip_tls_mode = SipTlsMode::ClientAndServer;
        config.tls_bind_addr = Some(SocketAddr::from(([127, 0, 0, 1], tls_port)));
        config.tls_cert_path = Some(cert.clone());
        config.tls_key_path = Some(key.clone());
        config.contact_uri = Some(format!(
            "sips:bench-caller@127.0.0.1:{tls_port};transport=tls"
        ));
        config.tls_insecure_skip_verify = true;
    }
    StreamPeer::with_config(config).await.expect("UAC starts")
}

/// Place a call and wait for the answer.
pub async fn call(peer: &mut StreamPeer, dut: &Dut) -> SessionHandle {
    let call_id = peer
        .invite(dut.target())
        .send()
        .await
        .unwrap_or_else(|error| panic!("INVITE not sent: {error}"));
    tokio::time::timeout(Duration::from_secs(10), peer.wait_for_answered(&call_id))
        .await
        .expect("answer within 10 s")
        .expect("200 OK")
}

fn tone(frame: usize) -> Vec<i16> {
    (0..FRAME_SAMPLES)
        .map(|sample| {
            let t = (frame * FRAME_SAMPLES + sample) as f32 / SAMPLE_RATE as f32;
            (0.3 * (2.0 * std::f32::consts::PI * TONE_HZ * t).sin() * 32_767.0) as i16
        })
        .collect()
}

/// Send `frames` 20 ms frames of a 440 Hz tone and collect what comes back.
pub async fn stream_tone(handle: &SessionHandle, frames: usize) -> Vec<i16> {
    let audio = handle.audio().await.expect("audio stream");
    let (sender, mut receiver) = audio.split();
    let collected = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let drain = tokio::spawn({
        let collected = std::sync::Arc::clone(&collected);
        async move {
            while let Some(frame) = receiver.recv().await {
                collected.lock().unwrap().extend_from_slice(&frame.samples);
            }
        }
    });
    let mut ticker = tokio::time::interval(Duration::from_millis(20));
    for index in 0..frames {
        ticker.tick().await;
        let frame = AudioFrame::new(tone(index), SAMPLE_RATE, 1, (index * FRAME_SAMPLES) as u32);
        if sender.send(frame).await.is_err() {
            break;
        }
    }
    tokio::time::sleep(Duration::from_millis(400)).await;
    drain.abort();
    let samples = collected.lock().unwrap().clone();
    samples
}

/// Goertzel magnitude of `samples` at `hz`.
pub fn goertzel(samples: &[i16], hz: f32) -> f32 {
    let n = samples.len() as f32;
    let k = (0.5 + n * hz / SAMPLE_RATE as f32).floor();
    let coeff = 2.0 * (2.0 * std::f32::consts::PI * k / n).cos();
    let (mut q1, mut q2) = (0.0f32, 0.0f32);
    for &sample in samples {
        let q0 = coeff * q1 - q2 + f32::from(sample);
        q2 = q1;
        q1 = q0;
    }
    (q1 * q1 + q2 * q2 - q1 * q2 * coeff).sqrt()
}

/// A hand-rolled UDP UAC, for checking responses on the wire.
pub struct RawUac {
    socket: tokio::net::UdpSocket,
    local: SocketAddr,
    target: SocketAddr,
}

/// What a raw INVITE got.
#[derive(Debug)]
pub enum RawOutcome {
    /// 200 OK; the call was ACKed and ended with BYE, which got `bye_status`.
    Answered { bye_status: u16 },
    /// A final non-2xx response (ACKed).
    Rejected {
        status: u16,
        retry_after: Option<u32>,
    },
}

fn header<'a>(message: &'a str, name: &str) -> Option<&'a str> {
    message.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then_some(value.trim())
    })
}

fn status(message: &str) -> u16 {
    message
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0)
}

impl RawUac {
    pub async fn new(target: SocketAddr) -> Self {
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let local = socket.local_addr().unwrap();
        Self {
            socket,
            local,
            target,
        }
    }

    async fn send(&self, message: String) {
        self.socket
            .send_to(message.as_bytes(), self.target)
            .await
            .unwrap();
    }

    async fn recv_final(&self, call_id: &str, method: &str) -> String {
        tokio::time::timeout(Duration::from_secs(10), async {
            let mut buffer = vec![0u8; 8192];
            loop {
                let (count, _) = self.socket.recv_from(&mut buffer).await.unwrap();
                let text = String::from_utf8_lossy(&buffer[..count]).into_owned();
                if !text.starts_with("SIP/2.0 ") || status(&text) < 200 {
                    continue;
                }
                let matches_call = header(&text, "Call-ID") == Some(call_id);
                let matches_method =
                    header(&text, "CSeq").is_some_and(|cseq| cseq.ends_with(method));
                if matches_call && matches_method {
                    return text;
                }
            }
        })
        .await
        .expect("final response within 10 s")
    }

    /// INVITE (with a signaling-only SDP offer); ACK the final response and,
    /// if answered, BYE.
    pub async fn call(&self, call_id: &str) -> RawOutcome {
        let target = self.target;
        let local = self.local;
        let sdp = format!(
            "v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\n\
             m=audio 9 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n"
        );
        let branch = format!("z9hG4bK-{call_id}");
        self.send(format!(
            "INVITE sip:rvoip-bench-uas@{target} SIP/2.0\r\n\
             Via: SIP/2.0/UDP {local};branch={branch}\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:raw@{local}>;tag=raw-{call_id}\r\n\
             To: <sip:rvoip-bench-uas@{target}>\r\n\
             Contact: <sip:raw@{local}>\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Content-Type: application/sdp\r\n\
             Content-Length: {}\r\n\r\n{sdp}",
            sdp.len()
        ))
        .await;
        let response = self.recv_final(call_id, "INVITE").await;
        let code = status(&response);
        let to = header(&response, "To").expect("To").to_string();
        if code >= 300 {
            self.send(format!(
                "ACK sip:rvoip-bench-uas@{target} SIP/2.0\r\n\
                 Via: SIP/2.0/UDP {local};branch={branch}\r\n\
                 Max-Forwards: 70\r\n\
                 From: <sip:raw@{local}>;tag=raw-{call_id}\r\n\
                 To: {to}\r\n\
                 Call-ID: {call_id}\r\n\
                 CSeq: 1 ACK\r\n\
                 Content-Length: 0\r\n\r\n"
            ))
            .await;
            let retry_after = header(&response, "Retry-After")
                .and_then(|value| value.split_whitespace().next()?.parse().ok());
            return RawOutcome::Rejected {
                status: code,
                retry_after,
            };
        }
        let contact = header(&response, "Contact")
            .map(|contact| {
                contact
                    .trim_start_matches('<')
                    .split('>')
                    .next()
                    .unwrap()
                    .to_string()
            })
            .unwrap_or_else(|| format!("sip:rvoip-bench-uas@{target}"));
        for (method, cseq, suffix) in [("ACK", 1, "ack"), ("BYE", 2, "bye")] {
            self.send(format!(
                "{method} {contact} SIP/2.0\r\n\
                 Via: SIP/2.0/UDP {local};branch=z9hG4bK-{call_id}-{suffix}\r\n\
                 Max-Forwards: 70\r\n\
                 From: <sip:raw@{local}>;tag=raw-{call_id}\r\n\
                 To: {to}\r\n\
                 Call-ID: {call_id}\r\n\
                 CSeq: {cseq} {method}\r\n\
                 Content-Length: 0\r\n\r\n"
            ))
            .await;
        }
        let bye = self.recv_final(call_id, "BYE").await;
        RawOutcome::Answered {
            bye_status: status(&bye),
        }
    }
}
