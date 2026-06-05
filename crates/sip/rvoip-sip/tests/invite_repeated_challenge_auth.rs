//! Repeated WWW-Authenticate header negotiation for challenged INVITE.
//!
//! RFC 3261 permits repeated authentication headers. Session-core should see
//! all challenges, not just the first one, so UAC negotiation can choose the
//! strongest configured compatible scheme.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout};

use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::SipClientAuth;

use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::{HeaderAccess, HeaderValue};

use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

const UAS_PORT: u16 = 36200;
const UAC_PORT: u16 = 36201;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invite_repeated_www_authenticate_headers_are_negotiated_together() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();

    let sock = Arc::new(
        UdpSocket::bind(format!("127.0.0.1:{UAS_PORT}"))
            .await
            .expect("UAS bind"),
    );
    let invite_count = Arc::new(AtomicU32::new(0));
    let retry_authorization: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    let sock_task = sock.clone();
    let count_task = invite_count.clone();
    let auth_task = retry_authorization.clone();
    let uas_task = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            let (n, from) = match sock_task.recv_from(&mut buf).await {
                Ok(pair) => pair,
                Err(_) => return,
            };
            let parsed = match parse_message(&buf[..n]) {
                Ok(message) => message,
                Err(_) => continue,
            };
            let request = match parsed {
                Message::Request(request) => request,
                _ => continue,
            };
            match request.method() {
                Method::Invite => {
                    let idx = count_task.fetch_add(1, Ordering::SeqCst);
                    if idx == 0 {
                        let mut resp = create_response(&request, StatusCode::Unauthorized);
                        resp.headers.push(TypedHeader::Other(
                            HeaderName::WwwAuthenticate,
                            HeaderValue::Raw(
                                br#"Digest realm="pbx", nonce="digest-first", algorithm=MD5, qop="auth""#
                                    .to_vec(),
                            ),
                        ));
                        resp.headers.push(TypedHeader::Other(
                            HeaderName::WwwAuthenticate,
                            HeaderValue::Raw(br#"Bearer realm="api", scope="sip.invite""#.to_vec()),
                        ));
                        let _ = sock_task
                            .send_to(&Message::Response(resp).to_bytes(), from)
                            .await;
                    } else {
                        *auth_task.lock().await =
                            request.raw_header_value(&HeaderName::Authorization);
                        let mut resp = create_response(&request, StatusCode::Ok);
                        for header in resp.headers.iter_mut() {
                            if let TypedHeader::To(to) = header {
                                if to.tag().is_none() {
                                    to.set_tag("uas-repeated-auth-tag");
                                }
                                break;
                            }
                        }
                        resp.headers.push(TypedHeader::Other(
                            HeaderName::Contact,
                            HeaderValue::Raw(
                                format!("<sip:bob@127.0.0.1:{UAS_PORT}>").into_bytes(),
                            ),
                        ));
                        let _ = sock_task
                            .send_to(&Message::Response(resp).to_bytes(), from)
                            .await;
                    }
                }
                Method::Ack => {}
                _ => {
                    let resp = create_response(&request, StatusCode::Ok);
                    let _ = sock_task
                        .send_to(&Message::Response(resp).to_bytes(), from)
                        .await;
                }
            }
        }
    });

    let coord = UnifiedCoordinator::new(Config::local("alice", UAC_PORT))
        .await
        .expect("UAC coordinator");
    sleep(Duration::from_millis(150)).await;

    let auth = SipClientAuth::any([
        SipClientAuth::digest("alice", "secret"),
        SipClientAuth::bearer_token("token-123").allow_bearer_over_cleartext(true),
    ]);
    let call_id = coord
        .invite(
            Some(format!("sip:alice@127.0.0.1:{UAC_PORT}")),
            format!("sip:bob@127.0.0.1:{UAS_PORT}"),
        )
        .with_auth(auth)
        .send()
        .await
        .expect("invite.send()");

    let observed = timeout(Duration::from_secs(8), async {
        loop {
            if invite_count.load(Ordering::SeqCst) >= 2 {
                return;
            }
            sleep(Duration::from_millis(40)).await;
        }
    })
    .await;
    assert!(
        observed.is_ok(),
        "UAS never saw initial INVITE plus auth retry"
    );

    let retry = retry_authorization
        .lock()
        .await
        .clone()
        .expect("retry INVITE must carry Authorization");
    assert_eq!(
        retry, "Bearer token-123",
        "repeated challenges must be negotiated together; got {retry:?}"
    );

    let _ = coord.bye(&call_id).send().await;
    uas_task.abort();
}
