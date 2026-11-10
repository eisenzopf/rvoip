//! A TCP connection that closes after the INVITE was written fails the call
//! promptly (#311).
//!
//! On a stream transport the response to a request must come back on the
//! connection that carried it, and rvoip accepts a stream response only on
//! that exact flow. Once the peer closes it, no response can ever be accepted
//! for the INVITE, so the transaction reports a transport error (RFC 3261
//! §8.1.3.1, §17.1.4) at once instead of waiting 32 s for Timer B.
//!
//! The peer here is a raw TCP socket, so the test controls exactly when the
//! connection closes relative to the INVITE.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::timeout;

use rvoip_sip::{Config, Event, StreamPeer};
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

/// A closed connection must fail the call well inside Timer B (32 s).
const FAIL_DEADLINE: Duration = Duration::from_secs(2);
const STEP_TIMEOUT: Duration = Duration::from_secs(15);

async fn bounded<T>(step: &str, future: impl std::future::Future<Output = T>) -> T {
    timeout(STEP_TIMEOUT, future)
        .await
        .unwrap_or_else(|_| panic!("{step} did not finish within {STEP_TIMEOUT:?}"))
}

/// Read one complete SIP request (headers plus Content-Length body).
async fn read_sip_request<S: AsyncRead + Unpin>(stream: &mut S) -> Request {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&buffer[..end]).to_ascii_lowercase();
            let body_len = headers
                .lines()
                .find_map(|line| {
                    line.strip_prefix("content-length:")
                        .or_else(|| line.strip_prefix("l:"))
                        .and_then(|value| value.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            let total = end + 4 + body_len;
            if buffer.len() >= total {
                match parse_message(&buffer[..total]).expect("parse SIP request") {
                    Message::Request(request) => return request,
                    Message::Response(_) => panic!("expected a SIP request"),
                }
            }
        }
        let read = stream.read(&mut chunk).await.expect("read from caller");
        assert!(read > 0, "caller closed before sending a full request");
        buffer.extend_from_slice(&chunk[..read]);
    }
}

#[derive(Clone, Copy)]
enum PeerBehaviour {
    /// Read the INVITE, then close without answering.
    CloseAfterInvite,
    /// Answer the INVITE with 486 Busy Here, then close at once.
    BusyThenClose,
}

/// Accept one TCP connection, act on its INVITE and report the request.
async fn spawn_raw_peer(
    behaviour: PeerBehaviour,
) -> (SocketAddr, tokio::sync::oneshot::Receiver<Request>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind peer");
    let addr = listener.local_addr().expect("peer address");
    let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept caller");
        let invite = read_sip_request(&mut stream).await;
        if let PeerBehaviour::BusyThenClose = behaviour {
            let mut busy = create_response(&invite, StatusCode::BusyHere);
            if let Some(TypedHeader::To(to)) = busy
                .headers
                .iter_mut()
                .find(|header| matches!(header, TypedHeader::To(_)))
            {
                to.set_tag("busy-then-close");
            }
            stream
                .write_all(&Message::Response(busy).to_bytes())
                .await
                .expect("write 486");
        }
        let _ = seen_tx.send(invite);
        let _ = stream.shutdown().await;
        drop(stream);
    });
    (addr, seen_rx)
}

async fn caller() -> StreamPeer {
    let mut config = Config::local("caller", 0);
    config.media_port_start = 43300;
    config.media_port_end = 43400;
    bounded("caller start", StreamPeer::with_config(config))
        .await
        .expect("caller")
}

/// Wait for the call's terminal failure, returning its status code.
async fn wait_for_failure(peer: &mut StreamPeer, call: &rvoip_sip::CallId) -> Option<u16> {
    timeout(FAIL_DEADLINE, async {
        loop {
            match peer.next_event().await {
                Some(Event::CallFailed {
                    call_id,
                    status_code,
                    ..
                }) if call_id == *call => return Some(status_code),
                Some(_) => {}
                None => return None,
            }
        }
    })
    .await
    .ok()
    .flatten()
}

async fn assert_sessions_released(peer: &StreamPeer) {
    let released = timeout(FAIL_DEADLINE, async {
        loop {
            if peer.coordinator().list_sessions().await.is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(released.is_ok(), "the failed call kept its session");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tcp_close_after_invite_fails_the_call_promptly() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();
    let (peer_addr, seen) = spawn_raw_peer(PeerBehaviour::CloseAfterInvite).await;
    let mut caller = caller().await;

    let started = Instant::now();
    let call = bounded(
        "caller INVITE",
        caller
            .invite(format!("sip:bob@{peer_addr};transport=tcp"))
            .send(),
    )
    .await
    .expect("the INVITE is written before the peer closes");
    bounded("peer reads INVITE", seen)
        .await
        .expect("peer saw the INVITE");

    let status = wait_for_failure(&mut caller, &call).await;
    let elapsed = started.elapsed();
    let status = status.unwrap_or_else(|| {
        panic!("a closed TCP connection did not fail the call within {FAIL_DEADLINE:?}")
    });
    assert!(
        (500..600).contains(&status),
        "a transport failure is reported as a 5xx-class failure, got {status}"
    );
    eprintln!("TCP close failed the call with {status} after {elapsed:?}");
    assert_sessions_released(&caller).await;

    let _ = bounded("caller shutdown", caller.shutdown()).await;
}

/// A final response written just before the close is still delivered: the
/// close must not overtake a response the peer already sent.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tcp_response_written_before_close_wins_over_the_close() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();
    let mut caller = caller().await;
    for attempt in 0..10 {
        let (peer_addr, seen) = spawn_raw_peer(PeerBehaviour::BusyThenClose).await;
        let call = bounded(
            "caller INVITE",
            caller
                .invite(format!("sip:bob@{peer_addr};transport=tcp"))
                .send(),
        )
        .await
        .expect("the INVITE is written before the peer closes");
        bounded("peer reads INVITE", seen)
            .await
            .expect("peer saw the INVITE");
        let status = wait_for_failure(&mut caller, &call).await;
        assert_eq!(
            status,
            Some(486),
            "attempt {attempt}: the peer's 486 must be reported, not the close"
        );
    }
    assert_sessions_released(&caller).await;
    let _ = bounded("caller shutdown", caller.shutdown()).await;
}
