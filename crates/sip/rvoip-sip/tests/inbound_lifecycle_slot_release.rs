//! A finished inbound call must return its lifecycle capacity as soon as its
//! resources are released, not after the session anti-reuse horizon.
//!
//! The UAS runs with `server_retained_lifecycle_capacity = RETAINED`. Calls
//! are strictly sequential, so active capacity is never the limit. Two kinds
//! of per-call state outlive a hold-0 call (INVITE, 200, ACK, immediate BYE):
//!
//! * RFC 3261/6026 transaction state: the INVITE server transaction stays in
//!   Accepted for Timer L (64*T1 = 32 s) and, over UDP, the BYE server
//!   transaction stays Completed for Timer J (64*T1). The stack bounds these
//!   reservations by the same configured retention capacity, so at most
//!   `RETAINED` INVITEs fit in any 32 s window. That is expected.
//! * The session anti-reuse fence (64 s). It must not hold retained capacity
//!   once the call is released.
//!
//! Each scenario therefore places a burst of `BURST` back-to-back calls,
//! waits for Timer L/J to release the transaction reservations (but well
//! inside the 64 s fence horizon), then places a second burst. The two bursts
//! together exceed `RETAINED`: with fences still counted the second burst is
//! rejected with 503 for the rest of the horizon; with fences reclaimed every
//! call is admitted. Within a burst it measures how long after the BYE's
//! 200 OK the next INVITE is admitted.
//!
//! A retransmitted BYE is still absorbed by its server transaction and
//! answered with the same 200 OK while the next call is already admitted.

use std::time::{Duration, Instant};

use rvoip_sip::api::callback_peer::{CallHandler, CallHandlerDecision, CallbackPeer};
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::api::unified::Config;
use tokio::net::UdpSocket;

/// Concurrent calls the UAS admits.
const ACTIVE: usize = 4;
/// Retained lifecycle records (live calls plus anti-reuse fences), and with
/// it the Timer L / Timer J retention bound.
const RETAINED: usize = 8;
/// Calls per burst: two bursts exceed `RETAINED`, one leaves Timer L room for
/// a few INVITEs retried while a slot is being released.
const BURST: usize = 5;
const _: () = assert!(2 * BURST > RETAINED && BURST < RETAINED && ACTIVE <= RETAINED);
/// Timer L and Timer J (64*T1, T1 = 500 ms) plus scheduling margin. Shorter
/// than the 64 s anti-reuse horizon, so a fence that still counted would
/// reject the whole second burst.
const TRANSACTION_RETENTION_DRAIN: Duration = Duration::from_secs(34);
/// Median delay from the BYE's 200 OK to the next INVITE's 200 OK.
const MEDIAN_SLOT_RELEASE_BOUND: Duration = Duration::from_millis(100);
/// Worst single admission delay tolerated on a loaded test host.
const MAX_SLOT_RELEASE_BOUND: Duration = Duration::from_secs(2);
/// A hold-0 caller sends BYE right after the ACK; leave the UAS a moment to
/// confirm the dialog so the measurement is not about the ACK/BYE race.
const ACK_TO_BYE: Duration = Duration::from_millis(20);
const RETRY_INTERVAL: Duration = Duration::from_millis(2);

struct AcceptAll;

#[async_trait::async_trait]
impl CallHandler for AcceptAll {
    async fn on_incoming_call(&self, _call: IncomingCall) -> CallHandlerDecision {
        CallHandlerDecision::Accept
    }
}

fn uas_config(name: &str, port: u16, media_start: u16, retained: usize) -> Config {
    let mut config = Config::local(name, port)
        .with_server_capacity(ACTIVE)
        .with_server_retained_lifecycle_capacity(retained);
    config.media_port_start = media_start;
    config.media_port_end = media_start + 200;
    config
}

fn median(samples: &mut [Duration]) -> Duration {
    samples.sort();
    samples[samples.len() / 2]
}

fn assert_slot_release(label: &str, mut samples: Vec<Duration>) {
    let max = samples.iter().copied().max().unwrap_or_default();
    let median = median(&mut samples);
    eprintln!(
        "{label}: BYE 200 OK -> next INVITE 200 OK median={median:?} max={max:?} n={}",
        samples.len()
    );
    assert!(
        median < MEDIAN_SLOT_RELEASE_BOUND,
        "{label}: median slot release {median:?} exceeds {MEDIAN_SLOT_RELEASE_BOUND:?}"
    );
    assert!(
        max < MAX_SLOT_RELEASE_BOUND,
        "{label}: slowest slot release {max:?} exceeds {MAX_SLOT_RELEASE_BOUND:?}"
    );
}

// ---------------------------------------------------------------------------
// Raw UDP UAC
// ---------------------------------------------------------------------------

struct RawUac {
    socket: UdpSocket,
    port: u16,
    uas_port: u16,
    sequence: u64,
}

struct Dialog {
    call_id: String,
    from_tag: String,
    to_header: String,
}

enum InviteOutcome {
    Answered(Dialog),
    Rejected(u16, String),
}

fn header<'a>(message: &'a str, name: &str) -> Option<&'a str> {
    message.split("\r\n").find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then_some(value.trim())
    })
}

fn status_code(message: &str) -> Option<u16> {
    message
        .strip_prefix("SIP/2.0 ")?
        .get(..3)?
        .parse()
        .ok()
}

fn cseq_method(message: &str) -> Option<&str> {
    header(message, "CSeq")?.split_whitespace().nth(1)
}

impl RawUac {
    async fn new(uas_port: u16) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").await.expect("raw UAC bind");
        let port = socket.local_addr().expect("raw UAC address").port();
        Self {
            socket,
            port,
            uas_port,
            sequence: 0,
        }
    }

    fn next(&mut self, prefix: &str) -> String {
        self.sequence += 1;
        format!("{prefix}-{}-{}", self.port, self.sequence)
    }

    async fn send(&self, message: &str) {
        self.socket
            .send_to(message.as_bytes(), ("127.0.0.1", self.uas_port))
            .await
            .expect("send raw SIP");
    }

    async fn recv_matching(
        &self,
        call_id: &str,
        method: &str,
        timeout: Duration,
    ) -> Option<String> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut buffer = vec![0_u8; 65_535];
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let (length, _) =
                tokio::time::timeout(remaining, self.socket.recv_from(&mut buffer))
                    .await
                    .ok()?
                    .expect("receive raw SIP");
            let message = String::from_utf8_lossy(&buffer[..length]).into_owned();
            if header(&message, "Call-ID") == Some(call_id)
                && cseq_method(&message) == Some(method)
                && status_code(&message).is_some_and(|code| code >= 200)
            {
                return Some(message);
            }
        }
    }

    fn sdp_offer(&self) -> String {
        let media_port = self.port.wrapping_add(2) | 1;
        format!(
            "v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\n\
             m=audio {media_port} RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n"
        )
    }

    async fn invite(&mut self) -> InviteOutcome {
        let call_id = self.next("slot-release-call");
        let from_tag = self.next("tag");
        let branch = format!("z9hG4bK-{}", self.next("invite"));
        let body = self.sdp_offer();
        let (port, uas) = (self.port, self.uas_port);
        let invite = format!(
            "INVITE sip:uas@127.0.0.1:{uas} SIP/2.0\r\n\
             Via: SIP/2.0/UDP 127.0.0.1:{port};branch={branch};rport\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:uac@127.0.0.1:{port}>;tag={from_tag}\r\n\
             To: <sip:uas@127.0.0.1:{uas}>\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Contact: <sip:uac@127.0.0.1:{port}>\r\n\
             Content-Type: application/sdp\r\n\
             Content-Length: {}\r\n\r\n{body}",
            body.len()
        );
        self.send(&invite).await;
        let response = self
            .recv_matching(&call_id, "INVITE", Duration::from_secs(8))
            .await
            .expect("final INVITE response");
        let code = status_code(&response).expect("status code");
        let to_header = header(&response, "To").expect("To header").to_string();
        if code == 200 {
            let ack_branch = format!("z9hG4bK-{}", self.next("ack"));
            let ack = format!(
                "ACK sip:uas@127.0.0.1:{uas} SIP/2.0\r\n\
                 Via: SIP/2.0/UDP 127.0.0.1:{port};branch={ack_branch};rport\r\n\
                 Max-Forwards: 70\r\n\
                 From: <sip:uac@127.0.0.1:{port}>;tag={from_tag}\r\n\
                 To: {to_header}\r\n\
                 Call-ID: {call_id}\r\n\
                 CSeq: 1 ACK\r\n\
                 Content-Length: 0\r\n\r\n"
            );
            self.send(&ack).await;
            InviteOutcome::Answered(Dialog {
                call_id,
                from_tag,
                to_header,
            })
        } else {
            // Non-2xx ACK belongs to the INVITE transaction (same branch).
            let ack = format!(
                "ACK sip:uas@127.0.0.1:{uas} SIP/2.0\r\n\
                 Via: SIP/2.0/UDP 127.0.0.1:{port};branch={branch};rport\r\n\
                 Max-Forwards: 70\r\n\
                 From: <sip:uac@127.0.0.1:{port}>;tag={from_tag}\r\n\
                 To: {to_header}\r\n\
                 Call-ID: {call_id}\r\n\
                 CSeq: 1 ACK\r\n\
                 Content-Length: 0\r\n\r\n"
            );
            self.send(&ack).await;
            InviteOutcome::Rejected(code, response)
        }
    }

    fn bye(&mut self, dialog: &Dialog, cseq: u32) -> String {
        let branch = format!("z9hG4bK-{}", self.next("bye"));
        let (port, uas) = (self.port, self.uas_port);
        format!(
            "BYE sip:uas@127.0.0.1:{uas} SIP/2.0\r\n\
             Via: SIP/2.0/UDP 127.0.0.1:{port};branch={branch};rport\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:uac@127.0.0.1:{port}>;tag={}\r\n\
             To: {}\r\n\
             Call-ID: {}\r\n\
             CSeq: {cseq} BYE\r\n\
             Content-Length: 0\r\n\r\n",
            dialog.from_tag, dialog.to_header, dialog.call_id
        )
    }

    /// End the call with a BYE and return that BYE and its 200 OK. A BYE that
    /// overtakes the ACK's confirmation can be answered 481; it is re-sent
    /// as a new request (new branch, next CSeq) until the dialog matches.
    async fn hang_up(&mut self, dialog: &Dialog) -> (String, String) {
        tokio::time::sleep(ACK_TO_BYE).await;
        for cseq in 2..52 {
            let bye = self.bye(dialog, cseq);
            self.send(&bye).await;
            let response = self
                .recv_matching(&dialog.call_id, "BYE", Duration::from_secs(8))
                .await
                .expect("BYE final response");
            match status_code(&response) {
                Some(200) => return (bye, response),
                Some(481) => tokio::time::sleep(RETRY_INTERVAL).await,
                _ => panic!("BYE answered:\n{response}"),
            }
        }
        panic!("BYE never matched the confirmed dialog");
    }

    /// Retry fresh INVITEs until one is answered. Returns the dialog and the
    /// delay from `since` until that INVITE's 200 OK arrived (admission
    /// complete, including the INVITE round trip and any 503 retries).
    async fn invite_until_admitted(&mut self, since: Instant) -> (Dialog, Duration) {
        let deadline = since + Duration::from_secs(10);
        let mut first_rejection = None;
        loop {
            let sent_after = since.elapsed();
            match self.invite().await {
                InviteOutcome::Answered(dialog) => return (dialog, since.elapsed()),
                InviteOutcome::Rejected(503, response) if Instant::now() < deadline => {
                    first_rejection.get_or_insert(response);
                    tokio::time::sleep(RETRY_INTERVAL).await;
                }
                InviteOutcome::Rejected(code, response) => panic!(
                    "INVITE not admitted {sent_after:?} after the previous BYE: {code}\n\
                     first rejection:\n{}\nlast:\n{response}",
                    first_rejection.unwrap_or_default()
                ),
            }
        }
    }
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_test_writer()
        .try_init();
}

async fn start_uas(
    config: Config,
) -> (
    rvoip_sip::api::callback_peer::ShutdownHandle,
    tokio::task::JoinHandle<()>,
) {
    init_tracing();
    let peer = CallbackPeer::new(AcceptAll, config)
        .await
        .expect("UAS CallbackPeer");
    let shutdown = peer.shutdown_handle();
    let task = tokio::spawn(async move {
        let _ = peer.run().await;
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    (shutdown, task)
}

impl RawUac {
    /// `calls` back-to-back hold-0 calls. The first INVITE must be admitted
    /// at once; each later one is measured from the previous BYE's 200 OK.
    async fn burst(&mut self, calls: usize, samples: &mut Vec<Duration>) {
        let started = Instant::now();
        let (mut dialog, first) = self.invite_until_admitted(started).await;
        assert!(
            first < MAX_SLOT_RELEASE_BOUND,
            "burst start not admitted for {first:?}"
        );
        for call in 1..=calls {
            self.hang_up(&dialog).await;
            if call == calls {
                break;
            }
            let bye_ok_at = Instant::now();
            let (next, delay) = self.invite_until_admitted(bye_ok_at).await;
            samples.push(delay);
            dialog = next;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn udp_hold_zero_calls_release_lifecycle_capacity_after_bye() {
    const UAS_PORT: u16 = 18_951;
    let (shutdown, task) =
        start_uas(uas_config("slot-release-udp", UAS_PORT, 29_500, RETAINED)).await;
    let mut uac = RawUac::new(UAS_PORT).await;

    let mut samples = Vec::new();
    uac.burst(BURST, &mut samples).await;
    // Transaction retention drains; anti-reuse fences would not.
    tokio::time::sleep(TRANSACTION_RETENTION_DRAIN).await;
    uac.burst(BURST, &mut samples).await;
    assert_slot_release("UDP", samples);

    shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn udp_retransmitted_bye_is_absorbed_while_the_slot_is_already_free() {
    const UAS_PORT: u16 = 18_953;
    // Room for the extra transactions this scenario opens; the tight
    // retention bound is exercised above.
    let (shutdown, task) = start_uas(uas_config(
        "slot-release-bye-retransmit",
        UAS_PORT,
        29_710,
        64,
    ))
    .await;
    let mut uac = RawUac::new(UAS_PORT).await;

    let (dialog, _) = uac.invite_until_admitted(Instant::now()).await;
    let (bye, first) = uac.hang_up(&dialog).await;
    let bye_ok_at = Instant::now();

    // A new call is admitted while the BYE server transaction (Timer J,
    // 64*T1 on UDP) still exists: transaction state does not hold the call.
    let (next, delay) = uac.invite_until_admitted(bye_ok_at).await;
    assert!(
        delay < MAX_SLOT_RELEASE_BOUND,
        "call not admitted while Timer J is running: {delay:?}"
    );

    // The retransmitted BYE (identical branch) is absorbed by that
    // transaction and answered with the same final response.
    uac.send(&bye).await;
    let retransmitted = uac
        .recv_matching(&dialog.call_id, "BYE", Duration::from_secs(2))
        .await
        .expect("retransmitted BYE is answered");
    assert_eq!(status_code(&retransmitted), Some(200));
    assert_eq!(header(&retransmitted, "To"), header(&first, "To"));
    assert_eq!(header(&retransmitted, "Via"), header(&first, "Via"));

    // A new BYE for the finished dialog is not misrouted to the live call:
    // it matches no dialog.
    let late_bye = uac.bye(&dialog, 1_000);
    uac.send(&late_bye).await;
    let late = uac
        .recv_matching(&dialog.call_id, "BYE", Duration::from_secs(2))
        .await
        .expect("late BYE is answered");
    assert_eq!(status_code(&late), Some(481), "late BYE:\n{late}");

    // The live call is untouched by the stale requests and ends normally.
    uac.hang_up(&next).await;
    let (_, delay) = uac.invite_until_admitted(Instant::now()).await;
    assert!(delay < MAX_SLOT_RELEASE_BOUND);

    shutdown.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
}

// ---------------------------------------------------------------------------
// TLS
// ---------------------------------------------------------------------------

#[cfg(feature = "dev-insecure-tls")]
mod tls {
    use super::*;
    use rvoip_sip::api::events::Event;
    use rvoip_sip::api::stream_peer::EventReceiver;
    use rvoip_sip::api::unified::{SipTlsMode, UnifiedCoordinator};
    use std::io::Write;
    use std::path::PathBuf;

    fn self_signed_localhost() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let cert_path = dir.path().join("server.crt");
        let key_path = dir.path().join("server.key");
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
            .expect("self-signed certificate");
        std::fs::File::create(&cert_path)
            .and_then(|mut file| file.write_all(cert.cert.pem().as_bytes()))
            .expect("write certificate");
        std::fs::File::create(&key_path)
            .and_then(|mut file| file.write_all(cert.signing_key.serialize_pem().as_bytes()))
            .expect("write key");
        (dir, cert_path, key_path)
    }

    fn with_tls(mut config: Config, port: u16, user: &str, cert: &PathBuf, key: &PathBuf) -> Config {
        let tls_port = port + 1;
        config.sip_tls_mode = SipTlsMode::ClientAndServer;
        config.tls_bind_addr = Some(format!("127.0.0.1:{tls_port}").parse().expect("TLS bind"));
        config.tls_cert_path = Some(cert.clone());
        config.tls_key_path = Some(key.clone());
        config.contact_uri = Some(format!("sips:{user}@127.0.0.1:{tls_port};transport=tls"));
        config.tls_insecure_skip_verify = true;
        config
    }

    /// Wait for the answer or failure of one exact outbound call.
    async fn answered(events: &mut EventReceiver, call: &rvoip_sip::api::events::CallId) -> Result<(), u16> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            match tokio::time::timeout(remaining, events.next()).await {
                Err(_) | Ok(None) => panic!("no outcome for TLS call"),
                Ok(Some(Event::CallAnswered { call_id, .. })) if &call_id == call => return Ok(()),
                Ok(Some(Event::CallFailed {
                    call_id,
                    status_code,
                    ..
                })) if &call_id == call => return Err(status_code),
                Ok(Some(_)) => {}
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn tls_hold_zero_calls_release_lifecycle_capacity_after_bye() {
        const UAC_PORT: u16 = 37_311;
        const UAS_PORT: u16 = 37_321;
        let (_dir, cert, key) = self_signed_localhost();

        let uas_config = with_tls(
            uas_config("slot-release-tls-uas", UAS_PORT, 29_920, RETAINED),
            UAS_PORT,
            "uas",
            &cert,
            &key,
        );
        let (shutdown, task) = start_uas(uas_config).await;

        let mut uac_config = with_tls(Config::local("slot-release-tls-uac", UAC_PORT), UAC_PORT, "uac", &cert, &key);
        uac_config.media_port_start = 30_200;
        uac_config.media_port_end = 30_400;
        let uac = UnifiedCoordinator::new(uac_config).await.expect("TLS UAC");
        let mut events = uac.events().await.expect("UAC events");
        let from = format!("sips:uac@127.0.0.1:{};transport=tls", UAC_PORT + 1);
        let target = format!("sips:uas@127.0.0.1:{}", UAS_PORT + 1);

        let mut samples = Vec::new();
        for burst in 0..2 {
            if burst > 0 {
                tokio::time::sleep(TRANSACTION_RETENTION_DRAIN).await;
            }
            let mut bye_ok_at: Option<Instant> = None;
            for _ in 0..BURST {
                let since = bye_ok_at.unwrap_or_else(Instant::now);
                let deadline = since + Duration::from_secs(10);
                let (session, sent_after) = loop {
                    let sent_after = since.elapsed();
                    let session = uac
                        .invite(Some(from.clone()), &target)
                        .send()
                        .await
                        .expect("TLS INVITE");
                    match answered(&mut events, &session).await {
                        Ok(()) => break (session, since.elapsed()),
                        Err(503) if Instant::now() < deadline => {
                            tokio::time::sleep(RETRY_INTERVAL).await;
                        }
                        Err(code) => {
                            panic!("TLS INVITE rejected {sent_after:?} after BYE: {code}")
                        }
                    }
                };
                match bye_ok_at {
                    Some(_) => samples.push(sent_after),
                    None => assert!(
                        sent_after < MAX_SLOT_RELEASE_BOUND,
                        "TLS burst {burst} start not admitted for {sent_after:?}"
                    ),
                }
                tokio::time::sleep(ACK_TO_BYE).await;
                // `hangup` returns once the peer's final BYE response arrived.
                uac.hangup(&session).await.expect("TLS BYE confirmed");
                bye_ok_at = Some(Instant::now());
            }
        }
        assert_slot_release("TLS", samples);

        shutdown.shutdown();
        let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
    }
}
