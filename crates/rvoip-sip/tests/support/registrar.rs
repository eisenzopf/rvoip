//! Mock-registrar harness for §10 #19 (`register_refresh_vs_initial`)
//! and #27 (`registrar_response_builder`).
//!
//! Boots a raw-UDP socket that captures every inbound REGISTER, replies
//! with a caller-supplied response builder, and exposes the captured
//! requests for assertions. The pattern is the proven shape from
//! `register_423_retry.rs` / `third_party_register_integration.rs`,
//! factored out so multiple §10 tests can share it.

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

/// Captured snapshot of a single inbound REGISTER. Cheap to clone for
/// assertion code that wants to inspect Call-ID/CSeq/Expires/etc.
#[derive(Clone, Debug)]
pub struct CapturedRegister {
    pub call_id: String,
    pub cseq: u32,
    pub contact: Option<String>,
    pub expires_header: Option<u32>,
    pub from_uri: String,
    pub pai: Option<String>,
    pub raw: String,
}

impl CapturedRegister {
    fn from_request(req: &Request, raw_bytes: &[u8]) -> Self {
        let call_id = req
            .call_id()
            .map(|c| c.value().to_string())
            .unwrap_or_default();
        let cseq = req.cseq().map(|c| c.sequence()).unwrap_or(0);
        let contact = req.raw_header_value(&HeaderName::Contact);
        let expires_header = req
            .raw_header_value(&HeaderName::Expires)
            .and_then(|s| s.trim().parse().ok());
        let from_uri = req
            .from()
            .map(|f| f.uri.to_string())
            .unwrap_or_default();
        let pai = req.raw_header_value(&HeaderName::PAssertedIdentity);
        let raw = String::from_utf8_lossy(raw_bytes).into_owned();
        Self {
            call_id,
            cseq,
            contact,
            expires_header,
            from_uri,
            pai,
            raw,
        }
    }
}

/// Knob for what the registrar should reply with on the Nth inbound
/// REGISTER (`count` is 0-indexed).
pub enum RegistrarReply {
    /// Echo Contact + a numeric Expires header in a 200 OK.
    Ok { expires: u32 },
    /// Echo Contact + a numeric Expires header **and** additional
    /// arbitrary headers (`name`, `wire-value`) in a 200 OK. Used by
    /// §10 #27 to assert `RegisterResponseBuilder` setters land on the
    /// wire when authored on the server side.
    OkWithHeaders {
        expires: u32,
        extras: Vec<(HeaderName, String)>,
    },
}

impl RegistrarReply {
    /// Default reply: 200 OK with a 1-hour expiry.
    pub fn ok_hour() -> Self {
        Self::Ok { expires: 3600 }
    }
}

/// Result of booting a [`MockRegistrar`]. Keep the handle alive for the
/// duration of the test and call [`MockRegistrar::shutdown`] before the
/// test exits to free the UDP port.
pub struct MockRegistrar {
    pub addr: String,
    pub captured: Arc<Mutex<Vec<CapturedRegister>>>,
    pub count: Arc<AtomicU32>,
    task: JoinHandle<()>,
}

impl MockRegistrar {
    /// Stop the background task. Idempotent.
    pub fn shutdown(self) {
        self.task.abort();
    }

    /// Block until at least `n` REGISTERs have been captured, or `timeout`
    /// elapses. Returns the captured snapshots.
    pub async fn wait_for_n(
        &self,
        n: usize,
        deadline: Duration,
    ) -> Vec<CapturedRegister> {
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
            "registrar never saw {} REGISTERs (count={})",
            n,
            self.count.load(Ordering::SeqCst)
        );
        // Tiny settle so the Nth 200 OK has time to land at the client.
        sleep(Duration::from_millis(120)).await;
        self.captured.lock().await.clone()
    }
}

/// Boot a mock registrar on `127.0.0.1:port` and reply to each inbound
/// REGISTER using `reply_for(count)`. `count` is 0-indexed.
///
/// The closure runs once per REGISTER. Tests that only care about the
/// first response can return the same [`RegistrarReply`] every time.
pub async fn boot_mock_registrar<F>(port: u16, reply_for: F) -> MockRegistrar
where
    F: Fn(u32) -> RegistrarReply + Send + Sync + 'static,
{
    let addr = format!("127.0.0.1:{port}");
    let sock = Arc::new(
        UdpSocket::bind(&addr)
            .await
            .expect("mock registrar bind"),
    );
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
                Message::Request(r) if r.method() == Method::Register => r,
                _ => continue,
            };

            let idx = count_task.fetch_add(1, Ordering::SeqCst);
            let snapshot = CapturedRegister::from_request(&request, bytes_slice);
            captured_task.lock().await.push(snapshot);

            let plan = reply(idx);
            let response_msg = build_response(&request, plan);
            let bytes_out = response_msg.to_bytes();
            let _ = sock_task.send_to(&bytes_out, from).await;
        }
    });

    MockRegistrar {
        addr,
        captured,
        count,
        task,
    }
}

fn build_response(request: &Request, plan: RegistrarReply) -> Message {
    match plan {
        RegistrarReply::Ok { expires } => {
            let mut resp = create_response(request, StatusCode::Ok);
            if let Some(contact) = request.header(&HeaderName::Contact) {
                resp.headers.push(contact.clone());
            }
            resp.headers.push(TypedHeader::Other(
                HeaderName::Expires,
                HeaderValue::Raw(expires.to_string().into_bytes()),
            ));
            Message::Response(resp)
        }
        RegistrarReply::OkWithHeaders { expires, extras } => {
            let mut resp = create_response(request, StatusCode::Ok);
            if let Some(contact) = request.header(&HeaderName::Contact) {
                resp.headers.push(contact.clone());
            }
            resp.headers.push(TypedHeader::Other(
                HeaderName::Expires,
                HeaderValue::Raw(expires.to_string().into_bytes()),
            ));
            for (name, value) in extras {
                resp.headers.push(TypedHeader::Other(
                    name,
                    HeaderValue::Raw(value.into_bytes()),
                ));
            }
            Message::Response(resp)
        }
    }
}
