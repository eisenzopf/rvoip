//! Auth-challenge raw-UDP UAS for `auth_challenge_*` scenarios.
//!
//! Responds with the configured digest challenge (401 or 407) to the
//! Nth inbound request and accepts the retry. The shape mirrors
//! [`support::registrar`] / [`support::ringing_uas`] — a single tokio
//! task pulls from a `UdpSocket` and dispatches per captured method.
//!
//! This harness backs auth-retry assertions for challenged methods. In-dialog
//! requests use the state-machine `SendRequestWithAuth` path; out-of-dialog
//! MESSAGE / OPTIONS / SUBSCRIBE builders use coordinator-side direct retry
//! helpers and the same capture / reply pattern.

#![allow(dead_code)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};

use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::{HeaderAccess, HeaderValue};

use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

/// What to respond with on the Nth captured request (0-indexed).
pub enum ChallengeReply {
    /// 401 Unauthorized with a WWW-Authenticate digest challenge.
    Challenge401 { realm: String, nonce: String },
    /// 407 Proxy Authentication Required with Proxy-Authenticate digest.
    Challenge407 { realm: String, nonce: String },
    /// 401 Unauthorized with a configurable Digest challenge.
    Challenge401Full {
        realm: String,
        nonce: String,
        algorithm: String,
        qop: Option<String>,
        stale: bool,
    },
    /// 407 Proxy Authentication Required with a configurable Digest challenge.
    Challenge407Full {
        realm: String,
        nonce: String,
        algorithm: String,
        qop: Option<String>,
        stale: bool,
    },
    /// 401 Unauthorized with a raw WWW-Authenticate value.
    Challenge401Raw(String),
    /// 407 Proxy Authentication Required with a raw Proxy-Authenticate value.
    Challenge407Raw(String),
    /// 200 OK.
    Ok,
}

/// Capture record: method + raw bytes (so tests can grep for header
/// names like `Authorization:` to assert retry behavior).
#[derive(Clone, Debug)]
pub struct CapturedAuthRequest {
    pub method: String,
    pub cseq: u32,
    pub call_id: String,
    pub from_tag: Option<String>,
    pub to_header: Option<String>,
    pub via_header: Option<String>,
    pub raw: String,
}

pub struct AuthUas {
    pub addr: String,
    pub captured: Arc<Mutex<Vec<CapturedAuthRequest>>>,
    pub count: Arc<AtomicU32>,
    task: JoinHandle<()>,
}

impl AuthUas {
    pub fn shutdown(self) {
        self.task.abort();
    }

    pub async fn wait_for_n(&self, n: usize, deadline: Duration) -> Vec<CapturedAuthRequest> {
        let waited = timeout(deadline, async {
            loop {
                if self.count.load(Ordering::SeqCst) as usize >= n {
                    return;
                }
                sleep(Duration::from_millis(40)).await;
            }
        })
        .await;
        assert!(
            waited.is_ok(),
            "auth UAS never saw {} requests (count={})",
            n,
            self.count.load(Ordering::SeqCst)
        );
        sleep(Duration::from_millis(120)).await;
        self.captured.lock().await.clone()
    }
}

/// Boot a raw-UDP auth UAS on `127.0.0.1:port`. `reply_for(count)`
/// returns the challenge plan for the Nth captured request
/// (0-indexed).
pub async fn boot_auth_uas<F>(port: u16, reply_for: F) -> AuthUas
where
    F: Fn(u32) -> ChallengeReply + Send + Sync + 'static,
{
    let addr = format!("127.0.0.1:{port}");
    let sock = Arc::new(UdpSocket::bind(&addr).await.expect("auth UAS bind"));
    let captured = Arc::new(Mutex::new(Vec::new()));
    let count = Arc::new(AtomicU32::new(0));

    let sock_task = sock.clone();
    let captured_task = captured.clone();
    let count_task = count.clone();
    let reply = Arc::new(reply_for);

    let task = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            let (n, from) = match sock_task.recv_from(&mut buf).await {
                Ok(p) => p,
                Err(_) => return,
            };
            let bytes_slice = &buf[..n];
            let parsed = match parse_message(bytes_slice) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let request = match parsed {
                Message::Request(r) => r,
                _ => continue,
            };
            let idx = count_task.fetch_add(1, Ordering::SeqCst);
            captured_task.lock().await.push(CapturedAuthRequest {
                method: request.method().to_string(),
                cseq: request.cseq().map(|cseq| cseq.sequence()).unwrap_or(0),
                call_id: request
                    .call_id()
                    .map(|call_id| call_id.value())
                    .unwrap_or_default(),
                from_tag: request
                    .from()
                    .and_then(|from| from.tag().map(str::to_string)),
                to_header: request.raw_header_value(&HeaderName::To),
                via_header: request.raw_header_value(&HeaderName::Via),
                raw: String::from_utf8_lossy(bytes_slice).into_owned(),
            });
            let plan = reply(idx);
            let resp = build_challenge(&request, plan);
            let _ = sock_task.send_to(&resp.to_bytes(), from).await;
        }
    });

    AuthUas {
        addr,
        captured,
        count,
        task,
    }
}

fn build_challenge(request: &Request, plan: ChallengeReply) -> Message {
    match plan {
        ChallengeReply::Challenge401 { realm, nonce } => {
            let mut resp = create_response(request, StatusCode::Unauthorized);
            resp.headers.push(TypedHeader::Other(
                HeaderName::WwwAuthenticate,
                HeaderValue::Raw(digest_challenge(&realm, &nonce, "MD5", None, false).into_bytes()),
            ));
            Message::Response(resp)
        }
        ChallengeReply::Challenge407 { realm, nonce } => {
            let mut resp = create_response(request, StatusCode::ProxyAuthenticationRequired);
            resp.headers.push(TypedHeader::Other(
                HeaderName::ProxyAuthenticate,
                HeaderValue::Raw(digest_challenge(&realm, &nonce, "MD5", None, false).into_bytes()),
            ));
            Message::Response(resp)
        }
        ChallengeReply::Challenge401Full {
            realm,
            nonce,
            algorithm,
            qop,
            stale,
        } => {
            let mut resp = create_response(request, StatusCode::Unauthorized);
            resp.headers.push(TypedHeader::Other(
                HeaderName::WwwAuthenticate,
                HeaderValue::Raw(
                    digest_challenge(&realm, &nonce, &algorithm, qop.as_deref(), stale)
                        .into_bytes(),
                ),
            ));
            Message::Response(resp)
        }
        ChallengeReply::Challenge407Full {
            realm,
            nonce,
            algorithm,
            qop,
            stale,
        } => {
            let mut resp = create_response(request, StatusCode::ProxyAuthenticationRequired);
            resp.headers.push(TypedHeader::Other(
                HeaderName::ProxyAuthenticate,
                HeaderValue::Raw(
                    digest_challenge(&realm, &nonce, &algorithm, qop.as_deref(), stale)
                        .into_bytes(),
                ),
            ));
            Message::Response(resp)
        }
        ChallengeReply::Challenge401Raw(value) => {
            let mut resp = create_response(request, StatusCode::Unauthorized);
            resp.headers.push(TypedHeader::Other(
                HeaderName::WwwAuthenticate,
                HeaderValue::Raw(value.into_bytes()),
            ));
            Message::Response(resp)
        }
        ChallengeReply::Challenge407Raw(value) => {
            let mut resp = create_response(request, StatusCode::ProxyAuthenticationRequired);
            resp.headers.push(TypedHeader::Other(
                HeaderName::ProxyAuthenticate,
                HeaderValue::Raw(value.into_bytes()),
            ));
            Message::Response(resp)
        }
        ChallengeReply::Ok => Message::Response(create_response(request, StatusCode::Ok)),
    }
}

fn digest_challenge(
    realm: &str,
    nonce: &str,
    algorithm: &str,
    qop: Option<&str>,
    stale: bool,
) -> String {
    let mut out = format!(r#"Digest realm="{realm}", nonce="{nonce}", algorithm={algorithm}"#);
    if let Some(qop) = qop {
        out.push_str(&format!(r#", qop="{qop}""#));
    }
    if stale {
        out.push_str(", stale=true");
    }
    out
}
