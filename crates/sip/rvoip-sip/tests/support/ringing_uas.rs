//! Ringing-only raw-UDP UAS for §10 #16 (`auto_emit_cancel_carries_headers`).
//!
//! The CANCEL auto-emit code path fires only when
//! `Action::SendCANCELWithOptions` runs with an empty
//! `pending_cancel_options` stash. The reachable shape is:
//!
//! 1. UAC originates INVITE.
//! 2. UAS responds with 100 Trying + 180 Ringing but never 200 / final.
//! 3. UAC calls `hangup(session)` while in `Initiating` →
//!    transitions to `CancelPending` without staging cancel options.
//! 4. UAS's 180 then transitions the state machine to
//!    `CancelPending + Dialog180Ringing → SendCANCELWithOptions`. The
//!    empty stash means the handler consults
//!    `dialog_adapter.auto_emit_extra_headers` and stamps them onto the
//!    CANCEL.
//!
//! A real CallbackPeer can't drive this reliably because the
//! deferred-call path still resolves through `IncomingCallGuard` (which
//! either accepts or rejects on resolve). A raw-UDP UAS gives us exact
//! control over what arrives at the UAC and when.

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
use rvoip_sip_core::types::headers::HeaderValue;

use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

/// Raw bytes plus the parsed method-name of an inbound captured request.
#[derive(Clone, Debug)]
pub struct CapturedRequest {
    pub method: String,
    pub raw: String,
}

/// Handle to a running raw-UDP ringing-only UAS. Drop or call
/// [`RingingUas::shutdown`] when the test is done.
pub struct RingingUas {
    pub addr: String,
    pub captured: Arc<Mutex<Vec<CapturedRequest>>>,
    pub count: Arc<AtomicU32>,
    task: JoinHandle<()>,
}

impl RingingUas {
    pub fn shutdown(self) {
        self.task.abort();
    }

    /// Wait for `predicate` to find a captured request, returning it.
    pub async fn wait_for<F>(&self, predicate: F, deadline: Duration) -> Option<CapturedRequest>
    where
        F: Fn(&CapturedRequest) -> bool + Send + Sync,
    {
        let waited = timeout(deadline, async {
            loop {
                let snapshot = self.captured.lock().await.clone();
                if let Some(found) = snapshot.into_iter().find(|r| predicate(r)) {
                    return Some(found);
                }
                sleep(Duration::from_millis(40)).await;
            }
        })
        .await;
        waited.ok().flatten()
    }
}

/// Boot a ringing-only raw-UDP UAS.
///
/// - On the first inbound INVITE: send 100 Trying, then after
///   `ring_delay`, send 180 Ringing. Never send a final response.
/// - On the first inbound CANCEL: send 200 OK to the CANCEL and a 487
///   Request Terminated to the original INVITE so the UAC's transaction
///   completes cleanly.
/// - Every captured request is appended to `captured` for assertions.
pub async fn boot_ringing_uas(port: u16, ring_delay: Duration) -> RingingUas {
    let addr = format!("127.0.0.1:{port}");
    let sock = Arc::new(UdpSocket::bind(&addr).await.expect("ringing UAS bind"));
    let captured = Arc::new(Mutex::new(Vec::<CapturedRequest>::new()));
    let count = Arc::new(AtomicU32::new(0));

    // Stash the most recent INVITE Request so we can echo a 487 against
    // it once the CANCEL lands.
    let last_invite: Arc<Mutex<Option<(Request, std::net::SocketAddr)>>> =
        Arc::new(Mutex::new(None));

    let sock_task = sock.clone();
    let captured_task = captured.clone();
    let count_task = count.clone();
    let last_invite_task = last_invite.clone();

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
            count_task.fetch_add(1, Ordering::SeqCst);
            let raw_str = String::from_utf8_lossy(bytes_slice).into_owned();
            captured_task.lock().await.push(CapturedRequest {
                method: request.method().to_string(),
                raw: raw_str,
            });

            match request.method() {
                Method::Invite => {
                    // 100 Trying immediately.
                    let trying =
                        Message::Response(create_response(&request, StatusCode::Trying)).to_bytes();
                    let _ = sock_task.send_to(&trying, from).await;
                    *last_invite_task.lock().await = Some((request.clone(), from));

                    // 180 Ringing after the ring_delay. Echo a To-tag so
                    // the UAC accepts the early dialog.
                    let sock_ring = sock_task.clone();
                    let invite_ring = request.clone();
                    let from_ring = from;
                    let delay = ring_delay;
                    tokio::spawn(async move {
                        sleep(delay).await;
                        let mut ringing = create_response(&invite_ring, StatusCode::Ringing);
                        // Stamp a To-tag so the UAC reliably treats this as
                        // a final 1xx with dialog context.
                        attach_to_tag(&mut ringing, "ringing-uas-tag");
                        let bytes_out = Message::Response(ringing).to_bytes();
                        let _ = sock_ring.send_to(&bytes_out, from_ring).await;
                    });
                }
                Method::Cancel => {
                    // 200 OK to the CANCEL itself.
                    let ok =
                        Message::Response(create_response(&request, StatusCode::Ok)).to_bytes();
                    let _ = sock_task.send_to(&ok, from).await;
                    // 487 Request Terminated to the originating INVITE so
                    // the UAC's INVITE transaction terminates per RFC 3261.
                    if let Some((invite, invite_from)) = last_invite_task.lock().await.clone() {
                        let mut terminated =
                            create_response(&invite, StatusCode::RequestTerminated);
                        attach_to_tag(&mut terminated, "ringing-uas-tag");
                        let bytes_out = Message::Response(terminated).to_bytes();
                        let _ = sock_task.send_to(&bytes_out, invite_from).await;
                    }
                }
                Method::Ack => {
                    // Echo nothing — the ACK is for the 487 above.
                }
                _ => {
                    // Other methods (BYE, etc.) — generic 200 OK.
                    let resp =
                        Message::Response(create_response(&request, StatusCode::Ok)).to_bytes();
                    let _ = sock_task.send_to(&resp, from).await;
                }
            }
        }
    });

    RingingUas {
        addr,
        captured,
        count,
        task,
    }
}

fn attach_to_tag(resp: &mut Response, tag: &str) {
    for hdr in resp.headers.iter_mut() {
        if let TypedHeader::To(to) = hdr {
            if to.tag().is_none() {
                to.set_tag(tag);
            }
            return;
        }
    }
}
