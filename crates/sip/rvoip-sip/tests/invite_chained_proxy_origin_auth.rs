//! A proxy 407 followed by an origin 401 must accumulate credentials while
//! retaining the exact proxy route and caller SDP bytes on every retry.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::types::Credentials;
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::headers::{HeaderAccess, HeaderValue};
use rvoip_sip_dialog::transaction::utils::response_builders::create_response;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout};

const PROXY_PORT: u16 = 36220;
const UAC_PORT: u16 = 36221;
const EXACT_SDP: &str =
    "v=0\r\no=caller 7 9 IN IP4 192.0.2.44\r\ns=exact\r\nt=0 0\r\na=x-byte-for-byte\r\n";

#[derive(Clone, Debug)]
struct SeenInvite {
    has_proxy_authorization: bool,
    has_authorization: bool,
    has_route: bool,
    session_expires: Option<String>,
    min_se: Option<String>,
    body: Vec<u8>,
}

#[test]
fn proxy_then_origin_challenge_retains_both_credentials_route_and_body() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .thread_stack_size(16 * 1024 * 1024)
        .enable_all()
        .build()
        .expect("chained auth test runtime");
    runtime.block_on(async move {
        tokio::spawn(async move {
    let socket = Arc::new(
        UdpSocket::bind(format!("127.0.0.1:{PROXY_PORT}"))
            .await
            .expect("proxy bind"),
    );
    let invite_count = Arc::new(AtomicU32::new(0));
    let seen = Arc::new(Mutex::new(Vec::<SeenInvite>::new()));

    let socket_task = Arc::clone(&socket);
    let count_task = Arc::clone(&invite_count);
    let seen_task = Arc::clone(&seen);
    let server = tokio::spawn(async move {
        let mut buffer = vec![0_u8; 8192];
        loop {
            let (length, peer) = match socket_task.recv_from(&mut buffer).await {
                Ok(received) => received,
                Err(_) => return,
            };
            let request = match parse_message(&buffer[..length]) {
                Ok(Message::Request(request)) if request.method() == Method::Invite => request,
                Ok(Message::Request(request)) if request.method() == Method::Ack => continue,
                _ => continue,
            };
            let attempt = count_task.fetch_add(1, Ordering::SeqCst);
            seen_task.lock().await.push(SeenInvite {
                has_proxy_authorization: request
                    .raw_header_value(&HeaderName::ProxyAuthorization)
                    .is_some(),
                has_authorization: request
                    .raw_header_value(&HeaderName::Authorization)
                    .is_some(),
                has_route: request.raw_header_value(&HeaderName::Route).is_some(),
                session_expires: request.raw_header_value(&HeaderName::SessionExpires),
                min_se: request.raw_header_value(&HeaderName::MinSE),
                body: request.body().to_vec(),
            });

            let mut response = match attempt {
                0 => {
                    let mut response =
                        create_response(&request, StatusCode::ProxyAuthenticationRequired);
                    response.headers.push(TypedHeader::Other(
                        HeaderName::ProxyAuthenticate,
                        HeaderValue::Raw(
                            br#"Digest realm="edge", nonce="proxy-nonce", algorithm=MD5, qop="auth-int""#
                                .to_vec(),
                        ),
                    ));
                    response
                }
                1 => {
                    let mut response = create_response(&request, StatusCode::Unauthorized);
                    response.headers.push(TypedHeader::Other(
                        HeaderName::WwwAuthenticate,
                        HeaderValue::Raw(
                            br#"Digest realm="origin", nonce="origin-nonce", algorithm=MD5, qop="auth-int""#
                                .to_vec(),
                        ),
                    ));
                    response
                }
                2 => {
                    let mut response =
                        create_response(&request, StatusCode::SessionIntervalTooSmall);
                    response.headers.push(TypedHeader::Other(
                        HeaderName::MinSE,
                        HeaderValue::Raw(b"120".to_vec()),
                    ));
                    response
                }
                _ => create_response(&request, StatusCode::Ok),
            };
            if attempt >= 3 {
                for header in &mut response.headers {
                    if let TypedHeader::To(to) = header {
                        if to.tag().is_none() {
                            to.set_tag("origin-tag");
                        }
                        break;
                    }
                }
                response.headers.push(TypedHeader::Other(
                    HeaderName::Contact,
                    HeaderValue::Raw(format!("<sip:origin@127.0.0.1:{PROXY_PORT}>").into_bytes()),
                ));
            }
            let _ = socket_task
                .send_to(&Message::Response(response).to_bytes(), peer)
                .await;
        }
    });

    let coordinator = UnifiedCoordinator::new(Config::local("alice", UAC_PORT))
        .await
        .expect("UAC coordinator");
    sleep(Duration::from_millis(150)).await;
    coordinator
        .invite(
            Some(format!("sip:alice@127.0.0.1:{UAC_PORT}")),
            "sip:bob@origin.invalid".to_string(),
        )
        .with_sdp(EXACT_SDP)
        .with_outbound_proxy(format!("sip:proxy@127.0.0.1:{PROXY_PORT};lr"))
        .with_credentials(Credentials::new("alice", "secret"))
        .send()
        .await
        .expect("initial INVITE dispatch");

    timeout(Duration::from_secs(8), async {
        while invite_count.load(Ordering::SeqCst) < 4 {
            sleep(Duration::from_millis(40)).await;
        }
    })
    .await
    .expect("four INVITEs (initial, proxy auth, origin auth, timer retry)");

    let captured = seen.lock().await;
    assert_eq!(captured.len(), 4);
    assert!(!captured[0].has_proxy_authorization);
    assert!(!captured[0].has_authorization);
    assert!(captured[1].has_proxy_authorization);
    assert!(!captured[1].has_authorization);
    assert!(captured[2].has_proxy_authorization);
    assert!(captured[2].has_authorization);
    assert!(captured[3].has_proxy_authorization);
    assert!(captured[3].has_authorization);
    assert!(captured[3].session_expires.is_some());
    assert_eq!(captured[3].min_se.as_deref(), Some("120"));
    assert!(captured.iter().all(|invite| invite.has_route));
    assert!(captured
        .iter()
        .all(|invite| invite.body.as_slice() == EXACT_SDP.as_bytes()));

            server.abort();
        })
        .await
        .expect("chained auth scenario task");
    })
}
