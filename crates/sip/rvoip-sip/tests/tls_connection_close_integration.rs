//! A TLS connection that closes after the INVITE was written fails the call
//! promptly (#311).
//!
//! A TLS 1.3 server that rejects the client certificate sends its
//! `certificate_required` alert just after the handshake, so the INVITE write
//! can land before the alert. A peer can also simply close the connection.
//! Either way no response can come back on that connection, and rvoip accepts
//! a stream response only on the exact flow that carried the request, so the
//! INVITE transaction reports a transport error (RFC 3261 §8.1.3.1, §17.1.4)
//! at once instead of waiting 32 s for Timer B.
//!
//! The peer is a raw TLS server, so the test controls exactly when the
//! connection closes relative to the INVITE.

use std::net::{SocketAddr, TcpListener as StdTcpListener};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Once};
use std::time::{Duration, Instant};

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair, KeyUsagePurpose,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tempfile::TempDir;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::timeout;
use tokio_rustls::TlsAcceptor;

use rvoip_sip::{Config, Event, StreamPeer};
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

/// A closed connection must fail the call well inside Timer B (32 s).
const FAIL_DEADLINE: Duration = Duration::from_secs(2);
const STEP_TIMEOUT: Duration = Duration::from_secs(15);

fn install_crypto_provider() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

async fn bounded<T>(step: &str, future: impl std::future::Future<Output = T>) -> T {
    timeout(STEP_TIMEOUT, future)
        .await
        .unwrap_or_else(|_| panic!("{step} did not finish within {STEP_TIMEOUT:?}"))
}

fn reserve_tcp_addr() -> SocketAddr {
    let listener = StdTcpListener::bind("127.0.0.1:0").expect("reserve TCP port");
    listener.local_addr().expect("reserved TCP address")
}

fn write(path: &Path, contents: &str) {
    std::fs::write(path, contents).expect("write PKI material");
}

/// A test CA, a server identity for the raw peer (in memory) and a client
/// identity for the caller (on disk, as `Config` expects).
struct TestPki {
    _dir: TempDir,
    ca: PathBuf,
    caller_cert: PathBuf,
    caller_key: PathBuf,
    server_cert: CertificateDer<'static>,
    server_key: Vec<u8>,
}

impl TestPki {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("PKI directory");
        let ca_key = KeyPair::generate().expect("CA key");
        let mut ca_params =
            CertificateParams::new(vec!["Close Test CA".into()]).expect("CA parameters");
        ca_params.distinguished_name = DistinguishedName::new();
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "Close Test CA");
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let ca_cert = ca_params.self_signed(&ca_key).expect("CA certificate");
        let issuer = Issuer::from_params(&ca_params, &ca_key);
        let ca = dir.path().join("ca.pem");
        write(&ca, &ca_cert.pem());

        let issue = || {
            let key = KeyPair::generate().expect("leaf key");
            let mut params = CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()])
                .expect("leaf parameters");
            params.extended_key_usages = vec![
                ExtendedKeyUsagePurpose::ServerAuth,
                ExtendedKeyUsagePurpose::ClientAuth,
            ];
            let cert = params.signed_by(&key, &issuer).expect("leaf certificate");
            (cert, key)
        };
        let (caller, caller_key_pair) = issue();
        let caller_cert = dir.path().join("caller.pem");
        let caller_key = dir.path().join("caller-key.pem");
        write(&caller_cert, &caller.pem());
        write(&caller_key, &caller_key_pair.serialize_pem());
        let (server, server_key_pair) = issue();
        Self {
            _dir: dir,
            ca,
            caller_cert,
            caller_key,
            server_cert: server.der().clone(),
            server_key: server_key_pair.serialize_der(),
        }
    }

    fn acceptor(&self) -> TlsAcceptor {
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![self.server_cert.clone()],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.server_key.clone())),
            )
            .expect("server TLS config");
        TlsAcceptor::from(Arc::new(config))
    }
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
    /// Answer 180 Ringing, then close: the INVITE is in Proceeding, where
    /// RFC 3261 runs no transaction timer at all.
    RingingThenClose,
}

/// Accept one TLS connection, act on its INVITE and report the request.
async fn spawn_raw_tls_peer(
    pki: &TestPki,
    behaviour: PeerBehaviour,
) -> (SocketAddr, tokio::sync::oneshot::Receiver<Request>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind peer");
    let addr = listener.local_addr().expect("peer address");
    let acceptor = pki.acceptor();
    let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.expect("accept caller");
        let mut stream = acceptor.accept(tcp).await.expect("TLS handshake");
        let invite = read_sip_request(&mut stream).await;
        if let PeerBehaviour::RingingThenClose = behaviour {
            let mut ringing = create_response(&invite, StatusCode::Ringing);
            if let Some(TypedHeader::To(to)) = ringing
                .headers
                .iter_mut()
                .find(|header| matches!(header, TypedHeader::To(_)))
            {
                to.set_tag("ringing-then-close");
            }
            stream
                .write_all(&Message::Response(ringing).to_bytes())
                .await
                .expect("write 180");
            stream.flush().await.expect("flush 180");
            // Let the caller see the 180 before the connection goes.
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        let _ = seen_tx.send(invite);
        // close_notify, then FIN.
        let _ = stream.shutdown().await;
        drop(stream);
    });
    (addr, seen_rx)
}

async fn caller(pki: &TestPki, media_base: u16) -> StreamPeer {
    let mut config = Config::local("caller", 0).tls_reachable_contact(
        reserve_tcp_addr(),
        &pki.caller_cert,
        &pki.caller_key,
    );
    config.tls_extra_ca_path = Some(pki.ca.clone());
    config.media_port_start = media_base;
    config.media_port_end = media_base + 100;
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

async fn run_close_case(behaviour: PeerBehaviour, media_base: u16) {
    install_crypto_provider();
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();
    let pki = TestPki::new();
    let (peer_addr, seen) = spawn_raw_tls_peer(&pki, behaviour).await;
    let mut caller = caller(&pki, media_base).await;

    let call = bounded(
        "caller INVITE",
        caller
            .invite(format!("sip:bob@{peer_addr};transport=tls"))
            .send(),
    )
    .await
    .expect("the INVITE is written before the peer closes");
    bounded("peer reads INVITE", seen)
        .await
        .expect("peer saw the INVITE");
    // The peer closes right after reporting the INVITE.
    let closed_at = Instant::now();

    let status = wait_for_failure(&mut caller, &call).await;
    let elapsed = closed_at.elapsed();
    let status = status.unwrap_or_else(|| {
        panic!("a closed TLS connection did not fail the call within {FAIL_DEADLINE:?}")
    });
    assert!(
        (500..600).contains(&status),
        "a transport failure is reported as a 5xx-class failure, got {status}"
    );
    eprintln!("TLS close failed the call with {status} after {elapsed:?}");
    assert_sessions_released(&caller).await;

    let _ = bounded("caller shutdown", caller.shutdown()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tls_close_after_invite_fails_the_call_promptly() {
    run_close_case(PeerBehaviour::CloseAfterInvite, 43500).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tls_close_after_ringing_fails_the_call_promptly() {
    run_close_case(PeerBehaviour::RingingThenClose, 43700).await;
}
