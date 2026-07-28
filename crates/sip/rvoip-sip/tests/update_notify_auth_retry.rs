//! RFC 3261 auth retry coverage for in-dialog requests.
//!
//! A 401 challenge must be answered with `Authorization`; a 407 challenge
//! must be answered with `Proxy-Authorization`. The retry must preserve the
//! method-shaped options staged by the public builders.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};

use rvoip_sip::api::headers::SipRequestOptions;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::types::Credentials;
use rvoip_sip::CallState;

use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::{HeaderAccess, HeaderValue};

use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

const REALM: &str = "testrealm";
const TRACE_HEADER_NAME: &str = "X-Trace";
const BYE_TRACE_VALUE: &str = "trace-bye-proxy-auth";
const REFER_TRACE_VALUE: &str = "trace-refer-proxy-auth";
const UPDATE_TRACE_VALUE: &str = "trace-update-auth";
const NOTIFY_TRACE_VALUE: &str = "trace-notify-auth";
const INFO_TRACE_VALUE: &str = "trace-info-auth-int";
const INFO_BODY: &[u8] = b"binary-info:\xff\x00\xfe";
const REFER_TARGET: &str = "sip:transfer-target@127.0.0.1:36188";
const UPDATE_SDP: &str = "v=0\r\n\
o=alice 0 1 IN IP4 127.0.0.1\r\n\
s=-\r\n\
c=IN IP4 127.0.0.1\r\n\
t=0 0\r\n\
m=audio 40000 RTP/AVP 0\r\n";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChallengedMethod {
    Bye,
    Refer,
    Update,
    Notify,
    Info,
}

impl ChallengedMethod {
    fn sip_method(self) -> Method {
        match self {
            Self::Bye => Method::Bye,
            Self::Refer => Method::Refer,
            Self::Update => Method::Update,
            Self::Notify => Method::Notify,
            Self::Info => Method::Info,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Bye => "BYE",
            Self::Refer => "REFER",
            Self::Update => "UPDATE",
            Self::Notify => "NOTIFY",
            Self::Info => "INFO",
        }
    }

    fn nonce(self, kind: ChallengeKind) -> &'static str {
        match (self, kind) {
            (Self::Bye, ChallengeKind::Origin401) => "bye-401-nonce",
            (Self::Bye, ChallengeKind::Proxy407) => "bye-407-nonce",
            (Self::Refer, ChallengeKind::Origin401) => "refer-401-nonce",
            (Self::Refer, ChallengeKind::Proxy407) => "refer-407-nonce",
            (Self::Update, ChallengeKind::Origin401) => "update-401-nonce",
            (Self::Update, ChallengeKind::Proxy407) => "update-407-nonce",
            (Self::Notify, ChallengeKind::Origin401) => "notify-401-nonce",
            (Self::Notify, ChallengeKind::Proxy407) => "notify-407-nonce",
            (Self::Info, ChallengeKind::Origin401) => "info-401-nonce",
            (Self::Info, ChallengeKind::Proxy407) => "info-407-nonce",
            (Self::Bye, ChallengeKind::Origin401AuthInt) => "bye-auth-int-nonce",
            (Self::Refer, ChallengeKind::Origin401AuthInt) => "refer-auth-int-nonce",
            (Self::Update, ChallengeKind::Origin401AuthInt) => "update-auth-int-nonce",
            (Self::Notify, ChallengeKind::Origin401AuthInt) => "notify-auth-int-nonce",
            (Self::Info, ChallengeKind::Origin401AuthInt) => "info-auth-int-nonce",
        }
    }

    fn auth_int_body(self) -> &'static [u8] {
        match self {
            Self::Refer | Self::Bye => b"",
            Self::Update => UPDATE_SDP.as_bytes(),
            Self::Notify => b"<presence/>",
            Self::Info => INFO_BODY,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChallengeKind {
    Origin401,
    Proxy407,
    Origin401AuthInt,
}

#[derive(Debug, Clone)]
struct MidDialogCapture {
    raw: String,
    body: Vec<u8>,
    cseq: u32,
    trace_value: Option<String>,
    authorization: Option<String>,
    proxy_authorization: Option<String>,
}

struct RawAuthUas {
    captures: Arc<Mutex<Vec<MidDialogCapture>>>,
    count: Arc<AtomicU32>,
    task: JoinHandle<()>,
}

impl RawAuthUas {
    async fn wait_for_two(&self) -> Vec<MidDialogCapture> {
        let observed = timeout(Duration::from_secs(8), async {
            loop {
                if self.count.load(Ordering::SeqCst) >= 2 {
                    return;
                }
                sleep(Duration::from_millis(40)).await;
            }
        })
        .await;
        assert!(
            observed.is_ok(),
            "UAS never saw two challenged requests (count={})",
            self.count.load(Ordering::SeqCst)
        );
        sleep(Duration::from_millis(150)).await;
        self.captures.lock().await.clone()
    }

    fn shutdown(self) {
        self.task.abort();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bye_407_retry_uses_proxy_authorization() {
    run_in_dialog_auth_retry(36188, 36189, ChallengedMethod::Bye, ChallengeKind::Proxy407).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refer_407_retry_uses_proxy_authorization() {
    run_in_dialog_auth_retry(
        36190,
        36191,
        ChallengedMethod::Refer,
        ChallengeKind::Proxy407,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn update_401_retry_uses_authorization() {
    run_in_dialog_auth_retry(
        36180,
        36181,
        ChallengedMethod::Update,
        ChallengeKind::Origin401,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn update_407_retry_uses_proxy_authorization() {
    run_in_dialog_auth_retry(
        36182,
        36183,
        ChallengedMethod::Update,
        ChallengeKind::Proxy407,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn notify_401_retry_uses_authorization() {
    run_in_dialog_auth_retry(
        36184,
        36185,
        ChallengedMethod::Notify,
        ChallengeKind::Origin401,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn notify_407_retry_uses_proxy_authorization() {
    run_in_dialog_auth_retry(
        36186,
        36187,
        ChallengedMethod::Notify,
        ChallengeKind::Proxy407,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refer_notify_info_and_update_auth_int_sign_exact_bodies() {
    for (uas_port, uac_port, method) in [
        (36200, 36201, ChallengedMethod::Refer),
        (36202, 36203, ChallengedMethod::Notify),
        (36204, 36205, ChallengedMethod::Info),
        (36206, 36207, ChallengedMethod::Update),
    ] {
        run_in_dialog_auth_retry(uas_port, uac_port, method, ChallengeKind::Origin401AuthInt).await;
    }
}

async fn run_in_dialog_auth_retry(
    uas_port: u16,
    uac_port: u16,
    method: ChallengedMethod,
    challenge_kind: ChallengeKind,
) {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();

    let uas = spawn_raw_auth_uas(uas_port, method, challenge_kind).await;
    let coord = UnifiedCoordinator::new(Config::local("alice", uac_port))
        .await
        .expect("UAC coordinator");
    sleep(Duration::from_millis(150)).await;

    let target = format!("sip:bob@127.0.0.1:{uas_port}");
    let call_id = coord
        .invite(Some(format!("sip:alice@127.0.0.1:{uac_port}")), target)
        .with_credentials(Credentials::new("alice", "password").with_realm(REALM))
        .send()
        .await
        .expect("invite.send()");

    wait_until_active(&coord, &call_id).await;

    match method {
        ChallengedMethod::Bye => {
            coord
                .bye(&call_id)
                .with_raw_header(
                    HeaderName::Other(TRACE_HEADER_NAME.to_string()),
                    BYE_TRACE_VALUE,
                )
                .expect("X-Trace on BYE")
                .send()
                .await
                .expect("bye.send()");
        }
        ChallengedMethod::Refer => {
            coord
                .refer(&call_id, REFER_TARGET)
                .with_raw_header(
                    HeaderName::Other(TRACE_HEADER_NAME.to_string()),
                    REFER_TRACE_VALUE,
                )
                .expect("X-Trace on REFER")
                .send()
                .await
                .expect("refer.send()");
        }
        ChallengedMethod::Update => {
            coord
                .update(&call_id)
                .with_sdp(UPDATE_SDP)
                .with_raw_header(
                    HeaderName::Other(TRACE_HEADER_NAME.to_string()),
                    UPDATE_TRACE_VALUE,
                )
                .expect("X-Trace on UPDATE")
                .send()
                .await
                .expect("update.send()");
        }
        ChallengedMethod::Notify => {
            coord
                .notify(&call_id, "presence")
                .with_subscription_state("active;expires=3600")
                .with_content_type("application/pidf+xml")
                .with_body("<presence/>")
                .with_raw_header(
                    HeaderName::Other(TRACE_HEADER_NAME.to_string()),
                    NOTIFY_TRACE_VALUE,
                )
                .expect("X-Trace on NOTIFY")
                .send()
                .await
                .expect("notify.send()");
        }
        ChallengedMethod::Info => {
            coord
                .info(&call_id, "application/dtmf-relay")
                .with_body(bytes::Bytes::from_static(INFO_BODY))
                .with_raw_header(
                    HeaderName::Other(TRACE_HEADER_NAME.to_string()),
                    INFO_TRACE_VALUE,
                )
                .expect("X-Trace on INFO")
                .send()
                .await
                .expect("info.send()");
        }
    }

    let captures = uas.wait_for_two().await;
    assert_eq!(
        captures.len(),
        2,
        "expected initial {} + authenticated retry",
        method.as_str()
    );

    assert_initial_request(&captures[0], method);
    assert!(
        captures[1].cseq > captures[0].cseq,
        "RFC 3261 §22.2 retry must increment CSeq for {}: initial={}, retry={}",
        method.as_str(),
        captures[0].cseq,
        captures[1].cseq
    );
    assert_retry_request(&captures[1], method, challenge_kind, uas_port);

    uas.shutdown();
}

async fn wait_until_active(coord: &Arc<UnifiedCoordinator>, call_id: &rvoip_sip::CallId) {
    let active = timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(state) = coord.get_state(call_id).await {
                if state == CallState::Active {
                    return true;
                }
            }
            sleep(Duration::from_millis(40)).await;
        }
    })
    .await;
    assert!(matches!(active, Ok(true)), "call never reached Active");
}

async fn spawn_raw_auth_uas(
    port: u16,
    challenged_method: ChallengedMethod,
    challenge_kind: ChallengeKind,
) -> RawAuthUas {
    let addr = format!("127.0.0.1:{port}");
    let sock = Arc::new(UdpSocket::bind(&addr).await.expect("UAS bind"));
    let captures = Arc::new(Mutex::new(Vec::new()));
    let count = Arc::new(AtomicU32::new(0));

    let sock_task = sock.clone();
    let captures_task = captures.clone();
    let count_task = count.clone();
    let task = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            let (n, from) = match sock_task.recv_from(&mut buf).await {
                Ok(p) => p,
                Err(_) => return,
            };
            let bytes = &buf[..n];
            let parsed = match parse_message(bytes) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let request = match parsed {
                Message::Request(r) => r,
                _ => continue,
            };

            if request.method() == Method::Invite {
                let mut resp = create_response(&request, StatusCode::Ok);
                for header in resp.headers.iter_mut() {
                    if let TypedHeader::To(to) = header {
                        if to.tag().is_none() {
                            to.set_tag("uas-auth-tag");
                        }
                        break;
                    }
                }
                resp.headers.push(TypedHeader::Other(
                    HeaderName::Contact,
                    HeaderValue::Raw(format!("<sip:bob@127.0.0.1:{port}>").into_bytes()),
                ));
                let _ = sock_task
                    .send_to(&Message::Response(resp).to_bytes(), from)
                    .await;
                continue;
            }

            if request.method() == Method::Ack {
                continue;
            }

            if request.method() == challenged_method.sip_method() {
                let idx = count_task.fetch_add(1, Ordering::SeqCst);
                captures_task.lock().await.push(MidDialogCapture {
                    raw: String::from_utf8_lossy(bytes).into_owned(),
                    body: request.body().to_vec(),
                    cseq: request.cseq().map(|cseq| cseq.sequence()).unwrap_or(0),
                    trace_value: request
                        .raw_header_value(&HeaderName::Other(TRACE_HEADER_NAME.to_string())),
                    authorization: request.raw_header_value(&HeaderName::Authorization),
                    proxy_authorization: request.raw_header_value(&HeaderName::ProxyAuthorization),
                });

                let resp = if idx == 0 {
                    build_challenge_response(
                        &request,
                        challenged_method.nonce(challenge_kind),
                        challenge_kind,
                    )
                } else {
                    Message::Response(create_response(&request, StatusCode::Ok))
                };
                let _ = sock_task.send_to(&resp.to_bytes(), from).await;
                continue;
            }

            let resp = create_response(&request, StatusCode::Ok);
            let _ = sock_task
                .send_to(&Message::Response(resp).to_bytes(), from)
                .await;
        }
    });

    RawAuthUas {
        captures,
        count,
        task,
    }
}

fn build_challenge_response(request: &Request, nonce: &str, kind: ChallengeKind) -> Message {
    let qop = if kind == ChallengeKind::Origin401AuthInt {
        "auth-int"
    } else {
        "auth"
    };
    let challenge =
        format!(r#"Digest realm="{REALM}", nonce="{nonce}", algorithm=MD5, qop="{qop}""#);
    match kind {
        ChallengeKind::Origin401 | ChallengeKind::Origin401AuthInt => {
            let mut resp = create_response(request, StatusCode::Unauthorized);
            resp.headers.push(TypedHeader::Other(
                HeaderName::WwwAuthenticate,
                HeaderValue::Raw(challenge.into_bytes()),
            ));
            Message::Response(resp)
        }
        ChallengeKind::Proxy407 => {
            let mut resp = create_response(request, StatusCode::ProxyAuthenticationRequired);
            resp.headers.push(TypedHeader::Other(
                HeaderName::ProxyAuthenticate,
                HeaderValue::Raw(challenge.into_bytes()),
            ));
            Message::Response(resp)
        }
    }
}

fn assert_initial_request(capture: &MidDialogCapture, method: ChallengedMethod) {
    assert!(
        capture.authorization.is_none(),
        "initial {} must not carry Authorization: {:?}",
        method.as_str(),
        capture.authorization
    );
    assert!(
        capture.proxy_authorization.is_none(),
        "initial {} must not carry Proxy-Authorization: {:?}",
        method.as_str(),
        capture.proxy_authorization
    );
    assert_method_fields_survive(capture, method);
}

fn assert_retry_request(
    capture: &MidDialogCapture,
    method: ChallengedMethod,
    challenge_kind: ChallengeKind,
    uas_port: u16,
) {
    match challenge_kind {
        ChallengeKind::Origin401 | ChallengeKind::Origin401AuthInt => {
            let auth = capture
                .authorization
                .as_deref()
                .expect("401 retry must carry Authorization");
            assert!(
                capture.proxy_authorization.is_none(),
                "401 retry must not carry Proxy-Authorization: {:?}",
                capture.proxy_authorization
            );
            assert_digest_header(auth, method, challenge_kind, uas_port);
        }
        ChallengeKind::Proxy407 => {
            let proxy_auth = capture
                .proxy_authorization
                .as_deref()
                .expect("407 retry must carry Proxy-Authorization");
            assert!(
                capture.authorization.is_none(),
                "407 retry must not carry Authorization: {:?}",
                capture.authorization
            );
            assert_digest_header(proxy_auth, method, challenge_kind, uas_port);
        }
    }
    assert_method_fields_survive(capture, method);
}

fn assert_digest_header(
    value: &str,
    method: ChallengedMethod,
    challenge_kind: ChallengeKind,
    uas_port: u16,
) {
    assert!(
        value.starts_with("Digest "),
        "retry {} auth header must be a full Digest response: {value}",
        method.as_str()
    );
    assert!(
        value.contains(r#"username="alice""#)
            && value.contains(r#"realm="testrealm""#)
            && value.contains("response=")
            && value.contains(if challenge_kind == ChallengeKind::Origin401AuthInt {
                r#"qop=auth-int"#
            } else {
                r#"qop=auth"#
            }),
        "retry {} auth header is incomplete: {value}",
        method.as_str()
    );
    assert!(
        value.contains(&format!(r#"uri="sip:bob@127.0.0.1:{uas_port}""#)),
        "retry {} Digest URI must use the in-dialog remote target: {value}",
        method.as_str()
    );
    if challenge_kind == ChallengeKind::Origin401AuthInt {
        let parsed = rvoip_sip::auth::DigestAuthenticator::parse_authorization(value)
            .expect("parse tracked auth-int Authorization");
        assert_eq!(parsed.qop.as_deref(), Some("auth-int"));
        assert!(
            rvoip_sip::auth::DigestAuthenticator::new(REALM)
                .validate_response_with_body(
                    &parsed,
                    method.as_str(),
                    "password",
                    Some(method.auth_int_body()),
                )
                .expect("validate tracked auth-int response"),
            "{} auth-int response must validate against the exact body",
            method.as_str()
        );
    }
}

fn assert_method_fields_survive(capture: &MidDialogCapture, method: ChallengedMethod) {
    match method {
        ChallengedMethod::Bye => {
            assert_eq!(capture.trace_value.as_deref(), Some(BYE_TRACE_VALUE));
        }
        ChallengedMethod::Refer => {
            assert_eq!(capture.trace_value.as_deref(), Some(REFER_TRACE_VALUE));
            assert!(
                capture.raw.contains(REFER_TARGET),
                "REFER retry must preserve Refer-To header; got:\n{}",
                capture.raw
            );
        }
        ChallengedMethod::Update => {
            assert_eq!(capture.trace_value.as_deref(), Some(UPDATE_TRACE_VALUE));
            assert!(
                capture.raw.contains("m=audio 40000 RTP/AVP 0"),
                "UPDATE retry must preserve SDP body; got:\n{}",
                capture.raw
            );
        }
        ChallengedMethod::Notify => {
            assert_eq!(capture.trace_value.as_deref(), Some(NOTIFY_TRACE_VALUE));
            assert!(
                capture.raw.contains("Event: presence"),
                "NOTIFY retry must preserve Event header; got:\n{}",
                capture.raw
            );
            assert!(
                capture
                    .raw
                    .contains("Subscription-State: active;expires=3600"),
                "NOTIFY retry must preserve Subscription-State header; got:\n{}",
                capture.raw
            );
            assert!(
                capture.raw.contains("<presence/>"),
                "NOTIFY retry must preserve body; got:\n{}",
                capture.raw
            );
        }
        ChallengedMethod::Info => {
            assert_eq!(capture.trace_value.as_deref(), Some(INFO_TRACE_VALUE));
            assert_eq!(
                capture.body, INFO_BODY,
                "INFO retry must preserve non-UTF8 body bytes"
            );
        }
    }
}
